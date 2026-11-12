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

Two backends are available. `--backend auto` (the default) selects one.

| Backend | Needs | Mechanism |
| --- | --- | --- |
| `netns` | unprivileged user namespaces, and `/dev/net/tun` or a veth pair | The program gets a network namespace. A TUN device (else a veth pair) is its only interface. A userspace TCP/IP stack sends each flow to the upstream. |
| `seccomp` | seccomp with `SECCOMP_RET_USER_NOTIF` | A seccomp filter stops the socket system calls. The supervisor answers them and gives the program connected descriptors. |

Both backends answer DNS from a fake-IP pool (`198.18.0.0/15`; the
`seccomp` backend answers AAAA from `fd63:6672:7300::/96` too) and send the
name to the upstream. Thus no DNS query leaves the host. With the `netns`
backend, `--host-gateway` lets the program reach a service on the loopback
of the host at the gateway `10.66.255.254` (TCP and UDP, as the gateway of
podman and slirp4netns); without it, the gateway refuses. See
[`crates/podnet-netns/README.md`](../crates/podnet-netns/README.md) for all options.

## Upstreams

`-x SPEC` (or `--proxy SPEC`, or `--egress SPEC`) gives an upstream. It is repeatable: more than one `-x`
makes a chain, in the order given. Without `-x`, the upstream comes from `$ALL_PROXY`, `$HTTPS_PROXY` or
`$HTTP_PROXY`. A pod with no upstream is a usage error (exit 64).

* `socks5://[user:pass@]host:port` and `http://[user:pass@]host:port`.
* `socks5://unix:/path` and `http://unix:/path` (also `socks5h+unix:/path` and `http+unix:/path`): a proxy
  on a unix socket.
* `tailscale:/path/to/tailscaled.sock`: a tailnet ([networks.md](networks.md)).
* `netbird:[PORT]` through `--egress` (a SOCKS5 hop on `127.0.0.1:PORT`; [networks.md](networks.md)).
* `--serve LISTEN` runs no program. It serves SOCKS5, HTTP CONNECT and UDP ASSOCIATE on `unix:/path` or
  `tcp://host:port`, and sends the flows through the chain.

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
* `cidr:ADDR/PREFIX=WAY` matches an address.
* `WAY` is an upstream (`socks5://host:port`), `direct` (no proxy) or `reject` (or `block`). A rejected
  connect fails with `ECONNREFUSED` (netns: a reset). A rejected datagram gets `EACCES` (netns: it is
  dropped). `--serve` answers SOCKS5 `0x02` or HTTP 403.
* A rule that an earlier rule shadows (it can never match) gives a warning. A shadowed `reject` rule is an
  error (exit 64). `--allow-shadowed-rules` makes it a warning.
* `--default-action allow|reject` sets what the pod does with a target that no rule matches (default
  `allow`).
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

## Other options

* `--backend auto|netns|seccomp`; `--prefer-udp` (with `auto`: `seccomp` first when it works, because
  `netns` carries a datagram of 1500 bytes at most); `--name-backend` prints one line on stderr with the
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
* A pod holds the traffic of the program. The `netns` backend enforces it, except unix sockets of the file
  system and the descriptors that the program inherits. The `seccomp` backend is advisory: unix, netlink,
  vsock, raw and packet sockets, loopback TCP and inherited descriptors are not held.
* Exit status: the exit code of the program; 64 for a usage mistake (also no upstream); 3 when no backend
  can run on this host; 4 for a violation of `--fail-on-violation` when the program exits 0.

## Check the host: `podnet pod doctor`

`podnet pod doctor [--json]` measures what the host permits and says which backend can run
([doctor.md](doctor.md)). It exits 3 when no backend can run. When a program and WARP are the goal, use
`podnet warp run` and `podnet warp exec` ([warp.md](warp.md)): they run a pod over WARP.

