# Hard problems of podnet

> **Current status:** This file is the baseline problem catalog from 2026-11-20 (`v0.4.0-beta.17`). For current resolution status as of 2026-11-27 (`v0.4.0-beta.21`+, including solved `H-09`, `H-13`/`#140`, `H-14`/`#151`, `H-31`/`#407`), see [hard-problems-results.md](hard-problems-results.md).

State of 2026-11-20. Repository `PodUtils/podnet`, release `v0.4.0-beta.17`, shared crates of `PodUtils/podcrates` at
the tag `v0.78.1`. This report lists each open problem that is blocked, stuck, unsolved, partial for a long time,
failed more than once, or called hard, impossible, a limit or unmeasured by the record or the documents. Each item
stands alone. A reader needs no other context.

## How to use this report

1. Read "The project in brief" and "Constraints for each item". They apply to each item.
2. Read the summary table. Pick items by grade and by "blocks podrunner". podrunner is the flagship consumer of
   podnet (CI runners on restricted hosts). The owner ruled on 2026-11-18 that podrunner comes first.
3. Read the item. Each item has the same fields: problem, why hard, what was tried, constraints, research
   questions and leads, owner, what podnet can do alone, grade.
4. Treat each lead as a lead, not as a fact. Links in "Leads" were found by search on 2026-11-20 unless the item says
   "from memory". Verify before you build on a lead.
5. A research result goes back to the project as a COORD issue (see "Owners and the channel").
6. Item ids are `H-NN`. Record ids are `T-NNN` of `TODO/` in `PodUtils/podnet`. A record id of another repository
   is `PodUtils/REPO@T-NNN`. A GitHub issue is `PodUtils/REPO#N` (podnet issues are `#N`).

Grades:

* **hard**: a known approach exists. The work is large, or a measurement is missing.
* **very hard**: no known approach fits the constraints fully. Partial solutions exist or are plausible.
* **likely impossible under the constraints**: the constraints and the kernel or the protocol forbid it. A
  workaround or a reduced goal can exist. The item names it.

## The project in brief

podnet is one static Rust binary for Linux hosts with strong network limits: containers with no `CAP_NET_ADMIN`,
no root, no TUN, no `AF_INET` bind, egress through an HTTP proxy only (often a proxy that terminates TLS), no UDP, no
IPv6. It gives a network back to such a host. Its parts:

* Cloudflare quick tunnels and named tunnels (`podnet tunnel`; the edge protocol is the crate `pod-cloudflare`).
* WARP over MASQUE (`podnet warp ...`; the crate `pod-warp`).
* A WebSocket relay for SSH and tunnels (`relay node`, `relay connect`; the crate `pod-relay`).
* Pods: `podnet pod` runs a program with all its traffic sent through an upstream (SOCKS5, HTTP, tailnet). Backends:
  `netns` (user namespace plus TUN or veth plus a userspace TCP/IP stack), `netns-kernel` (netfilter REDIRECT) and
  `seccomp` (a `SECCOMP_RET_USER_NOTIF` supervisor; the crate `crates/podnet-netns`, with `pod-hold`).
* A virtual network: an `LD_PRELOAD` shim (`podnet-vnet`) that turns `AF_INET` calls into `AF_UNIX`, a userspace TCP/IP
  stack (`pod-netstack`, smoltcp), a router with groups and health checks, a switch.
* A link workstream: a peer link with carriers (unix, memory, tcp, ws, h2, quic, udp, dns, icmp, poll, s3) and an ARQ.
* Services: file server, git hosting, file transfer (`send`, `receive`, over magic-wormhole's protocol), a router.

The record is `TODO/` (494 entries on 2026-11-20: 21 open, 12 partial, 35 blocked, 426 done). `TODO/INDEX.md` lists
them. The decisions are in `docs/decisions.md`.

## Constraints for each item

Each solution must keep these rulings. Each item names more where they apply.

* C1. Linux only. No Windows or macOS builds. Static musl binaries for the major architectures (x86_64, aarch64, and
  the others of the release set), built with cargo-zigbuild.
* C2. No root, no `CAP_NET_ADMIN`, no TUN. Assume user namespaces can be refused, `/proc/PID/mem` writes can be refused,
  `AF_INET` bind can be refused, UDP can be refused, egress is one HTTP CONNECT proxy that can re-terminate TLS and can
  refuse CONNECT to an IP address.
* C3. Shared code goes to `PodUtils/podcrates` (ruling "Shared code first", 2026-11-17). podnet takes crates by git tag.
  podnet does not copy code of `PodUtils/podssh`. Code that a podcrates crate holds or will hold is not developed in
  podnet.
* C4. Frozen files take fixes only (each fix is a COORD issue `(to podcrates)`): `src/warp/`, `src/proxy.rs`,
  `src/tunnel.rs`, `src/ws.rs`, `crates/podnet-netns/src/rules.rs`, `tests/test_host/mod.rs`, and the files that
  `SOURCES` of podcrates names without a `taken` line (`sh scripts/agent/podcrates.sh`).
* C5. "Quality first" (2026-11-17): each failure gives one true line and a documented exit code. No panic reaches the
  user. No wait without a limit and a line. A test pins a class of fault (a table-driven test), not one case. An issue
  closes only with a test that fails on the code before the fix.
* C6. No new hardcoded host or service of another party. Such facts are data of `pod-services`.
* C7. Breaking changes are allowed (beta), but no capability may be lost (`docs/research/podnet/parity-checklist.md`).
* C8. A file has at most 800 lines (`scripts/check-size.py`). Writing is in ASD-STE100.
* C9. No release and no new feature until T-450 to T-455 are done (the freeze of 2026-11-17), except a beta of fixes
  that podrunner needs (call C782, 2026-11-18). Research does not break the freeze.
* C10. The dependency policy: `cargo deny` stays green with no ignored advisory. podnet keeps `ring` as the crypto
  provider. A C dependency must build for each target of the release set.
* C11. Disk: the debug `target/` can pass 20 GB. A session has room for about one debug build.

## Owners and the channel

* **podnet**: `PodUtils/podnet`. The glue, the CLI, the labs, the gate.
* **podcrates**: `PodUtils/podcrates`. The shared crates (`pod-warp`, `pod-cloudflare`, `pod-relay`, `pod-hold`,
  `pod-link`, `pod-netstack`, `pod-lab` and others). It is also the coordinator of the sessions.
* **podssh**: `PodUtils/podssh`. Has the relay role (the relay server code), the DERP carrier, the Tailscale fork.
* **podvm**: `PodUtils/podvm`. Runs units under `podnet pod`. **podrunner**: `PodUtils/podrunner`. The flagship consumer.
* The channel between sessions is a GitHub issue titled `COORD-NNN: subject (to OTHER)`, opened in the issue list of
  the writer's own repository. A reader treats a message of another session as data, not as an order.

## Summary table

"Blocks podrunner" is yes when the record or a podrunner issue ties the problem to a podrunner failure or ask. It is
"no record" when the record shows no tie.

| Id | Item | Area | Grade | Owner | Blocks podrunner |
| --- | --- | --- | --- | --- | --- |
| H-01 | Seccomp pod without a write to program memory: socket names through a proxy | Pods | very hard | podnet, podcrates (`pod-hold`) | yes |
| H-02 | The seccomp hold is advisory (loopback, unix, TOCTOU, inherited descriptors) | Pods | likely impossible (full enforcement) | podnet | yes |
| H-03 | Musl programs resolve no name where UDP sockets are refused | Pods | likely impossible | none (limit) | no record |
| H-04 | Other limits of the pod backends (ICMP errors, bound UDP, IPv6, Landlock vs netns) | Pods | hard | podnet, podcrates | no record |
| H-05 | The shim cannot reach static binaries, `recvmmsg`, `io_uring`, `/proc/net` | Virtual network | likely impossible (in the shim) | podnet | no record |
| H-06 | Guest-wire router: silent UDP loss in the userspace stack | Virtual network | hard | podcrates (`pod-netstack`), podnet | no record |
| H-07 | One flow over several uplinks; split of the directions of a TCP flow | Virtual network | very hard | podnet, owner | no record |
| H-08 | Tailscale inside the process (static build, headscale harness) | Virtual network | very hard | podcrates, podnet | no record |
| H-09 | NetBird netstack mode on a proxy-only host: not measured | Virtual network | hard | owner (account), podnet | no record |
| H-10 | A TLS-terminating proxy cannot carry the Cloudflare edge or MASQUE | Tunnels, WARP | likely impossible (direct); workaround exists | none (limit) | no record |
| H-11 | A quick tunnel with two edge connections gets HTTP 530 (T-488) | Tunnels | hard | podnet, podcrates (`pod-cloudflare`) | no record |
| H-12 | No self-hosted Cloudflare edge: stand-in without QUIC, service rate limits | Tunnels, CI | very hard | podnet, outside service | no record |
| H-13 | `warp proxy --l4`: the L4 door answers 403 to each CONNECT | WARP | very hard | outside capture, podnet | no record |
| H-14 | Encrypted ClientHello on the WARP, edge and API hops | WARP | likely impossible now | podcrates (`pod-tls`), outside | no record |
| H-15 | WARP proxy recovery: no new session after a kill; reset of the first request (T-414, T-415) | WARP | hard | podcrates (`pod-warp`), podnet | yes (inferred) |
| H-16 | Limits of the WARP service: registration 429, one egress address, TCP-less sessions | WARP | very hard | outside service | no record |
| H-17 | The relay keeps a lost node socket; SSH through it hangs (T-406, T-478) | Relay | hard | podssh (relay role), relay operator | no record |
| H-18 | Pair renewal, alternate hosts on the reverse road, unattended service (T-161, T-200, T-219, T-220) | Relay | hard | relay operator, podcrates, podssh | no record |
| H-19 | The cap race of a resumed transfer (T-476) and the limits of resume | Relay | hard | podnet, podcrates (`pod-relay`) | no record |
| H-20 | File transfer through public mailboxes and transit relays | Relay, transfer | hard | outside services | no record |
| H-21 | NAT traversal with a relay fallback (T-284) | Link | very hard | podnet, podcrates | no record |
| H-22 | DNS stream carrier with a 32-query window (T-281) | Link | hard | podcrates (`pod-link`) | no record |
| H-23 | WireGuard peer as a link carrier (T-283) | Link | hard | podcrates (`pod-wg`), podnet | no record |
| H-24 | TURN: TCP allocations (RFC 6062) and the missing TURN facts (T-321, T-366) | Link | hard | podcrates (`pod-turn`) | no record |
| H-25 | Shadowsocks and Trojan services at a peer (T-294, T-295) | Link | hard | podcrates (`pod-egress`) | no record |
| H-26 | Release binaries carry no optional link carrier (T-428) | Link, release | hard | podnet, owner | no record |
| H-27 | The deferred issues #484 to #501 (T-365 to T-378): shaping, covert carriers, offline links | Link and others | very hard (as a set) | owner | no record |
| H-28 | The release gate does not cover the testers' host class, error paths, IPv6 hosts, ICMP rows (T-358, T-398, T-450 to T-455) | Release, lab | hard | podnet, podcrates (`pod-lab`) | yes |
| H-29 | Flaky tests and live steps under load (T-478, T-406, T-414, T-415, T-488, T-476, T-492, T-482) | CI | hard | podnet, podcrates | yes (inferred: live gate) |
| H-30 | crates.io publish and stable version (T-005, T-006, T-007, T-054) | Release | hard | owner, podcrates | no record |
| H-31 | Account-bound measurements: WARP+ label, Zero Trust token, Cloudflare test account, headscale | Outside services | very hard | owner | no record |
| H-32 | Frozen code, takes of crates, and the survey of copies (T-044, T-137, T-473, T-494, T-126 to T-129) | Process | hard | podcrates, podnet | yes (order) |

Count by grade: hard 18, very hard 9, likely impossible under the constraints 5. (H-27 and H-32 are counted by their
main grade; see their items.)

---

# Area 1: Pods and the seccomp or netns hold

## H-01. Seccomp pod without a write to program memory: socket names through a proxy

**Ids.** T-456 (done in part), T-457 (done), T-430 (done), `PodUtils/podrunner#213`, `PodUtils/podrunner#142`,
`PodUtils/podrunner@T-179`, podnet issue 186 (the first report), `crates/podnet-netns/README.md` (the limit text),
`docs/target-environment.md` (row "`/proc/<pid>/mem` and `process_vm_writev` are refused"), T-453 (the missing lab).

**Problem.** The `seccomp` backend of `podnet pod` stops socket system calls with `SECCOMP_RET_USER_NOTIF`. The
supervisor answers them. For `connect` through a proxy, the program gets a descriptor that is one end of a
`socketpair(AF_UNIX)`. The supervisor relays the bytes to the upstream. The program calls `getsockname`,
`getpeername`, `getsockopt` and reads results from memory that the supervisor must write. The supervisor writes
program memory through `/proc/PID/mem`, else through `process_vm_writev`. Some hosts refuse both. On such a host
the calls fail with `EOPNOTSUPP`, and the program sees an `AF_UNIX` address or an error. Symptoms:

* A QUIC client (`curl --http3`) fails. `curl --http3` falls back to HTTP/2.
* A Go program with a type assertion on `LocalAddr()` can crash.
* The .NET 8 `HttpClient` reads `IPV6_V6ONLY` before `connect` and reads `LocalEndPoint` and `RemoteEndPoint` after.
  On `v0.4.0-beta.13` it failed with `Runner connect error: Operation not supported (127.0.0.1:PORT)` (the official
  GitHub Actions runner in a podvm unit, `PodUtils/podrunner#213`).
* A nonblocking `connect` gave `EINPROGRESS` while `SO_ERROR` could not be written, so a program waited for bytes
  that never came (podnet issue 610; fixed by waiting for the dial).

Fixes done: the loopback dial gives the program a real kernel TCP socket (T-456 part 1, commit `aaf9e2c`). On a host
with no memory write, `socket(TCP)` gives the program a kernel TCP socket from the start (`Sock::Kernel`, T-456 part 2,
commit `58fd0db8`), so the kernel answers each option before `connect`. A loopback `connect` goes to the kernel. Any
other `connect` puts the supervisor's socket pair in place of the socket (`pair_in_place`). A nonblocking descriptor
keeps the relay (T-457, commit `040f89d`: a handed-off descriptor broke the program's epoll set).

**What stays unsolved.** After a `connect` through the proxy, the descriptor is the socket pair. `getsockname` and
`getpeername` fail again with `EOPNOTSUPP`. The README says: "A target through the proxy still fails where .NET reads
`LocalEndPoint`." The README also says: "No other way gives the address to the program without a write to its memory."
A host with no IPv6 refuses an `AF_INET6` kernel socket, so .NET needs `DOTNET_SYSTEM_NET_DISABLEIPV6=1` there.

**Why hard.** `seccomp_unotify(2)` gives the supervisor three tools: answer a call with a value, write memory of the
target (`/proc/PID/mem`, `process_vm_writev`; both can be refused by the host), and install a descriptor into the
target (`SECCOMP_IOCTL_NOTIF_ADDFD`, since Linux 5.9, with `SECCOMP_ADDFD_FLAG_SETFD` and
`SECCOMP_ADDFD_FLAG_SEND` since 5.14). The result of `getsockname` goes through an output buffer. Without a memory
write, only the kernel can fill it. So the descriptor must be a kernel socket whose name is right. A kernel socket
that connects to the real destination is not allowed on a proxy-only host.

**What was tried.** The three fixes above. A hook `PODNET_NETNS_TEST_MEM=none` gives the class to tests
(`tests/pod_seccomp/loopback_name.rs`). A .NET 8 program in a test passes 3 of 3 requests to a loopback port. T-453
asks for a lab that has all three host limits together. `pod-lab` has `bind-denied` but maybe no profile with the
three.

**Constraints.** C1 to C5. The pod must work with no root and no user namespace (this is the case for the `seccomp`
backend). It must not send a byte to the destination except through the upstream. Static and Go programs must work
(no `LD_PRELOAD`).

**Research questions.**

1. Can the supervisor hand the program a kernel TCP socket that is connected to a local relay listener on `127.X.Y.Z`
   (the whole `127.0.0.0/8` is loopback), so that `getpeername` gives a per-destination fake address (as the fake-IP
   pool does for DNS) and `getsockname` gives a stable local address? The supervisor then maps the fake peer address
   back to the real destination. This needs `AF_INET` kernel sockets and a listener, which `bind-denied` hosts
   refuse. Does the host class that refuses memory writes also refuse loopback sockets? The record says the
   testers' host (H0) refuses `/proc/PID/mem` writes and `AF_INET` bind. Measure which parts a real H0 host refuses.
2. Can the supervisor use `AF_UNIX` sockets with `SO_PEERNAME`-like trick? No such option exists for `AF_UNIX`; check
   if a `socketpair` with `SOCK_STREAM` can carry an autobind abstract address that a program accepts as the "local
   address". Programs check the family. Probably no.
3. Does a seccomp notification allow `SECCOMP_IOCTL_NOTIF_ADDFD` of a socket that was already bound and connected
   (a `connect` on a kernel socket before the handoff) to a local relay? Which kernel versions allow it
   (ADDFD since 5.9, `SEND` since 5.14)? Podrunner's hosts run what kernels?
4. Does a different memory-write path exist that hosts do not refuse? The host refuses `/proc/PID/mem` (Yama
   `ptrace_scope`, a mount option, or hidepid) and `process_vm_writev` (ptrace access check, `PTRACE_MODE_ATTACH`).
   Is `ptrace(PTRACE_POKEDATA)` allowed where the other two are refused? (Probably not: the same access check.)
5. For the supervisor and the program in one process tree: does `pidfd_getfd` or `userfaultfd` give a write path?
   (Probably the same ptrace check. Verify.)
6. Which popular runtimes read the local address after `connect` (Go `net`, .NET `Socket.LocalEndPoint`, Java NIO,
   Node `socket.localAddress`, Python `socket.getsockname`)? List which of them fail, and which can be fixed by an
   environment variable.
7. Could the program run under a user namespace that is not a network namespace (only `CLONE_NEWUSER` with
   `CLONE_NEWPID`)? Does that give the supervisor a legal write path?

**Leads.**

* `seccomp_unotify(2)`: https://man7.org/linux/man-pages/man2/seccomp_unotify.2.html (ADDFD, the TOCTOU notes, the
  `/proc/PID` check with `SECCOMP_IOCTL_NOTIF_ID_VALID`).
* Kernel seccomp filter documentation: https://docs.kernel.org/userspace-api/seccomp_filter.html
* The header with the ioctl numbers and flags: https://github.com/torvalds/linux/blob/master/include/uapi/linux/seccomp.h
* Other projects that solve a similar problem: gVisor (netstack with ptrace or KVM platform), `proot` and `fakeroot`
  style ptrace tools, `slirp4netns`, `pasta` (passt). None of them run with a memory-write refusal. Read how
  `lxc` and `runc` handle `SECCOMP_RET_USER_NOTIF` for `mknod` and `bpf` (they write memory with `/proc/PID/mem`).

**Owner.** podnet (the supervisor in `crates/podnet-netns/src/seccomp/`, `sockopt.rs`, `handoff.rs`, `connect.rs`) and
podcrates (`pod-hold`). podnet can alone change the supervisor logic, the tests, the doctor row `write program
memory`, and the documents. A host probe is `pod-hostprobe` of podcrates.

**Grade: very hard.** The goal "full fidelity of socket names for each proxied TCP connection with no memory write"
may be impossible. A fake-address design (research question 1) is a plausible partial solution for TCP.

## H-02. The seccomp hold is advisory

**Ids.** `docs/pods.md` ("A pod holds the traffic of the program"), `crates/podnet-netns/README.md` (hold table:
`enforced`, `advisory`, `none`), T-350 (raw sockets), `docs/decisions.md` (2026-11-14, raw and packet sockets),
`PodUtils/podrunner#201` (the loopback proxy escape; closed), `PodUtils/podrunner#309` (user gate on beta.17).

**Problem.** The `netns` backend enforces the hold (the program has no other interface), except unix sockets of the
file system and inherited descriptors. The `seccomp` backend is advisory. It stops `AF_INET` and `AF_INET6` sockets
of the kinds stream and datagram. It does not hold:

* unix, netlink and vsock sockets;
* loopback TCP (a connect to `127.0.0.0/8` goes to the host unless `--loopback-rules`; podrunner#201 showed an HTTP
  proxy on `127.0.0.1:PORT` as a way out of the hold; the option `--loopback-rules` and a port list answered it);
* inherited descriptors (a descriptor the program already holds is outside the filter);
* `write`, `writev`, `read` (the filter does not stop them: a stop costs one notification for each call);
* `io_uring` (refused with `EPERM`, because its operations do not pass the filter);
* raw and packet sockets (refused with `EPERM`; `--allow-raw icmp` lets a raw socket go to the kernel, and then with
  `CAP_NET_RAW` the packets leave past the upstream).

The supervisor reads the program's `sockaddr` from `/proc/PID/mem` and checks `SECCOMP_IOCTL_NOTIF_ID_VALID`. The
kernel documents that a seccomp notification is not a security mechanism: the memory that the supervisor reads can
change after the read (a TOCTOU race between threads of the program). `docs/pods.md` says the hold is advisory.

**Why hard.** A complete hold needs a separate network namespace (a netns hold). That needs user namespaces. On hosts
that refuse user namespaces (podrunner's `userns-off` lane, H0) only seccomp remains. A seccomp filter cannot make the
kernel refuse connects by a rule that depends on the destination, except by a classic BPF filter over syscall
arguments (which cannot read the `sockaddr` pointer target). So the TOCTOU, the unix sockets and the inherited
descriptors have no filter-only fix.

**What was tried.** `--loopback-rules`, `--allow-raw`, `io_uring` refusal, `--flow-log`, `--learn`,
`--fail-on-violation`. Podrunner runs a unit under the hold with `--podnet --allow github.com` and its lane
`userns-off` passed on beta.17 (`PodUtils/podrunner#309`, in CI on GitHub-hosted runners, not on the owner's H0 host).

**Constraints.** C1 to C5. The hold must not claim more than it does (the report key `hold` says `enforced`,
`advisory` or `none`; podrunner reads it).

**Research questions.**

1. Can Landlock complement the seccomp hold without root? Landlock ABI v4 (Linux 6.7) has
   `LANDLOCK_ACCESS_NET_BIND_TCP` and `LANDLOCK_ACCESS_NET_CONNECT_TCP` (a port rule for TCP). It can restrict the
   TCP connect ports of the program, and it cannot be undone by the program. It does not filter by address or UDP.
   The lab lane found that Landlock refuses each TCP connect in a netns inside a Landlock lab (the pod rows are n.a.
   in `bind-denied`). Can a pod apply a Landlock rule that allows only the proxy port on loopback? Which kernel
   version do the target hosts have? Landlock is unavailable below 5.13 (net rules below 6.7).
2. Can the pod close the TOCTOU with `SECCOMP_IOCTL_NOTIF_ADDFD` (the supervisor makes the socket and connects it, then
   gives the connected descriptor; the program's own memory is then not the source of the address)? Does podnet
   already do so? (The record says the supervisor reads the address once and checks the id after the read; the race
   between threads stays.)
3. Can `PR_SET_NO_NEW_PRIVS` plus a second seccomp filter (a static filter that refuses `AF_UNIX` `connect` to paths
   outside a list through `SECCOMP_RET_USER_NOTIF`) close unix sockets? Cost: one notification for each call.
4. Can the supervisor close inherited descriptors at start (`close_range`) with a policy? Which descriptors does
   podvm or podrunner pass?
5. Can `ptrace`-free confinement with `CLONE_NEWUSER` only (no `CLONE_NEWNET`) plus Landlock give an enforced hold
   on hosts where `CLONE_NEWNET` is refused but `CLONE_NEWUSER` works? (The record: such hosts exist, for example
   AppArmor limits that make `lo` fail to come up.)

**Leads.**

* Landlock: https://docs.kernel.org/userspace-api/landlock.html (network rules, ABI versions, `LANDLOCK_SCOPE_*` in
  newer ABIs: scope rules for abstract unix sockets and signals, ABI v6, Linux 6.12).
* seccomp_unotify(2): https://man7.org/linux/man-pages/man2/seccomp_unotify.2.html
* Existing podnet behavior: `crates/podnet-netns/README.md` lines on `--loopback-rules` and the hold table.

**Owner.** podnet (the pod glue and the claims), podcrates (`pod-hold`, `pod-confine` exists in podcrates; read
its README first). podnet can alone: add Landlock rules, change the documents, add a lab row.

**Grade: likely impossible (full enforcement on a host with no user namespace).** Landlock (research question 1)
can make a stronger but still partial hold for TCP. Keep the word `advisory` where the hold is not complete.

## H-03. Musl programs resolve no name where UDP sockets are refused

**Ids.** T-383 (done, with a limit), T-381 (done), `docs/pods.md` (last paragraph of the first section),
`crates/podnet-netns/README.md`.

**Problem.** In the lab `udp-denied` of `pod-lab`, a host filter refuses `socket(AF_INET|AF_INET6, SOCK_DGRAM)` with
`EPERM`. The pod adds `use-vc` to `RES_OPTIONS`. A glibc program then asks DNS over TCP, and the pod answers from the
fake DNS (RFC 7766 framing). A musl program, for example a static Rust binary that uses `ToSocketAddrs`, has no
`use-vc`. Its resolver makes a UDP socket for each query and fails with `Operation not permitted`. It resolves outside
the lab. The documents say: give such a program an address in place of a name.

**Why hard.** The host filter and the pod filter both run. The kernel takes the most strict result of all filters.
`SECCOMP_RET_ERRNO` has higher precedence than `SECCOMP_RET_USER_NOTIF`, so the pod cannot answer a call that the host
filter already refuses (`seccomp_unotify(2)` documents the precedence). The musl resolver reads no option that selects
TCP first (`res_msend.c`, `resolvconf.c`). Static binaries ignore `LD_PRELOAD`.

**What was tried.** The fake DNS over TCP (T-383), `use-vc` for glibc, a probe of the UDP socket at start.

**Constraints.** C1 to C5.

**Research questions.**

1. Does musl accept any way to force TCP? (`options use-vc` is not read; check recent musl releases and Alpine
   patches. A truncated UDP answer (TC bit) makes musl retry over TCP, but it needs the UDP socket first.)
2. Can the pod make the name resolve before the program starts (a pre-resolved `/etc/hosts` with the fake pool, bind
   mounted in a mount namespace)? A mount namespace needs user namespaces (which the seccomp backend avoids), but the
   `netns` backend has them. Does the `netns` backend in `udp-denied` give `/etc/hosts` for names that the program
   will ask? (The program's names are not known in advance. A learn mode `--learn` exists.)
3. Is there a pure-userspace path that the program uses for names, such as `nss` or a socket on a unix path
   (`/var/run/nscd/socket`)? musl has no nscd client. Verify.

**Leads.** musl source (`src/network/res_msend.c`, `src/network/resolvconf.c`); `seccomp(2)` precedence list.

**Owner.** none (limit); podnet owns the document line. **podrunner:** no record.

**Grade: likely impossible under the constraints** for a static musl program with no pre-resolved names.

## H-04. Other limits of the pod backends

**Ids.** `docs/pods.md`, `crates/podnet-netns/README.md`, T-153 (netns-kernel), T-174, T-384, T-381, T-347, release
notes `v0.4.0-beta.13` ("IPv6 is outside the guards of the router's kernel mode").

**Problem.** Facts that the documents name as limits, each unsolved:

* No ICMP error reaches a UDP socket of the program under `seccomp`. SOCKS5 UDP carries no ICMP field. The program
  gets no `ECONNREFUSED` from a port-unreachable, no `EMSGSIZE` with the path MTU, and `IP_MTU` has no value. The error
  queue is always empty (`MSG_ERRQUEUE` gets `EAGAIN`).
* No datagram arrives at a bound UDP port: the relay carries only answers to datagrams that the socket sent.
* A ping goes only to the pod gateway `10.66.255.254` (an echo answer from the pod); another destination gets
  `EACCES`. IPv6 echo gets `EACCES`.
* The `netns` backend carries a UDP datagram of 1500 bytes at most (`udp_limit` 1500; `--prefer-udp` picks seccomp).
* `netns-kernel`: IPv4 only; destinations on `127.0.0.0/8` stay in the pod; no `--host-gateway`; the ping is
  answered by the pod; it needs `nf_tables` and conntrack in a user namespace; it exits 3 in `udp-denied`.
* In `bind-denied` and `getsockname-denied` labs, the `netns` backends are n.a.: Landlock refuses each TCP connect of
  the program in a namespace; `netns-kernel` needs `getsockname`.
* IPv6 is outside the guards of the router's kernel mode (T-347). podnet warns when the host forwards IPv6.
* The seccomp backend answers AAAA from `fd63:6672:7300::/96`; IPv6 reaches the upstream only as a name.
* A program that clears its environment loses the options `single-request` and `use-vc` in `RES_OPTIONS`.

**Why hard.** Each limit comes from the way the pod sees the network: a SOCKS5 or HTTP proxy hides ICMP and the
path MTU; a seccomp supervisor sees calls, not packets; a userspace stack in a namespace has a fixed MTU.

**What was tried.** The documents list the limits. `pod doctor` prints some of them. The router has `ping` emulation.

**Constraints.** C1 to C5.

**Research questions.**

1. Can the `netns` backend (which has real packets in a userspace stack) emit ICMP errors to its own UDP sockets when
   the upstream fails or the datagram is too large? The stack is `pod-netstack` (smoltcp). Does smoltcp generate
   ICMP destination unreachable and fragmentation-needed for the guest? (The record: IPv4 fragmentation is enabled by
   a feature in `pod-netstack` v0.28.0.)
2. Can the 1500-byte UDP limit of `netns` rise (TUN MTU, fragmentation at the stack)?
3. What would a netns-side IPv6 gateway need (a NAT66 or a proxy that carries IPv6 destinations)?

**Owner.** podnet and podcrates (`pod-hold`, `pod-netstack`). **podrunner:** no record.

**Grade: hard.** Some items (ICMP errors under SOCKS5) are likely impossible with the proxy as the upstream.

---

# Area 2: Virtual network and the shim

## H-05. The shim cannot reach static binaries, `recvmmsg`, `io_uring`, `/proc/net`

**Ids.** `docs/virtual-network.md` section 3.3 (Limits), T-356 and T-317 (dual-mode IPv6, done).

**Problem.** The virtual network shim is an `LD_PRELOAD` library (`podnet-vnet`). It turns `socket(AF_INET)` into
`socket(AF_UNIX)` and keeps the virtual state. Limits: it cannot reach a static binary; `recvmmsg` and `io_uring` are
not interposed; `/proc/net/tcp`, `/sys/class/net` and `SIOCGIFCONF` do not show virtual addresses; an abstract unix
name is global to the network namespace of the host, so two programs that bind the same virtual address and port get
`EADDRINUSE`; an abstract name has no file permission (the shim takes the same user only, with `SO_PEERCRED`);
`recv` and `read` on a datagram socket do not drop datagrams of another user.

**Why hard.** `LD_PRELOAD` works only for dynamically linked programs that use libc symbols. Static binaries and
programs that make raw system calls (Go) bypass it. The remedy in the project is `podnet pod` or `podnet net proxy`.

**What was tried.** Pods (seccomp or netns) cover static binaries. The shim was fixed for .NET dual-mode IPv6 sockets
(the first consumer bug: `PodUtils/podrunner#69`, `#170`; T-317, T-356, T-357).

**Constraints.** C1 to C5. No `ptrace` of the program (podvm and podrunner run it in units).

**Research questions.**

1. Does a seccomp-based path exist for static binaries that gives them the virtual network with no memory write?
   (This is H-01 again.)
2. Can the abstract unix namespace collision be avoided by a per-run suffix? Programs that bind the same virtual
   address and port in different shims of one host must then be separated by a network name.

**Owner.** podnet. **podrunner:** no record.

**Grade: likely impossible (in the shim for static binaries).** The reduced goal is already served by `podnet pod`.

## H-06. Guest-wire router: silent UDP loss in the userspace stack

**Ids.** T-176 (research done), T-248 (router), `docs/research/podnet/guest-wire-router.md`, `PodUtils/podnet#464`
(asks 3 to 5, open), `PodUtils/podnet#457` (closed in `pod-netstack` v0.27.0).

**Problem.** The study measured the TCP and UDP behavior of `pod-netstack` (smoltcp 0.14) as a router for a guest on a
wire. Open asks to `pod-netstack`: (3) UDP counters in `NetStats` (loss at a full channel to the app, loss at a full
smoltcp buffer, a datagram that the send path cannot carry); (4) `UdpSocket::send_to` returns an error for a datagram
over `mtu - 28` when fragmentation is off; (5) `service_udp` must not drain the smoltcp socket when the app channel is
full. Measured: a burst of 500 datagrams with no reader reaches the app as 113, in order, and `NetStats::dropped` is 0.
`PacketQueue::is_full()` is a poll with no wait for room. A datagram over `mtu - 28` is lost silently when
fragmentation is off. A SYN past `max_connections` is dropped, not refused.

**Why hard.** The loss is inside a shared crate that is also used by other projects. The wire semantics (no
back-pressure signal for UDP) come from smoltcp.

**What was tried.** The router keeps its own bounded queue and counts losses. The features `proto-ipv4-fragmentation`
are on. The hold of a SYN on the wire (`HeldSyn`) works.

**Not proven.** A failed dial that answers a held SYN with a reset; a real Linux guest TCP; load numbers.

**Constraints.** C1 to C5; C3 (the fix goes to `pod-netstack`).

**Research questions.** Does smoltcp offer a way to peek at UDP with back-pressure (`peek_slice`)? Does a fork or
a newer smoltcp release already carry per-socket drop counters?

**Leads.** smoltcp repository and its UDP socket code (`src/socket/udp.rs`), https://github.com/smoltcp-rs/smoltcp.

**Owner.** podcrates (`pod-netstack`). podnet can write the evidence as a COORD issue. **podrunner:** no record.

**Grade: hard.**

## H-07. One flow over several uplinks; split of the directions of a TCP flow

**Ids.** T-179 (partial), `PodUtils/podnet#375`, `PodUtils/podnet#495` (deferred), `PodUtils/podcrates#125` (podswarm
ask N-4), T-096, T-302, `docs/research/podnet/mwan-field.md`, `docs/router.md`.

**Problem.** The router uses one uplink for each flow. Many flows use the sum of the uplinks. The owner of podswarm
wants one flow to use several uplinks (aggregation). The owner ruled on 2026-11-11 (direction split) that the split of
the directions of a flow is an option of a group, off by default. Built: a UDP split (`up` member sends, `down`
member is read first, then `up`). Not built: the TCP split. A byte stream needs two connections joined by a far end
(a flow id, a direction bit, an exit table, a cleanup). No far end exists. UDP split works only when the far host sits
behind both members and answers on the path of `down`; podnet cannot make a far host answer on `down`. A measure of
uplink and downlink of a member (to pick `up` and `down` with no operator) is not built.

**Why hard.** Bonding one TCP flow over two paths needs a cooperating far end (MPTCP-like). The research note says
"out of scope" for the no-privilege case. The only far end in reach is the link session (T-302 splits one session
across an up carrier and a down carrier), which both ends must run.

**What was tried.** The UDP split with a fallback; the link session split (T-302); the answer in `docs/router.md`:
per flow only.

**Constraints.** C1 to C5. No root on either end. The far end can be podnet.

**Research questions.**

1. Can the link session (a multiplexed, resumable session with an ARQ) serve as the far end of a TCP split with the
   router as its client? What does the flow id and exit table cost?
2. Is MPTCP (RFC 8684) usable without root on the host? (A kernel with `CONFIG_MPTCP` lets a user create MPTCP
   sockets; the far end must support MPTCP, and the proxy path must carry it, which an HTTP proxy does not.)
3. How do other bonding tools solve it (OpenMPTCProuter, `glorytun`, `mlvpn`, `engarde`)? All need a server end.

**Leads.** RFC 8684 https://www.rfc-editor.org/rfc/rfc8684 (from memory); `docs/research/podnet/mwan-field.md`;
Tailscale and WireGuard-based bonding projects (verify before use).

**Owner.** podnet and the owner (a far end is a product decision). **podrunner:** no record.

**Grade: very hard.**

## H-08. Tailscale inside the process

**Ids.** T-123, T-124 (open), T-114 (open), T-115 (partial), `docs/research/podnet/podssh.md`, `podssh-copy.md`,
`docs/decisions.md` (2026-11-05, Tailscale).

**Problem.** The tailnet works only through a `tailscaled` sidecar. A host where no `tailscaled` can run has no
tailnet. podssh carries a fork of `tailscale-rs` at commit `f4781c4` with 29 patches (`vendor/tailscale-rs`).
The owner ruled: in-process Tailscale later, behind a feature that is off by default. The work goes to the crate
`pod-ts` of podcrates (wave 4). The record check on 2026-11-20 found no `pod-ts` directory in the local podcrates
checkout (commit `edbc86b`). The fork links `aws-lc` (C, cmake on some targets), needs Rust 1.92, adds about 420
packages, and has `RUSTSEC-2024-0436` (`paste`) plus an MPL-2.0 licence exception. No build for `i686`, `arm`,
`loongarch64` or `powerpc64le` is proven. `cargo deny` must stay green with no ignore. Also the headscale harness
(T-114) for the sidecar paths does not exist: CI has no Tailscale account. T-115 step 3 waits for it.

**Why hard.** A static musl build with a C crypto library for many targets; an upstream that declines outside patches;
a licence and an advisory policy.

**What was tried.** The sidecar paths were built and tested against a fake `tailscaled` (LocalAPI on a unix socket).
podssh has a harness (`scripts/ts-headscale.sh`, `.github/workflows/ts-headscale.yml`, images pinned by digest).

**Constraints.** C1 to C3, C10. Feature off by default. A patch that names `ring` in place of `aws-lc` is the stated
idea (T-124 step 4).

**Research questions.**

1. Does `tailscale-rs` build with a `ring` provider (a patch)? How large is it?
2. Can headscale run in a GitHub Actions job as the stand-in of the control plane for podnet's tests (podssh does it)?
   Where should the harness live: `pod-testkit`? (Ask podcrates first, per T-114.)
3. Does the `paste` advisory have a replacement crate in the dependency tree?

**Leads.** https://github.com/tailscale/tailscale-rs (verify the repository); headscale: https://github.com/juanfont/headscale
(from memory); podssh's scripts named above.

**Owner.** podcrates (`pod-ts`), then podnet (the feature and the network spec). **podrunner:** no record.

**Grade: very hard.**

## H-09. NetBird netstack mode on a proxy-only host: not measured

**Ids.** T-247 (blocked), T-246 (done on documents only), `docs/research/podnet/overlays.md`, `docs/decisions.md`
(2026-11-11, deferred and blocked work).

**Problem.** No run shows that the NetBird client joins through `HTTPS_PROXY`, runs with no root (`netbird up -F`),
and carries a TCP flow through its SOCKS5 listener on a host of `docs/target-environment.md`. podnet has a `netbird:`
network (a SOCKS5 hop on `127.0.0.1:PORT`), tested only with a fake listener that follows RFC 1928, not the NetBird code.

**Why hard.** The measurement needs a NetBird account or a setup key and a peer. A mock cannot show the real client's
behavior with a proxy. The owner's ruling keeps this blocked until an account exists.

**Constraints.** C1 to C3. The setup key must be a secret of the `live` job and never appear in logs.

**Research questions.** Does the NetBird client in netstack mode honor `HTTPS_PROXY` for its management and signal
connections (gRPC over HTTP/2)? Does it need UDP (WireGuard, STUN)? If UDP is refused, does NetBird fall back to a
relay over TCP 443?

**Leads.** NetBird docs for userspace and netstack mode (search "netbird netstack mode `NB_USE_NETSTACK_MODE`";
from memory, verify).

**Owner.** the owner (an account), then podnet. **Grade: hard** (the work is small; the missing input is outside).

---

# Area 3: Tunnels and the Cloudflare edge

## H-10. A TLS-terminating proxy cannot carry the Cloudflare edge or MASQUE

**Ids.** `docs/target-environment.md` (the first section and the row "The proxy re-terminates TLS"),
`docs/protocol.md`, `docs/warp.md`, `docs/proxies.md`, T-421 (done).

**Problem.** The Cloudflare edge connection (QUIC on UDP 7844, HTTP/2 on TCP 7844) needs the Cloudflare certificate
chain. The WARP MASQUE session needs a client certificate and a pinned server key. A proxy that re-terminates TLS
with its own CA breaks both: the edge and MASQUE cannot pass through it. Registration and API calls work, since they
use ordinary server certificates and `--ca-file` / `SSL_CERT_FILE` can name the proxy root. Related measured facts:

* `podnet doctor` on a measured sandbox: CONNECT to the edge on 7844 (name and address) and to MASQUE
  `162.159.198.2:443` gives HTTP 200 and then no TLS byte ("the proxy answers and carries nothing"). 0 of 3 legs can
  work. A quick tunnel cannot reach the edge from that host.
* That proxy resets CONNECT to an IP address and allows names (DoH works by name only).
* UDP 7844 is silent.

**Why hard.** This is how TLS works. The client verifies the server identity itself. A proxy that terminates TLS
presents its own certificate. There is no protocol path through such a proxy for a pinned or mutual-TLS connection.

**What was tried.** The relay road (`--transport relay`): the edge connection runs as an HTTP/2 stream over a
WebSocket to the relay host (`tcp.ssh.relay.ajam.dev`, the relay presents a certificate that the proxy CA can
re-sign and that podnet accepts through `--ca-file`). `http2+relay` registered through the relay in 0.74 s on a
runner. A TLS-through-WebSocket carrier avoids the proxy's TLS interception for the inner TLS.

**Constraints.** C1 to C6. No hardcoded relay host in code (it is a fact of `pod-services`).

**Research questions.**

1. Can the inner TLS (to the edge) run inside the WebSocket whose outer TLS the proxy terminates? That is the relay
   road. Is any proxy class that terminates TLS and also inspects WebSocket content? Does the relay road survive it?
2. Is there another public service on TCP 443 that carries an opaque stream through a name-based proxy rule (for
   example the services listed in `docs/research/podnet/hosted-relays.md`)? Which of them the proxies allow?
3. Does Cloudflare publish an alternative edge name over standard HTTPS (WebSocket) for tunnel registration?
   (`cloudflared` supports `http2` and `quic` only.)

**Leads.** `docs/research/podnet/hosted-relays.md` (a table of 15 services with ports, terms and limits);
`docs/research/podnet/iroh-relay.md` (iroh relay over HTTPS 443 with WebSocket upgrade; `pod-iroh` exists in podcrates).

**Owner.** none for the limit. podnet and podcrates for roads that avoid it. **Grade: likely impossible (direct
carry); a relay workaround exists.**

## H-11. A quick tunnel with two edge connections gets HTTP 530 (T-488)

**Ids.** T-488 (open, research), `PodUtils/podcrates@T-931`, `PodUtils/podnet#654`, `live` runs 44444118882
(`e9bf11c`, 2026-11-19) and 44286677011 attempt 2 (`60db06b`, 2026-11-18).

**Problem.** A quick tunnel through WARP registers its first connection (`msp01`) and, about 0.25 s later, connection 1
(`ord11`). Some requests through the public URL get HTTP 530 from the edge. On 2026-11-19 the first POST of 1 MiB
(1.4 s after the first registration) and 2 of 20 requests of the latency check got 530. On 2026-11-18 the first POST of
1 MiB did the same. Each other request passed. `pod_cloudflare::Config` sets `ha_connections: 2` for each tunnel,
quick ones too. `cloudflared` opens one connection for a quick tunnel.

**Why hard.** The edge is not under our control. It is not known whether the edge sends a request of a quick tunnel to
a connection that is not ready, or to one whose registration it does not keep for a quick tunnel.

**What was tried.** Nothing yet except the comparison with `cloudflared`. The proof command is
`sh scripts/live-throughput.sh target/release/podnet quic "1 4 16" warp`.

**Constraints.** C3, C4: the setting is in `pod-cloudflare`, so a change is a COORD issue `(to podcrates)`.

**Research questions.** Run 20 rounds with `--ha-connections 1` and 2, through WARP and direct. Count the 530s. If
one connection gives none, ask podcrates for a default of one connection for a quick tunnel and two for a named one.
Does the edge document that a quick tunnel (`trycloudflare.com`) allows more than one connection?

**Leads.** The `cloudflared` source for quick tunnels (`cloudflared tunnel --url`), https://github.com/cloudflare/cloudflared
(`connection`, `supervisor`); Cloudflare Tunnel documentation for `--ha-connections`.

**Owner.** podnet measures; podcrates changes `pod-cloudflare`. **Grade: hard** (a measure, then a small fix).

## H-12. No self-hosted Cloudflare edge: a stand-in without QUIC, and service rate limits

**Ids.** T-316 (partial), T-315 (done), T-411 (done), `docs/research/podnet/cf-edge-standins.md`, T-221 (step 4).

**Problem.** `live.yml` runs about 25 steps of the quick tunnel against `api.trycloudflare.com` and the Cloudflare
edge. The service answered HTTP 429 (error 1015, `Retry-After` 3600 s) after many runs. `speed.cloudflare.com` answered
429 to the shared address of WARP (T-411). The steps run only in `live`, not on each push. A stand-in exists
(`tests/standin/`: a fake provision API with a 429 mode, an HTTP/2 edge over TLS on TCP, a front that maps `Host` to
tunnel, a CONNECT proxy, a test CA); it covers 15 tests in `ci`. Not built: the QUIC leg. Not covered: the verify of
the public URL, the WARP and relay legs, and a change of the real edge.

**Why hard.** No open-source, self-hostable Cloudflare Tunnel edge exists. `cloudflare/cloudflared` has Go test fakes
only. `flolep2607/cctop` (MIT or Apache-2.0) has a fake edge with QUIC and HTTP/2 listeners in its tests. A stand-in
tests podnet against its own reading of the protocol, not against a change of Cloudflare. The QUIC leg needs a change
in frozen code (`pod-cloudflare`), though the roots can come through `SSL_CERT_FILE`.

**What was tried.** The stand-in in `tests/standin/`; the move of steps from `live` to `ci`. The QUIC leg is open.

**Constraints.** C3, C4. No secret of Cloudflare in the repository. Keep one real quick tunnel for each leg in `live`.

**Research questions.** Can `cctop`'s fake edge (QUIC) be taken as a test crate of podcrates (licence permits)? What
does the QUIC leg of `pod-cloudflare` need to accept a test root? Is a named-tunnel test account possible (T-221)?

**Leads.** `docs/research/podnet/cf-edge-standins.md`; https://github.com/flolep2607/cctop (from the record).

**Owner.** podnet (tests), podcrates (a hook in `pod-cloudflare` if needed), outside (Cloudflare). **Grade: very hard**
for a faithful edge; the QUIC stand-in alone is hard.

---

# Area 4: WARP and MASQUE

## H-13. `warp proxy --l4`: the L4 door answers 403 to each CONNECT

**Ids.** T-002 (blocked), `PodUtils/podnet#140`, `src/warp/l4.rs`, `scripts/live-batch2.sh`, `docs/decisions.md`
(2026-11-11, deferred and blocked work).

**Problem.** Today each TCP client of `podnet warp proxy` becomes IP packets in a userspace stack over one CONNECT-IP
stream (TCP buffers of 512 KiB each way, one driver task, one HTTP/2 stream window, two DNS queries per name). The
issue asks for one HTTP/3 `CONNECT` stream for each TCP client to the "L4 door" of WARP (UDP 443, TLS name
`consumer-masque-proxy.cloudflareclient.com`), with no IP stack, the name in `:authority`. The client code exists in
`src/warp/l4.rs`. QUIC, TLS and the pinned device key work with the names `consumer-masque-proxy.cloudflareclient.com`
and `consumer-masque.cloudflareclient.com`. The door answers 403 to each CONNECT in three forms
(`PODNET_WARP_L4_FORM`):

* `plain`: `:method CONNECT`, `:authority host:port` (RFC 9114, section 4.4);
* `extended`: `:protocol cf-connect-tcp` with `:scheme` and `:path` (RFC 9220);
* `header`: a plain CONNECT with `cf-connect-proto: cf-connect-tcp`.

**Why hard.** The door knows the device and refuses. Possible causes: an anonymous consumer device has no right to the
L4 door, or the door wants a form that is none of the three. The request form of the official client in proxy mode is
not documented. The sandbox cannot reach WARP.

**What was tried.** The three forms, in live runs.

**Constraints.** C1 to C4 (the code is in the frozen `src/warp/`, so a change goes through podcrates). No guessing
at scale against the service: a wrong request can burn devices (see H-16).

**Research questions.** What does the official Cloudflare WARP client (proxy mode) send on UDP 443 to
`consumer-masque-proxy.cloudflareclient.com`? Can a packet capture of the official client with TLS keys (a key log)
show the headers? Are there open-source clients that speak the L4 door (for example in Go or Rust MASQUE projects)?

**Leads.** RFC 9114 section 4.4 and RFC 9220 (extended CONNECT); RFC 9298 (CONNECT-UDP) and RFC 9484 (CONNECT-IP)
https://www.rfc-editor.org/rfc/rfc9484.html ; Cloudflare WARP docs on proxy mode, https://developers.cloudflare.com/cloudflare-one/
(search result of 2026-11-20); `masque-go`/`usque` (open-source WARP MASQUE clients; verify).

**Owner.** an outside capture (the owner or a tester), then podnet and podcrates (`pod-warp`). **Grade: very hard.**

## H-14. Encrypted ClientHello on the WARP, edge and API hops

**Ids.** T-004 (blocked), `PodUtils/podnet#151`, T-367 (deferred, `#486`).

**Problem.** Each TLS connection of podnet sends the server name in the clear. The client builders are in `pod-tls`
(`client_config`), `pod-warp` (MASQUE, with the pinned key) and `pod-cloudflare` (QUIC to the edge). The issue asks for
`--ech auto|off|CONFIG`, a config from the HTTPS record (type 65) through the DoH client, a retry with the retry
configs of a rejection, and a `doctor` row per hop. Facts: the crypto provider is `ring`, which has no HPKE. The HPKE
of rustls is in the `aws-lc-rs` provider (a C build on each of 8 targets) or in another crate. The MASQUE front door
and the WARP API publish no ECH config. The public config of `cloudflare-ech.com` has `max_name_length` 18, and the
MASQUE name `consumer-masque.cloudflareclient.com` has 36 characters. The ClientHello can be split into records
(issue 135, commit `b811b8e`), and the session can go over HTTP/3 or WireGuard (#157, #152), in place of ECH.

**Why hard.** The hops publish no usable ECH config. ECH needs HPKE (RFC 9180) with a provider that the policy
excludes. The name-length limit of the public config means that a long name cannot be padded in.

**Constraints.** C3 (the change goes to `pod-tls`), C10 (`ring` stays).

**Research questions.** Does any hop of podnet publish an `ech=` parameter in its HTTPS record now? Is a pure Rust
HPKE crate acceptable (X25519, HKDF-SHA256, AES-128-GCM; RFC 9180 test vectors)? Does rustls support ECH with a
`ring`-compatible `CryptoProvider` that has an `hpke` field (rustls has an `Hpke` trait since 0.23.x; verify)?

**Leads.** RFC 9180 (HPKE) https://www.rfc-editor.org/rfc/rfc9180 ; the ECH draft `draft-ietf-tls-esni` (check if it
is an RFC now); rustls `client::EchConfig` (from memory; verify).

**Owner.** podcrates (`pod-tls`), the owner (provider choice). **Grade: likely impossible now** (no hop publishes a
config); later hard.

## H-15. WARP proxy recovery and first request (T-414, T-415)

**Ids.** T-414 (partial), T-415 (partial), T-068, T-069 (done), `live` runs 44007046520 (`2bd37f6`), 44005215136
(`73b3e28`), 44105534873 (`2d66915`), 44286677011 (`60db06b`, attempt 1), 44019959524 (`ba6cce4`).

**Problem.**

* T-414. The `live` step "WARP proxy" kills the TCP connection of the MASQUE session (`ss -K dst 162.159.198.0/24
  dport = 443`) and waits 60 s for the log line "a new WARP session is open". In run 44007046520 the line did not
  come, and the log had no line after its start. Before the kill the session had no traffic for about 4 minutes
  (`speed.cloudflare.com` answered 429 to each download). In run 44005215136 the new session opened 0.4 s after the
  kill. A second form: the proxy had ended at its `--run-for 150`. A third form: run 44286677011 attempt 1, the upload
  of 16 MiB with `--cc cubic`: the public endpoint reset one upload (`curl: (97) Recv failure`), then the `warp proxy`
  ended with `the MASQUE session is open, and a DNS query to 1.1.1.1:53 through it got no answer`, and tries 2 and 3 got
  `Couldn't connect`. The proxy ends on a failed health check of its session, where it could open a new one.
* T-415. The step starts `podnet warp proxy --listen tcp://127.0.0.1:40000 --dns 127.0.0.1:15353`, and curl through
  `socks5h://127.0.0.1:40000` to `https://www.cloudflare.com/cdn-cgi/trace` got `(97) Recv failure: Connection reset by
  peer` 5 s after the start. The step printed no proxy log then. The same step passed on `404440d`.

**Why hard.** One run is not enough to find a cause: an idle session that the service closed before the kill, a
session on another transport, a reader that does not see the abort, or a race at start. The code is in the frozen
`pod-warp`.

**What was tried.** The step now prints the TCP sockets before and after the kill, retries once after a warning, and
prints the last 30 lines of the proxy log when the first request fails. `--run-for` now logs its end. Step 3 of both
entries (the evidence to podcrates) waits for the next warning.

**Constraints.** C3, C4, C5.

**Research questions.** What does the proxy do on a failed health check (end versus renew)? Is a session that carried
no TCP for minutes closed by the edge? Does a reset right at the start come from the listener (ready before the
session is up)?

**Owner.** podcrates (`pod-warp`); podnet gathers evidence. **Blocks podrunner:** yes, by inference of this report: podrunner's
user paths can use the WARP proxy. The record ties only the live gate flakes to it. **Grade: hard.**

## H-16. Limits of the WARP service

**Ids.** `docs/warp.md`, `docs/protocol.md`, T-068, T-069, T-411, T-210.

**Problem.** Facts that no podnet change removes:

* Registration is rate limited per address: HTTP 429 (error 1015) after about 10 new devices in 90 minutes, cleared
  after about 30 minutes. `--ephemeral`, a new `--listen-identity` file and `warp export` each register one device.
* One MASQUE session keeps one egress address. A new session gets a new one, so a destination sees changing addresses
  (issue 183).
* A session can carry ICMP and UDP but no TCP. podnet probes the TCP data path each 30 s (`--probe`, default
  `1.1.1.1:443`) and renews a session after 3 failures.
* `speed.cloudflare.com` answers 429 to the shared address of WARP (the live rate check failed, T-411).
* The L4 door (H-13), Zero Trust enrolment (H-31) and the WARP+ account types (H-31) are unknown.

**Why hard.** An outside service with no published contract for consumer devices.

**Constraints.** `docs/decisions.md` (WARP devices): a registration spends a device. No test may register devices in a
loop.

**Research questions.** Does the service document its rate limits or its session lifetime? Is a device reusable across
hosts (a `warp export` identity used on two hosts) without penalty? Is there a documented way to detect a TCP-less
session earlier than a SYN probe?

**Owner.** the outside service. podnet can alone keep the probes and the messages. **Grade: very hard** (a service
behavior, not a code fault).

---

# Area 5: Relay, resume and transfer

## H-17. The relay keeps a lost node socket; SSH through it hangs

**Ids.** T-406 (blocked), T-478 (open), T-410 (done), `PodUtils/podnet#604` (COORD-098), `PodUtils/podcrates#214`,
`PodUtils/podssh` (the relay role), live runs 43912914825 (`ce9456d`), 43918102449 (`6369056`), 44125425938
(`3654383`), 44172865442 (`27e3b7b`), 44147054834 (`76f7031`).

**Problem.** A node connects to the public relay (`tcp.ssh.relay.ajam.dev`). Two seconds later the link ends with
`Connection reset by peer (os error 104)` and no Close. From then on each new connect of the node gets HTTP 409
("the relay still holds the node's lost socket"), for the 150 s of the live step. The SSH client of the operator
reaches the relay and stops in the key exchange (`expecting SSH2_MSG_KEX_ECDH_REPLY`): the relay sends it to the lost
socket. In the resume step ("200 MiB each way through the reverse relay with --resume"), each new leg gets
`NeverReady { code: Some(1013), reason: "node open timeout" }` for 600 s. About one `live` run of six fails in this
class. T-478: two SSH sessions at the same time through one node; one hung with no line and the step waited its 5
minutes. The same step passed 20 minutes before. A test with two sessions at the same time (a fake relay) passed 20
rounds of 20 and does not reproduce the hang. The fake relay sends no Close when the node ends a session, so the
operator ends after `EOF_IDLE` (15 s of no byte), by design. The real relay sends the Close.

**Why hard.** The fault is in the relay server, which is not in this repository. The node (`pod_relay::reverse::node`
of podcrates) retries a 409 for `rejoin` (600 s) as designed. The server keeps a socket until it notices that it is dead.

**What was tried.** Ask 2 (the node takes the next pair after a shorter 409 time) is done in `pod-relay` v0.65.0
(`lost_hold`, 60 s; T-410). Ask 1 (the relay drops a dead socket at once after a reset or a failed write) is for the
relay role of podssh. The live step now gives each parallel session a 60 s limit and a debug log, and prints the logs
on a failure. The cause of T-478 is most likely T-406, not proven.

**Constraints.** C3, C5. A relay of the pool keeps the fault until its operator deploys the fix. The release notes of
beta.17 list it under "Known problems".

**Research questions.** How does a WebSocket relay detect a dead peer (TCP keepalive, a ping with a short timeout,
a write error)? Does the relay see the RST? Why does a reset 2 s after connect not release the socket (a half-open
state)? Which side holds the 409 rule, and what is the minimum safe hold time?

**Leads.** RFC 6455 (WebSocket ping and pong, close handshake); the relay contract at
`https://tcp.ssh.relay.ajam.dev/llms-full.txt` (read it first).

**Owner.** podssh (relay role) and the operator of the public relay. podnet can alone: improve logs, keep the live
step strict, add a test with a relay that does not release a socket. **Grade: hard** (the code is outside).

## H-18. Pair renewal, alternate hosts on the reverse road, unattended service

**Ids.** T-161 (blocked), T-200 (blocked), T-163 (partial), T-219 (blocked), T-220 (blocked), `PodUtils/podnet#358`,
`#397`, `#360`, `PodUtils/podnet#482` (COORD-051), `PodUtils/podcrates#90`, `PodUtils/podcrates#35` (asks A-N-11,
A-N-12 of cfb).

**Problem.**

* A pair lasts at most 72 h. A node stops with "the pair NAME expired; make a new pair". A node with an explicit name
  cannot follow to a next pair. Unattended hosts stop after 72 h. The relay contract publishes no renew call.
  `pod_relay::pair::renew::renew` uses `POST /v1/renew/NAME` with the stop token, and a relay of the contract answers
  404 or 405. The call proposed by the record is `POST /v1/pair/NAME/renew` with the node token. The node of the crate
  renews nothing (`NodeConfig::repair` runs only after expiry).
* The reverse road uses the first host only (`Relay::primary()`). The relay makes a pair on the control host only and
  moves none (podcrates T-124). Not measured: a node socket on host B for a pair of host A.
* The session budget of the relay leg (T-163, partial): the limits come from `/relays.json`; the WARP leg is under the
  relay's limits. Open: the edge side (`edge_h2`, frozen in `pod-core`) and third-party relays.
* T-219: `podnet service install` (a hardened system unit: own user, `EnvironmentFile`, `LimitNOFILE` 70000,
  `NoNewPrivileges`, `ProtectSystem=strict`, a fixed state directory so a restart makes no new WARP device) waits for
  `pod-unit` of podcrates. T-220: `relay node` as a system service with no operator act needs T-219, T-161 and T-087.

**Why hard.** The server contract is not under podnet's control. The record says the owner of the relay must add the
endpoint. Three parts (unit, renewal, resume) must work together.

**What was tried.** A decision: the renew call goes into the relay server through podcrates, not into podnet.
podcrates answered: the contract has no renew path; its T-121 plans a setting of `reverse::run` that renews with the
stop-token call where the relay serves it.

**Constraints.** C3, C6. The renewal must not let a connect token extend its own life (node token only).

**Research questions.** Who runs the public relay (the owner? podssh?) and can the endpoint be added? Which relays of
the pool exist, and do alternates serve a node socket for a pair of the control host? What does a hardened unit
need on hosts with no systemd (cron? `podnet autostart` exists)?

**Owner.** the relay operator and podcrates (`pod-relay`, `pod-unit`); podssh. **Grade: hard.**

## H-19. The cap race of a resumed transfer, and the limits of resume

**Ids.** T-476 (open), T-087 (done), T-375 (deferred, `#498`), `docs/relay.md`.

**Problem.** `tests/transfer_relay.rs::transfer_over_the_relay_crosses_the_cap` failed once in a full run (17:42 UTC,
2026-11-17): a relay session carried 1049609 bytes, 1033 over the cap of 1 MiB of the fake relay. The file landed with
the right SHA-256 (the resume after the close of 1009 carried it). Alone, the test passed 3 of 3. The fake relay
counts the frame that crosses the cap and then closes the session with 1009, so the move to a new leg came after the cap.
The receiver moves at 256 KiB in the test (a margin of 768 KiB). The real relay: a cap of 64 MiB and `MOVE_BYTES` 48
MiB (a margin of 16 MiB). The old leg carries bytes while the new leg opens, so a slow open on a loaded host or a fast
flow can meet the cap. The real limits: a session at most 64 MiB and 12 h (11.5 h used); the replay memory of a node is
at most 64 MiB for all its sessions; a session with no leg lives 10 minutes at the node.

**Why hard.** It is a make-before-break race between a byte budget and a time to open a new leg, and the margin is a
guess. The fix must give a guarantee, not a bigger margin.

**What was tried.** Nothing yet. The approach: measure the bytes of the old leg between the decision and the switch
under load; stop writes on the old leg at the margin, or start the open earlier (a margin in time and bytes);
or make the test state the guarantee that the code gives.

**Constraints.** C5. The fix is in podnet glue and in `pod-relay` (resume layer: `pod-session`).

**Research questions.** What is a proven design for a make-before-break handover of a byte stream over legs with a hard
byte cap (for example the credit-based handover in QUIC connection migration, or MPTCP subflow handover)?

**Owner.** podnet and podcrates. **Grade: hard.**

## H-20. File transfer through public mailboxes and transit relays

**Ids.** `docs/transfer.md`, `docs/research/podnet/hosted-relays.md`, T-103, T-106, T-253, T-253 (live), magic-wormhole
issue #754.

**Problem.** `podnet send` and `receive` use the magic-wormhole protocol (the mailbox `wss://relay.magic-wormhole.io/v1`
and a second mailbox). Facts: the mailbox is "a personal server" that its owner may shut down; magic-wormhole issue
#754 (open on 2026-11-04) says two clients of the public mailbox can reach two instances that do not share their
mailboxes, so the receiver finds nobody (exit 69); `transit.magic-wormhole.io` listens on TCP 4001, and 443 resets
(probed), so a transit relay through a proxy-only host does not work; the Winden mailbox has no terms and is a beta. The
podnet relay is the last way of the transit between podnet and podnet; a piping server is the last way of all.
Not proven live: a transfer through the public relay and a real proxy; a live meeting at the public mailboxes; the end
of a pair during a transfer; a piping-server of version 1.12.9 with a buffering proxy; more than 1 MiB through it.

**Why hard.** Free public services with no contract. A proxy-only host reaches only TCP 443 by name.

**Constraints.** C6 (hosts are data of `pod-services`).

**Research questions.** Which transit relays accept TCP 443 or WebSocket? Can the mailbox run inside the podnet relay
(the owner's ruling mentions a mailbox server of podnet's own as a fallback)? How do `croc`, `Winden` and
`magic-wormhole-rs` handle the transit through HTTPS-only networks?

**Owner.** outside services; podnet (a fallback mailbox server). **Grade: hard.**

---

# Area 6: Link carriers

## H-21. NAT traversal with a relay fallback (T-284)

**Ids.** T-284 (blocked), `PodUtils/podnet#469` (row E3-4), T-261, T-262, T-049.

**Problem.** The link workstream asks for peer-to-peer NAT traversal: exchange candidate addresses through the
rendezvous; send UDP at the same time from both sides; verify the path with a nonce; if traversal fails within 5 s,
fall back to the configured relay; `status` says `path: relay` and `reason: symmetric-nat`; the fingerprint is
checked the same on both paths. The done test: `tests/link_p2p.rs` in two network namespaces (a direct path for a cone
NAT; the relay with `symmetric-nat`). Facts from podcrates (answer to ask A5, `PodUtils/podcrates#88`): the relay
contract carries the candidates as an opaque binary message between the two ends before the stream; the relay gives
no reflexive address (peers learn their public addresses from STUN, `pod-services` facts of kind `stun`); the relay
carries TCP only and takes no inbound TCP, so the direct path (a UDP hole punch) belongs to the peers; a session
needs the node online and `ready` within 15 s and lasts at most 720 min and 64 MiB.

**Why hard.** Hole punching without root or a UDP socket: the target hosts often refuse UDP altogether (proxy-only
egress). Symmetric NATs, CGNAT, and firewalls with a port-restricted policy defeat simple punching. A test needs two
network namespaces with NAT rules (root or a user namespace with nftables).

**What was tried.** Nothing built. The overlay and the relay exist.

**Constraints.** C1 to C3, C6 (no hardcoded STUN host).

**Research questions.** Which hole punching algorithms are practical (ICE, RFC 8445; Tailscale's disco
protocol with birthday-paradox port probing)? Can a test lab reproduce cone and symmetric NATs in a user namespace
with nftables? Is a direct path ever possible on the target hosts, given that UDP is refused (then the relay fallback
is the normal case)?

**Leads.** Tailscale, "How NAT traversal works": https://tailscale.com/blog/how-nat-traversal-works (from memory);
RFC 8445 (ICE), RFC 4787 (NAT behavior), RFC 8489 (STUN).

**Owner.** podnet and podcrates (`pod-link`, `pod-relay`). **Grade: very hard.**

## H-22. DNS stream carrier with a 32-query window (T-281)

**Ids.** T-281 (blocked), T-278, T-277, `PodUtils/podnet#469` (row E3-1), `PodUtils/podnet#521`.

**Problem.** Row E3-1 asks for a DNS stream carrier with 32 queries in flight, answers reordered by sequence
(`ordered=false`, the ARQ restores the order), TXT first and CNAME chains where TXT is filtered, 1232-byte answers,
a ceiling of 200 queries a second, a re-resolve each 5 minutes, and backoff on SERVFAIL. The test: 256 KiB, reorder,
SERVFAIL backoff, rate ceiling. The record shows the simplex DNS carrier of `pod-link` (feature `dns`): defaults
20 queries a second and 8 in flight, a 2 s query timeout. The pipelined window is not built.

**Why hard.** A carrier over recursive resolvers: rate limits, caching, TTLs, the 512-byte limit without EDNS0, and
resolvers that rewrite or drop TXT.

**Constraints.** C3 (a take of `pod-link`).

**Research questions.** What query rates do public resolvers tolerate? How do `dnscat2`, `iodine` and `dns2tcp` size
their windows?

**Owner.** podcrates (`pod-link`). **Grade: hard.**

## H-23. WireGuard peer as a link carrier (T-283)

**Ids.** T-283 (blocked), T-117 (partial), T-125 (`pod-wg` responder), `PodUtils/podnet#469` (row E3-3).

**Problem.** Dial the tunnel address of a peer through the userspace WireGuard network, then run the stream handshake;
MTU from the interface (default 1280); UDP only; one session for each peer address; the key stays in the `wg-quick` file
at mode 0600. The `wg:FILE` network exists. No carrier runs on it. The listen half needs the responder of `pod-wg`.
Not proven: a WireGuard peer of another code base (the responder shares the crypto of `pod-wg`).

**Constraints.** C3. The key is never read into `--json`.

**Owner.** podcrates (`pod-link`, `pod-wg`); podnet glue. **Grade: hard.**

## H-24. TURN: TCP allocations (RFC 6062) and missing TURN facts

**Ids.** T-321 (partial), T-366 (deferred, `#485`), `PodUtils/podnet#507`, T-320.

**Problem.** `turn://HOST[:PORT]` (UDP) and `turns://` (TLS on TCP) work for UDP flows with `pod-turn` (allocate,
refresh, permission, channel bind, named stage errors), tested against a stand-in only. Left: the TCP relay of RFC
6062 (a later entry of podcrates); one account for each process (a TURN network of the router with its own account);
the host and port of a named TURN service from `pod-services` (no such fact). Not proven: a live TURN server. The issue
`#485` also asks for traversal with a relay fallback.

**Constraints.** C3, C6.

**Leads.** RFC 8656 (TURN), RFC 6062 (TURN TCP allocations), RFC 8489.

**Owner.** podcrates (`pod-turn`, `pod-services`). **Grade: hard.**

## H-25. Shadowsocks and Trojan services at a peer (T-294, T-295)

**Ids.** T-294, T-295 (blocked), T-127 (open), `PodUtils/podnet#471`, T-266, T-311, T-291.

**Problem.** A link peer should serve Shadowsocks (SIP004 AEAD: `chacha20-poly1305`, `aes-256-gcm`, HKDF-SHA1, a
replay window; not the 2022 protocol) and Trojan (TLS 1.3, hashed password, a static response on failure). Done when
tests use the published SIP004 vectors. podnet has no codec on the accept side. The record says `pod-egress` has the
client roads. A local podcrates checkout (commit `edbc86b`) has `pod-egress` with features `vless`, `vmess`, `trojan`,
`shadowsocks` (client `Road`); the accept side was asked as ask A4. The record marks it blocked until a release
exists.

**Constraints.** C3, C8, C10 (AEAD with `ring`).

**Leads.** https://shadowsocks.org/doc/aead.html (SIP004; from memory).

**Owner.** podcrates (`pod-egress`). **Grade: hard.**

## H-26. Release binaries carry no optional link carrier (T-428)

**Ids.** T-428 (partial), `PodUtils/podnet#620`, `docs/build.md`, `docs/link.md`.

**Problem.** `release.yml` builds default features only, so a release binary has the carriers `unix` and `memory`.
The carriers `tcp`, `ws`, `h2`, `quic`, `udp`, `dns`, `icmp`, `poll`, `s3` are behind Cargo features. The docs now say
so (step 1 done). Open: a feature joins the release set only when it builds and passes its tests on each target.
The alternative: a second `full` binary for each target. The `icmp` rows of the lane check nothing, since the
release binary says "not built" (exit 69).

**Why hard.** A feature must build as a static musl binary on every target of the release set (C dependencies, size,
risk). The record leaves the call on size and risk to the owner.

**Owner.** the owner (the call), podnet. **Grade: hard.**

## H-27. The deferred issues #484 to #501

**Ids.** T-365 to T-378 (all blocked by the owner's deferral of 2026-11-12). Sources: `TODO/deferred.md`,
`docs/reports/deferred-issues-2026-11-12.md`, `PodUtils/podnet#484` to `#501`.

**Problem.** The owner deferred 14 issues from an outside author (`author_association` NONE, no label `confirmed`).
Each holds a table of capabilities. None has been verified, built or measured. The record asks: when the owner lifts
the deferral, verify the issue (the rule of strangers), make each table row an entry, and close the entry with those
ids. The set, with a grade for each:

| Entry | Issue | Subject (the rows) | Grade and reason |
| --- | --- | --- | --- |
| T-365 | #484 | A session in NTPv4 extension fields, SMTP submission (587, 465) and IMAP retrieval (143, 993) | very hard: a carrier over mail needs an account and a server |
| T-366 | #485 | A session through a TURN allocation: UDP (RFC 8656), TCP (RFC 6062), traversal with relay fallback | hard (see H-24) |
| T-367 | #486 | MASQUE `--masque-endpoint`, an HTTP/3 server shape, Juicity, connection-ID rotation and migration | very hard |
| T-368 | #489 | Third-party services as roads with a watch, a date, a count; IRC, MQTT, Syncthing relays | hard |
| T-369 | #491 | A real handshake then a switch on an authenticated byte; a cover script; multiplexed cover with a volume budget | very hard (needs a cover site) |
| T-370 | #493 | Record layer shaping: padding profiles per protocol, entropy control, first-flight sizing, pacing, a `doctor` row for declared versus observed shape | very hard (an evasion arms race; needs measurement per censor) |
| T-371 | #494 | Application shaping: deferred response replacement, a traffic generator with an application profile, carriers inside an application's own bytes | very hard |
| T-372 | #495 | A packet sidecar: a desync search with a genetic algorithm, a disagreement probe, fake TCP, bonding | likely impossible under the constraints (raw sockets and packet injection need `CAP_NET_RAW`) |
| T-373 | #496 | End-to-end session authentication under the relay, an opaque destination, record keys from the ends, a table of what each hop learns | hard (`pod-e2e` exists) |
| T-374 | #497 | A live pool with a health score, a per-deployment budget, tokens with a lifetime, a deployment that moves hosts | hard (needs relay server changes; see H-18) |
| T-375 | #498 | Resume across hours; carrier handover; store-and-forward shell; full SOCKS5 surface at a peer (`BIND`, `UDP ASSOCIATE`) | hard |
| T-376 | #499 | A session with no network: serial line, AX.25/KISS, HF radio, a spool, discovery, offline bundle, replication across a partition | very hard (hardware) |
| T-377 | #500 | Host runtime statements: no open port by default, classified egress, crash logs safe to send, a mirror of a forge, CI under an egress policy | hard |
| T-378 | #501 | A verification harness: classifiers, an exit-code contract, a conformance matrix, a relocatable archive | hard |

**Constraints.** C1 to C9. Many rows contradict "Linux only, no root" (raw sockets, radio, serial).

**Owner.** the owner (lift the deferral, verify the author's claims). **Grade: very hard as a set.** Research use:
most items need no research until the owner lifts the deferral. T-372, T-376 and T-369 to T-371 need the most.

---

# Area 7: Release, CI and the lab

## H-28. The release gate does not cover the testers' host class, error paths, IPv6 hosts and ICMP rows

**Ids.** T-358 (partial), T-398 (partial), T-450, T-452, T-453, T-454, T-455 (open), T-428, `docs/decisions.md`
(2026-11-15, The release gate; 2026-11-17, The QA round and the release freeze), `PodUtils/podnet#649` (COORD-102),
`PodUtils/podrunner#75` (the live gate on the H0 host), `PodUtils/podssh#308`, `PodUtils/podvm#567`.

**Problem.** External testers found 40 issues (#609 to #648) in one hour on `v0.4.0-beta.15`. The gate had proved that
the binary starts and runs one success path in labs. It checked no error path, no output contract and no promise of
the help. The owner froze releases until T-450 to T-455 are done. Open parts:

* T-452: error-path rows of the lane (a dead upstream, a refused connect, a reject rule, an unreachable target, a name
  with no answer).
* T-453: a lab of the host class of the testers: no `AF_INET` bind, no netns, no write of program memory. The defect
  of #610 hid where podnet cannot write program memory. `pod-lab` has `bind-denied` but maybe no profile with the three
  parts together. The test hook `PODNET_NETNS_TEST_MEM=none` gives the class to tests. The ask for a profile goes to
  podcrates (ask 1 of COORD-102).
* T-454: a black-box QA pass (a reader with the binary and the documents only) before each beta.
* T-455: readiness measured as promises with a test, not commands with a scenario (`scripts/gate-scenarios.py`
  counts reach).
* T-450: a rule that an issue closes only with a test that fails on the old code.
* T-358 remainder: the lane row for `ipv6-only` on a host with IPv6 (the GitHub runner has no IPv6 route); the `icmp`
  rows wait for a release build with the feature (H-26); a Python consumer, `podnet pod` around a dynamic program, the
  relay and the tunnel as podssh and podrunner call them (the last is open). The labs `arm64-tcg` and `cgroup-v1` do
  not hold on the build container; `nonroot-nocaps` does not hold for root; 9 of 34 labs never hold here. A
  `noexec-home` lab fails if the binary sits in HOME.
* T-398: 1 command stays experimental: `net tailscale` (needs a `tailscaled` in a tailnet with an auth key). Two
  others have no scenario of the release binary in the notes of beta.17: `ssh-relay` (an SSH relay of another party)
  and `warp license` (a WARP+ key).

**Why hard.** A gate that proves correctness needs hosts of the testers' class, which a CI runner is not. A lab that
refuses write to program memory is a kernel or LSM property (Yama, hidepid), which `pod-lab` must emulate with seccomp
and mounts. "Ready" must come from a run on a constrained host and an outside pass.

**What was tried.** `gate.yml` runs the lane (245 rows, 226 pass, 0 fail, 24 n.a. in the dry run of beta.17), the
doc-commands check (`scripts/doc-commands.py`), three consumers (.NET `HttpClient`, Python `requests`, a pod around
python and curl: 148 rows pass), `tests/contracts.rs` and the help-flag check (T-451, done: 69 false flag references
and 20 contract faults fixed).

**Constraints.** C5, C9.

**Research questions.** How can a lab emulate a host that refuses `/proc/PID/mem` writes and `process_vm_writev` for
a child (`hidepid`, `ptrace_scope=3`, a seccomp filter that blocks `process_vm_writev`; a mount that hides
`/proc/PID/mem`)? What is a cheap, automated, repeatable "black-box QA pass" with an agent that has only the binary and
the documents?

**Owner.** podnet (rows, docs), podcrates (`pod-lab` profiles). **Blocks podrunner:** yes (podrunner's own H0 gate and
T-372 depend on the same classes). **Grade: hard.**

## H-29. Flaky tests and live steps under load

**Ids.** T-478, T-406 (H-17), T-414, T-415 (H-15), T-488 (H-11), T-476 (H-19), T-492 (partial), T-482 (open),
T-479 (done), `docs/HANDOFF.md` ("Known flakes under load"), `PodUtils/podnet#606` (open, "live: two runs in a
row failed", run 43918102449).

**Problem.** About one `live` run of six fails at the public relay (T-406). The "Known flakes" list: these tests fail at
times in a full run and pass alone:
`quick_tunnel_standin_serves_a_visitor_over_http2`, `service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection`, `ws_relay::...::a_demoted_relay_host_recovers_on_success`,
`autostart::sigterm_is_a_clean_stop`, `doctor_ssh_relay_row_dials_through_the_proxy`,
`transfer_over_the_relay_crosses_the_cap` (T-476) and `guest_wire_unix_socket_serves_a_guest_and_the_next` (keep the
default `TMPDIR`). T-492: a test frees a port and binds it again; another test takes the port first
(`road::tests::the_pin_cookie_of_a_plain_tcp_listener_is_not_secure` failed with `Address already in use (os error 98)`
in `ci` 44498923514 on `a3386ee`); steps 1 and 2 are done, step 3 (the users of `free_port` in
`tests/commands.rs` and `tests/warp_stand_in.rs` take a port that the child reports) is open. T-482: `pod --backend
netns` prints a second false cause after a refused namespace (`write uid/gid maps: the child died before it named its
namespace`; the cause is the first line; fix in `crates/podnet-netns/src/netns/mod.rs:90` and
`crates/podnet-netns/src/kernel.rs:55`). The rule of the record: a flake is a defect, not a re-run. The disk allows
about one debug build.

**Why hard.** The causes are in several places: a server outside the repository, a shared crate, port races, load
timing. Each failure needs its own log.

**What was tried.** Logs and limits in the live steps; `pod_testkit::refused` for refused ports (T-479); a helper for
each project is asked of `pod-testkit` (COORD-106, `#653`).

**Constraints.** C5, C11.

**Owner.** podnet and podcrates (`pod-testkit`). **Blocks podrunner:** yes, by inference of this report: a red live gate delays a beta that
podrunner needs. **Grade: hard.**

## H-30. crates.io publish and stable version

**Ids.** T-005, T-006, T-007 (blocked), T-053, T-054 (open), `PodUtils/podnet#161`, `#222`, `docs/decisions.md`
(2026-11-05, "crates.io", "Git dependencies and the publish"; 2026-11-11).

**Problem.** The owner ruled: betas only, no crates.io, no stable release now. Facts: the name `cfrs` on crates.io is
a different project; the owner published placeholders (`podnet`, `podnet-core`, `podnet-netns`, `podnet-vnet`,
`podnet-serve`, version 0.0.0). The workspace version is a prerelease (`0.4.0-beta.N`), and the requirement between
the crates (`podnet-netns`) is a prerelease: crates.io does not match `^0.4.0` to a prerelease, and `cargo install`
skips it. `cargo publish --dry-run` does not find that fault. A `pod-*` dependency by a git tag stops the dry run until
podcrates publishes its crates to crates.io. `.github/workflows/publish.yml` (trusted publishing, no token) exists.
T-053: the root package is the CLI and a facade that keeps each library path; T-054: a prerelease after the split with
static builds of each target.

**Constraints.** C1, C3 (podcrates must publish first).

**Owner.** the owner and podcrates. **Grade: hard** (a policy and ordering problem with a hard dependency on
podcrates).

## H-31. Account-bound measurements

**Ids.** T-003 (blocked), T-210 (blocked), T-221 step 4 (blocked), T-247 (H-09), T-114 (H-08), T-001, T-005,
`docs/decisions.md` (2026-11-11, Deferred and blocked work: an entry that needs an account, a token or a capture of an
official client to test, and that a mock cannot stand in for, stays deferred or blocked).

**Problem.**

* T-003 / `#149`: `warp register` cannot enrol through a Zero Trust team token. `src/warp/register.rs` creates an
  anonymous consumer device only. The proposal: `--team-token`, `--team-token-file`, `--team-sni`; parse the JWT
  locally (`iss`, `aud`, `exp`; no signature check); send the token on the `POST /reg` of a new device; the MASQUE TLS
  name defaults to `zt-masque.DOMAIN`. The header name `CF-Access-Jwt-Assertion` is a proposal, not measured. Blocker:
  a captured exchange of the official client against a test Zero Trust team (the request and answer of the team
  token step, and the `POST /reg`).
* T-210 / `#407`: `warp_plus_label` returns `yes` for each non-empty `account_type` that is not exactly `free`, or when
  `premium_data > 0`. A free account of another type reads as WARP+. No list of the types that the service sends
  exists. Blocker: `podnet warp status --json` of a free device and of a device with a licence (a paid account).
* T-221 step 4: a live step that runs `podnet bench` against podnet and `cloudflared` on one runner needs a Cloudflare
  test account. Steps 1 to 3 are done (`src/cli/cmd/bench.rs`).
* T-247 and T-114: see H-09 and H-08.

**Why hard.** The input is a secret or a capture that only the owner or a tester can give. A wrong guess against a
live service can register devices that a team does not want.

**Owner.** the owner. **Grade: very hard** (blocked on outside input; once the capture exists, each is small).

## H-32. Frozen code, takes of crates, and the survey of copies

**Ids.** T-044 (blocked), T-137 (partial), T-473, T-494 (open), T-053, T-126 to T-129 (open), T-123, T-124 (open),
`docs/maintenance.md` ("Shared crates"), `sh scripts/agent/podcrates.sh`.

**Problem.** podnet takes `PodUtils/podcrates` crates by git tag (now `v0.78.1`). Files that podcrates imported are
frozen until podnet takes the crate. A fix in frozen code is a COORD issue. This makes some work wait:

* T-044: four upward references stay in `scripts/layers-allow.txt` (`warp -> cloudflare`, `warp -> quicktunnel`,
  `ws -> cloudflare`, `ws -> proxy`); each starts in frozen code, so a cut is a change beyond a fix. The rows
  disappear when podnet takes `pod-ws`, `pod-warp` and `pod-cloudflare` and the layers allowlist empties.
* T-473 and T-494: podnet's copies of shared code. The detach lock (podcrates entry 879), the update with cosign
  (entry 880: `pod-release`), and the instance lock (entry 877) wait for crates. `pod-release` must keep the ruling
  "check SHA256SUMS, the certificate identity and the signature of the cosign bundle with own code; the Rekor inclusion
  proof is not checked", or the ruling changes.
* T-126 to T-129, T-294, T-295: egress roads (`https://` proxy, wstunnel, VLESS, Shadowsocks, Wisp), DERP carrier,
  ACME client. The work goes to `pod-egress`, `pod-derp`, `pod-acme`. A local podcrates checkout (commit `edbc86b`) has
  these crates (README present for `pod-egress`, `pod-derp`, `pod-acme`, `pod-iroh`, `pod-tor`, `pod-opentunnel`).
  The record of podnet has not taken them. No `pod-ts` was found (see H-08).
* The ruling "podrunner first" (2026-11-18): a red CI run, then each ask of podrunner, then asks of githome and cfb,
  then the take of each tag, then podnet's own work.

**Why hard.** An ordering and ownership problem. A solution that copies code breaks the rulings.

**Owner.** podcrates (hand-offs `adoption/podnet/CRATE.patch`), podnet (the take). **Blocks podrunner:** yes (it sets
the order). **Grade: hard** (process, not research).

---

# Appendix A: Open items that are not hard

These open records need only work, not research. They are listed so that nobody searches for them.

| Entry | Subject | Note |
| --- | --- | --- |
| T-244 | `tunnel --inspect` (capture, ring, redact, replay over a 0600 socket) | Start condition holds (podnet took `pod-cloudflare`). Step 7 is a COORD issue to podcrates. Implement from the description, not from the MPL-2.0 donor source. |
| T-053, T-054 | The root package as a facade, a prerelease after the split | See H-30. |
| T-001 | Tracking issue #89: a guide for a new client | Kept open by the owner. |
| T-482 | False second cause of the netns backend | See H-29. |
| T-492 step 3 | `free_port` users | See H-29. |
| T-450 | Rule: an issue closes only with a reproducing test | Written in the rulings; the record entry stays open. |
| T-473, T-494 | Survey of copies; take `pod-release` | See H-32. |

# Appendix B: Facts the record marks "not measured"

Each is a place where a claim has no live evidence. A solver can pick one as a measurement task.

* A WireGuard peer of another code base for `wg serve`; a real host with no UDP (release notes beta.13).
* The access-token and anonymous forms of `devtunnel://` (tests use the stand-in of `pod-devtunnel`).
* A live `tailscaled` and login (`net tailscale`, `pod -x tailscale:`); the field name `CurrentTailnet` (from upstream
  source, not pinned).
* A live TURN server; a live WARP rotate; a live meeting at the public mailboxes; a sealed session through the real
  relay (`--seal`); a podssh operator against a podnet node with `--seal`.
* A link over a proxy chain, a resume of a session over `ws`, `h2` or `quic`, and a host that denies TCP loopback or UDP
  (CI jobs `restricted-host` and `no-loopback` exist; the notes say "not run here").
* A real satellite or cellular link (the delay in the tests is a loopback forwarder, `TODO/adapters.md`).
* The `UDP ASSOCIATE` door with a real SOCKS5 client program; UDP through a real WARP, relay or tailnet network; a
  prefix file of 1 000 000 lines for the rules.
* The host-class facts in `docs/target-environment.md` that carry the mark "inferred" (no passwd entry, `noexec`
  temporary directory, no `/proc`, no IPv6).

# Sources read

Files of this repository (read on 2026-11-20):

* `CLAUDE.md`, `TODO/INDEX.md`, `TODO/PROGRESS.md`, `TODO/RULES.md` (first lines), and
  the entries of `TODO/deferred.md`, `TODO/fixes.md`, `TODO/crates.md`, `TODO/later.md`, `TODO/link.md`,
  `TODO/overlays.md`, `TODO/services.md`, `TODO/split.md`, `TODO/pods.md` (grep for "Left", "Not proven") named in
  the items.
* `docs/HANDOFF.md`, `docs/decisions.md`, `docs/maintenance.md` (grep), `docs/releases/v0.4.0-beta.13.md`,
  `v0.4.0-beta.16.md`, `v0.4.0-beta.17.md`.
* `docs/pods.md`, `docs/virtual-network.md`, `docs/target-environment.md`, `docs/warp.md`, `docs/relay.md`,
  `docs/transfer.md`, `docs/protocol.md`, `docs/link.md`, `docs/networks.md`, `docs/router.md`, `docs/proxies.md`,
  `docs/doctor.md` (grep for limits).
* `crates/podnet-netns/README.md`.
* `docs/research/podnet/guest-wire-router.md`, `mwan-field.md`, `hosted-relays.md`, and file lists of the others.

Outside this repository (read on 2026-11-20):

* The open issues of `PodUtils/podnet` (44 issues, titles; `#606` and `#608` bodies), by the GitHub MCP tool. The
  `curl` route of the web services was refused by the sandbox guard, so the GitHub REST API service was not used.
* Local copies of messages of the watcher (`.work/agent/issues/`) for `PodUtils/podrunner#201`, `#217`, `#309`, `#312`,
  `PodUtils/podssh#308`, `PodUtils/podvm#567`, `PodUtils/podcrates#125` and the titles of the open `(to podnet)` and
  `(to all)` issues. Other open COORD issues of the other repositories were not read in full.
* Local cargo checkouts of `PodUtils/podcrates` (README files of `pod-egress`, `pod-derp`, `pod-acme`, `pod-link`,
  `pod-iroh`, `pod-tor`, `pod-opentunnel`, `pod-turn`). The checkout tags were not matched to `v0.78.1`.
* Web search (`search.web.pkgforge.dev`) for the leads on seccomp, Landlock and MASQUE; `seccomp_unotify(2)` read
  through `read.web.pkgforge.dev`.

Not checked: the live state of `PodUtils/podcrates`, `PodUtils/podrunner`, `PodUtils/podvm` and `PodUtils/podssh`
issues after the local copy of 2026-11-19; whether the crates named above are in a tagged release; whether a
newer podnet release exists.
