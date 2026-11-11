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
**Status:** done

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

## Done

2026-11-10, commit c08818a (`T-108, T-107: the pod lives past its first program`). The supervisor
runs in a back process (`crates/podnet-netns/src/outlive.rs`) and is the subreaper of the pod. At
the exit of the first child, when other tasks use the filter, the front returns the exit code at
once; the back reaps each orphan and ends at POLLHUP or when it has no child. With no other task,
the pod drains the relays as before. Test: `seccomp_pod_outlives_its_first_child`
(`tests/pod_seccomp/outlive.rs`). `TMPDIR=/tmp/tm7 sh scripts/agent/check.sh`: each step 0.
CI: at the merge.


# T-108: netns: the pod lives past its first program

**Source:** `docs/research/podnet/mwan-field.md`, section 3 (F1, W-F1, part a).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** S
**Status:** done

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

## Done

2026-11-10, commit c08818a (`T-108, T-107: the pod lives past its first program`). The first
process of the namespace is a reaper (`PR_SET_CHILD_SUBREAPER`): it starts the program, reaps each
orphan, and reports the status of the program to the router. The pod runs in a back process
(`crates/podnet-netns/src/outlive.rs`); the front returns the exit code at once, and the router
stays until the reaper ends. Test: `netns_pod_outlives_its_first_child` (`tests/pod_seccomp/outlive.rs`):
the pod returns in less than 2.5 s, and the daemon connects through the pod after 3 s.
`TMPDIR=/tmp/tm7 sh scripts/agent/check.sh`: each step 0. CI: at the merge.


# T-109: `warp exec --detach --name N` keeps a pod under the supervisor

**Source:** `docs/reports/field-report-2026-11-05.md`, items 1 and 8 (request 1);
`docs/research/podnet/mwan-field.md`, section 3 (W-F1, part b).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`warp exec` has no `--detach`. A daemon in a pod has no supervisor, no name in `ps`, and no logs.

## Premise

- `crates/podnet-core/src/detach.rs` and `crates/podnet-core/src/instances.rs` supervise the other server commands: `ps`, `stop`,
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

## Correction

2026-11-10: the test needs no stand-in of T-012 (blocked). `warp exec` reads only the socket of
`warp up`: a unix socket that answers is the stand-in, as in `tests/pod_backend_line.rs`. The
program of the test opens no connection.

## Done

2026-11-10, commit 6992453 (`T-109: warp exec --detach --name N keeps a pod under the supervisor`).
`warp exec` takes `--detach`, `--name`, `--ready-timeout`, `--replace` and `--no-restart`. A
named or detached pod is an instance (`src/cli/cmd/warp/kept.rs`): the command starts
`podnet pod` as a child and is the subreaper of the pod, so a daemon of the program stays its
descendant. The instance lives until each process of the pod ends, with the exit code of the
first program. `podnet stop N` sends SIGTERM to each process of the pod, and SIGKILL after 5 s.
The program does not get the variables of `--detach`. Test: `warp_exec_detach_lists_the_pod`
(`tests/detach/warp_exec.rs`): the first program starts a daemon and exits; `ps` lists the pod,
`logs` shows the output of the daemon, `stop` ends the daemon. Unit tests:
`descendants_include_the_children_of_a_child`, `exit_codes_of_a_status`. The Prove command:
1 passed. A manual run with `--backend netns` and with `--backend seccomp`: each pod in `ps`, its
output in `logs`, no process left after `stop`.
`TMPDIR=/workspace/podnet/.work/tmp-m7c sh scripts/agent/check.sh`: each step 0. CI: at the merge.

# T-110: `warp exec --join N` runs a command in a detached pod

**Source:** `docs/reports/field-report-2026-11-05.md`, item 1;
`docs/research/podnet/mwan-field.md`, section 3 (W-F1, part c).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

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

## Done

2026-11-10, commit 101d6e2 (`T-110: warp exec --join N runs a command in a detached pod`).
`warp exec --join N -- CMD` reads the ready document of the instance N (its socket, its rules
and its session `RUN/N.pod`), and finds a process of the pod: another network namespace (netns)
or more seccomp filters (seccomp). netns: CMD enters the user, the mount and the network
namespace of that process with `setns` (`crates/podnet-netns/src/join.rs`): it has the loopback,
the fake DNS and the `resolv.conf` of the pod. seccomp: a new pod with `--session RUN/N.pod`; the
new option `podnet pod --session DIR` shares the inbound listeners (`DIR/inbound`) and the fake
DNS table (`DIR/dns`, one line for each fake address, under `flock`). Tests:
`warp_exec_join_reaches_a_listener_of_the_pod` (`tests/detach/warp_exec.rs`, each backend of the
host: a listener of the pod on 127.0.0.1 answers the joined program, and a name gets the address
of the pod N), `two_pods_of_a_session_share_the_fake_addresses`, `this_process_is_in_no_pod`,
`a_named_pod_gives_its_session`. Plant: with no shared DNS table, the seccomp case fails
(`198.18.0.0` against `198.18.0.1`). The Prove command: 1 passed.
`TMPDIR=/workspace/podnet/.work/tmp-m7c sh scripts/agent/check.sh`: each step 0. CI: at the merge.

# T-111: netns: generic UDP

**Source:** `docs/research/podnet/vn.md`, section 3.1 (P2) and 3.6;
`docs/research/podnet/mwan-field.md`, section 5 (question 3); GitHub #372.
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

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

## Done

2026-11-10, commit b63fa79 (`T-111: netns: generic UDP`). New file
`crates/podnet-netns/src/netns/udpflow.rs`: the flow table (60 s idle, 256 flows, the longest idle
flow goes at the cap), one task for each flow (0x83 in the stream for a unix proxy, else
`UDP ASSOCIATE`, both with `pod_proxy::udp`; a `direct` rule uses a host UDP socket), the reply
packets with the addresses swapped (fragments above the MTU), and a queue of 64 datagrams for each
flow: a full queue holds the datagram and stops the read of the TUN, with no failover (GitHub
#372). The Prove command: 5 passed (`netns_backend_relays_udp`,
`netns_backend_relays_udp_in_the_stream`, `netns_two_flows_to_one_port_lose_nothing`,
`netns_udp_flow_is_reaped`, `an_empty_datagram_survives`). Also: `a_large_reply_becomes_fragments`,
`a_partial_stream_write_keeps_the_record_stream_aligned` (a stream that takes 5 bytes for each
write). `TMPDIR=/tmp/tm7 sh scripts/agent/check.sh`: each step 0. CI: at the merge.


# T-112: netns: the gateway address reaches the loopback of the host, when the user asks

**Source:** `docs/research/podnet/vn.md`, sections 3.1 (P1) and 5 (question 1).
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** done

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

## Done

2026-11-10, commit af529c1 (`T-112: netns: the gateway address reaches the loopback of the host, when
the user asks`). `podnet pod --host-gateway`, off by default. With it, the netns backend dials a
TCP connect to the gateway `10.66.255.254` and a port as TCP to `127.0.0.1` and that port of the
host (`crates/podnet-netns/src/netns/backend.rs`), and a UDP datagram to the gateway (not port 53)
is a direct flow from a UDP socket of the host to `127.0.0.1` (`crates/podnet-netns/src/netns/udpflow.rs`).
Without it, the connect gets a RST at once, and the datagram is dropped (before, the flow went to
the proxy). The seccomp backend does not use it (127.0.0.0/8 goes to the host there). Tests:
`netns_host_gateway_reaches_the_host_loopback`, `netns_without_host_gateway_is_refused`
(`tests/pod_seccomp/host_gateway.rs`). The Prove command: 2 passed.
`TMPDIR=/workspace/podnet/.work/tmp-m7c sh scripts/agent/check.sh`: each step 0.
`unshare -n -- cargo test --offline --workspace --no-fail-fast`: exit 0.
`sh scripts/restricted-host.sh --no-fail-fast`: exit 0 (108 tests skipped). CI: at the merge.

# T-113: `warp exec` and `warp shell` try the netns backend first

**Source:** the owner's ruling of 2026-11-05 (`docs/decisions.md`, Pod backend);
`docs/research/podnet/mwan-field.md`, section 5 (question 3); `docs/research/podnet/vn.md`,
section 5 (question 5).
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** S
**Status:** done

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

## Done

2026-11-10, commit 44e323a (`T-113: warp exec and warp shell try the netns backend first`). The
launcher of `warp exec` and `warp shell` is now in the CLI layer (`src/cli/cmd/warp/pod.rs`):
`src/warp/pod.rs` is frozen for `pod-warp` and keeps its old functions with no caller. The pod
gets no `--prefer-udp`: `auto` takes netns when the user and network namespace and the TUN work,
else seccomp. The line of T-071 ends with `Choice: REASON` (the probe that failed, or
`--backend NAME`). Test hook: `PODNET_NETNS_TEST_NO_NETNS`. Tests: `warp_exec_tries_netns_first`,
`warp_exec_falls_back_to_seccomp` (`tests/pod_backend_line.rs`), `warp_exec_auto_has_no_prefer_udp`.
The Prove command: 2 passed. README and the netns manual changed.
`TMPDIR=/workspace/podnet/.work/tmp-m7c sh scripts/agent/check.sh`: each step 0. CI: at the merge.

# T-150: seccomp: an accepted inbound connection goes to the program with no relay (GitHub #351)

**Source:** GitHub #351 (https://github.com/PodUtils/podnet/issues/351) (fix 1, the inbound part)
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** done

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

## Done

2026-11-10, commit 974fce0 (`T-150: seccomp: the program holds an accepted inbound connection`).
`install_accepted` (`crates/podnet-netns/src/seccomp/accept.rs`) installs the accepted unix stream in
the program (`notif_addfd_send`) with the `O_NONBLOCK` of its `accept4`. No socket pair and no relay
task. `-v` prints `accept on port N: the program holds the connection (no relay)`. The supervisor
reads no byte of an inbound connection, so no case keeps the relay (step 3). The Prove commands: 1
passed (`seccomp_inbound_accept_leaves_no_relay`: the accepted socket has the name of the inbound
socket, and the pod exits in less than 2.5 s while the client holds its end open) and 2 passed.
Before the fix, the new test failed (`accepted_path` empty: a socket pair). The new test calls
`test_host::need_loopback!()` (the server binds 127.0.0.1): it skips under `unshare -n` and in
`scripts/restricted-host.sh`. `TMPDIR=/tmp/tm7b sh scripts/agent/check.sh`: each step 0. CI: at the merge.
GitHub #351 (fix 1) closes with T-151.

# T-151: seccomp: measure the relay throughput, then replace the copy with splice (GitHub #351)

**Source:** GitHub #351 (https://github.com/PodUtils/podnet/issues/351) (fixes 2 and 3)
**Category:** measurement
**Milestone:** M7
**Priority:** P3
**Effort:** M
**Status:** done

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

## Correction

2026-11-10: the measure (Done) gives a gap of less than 2 times, so step 2 (the splice loop) and
step 3 (its doctor row) do not run, and the test `seccomp_splice_relay_keeps_the_byte_totals` does
not exist. The relay keeps `copy_bidirectional` with a 64 KiB buffer for each direction;
`cargo test -p podnet-netns --lib -- flowlog::tests::the_relay_keeps_the_byte_totals` proves the byte
totals. The Prove runs with a non-loopback address: with seccomp, a connect to `127.0.0.0/8` goes
direct and does not use `-x`.

## Done

2026-11-10, commit 4b758d0 (`T-151: seccomp: a relay buffer of 64 KiB for each direction`). The
measure: this host (4 CPUs, kernel 6.18.44), the release binary, `curl -o /dev/null` of 1 GiB from an
HTTP server in C on `192.0.2.2:18000`, a SOCKS5 proxy in C (256 KiB buffers, two threads) on TCP
`127.0.0.1:18080` and on a unix socket. Three runs each, in MB/s:

| Path | 8 KiB relay (before) | 64 KiB relay | 256 KiB relay |
| --- | --- | --- | --- |
| `curl` with no proxy | 3994 to 5430 | 4408 to 5590 | 3901 to 4948 |
| `curl -x socks5://127.0.0.1:18080` outside the pod | 1572 to 1745 | (the same proxy) | 1594 to 1642 |
| pod `seccomp`, `-x socks5://127.0.0.1:18080` (relay) | 1095 to 1208 | 1557 to 1824 | 1594 to 1915 |
| pod `seccomp`, `-x unix:` with `--flow-log` (relay) | 1009 to 1161 | 1620 to 1761 | 1857 to 2109 |
| pod `seccomp`, `-x unix:` (T-065 hand-off, no relay) | 2175 to 2838 | 2494 to 3064 | 2522 to 3196 |

The gap of the Approach (the pod against the same `curl` outside the pod) was 1.4 times with 8 KiB
and is 1.0 times with 64 KiB: the proxy is the limit. The relay against the hand-off on the same unix
proxy was 2.2 times and is 1.7 times. Both are less than 2 times, so no splice loop and no TCP
hand-off flag (the Decision). The change: `RELAY_BUFFER` (64 KiB) in
`crates/podnet-netns/src/flowlog.rs`, the relay of each seccomp TCP flow. The cost: 128 KiB for
each relayed flow, not 16 KiB. 256 KiB gave 10 to 20 % more on the unix relay for four times the
memory. The tokio runtime has no splice. Tests: `the_relay_keeps_the_byte_totals` (1 passed), and
`cargo test --test pod_seccomp_rules` (10 passed, the byte totals of the flow record). Also measured:
the netns backend through the same TCP proxy moved 1 MiB at 9 MB/s, 8 MiB in 17 s (0.49 MB/s), and
64 MiB did not end in 120 s; for 1 GiB its process grew to 1 GB of memory. That is a defect of the
netns TCP path, not of this entry. `TMPDIR=/tmp/tm7b sh scripts/agent/check.sh`: each step 0. CI:
at the merge. GitHub #351 (fixes 1 to 3) closes at the merge.

# T-152: netns: measure in-namespace netfilter on the target hosts (GitHub #352)

**Source:** GitHub #352 (https://github.com/PodUtils/podnet/issues/352)
**Category:** research
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** done

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

## Done

2026-11-10, commit 07d150b (`T-152: pod doctor: a probe of netfilter in a user and net namespace`).
`crates/podnet-netns/src/nfprobe.rs`: the row `netfilter in a namespace (not a backend)` of `pod
doctor`. In a forked child: `unshare(CLONE_NEWUSER | CLONE_NEWNET)`, the uid and gid maps, `lo` up;
through nfnetlink a table, a chain `type nat hook output priority -100` and the rule `tcp dport 80
redirect to :9999`; `SO_ORIGINAL_DST` of a connect to 127.0.0.1:80; a UDP datagram to
127.0.0.1:5353 and its entry in a ctnetlink dump. The results:

- This host (Firecracker VM, kernel 6.18.44, nf_tables and conntrack built in: `/proc/modules` is
  empty): each step passes as root and as uid 65534 (`setpriv --reuid=65534 --regid=65534
  --clear-groups podnet pod doctor`): "nft table, nat chain and REDIRECT: ok; SO_ORIGINAL_DST: port
  80; UDP conntrack (ctnetlink): ok". The `nft` command and a C client give the same result.
- `/proc/self/net/nf_conntrack` in the namespace of uid 65534 gives "Permission denied"; root reads
  it. Thus the probe and a backend read conntrack through ctnetlink.
- With user namespaces refused (`max_user_namespaces` 0 in an outer namespace): the row fails at
  `unshare(NEWUSER|NEWNET)` with `ENOSPC`, as the netns backend.
- Not measured: the CI runner (ubuntu-24.04, AppArmor can restrict unprivileged user namespaces;
  the job `test` of `.github/workflows/ci.yml` now prints the row), a container (the Docker
  daemon does not run on this host; the default seccomp profile of Docker refuses
  `unshare(CLONE_NEWUSER)`), and the autoload of `nf_tables`, `nft_redir` and `nf_conntrack` as
  modules.

For T-153: the plane works on this host for an unprivileged user; T-153 waits for the row of the CI
runner and of a container (its Start condition). The Prove commands: the `unshare -Urn` command
succeeds; 1 passed (`doctor_netfilter_row_has_a_verdict`). Also:
`the_batch_has_its_lengths_and_the_dump_parses`. `TMPDIR=/tmp/tm7b sh scripts/agent/check.sh`:
each step 0. CI: at the merge (the row of the runner is in the log of the job `test`). GitHub #352
stays open for T-153.

# T-156: pod: a `reject` action in `-r`, for TCP and for datagrams (GitHub #354)

**Source:** GitHub #354 (https://github.com/PodUtils/podnet/issues/354)
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

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

## Done

2026-11-10, commit 727dc98 (`T-156: pod: a reject action in -r, for TCP and for datagrams`). The
matcher, `Rule`, `RuleAction { Proxy, Reject }` and `select_chain` are in
`crates/podnet-netns/src/rules.rs`; `reject` and `block` parse at the `=` split. A refusal: seccomp
`connect` gives `ECONNREFUSED`; a seccomp datagram (`sendto`, `sendmsg`, `sendmmsg`, `write`) gives
`EACCES`; netns gives a RST and drops a datagram; the front door answers SOCKS5 0x02 and HTTP 403.
`-v` prints each refusal with the count. A TCP connect to loopback with seccomp stays direct. The
Prove commands: 2 passed (`seccomp_block_rule_refuses_connect`,
`seccomp_reject_rule_refuses_a_datagram`) and 1 passed
(`reject_and_block_mean_the_same_in_both_rule_engines`, `tests/rule_words.rs`). Also:
`netns_block_rule_refuses_connect`, `a_rule_with_an_unknown_action_is_a_usage_error`,
`a_reject_rule_refuses_and_names_the_rule`. `TMPDIR=/tmp/tm7 sh scripts/agent/check.sh`: each
step 0. CI: at the merge. GitHub #354 closes at the merge.


# T-157: pod: `--default-action` and a JSON flow record (GitHub #355)

**Source:** GitHub #355 (https://github.com/PodUtils/podnet/issues/355) (fixes 1 to 3, 5)
**Category:** feature
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

A flow that matches no rule reaches the default chain. There is no way to refuse every undeclared
destination, and no machine record of (destination, rule, verdict). The `-v` log is for humans
(`cli.rs:104`). The decision counters (`crates/podnet-core/src/decisions.rs`) serve `warp proxy` only.

## Premise

- Read: `select_chain` returns the default chain for any target (`upstream.rs:334`).
  The dial log is `connect.rs:133-135`. `copy_bidirectional` at `connect.rs:214` knows the byte totals
  at the close.
- The refuse action itself is T-156. The datagram path is T-156 too.
- The ACL of the vnet has `default_deny` (`crates/podnet-vnet/src/policy.rs:265`): the same posture.

## Approach

1. After T-156: add `--default-action allow|reject` (`allow` is today).
2. Add `--flow-log FILE`: one JSON line at dial time (time, target, rule or `default`, chain, verdict)
   and one at close (bytes in, bytes out). Keep `-v` for humans.
3. Write the semantics for each backend in the README (seccomp: one reply; netns: RST).

## Prove

```sh
cargo test --test pod_seccomp_rules -- seccomp_default_reject_refuses_an_undeclared_host seccomp_flow_log_has_one_line_per_decision
```

## Done

2026-11-10, commit c3ecc07 (`T-157: pod: --default-action and a JSON flow record`).
`--default-action allow|reject`: `reject` adds a last rule (`RuleMatch::Any`, name `default`) that
refuses as T-156 does. `--flow-log FILE` (`crates/podnet-netns/src/flowlog.rs`): one JSON line for
each decision (time, target, rule or `default`, chain, verdict) and one at the close of each
relayed TCP flow (`bytes_out`, `bytes_in`), for the seccomp connect and datagrams, the netns TCP and
UDP flows and the front door. With `--flow-log`, the seccomp pod relays each TCP flow (no hand-off)
to count the bytes. The README gives the semantics of each backend. The Prove command: 2 passed.
Also: `netns_flow_log_has_one_line_per_decision`. `TMPDIR=/tmp/tm7 sh scripts/agent/check.sh`:
each step 0. CI: at the merge. GitHub #355 (fixes 1 to 3, 5) closes at the merge.


# T-158: pod: `--learn` and `--fail-on-violation` (GitHub #355)

**Source:** GitHub #355 (https://github.com/PodUtils/podnet/issues/355) (fix 4)
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** M
**Status:** done

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

## Done

2026-11-10, commit dea1f91 (`T-158: pod: --learn and --fail-on-violation`).
`crates/podnet-netns/src/learn.rs`. `--learn FILE`: the pod records each destination (`tcp` or `udp`
and the target); a flow that a rule refuses goes through the default `-x` chain; at the exit of the
program the pod writes FILE, one `PROTO TARGET` line each. `--fail-on-violation FILE`: a destination
that is not in FILE is refused (rule name `not-learned`) and is a violation; a learned destination
that a rule refuses is a violation. The pod writes each violation on stderr; the exit status is 4
when the program exits 0. A seccomp TCP connect to loopback is not recorded. Both work with
`seccomp` and `netns`; with `--serve` they are a usage error. The Prove command: 2 passed
(`seccomp_learn_then_enforce_passes`, `seccomp_enforce_fails_on_a_new_destination`). Also:
`a_learned_set_with_a_bad_line_is_a_usage_error`, `learn::tests::a_learned_set_reads_back`.
`TMPDIR=/tmp/tm7b sh scripts/agent/check.sh`: each step 0. CI: at the merge. GitHub #355 (fix 4)
closes at the merge.

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
- smoltcp has `medium-ip` only (`Cargo.toml:139`, `crates/podnet-netns/Cargo.toml:33`): add `medium-ethernet`.
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
**Status:** done

## Problem

The supervisor refuses a new socket at `MAX_TRACKED_FDS = 4096` with `EMFILE`. A failed handler answers
`EIO` or `ECONNREFUSED`. The ceiling is fixed, not documented, not tied to `RLIMIT_NOFILE`, and has no
counter. The pod sees a bare errno.

## Premise

- `crates/podnet-netns/src/seccomp/mod.rs:368` (constant), `fdtable.rs:74-82` (`has_room` sweeps then
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

## Done

2026-11-10, commit 05c9468 (`T-181: seccomp: a budget of tracked sockets with counters and a doctor
row`). Step 1: `crates/podnet-netns/src/fdtable.rs` counts each refused call by class (`table full`,
`socketpair failed`, `other`); a failed socket pair now gives `EMFILE`, not `EIO`. `-v` prints each
refusal; at the exit of the program the pod prints one line with the counts. Step 2: the default
ceiling is `(RLIMIT_NOFILE - 64) / 2`, from 64 to 65536; the supervisor raises its soft limit to the
hard limit after the fork of the program (the program keeps its limit). `--flow-budget N` (64 to
1048576) sets it; a value above the default gives a warning. Step 3: `pod doctor` has the row
`flow budget (seccomp)` (JSON `flow_budget`: `ceiling`, `nofile_soft`, `nofile_hard`,
`refusal_classes`). The counts of a run are per pod: doctor names the classes, and the pod prints
the counts. `crates/podnet-netns/README.md` documents both. Step 4 waits for the router. On this
host (`RLIMIT_NOFILE` 20000/20000) the ceiling is 9968, before 4096. The Prove command: 1 passed
(`refusals_are_counted_by_class`). Also: `the_ceiling_follows_rlimit_nofile`,
`doctor_flow_budget_row_names_the_ceiling`, `seccomp_flow_budget_refuses_with_emfile_and_counts`
(64 made, 36 `EMFILE`, the count line), `a_flow_budget_out_of_range_is_a_usage_error`. Not measured:
the 5000-connection run of the issue. `TMPDIR=/tmp/tm7b sh scripts/agent/check.sh`: each step 0. CI:
at the merge. GitHub #376 closes at the merge.

# T-229: Measure `podnet pod` with the official runner and three CI payloads

**Source:** PodUtils/podvm#228 (COORD-022 of podvm, 2026-11-10), ask 1; the podrunner design review
(PodUtils/podcrates#47), WP-N1.
**Category:** research
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`podnet pod` has never run the .NET runner of `actions/runner` or another CI payload. No test, document
or entry of podnet has such a run. podrunner refuses a `trusted` job at the hold level `none`, so without a
measure a host with no root runs no job.

## Premise

- Read: `--default-action reject` and `--flow-log FILE` exist (T-157); `-x http://unix:/PATH` takes a
  unix proxy (`crates/podnet-netns/README.md`).
- Not measured: the .NET runner, Node `fetch`, `actions/checkout` and `actions/setup-node` in a pod.

## Approach

1. Under `podnet pod --backend seccomp --default-action reject -x http://unix:/PATH --flow-log FILE`, run:
   `config.sh --help` and `run.sh` of the runner archive (a release asset of `actions/runner`) to its first
   long poll against a fake on loopback; a Node `fetch`; `actions/checkout` and `actions/setup-node` run by
   `node` directly.
2. For each flow of each payload: find it in the flow log, or name the way that it left (a unix socket,
   loopback TCP, an inherited descriptor).
3. Write the table and the log in `docs/reports/`.

## Prove

```sh
sh scripts/pod-runner-measure.sh   # the measure of step 1; it writes docs/reports/pod-runner.md
```

## Correction

2026-11-11, the measure differs from the Approach in two parts. `run.sh` does not reach a long poll: a
runner with no configuration stops at once, and a fake broker on loopback proves nothing for the hold
(the seccomp backend sends loopback direct, T-231). `config.sh --unattended` with a token that is not
valid makes the dials of the .NET client of the runner to the service instead. The witnesses are
three: the flow log, a proxy on a Unix socket that logs each CONNECT (`scripts/pod-runner-proxy.py`),
and strace.

2026-11-11: the owner ruled DR-1 and DR-2 yes (PodUtils/podcrates#55, COORD-030; podcrates call C49). The
entry has no wait now. A lead session of PodUtils/podrunner runs: it writes to podnet by a COORD issue of
PodUtils/podrunner `(to podnet)`.

## Done

2026-11-11, commit bb9dd91. `scripts/pod-runner-measure.sh` runs six payloads, each in its own seccomp pod
with `--default-action reject` and an allow list of rules to the witness proxy: `config.sh --help`,
`config.sh --unattended` (a refused registration), `run.sh` with no configuration (the runner `v2.338.2`),
a Node `fetch`, `actions/checkout` `v4.2.2` and `actions/setup-node` `v4.4.0` run by `node`.
`scripts/pod-runner-report.py` writes `docs/reports/pod-runner.md`. Result: each payload passes; each
CONNECT of the witness proxy has a flow of the flow log with the verdict `allow` (13 flows), no flow is
refused, and no flow goes to a fake address. The ways out that the flow log does not show are local:
DNS to the resolver of the host (the pod answers it), netlink (the addresses of the host), the unix
socket of `nscd`, the diagnostic socket of .NET (a listener), and UDP connects with no packet (the
address choice of `getaddrinfo`). The measure found T-233 (fixed): .NET dialed the IPv4-mapped form of a
fake address.


# T-230: `pod doctor --json` states the hold level and what the seccomp backend does not hold

**Source:** PodUtils/podvm#228 (COORD-022 of podvm, 2026-11-10), ask 1, item 2; the podrunner design review
(PodUtils/podcrates#47), spec R2, section 1.2, items 1 to 3.
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`podnet pod doctor --json` does not say how strong the hold of the seccomp backend is. A caller (podrunner)
must know that the hold is `advisory`, and which ways out it does not hold.

## Premise

- Read: `crates/podnet-netns/src/doctor.rs` prints the rows and the JSON keys of `pod doctor`.
- The seccomp backend does not hold unix, vsock and netlink sockets, loopback TCP, or inherited
  descriptors (spec R2, section 1.2).

## Approach

1. Add a row and a JSON object `hold`: `level` (`advisory` for seccomp) and `not_held` (a list of names).
2. A test pins the keys.

## Prove

```sh
cargo test -p podnet-netns doctor_hold_row_names_the_level
```

## Correction

2026-11-11: the owner ruled DR-1 and DR-2 yes (PodUtils/podcrates#55, COORD-030; podcrates call C49). The
entry has no wait now. A lead session of PodUtils/podrunner runs: it writes to podnet by a COORD issue of
PodUtils/podrunner `(to podnet)`.

## Done

2026-11-11, commit ae5d1e3. `crates/podnet-netns/src/doctor.rs`: `pod doctor` prints the rows `hold netns` and
`hold seccomp`; `--json` has the object `hold` with `netns` and `seccomp`, each with `level` (`enforced`,
`advisory` or `none`) and `not_held` (a list of names). The level is `none` when the backend cannot run, and
for seccomp also when the new row `read program memory` fails. The test
`doctor_hold_row_names_the_level` pins the keys and the levels. `crates/podnet-netns/README.md` describes
the levels. One object for each backend, not one `level`: the backend of a run is chosen at its start
(`--backend`, `auto`), and podrunner reads the object of the backend that it gives.

# T-231: `podnet pod` refuses a run with no hold, and can apply the rules to loopback

**Source:** PodUtils/podvm#228 (COORD-022 of podvm, 2026-11-10), ask 1, item 3 (wanted, not blocking); the podrunner design review
(PodUtils/podcrates#47), A-V-02 of PodUtils/podcrates#35.
**Category:** feature
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** partial

## Problem

When the supervisor cannot write the memory of the program, no hold mechanism works, but the pod runs.
Loopback TCP goes direct under `seccomp` (`seccomp_backend_sends_loopback_direct`), and no option applies
the rules to `127.0.0.0/8`.

## Premise

- Read: `tests/pod_seccomp/tcp.rs`, `seccomp_backend_sends_loopback_direct`.
- Not measured: a host where `process_vm_writev` to the child fails.

## Approach

1. Probe the write to the memory of the program at the start; refuse the run with a clear error when it
   fails.
2. An opt-in flag applies the rules to `127.0.0.0/8` under `seccomp`.

## Prove

```sh
cargo test --test pod_seccomp -- seccomp_rules_apply_to_loopback_when_asked
```

## Correction

2026-11-11: the owner ruled DR-1 and DR-2 yes (PodUtils/podcrates#55, COORD-030; podcrates call C49). The
entry has no wait now. A lead session of PodUtils/podrunner runs: it writes to podnet by a COORD issue of
PodUtils/podrunner `(to podnet)`.

2026-11-11: item 2 (the rules on loopback) changes the dial of a held `connect` in
`crates/podnet-netns/src/connect.rs`. That file is frozen: podcrates imports its dial state into `pod-hold`
(podcrates T-169, PodUtils/podcrates#55). Item 2 waits for the take of that release; then the choice of
the loopback route is podnet's part of the trait of the consumer. Item 1 is done.

Done so far: item 1 (ae5d1e3). The memory read is the condition, not the write: the backend reads the
address of each `connect`, and a failed read refuses the `connect` (`ECONNREFUSED`, `seccomp/mod.rs`), so
the program ran with no flow. Now `run_seccomp` stops at its start with exit status 3 and one line
(`seccomp_can_hold`); the row `read program memory` of `pod doctor` shows the probe. Test:
`seccomp_refuses_a_run_with_no_read_of_the_memory`.

# T-233: seccomp: a dual-stack connect to the IPv4-mapped form of a fake address goes to the proxy as an address

**Source:** T-229 (the measure with the runner of `actions/runner` v2.338.2, 2026-11-11).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Under `podnet pod --backend seccomp -x http://unix:PATH`, `config.sh` of the runner (.NET) first dials
`[::ffff:198.18.0.0]:443`: a dual-stack `AF_INET6` socket and the IPv4-mapped form of the fake address of
`api.github.com`. The flow log and the proxy see the address, not the name. The proxy cannot reach
`198.18.0.0`, so that dial fails, and .NET tries again with an `AF_INET` socket.

## Premise

- Measured: the flow log of the trial has `[::ffff:198.18.0.0]:443` (verdict `allow`), then
  `api.github.com:443`, for each of the three tries of the registration; the witness proxy
  (`scripts/pod-runner-proxy.py`) has `CONNECT [::ffff:198.18.0.0]:443`.
- Read: `crates/podnet-netns/src/fakedns.rs`, `unfake_v6` maps only the prefix `FAKE_IP6_PREFIX` back;
  `name_for_ip` and `name_for_ip_synced` use it (callers: `connect.rs:171`, `udp.rs:331`).
- `.NET` and Java open dual-stack sockets by default.

## Approach

1. `unfake_v6` also maps `::ffff:A.B.C.D` when `A.B.C.D` is in the pool (`198.18.0.0/15`).
2. A unit test of the mapped form, and of a mapped address out of the pool.

## Prove

```sh
cargo test -p podnet-netns --lib a_v4_mapped_fake_address_maps_back
sh scripts/pod-runner-measure.sh   # no flow of the runner has a target in 198.18.0.0/15
```

## Done

2026-11-11, commit 17da8c3. `unfake_v6` maps `::ffff:A.B.C.D` back when `A.B.C.D` is in the pool. Test:
`cargo test -p podnet-netns --lib a_v4_mapped_fake_address_maps_back` (it failed before the fix: `left:
None`). The measure of T-229 (`docs/reports/pod-runner.md`): `config.sh` of the runner makes 3 flows to
`api.github.com:443`, each by its name, and no flow to `198.18.0.0/15`.

