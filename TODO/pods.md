This file holds the entries of milestone M7: pods that live past their first program, generic
UDP in the netns backend, and the netns backend as the first choice of `warp exec`. Sources: the
field report of 0.3.0-beta.12 (`docs/reports/field-report-2026-11-05.md`, item 1);
`docs/research/podnet/mwan-field.md`, section 3 (W-F1, rank 8); `docs/research/podnet/vn.md`,
section 3.1 (P1, P2). An entry that changes the backends lands before the split of T-040
starts, or after it ends.

# T-107: seccomp: the pod lives past its first program

**Source:** `docs/reports/field-report-2026-11-05.md`, items 1 and 8 (request 1);
`docs/research/podnet/mwan-field.md`, section 3 (F1, W-F1, part a).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The pod ends when its first program exits. A program that starts a daemon (`rad node start`
starts `radicle-node`) loses the network of the daemon when the starter exits. The reporter
called this the highest cost of the session.

## Premise

- Read: `supervise_seccomp` sets `shutdown` when `waitpid` returns for the first child, drains
  the relays for `RELAY_DRAIN`, and exits (`crates/podnet-netns/src/seccomp/mod.rs`).
- The notification descriptor gives POLLHUP when no task uses the filter.

## Approach

1. When the first child exits and other tasks still use the filter, return the exit code of the
   first child to the caller at once.
2. Keep a supervisor in a forked process until POLLHUP of the notification descriptor.

## Prove

```sh
cargo test --test pod_seccomp -- seccomp_pod_outlives_its_first_child
```

A C program starts a daemon and exits 0. The daemon connects after 2 s through the pod, and the
connect succeeds.

## Start condition

T-063 is done.

# T-108: netns: the pod lives past its first program

**Source:** `docs/research/podnet/mwan-field.md`, section 3 (F1, W-F1, part a).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

The netns backend ends its router when the first program exits, as the seccomp backend does
(T-107).

## Premise

- Read: the netns backend returns when `waitpid` returns for its child
  (`crates/podnet-netns/src/netns/mod.rs`).

## Approach

1. Return the exit code of the first child at once.
2. Keep the router of the namespace while a process lives in it: keep a reaper as process 1 of
   the namespace, and end when it has no child.

## Prove

```sh
cargo test --workspace -- netns_pod_outlives_its_first_child
```

# T-109: `warp exec --detach --name N` keeps a pod under the supervisor

**Source:** `docs/reports/field-report-2026-11-05.md`, items 1 and 8 (request 1);
`docs/research/podnet/mwan-field.md`, section 3 (W-F1, part b).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

`warp exec` has no `--detach`. A daemon in a pod has no supervisor, no name in `ps`, and no logs.

## Premise

- `src/detach.rs` and `src/instances.rs` supervise the other server commands: `ps`, `stop`,
  `logs` and `wait`.
- Read: the arguments of `warp exec` have no detach options (`src/main.rs`).

## Approach

1. Add `--detach`, `--name` and the restart options to `warp exec`.
2. Run the pod under the supervisor. `ps` lists it, `stop N` ends it, and `logs N` shows its
   output.

## Prove

```sh
cargo test --test detach -- warp_exec_detach_lists_the_pod
```

The test uses the stand-in of T-012.

## Start condition

T-107 and T-108 are done.

# T-110: `warp exec --join N` runs a command in a detached pod

**Source:** `docs/reports/field-report-2026-11-05.md`, item 1;
`docs/research/podnet/mwan-field.md`, section 3 (W-F1, part c).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

A second `warp exec` gets a new pod. The client of a daemon cannot reach the loopback listeners
or the names of the pod of the daemon.

## Premise

- The seccomp backend has no namespace. It keeps the loopback listeners of a pod in its inbound
  directory, and its names in its fake DNS.
- The netns backend has a user and a network namespace for each pod.

## Approach

1. Add `warp exec --join N -- CMD`.
2. seccomp: use the inbound directory and the fake DNS state of the session N.
3. netns: enter the user and the network namespace of the session N with `setns`.

## Prove

```sh
cargo test --workspace -- warp_exec_join_reaches_a_listener_of_the_pod
```

The test runs with each backend.

## Start condition

T-109 is done.

# T-111: netns: generic UDP

**Source:** `docs/research/podnet/vn.md`, section 3.1 (P2) and 3.6;
`docs/research/podnet/mwan-field.md`, section 5 (question 3); GitHub #372.
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The netns backend relays UDP to port 53 only. The seccomp backend relays each UDP flow, but it
emulates the sockets. Thus a user must choose between UDP and correct socket calls.

## Premise

- Read: `answer_dns` takes the DNS datagrams from the TUN before smoltcp
  (`crates/podnet-netns/src/netns/backend.rs`).
- The upstream chain carries UDP in the stream with 0x83 frames
  (`crates/podnet-netns/src/upstream.rs`).
- A smoltcp UDP socket for each port loses datagrams of two flows to one port (the defect of the
  vn kit).

## Approach

1. Keep a table of flows by source and destination, with a reap after an idle time and a cap.
2. Take each datagram from the TUN before smoltcp, as `answer_dns` does. Build each reply as a
   packet with the addresses swapped.
3. Carry the datagrams in the stream of the upstream, else with SOCKS5 UDP ASSOCIATE. Send a
   name of the fake DNS as a name.
4. Add a test that a datagram of zero bytes survives the frame code.
5. The pump of a datagram over a stream follows the rule of T-099 step 4: a pending buffer with an offset,
   a bounded queue, and a pause with no failover when the queue is full (GitHub #372). The smoltcp
   socket items are in T-176.

## Prove

```sh
cargo test --workspace -- netns_backend_relays_udp netns_backend_relays_udp_in_the_stream netns_two_flows_to_one_port_lose_nothing netns_udp_flow_is_reaped an_empty_datagram_survives
```

# T-112: netns: the gateway address reaches the loopback of the host, when the user asks

**Source:** `docs/research/podnet/vn.md`, sections 3.1 (P1) and 5 (question 1).
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

A netns pod cannot reach a service on the loopback of the host. podman and slirp4netns give a
gateway address for it.

## Premise

- The gateway of the TUN of the pod is fixed (`crates/podnet-netns/src/netns/mod.rs`).
- A map of the gateway to the host loopback lets the pod reach each service on 127.0.0.1 of the
  host.

## Approach

1. Add `--host-gateway`, off by default.
2. With it, a TCP connect to the gateway and a port connects 127.0.0.1 and that port on the host.
   A UDP datagram to the gateway (not port 53) goes to a UDP socket of the host.

## Prove

```sh
cargo test --workspace -- netns_host_gateway_reaches_the_host_loopback netns_without_host_gateway_is_refused
```

## Start condition

T-111 is done.

# T-113: `warp exec` and `warp shell` try the netns backend first

**Source:** the owner's ruling of 2026-11-05 (`docs/decisions.md`, Pod backend);
`docs/research/podnet/mwan-field.md`, section 5 (question 3); `docs/research/podnet/vn.md`,
section 5 (question 5).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

`warp exec` passes `--prefer-udp`, so `auto` takes the seccomp backend first, with its emulated
sockets.

## Premise

- Read: `src/warp/up.rs` passes `--prefer-udp`, and `crates/podnet-netns/src/lib.rs` orders the
  backends.
- After T-063 and T-064, the seccomp backend keeps TCP sockets correct. After T-111, the netns
  backend carries each UDP flow.

## Approach

1. Make `auto` of `warp exec` and `warp shell` try netns first, then seccomp when the user
   namespace or the TUN fails.
2. Make the line of T-071 name the backend and the reason of the choice.
3. Change the manual and `README.md`.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Pod backend): after the fix of the TCP sockets of the
seccomp backend and generic UDP in the netns backend, `warp exec` and `warp shell` try the netns
backend first, then seccomp.

## Prove

```sh
cargo test --workspace -- warp_exec_tries_netns_first warp_exec_falls_back_to_seccomp
```

## Start condition

T-063, T-064, T-071 and T-111 are done.

# T-150: seccomp: an accepted inbound connection goes to the program with no relay (GitHub #351)

**Source:** GitHub #351 (https://github.com/PodUtils/podnet/issues/351) (fix 1, the inbound part)
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

`install_accepted` answers `accept` with one end of a new socketpair and copies the bytes between the
other end and the accepted connection in user space (`copy_bidirectional`, `seccomp.rs:1458`).

## Premise

- Read: the accepted connection is a unix stream (`seccomp.rs:1440-1460`, a unix listener).
  `notif_addfd_send` can install that descriptor directly. T-065 hands off unix upstreams already
  (`handoff.rs:57`), so the inbound case is inside the T-065 call (unix only).
- After the handoff the supervisor sees no byte of the flow (the same cost as T-065).
- Not measured: the gain.

## Approach

1. Install the accepted unix stream (`into_std`, then `notif_addfd_send`) and drop the relay task.
2. Log the handoff under `-v`, as `connect.rs:200` does.
3. Keep the relay when the supervisor must read the first bytes (none today).

## Prove

```sh
cargo test --test pod_seccomp_handoff -- seccomp_inbound_accept_leaves_no_relay
cargo test --test pod_seccomp -- seccomp_backend_accepts_inbound seccomp_inbound_wakes_epoll_and_accept_does_not_block_the_child
```

# T-151: seccomp: measure the relay throughput, then replace the copy with splice (GitHub #351)

**Source:** GitHub #351 (https://github.com/PodUtils/podnet/issues/351) (fixes 2 and 3)
**Category:** measurement
**Milestone:** M7
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

Each relayed TCP byte crosses the supervisor twice: `copy_bidirectional` with an 8 KiB buffer
(`seccomp.rs:1458`, `connect.rs:214`, `frontdoor.rs:117`, `:137`, `:187`). The issue cites a 30x gap
for host-socket installation in a peer tool (arXiv 2402.00365). That figure is not checked here.

## Premise

- Read: no `splice` call exists in `crates/podnet-netns/src` (grep). tokio has no splice.
- A TCP SOCKS5 or HTTP upstream, a chain, a loopback connect and a tailnet dial keep the relay
  (`handoff.rs:57`, T-065 Correction).
- The Decision rules the handoff of a TCP upstream.

## Approach

1. Measure: `curl` of a 1 GiB local file through `podnet pod --backend seccomp -x socks5://...`,
   and the same `curl` outside the pod. Record both in this entry.
2. Only when the gap is more than 2x: write a splice loop (two pipes, `AsyncFd`, fall back to
   `copy_bidirectional` on `EINVAL`). The loop returns the byte totals that `-v` reads.
3. Add a `pod doctor` row for splice on both leg types, gated on kernel 6.5.

## Decision

The lead, 2026-11-09: measure first (step 1 of the Approach), then decide. If the gap is more than 2 times, allow the handoff of a TCP upstream behind a flag that is off by default. The splice loop keeps the byte counts and needs no ruling. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
python3 -m http.server 8000 &   # 1 GiB file served; a SOCKS5 proxy on 127.0.0.1:1080
podnet pod --backend seccomp -x socks5://127.0.0.1:1080 -- curl -o /dev/null -w '%{speed_download}\n' http://127.0.0.1:8000/1g
cargo test --workspace -- seccomp_splice_relay_keeps_the_byte_totals
```

# T-152: netns: measure in-namespace netfilter on the target hosts (GitHub #352)

**Source:** GitHub #352 (https://github.com/PodUtils/podnet/issues/352)
**Category:** research
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

The netns backend ends TCP in smoltcp over a TUN and carries only DNS (`netns.rs:5-10`, `:861`).
T-111 plans a flow table for UDP. The issue proposes a different data plane: the child installs
`nat OUTPUT REDIRECT` rules through nfnetlink in its own user and net namespace, and the parent serves
the redirect listeners (`SO_ORIGINAL_DST`, conntrack for UDP). It also proposes `ping_group_range` for
ICMP echo.

## Premise

- Measured on this host (kernel 6.18.44, root, `unshare -Urn`): `nft add table ip t`, a `type nat hook
  output` chain and `tcp dport 80 redirect to :9999` all succeed. `/proc/net/nf_conntrack` exists.
- Measured: `echo "0 2147483647" > /proc/sys/net/ipv4/ping_group_range` fails with an I/O error in the
  new namespace. `0 0` works. The range must lie in the mapped gids, so fix 3 of the issue needs the
  mapped range.
- Not measured: unprivileged non-root start, `SO_ORIGINAL_DST`, the conntrack lookup, autoload of
  `xt_REDIRECT` and `nf_conntrack` on a restricted host (CI runner, container).
- The child holds `CAP_NET_ADMIN` in its namespace (`raw.rs:206-208`). The parent cannot join it.
- No record entry names a veth wire. `docs/research/podnet/vn.md` P5 describes it.

## Approach

1. Write a probe (a `pod doctor` row, `crates/podnet-netns/src/doctor.rs`): in a throwaway user and net
   namespace, add the table, the chain and the REDIRECT rule, read `SO_ORIGINAL_DST` on a loopback
   connect, and read the conntrack entry of a UDP flow.
2. Run it on this host, on the CI runner and in a container. Write the results here.
3. Decide T-153 from the results (see Decision).

## Decision

The lead, 2026-11-09: keep the flow table of T-111 as planned. This measurement runs now. The result decides the parked entry of the netfilter plane. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
unshare -Urn sh -c 'nft add table ip t && nft "add chain ip t out { type nat hook output priority -100 ; }" && nft add rule ip t out tcp dport 80 redirect to :9999'
cargo test -p podnet-netns --lib -- doctor_netfilter_row_has_a_verdict
```

# T-156: pod: a `reject` action in `-r`, for TCP and for datagrams (GitHub #354)

**Source:** GitHub #354 (https://github.com/PodUtils/podnet/issues/354)
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

A rule of `pod -r` selects a proxy and cannot refuse. A default-deny policy needs a dead proxy. The WARP
rules have `reject` and `block` (`src/warp/rules.rs:30-50`). A datagram send does not read rules.

## Premise

- Read: `Rule { matcher, proxy }` (`upstream.rs:299`). `cli.rs:188-198` splits at `=` and calls
  `Proxy::parse(rhs)`. `Proxy::parse("block")` fails with "needs host:port" (`upstream.rs:144-146`).
  So `block` and `reject` both fail today. The issue says only `reject` fails.
- Read: four callers of `select_chain`: `connect.rs:132`, `frontdoor.rs:92`, `:123`, `netns.rs:498`.
- Read: the datagram handlers take `&[Proxy]` only (`seccomp.rs:783-793`).
- Read: `src/warp/rules.rs` already imports `RuleMatch` of podnet-netns, so one matcher is shared.
- `upstream.rs` is 773 of 800 lines.

## Approach

1. Move `RuleMatch`, `Rule` and `select_chain` into a new file. Add `RuleAction { Proxy, Reject }`.
   Parse `reject` and `block` at the `=` split.
2. `select_chain` returns the action. Seccomp connect answers `ECONNREFUSED`; netns answers the RST;
   the front door answers SOCKS5 reply 0x02 and HTTP 403 (the WARP form). Count the refusal under `-v`.
3. Pass `rules` to `handle_sendto`, `handle_sendmsg`, `handle_sendmmsg`. A refused datagram gets
   `EACCES`.
4. Add a test that both rule engines read the same two words. Update the `-r` help and the README.
5. State the backend that honours it. Loopback stays direct (`connect.rs:117-130`).

## Prove

```sh
cargo test --test pod_seccomp_rules -- seccomp_block_rule_refuses_connect seccomp_reject_rule_refuses_a_datagram
cargo test --workspace -- reject_and_block_mean_the_same_in_both_rule_engines
```

# T-157: pod: `--default-action` and a JSON flow record (GitHub #355)

**Source:** GitHub #355 (https://github.com/PodUtils/podnet/issues/355) (fixes 1 to 3, 5)
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

A flow that matches no rule reaches the default chain. There is no way to refuse every undeclared
destination, and no machine record of (destination, rule, verdict). The `-v` log is for humans
(`cli.rs:104`). The decision counters (`src/decisions.rs`) serve `warp proxy` only.

## Premise

- Read: `select_chain` returns the default chain for any target (`upstream.rs:334`).
  The dial log is `connect.rs:133-135`. `copy_bidirectional` at `connect.rs:214` knows the byte totals
  at the close.
- The refuse action itself is T-156. The datagram path is T-156 too.
- The ACL of the vnet has `default_deny` (`src/vnet/policy.rs:265`): the same posture.

## Approach

1. After T-156: add `--default-action allow|reject` (`allow` is today).
2. Add `--flow-log FILE`: one JSON line at dial time (time, target, rule or `default`, chain, verdict)
   and one at close (bytes in, bytes out). Keep `-v` for humans.
3. Write the semantics for each backend in the README (seccomp: one reply; netns: RST).

## Prove

```sh
cargo test --test pod_seccomp_rules -- seccomp_default_reject_refuses_an_undeclared_host seccomp_flow_log_has_one_line_per_decision
```

# T-158: pod: `--learn` and `--fail-on-violation` (GitHub #355)

**Source:** GitHub #355 (https://github.com/PodUtils/podnet/issues/355) (fix 4)
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

A CI job needs to assert that a workload touched only the destinations that it declared.

## Premise

- Depends on T-156 and T-157 (the refuse action and the flow log).
- A workload whose destinations change between runs exceeds the learned set. That is the case that
  `--fail-on-violation` must catch.

## Approach

1. `--learn`: run with a refusing rule, record the flow, then use the default `-x` chain. Write the
   learned rule set.
2. `--fail-on-violation`: run under enforcement. Exit non-zero when the flow log contradicts the learned
   set (a refused flow that learning saw allowed, or an allowed flow that learning never saw).

## Prove

```sh
cargo test --test pod_seccomp_rules -- seccomp_learn_then_enforce_passes seccomp_enforce_fails_on_a_new_destination
```

# T-174: netns: a veth wire when `/dev/net/tun` is not there (GitHub #369)

**Source:** GitHub #369 (https://github.com/PodUtils/podnet/issues/369)
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

The netns backend needs user namespaces and `/dev/net/tun`. A host that denies the node and allows user
namespaces falls to the seccomp backend (no real sockets, QUIC breaks). A veth pair made in the guest
namespace, with an `AF_PACKET` socket on one end, can carry the frames to the same smoltcp stack.

## Premise

- `crates/podnet-netns/src/netns/tun.rs:41-43` opens the node; `send_fd`/`recv_fd` (`crates/podnet-netns/src/netns/fdpass.rs:51,85`) and the
  handoff (`crates/podnet-netns/src/netns/mod.rs:96,168`) are reusable. `doctor.rs:70,281` gates `auto` on the TUN probe.
- smoltcp has `medium-ip` only (`Cargo.toml:117`, `crates/podnet-netns/Cargo.toml:26`): add `medium-ethernet`.
- `docs/research/podnet/vn.md:313` is plan item P5; `vn.md:411` asks if any target host has no TUN. No
  measured host lacks it (`docs/target-environment.md:33`).
- The veth needs one small `RTM_NEWLINK` writer. Checksums: skip RX verification on this device.
- Overlap: #352 (netfilter instead of smoltcp).

## Approach

1. `Wire::{Tap, Veth}`; veth only when the TAP open fails. Drop frames of the own MAC.
2. `auto` picks netns when TAP or veth works; doctor row for the veth probe; README "Backends".

## Decision

The lead, 2026-11-09: keep the veth wire as a parked P3 entry. Drop it if no host with user namespaces and no `/dev/net/tun` is measured by the end of M7 (`docs/research/podnet/vn.md:411`). The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet-netns veth_wire_carries_a_tcp_flow
```

The test hides `/dev/net/tun` in a private mount namespace.

## Start condition

The owner can rule on this (see Decision). A pod-backend feature starts after T-040 ends.

# T-181: seccomp: a budget of tracked flows with counters, a documented ceiling and a doctor row (GitHub #376)

**Source:** GitHub #376 (https://github.com/PodUtils/podnet/issues/376)
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

The supervisor refuses a new socket at `MAX_TRACKED_FDS = 4096` with `EMFILE`. A failed handler answers
`EIO` or `ECONNREFUSED`. The ceiling is fixed, not documented, not tied to `RLIMIT_NOFILE`, and has no
counter. The pod sees a bare errno.

## Premise

- `crates/podnet-netns/src/seccomp/mod.rs:384` (constant), `fdtable.rs:74-82` (`has_room` sweeps then
  refuses), `socket.rs:19-21` (`EMFILE`), `seccomp/mod.rs:326-331` (`EIO` or `ECONNREFUSED`).
- Not measured: the 5000-connection run of the issue.

## Approach

1. Count the refusals by class (table full, `socketpair` failed, other).
2. Derive the ceiling from `RLIMIT_NOFILE`; make it configurable.
3. Show the counters and the ceiling in `pod doctor`; document them in `crates/podnet-netns/README.md`.
4. Per-door counters come with the router.

## Prove

```sh
cargo test -p podnet-netns fdtable:: -- refusals_are_counted_by_class
```

## Start condition

After T-040 (the split of the seccomp backend) ends.
