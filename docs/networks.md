# Virtual networks and overlays

This page gives `podnet net`: the virtual network, Tailscale, NetBird, WireGuard and the passwd shim. The design
of the virtual network is in [virtual-network.md](virtual-network.md). The specs `tailscale:`, `netbird:` and
`wg:` are networks of the registry ([egress.md](egress.md)).

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
podnet net doctor --network SPEC          # also the row of one network (repeatable)
podnet net shim --out /tmp/podnet-vnet        # build the LD_PRELOAD shim (--map-loopback: 127.0.0.1 connects go to the stack)
LD_PRELOAD=/tmp/podnet-vnet/libpodnet-vnet.so ./server      # bind 10.66.0.2:8080 works
podnet net proxy --forward 10.66.0.2:8080=unix:/run/app.sock
podnet net switch --listen unix:/tmp/podnet-vnet.switch   # connect stacks
podnet net proxy --switch unix:/tmp/podnet-vnet.switch --address 10.66.0.3
```

`net proxy` is the front door of [proxies.md](proxies.md) over the stack. The door
answers a closed virtual port with 502 (SOCKS5 `0x04`), and an address that nothing owns with 504 after 5 s.

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

`tailscaled` must run with `--tun=userspace-networking`. The door of `net tailscale` is the front door of
[proxies.md](proxies.md) over `tailscaled`. A dial that `tailscaled` does not answer in 10 s gets
SOCKS5 `0x04` or HTTP 504.

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

## WireGuard: `wg:FILE`

```sh
podnet forward --network wg:/etc/wireguard/wg0.conf -L 127.0.0.1:8080=10.9.0.1:80
```

The spec `wg:FILE` makes a WireGuard peer a network. `FILE` is a wg-quick file with one peer
(`PrivateKey`, `Address`, `MTU`; `PublicKey`, `PresharedKey`, `Endpoint`, `Reserved`). podnet does the handshake itself and
runs a userspace stack over the tunnel. It makes no TUN device and needs no capability. The spec gives TCP and
UDP from the `Address` of the file.

* A target name resolves at the first address of the `DNS` line of the file, with DNS over TCP (RFC 7766)
  through the tunnel. The resolver of this host is not used. With no address in `DNS`, the target must be an
  IP address, and a name gives one line that says so. The status of the link names the server in `dns`.
* A `PresharedKey` of the `[Peer]` goes into the handshake (Noise IKpsk2). A value that is not 32 bytes in
  base64 is a usage line (exit 64), as each other key of the file. When the peer has another preshared key, or
  only one end has one, the handshake response does not authenticate: `up` fails at once with one line that
  names `PresharedKey`, and `podnet forward --network` exits 77.
* `up` does the handshake. A path with no UDP gives no answer in 15 s, and the error says that the UDP path
  may be closed.
* The UDP of the tunnel goes out from this host. A way below the network (`Ctx::below`, a layer of a chain)
  can carry it instead. That way must carry UDP (`Dial::datagrams`). The status of the link names it in
  `carrier`. A way that carries no UDP, a proxy for example, is refused with `carries no UDP`.
* `wg:FILE` is a network of `podnet forward --network` and of a `networks:` entry of a router file
  ([router.md](router.md)). It is not a hop of `podnet pod` (T-117): run `podnet route` with the network and
  give its door to `-x`.
* A host with no UDP sends the UDP of the tunnel through a WARP session (MASQUE CONNECT-IP): give the chain
  `wg:FILE over warp`. The `Endpoint` of the file is reached from the far end of WARP.

```sh
podnet forward --network 'wg:/etc/wireguard/wg0.conf over warp' -L 127.0.0.1:8080=10.9.0.1:80
```

## WireGuard server: `podnet wg serve`

```sh
podnet wg serve --config /etc/podnet/wg-server.conf
podnet wg serve --config wg-server.conf --egress socks5h://127.0.0.1:1080 --run-for 600
```

`podnet wg serve` is a WireGuard peer for a list of peers. It makes no TUN device and needs no capability.
The file is a wg-quick file:

```ini
[Interface]
PrivateKey = ...
ListenPort = 51820
Address = 10.9.0.1/24

[Peer]
PublicKey = ...
AllowedIPs = 10.9.0.2/32

[Peer]
PublicKey = ...
PresharedKey = ...
AllowedIPs = 10.9.0.8/29
```

* `ListenPort` is the UDP port on each IPv4 address of the host (0: a port of the system; the first line on
  stdout gives it). `Address` is the IPv4 net of the peers. The server has the first address of the net.
  `MTU` is optional (default 1420).
* Each `[Peer]` has `PublicKey` and `AllowedIPs` inside the net of `Address`, and an optional `PresharedKey`. Two
  peers with one `PublicKey`, an `AllowedIPs` outside the net, or no `ListenPort` is a usage line (exit 64).
  `Endpoint` is not necessary: the server answers the address of the last authenticated message of the peer.
* The handshakes, the cookies under load and the sessions are `Responder` of `pod-wg`. A key that the file does
  not list gets no answer. The server sends no initiation: each rekey comes from the peer.
* A userspace stack ends each TCP and UDP flow of a peer. The server dials each target through the way of
  `--egress SPEC` (each spec of `podnet forward --network`, also a chain; default: the way out of the process).
  A TCP dial that fails answers the SYN of the peer with a RST.
* A packet to a peer goes to the peer whose `AllowedIPs` has the longest prefix that holds its destination. A
  packet of a peer must have a source that routes back to that peer by the same rule, else it is dropped: a peer
  cannot send with the address of another peer, also when their `AllowedIPs` overlap (T-342). The net of `Address`, the loopback and
  the link-local addresses are not targets: a peer does not reach another peer or the loopback of the server.
* Each peer has a share of the flows, so that one peer cannot take the flows of the others (T-345).
  `--flows N` is the TCP flows of the server (held SYNs and open flows; default 2048), `--udp-flows N` its UDP
  flows (default 512). `--peer-flows N` and `--peer-udp-flows N` are the share of one peer. The default share is
  the total divided by the peers, at least 64 TCP flows and 16 UDP flows. A new flow over the share is refused
  (a SYN is dropped, a datagram is dropped) and counted in `share_refused`. The log gets one line for each peer
  in each 10 s, not one line for each refused flow. The first line on stdout gives the shares.
* IPv4 only inside the tunnel. A client of the server can be podnet itself: `wg:FILE` with the public key of the
  server and its address as the `Endpoint`.

## A chain of networks: `TOP over BELOW`

A spec with the word `over` between two specs is a chain. The first spec is the top layer. Each layer comes up
over the way of the layer below it (`Ctx::below`), and the chain carries what its top layer carries.

* `wg:FILE over warp`: the UDP of the WireGuard peer goes through the datagrams of a WARP session.
* `wg:FILE over direct`: the same as `wg:FILE`.
* `turns://HOST over socks5h://PROXY`: the TLS connection to the TURN server goes through the proxy.
* A chain has at most 8 layers. An empty layer and a bad layer are usage errors (exit 64), and the error names
  the layer by its number.
* A layer that cannot go over another way (`warp`, a proxy, the relay) is refused when the chain comes up: the
  error names the layer and says `cannot go over another way`. A layer below another must give a way that dials
  a destination. A layer below `wg:FILE` or `turn://` must carry UDP (`Dial::datagrams`).
* A spec with `+exec:` is one layer, because a command can hold the word `over`.
* A chain is a network of `podnet forward --network`, of a `networks:` entry of a router file and of
  `podnet net doctor --network`. It is not a hop of a pod, and `--egress` of another command refuses it with
  one line. In a shell, put the spec in quotes.

## TURN: `turn://HOST[:PORT]` and `turns://HOST[:PORT]`

```sh
export PODNET_TURN_USER=alice PODNET_TURN_PASSWORD_FILE=/etc/podnet/turn-password
podnet forward -L udp:5353=10.0.0.53:53 --network turn://turn.example:3478
podnet forward -L udp:5353=10.0.0.53:53 --network turns://turn.example:443   # TURN over TLS on TCP 443
```

A host that reaches only a TURN server has no other UDP path. The network gives it one: each UDP socket of the
network makes one allocation on the server (RFC 8656) with `pod-turn`, and sends each datagram to its peer in
ChannelData. podnet has no TURN code of its own.

* `turn://HOST[:PORT]` is TURN over UDP (default port 3478). `turns://HOST[:PORT]` is TURN over TLS on TCP
  (default port 5349). The TLS connection uses the way out of the process (`--proxy`, the variables) and the
  roots of podnet (`--ca-file`). A way below the network (a layer of a chain) carries the UDP socket of
  `turn://`, or the TCP connection of `turns://`.
* The user, the password and the realm are settings, not parts of the spec, because a spec goes into `argv`, into
  logs and into JSON. `PODNET_TURN_USER` is the user. The password is the first line of the file in
  `PODNET_TURN_PASSWORD_FILE`, else the value of `PODNET_TURN_PASSWORD` (give one of them). `PODNET_TURN_REALM`
  is optional: with it, a server of another realm is refused. A spec with a user, a password, a path or a query
  is a usage error (exit 64) that does not repeat the secret. A missing user or password is a usage error too.
  The account is read when the network comes up. One account serves each process.
* `up` makes one allocation and ends it at once. It proves the server, the account and the transport. The
  status of the link has the scheme, the spec, the transport (`udp` or `tls`), the lifetime that the server gave
  and the address that the server saw. It has no user and no password.
* Each datagram socket has its own allocation, and so each UDP flow of `podnet forward` has one. A server can
  limit the allocations of a user (486): the flow that cannot allocate ends with the stage `allocate`. The first
  datagram to a peer binds a channel (ChannelBind, which also gives the permission), and the next ones use it.
  A datagram that the peer sends back comes as ChannelData. The allocation ends when the socket is dropped.
* The client refreshes the allocation, the permissions and the channels on its own timer. A 438 (stale nonce)
  is answered with the fresh nonce.
* A stage that fails ends the socket with an error that starts with the network and the stage: `allocate`,
  `refresh`, `permission`, `bind`, `send` or `receive`. For example
  `turn://turn.example:3478: refresh: TURN: the server answered the Refresh request with 437`. The socket does
  not try again. The flow ends, and the next datagram of its client starts a new flow with a new allocation.
  A datagram for a full queue is dropped (UDP has no flow control).
* TCP is not carried. A TCP target gives one line that names RFC 6062 (the TCP relay), which `pod-turn` does not
  have. `--egress turn://...` and `podnet pod -x` refuse the network by name.
* `turn://` and `turns://` are networks of `podnet forward --network` and of a `networks:` entry of a router
  file ([router.md](router.md)).
* `podnet net doctor --network turn://HOST:PORT` adds the row of the network: `ok` when the allocation comes up,
  `fail` with the stage (for example `allocate` and the code 401) when it does not. The row holds no password.
* Not in the network: NAT traversal between two peers, TURN over plain TCP or DTLS, and the credentials of a named
  service.

## Dev Tunnels: `devtunnel://TUNNEL`

```sh
export PODNET_DEVTUNNEL_TOKEN_FILE=/etc/podnet/devtunnel-token   # the first line is the token
podnet forward -L 127.0.0.1:5432=127.0.0.1:5432 --network devtunnel://lab
podnet forward -L 127.0.0.1:8080=127.0.0.1:80 --network devtunnel://bcd123.usw2
```

A host that reaches only TCP 443 can still reach a port of a Microsoft Dev Tunnels tunnel host through the relay
of the service. The network opens one `direct-tcpip` channel of an SSH session for each stream. The SSH session
runs over the relay WebSocket. podnet has no Dev Tunnels code of its own: `pod-devtunnel` reads the connect
information from the management API, opens the relay WebSocket, runs the SSH session with no user
authentication, and opens the channels.

* `devtunnel://NAME` or `devtunnel://ID.CLUSTER` names the tunnel (lower case; each part is a label of a host
  name). A spec with a user, a path, a query or a port is a usage error (exit 64) that does not repeat the
  secret.
* The port of the target is the port on the tunnel host. The host of the target is the local address of the
  tunnel host: `127.0.0.1`, `::1` or `localhost`. Another host is refused by name.
* The management token is a setting, not a part of the spec, because a spec goes into `argv`, into logs and into
  JSON. `PODNET_DEVTUNNEL_TOKEN_FILE` is a file whose first line is the token. `PODNET_DEVTUNNEL_TOKEN` is the
  token itself (give one of them). `PODNET_DEVTUNNEL_SCHEME` is `github` (default), `aad` or `bearer`. A missing
  token is a usage error. The token is read when the network comes up. The connect token comes from the connect
  information for each attempt, inside the crate. Neither token is in a log line, an error or `--json`.
* The management host, the API version and the SSH facts are facts of `pod-services` (`microsoft-devtunnel`).
  The relay URI comes from the connect information. `PODNET_DEVTUNNEL_MANAGEMENT` replaces the base of the
  management API (`https://HOST[:PORT]`, or `http://` to a loopback host, for a test).
* The management request and the relay WebSocket use the way out of the process (`--proxy`, the variables) and
  the roots of podnet (`--ca-file`). A way below the network carries them.
* `up` reads the connect information and opens the first relay session. A fault names the step: the management
  API (a refused token, an unknown tunnel), the relay WebSocket, the SSH session or the channel. The status of
  the link has the scheme, the spec, the tunnel ids and the session count.
* A relay session that ends ends each open stream with an error of the kind `ConnectionAborted` ("the relay
  session of the stream ended"). The next stream connects again, within a bounded number of tries, each with a
  fresh connect token. After the last try the error names the reconnect step.
* TCP only. The network has no host side (it does not publish a port). `--egress devtunnel://...` and
  `podnet pod -x` refuse it by name: it is not a proxy.
* Tests use the stand-in of `pod-devtunnel` on the loopback (`tests/devtunnel_network.rs`,
  `src/networks/devtunnel_tests.rs`). They need no account.
* `podnet net doctor --network devtunnel://TUNNEL` adds the row of the network: `ok` when the network comes up,
  `fail` with the stage (for example the Dev Tunnels management API) when it does not. The row holds no token.
* Not in the network: the creation and the deletion of tunnels, the device-code login, and a token store.

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
