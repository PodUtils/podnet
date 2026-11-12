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
* `--network SPEC` selects the network (see [egress.md](egress.md)): `direct`, a proxy URL, `relay` or
  `relay:URL`, `tailscale:/PATH`, `unix:/PATH`. The default is the way out of the process: `--proxy`, the
  proxy variables, else direct.
* UDP: `-L udp:PORT=HOST:PORT` (or `udp:ADDR:PORT`) carries UDP over the datagrams of the network. Each
  client has its own flow. A flow ends after 60 s with no datagram. A datagram for a full flow is dropped.
* `--run-for SECONDS` stops after that time.

### DNS door

`--dns ADDR:PORT` answers DNS queries (UDP and TCP) on `ADDR:PORT`. podnet sends each query through the
network to the resolver `--dns-upstream HOST:PORT` (default `1.1.1.1:53`), which is an address at the far
end of the network. A program that uses the door as its resolver sends no query from this host.

A door of the router has the same function with the keys `dns` and `dns_upstream` ([router.md](router.md)).

### `warp forward`

`podnet warp forward -L LISTEN=HOST:PORT` forwards through WARP ([warp.md](warp.md)).
`podnet warp forward --list` lists the forwards of the detached instances. `podnet warp forward` is not
an alias of `podnet forward`: it has the options of `podnet warp`.

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

* `TARGET` is an `http://` or `https://` URL, a port, `host:port`, `tcp://host:port` or `unix:/path`.
* `--as VIRT` (with `--via switch`) gives the virtual address of the service, as `VIRT` in
  `podnet net proxy --forward VIRT=REAL`, for example `10.66.0.2:8080`.
* The arguments after `--` go to the command that runs: for example `--detach`, a pair name or a
  `--switch`.

```sh
podnet expose 3000                                  # the same as podnet tunnel --port 3000
podnet expose 127.0.0.1:22 --via relay -- --detach
podnet expose unix:/run/app.sock --via switch --as 10.66.0.2:8080 -- --switch unix:/tmp/podnet-vnet.switch
```
