# podnet-netns

`podnet-netns` runs a program with all its network traffic sent through an
upstream: a SOCKS5 proxy, an HTTP `CONNECT` proxy, a chain of proxies, or a
tailnet through `tailscaled`. It does not use `LD_PRELOAD`. The kernel enforces
the redirection, so static binaries, Go binaries and programs that do not use
libc `connect(3)` get the same treatment.

`podnet-netns` is a library. `podnet pod` is its command line.

```sh
podnet pod -x socks5://127.0.0.1:1080 -- curl https://example.com
podnet pod -x socks5://unix:/run/proxy.sock -- ./static-binary
podnet pod -x socks5://hop1:1080 -x socks5://hop2:1080 -- ssh host
podnet pod -x tailscale:/run/tailscale/tailscaled.sock -- curl http://100.64.0.5/
podnet pod doctor --json
```

## Options

| Option | Function |
| --- | --- |
| `-x, --proxy SPEC` | An upstream. Repeat it to make a chain. Forms: `socks5://[user:pass@]host:port`, `http://[user:pass@]host:port`, `socks5://unix:/path`, `http://unix:/path`, `tailscale:/path/to/tailscaled.sock`. Default: `$ALL_PROXY`, `$HTTPS_PROXY` or `$HTTP_PROXY`. A SOCKS5 user or password longer than 255 bytes is an error (the limit of RFC 1929). A SOCKS5 target name longer than 255 bytes is refused. |
| `-r, --rule SPEC` | A route for some destinations: `domain:NAME=PROXY`, `domain:.NAME=PROXY` (the name and each name under it) `cidr:ADDR/PREFIX=PROXY` (IPv4 or IPv6) or `cidr:@FILE=PROXY` (one prefix on each line of FILE; `#` starts a comment). PROXY can be `direct` (TCP from podnet-netns, no proxy). PROXY can be `reject` or `block` (the words of the WARP rules): the pod refuses the flow. With `seccomp`, `connect` fails with `ECONNREFUSED` and a datagram send with `EACCES`. With `netns`, the program gets a RST, and the pod drops a datagram. `--serve` answers SOCKS5 reply `0x02` or HTTP 403. A TCP connect to `127.0.0.0/8` with `seccomp` does not read the rules. `-v` prints each refusal with the count. The first rule that matches is used. |
| `--default-action ACTION` | `allow` (default) or `reject`: what the pod does with a target that no rule matches. `reject` refuses it as a `reject` rule does: with `seccomp`, `connect` fails with `ECONNREFUSED` and a datagram send with `EACCES`; with `netns`, the program gets a RST, and the pod drops a datagram. The rule name of the refusal is `default`. |
| `--flow-log FILE` | Append one JSON line to `FILE` (mode 0600) for each decision: `time`, `event` (`dial`), `flow`, `proto` (`tcp` or `udp`), `target`, `rule` (the rule text, `default` or `loopback`), `chain` and `verdict` (`allow`, `direct` or `reject`). A UDP socket gets one line for each destination. Each relayed TCP flow also gets a line at its close (`event` `close`, `bytes_out` from the program, `bytes_in` to the program). With `seccomp`, the pod then relays each TCP flow: the program does not get the socket of the upstream. `-v` stays the log for humans. |
| `--learn FILE` | Write each destination of the run to `FILE` at the exit of the program: one line `PROTO TARGET` (`tcp` or `udp`) for each destination. A flow that a rule refuses goes through the `-x` chain, so the run sees each destination. A TCP connect to `127.0.0.0/8` with `seccomp` is not recorded. |
| `--fail-on-violation FILE` | Read a file of `--learn`. The pod refuses each destination that is not in `FILE`, as a `reject` rule does (the rule name is `not-learned`). A destination of `FILE` that a rule refuses is also a violation. At the exit of the program, the pod writes each violation on stderr. When the program exits 0 and a violation occurred, the exit status is 4. A bad line in `FILE` is a usage error (exit 64). Not with `--serve`. |
| `--flow-budget N` | Seccomp backend: the most sockets of the program that the pod tracks at one time (64 to 1048576). A `socket` call above it gets `EMFILE`. The default comes from `RLIMIT_NOFILE`: the supervisor raises its soft limit to the hard limit after the start of the program (the program keeps its limit), takes 64 descriptors as a reserve and two descriptors for each socket, and stops at 65536. `pod doctor` shows the default. A budget above the default gives a warning. The netns backend does not use it. |
| `--map NAME=ADDR` | Resolve `NAME` to `ADDR`, with no upstream. |
| `--backend NAME` | `auto` (default), `netns` or `seccomp`. |
| `--prefer-udp` | With `auto`: `seccomp` first when it works, because it carries UDP datagrams of each size (`netns` carries a datagram of 1500 bytes at most). `podnet warp run` sets it. |
| `--name-backend` | Write one line on stderr that names the backend that runs (also after `auto`) and its limits. `podnet warp exec` and `podnet warp shell` set it. `PODNET_NETNS_QUIET` (`podnet -q`) removes the line. |
| `--keep-proxy-env` | Keep `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` and their lower-case forms for the program. By default the program does not get them: the pod is its egress, and a program that used them would send to a host proxy that it cannot reach. |
| `--inbound-dir DIR` | Seccomp backend only: where a port that the program listens on becomes a unix socket (`DIR/<port>.sock`). The default is `$XDG_RUNTIME_DIR/podnet-netns-inbound` when that directory is of the user, else `$TMPDIR/podnet-netns-inbound-<uid>`. A new directory gets mode 0700. A directory of another user, a symbolic link, or a directory that the group or others can write is refused (exit 3). With `--backend netns` it is a usage error (exit 64); when `auto` picks netns, a warning says that it is not used. |
| `--serve LISTEN` | Serve SOCKS5, HTTP `CONNECT` and UDP ASSOCIATE on `unix:/path` or `tcp://host:port`, through the `-x` chain. |
| `-v, --verbose` | Log each flow. With `seccomp`, also log each socket that the pod makes as a socket pair and each `setsockopt` that the pod answers. |

The pod counts each refused `socket` call of the seccomp backend by class:
`table full` (the flow budget), `socketpair failed` (the descriptor limit of
the supervisor) and `other` (a handler failed: `EIO`, or `ECONNREFUSED` for
`connect`). At the exit of the program, the pod writes one line with the
counts on stderr when a call was refused. `-v` writes one line for each
refusal.

`podnet pod doctor` prints what the host permits. The exit status is 3 when no
backend can run. The seccomp probe runs in a child process. It sets
`no_new_privs` before it installs its filter, as the backend does: an
unprivileged process cannot install the filter without it.

The row `netfilter in a namespace (not a backend)` is a measure (T-152). No
backend uses it. In a child process, in a new user and network namespace,
the probe adds an nftables table, a `nat` chain of the output hook and the
rule `tcp dport 80 redirect to :9999` through nfnetlink. Then it reads
`SO_ORIGINAL_DST` of a redirected loopback connection and finds the
conntrack entry of a UDP datagram through ctnetlink. The row names the
first step that fails, with its error.

## The life of the pod

`podnet pod` gives the exit code of its program at the exit of the program.
A process that the program starts (a daemon) keeps the network of the pod:

* The pod runs in a back process. The front process returns the exit code
  of the program, and the back stays while a process of the pod lives.
* `netns`: the first process of the namespace is a reaper
  (`PR_SET_CHILD_SUBREAPER`). It starts the program and reaps each orphan.
  The router ends when the reaper has no child.
* `seccomp`: the supervisor is the reaper. It ends when no task uses the
  filter, or when it has no child.
* When no other process lives, the pod ends as before, and the front
  returns after the relays finish.
* After the exit of the program, the back puts `/dev/null` on its standard
  input and output. With `-v`, it keeps its standard error when that is a
  terminal or a file.
* `podnet warp run` runs the pod on a thread: there the pod ends with its
  program.

## Backends

`--backend auto` uses `netns` when the host permits `unshare`, else `seccomp`.
With the `seccomp` backend, a TCP connect to `127.0.0.0/8` does not go to
the proxy: it goes to a listener of the program itself (its inbound
socket) when there is one, else to TCP on the host.

### netns

* The child calls `unshare(CLONE_NEWUSER | CLONE_NEWNET)` (or
  `CLONE_NEWNET` with privileges). The parent writes the uid and gid maps.
* The child brings up `lo` and makes a TUN device with `ioctl` calls (not with
  `ip`), then gives the TUN descriptor to the parent with `SCM_RIGHTS`.
* The parent runs a `smoltcp` stack on the TUN. For each SYN of the program,
  it dials the upstream first, in a task. When the dial succeeds, the stack
  accepts the connection and copies the bytes. When the dial fails (a dead
  proxy, a 407 of the proxy), the program gets a RST, and its `connect()`
  fails with `ECONNREFUSED`. The program cannot read the HTTP status of the
  proxy; `-v` prints it.
* The parent answers each UDP query to port 53, to any resolver address, from
  the fake-IP pool. The answer comes from the address that the program sent
  the query to.

* The parent takes each other UDP datagram of the program before the stack
  (T-111). Each flow (source, source port, destination, destination port)
  has its own task, so two flows to one port stay apart. The task sends the
  datagrams to the first proxy of the chain of the destination: in the
  stream of the proxy (SOCKS5 command `0x83`) for a proxy on a unix socket,
  else with `UDP ASSOCIATE`. A `direct` rule sends them from a UDP socket
  of the host. A fake address goes to the proxy as its name. Each reply
  comes from the address that the program sent to.
* A flow with no datagram for 60 seconds ends. The table holds 256 flows;
  a new flow above that ends the flow that was idle longest.
* Each flow has a queue of 64 datagrams. When the queue is full, the
  parent stops the read of the TUN until the queue has room: the kernel of
  the pod then counts the loss. A full queue does not end the flow.
* A datagram of the program that is larger than the MTU (1500 bytes) is a
  fragment, and it does not go through. A larger reply goes to the program
  as fragments.

Needs: unprivileged user namespaces and `/dev/net/tun`. This backend carries
TCP and UDP. No backend of the pod carries ICMP (`ping`) to the internet. The `seccomp` backend answers an echo to the
gateway of the pod (see below).

A listener of the program (for example `podnet connect -L tcp://127.0.0.1:8080`)
is in the network namespace of the pod: a process outside the pod does not
see it. Use a unix socket for it (`-L unix:///path`).

### seccomp

* A helper thread sets `no_new_privs`, installs a `SECCOMP_RET_USER_NOTIF`
  filter, and forks the child. Thus the supervisor threads have no filter.
* The filter stops: `socket`, `connect`, `bind`, `listen`, `accept`, `accept4`,
  `sendto`, `sendmsg`, `sendmmsg`, `recvfrom`, `recvmsg`, `recvmmsg`, `setsockopt`,
  `getsockopt`, `getsockname`, `getpeername`, `close`, `shutdown`, `io_uring_setup`,
  `io_uring_enter`, `io_uring_register`. A system call of a foreign ABI (i386 or x32 on x86_64)
  gets `ENOSYS`.
* `socket(AF_INET | AF_INET6)` gets one end of a socketpair, installed with
  `SECCOMP_IOCTL_NOTIF_ADDFD`. Thus `poll` and `epoll` work on a real
  descriptor, and `read` and `write` of a TCP socket go on the socketpair.
  The UDP path is below.
* The supervisor keeps one entry for each socket of the program. The key is
  the inode of the socket (`/proc/<pid>/fd/<n>` gives `socket:[INODE]`), so a
  copy made by `fork`, `dup`, `dup2`, `dup3` or `fcntl(F_DUPFD)` finds the
  entry. A `close` removes the entry only when no other descriptor of the pod
  refers to the socket. A sweep removes the entries of sockets that closed
  with no `close` call (the exit of a process, `close_range`).
* `connect` is answered after the supervisor dials the upstream. The dial
  runs in its own task: the caller waits, and the other system calls of the
  program get their answers during the dial. A second `connect` of the same
  socket during the dial waits for the same dial. A `close` of the socket
  during the dial gives `ECONNABORTED` to the waiting `connect`. When no
  caller waits at the end of the dial (a signal, or the exit of the caller),
  the supervisor drops the connection. The
  supervisor reads the address from `/proc/<pid>/mem` of the calling task. It
  opens that file again for each notification, and it checks the notification
  with `SECCOMP_IOCTL_NOTIF_ID_VALID` after the open. A connect that fails
  gives `ECONNREFUSED`, and `getsockopt(SO_ERROR)` then gives 0, as the
  kernel does.
* A `connect` of a nonblocking socket gives `EINPROGRESS` at once, and the
  dial goes on (T-223). A second nonblocking `connect` during the dial gives
  `EALREADY`. The socketpair is writable before the dial ends, so `poll`
  does not wait: `getsockopt(SO_ERROR)` during the dial waits for the end of
  the dial. It gives 0 or `ECONNREFUSED` one time, and 0 after that. A
  `read` or `write` after a failed nonblocking dial does not give the error.
* `bind` of a TCP socket keeps the address, with an ephemeral port for
  port 0; a second `bind` gives `EINVAL`. `listen` with no `bind` binds the
  unspecified address and an ephemeral port. `shutdown` of a TCP socket
  with no connection gives `ENOTCONN` (T-223).
* `bind`, `listen` and `accept` make a unix listener in `--inbound-dir`. The
  listener fd of the program is readable while a connection waits, so
  `poll`, `select` and `epoll` wake for it. A non-blocking `accept` with no
  connection gets `EAGAIN`; a blocking `accept` waits without stopping the
  other system calls of the program. The peer address of an accepted
  connection is not known (the client is a unix socket). The program gets
  the accepted unix stream itself: the kernel moves the bytes, and the
  supervisor starts no relay (T-150). `-v` prints one line for each accept.
* UDP to port 53 is answered from the fake-IP pool. Other UDP goes through
  SOCKS5 UDP ASSOCIATE. `sendmmsg` sends all messages when the supervisor can
  write `msg_len` into the child. Else it sends one message and returns 1.
  glibc does not send the other messages, so the seccomp backend adds
  `single-request` to `RES_OPTIONS` of the program: glibc then sends the A
  and the AAAA query in two calls (issue 113). A program that clears its
  environment loses this option.
* The filter does not stop `write` and `writev`: a stop costs one
  notification for each write of each program. On a UDP socket, a reader
  task of the supervisor takes each datagram that the program writes on the
  socketpair, and the supervisor sends it to the address of `connect`, as
  `sendto` does (a query to port 53 gets its answer from the fake-IP pool).
  The `write` gives the length at once, so an error of the relay does not
  get to the program. A datagram written on a socket with no `connect` has
  no destination: the supervisor drops it, and the `write` gives no
  `EDESTADDRREQ`. `-v` prints one line for each dropped datagram.
* The entry of a UDP socket holds its relay: the relay socket or the control
  connection, and the tasks that read them. The drop of the entry stops the
  relay, and the supervisor closes its descriptors of the socket. A `close`
  of the last descriptor drops the entry, a sweep drops the entry of a
  socket that closed with no `close` call, and the end of the pod drops each
  entry.
* The supervisor checks each length that the program gives before it reads
  the memory of the program. A datagram that does not fit in one SOCKS5 UDP
  datagram of 65507 bytes (the header and the payload) gets `EMSGSIZE`, as
  the kernel gives for a datagram that is too large. A `msghdr` with more
  than 1024 iovecs also gets `EMSGSIZE`. A buffer that cannot be read gets
  `EFAULT`. A datagram to a name over 255 bytes (the limit of a SOCKS5
  name) gets `EINVAL`, and the pod writes one log line.
* `recvfrom`, `recvmsg` and `recvmmsg` on an unconnected UDP socket give the source
  address of the datagram, not the `AF_UNIX` address of the socketpair. The
  source is the address that the program sent to: the fake IP of a name
  also when the proxy answers from the address of the host, and the address
  of the resolver for a DNS answer. The supervisor reads the datagram from
  the socketpair and writes the data, the address and its length into the
  memory of the program. `MSG_PEEK`, `MSG_TRUNC` and `MSG_DONTWAIT` work. A
  blocking read waits without stopping the other system calls of the
  program. These reads stay direct: a read of a connected socket, a read of
  a socket that is not of the pod, `read`, and each read when
  the host refuses writes to the memory of the program. A `recvfrom` with
  no source buffer (`recv`, the usual read of a TCP program) does not go
  through the supervisor: the kernel runs it. Each other `recvfrom` and each
  `recvmsg` goes through the supervisor, also a read of a TCP socket, which
  the supervisor passes on at once.
* A connected UDP socket takes the datagrams of its peer only, as in the
  kernel. The supervisor compares the source of each datagram of the relay
  (after it maps a name to its fake IP) with the address of `connect`, and
  drops a datagram of another source before it gets to the socketpair.
* No ICMP error gets to a UDP socket of the program. The relay peers with
  the proxy, and the SOCKS5 UDP header has no ICMP field. Thus the program
  gets no `ECONNREFUSED` from a port-unreachable, no `EMSGSIZE` with the
  path MTU, and `IP_MTU` has no value. The error queue is always empty:
  `recvmsg` with `MSG_ERRQUEUE` on a UDP or ICMP socket gets `EAGAIN` from
  the supervisor (the kernel would read the socketpair). `pod doctor`
  prints this limit.
* `recvmmsg` is a series of `recvmsg` reads: each `mmsghdr` gets the real
  source and its `msg_len`. The call waits for the first message only, as
  with `MSG_WAITFORONE`, then takes each message that waits. The timeout
  is not used (the kernel also checks it only after a message).
* `socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP)` (a ping socket) and the
  `AF_INET6` socket with `IPPROTO_ICMPV6` get their own kind. The proxy
  carries no ICMP. The supervisor answers an echo request to the gateway of
  the pod, `10.66.255.254`, with an echo reply from that address. An echo
  to another destination gets `EACCES`, and each echo of an IPv6 socket
  gets `EACCES` (the pod has no IPv6 gateway). A message that is not an
  echo request gets `EINVAL`, as the kernel gives.
* With a proxy on a unix socket, UDP goes on the control connection first
  (SOCKS5 command `0x83`, "UDP in the stream"). The supervisor then needs no
  kernel UDP socket, and a host that refuses `AF_INET` sockets keeps UDP. A
  proxy that answers `0x07`, or closes, gets UDP ASSOCIATE.
* `io_uring` is refused with `EPERM`, because its operations do not go through
  the filter.
* A TCP socket keeps its entry in the supervisor after its `connect`
  (`src/sockopt.rs`). `setsockopt` gives 0 and the supervisor keeps the
  value; `getsockopt` gives the value that the program set (for example
  `TCP_NODELAY` and `TCP_KEEPIDLE`). `getpeername` gives the address and
  the port of the `connect` (a name keeps its fake address); a stream with
  no `connect` gives `ENOTCONN`. `getsockname` gives the bound address,
  else the unspecified address of the family with an ephemeral port.
* A UDP socket and an ICMP socket keep their options too. The values are
  echoes: `getsockopt` gives the value that the program set (for example
  `IP_TTL`), and SOCKS5 carries no IP option, so the value has no effect on
  the datagrams upstream. An option that the program did not set gives
  zeros: `IP_MTU` has no value.
* A UDP socket keeps the address of `bind`. Port 0 gets a stable port of
  the ephemeral range, and a second `bind` gets `EINVAL`. `getsockname`
  gives the bound address, else the unspecified address of the family: with
  an ephemeral port after a `connect` or a relayed datagram, with port 0
  before. `getpeername` gives the address of `connect`, else `ENOTCONN`.
  `connect` with `AF_UNSPEC` dissolves the association, as in the kernel:
  then `getpeername` gives `ENOTCONN`, `send` with no address gives
  `EDESTADDRREQ`, and `recvfrom` gives the source of each datagram again.
  No datagram arrives at a bound port: the relay carries only the answers to
  the datagrams that the socket sent. An ICMP socket keeps only the
  identifier of `bind` (its port), and its `getsockname` gives the
  unspecified address with that identifier.
* `getsockname` and `getpeername` give the logical address. The supervisor
  writes it into the child memory through `/proc/<pid>/mem`, else with
  `process_vm_writev`. As the kernel does, it writes at most the length that
  the caller gives, and it sets the length to the full size of the address. When the host refuses both, the calls fail with
  `EOPNOTSUPP` (not the `AF_UNIX` address of the socketpair), the pod prints
  a warning once, and `pod doctor` shows the row `write program memory` as
  `fail`. A QUIC client (`curl --http3`) then fails; `curl --http3` falls back
  to HTTP/2. A program that reads its own address can crash: for example a
  Go program with a type assertion on `LocalAddr()`. Use `--backend netns`
  where the host permits it. No other way gives the address to the program
  without a write to its memory.
* An errno of the supervisor is a failure of the system call (-1 and the
  errno), for example `EPERM` for `io_uring_setup`.
* After the child exits, the supervisor waits up to 3 seconds for the relays
  to finish.

Needs: seccomp with `SECCOMP_RET_USER_NOTIF` (Linux 5.0 or newer).

### UDP in the stream (SOCKS5 command `0x83`)

The request is a SOCKS5 request with the command `0x83` and the address
`0.0.0.0:0`. The reply is a SOCKS5 reply. After a reply of `0x00`, each
datagram in each direction is one frame on the same connection:

| Bytes | Field |
| --- | --- |
| 2 | The length of the datagram, big endian |
| N | A SOCKS5 UDP datagram (RFC 1928, section 7): header and payload |

The association ends when the connection closes. `podnet warp proxy` and
`podnet pod --serve` serve the command. A server that does not know it
answers `0x07`.

## DNS

Both backends answer A queries from the pool `198.18.0.0/15`. The same name
always gets the same address. Names match with no case, and with no
trailing dot: `Example.COM` and `example.com.` get the address of
`example.com`. The pool has 131072 addresses, and it gives each address
once: a pod that asks for more names gets `SERVFAIL` for each new name (the
names that have an address keep it). A reused address would send the flows
of an older name to the newer name. With the `seccomp` backend, AAAA queries get
an address of the ULA prefix `fd63:6672:7300::/96` that holds the same pool
address, so `getaddrinfo(AF_INET6)` works. The `netns` backend (IPv4 only)
answers AAAA with no address. When the program connects to a pool address
of either family, the upstream gets the name (SOCKS5 `ATYP=3`, or the
`CONNECT` host), so the upstream resolves it. A name of `--map` gets no
AAAA answer. `--map` names match with no case too.

## Library

```rust
// Run the command line in this process. Return the exit status.
let status = podnet_netns::main_with_args(&["-x".into(), "socks5://127.0.0.1:1080".into(),
                                         "--".into(), "curl".into(), "https://example.com".into()]);
```

The public modules are also used alone:

| Module | Contents |
| --- | --- |
| `upstream` | Proxy specs, rules, SOCKS5 and HTTP `CONNECT` clients, chains, UDP ASSOCIATE. |
| `socks` | The server side of SOCKS5 and HTTP `CONNECT`. |
| `dns` | DNS query and answer encoding. |
| `listen` | `unix:/path` and `tcp://host:port` listen addresses. |
| `tailscale` | A dialer for the `ts-dial` endpoint of the `tailscaled` LocalAPI, and a SOCKS5 front door for it. |

## Tests

`tests/seccomp.rs` compiles small C programs (static and dynamic) and runs them
with both backends. A backend that the host does not permit is skipped. Each
test program has a 10-second alarm, so a lost reply fails the test.

`tests/pod_seccomp_diff.rs` (a test of the workspace root) runs one series
of socket calls on the host kernel and in a seccomp pod, and compares the
result or the errno of each call and the form of each address. A
difference fails the test, except a known difference that names its entry
of the record.

## DNS of the program (netns backend)

The netns backend gives the program a private `/etc/resolv.conf` with
`nameserver 10.66.255.254` (the gateway of the TUN, where the pod answers
DNS): a bind mount in a mount namespace of the program, so the file of the
host stays as it is. The resolver of the host (for example `127.0.0.53`) is
on the loopback of the host, which the namespace of the program does not
have. When the mount fails, a warning says so and the program keeps the file
of the host. The seccomp backend reads the DNS packets of the program
itself, with any resolver file.
