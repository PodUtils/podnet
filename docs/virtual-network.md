# Virtual network (`podnet net`, `podnet::vnet`)

The virtual network gives programs IPv4 and IPv6 addresses on a host where the
kernel refuses `AF_INET` and `AF_INET6` binds. It needs no network namespace,
no TUN device and no capability. It needs only `AF_UNIX`, including the
abstract namespace.

Use `podnet net doctor` to see what the host permits.

## 1. Parts

| Part | Module | Function |
| --- | --- | --- |
| Shim | `crates/podnet-vnet/shim/podnet-vnet.c`, `vnet::shim` | An `LD_PRELOAD` library. It changes `AF_INET` sockets into `AF_UNIX` abstract sockets. |
| Stack | `vnet::stack` | A `smoltcp` TCP/IP stack. It owns the virtual addresses. The code is `pod-netstack` of PodUtils/podcrates, at its old path. |
| Link | `vnet::device`, `vnet::framing` | The packet queues of the stack, and the framing for a byte stream. |
| Proxy | `vnet::proxy` | A SOCKS5 and HTTP `CONNECT` front door into the stack. |
| Switch | `vnet::switch` | A hub that connects stacks, and the fault injector. |
| Resolver | `vnet::dns` | A DNS server at the gateway for virtual names. |
| Policy | `vnet::policy` | Allow and deny rules, applied at connect time. |
| Capture | `vnet::record` | pcap capture and replay of the link. |
| Control | `vnet::control` | The protocol between the shim and the stack. |

## 2. Addresses

| Address | Use |
| --- | --- |
| `10.66.0.0/24`, `fd00:66::/64` | The virtual subnets. |
| `10.66.0.1` | The gateway. The virtual resolver listens on port 53. |
| `10.66.0.2` to `10.66.0.254` | Stacks and programs. |

The kernel does not see these addresses. `podnet net addresses` prints the plan.

## 3. Shim

Build the shim and start a program with it. The command prints the prefix
of this run as an export, so evaluate the output before the program starts:

```sh
eval "$(podnet net shim --log)"
./server
```

The printed prefix reaches the shim through the environment that the
command also prints, so a reader who copies only `LD_PRELOAD` gives the
program another prefix than the stack it talks to.

### 3.1 Address translation

An IP endpoint becomes an abstract socket name:

```text
\0podnet-vnet/<family>/<address>/<port>

10.66.0.2:8080   ->  \0podnet-vnet/4/10.66.0.2/8080
[fd00::2]:443    ->  \0podnet-vnet/6/fd00::2/443
```

The IPv6 address is in the RFC 5952 form. A `sun_path` holds 108 bytes with
the leading NUL, so a name is at most 107 bytes. With the default prefix the
longest name is 59 bytes, and a prefix may be at most 59 bytes.

`podnet net shim` prints a prefix of its own for each run, as
`PODNET_VNET_PREFIX`, so two shims in one network namespace do not bind the
same name. A program and the stack it talks to must be started with the same
value. Set the variable yourself to choose a fixed prefix. A value that is
empty, over 59 bytes, or holds a byte other than a letter, a digit, `.`, `-`
or `_` is a usage error of `podnet net shim`; the C shim ignores such a value
with a line on stderr when a program loads it directly.

An IPv4-mapped IPv6 address (`::ffff:A.B.C.D`) gets the IPv4 name (T-356).
Thus a dual-mode `AF_INET6` client (for example .NET) that connects to
`[::ffff:127.0.0.1]:7001` meets a listener of `127.0.0.1:7001`, and the
other way. With `PODNET_VNET_MAP_LOOPBACK`, `::1` gets the name of the local
IPv4 address, as `127.0.0.1` does. `accept`, `recvfrom` and `recvmsg` give
the address in the family of the descriptor: an `AF_INET6` descriptor gets
an IPv4 peer in the mapped form.

A bind to the wildcard address (`0.0.0.0` or `::`) takes the local address
(`PODNET_VNET_LOCAL_ADDR`, default `10.66.0.2`; for an `AF_INET6`
descriptor the mapped form) (T-391). Under the shim the host has one
address, so a server bound to the wildcard address meets each client that
dials the local address, and each client that dials `127.0.0.1` when
`PODNET_VNET_MAP_LOOPBACK` is set. `getsockname` then gives the local
address, not the wildcard.

### 3.2 Interposed calls

| Call | Action on a virtual descriptor |
| --- | --- |
| `socket` | Make an `AF_UNIX` socket of the same type. Mark the descriptor as virtual. `SOCK_RAW` gives `EAFNOSUPPORT`. |
| `bind`, `connect` | Translate the address to the abstract name. |
| `sendto`, `recvfrom`, `sendmsg`, `recvmsg` | Translate the datagram addresses. |
| `accept`, `accept4` | The new descriptor is virtual. The logical peer is recorded. |
| `getsockname`, `getpeername` | Return the logical address. Truncate to `*len` as Linux does. A null address gives the length. A stream client with no `bind` gets a local address at `connect` (the shim address and a port of 49152-65535), and the server sees it as the peer. |
| `setsockopt`, `getsockopt` | Record `IPPROTO_TCP` options (`TCP_NODELAY`, `TCP_KEEPIDLE`) and answer them. |
| `close`, `dup`, `dup2`, `dup3`, `fcntl(F_DUPFD*)` | Clear or copy the virtual state. |
| `socket(AF_NETLINK, *, NETLINK_ROUTE)` | A `socketpair(AF_UNIX, SOCK_DGRAM)`. The shim answers each request that the program sends with `send`, `sendto` or `sendmsg`; a request of `write` gets its answer at the next `recv`, `recvfrom` or `recvmsg`. See 3.4. |
| `getifaddrs`, `if_nameindex`, `if_nametoindex`, `if_indextoname` | Give the interfaces of 3.4. |
| `fcntl` (each other command) | Forward with the third argument in its type: none, an `int`, or a pointer (`F_GETLK`, `F_SETLK`, `F_SETLKW`, the `F_OFD_*` locks, `F_GETOWN_EX`, `F_SETOWN_EX`, the pipe, seal and hint commands, T-390). |

`read`, `write`, `poll`, `epoll` and `shutdown` are not interposed. They work
on the real `AF_UNIX` descriptor.

### 3.3 Limits

* The shim cannot reach a static binary. Use `podnet pod` or `podnet net proxy`
  for such programs.
* `recvmmsg` and `io_uring` are not interposed.
* `/proc/net/tcp`, `/sys/class/net` and an `ioctl` of `SIOCGIFCONF` do not
  show the virtual addresses. The internal calls of libc (for example the
  `AI_ADDRCONFIG` check of `getaddrinfo`) ask the host's kernel.
* An abstract name is global on the host (in one network namespace). Two
  programs that bind the same virtual address and port under the same prefix
  get `EADDRINUSE`. Each run takes its own prefix unless
  `PODNET_VNET_PREFIX` names one, so two runs of the shim do not collide.
* An abstract name has no file permissions. Each user of the host can bind
  or connect to it. Thus the shim takes the same user only (`SO_PEERCRED`):
  `accept` closes a client of another user and takes the next client, and
  `connect` to a listener of another user fails with `ECONNREFUSED`. A
  datagram socket of the shim has `SO_PASSCRED`: before `recvfrom` and
  `recvmsg`, the shim drops each datagram of another user, with one line on
  stderr (T-075). `recv` and `read` do not drop: a program that reads a
  datagram socket with them gets each datagram.

### 3.4 Interfaces

Under the shim the program has one address. The shim gives the same two
interfaces to `NETLINK_ROUTE`, `getifaddrs` and `if_nameindex` (T-392):

| Index | Name | Addresses | Flags |
| --- | --- | --- | --- |
| 1 | `lo` | `127.0.0.1/8`, `::1/128` | `UP`, `LOOPBACK`, `RUNNING`, `LOWER_UP` |
| 2 | `vnet0` | the local address `/24` (`PODNET_VNET_LOCAL_ADDR`) | `UP`, `BROADCAST`, `RUNNING`, `MULTICAST`, `LOWER_UP` |

* `RTM_GETLINK` and `RTM_GETADDR` dumps give these rows, with
  `NLM_F_MULTI`, then `NLMSG_DONE`. Each other dump (routes, neighbours,
  rules) gives `NLMSG_DONE` only. A get of one link by index or name gives
  that link, or `ENODEV`. A change (`RTM_NEW*`, `RTM_DEL*`, `RTM_SET*`)
  gives `EPERM`.
* Each answer has the port id of the socket in `nlmsg_pid`: the port id of
  `bind`, else `getpid()` for the first socket. `getsockname` gives the
  same port id, and `recvfrom` and `recvmsg` give the address of the kernel
  (port 0). libtorrent drops each answer with another port id.
* A program that enumerates the interfaces (libtorrent, `ip addr`) thus
  listens on and advertises the virtual address, not an address of the host.
* `PODNET_VNET_HOST_NETLINK=1` gives these calls to the host again, for a
  program that must see the host (for example `ip` of the host).

## 4. Stack

`NetStack::loopback()` makes a stack whose link sends each packet back into the
stack. Programs and the proxy in one network namespace use it.

`NetStack::over_link(config)` makes a stack whose packets go to
`link_out()` and come from `link_in()`. A pump moves the packets between the
queues and a byte stream. `vnet::switch::pump` is that pump.

* The clock is `std::time::Instant`. The stack waits on the inbound queue or on
  the next timer of `smoltcp`. It does not spin.
* On a byte stream, each packet has a 2-byte big-endian length in front of it
  (`vnet::framing`).
* The MTU is 1280 or more.

## 5. Modes

### 5.1 Direct (one network namespace, no stack)

Two programs with the shim talk through the abstract name. No other process is
necessary.

```text
program A: bind 10.66.0.2:8080     ->  \0podnet-vnet/4/10.66.0.2/8080
program B: connect 10.66.0.2:8080  ->  \0podnet-vnet/4/10.66.0.2/8080
```

### 5.2 Proxy

`podnet net proxy` puts a SOCKS5 and HTTP `CONNECT` front door on the stack.
`--forward VIRT=REAL` makes the stack listen on a virtual address and send each
connection to a real destination (`host:port` with a name or an address,
`[v6]:port`, or `unix:/path`). A target that is not loopback goes through the
proxy of the process, with `NO_PROXY`. A loopback target and a unix path go
direct. A failed dial logs the target, the way and the hop.

```sh
podnet net proxy --listen unix:/tmp/podnet-vnet.socks \
    --forward 10.66.0.2:8080=unix:/run/app.sock
```

A client speaks SOCKS5 over `AF_UNIX`. curl 7.84 and later takes the
socket path in the proxy URL:
`curl --proxy socks5h://localhost/tmp/podnet-vnet.socks http://10.66.0.2:8080/`.
For a client that takes no socket path, on a host that permits a TCP bind,
use `--listen tcp://127.0.0.1:1080` and `ALL_PROXY=socks5h://127.0.0.1:1080`.

Each unix listener (`net proxy`, `net switch`, `net tailscale`) claims its
socket path first. A socket that another process serves gives exit 69. A file
that is not a socket gives exit 64. A stale socket is removed. A client that
sends no request in 30 s gets SOCKS5 code 4 or HTTP 504.

### 5.3 Switch (more than one network namespace)

`podnet net switch` is a hub for stacks. Each stack connects to it with a byte
stream. The switch records the source address of each peer. It sends each
packet to the peer that owns the destination address, and it sends a packet
for an unknown address to all other peers.

* A source address is of the first peer that sends from it, until that peer
  disconnects. The switch drops a packet of another peer with that source
  address, and counts it.
* A unix listener takes the same user only (`SO_PEERCRED`).
* A TCP listener needs a shared token. Set `PODNET_SWITCH_TOKEN`, or give
  `--switch-token-file PATH` (the text of the file with no white space at
  the ends). With no token, `podnet net switch --listen tcp://...` exits 64.
  `podnet net proxy --switch tcp://...` sends the same token, from the same
  variable or its own `--switch-token-file`.
* The first line of a TCP peer is `podnet-switch/1 TOKEN` and a line feed.
  The switch answers `ok` and a line feed, then the frames start. A peer
  with a wrong first line gets `refused` and the switch closes it. A peer
  that sends no line in 10 s is closed. The token is not encrypted: on a
  network that others can see, use a tunnel (`podnet tunnel --forward`) or
  another encrypted path.

```sh
# Host S: the switch. Expose it with a tunnel when the hosts are far apart:
podnet net switch --listen unix:/tmp/sw.sock
podnet tunnel --forward unix:/tmp/sw.sock

# Host A: a stack at 10.66.0.2 with a service.
podnet connect wss://<host>/__podnet/ws -L unix:///tmp/sw.sock     # if remote
podnet net proxy --switch unix:/tmp/sw.sock --address 10.66.0.2 \
    --forward 10.66.0.2:8080=unix:/run/app.sock

# Host B: a stack at 10.66.0.3. Its proxy reaches 10.66.0.2:8080.
podnet net proxy --switch unix:/tmp/sw.sock --address 10.66.0.3 \
    --listen unix:/tmp/b.socks
```

Shim programs can also use the switch:

* `--control NAME` serves the control socket `\0NAME`. Start the program with
  `PODNET_VNET_CONTROL=NAME`. When the program binds a virtual address, the shim
  sends `RegisterBind`. The stack then listens on that address and sends each
  connection from the switch to the program.
* `--import VIRT` binds the abstract name of the remote address `VIRT` on this
  host. A shim program that connects to `VIRT` goes through the stack and the
  switch.
* These sockets take the same user only. The control socket has
  `SO_PASSCRED` and drops a datagram of another user. `--import` closes a
  client of another user. The stack does not send a connection to an
  abstract name that a process of another user holds. The unix listener of
  `podnet net switch` closes a peer of another user (also root).

### 5.4 Faults

`podnet net proxy --switch ... --address ...` takes `--drop`, `--duplicate` and
`--reorder` (probabilities from 0.0 to 1.0) and `--seed`. They put faults on
the packets that this stack sends to the switch; the switch itself
(`podnet net switch`) has no faults. The same seed gives the same faults. With
no `--switch`, the flags are a usage error (exit 64), as a value out of 0.0
to 1.0 is. In code, use `vnet::switch::Faults` and `FaultInjector`.

```sh
podnet net switch --listen unix:/tmp/sw.sock &
podnet net proxy --switch unix:/tmp/sw.sock --address 10.66.0.3 --drop 0.1 --seed 7
```

### 5.5 Routes to a SOCKS5 front door

A program can send a TCP connect to a SOCKS5 front door on a unix socket
(`podnet net tailscale`, `podnet net proxy`). There are three tiers:

| Tier | Shim | Destinations to the door | Other connects |
| --- | --- | --- | --- |
| Tailnet (the default) | `libpodnet-socks.so` (`net socksify`) | `100.64.0.0/10` and `fd7a:115c:a1e0::/48` | the real network |
| Chosen | `libpodnet-socks.so` (`net socksify --cidr ROUTE`) | only the listed routes | the real network |
| All | `libpodnet-socks.so` with `PODNET_SOCKS_ALL=1` | each TCP connect | none |

A route is `tailnet`, `ADDR`, `ADDR/BITS`, `IPV4:PORT` or `[IPV6]:PORT`.
`--cidr` is repeatable. The list replaces the tailnet default: add `tailnet`
to keep it. The shim keeps 32 routes at most. An IPv4-mapped IPv6 address
matches as IPv4. With `--log` (`PODNET_SOCKS_LOG=1`), the shim logs each
route at the first connect, and each redirect.

```sh
eval "$(podnet net socksify --socket /tmp/podnet-socks.sock --cidr 127.0.0.1:3000 --cidr tailnet)"
```

The shim of the virtual network (`net shim`) takes the same routes with
`--socks-route ROUTE` and `--socks-socket PATH` (`PODNET_VNET_SOCKS` and
`PODNET_VNET_SOCKS_ROUTES`). A stream connect to a listed destination goes to
the door. Each other connect goes to its abstract name, as before.

```sh
podnet net shim --socks-route 192.168.1.5:5432 --socks-socket /tmp/podnet-socks.sock
```

podnet checks each route and exits 64 at a bad route. The C shims ignore a
bad route of the environment, and log it with the log variable.

### 5.6 Forward to a unix socket

A host can refuse each `AF_INET` and `AF_INET6` `bind` and `connect`
(`EACCES`, also on loopback), and allow `AF_UNIX`. A program under the
shim can then reach a service that listens on a unix socket path, at a TCP
address (T-357). Give each forward with `--forward ADDR:PORT=unix:/PATH`
(`PODNET_VNET_FORWARD`, a comma list):

```sh
podnet net shim \
  --forward 127.0.0.1:7001=unix:/run/podrunner/broker.sock \
  --forward 127.0.0.1:3128=unix:/run/podrunner/proxy.sock
```

* `ADDR:PORT` is `IPV4:PORT` or `[IPV6]:PORT`. An IPv4-mapped IPv6 address
  (`[::ffff:127.0.0.1]:7001`) matches its IPv4 forward.
* `PATH` is absolute and has 107 bytes at most, the 108-byte `sun_path` less
  its NUL. The shim keeps
  16 forwards at most.
* A stream `connect` to a listed destination connects the descriptor to
  `PATH`. There is no SOCKS step and no other process. `getpeername` gives
  the TCP address that the program asked for. A `PATH` with no socket gives
  `ECONNREFUSED`.
* The order is: the forwards, then the SOCKS routes (5.5), then the
  abstract name.
* The shim does not check the user of the peer of a forward. The file
  permissions of `PATH` control the access.

`net shim` checks each forward and exits 64 at a bad forward. The C shim
ignores a bad forward of the environment, and logs it with
`PODNET_VNET_LOG`.

## 6. Resolver, policy and capture

* `vnet::dns::Resolver` answers A and AAAA queries for a name table at
  `10.66.0.1:53`. `net proxy --map NAME=ADDR` and `--map-file PATH` fill the
  table of the gateway and of the proxy door; `net shim --map` and
  `--map-file` fill the table of `getaddrinfo`. An address outside the subnet
  is a usage error (exit 64). It can send unknown names to an upstream, or answer all
  unknown names with one address.
* `vnet::policy::Acl` has allow and deny rules for addresses and ports. The
  stack applies them when a program binds or connects.
* `vnet::record` writes the link packets to a pcap file. No `CAP_NET_RAW` is
  necessary. `Replayer` sends a capture into a stack again.

## 7. Control protocol

The shim sends control messages as datagrams to `\0<PODNET_VNET_CONTROL>`:

```text
u8 op | u16 length | payload
```

| Op | Message | Payload |
| --- | --- | --- |
| 1 | `RegisterBind` | `u32 id`, address |
| 2 | `RegisterConnect` | `u32 id`, address |
| 3 | `Unregister` | `u32 id` |
| 4 | `Datagram` | `u32 id`, from, to, data |

An address is `u8 family` (4 or 6), the address bytes and a `u16` port.
`ControlDecoder` finds the next frame after a bad frame and counts the dropped
bytes.

## 8. Verification

* `tests/af_inet_kernel.rs` reads the socket inode from `/proc/self/fd/N` and
  checks that the kernel lists the socket in `/proc/net/unix`, not in
  `/proc/net/tcp`. It does not use `getsockname`, because the shim answers that
  call.
* `tests/shim_no_socket.rs` checks programs that never open a socket.
* `vnet::switch` tests send 64 KiB through a switch with 10% drop, 5%
  duplicates and 5% reorder, and check that the bytes arrive in order.
