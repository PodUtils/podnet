# Networks and `--egress`

A network is a way to send a flow out of this host. podnet names each network with a spec: one text in
one form for each command. The same specs are used by `--egress`, `podnet forward --network`,
`podnet pod -x`, `podnet bench` and the `networks` of a router file ([router.md](router.md)).

## The registry

| Scheme | Forms of the spec | What it is | Carries |
| --- | --- | --- | --- |
| `direct` | `direct` | TCP and UDP from this host, with no proxy. | TCP, UDP |
| proxy | `http://[USER:PASS@]HOST[:PORT]`, `socks5://...`, `socks5h://...`, `SCHEME+unix:/PATH`, `SCHEME://unix:/PATH`, `SCHEME+exec:COMMAND`, `unix:/PATH`, `HOST:PORT` | An HTTP CONNECT proxy or a SOCKS5 proxy. | TCP |
| `relay` | `relay`, `relay:URL`, `relay:HOST` | The WebSocket relay (`PODNET_RELAY`, default `https://tcp.ssh.relay.ajam.dev`). IPv4 TCP only. | TCP |
| `tailscale` | `tailscale:/PATH/tailscaled.sock` | A tailnet, through the LocalAPI of `tailscaled` ([networks.md](networks.md)). | TCP |
| `netbird` | `netbird`, `netbird:[PORT]` | The SOCKS5 listener of NetBird in netstack mode on `127.0.0.1:PORT` (default 1080). The target must be an IP address ([networks.md](networks.md)). | TCP |
| `wg` | `wg:FILE` | A WireGuard peer of a wg-quick file, on a userspace stack. A name resolves at the `DNS` of the file, through the tunnel; with no `DNS` line, the target must be an IP address ([networks.md](networks.md)). | TCP, UDP |
| `turn` | `turn://HOST[:PORT]`, `turns://HOST[:PORT]` | An allocation on a TURN server (RFC 8656) for UDP and a TCP allocation (RFC 6062) for TCP, over UDP (`turn`, port 3478) or over TLS on TCP (`turns`, port 5349). `turn://` uses plain TURN over TCP for a TCP stream. The user and the password come from `PODNET_TURN_USER` and `PODNET_TURN_PASSWORD_FILE`, never from the spec ([networks.md](networks.md)). | TCP, UDP |
| `devtunnel` | `devtunnel://NAME`, `devtunnel://ID.CLUSTER` | A stream to a port of the host of a Microsoft Dev Tunnels tunnel, through its relay. The target host is `127.0.0.1`. The token comes from `PODNET_DEVTUNNEL_TOKEN_FILE` or `PODNET_DEVTUNNEL_ACCESS_TOKEN_FILE` (or none with `PODNET_DEVTUNNEL_ANONYMOUS=1`), never from the spec ([networks.md](networks.md)). | TCP |
| `https` | `https://[USER@]HOST[:PORT]` | A forward proxy over TLS: TLS to the proxy, then CONNECT on HTTP/2 or HTTP/1.1 (`via` of `pod-egress`). The credentials come from `PODNET_HTTPS_PROXY_AUTH_FILE` or `PODNET_HTTPS_PROXY_AUTH`, never from the spec ([networks.md](networks.md), "A proxy over TLS"). | TCP |
| `wstunnel` | `wstunnel:wss://HOST[:PORT]`, `wstunnel:ws://HOST[:PORT]`, `wstunnel:HOST[:PORT]` | A server of erebe/wstunnel, 9.1 to 11 (`wstunnel` of `pod-egress`). The path prefix and the credentials come from `PODNET_WSTUNNEL_SECRET_FILE` or `PODNET_WSTUNNEL_SECRET`, never from the spec ([networks.md](networks.md), "wstunnel"). | TCP |
| `vless` | `vless://HOST:PORT?type=ws\|xhttp&security=tls\|none&sni=&host=&path=&mode=` | A VLESS server (Xray-core, sing-box) over a WebSocket or XHTTP (`vless` of `pod-egress`). The spec is the share link with no ID. The ID comes from `PODNET_VLESS_ID_FILE` or `PODNET_VLESS_ID`, never from the spec ([networks.md](networks.md), "VLESS"). | TCP |
| `shadowsocks` | `ss://HOST:PORT` | A Shadowsocks server, in the AEAD form or Shadowsocks 2022, on TCP (`shadowsocks` of `pod-egress`). The key (`METHOD:PASSWORD`, or the `ss://` URI of the server) comes from `PODNET_SHADOWSOCKS_KEY_FILE` or `PODNET_SHADOWSOCKS_KEY`, never from the spec ([networks.md](networks.md), "Shadowsocks"). | TCP |
| `wisp` | `wisp:wss://HOST[:PORT]/PATH`, `wisp:ws://HOST[:PORT]/PATH`, `wisp+wss://HOST[:PORT]/PATH` | Many TCP streams over one WebSocket to a Wisp server, versions 1 and 2 (`wisp` of `pod-egress`). The server takes no credential ([networks.md](networks.md), "Wisp"). | TCP |
| chain | `SPEC over SPEC` | A network over the way of the network below it: `wg:FILE over warp` sends the UDP of WireGuard through a WARP session. The top layer comes first. The chain carries what its top layer carries ([networks.md](networks.md), "A chain of networks"). | as the top layer |
| `peer` | `peer:FINGERPRINT/SERVICE` | One stream to a service of a peer, over a link that a running `podnet link connect` or `link listen` instance holds ([link.md](link.md)). A peer way: it meets a peer and dials no destination. | TCP |

* A spec with no scheme (`HOST:PORT`) is a SOCKS5 proxy, as in `podnet pod -x`.
* `socks5://` resolves the name on this host. `socks5h://` gives the name to the proxy.
* `SCHEME+exec:COMMAND` starts `COMMAND` with `sh -c` for each connection and uses its stdin and stdout
  as the proxy connection (see [proxies.md](proxies.md)).
* An error about a spec shows the spec without its user and password.
* `turn://` and `turns://` carry UDP through a UDP allocation and TCP through a TCP allocation (RFC 6062). Use them with
  `podnet forward -L ... --network SPEC`, `podnet link p2p --relay SPEC`, or a `networks:` entry of a router file.
  `--egress turn://...` gives one line that says that the TLS or TCP connection of the network is not a proxy of the process.
* `devtunnel://` opens streams to ports of the tunnel host and is not a proxy. Use it with `podnet forward -L ... --network SPEC`.
  `--egress devtunnel://...` gives one line that says so.
* `https://HOST[:PORT]` and `wstunnel:URL` open their own streams through a proxy over TLS or a wstunnel server. Use them
  with `podnet forward -L ... --network SPEC`, a `networks:` entry of a router file, or `podnet net doctor --network SPEC`.
  `--egress https://...` and `--egress wstunnel:...` give one line that says that the network is not a proxy of the
  process or a hop of a pod. The proxy of the process does not take `https://` ([proxies.md](proxies.md)).
* `vless://`, `ss://` and `wisp:` open their own streams through a VLESS, a Shadowsocks or a Wisp server. Use them as
  `https://` and `wstunnel:`. `--egress` gives one line that names the network and `--network`.
* `warp` and `warp:identity=FILE` are networks of the registry: TCP and UDP through a WARP session, with names
  resolved inside the tunnel ([router.md](router.md), "WARP as a member"). Use them with
  `podnet forward --network warp` or in a `networks:` entry of a router file. `--egress warp` gives one line
  that says that WARP is not a proxy of the process: use `podnet warp proxy` ([warp.md](warp.md)) or
  `tunnel --transport warp` for that.

## `--egress`

`--egress SPEC` is a flag of each command that opens connections. It selects the way out of the process,
and it replaces the older flags:

| `--egress` | Same as |
| --- | --- |
| `direct` | `--no-proxy` |
| A proxy URL | `--proxy URL` |
| `relay`, `relay:URL` | `--transport relay` and `--relay URL` |

* `--egress` beside `--proxy`, `--no-proxy`, `--transport` or `--relay` is a usage error (exit 64), because
  the two flags name one way.
* With `podnet pod`, each `--egress` is one hop `-x SPEC` of the chain, in its order. The forms
  `tailscale:/PATH`, `netbird:[PORT]` and `unix:/PATH` are hops of a pod. The relay is not a hop of a
  pod.
* `peer:FINGERPRINT/SERVICE` is a peer way. A consumer that dials a destination (`--egress` of the other commands, `podnet forward --network`, `pod -x`)
  refuses it with exit 64. Use `podnet forward -L LISTEN=peer:FINGERPRINT/SERVICE` ([forward.md](forward.md)). A `PORT` after
  `peer:` (`peer:1080`) is still the SOCKS5 hop of a host named `peer`.
* `wg:FILE` is not a hop of a pod. Of the process commands, only `podnet wg serve` takes it in `--egress`; each other
  gives one line. Use `podnet forward --network wg:FILE` or a `networks:` entry of a router file.
* A chain (`TOP over BELOW`) is not a hop of a pod. Of the process commands, only `podnet wg serve` takes it in
  `--egress`; each other gives one line. Use `podnet forward --network 'wg:FILE over warp'` or a `networks:` entry of a router file.
* `tailscale:/PATH` is a hop of a pod and the only hop of its chain. A command that is not `pod` refuses
  `--egress tailscale:/PATH` with one line: use `podnet pod`, `podnet forward --network` or a router file.
* `podnet link dial` and `podnet link connect` take each `--egress` as one element of a chain before the `tcp` carrier, in
  order: the last element reaches the carrier address, and no `direct` is added. The elements are `direct`, a proxy and
  `peer:FINGERPRINT/SERVICE`; at most 8; a duplicate or a cycle exits 78; an element that cannot be built exits 69 and is named.
  `--check` dials each element once ([link.md](link.md), "A chain before the carrier").
* `podnet wg serve` takes one `--egress SPEC`: the way of the TCP and UDP flows of its peers. Each spec of
  `podnet forward --network` is good (a chain too). Its default is the way out of the process
  ([networks.md](networks.md), "WireGuard server").
* A spec that the command cannot use is a usage error with one line that names the spec.
* `podnet forward --network SPEC` selects the network of the forwards ([forward.md](forward.md)). Its default
  is the way out of the process.

```sh
podnet --egress socks5h://127.0.0.1:1080 tunnel --url http://127.0.0.1:8080
podnet --egress relay tunnel --url http://127.0.0.1:8080
podnet --egress direct doctor
podnet pod --egress socks5://proxy-a:1080 --egress tailscale:/run/tailscale/tailscaled.sock -- ssh host
podnet bench --egress socks5h://127.0.0.1:1080 https://example.com/
```

## A proxy over TLS and wstunnel

```sh
export PODNET_HTTPS_PROXY_AUTH_FILE=/etc/podnet/proxy-auth   # one line: USER:PASSWORD
podnet forward -L 127.0.0.1:2222=code.example:22 --network https://proxy.example:443

export PODNET_WSTUNNEL_SECRET_FILE=/etc/podnet/wstunnel-secret   # path-prefix PREFIX
podnet forward -L 127.0.0.1:2222=10.0.0.5:22 --network wstunnel:wss://wst.example
```

Both are roads of `pod-egress` (PodUtils/podcrates); podnet holds no code of them. A spec holds no secret: a
password, a path prefix or a token in a spec is a usage error (exit 64) that does not repeat it. A missing or bad
secret is a usage error that names the variable. `up` proves the reach and the certificate of the server with one
connection, and a fault names the hop (`the proxy`, `the wstunnel server`). Each form, each setting and each fault
is in [networks.md](networks.md).

| Item | Value | Checked |
| --- | --- | --- |
| The protocol of the proxy | CONNECT of RFC 9110 (section 9.3.6) and of RFC 9113 (section 8.5), over TLS | 2026-11-26 |
| The protocol of wstunnel | the WebSocket transport of erebe/wstunnel 9.1 to 11 (its token, its path prefix) | 2026-11-26 |
| The client | `pod-egress` of PodUtils/podcrates (features `via` and `wstunnel`), at the tag of `Cargo.toml` | 2026-11-26 |
| The test servers | stand-ins on the loopback (`src/networks/road_stand.rs`); the runs against wstunnel and Caddy are tests of `pod-egress` | 2026-11-26 |

## VLESS, Shadowsocks and Wisp

```sh
export PODNET_VLESS_ID_FILE=/etc/podnet/vless-id   # the UUID of the user
podnet forward -L 127.0.0.1:2222=code.example:22 --network 'vless://v.example:443?type=ws&security=tls&path=/ws'

export PODNET_SHADOWSOCKS_KEY_FILE=/etc/podnet/ss-key   # one line: METHOD:PASSWORD
podnet forward -L 127.0.0.1:2222=code.example:22 --network ss://ss.example:8388

podnet forward -L 127.0.0.1:2222=code.example:22 --network wisp:wss://w.example/wisp/
```

The three are roads of `pod-egress` (PodUtils/podcrates); podnet holds no code of them. A spec holds no secret: an
ID or a key in a spec is a usage error (exit 64) that does not repeat it. A missing or bad secret is a usage error
that names the variable. `up` proves the reach of the server (and its certificate over TLS); for Wisp it opens the
connection. A fault names the hop (`the VLESS server`, `the Shadowsocks server`, `the Wisp server`). Each form,
each setting and each fault is in [networks.md](networks.md).

| Item | Value | Checked |
| --- | --- | --- |
| The protocol of VLESS | the request and the answer of Xray-core (`proxy/vless/encoding`), over a WebSocket or XHTTP | 2026-11-26 |
| The protocol of Shadowsocks | the AEAD form of shadowsocks.org and SIP022 (Shadowsocks 2022); the URI of SIP002 | 2026-11-26 |
| The protocol of Wisp | the Wisp protocol of Mercury Workshop, versions 1 and 2 (`protocol.md`) | 2026-11-26 |
| The client | `pod-egress` of PodUtils/podcrates (features `vless`, `shadowsocks` and `wisp`), at the tag of `Cargo.toml` | 2026-11-26 |
| The test servers | stand-ins on the loopback (`src/networks/servers_stand.rs`); the published vectors of each protocol are tests of `pod-egress` | 2026-11-26 |

## TURN servers

A TURN server is a third-party service. podnet holds no server and mints no credential. It reads RFC 8656 over UDP or
TLS, and RFC 6062 for a relayed TCP stream.

| Item | Value | Checked |
| --- | --- | --- |
| The protocol | RFC 8656 (TURN), RFC 8489 (STUN), RFC 6062 (TCP relay) | 2026-11-20 |
| The server of the operator | `turn://HOST[:PORT]` or `turns://HOST[:PORT]`, from the deployment | 2026-11-20 |
| The client | `pod-turn` of PodUtils/podcrates, at the tag of `Cargo.toml` | 2026-11-20 |
| The RFC texts | <https://www.rfc-editor.org/rfc/rfc8656>, <https://www.rfc-editor.org/rfc/rfc6062> | 2026-11-20 |
| The test server | a stand-in on the loopback (`src/networks/turn_mock.rs`), and coturn for the tests of `pod-turn` | 2026-11-20 |

The check each time: the RFC numbers are current, the workspace still takes the `pod-turn` release, and the stand-in answers
each request that the client sends. The date is the date of the last reading. It is no promise about the service of another
party: a server of the operator can change its policy or its ports without notice. A failed allocation names the stage
(`allocate`, `refresh`, `permission`, `bind`, `relay`, `send`, `receive`) and the code of the server.

The credential stays out of `argv`, a log line, an error and `--json` (TURN-5). A spec holds no user and no password; a
credential in a spec is refused before any connection, and the message does not repeat it.
