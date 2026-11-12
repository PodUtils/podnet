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
| `wg` | `wg:FILE` | A WireGuard peer of a wg-quick file, on a userspace stack. The target must be an IP address ([networks.md](networks.md)). | TCP, UDP |
| `turn` | `turn://HOST[:PORT]`, `turns://HOST[:PORT]` | An allocation on a TURN server (RFC 8656), over UDP (`turn`, port 3478) or over TLS on TCP (`turns`, port 5349). The user and the password come from `PODNET_TURN_USER` and `PODNET_TURN_PASSWORD_FILE`, never from the spec ([networks.md](networks.md)). | UDP |
| `peer` | `peer:FINGERPRINT/SERVICE` | One stream to a service of a peer, over a link that a running `podnet link connect` or `link listen` instance holds ([link.md](link.md)). A peer way: it meets a peer and dials no destination. | TCP |

* A spec with no scheme (`HOST:PORT`) is a SOCKS5 proxy, as in `podnet pod -x`.
* `socks5://` resolves the name on this host. `socks5h://` gives the name to the proxy.
* `SCHEME+exec:COMMAND` starts `COMMAND` with `sh -c` for each connection and uses its stdin and stdout
  as the proxy connection (see [proxies.md](proxies.md)).
* An error about a spec shows the spec without its user and password.
* `turn://` and `turns://` carry UDP only. A TCP target is refused by name (the TCP relay of RFC 6062 is not
  in podnet). Use them with `podnet forward -L udp:... --network SPEC` or in a `networks:` entry of a router
  file. `--egress` is a way for TCP: `--egress turn://...` gives one line that says so.
* `warp` is not a network of the registry. Use `podnet warp proxy` ([warp.md](warp.md)) or
  `tunnel --transport warp`.

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
* `peer:FINGERPRINT/SERVICE` is a peer way. A consumer that dials a destination (`--egress`, `podnet forward --network`, `pod -x`)
  refuses it with exit 64. Use `podnet forward -L LISTEN=peer:FINGERPRINT/SERVICE` ([forward.md](forward.md)). A `PORT` after
  `peer:` (`peer:1080`) is still the SOCKS5 hop of a host named `peer`.
* `wg:FILE` is not a hop of a pod, and no process command takes it in `--egress`: each gives one line.
  Use `podnet forward --network wg:FILE` or a `networks:` entry of a router file.
* `tailscale:/PATH` is a hop of a pod and the only hop of its chain. A command that is not `pod` refuses
  `--egress tailscale:/PATH` with one line: use `podnet pod`, `podnet forward --network` or a router file.
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
