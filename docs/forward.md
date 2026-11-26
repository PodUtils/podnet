# Forwards, DNS door and `expose`

This page gives `podnet forward` (local ports that go out through a network), its DNS door, and
`podnet expose` (one word to publish a local service).

## `podnet forward`

`podnet forward -L LISTEN=HOST:PORT` listens on `LISTEN`. Each connection to it goes through a network to
`HOST:PORT`, as `ssh -L` does. No root and no TUN device are necessary.

```sh
podnet forward -L 5432=db.internal:5432                          # TCP, through the way out of the process
podnet forward --network socks5h://127.0.0.1:1080 -L 8443=example.com:443
podnet forward --network tailscale:/run/tailscale/tailscaled.sock -L 2222=host:22
podnet forward --network relay -L 2222=203.0.113.7:22
podnet forward -L udp:5353=10.0.0.53:53 --network socks5h://127.0.0.1:1080   # UDP
podnet forward --dns 127.0.0.1:5353 --dns-upstream 10.0.0.53:53              # a DNS door
podnet forward -L unix:/run/db.sock=db.internal:5432 --run-for 3600
```

* `-L, --local LISTEN=HOST:PORT` is repeatable. `LISTEN` is `PORT` (on 127.0.0.1), `ADDR:PORT`,
  `tcp://ADDR:PORT` or `unix:/PATH`.
* `-L LISTEN=peer:FINGERPRINT/SERVICE` opens one stream of a link to a peer for each accepted connection, and asks the peer
  for its service (see "A service of a peer" below). It carries TCP only.
* `--network SPEC` selects the network (see [egress.md](egress.md)): `direct`, a proxy URL, `relay` or
  `relay:URL`, `tailscale:/PATH`, `netbird:[PORT]`, `wg:FILE`, `devtunnel://TUNNEL` (TCP only), `https://HOST[:PORT]` (a proxy over TLS, TCP only),
  `wstunnel:wss://HOST[:PORT]` (TCP only), `turn://HOST[:PORT]` or `turns://HOST[:PORT]` (UDP
  only), `unix:/PATH`, or a chain `TOP over BELOW` (`'wg:FILE over warp'`). The default is the way out of the process: `--proxy`, the
  proxy variables, else direct.
* `devtunnel://TUNNEL` carries a TCP forward through a Microsoft Dev Tunnels relay to a port of the tunnel host:
  `-L 127.0.0.1:5432=127.0.0.1:5432 --network devtunnel://lab`. The target host is the local address of the tunnel
  host. The token is in `PODNET_DEVTUNNEL_TOKEN_FILE` or `PODNET_DEVTUNNEL_ACCESS_TOKEN_FILE`, or none with
  `PODNET_DEVTUNNEL_ANONYMOUS=1` ([networks.md](networks.md)).
* UDP: `-L udp:PORT=HOST:PORT` (or `udp:ADDR:PORT`) carries UDP over the datagrams of the network. Each
  client has its own flow. A flow ends after 60 s with no datagram. A datagram for a full flow is dropped.
  A SOCKS5 or HTTP proxy network carries no UDP: the forward logs the refusal and ends the flow. `direct`
  and `wg:FILE` carry UDP. `turn://HOST[:PORT]` and `turns://HOST[:PORT]` carry UDP through an allocation on a
  TURN server, for a host that reaches only that server ([networks.md](networks.md)): the account is in
  `PODNET_TURN_USER` and `PODNET_TURN_PASSWORD_FILE`, and a TCP forward over them is refused. A name in the target of a UDP forward resolves with the resolver of this host.
  For UDP through a door that clients choose (SOCKS5 UDP ASSOCIATE) and for names that resolve inside a
  network, use the router ([router.md](router.md), "Doors" and `resolvers`).
* `--run-for SECONDS` stops after that time.

### Names for a client that takes only HOST:PORT

```sh
podnet forward --network relay --alias-port 7000 \
  --name win.internal=win.internal:3389 --name db.internal=db.internal:5432 \
  --dns 127.0.0.1:5353
```

* `--name NAME=HOST:PORT` (repeatable, 254 at most) gives the name the next address of `127.0.0.2` to `127.0.0.255`
  and a forward from that address to `HOST:PORT`. The listener takes the `PORT` of the name, or `--alias-port PORT` for
  each name. The client connects to `NAME:PORT`. `--alias-port` needs `--name`. The whole of `127.0.0.0/8` is the
  loopback of Linux: no interface is added.
* With `--dns ADDR:PORT`, the DNS door answers each name with its address (type A, on UDP and on TCP) and sends each
  other name to `--dns-upstream` through the network. With no `--dns`, the lines `podnet: name ADDRESS NAME` are the
  lines of a hosts file.
* A name is letters, digits, `-` and `.`, in lowercase; each name once (else exit 64) (T-404).

### A service of a peer

`-L LISTEN=peer:FINGERPRINT/SERVICE` reaches a service of a peer through a link ([link.md](link.md)):

```sh
podnet link connect /run/user/1000/app.sock --peer FINGERPRINT --control /run/user/1000/link.ctl --detach --name peer1
export PODNET_LINK_CONTROL=/run/user/1000/link.ctl            # not needed when the instance is in the run directory
podnet forward -L 8080=peer:FINGERPRINT/http                  # 127.0.0.1:8080 -> the service http of the peer
podnet forward -L unix:/run/db.sock=peer:FINGERPRINT/db       # a unix listener
```

* A link instance (`podnet link connect`, or `link listen` for the links it accepts) holds the link and runs a control socket
  ([link.md](link.md)). The forward is another process: it asks that socket to open a stream. `PODNET_LINK_CONTROL` names one
  socket. Without it, the forward asks the control socket of each link instance in the run directory.
* The check runs before the forward listens. A link to a peer that is not the named `FINGERPRINT` exits 77, and no stream opens.
  No link instance that runs, or one with no link up, exits 69.
* Each accepted connection opens one new stream, and the first line of the stream names the service. The listener of the far side
  maps the name to a target (`link listen --service NAME=TARGET`, or `--to TARGET` for `raw`). A service that the peer does not
  have, or whose origin does not answer, closes that connection. The forward and the link stay up.
* `--network peer:...` is refused with exit 64: a peer way takes the form `-L LISTEN=peer:FINGERPRINT/SERVICE`.
* The services that a peer offers (`raw`, `socks5`, `http`, `exec:` and the others) are in [link.md](link.md).

### DNS door

`--dns ADDR:PORT` answers DNS queries (UDP and TCP) on `ADDR:PORT`. podnet sends each query through the
network to the resolver `--dns-upstream HOST:PORT` (default `1.1.1.1:53`), which is an address at the far
end of the network. A program that uses the door as its resolver sends no query from this host.

* podnet binds the door first. The line `podnet: dns door ADDR:PORT -> UPSTREAM via NETWORK` comes after the bind,
  with the bound port (`--dns 127.0.0.1:0` shows the port that the system gave). An address in use prints no line and
  exits 69.
* A query that the door cannot send to the upstream (the network carries no UDP, or the dial fails) gets the answer
  SERVFAIL, not a timeout. The log has the cause (a line of the level WARN). A query that goes out, and that gets no
  answer, still times out at the client.

A door of the router has the same function with the keys `dns` and `dns_upstream` ([router.md](router.md)).

### `warp forward`

`podnet warp forward -L LISTEN=HOST:PORT` forwards through WARP ([warp.md](warp.md)).
`podnet warp forward --list` lists the forwards of the detached instances. `podnet warp forward` is not
an alias of `podnet forward`: it has the options of `podnet warp`. `podnet forward --network warp -L ...`
does the same work with the network `warp` of the registry (TCP, `-L udp:...` and `--dns` through the
session; the device is kept as `podnet warp` keeps it, or in `warp:identity=FILE`). Both use the session and the
dial of `pod-warp`.

## `podnet connect`

`podnet connect URL -L LISTEN` forwards local sockets through the `--forward` endpoint of a quick tunnel
(see [tunnels.md](tunnels.md), "Raw streams through a quick tunnel").

## `podnet expose`

`podnet expose TARGET` publishes a local service. It runs another command for the way that `--via` selects:

| `--via` | Command that runs | Result |
| --- | --- | --- |
| `tunnel` (default) | `podnet tunnel` | A public URL on a Cloudflare tunnel ([tunnels.md](tunnels.md)). |
| `relay` | `podnet relay node` | A node of the reverse relay: operators connect with `podnet relay connect` ([relay.md](relay.md)). |
| `switch` | `podnet net proxy --forward` | A virtual address on a switch of the virtual network ([networks.md](networks.md)). |

* `TARGET` with `--via tunnel` is an `http://` or `https://` URL, a port, `host:port`, `tcp://host:port` or
  `unix:/path`. A URL is an HTTP origin (`tunnel --url`). A port is the HTTP origin `127.0.0.1:PORT`
  (`tunnel --port`). A bare `host:port` is a raw TCP origin (`tunnel --tcp`, for a named tunnel: a quick tunnel
  answers an HTTP request to it with 501; see [tunnels.md](tunnels.md)). Give `http://host:port` for an HTTP
  origin. `tcp://` and `unix:` go to `tunnel --forward` (a client uses `podnet connect`).
* `TARGET` with `--via relay` and `--via switch` is a port, `host:port`, `tcp://host:port` or `unix:/path`: these
  roads carry TCP. A URL exits 64, and the message names the `HOST:PORT` to give (`http://127.0.0.1:8081/` gives
  `127.0.0.1:8081`).
* `--as VIRT` (with `--via switch`) gives the virtual address of the service, as `VIRT` in
  `podnet net proxy --forward VIRT=REAL`, for example `10.66.0.2:8080`. With a `--switch` after `--`, the IP of
  `VIRT` is also the address of the stack on the switch (`--address`), unless an `--address` after `--` gives
  another one.
* The arguments after `--` go to the command that runs: for example `--detach`, a pair name or a
  `--switch`.

```sh
podnet expose 3000                                  # the same as podnet tunnel --port 3000
podnet expose 127.0.0.1:22 --via relay -- --detach
podnet expose unix:/run/app.sock --via switch --as 10.66.0.2:8080 -- --switch unix:/tmp/podnet-vnet.switch
```
