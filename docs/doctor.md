# Checks: doctor, status, bench and bug reports

This page gives the commands that check this host and podnet: `doctor` (the path to the edge, and what
dev work can do), `pod doctor`, `net doctor`, `status`, `ports`, `components`, `bench`, `bug-report` and `agent`. Each
check gives `ok`, `fail` or `unknown`.

## `podnet doctor`

`podnet doctor` tests the path from this host to the edge, and gives `ok`,
`fail` or `unknown` for each hop (the words of `podnet net doctor` and
`podnet doctor --agent` too). In `--json`, the key is `status`; the old key
`verdict` has the same value for one release:

* the proxy of each target (the password is never printed), and the
  targets that `NO_PROXY` sends direct;
* TCP to `api.trycloudflare.com:443`, `api.cloudflareclient.com:443`, the
  edge on 7844 by name and by address, the MASQUE front door
  (`162.159.198.2:443`) and the WebSocket relay (`PODNET_RELAY`). A name is
  resolved first, with the chain of podnet: the addresses and the time, or
  the error. Each path gets up to 9 samples, 125 ms apart, in
  at most 10 s: best, median, p95, jitter and loss. A refusal (a proxy 403,
  a TCP reset) is an answer, not a loss;
* TLS on each open path: the issuer, and if the chain ends at the roots that
  podnet expects. A proxy that answers 200 and then sends no byte, and a proxy
  that terminates TLS, are named;
* names and addresses: if the proxy allows one and refuses the other;
* UDP 7844: one QUIC packet (answered, refused or unknown);
* the lane `masque_udp443`: one QUIC packet to the MASQUE front door on UDP 443.
  `ok` on any answer. `fail` on a refusal, and on silence when UDP 7844
  answers. `unknown` on silence when UDP 7844 does not answer, and when a
  proxy is set (no proxy carries UDP);
* the lane `ws_upgrade`: an HTTP/1.1 `Upgrade: websocket` request over TLS to
  the edge host on 7844, through the proxy when one is set. `ok` on `101`.
  `fail` on another status, a TLS fault or a refusal. `unknown` when no
  answer comes in twice `--timeout`. The probe adds no host: it uses the
  targets that the doctor already has;
* the clock: this host against the `Date` of the first API that answers
  TLS. More than 300 s off is `fail` (certificates and tokens fail).

Then a table of the legs `quic`, `http2`, `quic+warp` and `http2+relay`
(TCP and TLS to the relay; the relay protocol is not tried), with the
reason for each leg that cannot work, and the roads: `direct` (quic or
http2), `warp` (quic+warp) and `relay` (http2+relay). The last line,
`next:`, gives the step to take: the `podnet tunnel` command for the legs
that work, the clock to set, or the report for an issue (`next` in
`--json`). Exit 0: at least one leg can work. Exit 1: none can. `--json` gives one document; `--samples` and `--timeout`
make it shorter.

```sh
podnet doctor
podnet doctor --json | jq '.legs'
```

## What dev work can do: `podnet doctor --agent`

```sh
podnet doctor --agent           # a table and the commands to run
podnet doctor --agent --json
```

For each task (git over HTTPS and over SSH, pip, npm, cargo, go modules,
docker pull), the doctor tries three paths: `direct`, the proxy of the
environment, and a running `podnet warp up`. A TLS check is a handshake
with the public roots: `tls-intercepted` means that the chain is not
public but the CA bundle of this host (`SSL_CERT_FILE`) trusts it, so a
proxy or a gateway terminates TLS. It also tests UDP DNS to 1.1.1.1,
IPv6 and TCP on 127.0.0.1, and prints the commands: for example
`podnet warp up`, `podnet warp ssh-config`, or "nothing to do".

## Options of `podnet doctor`

* `--quick`: one sample for each path and 15 s for the whole run. A part (the two lane probes too) that does not end in time gets
  the verdict `unknown`. The JSON document keeps the same keys.
* `--samples N` (1 to 9; default 9) and `--timeout SECS` (default 3, for one connection and one TLS
  handshake) make the run shorter.
* `--relay URL` (`PODNET_RELAY`) selects the relay to test. A loopback `http://` relay is allowed.
* `--ssh-relay HOST:PORT` and `--ssh-user USER` add the row `ssh_relay`: the TCP connect, the SSH banner and
  the login of the user (the methods of [relay.md](relay.md); the key `auth_method` names the one that the relay took), with no session and no forward. With no flag, the row reads the relay of
  a running `podnet ssh-relay` instance; with no relay, the verdict is `unknown` ([relay.md](relay.md)).
* `--connect-proxy HOST:PORT` (requires `--ssh-relay`) sends the probe through this HTTP CONNECT proxy. Without
  it, the probe uses the proxy that the other commands use for the relay: `--proxy`, `PODNET_PROXY`, then the
  proxy variables, and `NO_PROXY` applies (a relay on loopback goes through the proxy too). The probe sends no
  proxy credentials. If that proxy is SOCKS5, a unix socket, a command, or has credentials, the row is `fail`
  with the reason and the probe does not dial. A proxy that answers 407 is `fail` at the stage `connect`. The
  text row names the proxy (`proxy HOST:PORT`, or `proxy none`). The JSON row has `proxy` (`http://HOST:PORT`,
  with no credentials, or `null`).
* `--json`: one document.

## The JSON document

`podnet doctor --json` prints one object with `"schema": 1`. A key is added within schema 1 and never
removed or renamed. A consumer ignores a key that it does not know. The tests pin the key sets
(`tests/doctor.rs`).

| Key | Meaning |
| --- | --- |
| `schema` | Always 1. |
| `podnet` | The version of podnet. |
| `ok` | True when one leg is `ok`. |
| `quick` | True for a run of `--quick`. |
| `next` | The step to take now. |
| `names` | What the proxy does with names and addresses on 7844. |
| `roads` | `direct`, `warp` and `relay`, each with `verdict`, `status` and `reason`. |
| `lanes` | Seven egress lanes: `tunnel_quic`, `tunnel_http2`, `tunnel_quic_warp`, `masque_tcp443`, `masque_udp443`, `ws_upgrade` and `proxy`, each with `verdict`, `ms` and `reason`. |
| `legs` | `quic`, `http2`, `quic+warp` and `http2+relay`, each with `verdict`, `status` and `reason`. |
| `paths` | One object for each TCP path: `id`, `target`, `via`, `verdict`, `detail`, `samples` (loss, best, median, p95, jitter), `tls` and `dns`. |
| `clock`, `git`, `ssh_relay`, `proxy`, `udp` | The clock, the installed git, the SSH relay, the proxy of the run and the UDP 7844 probe. |
| `carriers` | One object for each carrier of `podnet link` ([link.md](link.md)): `carrier`, `verdict`, `status` and `detail` (the reason when the host cannot use the carrier). The row `icmp` says whether a raw ICMP socket opens here: `ok` names `--allow-raw icmp`; `fail` gives `a raw ICMP socket needs CAP_NET_RAW or uid 0`. A carrier row does not change `ok`. |

`docs/protocol.md` section 9 gives each key and its type. Section 10 is the profile for a program that
reads the document (flags, exit codes, readiness, failure causes).

## Other checks

* `podnet pod doctor [--json]` measures what the host permits for `podnet pod`: `unshare`, `/dev/net/tun`,
  `ptrace`, `seccomp` user notification, memory access of the program, netfilter in a namespace and a
  veth wire. It says whether the `netns`, the `netns-kernel` and the `seccomp` backend can run (`backend_netns_kernel` in the JSON), the flow budget of the
  `seccomp` backend, and how much each backend holds. It exits 3 when no backend can run ([pods.md](pods.md)).
* `podnet net doctor [--json]` probes what the host permits for the virtual network: `bind` and `listen`
  for each socket family, `/dev/net/tun`, `/dev/vsock`, a C compiler for the shims, seccomp and
  capabilities, the passwd entry of this uid, UDP to the edge, `tailscaled` (`--socket PATH`; a daemon that is not `Running` is a `fail`) and the SOCKS5 listener of
  NetBird. It prints the commands that each listener allows ([networks.md](networks.md)).
  `--network SPEC` (repeatable) brings up a network of the registry, as `forward --network` takes it, and adds
  its row `network SPEC`: `ok` with the status of the link, or `fail` with the stage that failed. The row holds
  no credential.
* `podnet components [--json]` lists each carrier and each service of this build, with whether this host can use it
  and the reason if not ([link.md](link.md), Carriers).
* `podnet status` prints one line of JSON about this host and opens no connection: the version,
  the platform, the proxy and its source, the TLS roots in `tls_roots`, the state of the WARP identity,
  the limit of open files in `nofile`, the detached instances and `PODNET_ERROR_PROGRAM`. It has
  `"schema": 1`.
* `podnet ports [--probe] [--json]` lists the listening TCP ports of this host (Linux).
  `--probe` connects to each port and lists only those that accept. With `--json`, the others are in
  `unreachable`.
* `podnet bug-report [--doctor] [--include-link]` prints a text for a bug report: the version, `podnet status`, the
  detached instances with the end of their logs, and the names of the variables that are set. It hides
  device IDs, tunnel IDs, tokens and credentials. It opens no connection unless `--doctor` adds the JSON
  of `podnet doctor`. `--include-link` adds the dump of each link instance that runs (the verb `dump` of its control
  socket, [link.md](link.md)); an instance that does not answer is one line with the error.

## Measure a way out: `podnet bench`

`podnet bench URL` sends `GET` requests to an `http://` or `https://` URL through the way out of the
process (`--egress`, `--proxy` or the proxy variables; see [proxies.md](proxies.md)) and measures the
latency, the rate and the errors.

```sh
podnet bench https://example.com/
podnet --egress socks5h://127.0.0.1:1080 bench https://example.com/ --connections 8 --duration 20 --json
```

* `--connections N` (default 4): the requests in flight, each on its own connection.
* `--duration SECS` (default 10): the time to send requests. A request that started in time ends.
* `--timeout SECS` (default 10): the time limit of one request.
* The text output has the requests, the errors, the rate and the latency (min, p50, p95, p99, max). With
  `--json`, one document: `schema` (1), `podnet`, `url`, `connections`, `duration_s`, `requests`, `ok`,
  `errors`, `first_error`, `bytes`, `rate` and `latency_ms` (`min`, `p50`, `p95`, `p99`, `max`).

## Agent sandbox: `podnet agent`

```sh
podnet agent up --expose 3000      # test, start WARP if needed, expose port 3000
podnet agent status                # add --json for one JSON document: instances and report
podnet agent down                  # stop each part, put back each change
```

`podnet agent up` runs the checks of `podnet doctor --agent`. When direct networking does not do each task
(git over HTTPS and SSH, the registries, UDP DNS), it starts `podnet warp up` (`--persist` adds the shell
block; `--warp` starts it also when direct networking works; `--port PORT` selects the port of the WARP
proxy, default 40000). For each `--expose PORT`, it starts a detached quick tunnel (instance
`expose-PORT`), and tries `--transport relay` when the first try fails. It then runs the checks again and
prints one JSON document: `mode` (`warp` or `direct`), the paths that work for each task, the URLs, and
the commands to use (`eval "$(podnet warp env)"`, `podnet warp exec -- CMD`, `podnet warp ssh-config`).
When WARP was needed and `podnet warp up` failed, the document has `warp_error`, and the exit code is 69.
When an expose failed (its `state` in `exposed` is not `ready`), the exit code is 69 too.
