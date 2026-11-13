# Edge protocol and WARP protocol

This document gives the wire behaviour that `podnet` uses. The reference is
`cloudflared` (`github.com/cloudflare/cloudflared`). File names in this
document are paths in that repository.

## 1. Quick-tunnel provisioning

A quick tunnel starts with one HTTPS request
(`cmd/cloudflared/tunnel/quick_tunnel.go`):

```text
POST https://api.trycloudflare.com/tunnel
Content-Type: application/json
User-Agent: cloudflared/<version>
```

The body is empty. A good answer is:

```json
{"success":true,"result":{
  "id":"<uuid>","name":"qt-...","hostname":"<random>.trycloudflare.com",
  "account_tag":"<hex>","secret":"<base64, 32 bytes>"},"errors":[]}
```

`id`, `account_tag` and `secret` are the `TunnelAuth` credentials for the
registration. `--service-url` changes the endpoint. The request goes on TCP
443 and uses `HTTPS_PROXY` when it is set.

`podnet` does not log the answer body when it cannot decode it, because the body
contains the tunnel secret.

## 2. Edge discovery

The edge addresses come from the SRV record
`_v2-origintunneld._tcp.argotunnel.com`. The targets are
`region1.v2.argotunnel.com` and `region2.v2.argotunnel.com`, port 7844
(`edgediscovery/allregions`).

`podnet` tries these sources in sequence:

1. DNS over HTTPS (the JSON API of `cloudflare-dns.com`). This works on hosts
   that have no DNS resolver but have HTTPS egress.
2. The system resolver, then DNS over TLS.
3. A static list of edge addresses.

`PODNET_EDGE=ip:port,ip:port` replaces discovery.

## 3. Transports

| Protocol | Socket | TLS SNI | ALPN |
| --- | --- | --- | --- |
| QUIC | UDP 7844 | `quic.cftunnel.com` | `argotunnel` |
| HTTP/2 | TCP 7844 | `h2.cftunnel.com` | none |

* The SNI and the ALPN of QUIC are different strings. If the ALPN value is
  sent as the SNI, the handshake completes key derivation and then stops.
* The edge certificate chains to Cloudflare roots that are not in public
  trust stores. The roots are the fact `roots` of the edge of `pod-services`
  (PodUtils/podcrates), the certificates of `src/cloudflare/cf-edge-roots.pem`
  (from `tlsconfig/cloudflare_ca.go`). Without them, TLS fails with
  `UnknownIssuer`.
* Port 443 of the edge does not serve the tunnel service: TLS there is
  TLS 1.2, and its chain does not end at the edge roots (section 6). So
  podnet uses 7844 only, and carries the edge on TCP 443 inside WARP
  (section 5.3).

### 3.1 QUIC

The first bidirectional stream that `podnet` opens is the control stream. It has
no preamble. It carries capnp RPC: `podnet` calls
`RegistrationServer.registerConnection(auth, tunnelId, connIndex, options)`
(`connection/quic_connection.go`, `tunnelrpc/registration_client.go`). The edge
opens each later stream for one visitor request.

`podnet` also serves a `CloudflaredServer` bootstrap object on the control
stream. The edge uses it as a liveness check. Without it, the edge marks the
tunnel offline and visitors get HTTP 530.

Registration features (`ClientInfo.features`): podnet sends the default
list of cloudflared 2026.10.7 (`features/features.go`), which is
`support_datagram_v2` only (`schema::FEATURES`). cloudflared 2026.10.7
removes `serialized_headers`, `quick_reconnects`, `allow_remote_config`,
`support_quic_eof` and `management_logs` before it sends the list
(TUN-10904: the edge supports them with no flag), and also
`support_datagram_v3` and `support_datagram_v3_1`. podnet does not send
`postquantum` (it uses a classical key exchange, section 6) or
`support_datagram_v3_2` (it carries no UDP or ICMP datagram of a visitor).
The answers of HTTP/2 always carry the serialized header (section 3.2).

cloudflared makes one connection for a quick tunnel (`ha-connections` is
set to 1 in `quick_tunnel.go`) and four for a named tunnel. podnet makes two
by default for both (`--ha-connections`, 1 to 8). The extra connections are
for redundancy: each one registers with the edge, and the edge chooses the
connection of each visitor request. One request (one stream) uses one
connection, so `--ha-connections` does not make one transfer faster; it can
let more requests run at the same time on more connections.

Each connection starts at another region, as in cloudflared: connection 0
(and 2) dials the addresses of `region1` first, connection 1 (and 3) the
addresses of `region2`. A fault of one region then does not hit each
connection. IPv4 addresses stay before IPv6 addresses.

### 3.2 HTTP/2

`connection/http2.go`. On the TLS connection, the edge is the HTTP/2 client and
`podnet` is the HTTP/2 server. The edge opens a stream for each request.
`podnet` classifies each stream from its headers, in this order:

| Condition | Type |
| --- | --- |
| `Cf-Cloudflared-Proxy-Connection-Upgrade: update-configuration` | configuration push |
| `Cf-Cloudflared-Proxy-Connection-Upgrade: websocket` | WebSocket |
| `Cf-Cloudflared-Proxy-Src` is not empty | TCP stream |
| `Cf-Cloudflared-Proxy-Connection-Upgrade: control-stream` | control stream |
| all other requests | HTTP |

* Control stream: `podnet` sends status 200 and keeps the stream open. The
  request body and the response body carry the same capnp RPC as on QUIC.
* HTTP: the visitor headers are plain HTTP/2 headers. `podnet` changes the
  request into the `ConnectRequest` of section 4 and serves it with the same
  service layer as QUIC.
* Response: the origin headers go in one header,
  `cf-cloudflared-response-headers`, as `base64(name):base64(value)` pairs with
  `;` between them (raw standard base64, no padding). The HTTP/2 rules for
  header values do not apply to that header. `content-length` is also sent as
  a plain header. `cf-cloudflared-response-meta: {"src":"origin"}` tells the
  edge that the origin answered. Status 101 becomes 200, because HTTP/2 has no
  101.
* WebSocket: `podnet` adds `Connection: Upgrade` and `Upgrade: websocket` for the
  origin, because HTTP/2 has no upgrade headers.
* Configuration push: `podnet` answers `{"lastAppliedVersion":N,"err":null}`
  with the version of the push. A push of more than 64 KiB, or a push that
  does not end in 30 seconds, gets `{"lastAppliedVersion":-1,"err":"..."}`.
* Limits: the HTTP/2 handshake after TLS has a time limit of 10 seconds.
  The edge can open 256 streams at the same time, as on QUIC.

`tools/check-header-oracle.sh` compares the header encoding of `podnet` with the
Go function `SerializeHeaders` of cloudflared.

### 3.3 Leg order

`--protocol auto --transport auto` tries QUIC, then HTTP/2, then QUIC through
WARP (section 5). The attempts are real registrations, in a staggered race
(`crates/podnet-core/src/race.rs`, `--leg-failover staggered`, the default):

* The next leg starts when the leg before has not registered in 2 seconds,
  or at once when it fails. A filtered port does not answer, so a host that
  drops UDP and TCP 7844 starts WARP after about 4 seconds, not 16.
* The first leg that registers stays. When two legs register in the same
  turn, the earlier leg of the order stays. Each other leg that registered
  gets the unregister call, and then its connection closes. The legs that
  still run are cancelled.

`--leg-failover serial` starts a leg only after the one before failed. When
another leg can follow, an attempt has the time limit of the budgets of its
leg (`attempt_limit` in `src/tunnel.rs`): 3 tries of the registration, each
with the limits of its steps, and 5 seconds at most between two tries.

| Leg | One try | Time limit |
| --- | --- | --- |
| `quic` | QUIC handshake 10 s, registration 30 s | 130 s |
| `http2`, `http2+relay` | TCP, TLS and HTTP/2 handshake 10 s each, control stream and registration 30 s each | 280 s |
| `quic+warp` | | no limit |

A try over more than one edge address can take longer than one try of the
table. The time limit then stops it, and the next leg starts.

UDP (`PODNET_UDP`, `crates/podnet-core/src/udp.rs`): with `probe` (the default) and
`--protocol auto`, podnet sends one QUIC packet with a reserved version (RFC
9000 section 15) to an edge address on UDP 7844 while the quick tunnel is
provisioned. The result is answered (a datagram came back), refused (the send
failed, or an ICMP error came back on the connected socket) or unknown (no
answer in 150 ms). Only refused removes the QUIC leg; unknown and answered
keep the order. A bind alone never gives answered. `PODNET_UDP=off` removes the
QUIC leg; `PODNET_UDP=on` sends no probe. `podnet net doctor` sends the same
packet with a deadline of 3 s.

The stagger is not a probe: each leg is a full attempt on its own port and
protocol. `podnet` does not probe TCP 7844 to select a leg, because a firewall
can close TCP 7844 and keep UDP 7844 open. Inside a leg, the HTTP/2 dial
starts the next edge address 250 ms after the one before (RFC 8305).

### 3.4 HTTP/2 through a WebSocket relay

`--transport relay` (`src/ws_relay.rs`) has one leg, `http2+relay`. It is
the HTTP/2 leg of section 3.2, with a different TCP dial:

1. TCP to the relay on port 443 (through `HTTPS_PROXY` when it is set),
   then TLS with ALPN `http/1.1` and the public roots.
2. A WebSocket upgrade to `wss://RELAY/connect/IP/7844`, with the token in
   the `X-Relay-Token` header. IP is an IPv4 edge address of discovery.
3. Binary frames carry the TCP bytes both ways. podnet sends frames of 16 KiB
   or less; the relay writes at most 64 KiB to a Worker socket in one call.
   An empty frame from the relay is a keepalive; podnet skips it.
4. In the WebSocket: TLS to the edge (SNI `h2.cftunnel.com`, the edge
   roots), then HTTP/2 as in section 3.2.

The token: `PODNET_RELAY_TOKEN`, or `POST /v1/mint` with the body `{}`, which
gives `{token, expires}`. A minted token is minted again when it has less
than 10 minutes. A 403 for a minted token mints a new one one time. A 403
that names the ALLOW list is a policy refusal: a new token does not help.
A minted token is kept in `relay-token-HOST.json` (mode 0600) in the state
directory, so the next process does not mint again.

The hosts: `--relay` (or `PODNET_RELAY`) takes one URL or a comma list.
`pod-relay` of PodUtils/podcrates keeps the hosts, the tokens and the pool
(`src/ws_relay.rs` is the glue). Each deployment of a list gets its own
token. The default relay takes the first 3 hosts of its pool as alternates
(`MAX_ALTERNATES` of `pod-relay`): the `pool[].host` names of
`/relays.json`, read after a mint and kept for 7 days in
`relay-pool-HOST.json`. Until a pool is cached, the default relay uses the
first 3 hosts of its seed. The seed holds 6 hosts in different regions
(`pod-services`). Another relay host has no alternates. A connect tries the
first host, then the next host after 2 s, or at once after a failure. A
refused fixed token or a policy refusal stops the hosts that did not start:
another host gives the same answer. On 2026-11-04 each of the 37 hosts of the
pool reached `198.41.192.167:7844` (`/trace?target=`, road `vpc`).

Limits of `tcp.ssh.relay.ajam.dev` (podssh measured them; the relay sets
them, and they can change):

| Limit | Value | Effect on the leg |
| --- | --- | --- |
| No payload | 180 s | The HTTP/2 PING each 30 s is payload. |
| Bytes of a session | 64 MiB, both directions, TLS included | At 60 MiB, podnet takes no new stream on the connection and connects a replacement. The open streams go on. |
| Time of a session | 12 h | The same at 11 h 30 min. |
| MASQUE through the relay | the same budget | At 60 MiB or 11 h 30 min, a new MASQUE session takes the new flows. The open flows go on in the old session; it closes after 90 s (`warp::live::RENEW_DRAIN`). |
| Egress | IPv4 TCP; Cloudflare addresses through the Workers VPC route | No QUIC; IPv6 edge addresses are skipped. |

podssh measured TLS to `region1.v2.argotunnel.com:7844` through this relay
(GitHub Actions run 41072705691). The live check of podnet registers a quick
tunnel through it with UDP and TCP 7844 dropped on the host. Measured
2026-11-01 (live run 41303368779): `http2+relay` registered at ord02 in
743 ms, the second connection registered at ord02, and the public URL
answered 200.

## 4. Stream framing (QUIC)

A visitor stream starts with a 6-byte signature (`0A 36 CD 12 A1 3E`), the
2-byte version `01`, and a capnp `ConnectRequest`:

* `dest`: the URL (HTTP and WebSocket) or `host:port` (TCP).
* `type`: `http`, `websocket` or `tcp`.
* `metadata`: `HttpMethod`, `HttpHost`, and one `HttpHeader:<Name>` entry for
  each header value.

`podnet` answers with the same preamble and a capnp `ConnectResponse` (`error`,
`metadata`). After that, the stream is a byte pipe. The schemas are in
`src/cloudflare/schemas/`. `src/cloudflare/capnp.rs` implements the Cap'n
Proto encoding and `src/cloudflare/rpc.rs` the RPC subset (section 3.1).

In the response metadata, the status is the entry `HttpStatus`. Each header
must be an entry `HttpHeader:<Name>`. The edge drops a key without the prefix.
Then it makes its own chunked framing and sends no origin headers.

## 5. WARP

### 5.1 Registration

The API is `https://api.cloudflareclient.com/v0a5641`. All calls use the
headers `User-Agent: WARP for Android` and `CF-Client-Version: a-6.38.9-5641`.
After a dial, TLS or 5xx failure, podnet sends the call again to the second
name of the API, `https://api.devices.cloudflare.com` (it answered
`GET /reg/{id}` of an unknown device with the same 404 on 2026-11-04).
`PODNET_WARP_API=https://HOST[:PORT]` sets the one base to use.

| Call | Use |
| --- | --- |
| `POST /reg` | Make a device. The answer has the device `id` and a bearer `token`. |
| `PATCH /reg/{id}` | Enrol a P-256 key (X.509 SPKI, base64), `key_type: secp256r1`, `tunnel_type: masque`. |
| `GET /reg/{id}` | Read the device. |
| `GET /reg/{id}/account` | Read the account: type, WARP+ state, quota. |
| `PUT /reg/{id}/account` | Attach a license key: `{"license":"..."}`. |
| `DELETE /reg/{id}` | Remove the device. |

Only the answer to `POST /reg` has the token. `podnet` keeps it in the saved
identity file, because the other calls need it. A key rotation is a new
`PATCH /reg/{id}` with a new key.

The device record gives the MASQUE endpoint (for example `162.159.198.2:443`),
the public key of the endpoint, and the tunnel addresses (`172.16.0.2` and an
IPv6 address in `2606:4700:110::/48`).

### 5.2 MASQUE session

* TLS to the endpoint on TCP 443, SNI `consumer-masque.cloudflareclient.com`,
  ALPN `h2`. The client certificate is self-signed with the enrolled P-256 key.
  `podnet` checks the server key against the key in the device record.
* One HTTP/2 extended `CONNECT` to `https://cloudflareaccess.com` with
  `cf-connect-proto: cf-connect-ip`.
* The DATA frames of that stream carry capsules (RFC 9297). Each IP packet is
  one capsule of type 0. The value is the bare IP packet, with no context
  ID before it (RFC 9484 puts a context ID of 0 there). podnet writes and
  reads no context ID (`capsule::tests::a_data_capsule_has_no_context_id`).
  The tunnel MTU is 1280.
* The request headers: `cf-connect-proto: cf-connect-ip`,
  `capsule-protocol: ?1`, `pq-enabled: false` and an empty `user-agent`.
  The front door accepts them (measured, podssh issue 63, a run of podnet).
* The server certificate is not publicly trusted: its root is
  `CN=2024-02-27 Self-Signed Root, O=Cloudflare, Inc.` (measured, live run
  41268348982, `podnet doctor`). Its key is byte for byte the SPKI of the
  `PATCH` answer, which is what podnet pins (measured, podssh issue 63).
* A client certificate that does not match the enrolled key gives a TLS
  alert, not an HTTP status (measured, podssh issue 63). podnet reports it as
  "TLS alert from the peer" (`hop::tls_text`).
* The `PATCH` answer lists the endpoint ports
  `[443, 500, 1701, 4500, 4443, 8443, 8095]` (measured, podssh issue 63).
  podnet uses 443 only (`register::MASQUE_PORT`).
* The newer L4 proxy mode of the front door is HTTP/3 only (read, podssh
  issue 63). podnet uses CONNECT-IP over HTTP/2.
* `smoltcp` owns the tunnel address and runs TCP and UDP on it. This is
  `podnet::warp::WarpStack`.

### 5.3 The edge through WARP

`podnet::warp::quic::endpoint` makes a `noq::Endpoint` whose UDP socket is a
UDP socket of the `WarpStack` (`pod_quic::queued::QueuedDatagrams` of
PodUtils/podcrates). The QUIC connection to the edge (UDP 7844) thus
goes inside the MASQUE session. The host opens only TCP 443.

### 5.4 Limits

* The WARP service limits new devices (`POST /reg`) for each address:
  HTTP 429 with error 1015 (observed after about 10 in 90 minutes).
* A destination sees an egress address of Cloudflare, not the address of
  the device. One MASQUE session keeps one egress address; a new session
  gets a new one (measured by a tester, issue 183). `--session-max-age`
  renews the session on purpose.

* A proxy that terminates TLS (and makes a new TLS connection with its own CA)
  cannot carry the MASQUE session, because the session needs the client
  certificate and the pinned server key. The registration calls still work
  through such a proxy.
* The WARP proxy of `podnet` (`podnet warp proxy`) carries TCP (SOCKS5 and HTTP
  `CONNECT`) and UDP (SOCKS5 UDP ASSOCIATE). It resolves names through the
  tunnel. The UDP relay socket of an association is on `127.0.0.1`. When the
  proxy cannot open it, the reply is `0x01` (general failure).
* A host that refuses UDP on `127.0.0.1` uses the UDP association in the
  stream: the SOCKS5 command `0x83`
  (`pod_proxy::udp::CMD_UDP_IN_STREAM`) in place of `0x03`. The reply
  has no relay address. Each datagram then goes on the control connection
  as a frame: a length of 2 bytes (big-endian), then the datagram with its
  SOCKS5 UDP header (RFC 1928 section 7). The answers come back the same
  way. `podnet warp run --backend seccomp` and `podnet pod` use it; the unix
  socket of `podnet warp up` serves it.
* A name in a UDP datagram (ATYP 3) is resolved once for each association,
  with an A query through the tunnel; AAAA only when there is no A record.
  A DNS query through the tunnel is sent again after 1.2 s, in a time limit
  of 5 s.

## 6. Edge, measured

Each row has a mark: **measured** (a command ran, with its date and run),
**read** (from the source of cloudflared 2026.10.7 or from a document) or
**inferred**. The tool is `PodUtils/podcrates:scripts/services/edge-facts.sh`
(from the tag v0.14.0); the workflow `services.yml` of
podcrates runs it each day and by hand (input `measure`). The rows below are
from run 41268348982 (2026-11-01, a GitHub runner, ubuntu-24.04), address
198.41.192.107. Its exit code is 1 when one of the two facts that podnet needs
is not true (the rows marked *need*).

| Fact | Result | Mark |
| --- | --- | --- |
| SRV `_v2-origintunneld._tcp.argotunnel.com` | `1 1 7844 region1.v2.argotunnel.com.` and `2 1 7844 region2.v2.argotunnel.com.` | measured |
| Addresses of `region1.v2.argotunnel.com` | 10: 198.41.192.7, .27, .37, .47, .57, .67, .77, .107, .167, .227 | measured |
| Addresses of `region2.v2.argotunnel.com` | 10: 198.41.200.13, .23, .33, .43, .53, .63, .73, .113, .193, .233 | measured |
| *need*: TLS on TCP 7844, SNI `h2.cftunnel.com`, no ALPN | TLS 1.3, X25519, verified against `cf-edge-roots.pem`, 31 ms | measured |
| SNI `probe.cftunnel.com`, `region1.v2.argotunnel.com`, or no SNI | Each: TLS 1.3, verified, 20 to 29 ms | measured |
| ALPN `h2` offered | The edge selects no ALPN | measured |
| Key group X25519 only; P-256 only | Both work; no post-quantum group is needed | measured |
| Issuer of the edge certificate | `OU=CloudFlare Origin SSL ECC Certificate Authority, O=CloudFlare, Inc.` (`podnet doctor`) | measured |
| TCP 443 and 8443 of the edge address | TLS 1.2; the chain does not verify against the edge roots | measured |
| TCP 80 of the edge address | Not TLS | measured |
| *need*: the first bytes after the handshake on 7844 | `PRI * HTTP/2.0`: the edge is the HTTP/2 client | measured |
| UDP 7844, one QUIC packet with a reserved version | A version negotiation of 27 bytes (`podnet doctor`) | measured |
| Latency from the runner to the edge, TCP | best 6.2 ms, median 7.2 ms (`podnet doctor`, 9 samples) | measured |
| cloudflared and `HTTPS_PROXY` | Two requests for a proxy for the edge, cloudflare/cloudflared#350 and #1025, are closed (2023-09-05). podnet uses the proxy for each TCP connection (section 1, `src/egress.rs`) | read |
| The prechecks of cloudflared | They use the name `probe.cftunnel.com` and test DNS, QUIC, TCP and TLS to both regions, and `api.cloudflare.com:443` | read (podssh design notes, cloudflared 2026.10.6) |

`podnet doctor` measures the same path on any host, through its proxy.

## 7. Limits of quick tunnels

From the Cloudflare page
[Quick Tunnels](https://developers.cloudflare.com/tunnel/get-started/quick-tunnels/)
(read 2026-11-01):

* Quick tunnels are for testing and development. For production, use a
  named tunnel (`--token` or `--config` with `credentials-file:`).
* No uptime guarantee.
* At most 200 requests in flight for each quick tunnel. More get `429`.
* No Server-Sent Events (SSE). An event stream can arrive in bursts, or at
  its end.

The provision service (`api.trycloudflare.com`) also limits the
provisions of each address. Observed (issue 182): after 5 to 7
provisions in 12 to 70 minutes, the service answers `429` (error 1015)
with a `Retry-After`. The wait is a fixed expiry: a new request does not
make it longer. podnet waits at most 300 s for one provision (with
`--detach`, at most `--ready-timeout` for the first start), then exits 75
with `retry_after_s` in the error report. A script needs its own loop. A
supervisor of `--detach` waits the `Retry-After` (at most 1 h) before its
next start. Each restart of a quick tunnel uses one provision.

cloudflared makes one edge connection for a quick tunnel (section 3.1).

## 8. Terms

These documents apply. This section gives facts, not legal advice.

* Quick tunnels: the disclaimer of cloudflared 2026.10.7
  (`cmd/cloudflared/tunnel/quick_tunnel.go`) says that account-less
  tunnels "are subject to the Cloudflare Online Services Terms of Use
  (https://www.cloudflare.com/website-terms/)", and that Cloudflare may
  investigate the use of tunnels for violations of those terms.
* WARP: the Application Terms of the WARP client license "the
  Application", and forbid reverse engineering of it. Cloudflare may
  "block certain kinds of usage" (read, podssh design notes). podnet is a
  separate program: it contains no code of the WARP application, and it
  uses the public registration API of section 5.1.

## 9. `podnet doctor --json`, schema 1

`podnet doctor --json` prints one JSON object. The key `schema` is 1. Within
schema 1, a key is added, and never removed or renamed. A consumer ignores a
key that it does not know. The test `doctor_json_keys_are_fixed`
(`tests/doctor.rs`) pins the key sets.

`--quick` makes one sample for each path and ends the run in 15 s. A part that
does not end in time gets the verdict `unknown` and the reason `no answer in
the time limit of --quick`. The keys stay the same.

The verdict words are `ok`, `fail` and `unknown`. The key `status` has the
same word as `verdict` in each object that has both. In `udp`, `verdict` is
one of `answered`, `refused` and `unknown`.

| Key | Type | Meaning |
| --- | --- | --- |
| `schema` | number | Always 1. |
| `podnet` | string | The version of podnet. |
| `ok` | bool | True when one leg is `ok`. |
| `quick` | bool | True for a run of `--quick`. |
| `next` | string | The step to take now. |
| `names` | string or null | What the proxy does with names and addresses on 7844. |
| `roads` | array | One object for each road (`direct`, `warp`, `relay`). |
| `lanes` | object | The seven egress lanes (below). |
| `legs` | array | One object for each leg (`quic`, `http2`, `quic+warp`, `http2+relay`). |
| `paths` | array | One object for each TCP path (the hops). |
| `clock` | object | The clock against the `Date` of an API. |
| `git` | object | The installed git. |
| `ssh_relay` | object | The SSH relay of `podnet ssh-relay`. |
| `carriers` | array | One object for each carrier of `podnet link`. |
| `proxy` | object | The proxy of the run. |
| `udp` | object | The UDP 7844 probe. |

Each object of `roads` has `road`, `verdict`, `status` and `reason` (strings).
Each object of `legs` has `leg`, `verdict`, `status` and `reason` (strings).

Each object of `paths`:

| Key | Type |
| --- | --- |
| `id` | string |
| `target` | string (`host:port`) |
| `via` | string or null (the proxy) |
| `verdict`, `status`, `detail` | string |
| `samples` | object: `sent` and `answered` (numbers), `loss` (number, 0 to 1), `best_ms`, `median_ms`, `p95_ms` and `jitter_ms` (number or null) |
| `tls` | object or null: `verdict`, `status`, `issuer` (string or null), `detail` |
| `dns` | object or null: `addresses` (strings) and `ms`, or `error` |

* `clock`: `verdict`, `status` and `detail` (strings), `skew_s` (number or
  null).
* `git`: `verdict`, `status` and `detail` (strings), `version` (string or
  null).
* `ssh_relay`: `verdict`, `status` and `detail` (strings); `relay` (string
  or null: `HOST:PORT`), `user` (string or null) and `source` (string or
  null: `flag` or `instance NAME`); `connect_ms` and `total_ms` (number or
  null); `banner` (string or null: the SSH identification of the relay);
  `auth` (bool or null: the relay accepted the user). The probe opens no
  session and asks for no forward. The relay comes from `--ssh-relay
  HOST:PORT` with `--ssh-user USER`, else from the first running
  `ssh-relay` instance (`podnet ps`). With neither, `verdict` is `unknown`
  and `detail` is `no SSH relay is given`. A stage that fails has `fail`
  and the stage in `detail` (`connect`, `banner`, `handshake` or `auth`). A
  stage with no answer in three times `--timeout` has `unknown`.
* `carriers`: one object for each carrier of the registry, in its order: `carrier`, `verdict`, `status` and `detail` (strings).
  `ok` means that the host can use the carrier. `fail` means that it cannot, and `detail` gives the reason of `available()`.
* `proxy`: `url` (string or null), `credentials` (bool or null), `direct`
  (strings), `text` (string).
* `udp`: `target`, `verdict`, `status` and `detail` (strings).

### Lanes

`lanes` has exactly the seven keys below. Each value is an object with
`verdict` (`ok`, `fail` or `unknown`), `ms` (number or null: the best TCP
sample of the lane, or the answer time of a probe) and `reason` (string).

| Lane | Source | `ms` |
| --- | --- | --- |
| `tunnel_quic` | the leg `quic` | null |
| `tunnel_http2` | the leg `http2` | the path `edge-address` |
| `tunnel_quic_warp` | the leg `quic+warp` | the MASQUE path |
| `masque_tcp443` | the path `masque`, or `masque-v6` when only it works | the same path |
| `masque_udp443` | one QUIC packet to the MASQUE endpoint on UDP 443: `ok` on any answer (`ms` is the time); `fail` on a refusal, and on silence when UDP 7844 answers; `unknown` on silence when it does not, and with a proxy (no proxy carries UDP) | the answer time, else null |
| `ws_upgrade` | an HTTP/1.1 `Upgrade: websocket` request over TLS to the edge host on 7844, through the proxy when one is set: `ok` on `101`; `fail` on another status, a TLS fault or a refusal; `unknown` on no answer in twice `--timeout` | the time to the status line when `ok`, else null |
| `proxy` | the paths that go through a proxy: `ok` when one works, `fail` when each fails, else `unknown` (`no proxy is set` without a proxy) | the best of the working paths |

## 10. The connector profile

A connector is a program that starts `podnet tunnel` and reads its output.
This section lists what such a program can rely on. The newest signed
release that has the profile is v0.4.0-beta.3.

The rule: within a minor version (0.4.x), a change to the profile only adds.
A flag, an exit code, a JSON key or a value word is never removed or renamed.
A consumer ignores a key that it does not know. The test
`connector_profile_is_pinned` (`tests/cli/profile.rs`) checks the lists below
against the binary and against this file. The test `the_ready_keys_are_fixed`
(`src/cli/cmd/run.rs`) pins the keys of the ready line.

### Flags of `podnet tunnel`

`--auth-password-file`, `--auth-user`, `--ca-file`, `--check`, `--color`,
`--config`, `--credentials-file`, `--detach`, `--egress`, `--forward`,
`--ha-connections`, `--hello-world`, `--help`, `--http-host-header`, `--json`,
`--leg-failover`, `--log-format`, `--masque-endpoint`, `--name`, `--no-proxy`,
`--no-restart`, `--no-tls-verify`, `--no-verify`, `--port`, `--protocol`, `--proxy`, `--qr`,
`--quiet`, `--ready-timeout`, `--relay`, `--replace`, `--resolve`,
`--run-for`, `--service-url`, `--strip-accept-encoding`, `--tcp`, `--token`,
`--token-file`, `--transport`, `--unix`, `--url`, `--user-agent`, `--verbose`,
`--warp-identity`.

### Exit codes

The list is `podnet man exit-status`. An error with no class exits 1.

| Code | Name |
| --- | --- |
| 0 | `success` |
| 1 | `failure` |
| 3 | `no-backend` |
| 64 | `usage` |
| 66 | `no-input` |
| 68 | `no-host` |
| 69 | `unavailable` |
| 71 | `os-error` |
| 75 | `tempfail` |
| 77 | `no-permission` |
| 78 | `config` |
| 124 | `timeout` |

### Readiness

podnet has no HTTP endpoint `/ready` and no metrics listener (`--metrics` is
refused with exit 64). A connector reads the ready line instead. With
`--json`, `tunnel` prints it on stdout when the tunnel is up. With
`--detach`, the same data is the ready document: the last line of stdout of
the start, and the key `ready` of an instance in `ps --json`.

The ready line has `event` (always `ready`), `schema` (1), `podnet` (the
version), `url` (string or null), `tunnel_id` (string), `locations` (array of
strings) and `leg` (string). With `--forward` it also has `forward` (string
or null) and `forward_path` (string).

The ready document has `url` (string or null), `verified` (bool), `tunnel_id`,
`locations` and `leg`. It has `unverified` (string) when the check of the
public URL did not finish, and `forward` and `forward_path` as the ready line
does.

### `podnet ps --json`

One object with `schema` (1), `podnet` (the version), `dir` (string: the run
directory) and `instances` (array). Each instance has these keys:

| Key | Type |
| --- | --- |
| `name` | string |
| `state` | string (`starting` or `ready`) |
| `pid` | number |
| `supervisor` | number or null |
| `command` | string |
| `args` | array of strings |
| `cwd` | string |
| `started` | number (Unix seconds) |
| `restarts` | number |
| `log` | string (a path) |
| `ready` | object or null (the ready document) |

### `podnet doctor --json`

Section 9. The test `doctor_json_keys_are_fixed` (`tests/doctor.rs`) pins it.

## Failure causes

The error line of podnet names the hop that failed, and a second line says
what may help. The failure report (`PODNET_ERROR_PROGRAM`) has the hop in
`hop`. The text of a peer (a proxy reply, an edge reason) is kept, with no
control characters.

| Hop | Example cause | podnet does | What may help |
| --- | --- | --- | --- |
| `proxy` | `403 Forbidden: not on the egress allowlist`, 407 with NTLM | Stops the attempt; another leg can follow | Fix the proxy URL or its allow-list; `--no-proxy` |
| `dns` | No resolver answers, DoH blocked | Tries the system resolver, then DoH by IP address | `--resolve HOST=IP`; `socks5h://` |
| `tls` | `UnknownIssuer`, `TLS alert from the peer: HandshakeFailure`, an expired certificate | Stops the attempt | `--ca-file` for a TLS-inspecting proxy; check the clock |
| `edge-register` | A refusal with `should_retry = false` (bad token, deleted tunnel) | Stops the leg; does not retry | Check the token, or use a quick tunnel |
| `edge-register` | A refusal with `retry_after` | Retries after `retry_after` (1 s to 300 s) | Nothing |
| `edge-stream` | No PING answer, GOAWAY, a lost connection | Connects again at once, then with backoff and jitter, with no limit | `--transport warp` on a host with only TCP 443 |
| `origin` | Connection refused | Answers the visitor with 502 | Start the origin; check `--url` |

`scripts/soak.sh` runs N quick tunnels end to end, keeps one log for each
trial, and prints the failure rate.
