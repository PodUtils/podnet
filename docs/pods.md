# Pods: `podnet pod`

This page gives `podnet pod`: run a program with all its network traffic sent through an upstream. The
backends are in the library `crates/podnet-netns` (`crates/podnet-netns/README.md`).

`podnet pod` runs a program with all its
network traffic sent through a SOCKS5, HTTP or tailnet upstream. It does not
use `LD_PRELOAD`, so static binaries and Go binaries are also covered.

```sh
podnet pod -x socks5://127.0.0.1:1080 -- curl https://example.com
podnet pod -x http://proxy:3128 -- git clone https://example.com/repo
podnet pod -x socks5://unix:/run/proxy.sock -- ./static-binary
podnet pod -- curl https://example.com        # upstream from $ALL_PROXY or $HTTPS_PROXY
podnet pod doctor                             # which backend can run here
```

Three backends are available. `--backend auto` (the default) selects `netns` or `seccomp`. `netns-kernel` is opt in.

| Backend | Needs | Mechanism |
| --- | --- | --- |
| `netns` | unprivileged user namespaces, and `/dev/net/tun` or a veth pair | The program gets a network namespace. A TUN device (else a veth pair) is its only interface. A userspace TCP/IP stack sends each flow to the upstream. |
| `netns-kernel` | as `netns`, and `nf_tables` and conntrack in the namespace (`pod doctor` row `netfilter in a namespace`) | The same namespace with `lo` only. `nat OUTPUT REDIRECT` rules of the kernel send each TCP connection, each UDP datagram and each DNS query to listeners that the child makes (127.0.0.1, ports 61001 to 61003 of the new namespace, above the ephemeral range; no `getsockname`). The parent serves them. No TUN, no userspace stack. |
| `seccomp` | seccomp with `SECCOMP_RET_USER_NOTIF` | A seccomp filter stops the socket system calls. The supervisor answers them and gives the program connected descriptors. |

`--backend netns-kernel` (T-153) is not in the `auto` order: the flow table of the `netns` backend
stays the default, and the netfilter plane is an option for a host where the userspace stack is too slow.
It works only where the `netns` backend works (a refused user namespace refuses both). Its limits: IPv4
only; a destination on `127.0.0.0/8` stays in the pod; `--host-gateway` and `--inbound-dir` do not apply;
a ping of another address is answered by the pod itself, because an ICMP echo cannot be redirected (the
child sets `ping_group_range` to its mapped group, so a ping socket opens). The child installs the rules
after `unshare` and before `exec`, and sends the TCP listener, the UDP socket, the DNS socket and the
netfilter socket to the parent over the `SCM_RIGHTS` pair. The parent dials each flow with the rules and
`connect_upstream`, as the other backends do. It reads the original destination of a TCP connection with
`SO_ORIGINAL_DST`, and of a UDP datagram from the conntrack entry of the client.

The other backends answer DNS from a fake-IP pool (`198.18.0.0/15`; the
`seccomp` backend answers AAAA from `fd63:6672:7300::/96` too) and send the
name to the upstream. Thus no DNS query leaves the host. With the `netns`
backend, `--host-gateway` lets the program reach a service on the loopback
of the host at the gateway `10.66.255.254` (TCP and UDP, as the gateway of
podman and slirp4netns); without it, the gateway refuses. Port 53 of the
gateway is always the fake DNS. See
[`crates/podnet-netns/README.md`](../crates/podnet-netns/README.md) for all options.

The fake DNS answers a query over UDP and over TCP (the framing of RFC 7766).
The `seccomp` backend answers each connection to port 53; the `netns` backend
answers the gateway `10.66.255.254`, the nameserver of its `resolv.conf`.
When the netns backend cannot put its `resolv.conf` over `/etc/resolv.conf`
(for example, the host file is a mount of a deleted file), the program keeps
the host file, the pod prints one warning, and it answers port 53 of the
loopback of its namespace (UDP and TCP) and each other nameserver of that
file. A file with no nameserver (glibc then asks `127.0.0.1`) and a
nameserver such as `127.0.0.53` then work too.
Where a seccomp filter of the host refuses each UDP socket (the lab
`udp-denied` of pod-lab), the pod adds `use-vc` to `RES_OPTIONS` of the
program, and prints one line on stderr. A glibc program then sends each query
over TCP to the pod. A musl program cannot resolve a name there: musl reads
no `use-vc`, and its resolver stops when its UDP socket is refused. Give such
a program an address in place of a name.

## Upstreams

`-x SPEC` (or `--proxy SPEC`, or `--egress SPEC`) gives an upstream. It is repeatable: more than one `-x`
makes a chain, in the order given. Without `-x`, the upstream comes from `$ALL_PROXY`, `$HTTPS_PROXY` or
`$HTTP_PROXY`. A pod with no upstream is a usage error (exit 64).

* `socks5://[user:pass@]host:port` and `http://[user:pass@]host:port`.
* `socks5://unix:/path` and `http://unix:/path` (also `socks5h+unix:/path` and `http+unix:/path`): a proxy
  on a unix socket.
* `tailscale:/path/to/tailscaled.sock`: a tailnet ([networks.md](networks.md)).
* `direct`: no proxy. The pod dials each target from this process. `-x direct` and `--egress direct` are
  the same.
* `wg:FILE` is not a hop of a pod. Run `podnet route` with a `wg:FILE` network, and give its door to `-x`.
* `netbird:[PORT]` through `--egress` (a SOCKS5 hop on `127.0.0.1:PORT`; [networks.md](networks.md)).
* `--serve LISTEN` runs no program. It serves SOCKS5, HTTP CONNECT and UDP ASSOCIATE on `unix:/path` or
  `tcp://host:port`, and sends the flows through the chain. It is the front door of
  [proxies.md](proxies.md), over the chain and the rules. UDP ASSOCIATE and UDP in the stream go
  through the first upstream, which must be SOCKS5. BIND gets `0x07`. `--early-reply` answers a CONNECT
  before the dial (see [proxies.md](proxies.md), "Early reply").

At the start, the pod connects to each upstream (a 2 s limit). An upstream that does not answer gives the
line `warning: the upstream ... does not answer (...); each connection of the program through it fails with
ECONNREFUSED`. The program still runs. Each TCP `connect` through that upstream fails with `ECONNREFUSED`;
it does not wait for the time limit of the program. On the netns-kernel backend the connect works and the
connection is reset: the line then ends with `is reset`. `direct` gets no line.

When a connect of the program fails at the upstream (a name with no answer, a refused target, a dead
proxy) or at a rule (`reject`), the pod prints one line with the cause, for example `upstream for
nowhere.invalid:80 failed: direct TCP to nowhere.invalid:80: ...; the connect of the program is refused`
or `connect lab.test:80 refused by the rule domain:lab.test=reject (1 refused)`. The program sees only
`ECONNREFUSED` or a reset: in the pod, each name resolves, and the upstream resolves it. The line comes
once for each target, and at most for 64 targets; `-v` prints one line for each connect, and `podnet -q`
prints none. A failure through an upstream that the warning at the start named gets no second line.

By default the program does not get `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY`.
`--keep-proxy-env` keeps them.

## Rules

`-r SPEC` (`--rule`) sends a destination its own way. The first rule that matches wins.

```sh
podnet pod -x socks5://127.0.0.1:1080 \
  -r domain:.corp.example=socks5://10.0.0.1:1080 \
  -r cidr:10.0.0.0/8=direct \
  -r domain:.ads.example=reject -- ./program
```

* `domain:NAME=WAY` matches a name. `domain:.NAME=WAY` matches the name and each name under it.
* `cidr:ADDR/PREFIX=WAY` matches an address. It does not match a name: the fake DNS of the pod answers a
  name, and the pod does not look up its real address. Thus `cidr:140.82.0.0/16=reject` does not refuse
  `curl https://api.example.com/`, which connects by name. Use a `domain:` rule for a name.
* `port:N=WAY` and `port:N-M=WAY` match these ports of each host. `tcp:HOST:PORT=WAY` matches one host and one
  port. `HOST` is a name, an IPv4 address or `[IPv6]`. A name matches the same name only: no lookup. These
  rules match a TCP connect only. A datagram has no port for the rules, so it takes the next rule that
  matches (a `domain:` or a `cidr:` rule) or the `-x` chain. The reader is the one of `podnet warp proxy -r`
  (`pod-warp`). The shadow check covers the kinds: `cidr:127.0.0.1/32=a` before `tcp:127.0.0.1:5432=b`
  shadows the second rule, and `port:8000-8100=a` before `port:8080=b` does too.
* With `--loopback-rules` (backend `seccomp`), `tcp:127.0.0.1:5432=WAY` sends a connect to that port of the
  loopback to `WAY`; a connect to another loopback port does not match.
* `WAY` is an upstream (`socks5://host:port`), `direct` (no proxy) or `reject` (or `block`). A rejected
  connect fails with `ECONNREFUSED` (netns: a reset). A rejected datagram gets `EACCES` (netns: it is
  dropped). `--serve` answers SOCKS5 `0x02` or HTTP 403.
* A rule that an earlier rule shadows (it can never match) gives a warning. A shadowed `reject` rule is an
  error (exit 64). `--allow-shadowed-rules` makes it a warning.
* `--default-action allow|reject` sets what the pod does with a target that no rule matches (default
  `allow`).
* The egress policy of the process (`PODNET_POLICY`, [host-runtime.md](host-runtime.md), "The classified egress") decides before the rules, on
  each backend and with `--serve`. A destination that the policy refuses gets the answer of a `reject`
  rule, and the line names the rule of the policy, for example `connect example.com:443 refused by
  PODNET_POLICY: the rule "no-example" refuses the destination example.com:443 (any family), before any
  connect`. The flow log names the rule `policy`. `--learn` does not send such a flow through the `-x`
  chain.
* With the `seccomp` backend, `127.0.0.0/8` goes direct (or to a listener of the program itself).
  `--loopback-rules` applies the rules to a TCP connect to `127.0.0.0/8` and `::1` too.

## Audit

* `--flow-log FILE` appends one JSON line to FILE for each decision (time, target, rule or default, chain,
  verdict) and one at the close of each relayed TCP flow (`bytes_out`, `bytes_in`).
* `--learn FILE` writes each destination of the run to FILE, one `PROTO TARGET` line each. A flow that a
  rule refuses goes through the `-x` chain.
* `--fail-on-violation FILE` refuses each destination that is not in FILE (a file of `--learn`). A learned
  destination that a rule refuses is also a violation. The pod writes each violation on stderr and exits 4
  when the program exits 0.

## Report

* `--report FILE` writes one JSON object to FILE when the pod starts, and again when it ends. Each write
  replaces the file with a rename of a temporary file in the same directory. The mode is 0600.
* The keys: `schema` (1), `version` (of podnet), `backend` (`netns`, `netns-kernel`, `seccomp` or null),
  `choice` (the reason of the choice), `hold` (`enforced`, `advisory` or `none`: the words of `podnet pod doctor --json`),
  `not_held` (the ways out that the backend does not hold), `udp_limit` (the most bytes of a UDP datagram:
  65507 for `netns`, null for `netns-kernel` and `seccomp`), `exit` and `reason`.
* `exit` is null in the first write. In the last write it is the exit status of `podnet pod`. `reason` is
  the cause of a failure (exit 64 or 3), else null.
* A pod that cannot start writes the report with `backend` null, `hold` `none` and its `reason`. Exit 3 of a
  child of the pod has the reason `the pod did not start the program, or the program exited 3`.
* The release file `pod-report.schema.json` (source: `docs/schema/pod-report.schema.json`) is the JSON
  Schema of the report. `--report` does not work with `--serve`. A file that cannot be written at the start
  is exit 64.

## Other options

* `--backend auto|netns|netns-kernel|seccomp`; `--prefer-udp` (with `auto`: `seccomp` first when it works); `--name-backend` prints one line on stderr with the
  backend, its limits and the reason of the choice.
* `--map NAME=ADDR` resolves NAME to ADDR locally (repeatable).
* `--host-gateway` (netns): a connect to `10.66.255.254` and a port goes to `127.0.0.1` on the host.
* `--inbound-dir DIR` (seccomp): where a bound port is exposed as a unix socket. The directory must be of
  this user and not writable by the group or by others. The default is
  `$XDG_RUNTIME_DIR/podnet-netns-inbound`, else `$TMPDIR/podnet-netns-inbound-UID`.
* `--session DIR` (seccomp): share the inbound listeners and the fake DNS table with each pod of DIR.
  `podnet warp exec --join` uses it ([warp.md](warp.md)).
* `--flow-budget N` (seccomp): the most sockets that the pod tracks at one time (64 to 1048576). A socket
  call above it gets `EMFILE`. The default comes from `RLIMIT_NOFILE`.
* `--allow-raw icmp` (seccomp): a raw socket of `AF_INET` or `AF_INET6` (`SOCK_RAW`) goes to the kernel.
  With `CAP_NET_RAW`, its packets go out of the host past the upstream of the pod. By default the filter of
  the kernel refuses the socket with `EPERM`. A packet socket (`AF_PACKET`, `AF_XDP`, or `SOCK_PACKET` of
  `AF_INET`) gets `EPERM` also with `--allow-raw icmp`. A ping socket (`SOCK_DGRAM` with `IPPROTO_ICMP`) is
  emulated in each case: an echo to the gateway `10.66.255.254` gets an answer, another destination gets
  `EACCES`.
* A pod holds the traffic of the program. The `netns` backend enforces it, except unix sockets of the file
  system and the descriptors that the program inherits. The `seccomp` backend is advisory: unix, netlink
  and vsock sockets, loopback TCP and inherited descriptors are not held; a raw socket of
  `AF_INET` and `AF_INET6` and each packet socket get `EPERM` (see `--allow-raw icmp`).
* Exit status: the exit code of the program; 64 for a usage mistake (also no upstream); 3 when no backend
  can run on this host; 4 for a violation of `--fail-on-violation` when the program exits 0; 78 for a bad
  `PODNET_RESOLVE` or `PODNET_POLICY`, before the program starts.
* Stop signals: a SIGTERM, SIGINT or SIGHUP that a process sends to `podnet pod` (`kill`, a service manager)
  goes to the program, in each backend. podnet prints one line (`podnet pod: SIGTERM: stopping the program`),
  waits for the program, and exits with its status (128+N when the signal ended it). Then no process of the
  pod stays. A program that does not end in 10 s, or a second stop signal, gets SIGKILL with each process of
  the pod, and podnet exits 128+N. A stop signal of the terminal (Ctrl-C) reaches the program directly, as
  each process of the foreground group; podnet waits for the program.
* A program that cannot run: each backend finds the program as `execvp` does (a name with a `/` as it is,
  another name in `PATH`) and starts it with `execve`. A script needs a `#!` line: the pod does not give a file
  with no `#!` and no ELF header to `/bin/sh`. A program that cannot run (not found, no execute bit, a
  directory, a format that the kernel refuses) gives one line with the program and the cause, and exit 3, on
  each backend.

## Check the host: `podnet pod doctor`

`podnet pod doctor [--json]` measures what the host permits and says which backend can run
([doctor.md](doctor.md)). It exits 3 when no backend can run. Its verdict of `netns` is the probe that
the start of the backend runs: an `unshare` of a network namespace (with or without a user namespace), with
`lo` and a TUN or a veth pair in it, and a TCP connect to `127.0.0.1` that is not refused. `netns-kernel`
needs the same and the row `netfilter in a namespace`. `auto` takes `netns` only when the probe passes.
`--backend netns` and `--backend netns-kernel` run the probe before the start. Where it fails, they print
one line with the cause and the next step (`use --backend seccomp`) and exit 3, and the report has
`backend` null, `hold` `none` and that line as `reason`. Where `auto` takes `seccomp` because `netns` cannot
run, the line of `--name-backend` does not give `--backend netns` as a next step. When a program and WARP are the goal, use
`podnet warp run` and `podnet warp exec` ([warp.md](warp.md)): they run a pod over WARP.

