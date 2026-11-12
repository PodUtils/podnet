# Virtual networks and overlays

This page gives `podnet net`: the virtual network, Tailscale, NetBird and the passwd shim. The design
of the virtual network is in [virtual-network.md](virtual-network.md). The specs `tailscale:` and
`netbird:` are networks of the registry ([egress.md](egress.md)).

| Command | Function |
| --- | --- |
| `net demo` | Prove a userspace TCP handshake and a transfer in the process. |
| `net doctor` | Probe what this host permits ([doctor.md](doctor.md)). |
| `net addresses` | Print the default virtual address plan. |
| `net shim` | Build or find the `LD_PRELOAD` shim and print how to use it. |
| `net proxy` | Run a SOCKS5 and HTTP CONNECT proxy into the virtual network. |
| `net switch` | Run a switch that connects stacks on this host and on other hosts. |
| `net tailscale` | A SOCKS5 and HTTP CONNECT front door to a tailnet through `tailscaled`. |
| `net socksify` | Build the shim that sends tailnet connects to a front door. |
| `net passwd-shim`, `net ts-shims` | Files for a user with no passwd entry. |

## Virtual network: `podnet net`

`podnet net` gives programs IP addresses on a host that refuses `AF_INET` binds.
It needs no namespace, no TUN device and no capability.

```sh
podnet net doctor                         # what this host permits
podnet net shim --out /tmp/podnet-vnet        # build the LD_PRELOAD shim
LD_PRELOAD=/tmp/podnet-vnet/libpodnet-vnet.so ./server      # bind 10.66.0.2:8080 works
podnet net proxy --forward 10.66.0.2:8080=unix:/run/app.sock
podnet net switch --listen unix:/tmp/podnet-vnet.switch   # connect stacks
podnet net proxy --switch unix:/tmp/podnet-vnet.switch --address 10.66.0.3
```

A unix switch takes the same user only. A TCP switch needs a shared token:
set `PODNET_SWITCH_TOKEN`, or give `--switch-token-file PATH`, to the switch
and to each `net proxy --switch tcp://...`. With no token, the switch does
not start (exit 64). A peer with no token or a wrong one is closed. The
switch drops a packet whose source address is of another peer.

```sh
export PODNET_SWITCH_TOKEN="$(head -c 24 /dev/urandom | base64)"
podnet net switch --listen tcp://0.0.0.0:7000
podnet net proxy --switch tcp://switch-host:7000 --address 10.66.0.3
```

`podnet net doctor` calls `bind` and then `listen` for each stream socket
(a policy can allow one and refuse the other; `SO_ACCEPTCONN` confirms the
listen), and sends one QUIC packet to the edge on UDP 7844: answered, refused
or `?` (no answer in 3 s). It prints the commands that need each listener.
`PODNET_UDP=off` makes a tunnel use no QUIC on UDP 7844; `PODNET_UDP=on` skips
the probe at start.

See [virtual-network.md](virtual-network.md).

## Tailscale on a sealed host

```sh
podnet net tailscale --socket /run/tailscale/tailscaled.sock   # SOCKS5 front door to the tailnet
eval "$(podnet net socksify --socket /tmp/podnet-socks.sock)"     # shim for tailnet connects
podnet net ts-shims --out /run/podnet-ts-shims                   # user tables for Tailscale SSH
podnet pod -x tailscale:/run/tailscale/tailscaled.sock -- ssh host
```

`tailscaled` must run with `--tun=userspace-networking`.

`--tailnet NAME` (or `PODNET_TAILNET`) starts the door only when `tailscaled`
is `Running` in that tailnet. NAME is the tailnet name or its DNS suffix.
Another tailnet exits 78 and binds nothing. A daemon that is not logged in
after 20 s exits 69. While the door runs, it refuses new dials when the daemon
changes to another tailnet (SOCKS5 "connection not allowed", HTTP 403). Open
connections stay. `podnet net doctor [--socket PATH]` shows the state of the daemon: `ok` when it is
`Running`, `fail` when it is `NeedsLogin`, `Stopped` or another state, and `unknown` when the socket does not
answer.

The spec `tailscale:/PATH` is a network of the registry ([egress.md](egress.md)). It is a hop of `podnet pod -x`
and `--egress`, the `--network` of `podnet forward`, a network of a router file and a member of a group
([router.md](router.md)). It is the only hop of a chain. `podnet route --check` prints its login state. A
router that starts with a daemon that is not `Running` prints one warning, and the link of the network has the
`state` in its status. The name of the target goes to `tailscaled` (MagicDNS), so a name or an address of the tailnet works.

A host whose outbound traffic is limited to port 443 starts `tailscaled`
with `TS_FORCE_NOISE_443=true`. Without it, the control connection tries port
80 first, and a middlebox can accept that connection and stall.

`net socksify --cidr ROUTE` routes only the listed destinations to the door
(repeatable: `tailnet`, `ADDR`, `ADDR/BITS`, `IPV4:PORT` or `[IPV6]:PORT`),
for example a client with the fixed origin `127.0.0.1:3000`. `net shim
--socks-route ROUTE` does the same in the shim of the virtual network. See
[virtual-network.md](virtual-network.md),
section 5.5.

## NetBird in netstack mode

NetBird joins a host with no TUN device in netstack mode. The daemon then
gives a SOCKS5 listener on `127.0.0.1`.

```sh
NB_USE_NETSTACK_MODE=true NB_SOCKS5_LISTENER_PORT=1080 NB_SETUP_KEY=... netbird up -F
podnet pod -x netbird:1080 -- ssh 100.64.0.7        # the peer by its NetBird IP address
podnet net doctor                                    # the row "NetBird (SOCKS5 listener)"
```

The network spec is `netbird:[PORT]` (default 1080). It dials the SOCKS5
listener at `127.0.0.1:PORT`. podnet does not start the daemon. The spec
gives TCP only. It takes the IP address of a peer. A host name fails with one
line, because netstack mode has no DNS. The listener has no authentication:
keep it on `127.0.0.1`. The doctor row sends a SOCKS5 greeting to the port
(`NB_SOCKS5_LISTENER_PORT`, else 1080). A listener that does not answer is
`unknown`, not a fault.

## A user with no passwd entry

On a host where `/etc/passwd` has no entry for your uid, `getpwuid` fails.
Then `ssh` stops, and `ssh-keygen` exits 0 and writes no key. `podnet net
doctor` shows the row `passwd entry for this uid`. `podnet net passwd-shim`
writes a passwd file and the preload `fakepwd.so`, and prints `LD_PRELOAD`
(before an old value) and `SANDHOME_PASSWD`:

```sh
eval "$(podnet net passwd-shim)"
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519 -N ''
```

`--user`, `--uid`, `--gid` and `--shell` set the entry. `--json` prints the
environment as one object. `net ts-shims` writes the same files, with
`getent` and `id`, and prints the hints for `tailscaled`.
