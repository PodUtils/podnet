# Virtual networks and overlays

This page gives `podnet net`: the virtual network, Tailscale, NetBird, WireGuard and the passwd shim. The design
of the virtual network is in [virtual-network.md](virtual-network.md). The specs `tailscale:`, `netbird:`,
`wg:`, `turn://`, `devtunnel://`, `https://`, `wstunnel:`, `vless://`, `ss://` and `wisp:` are networks of the
registry ([egress.md](egress.md)).

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
podnet net shim --out "$HOME/.local/share/podnet-vnet"   # build the LD_PRELOAD shim (--map-loopback: 127.0.0.1 connects go to the stack)
LD_PRELOAD="$HOME/.local/share/podnet-vnet/libpodnet-vnet.so" ./server   # bind 10.66.0.2:8080 works
podnet net proxy --forward 10.66.0.2:8080=unix:/run/app.sock
podnet net switch --listen unix:/tmp/podnet-vnet.switch   # connect stacks
podnet net proxy --switch unix:/tmp/podnet-vnet.switch --address 10.66.0.3
```

`net proxy` is the front door of [proxies.md](proxies.md) over the stack. The door
answers a closed virtual port with 502 (SOCKS5 `0x05`, connection refused), and an address that nothing owns with 504
(SOCKS5 `0x04`) after 5 s.

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
or `?` (no answer in 3 s). It connects to a TCP listener on 127.0.0.1 and to a
unix socket (the rows `connect(AF_INET, 127.0.0.1)` and `connect(AF_UNIX, path)`):
a host can allow a bind and a listen on the loopback and refuse each connect to
it (no address on `lo`). Then the last line says `TCP on the loopback does not
work here: use unix sockets`. It prints the commands that need each listener.

A shim must load from a file system that allows exec. With no `--out`,
`net shim`, `net socksify`, `net ts-shims` and `net passwd-shim` take the first
directory that is writable and not `noexec`. An `--out` on a `noexec` file
system is an error (exit 78) that names the directory that podnet would use.
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
* The file is 1 MiB at most (a larger file is a usage line, exit 64). When other users can read the file, podnet
  writes a warning: it holds the private key, so set its mode to 0600.
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
  `MTU` is optional (576 to 9000; default 1420). The file is 1 MiB at most, as for `wg:FILE`, and holds the
  private key: set its mode to 0600.
* Each `[Peer]` has `PublicKey` and `AllowedIPs` inside the net of `Address`, and an optional `PresharedKey`. Two
  peers with one `PublicKey`, an `AllowedIPs` outside the net, or no `ListenPort` is a usage line (exit 64).
  `Endpoint` is not necessary: the server answers the address of the last authenticated message of the peer.
* The handshakes, the cookies under load and the sessions are `Responder` of `pod-wg`. Above 64 initiations in one
  second, the server is under load: each initiation then needs the `mac2` of a cookie. A key that the file does
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
  flows (default 512), each 1 to 65536. `--peer-flows N` and `--peer-udp-flows N` are the share of one peer (1 to
  the total; another value exits 64). The default share is
  the total divided by the peers, at least 64 TCP flows and 16 UDP flows, and not more than the total. A new flow over the share is refused
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
* A chain is a network of `podnet forward --network`, of a `networks:` entry of a router file, of
  `podnet net doctor --network` and of `podnet wg serve --egress`. It is not a hop of a pod, and `--egress` of another command refuses it with
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
  The account is read when the network comes up. A network `NAME` of a router file (`networks:`) reads the
  variables with the suffix `_NAME` (the name in capitals, `-` as `_`) when `PODNET_TURN_USER_NAME` is set:
  `PODNET_TURN_USER_OFFICE_4G` and `PODNET_TURN_PASSWORD_FILE_OFFICE_4G` for the network `office-4g`, and the same for
  the realm, the lifetime and the rate limit. Else it reads the variables with no suffix, the account of the process.
* `up` makes one allocation and ends it at once. It proves the server, the account and the transport. The
  status of the link has the scheme, the spec, the transport (`udp` or `tls`), the lifetime that the server gave
  and the address that the server saw. It has no user and no password.
* Each datagram socket has its own allocation, and so each UDP flow of `podnet forward` has one. A server can
  limit the allocations of a user (486): the flow that cannot allocate ends with the stage `allocate`. The first
  datagram to a peer binds a channel (ChannelBind, which also gives the permission), and the next ones use it.
  A datagram that the peer sends back comes as ChannelData. The allocation ends when the socket is dropped.
* The client refreshes the allocation, the permissions and the channels on its own timer. A 438 (stale nonce)
  is answered with the fresh nonce.
* A TCP connection of the network (`podnet forward -L PORT=HOST:PORT --network turns://...`, a way of `route`) goes
  through a TCP allocation of RFC 6062 (`Connect`, then `ConnectionBind` on a second connection to the server), with
  the bare bytes of each direction. The allocation lives while the stream lives. The status of the link has
  `tcp_relay: rfc6062`. A server with no RFC 6062 refuses the allocation, and the dial ends with the stage
  `allocate`.
* A stage that fails ends the socket with an error that starts with the network and the stage: `allocate`,
  `refresh`, `permission`, `bind`, `send` or `receive`. For example
  `turn://turn.example:3478: refresh: TURN: the server answered the Refresh request with 437`. The socket does
  not try again. The flow ends, and the next datagram of its client starts a new flow with a new allocation.
  A datagram for a full queue is dropped (UDP has no flow control).
* `--egress turn://...` and `podnet pod -x` refuse the network by name: it makes an allocation for each flow, and it
  is not a proxy of the process.
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

# No account: a tunnel access token that the owner of the tunnel gives
export PODNET_DEVTUNNEL_ACCESS_TOKEN_FILE=/etc/podnet/devtunnel-access   # the first line is the token
podnet forward -L 127.0.0.1:5432=127.0.0.1:5432 --network devtunnel://lab

# No token: a tunnel with anonymous access
PODNET_DEVTUNNEL_ANONYMOUS=1 podnet forward -L 127.0.0.1:8080=127.0.0.1:80 --network devtunnel://lab
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
* The credential is a setting, not a part of the spec, because a spec goes into `argv`, into logs and into
  JSON. Give one of three forms:
  * A user token (the management token): `PODNET_DEVTUNNEL_TOKEN_FILE` is a file whose first line is the token.
    `PODNET_DEVTUNNEL_TOKEN` is the token itself (give one of them). `PODNET_DEVTUNNEL_SCHEME` is `github`
    (default), `aad` or `bearer`.
  * A tunnel access token that the owner of the tunnel gives (`devtunnel token TUNNEL --scopes connect`):
    `PODNET_DEVTUNNEL_ACCESS_TOKEN_FILE` is a file whose first line is the token. `PODNET_DEVTUNNEL_ACCESS_TOKEN`
    is the token itself (give one of them). The client needs no account.
  * No token, for a tunnel with anonymous access: `PODNET_DEVTUNNEL_ANONYMOUS=1`. podnet sends no token only when
    you set this variable.

  Each form goes to the management API, so no form needs another. More than one form is a usage error (exit 64)
  that names the variables and not the values. `PODNET_DEVTUNNEL_SCHEME` with no user token is a usage error. No
  form is a usage error that names each setting. A refused token or a refused anonymous caller fails at the
  management API with the class `refused`; the error names the form of the credential, never the token. The
  credential is read when the network comes up. The connect token comes from the connect
  information for each attempt, inside the crate. Neither token is in a log line, an error or `--json`.
* The management host, the API version and the SSH facts are facts of `pod-services` (`microsoft-devtunnel`).
  The relay URI comes from the connect information. `PODNET_DEVTUNNEL_MANAGEMENT` replaces the base of the
  management API (`https://HOST[:PORT]`, or `http://` to a loopback host, for a test).
* The management request and the relay WebSocket use the way out of the process (`--proxy`, the variables) and
  the roots of podnet (`--ca-file`). A way below the network carries them.
* `up` reads the connect information and opens the first relay session. A fault names the step: the management
  API (a refused token, an unknown tunnel), the relay WebSocket, the SSH session or the channel. The status of
  the link has the scheme, the spec, the tunnel ids, the form of the credential and the session count.
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

## A proxy over TLS: `https://[USER@]HOST[:PORT]`

```sh
export PODNET_HTTPS_PROXY_AUTH_FILE=/etc/podnet/proxy-auth   # one line: USER:PASSWORD
podnet forward -L 127.0.0.1:2222=code.example:22 --network https://proxy.example:443
podnet net doctor --network https://proxy.example:443
```

A host whose proxy takes only TLS (Caddy's forwardproxy, the `https_port` of squid, h2o) can use the proxy as a
network. Each stream opens TLS to the proxy, then a CONNECT inside the TLS: HTTP/2 when the proxy chooses `h2`
by ALPN (the streams share one connection), else HTTP/1.1 (one connection for each stream). podnet has no code
of this road: it is `via` of `pod-egress` (PodUtils/podcrates).

* The default port is 443. A password, a path or a query in the URL is a usage error (exit 64) that does not
  repeat the secret. `https://USER@HOST` names the user, and the file then holds the password alone.
* The credentials are a setting, not a part of the spec. `PODNET_HTTPS_PROXY_AUTH_FILE` is a file whose one line
  is `USER:PASSWORD`. `PODNET_HTTPS_PROXY_AUTH` is the same line (give one of them; the file is the better
  choice). With neither, no `Proxy-Authorization` goes. A URL that names a user and no password, two forms, a file
  that cannot be read and a bad line are usage errors (exit 64) that name the variable and not the value.
* `PODNET_HTTPS_PROXY_VERSION` is the versions of HTTP that the client offers: `auto` (default), `1.1` or `2`.
* A network `NAME` of a router file reads the variables with the suffix `_NAME` (the name in capitals, `-` as `_`)
  when one of them is set: `PODNET_HTTPS_PROXY_AUTH_FILE_OFFICE` for the network `office`.
* The TCP connection to the proxy uses the way out of the process (`--proxy`, the variables), or the way below
  the network (`'https://HOST over socks5h://127.0.0.1:1080'`). The certificate of the proxy is checked with the
  roots of podnet (`--ca-file`, `SSL_CERT_FILE`).
* `up` makes one TLS handshake with the proxy. It proves the reach and the certificate. A fault starts with the
  spec and names the hop: `https://proxy.example:443: the proxy: TLS: ...`. The credentials go with each CONNECT.
  A refusal names the proxy, the target and the status: `the proxy https://proxy.example:443 refused CONNECT
  code.example:22: 407 Proxy Authentication Required; ...`.
* The status of the link has the scheme, the spec, the form of the credentials (`file`, `variable` or `none`) and
  the versions offered. It has no user and no password.
* `--egress https://...` and `podnet pod -x` refuse the network by name: it is not a proxy of the process. The
  proxy of the process (`--proxy`, `HTTPS_PROXY`) does not take `https://` ([proxies.md](proxies.md)).
* `podnet net doctor --network https://HOST:PORT` adds the row of the network: `ok` when the TLS handshake with the
  proxy passes, `fail` with the hop when it does not. The row holds no credential.
* Tests use a stand-in proxy on the loopback with a CA of the test (`src/networks/road_stand.rs`,
  `src/networks/via_tests.rs`, `tests/road_network.rs`). They need no account.

## wstunnel: `wstunnel:wss://HOST[:PORT]`

```sh
printf 'path-prefix %s\n' "$PREFIX" > /etc/podnet/wstunnel-secret   # the -r PREFIX of the server
export PODNET_WSTUNNEL_SECRET_FILE=/etc/podnet/wstunnel-secret
podnet forward -L 127.0.0.1:2222=10.0.0.5:22 --network wstunnel:wss://wst.example
```

A host that reaches only a WebSocket on TCP 443 can use a server of erebe/wstunnel (9.1 to 11,
`wstunnel server wss://[::]:443`) as a network. Each stream opens a WebSocket to the server with a token that
names the target. The server dials the target before it answers. podnet has no code of this road: it is
`wstunnel` of `pod-egress` (PodUtils/podcrates).

* `wstunnel:wss://HOST[:PORT]` is TLS (default port 443). `wstunnel:ws://HOST[:PORT]` has no TLS (default port 80).
  `wstunnel:HOST[:PORT]` is the same as `wss://`. The URL has no path, no query, no user and no password: a usage
  error (exit 64) that does not repeat them. The other transports of wstunnel (`https://`, `wts://`) are refused by
  name. `wstunnel:PORT` is still the SOCKS5 hop of a host named `wstunnel`.
* The secret is a setting, not a part of the spec. `PODNET_WSTUNNEL_SECRET_FILE` is a file with the lines
  `path-prefix PREFIX` (the `--restrict-http-upgrade-path-prefix` of the server), `credentials USER[:PASSWORD]`
  (Basic) and `authorization VALUE` (the header as it is), each one time, or one line with the prefix alone. `#`
  starts a comment line. `PODNET_WSTUNNEL_SECRET` is the same text (give one of them). With neither, the client
  sends the defaults of wstunnel: the prefix `v1` and no `Authorization`.
* A bad line is a usage error (exit 64) that names the variable and the number of the line, never its text. A
  secret over `ws://` to a host that is not the loopback is a usage error: give `wss://`.
* A network `NAME` of a router file reads the variables with the suffix `_NAME` when one of them is set.
* The TCP connection to the server uses the way out of the process, or the way below the network. The
  certificate of the server is checked with the roots of podnet (`--ca-file`).
* `up` makes one connection to the server (TLS for `wss://`). A fault starts with the spec and names the hop:
  `wstunnel:wss://wst.example: the wstunnel server: ...`. A refusal of the server names it and its status:
  `the wstunnel server wss://wst.example refused the tunnel: 400 Invalid request; ...`. The server does not give
  the cause (a prefix, a restriction, or a target that it could not reach); its log gives it.
* The status of the link has the scheme, the spec, the transport (`tls`) and the form of the secret (`file`,
  `variable` or `default`). It has no secret.
* `--egress wstunnel:...` and `podnet pod -x` refuse the network by name: it is not a proxy of the process.
* `podnet net doctor --network wstunnel:wss://HOST` adds the row of the network: `ok` when the server takes the
  connection, `fail` with the hop when it does not. The row holds no secret.
* Tests use a stand-in wstunnel server on the loopback with a CA of the test (`src/networks/road_stand.rs`). The
  run against the binary of wstunnel is a test of `pod-egress`.
* Not in the network: the HTTP/2 transport of wstunnel, a reverse tunnel, and UDP.

## VLESS: `vless://HOST:PORT?type=ws&security=tls&path=/PATH`

```sh
printf '%s\n' "$VLESS_UUID" > /etc/podnet/vless-id   # the ID of the user, a UUID
export PODNET_VLESS_ID_FILE=/etc/podnet/vless-id
podnet forward -L 127.0.0.1:2222=code.example:22 --network 'vless://v.example:443?type=ws&security=tls&path=/ws'
```

A host that reaches only HTTPS can use a VLESS server (Xray-core, sing-box) as a network. The spec is the share link
of Xray-core with no ID. Each stream opens a WebSocket (`type=ws`) or XHTTP requests (`type=xhttp`, on HTTP/2, or
on HTTP/1.1 with two connections) to the server. VLESS's request names the target, and the server dials it. podnet
has no code of this road: it is `vless` of `pod-egress` (PodUtils/podcrates).

* The query takes `type` (`ws` or `xhttp`), `security` (`tls` or `none`), `sni`, `host`, `path`, `mode` (of XHTTP:
  `auto`, `packet-up`, `stream-up` or `stream-one`), `alpn` and `extra`. A setting that the client does not speak
  (`type=tcp`, the default of a share link; `security=reality`; a `flow`; an `encryption` other than `none`) is a
  usage error (exit 64) that names it.
* The ID is a setting, not a part of the spec. `PODNET_VLESS_ID_FILE` is a file with the ID: a UUID, or a word of 1
  to 30 bytes, which Xray-core maps to a UUID. `PODNET_VLESS_ID` is the same text (give one of them). The ID is
  required. An ID in the spec (`vless://ID@HOST`), a missing ID and a bad ID are usage errors (exit 64) that name
  the variable and do not repeat the ID.
* A network `NAME` of a router file reads the variables with the suffix `_NAME` when one of them is set.
* The connections to the server use the way out of the process, or the way below the network. The certificate is
  checked for the `sni` of the link (else its host) with the roots of podnet (`--ca-file`).
* `up` makes one connection to the server (TLS for `security=tls`). A fault starts with the label and names the
  hop: `vless://v.example:443/ws: the VLESS server: ...`.
* The network does not wait for the answer of the server: a client can speak first. A server that refuses the ID
  ends each stream before its answer. The read of the stream then fails with the words `the server ended the
  session before VLESS's answer: it refused the ID, or it could not reach the target`.
* The status of the link has the scheme, the label (`vless://HOST:PORT/PATH`), the transport, `tls` and the form of
  the ID (`file` or `variable`). It has no ID.
* `podnet net doctor --network SPEC` adds the row of the network: `ok` when the server takes the connection, `fail`
  with the hop when it does not. The row holds no ID.
* Tests use a stand-in VLESS server on the loopback (`src/networks/servers_stand.rs`,
  `src/networks/servers/tests.rs`, `tests/road_network.rs`).
* Not in the network: early data (0-RTT), UDP, and the VMess and Trojan protocols.

## Shadowsocks: `ss://HOST:PORT`

```sh
printf 'aes-256-gcm:%s\n' "$SS_PASSWORD" > /etc/podnet/ss-key   # METHOD:PASSWORD
export PODNET_SHADOWSOCKS_KEY_FILE=/etc/podnet/ss-key
podnet forward -L 127.0.0.1:2222=code.example:22 --network ss://ss.example:8388
```

A host whose proxy is a Shadowsocks server can use it as a network. Each stream opens a TCP connection to the server.
The address of the target goes in the first chunk, and the server dials it. podnet has no code of this road: it is
`shadowsocks` of `pod-egress` (PodUtils/podcrates).

* The spec is `ss://HOST:PORT`, with no key, no path, no query and no tag. A bad spec is a usage error (exit 64)
  that gives the form and does not repeat the spec: a whole `ss://` URI holds the password.
* The key is a setting, not a part of the spec. `PODNET_SHADOWSOCKS_KEY_FILE` is a file whose one line is
  `METHOD:PASSWORD`, or the `ss://` URI of the server (SIP002, as an access key gives it), which must name the host
  and the port of the spec. `PODNET_SHADOWSOCKS_KEY` is the same text (give one of them). The key is required.
* The methods are `chacha20-ietf-poly1305`, `aes-256-gcm`, `aes-128-gcm` (the AEAD form), and
  `2022-blake3-aes-128-gcm`, `2022-blake3-aes-256-gcm` and `2022-blake3-chacha20-poly1305` (Shadowsocks 2022, whose
  password is a key in base64). A plugin and a key of several users (`iPSK:uPSK`) are refused by name. A missing or
  bad key is a usage error (exit 64) that names the variable and does not repeat the key.
* A network `NAME` of a router file reads the variables with the suffix `_NAME` when one of them is set.
* The TCP connection to the server uses the way out of the process, or the way below the network.
* `up` makes one TCP connection to the server. A fault starts with the spec and names the hop:
  `ss://ss.example:8388: the Shadowsocks server: ...`.
* The network does not wait for the answer of the server: a client can speak first. A server that does not take
  the key says nothing and ends the stream.
* The status of the link has the scheme, the spec, the method and the form of the key (`file` or `variable`). It has
  no password.
* `podnet net doctor --network ss://HOST:PORT` adds the row of the network: `ok` when the server takes the TCP
  connection, `fail` with the hop when it does not. The row holds no key.
* Tests use a stand-in Shadowsocks server of the AEAD form on the loopback (`src/networks/servers_stand.rs`).
* Not in the network: the WebSocket of an Outline server, UDP, and a plugin.

## Wisp: `wisp:wss://HOST[:PORT]/PATH`

```sh
podnet forward -L 127.0.0.1:2222=code.example:22 --network wisp:wss://w.example/wisp/
```

A host that reaches only a WebSocket can use a Wisp server (the Wisp protocol of Mercury Workshop, versions 1 and
2) as a network. The streams go over one WebSocket to the server. Each stream names its target, and the server
dials it. podnet has no code of this road: it is `wisp` of `pod-egress` (PodUtils/podcrates).

* `wisp:wss://HOST[:PORT]/PATH` is TLS (default port 443). `wisp:ws://HOST[:PORT]/PATH` has no TLS (default port
  80). `wisp:HOST[:PORT]/PATH` is the same as `wss://`. `wisp+wss://` and `wisp+ws://` (the form of a forwarder
  template) are the same. The URL takes no user and no password: a usage error (exit 64). `wisp:PORT` is still the
  SOCKS5 hop of a host named `wisp`.
* The server takes no credential of this client. A server that requires a password or a key is refused at `up`.
* The connection to the server uses the way out of the process, or the way below the network. The certificate is
  checked with the roots of podnet (`--ca-file`).
* `up` opens the connection: the WebSocket and the opening of Wisp. A fault starts with the label and names the hop:
  `wisp:wss://w.example/wisp/: the Wisp server: ...`. The message of the day of the server goes to the log.
* A target that the server cannot reach fails the stream with the reason of the server: `the Wisp server closed the
  stream: 0x44, the destination refused the connection`. A server of version 1 does not confirm a stream: a stream
  that the server does not close in 3 s is taken as open.
* When the server closed the connection, the next stream opens a new connection, one time.
* The status of the link has the scheme, the label, `tls`, the version of Wisp and `confirms` (the server confirms
  each stream).
* `podnet net doctor --network SPEC` adds the row of the network: `ok` when the connection opens, `fail` with the
  hop when it does not.
* Tests use a stand-in Wisp server of version 2 on the loopback (`src/networks/servers_stand.rs`).
* Not in the network: UDP streams, and the password and key extensions of Wisp version 2.

## A user with no passwd entry

On a host where `/etc/passwd` has no entry for your uid, `getpwuid` fails.
Then `ssh` stops, and `ssh-keygen` writes no key (some builds exit 0, some
exit 255 with `No user exists for uid N`). `podnet net doctor` shows the row
`passwd entry for this uid`. `podnet net passwd-shim`
writes a passwd file and the preload `fakepwd.so`, and prints `LD_PRELOAD`
(before an old value) and `SANDHOME_PASSWD`:

```sh
eval "$(podnet net passwd-shim)"
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519 -N ''
```

The home of the entry is `$HOME`: `ssh` and `ssh-keygen` find `~/.ssh` there.
`--user`, `--uid`, `--gid` and `--shell` set the entry. `--json` prints the
environment as one object. `net ts-shims` writes the same files, with
`getent` and `id`, and prints the hints for `tailscaled`.
