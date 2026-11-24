# QUIC

This page gives the QUIC work of issue 486: the HTTP/3 server shape, the stock client, the TUIC and Juicity roads, the
connection ID rotation and the migration fallback. The measurements are from 2026-11-20 on Linux x86_64.

## The HTTP/3 shape

`--h3-shape` selects the congestion profile of an HTTP/3 server: `common` (the default), `cubic`, `newreno`, `bbr`, or
`NAME-no-control`. The shape is in the HTTP/3 module, `src/warp/h3.rs`; `doctor --quick` prints it:

```text
h3-shape  common congestion=cubic control-stream=no settings=no
h3-control-stream  not observed (this command runs no HTTP/3 server)
h3-cid-rotation  480 to 780 s, jitter 200 per 1000, samples 615 s and 629 s
```

The line `control-stream=no` is the shape as a value. `doctor` runs no server, so it cannot observe a control stream,
and it says so.

The server is `src/warp/h3/serve.rs`. It keeps the HTTP/3 control stream open for the life of the connection and
serves one request. `src/warp/h3/qlog.rs` writes a qlog trace. The command `podnet warp h3` serves it:

```sh
podnet warp --h3-shape common h3 --listen 127.0.0.1:0 --qlog /tmp/h3.qlog --run-for 120
```

```text
h3 listen 127.0.0.1:56481
h3 qlog /tmp/h3.qlog
h3-shape=common congestion=cubic control-stream=no settings=no
```

The shape line is printed before the first session, so `control-stream=no` there means no session has been observed. The qlog of the run records the control stream and each request.

### The control stream

`pod-warp` of `podcrates` v0.82.0 or later (`pod-warp/src/h3/server.rs`) keeps the control stream open for the lifetime
of the connection (RFC 9114, section 6.2.1). A server that ends the control stream sends a FIN, and a client closes the
connection with `H3_CLOSED_CRITICAL_STREAM`. `src/warp/h3/serve.rs` stays in podnet because `podnet warp h3` adds
transport qlog output (`--qlog`) and structured HTTP/3 trace events on top of the shape and the wire framing of `pod-warp`.

## The stock client

`tests/quic_stock/main.go` is a quic-go v0.59.0 `http3.Transport` client. It uses `InsecureSkipVerify`, the ALPN `h3`,
and one GET. `tests/quic_h3_stock.rs` starts the podnet listener, runs the client, and asserts the trace.

The test needs Go 1.24 and quic-go v0.59.0 in the Go module cache. Without `go` it prints a named skip; the test is
not ignored.

```sh
cargo test -p podnet --test quic_h3_stock -- --nocapture
```

```text
client=quic-go/http3 version=v0.59.0 proto=HTTP/3.0 status=200 body="pod-warp h3 shape\n"
test a_stock_quic_go_client_completes_a_request_against_the_shape ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s
```

The client also prints a warning of quic-go about the receive buffer of the host (`208 kiB`, wanted `7168 kiB`). It
does not change the result: the request ends with 200.

The same test asserts that the HTTP/3 trace holds `http3:control_stream`, `http3:request` and `status:200`.

## The qlog of the control stream

`tests/quic_qlog.rs` starts the podnet listener with `Trace` and `qlog_dir`, connects with
`pod_quic::connect::client_config`, sends the control stream (`[0x00]` and SETTINGS), sends `GET /` with a QPACK
static-table section, reads 200 and closes. It reads both qlog files.

The qlog of noq is JSON-SEQ: each line starts with the record separator `0x1e`, and the test strips it. A stream frame
is in `data.frames` with `frame_type: "stream"`, `stream_id`, `offset` and `fin`. `StreamId::index()` is the ordinal,
not the full QUIC id, so the server writes the full id. The trace and the qlog agree at `stream_id=3`.

```sh
cargo test -p podnet --test quic_qlog -- --nocapture
```

```text
h3 qlog: control stream_id=3, traced files=["client-...-client.qlog", "podnet-h3-...-server.qlog"]
test the_qlog_shows_the_http3_control_stream ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

## The TUIC road

`src/roads/tuic/` is a client of TUIC version 5 over `pod-quic`: the authentication token of the TLS exporter, the
`Connect` command, the `quic` relay mode of UDP and the session stream with its first read and its close. The layout is
the one of `tuic-protocol/tuic`'s `SPEC.md`.

The road is a port of the TUIC code of `podssh-ws`: `command.rs` is podssh's command layout, `link.rs` its link grammar
and credential types, and `stream.rs` its session stream. Podnet adds `PENDING_MAX`, the bounded queue and the UDP relay.
A shared `pod-tuic` crate of podcrates would replace the port. No such crate exists, so the road is a port, and the
follow-up that removes the podssh copy is named below.

The three deliberate differences from podssh:

- The link takes `allow_insecure=1`. Podnet's `--tuic` gate and its tests run a self-signed server on the loopback;
  podssh refuses the setting.
- The connect sends no one-packet probe of UDP before the handshake. podssh's probe is one packet, and the QUIC-8
  gate puts a lossy link in front of the server: a dropped probe would fail the connect. QUIC's own handshake retries
  its Initials, so the road lets it.
- The heartbeat of `stream.rs` ends at once. The road advertises no QUIC DATAGRAM (see the reference server below), so
  `connection.max_datagram_size()` is `None`. QUIC's own keepalive, which `Settings` sets, keeps the connection alive.

A parameter that is not known is refused, so a typo is not a silent default. `version` other than 5 and a
`congestion_control` other than cubic, `new_reno` or bbr are refused by name.

```sh
podnet warp --tuic 'tuic://UUID:PASSWORD@HOST:PORT?sni=NAME' nc HOST PORT
podnet warp --tuic 'tuic://UUID:PASSWORD@HOST:PORT?sni=NAME' nc -u HOST PORT
```

`--tuic` goes with `nc` only, uses no WARP identity and starts no WARP session. A link with a self-signed certificate
needs `allow_insecure=1`. `PODNET_WARP_TUIC` gives the same link.

### The drop policy (QUIC-8)

The queue from the local socket to the command stream is bounded: `PENDING_MAX` is 64 datagrams.

- A full queue never drops a datagram. The reader stops reading the local socket, and the next push waits for room.
  That wait is the stop-read signal.
- `QueueStats.max_pending` is the most datagrams that waited at one time. It is at most 64.
- `QueueStats.full` counts the times the queue was full. It does not count lost datagrams, because the queue loses
  none.
- The road does not drop a datagram at the queue. QUIC carries each command on its own stream, so the retransmission
  of a lost QUIC packet recovers the datagram.

The test sends 200 datagrams through a relay that drops 5 of each 100 QUIC datagrams of the loopback:

```sh
cargo test -p podnet --test quic_tuic -- --nocapture
```

```text
tuic udp: sent=200 received=200 link_dropped=3 link_passed=40 loss=7.0% queue=QueueStats { max_pending: 64, full: 4 }
test udp_echo_survives_five_percent_loss_within_the_queue_bound ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.01s
```

The relay saw 43 QUIC datagrams for the 200 commands, because QUIC coalesces commands in one packet. It dropped 3 of
them, 7.0 percent of that small sample; the target is 5 percent, and the test accepts 1 to 15 percent. The loss is the
loss of the link, not of the commands: each of the 200 came back. The queue reached its bound of 64; `full` counts 5
events at the bound, and no datagram was lost at the queue.

The CLI path has its own test, against a server built from `SPEC.md`:

```sh
cargo test -p podnet --test quic_tuic_cli -- --nocapture
```

```text
tuic cli conflict: status=ExitStatus(unix_wait_status(16384)) stderr="podnet: --tuic and --via-relay are exclusive"
test the_tuic_option_refuses_the_relay ... ok
tuic cli: status=exit status: 0 stdout="podnet tuic nc"
test the_tuic_option_of_warp_nc_carries_stdin_to_stdout ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

The second test proves that `--tuic` with `--via-relay` stops before the network with exit 64. The two flags pick
different transports for `nc`, so the command line takes one of them.

### The reference server

The reference server is `tuic-server` 2.0.0-dev10 of `tuic-protocol/tuic` (formerly `EAimTY/tuic`). The release asset
`tuic-server-x86_64-linux-musl` was fetched through the operator's proxy:

```sh
curl -sSL -o "${TMPDIR:-/tmp}/tuic-server-v2" \
  "https://github.com/Itsusinn/tuic/releases/download/v2.0.0-dev10/tuic-server-x86_64-linux-musl"
sha256sum "${TMPDIR:-/tmp}/tuic-server-v2"
```

```text
d2c76a3bd4fbd32b209d4f29bbf76dcb80b4c7d25ae8ef1a8b5130e0364c498a  tuic-server-v2
```

The hash is the pin in podssh's own `.github/workflows/tuic-interop.yml`. The binary says:

```sh
"${TMPDIR:-/tmp}/tuic-server-v2" --version
```

```text
tuic-server 2.0.0-dev10
```

The test writes a self-signed certificate and this configuration, starts the binary with `-c`, and waits for its
port. The server binds `127.0.0.1`, the certificate is for `localhost`, ALPN is `h3`, and the experimental rules that
drop loopback and private targets are off:

```toml
log_level = "info"
server = "127.0.0.1:PORT"
[users]
"123e4567-e89b-12d3-a456-426614174000" = "reference-password"
[tls]
certificate = "cert.pem"
private_key = "key.pem"
alpn = ["h3"]
hostname = "localhost"
[experimental]
drop_loopback = false
drop_private = false
```

The client carries a TCP echo and 200 UDP datagrams of 32 bytes through a loopback relay that drops 5 of each 100 QUIC
datagrams in front of the server:

```sh
TUIC_SERVER="${TMPDIR:-/tmp}/tuic-server-v2" cargo test -p podnet \
  --test quic_tuic_reference -- --nocapture
```

Without `TUIC_SERVER` the test prints a named skip. `scripts/ci/reference-tests.sh` runs it and fails when the
variable is absent.

```text
tuic reference: server=tuic-server-v2 version=tuic-server 2.0.0-dev10 udp=127.0.0.1:33661 tcp=127.0.0.1:39957 datagrams=200 tcp_bytes=21 link_dropped=4 link_passed=70 loss=5.4% queue=QueueStats { max_pending: 64, full: 7 }
test the_reference_tuic_server_carries_udp_at_loss_and_tcp ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 8.09s
```

All 200 datagrams came back once, the TCP echo returned its 21 bytes, and the queue reached its bound of 64 with no
loss at the queue. The ports are ephemeral, so a later run prints other values.

The run found one divergence between the server and the specification. `SPEC.md` says a server replies to a `Packet`
in the relay mode of that first `Packet`. This server chooses by the peer's QUIC capability instead: when the client
advertises DATAGRAM, the server relays UDP on QUIC datagrams, which are unreliable and which a stream-only client does
not read. The first runs therefore forwarded all 200 datagrams (`UDP packet -> outbound` in the server's trace) and
returned no echo. The client now advertises no DATAGRAM:

```rust
let mut transport = settings.transport(false);
transport.datagram_receive_buffer_size(None);
```

`Settings::datagram_buffer = None` is not the same setting: `None` keeps noq's default, which is on. The transport
takes the field directly. With DATAGRAM off, the server takes the `quic` mode of the first `Packet`, and every echo
returns on a stream. This is an interop workaround, and it is why the 5-percent-loss run loses no datagram.

### The dedupe with podssh

The podssh copy remains, and removing it is the named follow-up. One change in `PodUtils/podssh` deletes these files
and the references to them:

```text
crates/podssh-ws/src/tuic/mod.rs
crates/podssh-ws/src/tuic/command.rs
crates/podssh-ws/src/tuic/link.rs
crates/podssh-ws/src/tuic/link_tests.rs
crates/podssh-ws/src/tuic/stream.rs
crates/podssh-ws/tests/tuic.rs
crates/podssh-cli/src/tuic/mod.rs
crates/podssh-cli/src/tuic/pipe.rs
crates/podssh-cli/src/tuic/reach.rs
crates/podssh-cli/src/doctor/tuic.rs
crates/podssh-cli/src/man/notes_tuic.rs
.github/workflows/tuic-interop.yml
```

A shared `pod-tuic` crate in podcrates, which both podnet and podssh take by a git tag, would remove the copy. No such
crate exists. A path dependency works in one checkout only. The podnet copy is the one that the tests above exercise.

TUIC keeps no 30-stream bound. `SPEC.md` names none, and QUIC's own `max_concurrent_uni_streams` and
`max_concurrent_bidi_streams` make an `open_uni` or `open_bi` wait until the peer allows it. The 30-stream rule below
is Juicity's.

## The Juicity road

`src/roads/juicity.rs` is a client of Juicity over `pod-quic`. The layout is the one of the reference implementation,
`juicity/juicity` v0.5.0, which `docs/spec_en.md` does not describe exactly:

- Authentication: `[version 0][command 0][uuid 16][token 32]`, where the token is the TLS exporter of the uuid bytes
  and the password, with a length of 32.
- TCP: `[network 1][type][address][port]`, then the bytes.
- UDP: one stream for each source triplet. The stream starts with `[network 3][type][address][port]`. Each datagram
  then has `[type][address][port][length 2][payload]`, with no network byte.
- The address types are the Trojan types of the reference: 1 IPv4, 4 IPv6, 3 a name. `docs/spec_en.md` gives 0, 1 and
  2. The reference server of v0.5.0 refuses the values of the document with `unexpected metadata type`. The client
  uses the reference wire, because that is what a Juicity server answers.

The gate names a reference server. The test `tests/quic_juicity_reference.rs` runs `juicity-server` v0.5.0 (release
`juicity-linux-x86_64.zip`), a local UDP echo and a local TCP echo. It needs the binary in `JUICITY_SERVER`, and
prints a named skip without it:

```sh
JUICITY_SERVER=/path/juicity-server cargo test -p podnet \
  --test quic_juicity_reference -- --nocapture
```

`scripts/ci/reference-tests.sh` requires both reference binaries and `go`, writes the raw output of each test to
`$PODNET_EVIDENCE_DIR` (default `.work/evidence`), and exits nonzero when a required binary is
missing.

```text
juicity reference: server=juicity-server udp=127.0.0.1:33040 tcp=127.0.0.1:45771 datagrams=20 tcp_bytes=24 streams=32 connections=2 both echoed
test the_reference_juicity_server_carries_udp_and_tcp ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.05s
```

The ports are ephemeral, so a later run prints other values. The 32 streams and 2 connections are the 30-stream test
below.

The stand-in test `tests/quic_juicity.rs` covers the same paths with no binary:

```sh
cargo test -p podnet --test quic_juicity -- --nocapture
```

```text
juicity: udp datagrams=50 tcp bytes=14 both echoed
test a_juicity_client_completes_udp_and_tcp_sessions ... ok
juicity 30-stream: streams=31 connections=2 server_connections=2 limit=30
test a_juicity_client_opens_a_new_connection_after_thirty_streams ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

### The 30-stream gap

`docs/spec_en.md` asks a client to open a new QUIC connection after opening a cumulative total of 30 streams on the
same connection, when the library cannot report the available streams. `Juicity` keeps a pool of authenticated
connections and `STREAM_LIMIT = 30`. Each `tcp` and `udp` stream takes one slot; when the current connection has 30,
the next stream opens and authenticates a second connection, and the retired connection stays until its own streams
end.

The stand-in test opens 31 TCP streams and asserts the client's count and the server's own count; the run above shows
`streams=31 connections=2 server_connections=2 limit=30`. The reference test adds 30 streams after its UDP flow and
first TCP stream, so 32 streams in all, and its line above shows `streams=32 connections=2`.

The two clients keep their own specifications: Juicity gets the fixed 30-stream fallback, and TUIC keeps QUIC's flow
control with no fixed bound.

## The rotation and the fallback

`pod_quic::migrate::Schedule::common()` gives an interval of 8 to 13 minutes and a jitter of 200 parts of 1000, so
each draw is between 6.4 and 15.6 minutes. `doctor` prints the two samples of the run:

```text
h3-cid-rotation  480 to 780 s, jitter 200 per 1000, samples 615 s and 629 s
```

`tests/quic_rotation.rs` carries a transfer of 10 minutes and two rotations, then a peer that refuses migration. The
10 minutes are simulated: the test uses `#[tokio::test(start_paused = true)]`, so the run takes 0.38 s of wall time.

```sh
cargo test -p podnet --test quic_rotation -- --nocapture
```

```text
test a_peer_that_refuses_migration_gets_a_new_connection ... ok
test ten_minutes_and_two_rotations_lose_no_byte ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s
```

The first test asserts two or more rotations on one connection, one resumption, at least 10 new connection IDs and no
lost datagram. The second asserts `Outcome::Reconnected`, a new connection and a resumed session; its probe wait is
600 ms, below `migrate::PROBE_WAIT` of 5 s.

## What a stand-in proves

A server built from a specification proves the layout, the token, the state machine and the bytes in each direction.
It does not prove interoperability with another implementation, and it does not find a divergence between the
specification and the code that ships. The Juicity and TUIC reference tests are the answer to that limit: both run a
release binary of the reference implementation. The stand-in tests remain, because they need no binary.

The rotation test proves the migration code of `pod-quic` over a switch in memory. It does not prove a path change on
a real network.

## The gates of issue 486

| Gate | Status | Evidence |
| --- | --- | --- |
| A stock HTTP/3 client completes a request | done | `quic_h3_stock`, quic-go v0.59.0, 200 |
| A qlog shows the control stream | done | `quic_qlog`, `stream_id=3` |
| `doctor` prints the congestion profile | done | `doctor --quick`, `h3-shape` |
| UDP echo through TUIC at 5 percent loss with the queue bound | done | `quic_tuic`, 200 of 200, `max_pending=64`; `quic_tuic_reference`, tuic-server 2.0.0-dev10, 200 of 200 |
| A Juicity client completes a session against a reference server | done | `quic_juicity_reference`, v0.5.0, UDP and TCP |
| A Juicity client opens a new connection after 30 streams | done | `quic_juicity`, server counts 2; `quic_juicity_reference`, 32 streams, 2 connections |
| One TUIC implementation, with the dedupe recorded | done as a port | `src/roads/tuic/`; the podssh follow-up is the file list above |
| 10 minutes and two rotations lose no byte; a refused migration reconnects | done | `quic_rotation`, two tests |
| The drop policy is documented | done | this page, The drop policy |

The MASQUE work of QUIC-1 to QUIC-5 (the split of the provider enrolment, `--masque-endpoint`, `CONNECT-UDP` to a
local echo, the IPv6-only host and the second provider) is not part of these seven gates and was not changed here.
