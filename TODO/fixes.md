This file holds the entries of milestone M5: the defects and the small gaps that the field report
of 0.3.0-beta.12 (`docs/reports/field-report-2026-11-05.md`), the lab kit, the section "Open
work" of `docs/maintenance.md` and the seccomp ideas of the vn kit found.
`docs/research/podnet/mwan-field.md`, section 4, ranks them. The lab kit's reports are
attachments, not files of the repository: `docs/research/podnet/mwan-field.md`, section 2.3,
quotes them. These entries can start at any time from M0. An entry that changes a file of a split
of M2 lands before that split starts, or after it ends. Names: before T-016 a variable is
`CFRS_X`; after it, `PODNET_X`. A fix in a module that podcrates already holds, or that
T-137 plans to move (`src/warp/`, `src/resolve.rs`, `src/egress.rs`, `src/cloudflare/`), lands in
podnet until podnet takes the crate. Tell the podcrates session of the fix (`docs/decisions.md`, Who edits a
shared crate). After the take, the fix goes to PodUtils/podcrates.

# T-063: seccomp: a TCP socket stays tracked after its connect, and its options and addresses come from its entry

**Source:** `docs/reports/field-report-2026-11-05.md`, item 2;
`docs/research/podnet/mwan-field.md`, section 3 (F2, W-F2, rank 1);
`docs/research/podnet/vn.md`, section 3.1 (P3, part a).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

Under the seccomp backend, the default of `warp exec`, `radicle-node` logs
`TCP_NODELAY: Not supported (os error 95)`. Then each peer resets the connection during the
handshake. With `--backend netns`, the same program works. The reporter lost about one hour.

## Premise

- Read: after a TCP connect succeeds, `handle_connect` removes the descriptor from the table of
  the supervisor (`crates/podnet-netns/src/seccomp/mod.rs`). Then `setsockopt` goes to the kernel
  AF_UNIX socket and gives EOPNOTSUPP for `TCP_NODELAY` and `TCP_KEEPIDLE`. `getsockname` and
  `getpeername` go to the kernel and give the AF_UNIX address.
- The evidence log shows `ss_family: 1` for the local and the peer address
  (`docs/reports/field-report-2026-11-05-evidence.log`).
- Read: `getpeername` of a tracked stream gives the local bound address, not the peer.
- The only test of the addresses uses a UDP socket (`tests/pod_seccomp/main.rs`).
  `crates/podnet-netns/README.md` says that each socket shows its logical address.
- Not proven: that the AF_UNIX addresses, not the options, cause the resets. A run of
  radicle-node decides it.

## Approach

1. Keep an entry for a connected TCP socket: its state, its peer (the address, or the fake
   address of a name), a plausible local address, and each option that the program set.
2. Answer `getpeername` with the peer and `getsockname` with the local address. Answer
   `setsockopt` of the IP and TCP levels with 0, and `getsockopt` with the value that the program
   set.
3. Make `getpeername` of a tracked stream give its peer.
4. Change `crates/podnet-netns/README.md` and the words of `pod doctor` about these calls.

## Prove

```sh
cargo test --test pod_seccomp -- seccomp_tcp_socket_keeps_its_options_and_addresses
```

The new C test connects through the SOCKS5 stand-in of the test. It asserts that
`setsockopt(TCP_NODELAY)` gives 0, `getsockopt(TCP_NODELAY)` gives 1, `getpeername` gives the
AF_INET target and port, and `getsockname` gives an AF_INET address. Plant: remove the entry
after the connect again. The test must fail. Then run radicle-node once under `warp exec`, and
write the result in the Done section.

## Done

2026-11-06, commit 7614ddb (`T-063: seccomp: a TCP socket keeps its entry after its connect`). A
connected TCP socket is the entry `Sock::Connected` (`crates/podnet-netns/src/sockopt.rs`) until
the program closes it: its peer, a local address and its options. `setsockopt` keeps the value;
`getsockopt` gives it. `getpeername` of a stream with no connect gives `ENOTCONN`. A second
`connect` gives `EISCONN`, a `bind` after the connect `EINVAL`. The test is in a new file,
`tests/pod_seccomp_tcp.rs` (`tests/pod_seccomp/main.rs` is in the size allowlist): it passes. Plant: no
entry after the connect; the test fails with the values of the field report (`setsockopt` -1,
`ss_family` 1). The words of `pod doctor` about these calls stay true (they name the case with no
write to the memory of the program). The run of radicle-node under `warp exec` is not done: this
host has no radicle-node and no WARP. `seccomp.rs`: 1,900 to 1,803 lines.

# T-064: seccomp: one entry for each socket of each process, and copies of a descriptor are tracked

**Source:** `docs/research/podnet/mwan-field.md`, section 3 (F2, W-F2);
`docs/research/podnet/vn.md`, section 3.1 (P3, part b).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

The table of the supervisor uses the descriptor number as its key, and each process of the pod
shares it. A `close` of descriptor N in one process drops the entry of N for each process. A copy
made by `dup`, `dup2` or `fcntl(F_DUPFD)` has no entry, so it sends to the wrong place.

## Premise

- Read: the descriptor table and the `close` branch in `crates/podnet-netns/src/seccomp/mod.rs`.
- `docs/research/podnet/vn.md`, section 3.1: the C supervisor of vn counts references and misses
  `F_DUPFD`. The Rust `try_clone` uses `F_DUPFD_CLOEXEC`.

## Approach

1. Use the pair (thread group id, descriptor) as the key. Copy the entries of the parent at a
   fork.
2. Intercept `dup`, `dup2`, `dup3`, and `fcntl` with `F_DUPFD` and `F_DUPFD_CLOEXEC`. Copy the
   entry to the new descriptor.
3. Compare this with a key of the socket inode (from `/proc/PID/fd/N`). Write the choice and the
   reason in the Decision section.

## Decision

The key is the socket inode of the program end (design b), not the pair (thread group id,
descriptor) (design a). The reasons:

- A `fork`, each `dup`, `dup2`, `dup3`, `fcntl(F_DUPFD)`, `fcntl(F_DUPFD_CLOEXEC)` and a
  descriptor sent with `SCM_RIGHTS` give the same socket, thus the same key. The filter stops no
  new system call (`intercepted_syscalls` does not change).
- Design a must copy the entries at each fork. The supervisor gets no notification at the
  return of `fork` or `clone`, so it does not know the pid of the child. An entry holds a
  socketpair and relay tasks, so a copy must be a shared reference with a count. A copy by
  `SCM_RIGHTS` stays without an entry.
- A thread shares the descriptor table of its process: `/proc/TID/fd/N` gives the same link as
  `/proc/TGID/fd/N`. No thread group id is necessary for the lookup.

The costs and the limits:

- Each stopped call on a descriptor does one `readlink` of `/proc/PID/fd/N`
  (`crates/podnet-netns/src/fdtable.rs`). The supervisor reads `/proc/PID/mem` for each call
  already, and that access needs more permission than the `readlink`.
- A `close` of a socket of the table reads the descriptor table of the caller first, then the
  tables of the other processes of the pod (the descendants of the first process, and each
  process that made a stopped call). It removes the entry only when no other descriptor refers
  to the socket.
- A descriptor that closes with no `close` call (the exit of a process, `execve` with
  close-on-exec, `close_range`) keeps its entry until a sweep. The sweep reads the descriptor
  tables of the pod and removes each entry of a socket that no process holds. It runs when the
  table doubles (from 64 entries), and before the table refuses a socket at `MAX_TRACKED_FDS`.
  Thus the table does not grow for the life of the pod. Until the sweep, the UDP association of
  such a socket stays open.
- `listen` puts a new socket at the number of the program (`SECCOMP_ADDFD` with
  `SECCOMP_ADDFD_FLAG_SETFD`). The entry moves to the key of the new socket. A copy of the
  descriptor made before the `listen` keeps the old socket and has no entry.
- An orphan process that made no stopped call is not in the list of the pod. A sweep or a
  `close` does not see its descriptors.

## Prove

```sh
cargo test --test pod_seccomp_fds -- seccomp_close_in_a_child_keeps_the_parent_entry seccomp_dup_keeps_the_udp_destination seccomp_closed_sockets_free_their_entries
```

## Start condition

T-063 is done.

## Done

2026-11-06, commit 7c3d1d8 (`T-064: seccomp: key the socket table by the socket inode`). The key
of the table is the inode of the socket of the program end (`crates/podnet-netns/src/fdtable.rs`).
The tests are in a new file, `tests/pod_seccomp_fds.rs` (`tests/pod_seccomp/main.rs` is in the size
allowlist). `seccomp_close_in_a_child_keeps_the_parent_entry`: the parent connects through the
proxy after a child closes its copy; it passes. `seccomp_dup_keeps_the_udp_destination`: a copy by
`dup`, `dup2`, `dup3`, `F_DUPFD` and `F_DUPFD_CLOEXEC` keeps the destination of a connected UDP
socket after the close of the first descriptor; it passes for each. Control on the old code: the
first test fails (`connect: Invalid argument`), the second fails (`send failed` for each copy).
`seccomp_closed_sockets_free_their_entries`: 5000 sockets closed with `close_range`; it passes.
Control with no sweep: it fails at socket 4096 (`EMFILE`).

# T-065: seccomp: hand a plain upstream descriptor to the program after its handshake

**Source:** `docs/research/podnet/vn.md`, sections 3.1 (P3, part c) and 5 (question 2).
**Category:** feature
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

For a plain upstream (a Unix socket, or TCP to an HTTP or SOCKS5 proxy, with no TLS and no
framing in the process), the supervisor relays each byte after the handshake. The program holds
an AF_UNIX socket pair, not the upstream socket.

## Premise

- The kernel can install a descriptor at a number of the target process
  (`SECCOMP_IOCTL_NOTIF_ADDFD` with `SECCOMP_ADDFD_FLAG_SETFD`). The seccomp backend uses ADDFD
  already (`crates/podnet-netns/src/seccomp/notif.rs`).
- After a handoff, the supervisor sees no byte of that flow: no byte count, no idle limit, no
  log line of the flow.

## Approach

1. After the handshake with a Unix-socket upstream, install the upstream descriptor at the
   program's descriptor number. Drop the relay task.
2. Keep the relay for TCP upstreams. Ask the owner before the handoff becomes the default for
   them, because of the lost byte counts.

## Prove

```sh
cargo test --test pod_seccomp_handoff -- seccomp_handoff_of_a_unix_upstream_leaves_no_relay
```

The test asserts that `setsockopt(TCP_NODELAY)` on the handed descriptor gives 0 from the kernel,
and that no relay task stays.

## Start condition

T-064 is done.

## Correction

2026-11-06:

- The test is in `tests/pod_seccomp_handoff.rs`, not in `tests/pod_seccomp/main.rs`.
  `tests/pod_seccomp/main.rs` is in `scripts/size-allow.txt` and cannot get larger. The Prove names
  the new test target.
- `setsockopt(TCP_NODELAY)` on the handed descriptor cannot give 0 from the kernel. The handed
  socket is the `AF_UNIX` socket to the proxy, and the kernel gives `EOPNOTSUPP` for an
  `IPPROTO_TCP` option on it. The entry of the socket stays under the inode of the handed
  socket (`Sock::Connected`, T-063), so the supervisor gives 0. The test asserts 0 from the pod.
- The handoff occurs for one SOCKS5 or HTTP hop on a Unix socket. A chain of hops, a loopback
  connect, a tailnet dial and a TCP upstream keep the relay. The handoff also needs one waiting
  call, no other descriptor of the socket in the pod, and no unread byte of the program; else the
  relay starts (`crates/podnet-netns/src/handoff.rs`).
- The test proves "no relay stays" with the exit time of the pod: the echo proxy keeps its end
  open for 10 s after the program closes, so a relay keeps the pod for its drain limit (3 s).
  The test asserts an exit in less than 2.5 s, and the new socket inode at the descriptor.

## Done

2026-11-06, commit 075b83a. `cargo test --test pod_seccomp_handoff`: 2 passed
(`seccomp_handoff_of_a_unix_upstream_leaves_no_relay`, `seccomp_tcp_upstream_keeps_the_relay`).
The control on the code before the change: the handoff test failed (`same_socket 1`), the TCP
test passed. `cargo test -p podnet-netns handoff`: 2 passed. The tests `pod_seccomp_tcp`,
`pod_seccomp_slow_dial`, `pod_seccomp_fds` and `pod_seccomp_dgram` pass.
`sh scripts/agent/check.sh` exits 0.

Live run 42275332991 at 68098d1: success. CI run 42272728934 at 68098d1 failed in the jobs
`restricted-host` and `no-loopback` (the TCP test had no `need_loopback!()`); dfded5b adds it. CI run
42278745845 at dfded5b: success.

# T-066: seccomp: ICMP datagram sockets, `recvmmsg`, and UDP names over 255 bytes

**Source:** `docs/research/podnet/vn.md`, section 3.1 (P4); `docs/maintenance.md`, section "Open
work" (the bullet of the pod seccomp backend).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Three calls get a wrong result under the seccomp backend:

1. `socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP)` is relayed as UDP.
2. A `recvmmsg` read gets the AF_UNIX source address, not the real one.
3. A UDP target name over 255 bytes is cut.

## Premise

- Read: `handle_socket` does not read the protocol argument (`crates/podnet-netns/src/socket.rs`).
- `recvmmsg` is not in the list of intercepted calls. The ruling of 2026-11-04 names `recvfrom`
  and `recvmsg` (`docs/decisions.md`, pod seccomp UDP).
- `wrap_udp_name` cuts a name over 255 bytes (`crates/podnet-netns/src/upstream.rs`).

## Approach

1. Give ICMP datagram sockets their own kind. The supervisor answers an echo to the gateway of
   the pod. Another destination gets `EACCES`.
2. Intercept `recvmmsg` as a series of `recvmsg` reads, and write the real source of each
   message.
3. Refuse a UDP name over 255 bytes with `EINVAL` and one log line.

## Decision

The owner, 2026-11-04 (`docs/decisions.md`, pod seccomp UDP, #285): the seccomp backend
intercepts `recvfrom` and `recvmsg` on its sockets, so that a UDP reader sees the real source
address. `recvmmsg` is the same need.

## Prove

```sh
cargo test --test pod_seccomp -- seccomp_icmp_echo_to_the_gateway seccomp_recvmmsg_gets_the_real_source seccomp_long_udp_name_is_refused
```

The ICMP test follows the ICMP part of the vn kit's static guest: an echo to the gateway gets an
answer; an echo to 192.0.2.1 gets `EACCES`.

## Correction

2026-11-06. The three tests are in a new file, `tests/pod_seccomp_dgram.rs`: `tests/pod_seccomp/main.rs`
is in the size allowlist with no free lines. Run them with
`cargo test --test pod_seccomp_dgram`. The seccomp pod has no address of its own; its gateway is
`10.66.255.254`, the gateway of the netns backend (`TUN_GW`). An IPv6 ICMP socket gets its own
kind too, but each echo gets `EACCES`: the pod has no IPv6 gateway. `recvmmsg` waits for the first
message only (as with `MSG_WAITFORONE`); a blocking call of the kernel with no `MSG_WAITFORONE`
waits for `vlen` messages.

## Done

2026-11-06, commit f50da2e (`T-066: seccomp: ICMP datagram sockets, recvmmsg, and UDP names over
255 bytes`). An ICMP datagram socket is the entry `Sock::Icmp` (`crates/podnet-netns/src/icmp.rs`):
an echo to `10.66.255.254` gets an echo reply from that address, another destination gets
`EACCES`. The filter stops `recvmmsg`; the supervisor writes the real source and `msg_len` of each
message (`crates/podnet-netns/src/recvmmsg.rs`). A UDP name over 255 bytes gets `EINVAL` and one
log line (`crates/podnet-netns/src/dgram.rs`). `cargo test --test pod_seccomp_dgram`: 3 passed
(`seccomp_icmp_echo_to_the_gateway`, `seccomp_recvmmsg_gets_the_real_source`,
`seccomp_long_udp_name_is_refused`). Control on the code before the fix: 0 passed, 3 failed (no
echo answer; source family 0 and `msg_namelen` 0; the long name was sent).
`sh scripts/agent/check.sh`: exit 0.

# T-067: seccomp: a slow `connect` dial does not stop the supervisor for the other calls

**Source:** `docs/maintenance.md`, section "Open work" (the bullet of the pod seccomp backend).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

A blocking `connect` dial in the supervisor stops each other call of the program until the dial
ends. A program with a slow upstream freezes in calls that need no network.

## Premise

- Read: the notification loop dials the upstream before it answers the next notification
  (`supervise_seccomp` and `handle_connect` in `crates/podnet-netns/src/seccomp/mod.rs` and `crates/podnet-netns/src/connect.rs`).
- A notification id stays valid until the supervisor answers it. `SECCOMP_IOCTL_NOTIF_ID_VALID`
  tells whether the task still waits.

## Approach

1. Run each upstream dial in its own task. Answer its notification when the dial ends.
2. Serve the other notifications while the dial runs.
3. Check the id before the answer. Drop the dial when the task is gone.

## Prove

```sh
cargo test --test pod_seccomp -- seccomp_a_slow_dial_does_not_stop_other_calls
```

An upstream stand-in answers after 2 s. A second thread of the program calls `getsockname`, and
it returns in less than 100 ms.

## Correction

- 2026-11-06: `tests/pod_seccomp/main.rs` is at its size limit (`scripts/size-allow.txt`). The test is in
  `tests/pod_seccomp_slow_dial.rs`. The command is
  `cargo test --test pod_seccomp_slow_dial -- seccomp_a_slow_dial_does_not_stop_other_calls`.
- 2026-11-06: the code has no non-blocking connect (`EINPROGRESS`). A `connect` of a socket with
  `O_NONBLOCK` also waits for the dial, before and after this change. The socket of the program is
  one end of an `AF_UNIX` socket pair, which is writable before the dial ends; thus `EINPROGRESS`
  and a later `SO_ERROR` cannot tell the program the result of the dial.

## Done

2026-11-06, commit 39ab845 (`T-067: seccomp: a slow connect dial does not stop the supervisor`).
The dial of a TCP `connect` runs in its own task (`crates/podnet-netns/src/connect.rs`). The
notification loop serves the other calls during the dial. At the end of the dial, the loop changes
the entry of the socket and answers each waiting call that `SECCOMP_IOCTL_NOTIF_ID_VALID` finds
pending; it drops the connection when no call waits. A second `connect` of the socket during the
dial waits for the same dial; a `close` during the dial gives `ECONNABORTED`.
`cargo test --test pod_seccomp_slow_dial`: 2 passed (`getsockname` during a 2 s dial returns in
less than 100 ms). Control on the code before the fix: 2 failed (`getsockname` took 1701 ms; the
`connect` gave 0 after the `close`). `sh scripts/agent/check.sh`: exit 0.

# T-068: WARP: a probe of the TCP data path renews a session that carries no TCP

**Source:** the lab kit's report masque-session-tcp-blackhole.md, in
`docs/research/podnet/mwan-field.md`, section 2.3 (B1, rank 2).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

A MASQUE session can carry ICMP and UDP but no TCP. podnet keeps it as alive, and each TCP flow
fails until a restart. The edge causes the state; a new session ends it.

## Premise

- Read: the liveness check of HTTP/2 sends a PING only when no data came, and calls the session
  dead after three intervals with no pong and no data (`MAX_MISSED_PONGS` in
  `src/warp/session.rs`). ICMP and UDP replies count as data.
- The `Live` watcher can renew a session before the old one closes (`renew_due` in
  `src/warp/live.rs`).
- The report shows that the same host and binary work with another device at the same time.

## Approach

1. Run one probe task for each live WARP stack. Each 30 s with no TCP connect that succeeded,
   connect to 1.1.1.1:443 through the stack, with a limit of 5 s.
2. After three misses, mark the session `tcp-dead` and renew it through the path of
   `renew_due`. The old flows drain. Log `reason=tcp-probe`.
3. Add `--probe HOST:PORT` and `--probe off`.
4. Show `data_path` (`ok` or `tcp-dead`) and the last probe time in `status`, the ready document
   and `warp status`.

## Prove

```sh
cargo test --workspace -- warp_probe_renews_a_session_with_no_tcp
```

The test uses the "silenced" edge of T-012, or a fake session that drops each TCP SYN. The probe
renews the session after three misses.

## Done

2026-11-06, commit 86050ee (`T-068: WARP: a TCP probe of the data path renews a session that
carries no TCP`). Steps 1 to 3: `Live::probed` in `src/warp/live.rs` (each 30 s, 5 s limit, 3
misses; then the path of `renew_due`, the log names `reason=tcp-probe`); `--probe IP:PORT` and
`--probe off` (a name is refused: the probe needs no DNS); `warp up` gives it to its child. The
probe runs each 30 s also when other TCP flows succeed: one connect each 30 s. The test
`warp_probe_renews_a_session_with_no_tcp` uses a fake session whose probe fails: it passes. Plant:
no probe task; the test fails. Step 4 (`data_path` in `status`, the ready document and `warp
status`) is T-140: `Live::data_path` gives the value.

# T-069: `warp tun`: a SYN probe of the session, and `--session-max-age`

**Source:** the lab kit's report masque-session-tcp-blackhole.md, in
`docs/research/podnet/mwan-field.md`, section 2.3 (B1, fixes 2 and 3).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** L
**Status:** done

## Problem

`warp tun` reconnects only when its run loop returns. It has no probe of the data path, so the
state of T-068 stays for ever. `--session-max-age` is refused for `warp tun`.

## Premise

- Read: the keep loop of `warp tun` opens a new session only when `tun::run` returns
  (`src/warp/tun.rs`, `src/main.rs`).
- Read: `src/main.rs` refuses `--session-max-age` with `warp tun`.
- The TUN carries the packets of the host, so the probe of T-068 cannot open a stack socket here.

## Approach

1. Put a TCP SYN from the tunnel address and a reserved port to the probe target into the
   session.
2. Take the SYN-ACK or the RST out of the inbound path before the TUN (match the reserved port).
   Answer with a RST.
3. After three misses, return an error from `tun::run`. The keep loop opens a new session.
4. Allow `--session-max-age` on `warp tun`: a timer around `tun::run`.

## Prove

```sh
cargo test --workspace -- warp_tun_probe_takes_its_answer_out warp_tun_probe_ends_a_dead_session
cargo test --test cli -- warp_tun_takes_session_max_age
```

The first test asserts that a SYN-ACK to the probe port is not written to the TUN.

## Start condition

T-068 is done.

## Done

2026-11-06, commit 7382ff4 (`T-069: warp tun: a SYN probe of the session, and
--session-max-age`). `src/warp/tun/probe.rs`: the SYN from the tunnel address and port 61999 to
the target of `--probe` (an IPv4 target; an IPv6 target gives no probe), the filter of its answer,
and the RST for a SYN-ACK. `tun::run` sends the SYN each 30 s, takes the answer out before the TUN,
and ends after 3 probes with no answer; it also ends at `--session-max-age`. A TUN of `from_fd`
(the raw pod) has no address and no probe. `warp_tun_probe_takes_its_answer_out` and
`warp_tun_probe_ends_a_dead_session` test the probe; `warp_tun_takes_session_max_age` is in
`tests/commands.rs` (`tests/cli/main.rs` cannot grow). Control: before, `warp tun --session-max-age 60`
exits 64. No live run: this host has no TUN device for a test.

# T-070: `warp exec`, `warp shell` and `warp nc` find a running `warp up` with another state directory

**Source:** `docs/reports/field-report-2026-11-05.md`, items 3, 5 and 6;
`docs/research/podnet/mwan-field.md`, section 3 (F3, F5, F6, W-F3, rank 3).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`warp exec` failed with "warp up is not running (no answer on /state/kage/cfrs/run/warp.sock)".
The reporter had started `warp up` with another state directory. `warp exec` found the socket
only with both the state directory variable and the WARP socket variable set.

## Premise

- Read: `exec_in_pod`, `shell`, the `nc` path and the agent use `socket_path()`, which is the
  run directory and `warp.sock` (`src/warp/up.rs`, `src/warp/nc.rs`, `src/agent.rs`).
- Read: `run_dir()` depends on the state directory (`crates/podnet-core/src/instances.rs`).
- Read: `warp up` writes `PODNET_WARP_SOCKET` to `warp.env` and to the program of the pod, but
  nothing reads it (`src/warp/up.rs`).
- `warp.env` is in the same directory, so to read the env file alone does not help.

## Approach

1. Read the WARP socket variable first.
2. Add `--socket PATH` to `warp exec`, `warp shell` and `warp nc`.
3. Make `warp up` write a pointer file at a fixed place of the user (the runtime directory, else
   a directory of the user in the temporary directory). It has mode 0600 and an owner check. It
   holds the socket and the state directory.
4. Make the error list each path tried, and name the state directory of a running `warp up` when
   it finds one.
5. Add the variable to `src/manual/facts.rs`. In the manual, name the state directory variable
   for a host whose default state directory is on a shared disk.

## Prove

```sh
cargo test --workspace -- warp_exec_finds_warp_up_of_another_state_directory the_lookup_order_of_the_warp_socket
```

The first test starts `warp up` with a state directory A (with the stand-in of T-012), then runs
`warp exec -- true` with no state directory variable. It exits 0.

## Done

2026-11-06, commit 0293f91 (`T-070: warp exec, warp shell and warp nc find a running warp up with
another state directory`). `src/warp/find.rs`: the order `--socket`, `PODNET_WARP_SOCKET`, the run
directory, the pointer file (`$XDG_RUNTIME_DIR/podnet/warp-up.json`, else
`podnet-<uid>/warp-up.json` in the temporary directory; mode 0600, owner check). `warp up` writes it
with the env file; `warp down` removes it. The error names each socket tried and the state
directory of the pointer file. The agent uses the same lookup. The stand-in of T-012 is not there,
so the test of the binary is `warp_nc_finds_warp_up_of_another_state_directory`
(`tests/commands.rs`): a SOCKS5 stand-in on a unix socket that only the pointer file names; `warp
nc -N` reaches a loopback echo through it. `warp_exec_finds_warp_up_of_another_state_directory` and
`the_lookup_order_of_the_warp_socket` are unit tests of `warp::find`. Control: without the pointer in
the candidates, these three tests fail. The variable and the pointer file are in
`src/manual/facts.rs`. A test of `warp exec` with a real `warp up` waits for the stand-in (T-012).

# T-071: `warp exec` and `warp shell` take `--backend`, name the backend in one line, and log the seccomp emulation

**Source:** `docs/reports/field-report-2026-11-05.md`, items 2 and 8 (request 2);
`docs/research/podnet/mwan-field.md`, section 3 (F2b, W-F2b, rank 4); the owner's ruling of
2026-11-05 (`docs/decisions.md`, Pod backend).
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`warp exec` gave the reporter the seccomp backend with no word about its limits. `doctor` names
them, but nobody reads it before a failure. A reset from inside the pod looks like a refusal of
the peer.

## Premise

- Read: `warp exec` always passes `--prefer-udp` (`src/warp/up.rs`), so `auto` takes seccomp
  first where netns works (`crates/podnet-netns/src/lib.rs`).
- `warp exec` and `warp shell` have no `--backend`.
- The words about the seccomp limits are in `crates/podnet-netns/src/doctor.rs`.

## Approach

1. Add `--backend auto|netns|seccomp` to `warp exec` and `warp shell`. Pass it to the pod.
2. Print one line on stderr that names the backend and its limits, for example "seccomp: socket
   options and addresses are emulated; use --backend netns". `-q` and the quiet variable remove
   it.
3. With `-v`, log each emulation of the seccomp layer at the info level ("socket emulated as a
   socket pair", "setsockopt answered").
4. Keep the order of `auto` until T-113.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Pod backend): `warp exec` and `warp shell` get
`--backend` and a line that names the backend.

## Prove

```sh
cargo test --workspace -- warp_exec_names_its_backend_once warp_exec_passes_the_backend_to_the_pod seccomp_logs_its_emulation_at_info
```

## Done

2026-11-06, commit ee9b44c (`T-071: warp exec and warp shell take --backend and name the
backend`). `warp exec` and `warp shell` take `--backend auto|netns|seccomp` (default `auto`) and
give it to the pod (`src/warp/pod.rs`). `warp nc` does not take it; `--socket` stays. The new pod
option `--name-backend` writes one line on stderr that names the backend that runs (also after
`auto`) and its limits; `podnet -q` (`PODNET_NETNS_QUIET`) removes it. `podnet -v warp exec`
gives `-v` to the pod. With `-v`, the seccomp backend logs each socket that it makes as a socket
pair (`crates/podnet-netns/src/socket.rs`) and each `setsockopt` that it answers
(`crates/podnet-netns/src/sockopt.rs`). The order of `auto` does not change (T-113). The three
tests of Prove pass (`tests/pod_backend_line.rs` and a unit test in `src/warp/pod.rs`). Eleven
plants (no line, two lines, no quiet, no `--name-backend`, no backend, no `-v`, each log removed
or given with no `-v`) each made a test fail.

# T-072: `warp tun` sets `rp_filter` to 0 on its interface

**Source:** the lab kit's report warp-tun-rp-filter-strict.md, in
`docs/research/podnet/mwan-field.md`, section 2.3 (B2, rank 5).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

On a host with systemd, each new TUN of `warp tun` gets `rp_filter` 2. When the only egress is a
loopback proxy and the WAN routes are in their own tables, the loose mode drops the replies.

## Premise

- podnet never writes `rp_filter`. `Tun::create` sets only the interface, the address, the mask,
  the MTU and the flags (`src/warp/tun.rs`).
- The likely source: systemd-udevd runs systemd-sysctl for each new interface, and its default
  file sets `rp_filter = 2`. Not measured here.
- The value that the kernel uses is the larger of `conf.all` and `conf.IF`.

## Approach

1. After `Tun::create`, write 0 to the `rp_filter` file of the interface under
   `/proc/sys/net/ipv4/conf/`.
2. Read it again after 2 s and at each new session, because udev can run after podnet.
3. Warn when the value that the kernel uses is not 0 (for example, `conf.all` is 1).
4. Add `--keep-rp-filter` to keep the value. Show the value in the ready document and in
   `doctor`.

## Prove

```sh
cargo test --workspace -- the_effective_rp_filter_is_the_larger_value
gh workflow run live.yml
```

The live step of `warp tun` reads 0 for the interface.

## Done

2026-11-06, commit fb151ca (`T-072: warp tun sets rp_filter to 0 on its interface`).
`src/warp/tun/rp_filter.rs`: 0 for the interface after the create, at each session (`tun::run`) and
2 s later; a warning when the larger of `conf.all` and `conf.IF` is not 0; `--keep-rp-filter`; the
ready document has `rp_filter`. `the_effective_rp_filter_is_the_larger_value` and
`warp_tun_writes_0_unless_it_keeps_the_value` (a fake `conf` tree) pass. Not done: the row in
`doctor` (`src/doctor.rs` is frozen until the take of `pod-services`), and the live step: this
session starts no workflow, and this host has no TUN device.

# T-073: `warp forward`: the pins of `--resolve`, and the list of forwards

**Source:** `docs/reports/field-report-2026-11-05.md`, items 4 and 8 (requests 4 and 5);
`docs/research/podnet/mwan-field.md`, section 3 (F4a, F4b, W-F4a, W-F4b, ranks 6 and 7).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

1. `--resolve` parses on `warp forward` and does nothing. A user cannot pin the address of a host
   with unreliable DNS.
2. The text of `ps` shows the first listen address of a forward only, and no target. The reporter
   kept notes of each mapping.

## Premise

- Read: `warp forward` resolves each name through the tunnel (`resolve_all` in
  `src/warp/proxy.rs`). The pins are read only in the host chain (`resolve_chain` in
  `src/resolve.rs`).
- Read: the ready document has `forward: [{listen, to}]`, so `ps --json` shows each forward
  (`src/main.rs`). The text rows print `forward` only when it is a string.

## Approach

1. Add a public function in `src/resolve.rs` that gives the pin of a name. Make `resolve_all` and
   `resolve` of `src/warp/proxy.rs` return the pin before any query. Then `--resolve` works for
   `forward`, `proxy`, `nc` and `exec`.
2. For an array `forward`, make `ps` print each `LISTEN -> TO`.
3. Add `warp forward --list`: the forwards of `ps`.

## Prove

```sh
cargo test --workspace -- warp_resolve_takes_the_pin_first ps_prints_each_forward
```

The first test asserts that a pinned name gives the pin and sends no DNS packet into the stack.

## Correction

2026-11-05, `src/resolve.rs` moves to `pod-dns` (T-137). Until podnet takes `pod-dns`, this fix of `src/resolve.rs`
lands in podnet and the podcrates session ports it. After the take, the fix goes to PodUtils/podcrates.

2026-11-06, podnet took `pod-dns` and `pod-services` (bf0c7ca, podcrates v0.5.0). The freeze of
`src/resolve.rs` ended. `src/resolve.rs` is now glue over the resolver of `pod-dns`, and the pins
are in its settings (`pod_dns::Settings::pinned`). The fix lands in podnet and needs no change of
`pod-dns`: `src/resolve.rs` gets `pinned(host)`, and the DNS of the WARP commands moves from
`src/warp/proxy.rs` to `src/warp/proxy/names.rs`. The text of `ps` moves from `src/main.rs` to
`crates/podnet-core/src/instances/text.rs`. The test of `--list` is `warp_forward_list_prints_the_forwards_of_ps`.

## Done

2026-11-06, commit bbfe527 (`warp: the pins of --resolve come first, ps prints each forward, warp
forward --list (T-073)`). `pinned(host)` in `src/resolve.rs` gives the pins of `--resolve` and
`PODNET_RESOLVE`. `resolve_all` and `resolve` (`src/warp/proxy/names.rs`) return the pins before
a DNS query through the tunnel: `warp forward`, `warp proxy` and `warp nc`. `ps` prints each
forward of an array `forward` as `LISTEN -> TO` (`crates/podnet-core/src/instances/text.rs`). `warp forward --list`
prints the forwards of the detached instances. Control on the code before the fix:
`warp_resolve_takes_the_pin_first` fails (6 DNS packets go into the stack),
`ps_prints_each_forward` fails (no row `LISTEN -> TO`),
`warp_forward_list_prints_the_forwards_of_ps` fails (exit 64, unexpected argument `--list`). After
the fix, the three pass, and `sh scripts/agent/check.sh` exits 0.

CI run 42253187093 and live run 42253174816 at 3bf268a: success.

# T-074: `warp export`: the reserved bytes in the file, `--check`, and a TCP probe on the WireGuard leg

**Source:** the lab kit's report warp-export-kernel-wg-data-path-degrades.md, in
`docs/research/podnet/mwan-field.md`, section 2.3 (B3, rank 9).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** L
**Status:** done

## Problem

An exported device in kernel WireGuard has fresh handshakes and a data path in one direction
only. The user cannot tell a device that needs the reserved bytes from a dead device. The
WireGuard leg of podnet checks only the handshakes, so it does not see the same state.

## Premise

- Read: `warp export` writes the reserved bytes only as a comment (`Reserved` in
  `src/warp/export.rs`), and as `reserved` in the JSON form. Kernel WireGuard and wg-quick send
  zeros there.
- Read: the WireGuard leg sends the `client_id` bytes of the device in bytes 1 to 3 of each
  message (`src/warp/wg.rs`).
- Read: `WgKeepalive` checks only the handshakes (`src/warp/wg.rs`).

## Approach

1. Write in the head of the export and in the manual: what the reserved bytes are, that kernel
   WireGuard and wg-quick send zeros, and that `warp tun --leg wireguard` sends them.
2. Add `warp export --check`: run the WireGuard leg with the exported device, once with the
   reserved bytes and once with zeros, with a TCP probe through each. Print "device ok", "needs
   reserved bytes" or "device data path dead, use --new".
3. Add the TCP probe of T-068 to `WgKeepalive` for `--leg wireguard`.

## Prove

```sh
cargo test --workspace -- a_seal_with_zero_reserved_differs_in_three_bytes wireguard_probe_renews_a_dead_leg
gh workflow run live.yml
```

A live step runs `warp export --check` on the device of the run.

## Start condition

T-068 is done.

## Correction

2026-11-06: step 3 needs no probe in `WgKeepalive`. The leg of `--transport wireguard` is a
`MasqueSession` under the `WarpStack` of `Live`, and `Live::probed` (T-068) probes each such
stack; `warp tun` on the leg has the SYN probe of T-069. Measured: the test
`wireguard_probe_renews_a_dead_leg` (a stand-in front door that answers each handshake and drops
each data message) passes on the code before this change (`f88aacf`), and fails with no probe
(`probed(None)`: "no renewal" after 10 s). A probe in `WgKeepalive` would be a second probe of the
same path. The test stays as the proof of step 3. The flag of the entry, `warp tun --leg
wireguard`, is `--transport wireguard` in podnet.

## Done

2026-11-06, commit 3e57e0a (`T-074: warp export: the reserved bytes, --check, and the probe of
the WireGuard leg`). Step 1: the head of the wg-quick output (`RESERVED_NOTE` in
`src/warp/export.rs`), the help of `warp export` and `README.md`. Step 2: `warp export --check`
(`src/warp/export/check.rs`): the leg with the reserved bytes and the leg with zeros, each with a
TCP connect to the target of `--probe` in 5 s; exit 0 (`device ok`), 1 (`needs reserved bytes`)
or 69 (`device data path dead, use --new`); `--out` and `--delete` are refused. Step 3: see the
Correction. `cargo test --workspace -- a_seal_with_zero_reserved_differs_in_three_bytes
wireguard_probe_renews_a_dead_leg` passes; `export_check_tells_the_three_states` (a stand-in front
door for each verdict) and `warp_export_check_is_a_flag` (`tests/commands.rs`) pass;
`sh scripts/agent/check.sh` exits 0 (1012 tests passed). Control before the change: the two unit
tests of `check.rs` do not compile, `warp_export_check_is_a_flag` fails (unexpected argument
`--check`). Not run: `gh workflow run live.yml` (no network to WARP here); the step is in
`scripts/live-batch2.sh`.

CI run 42253187093 and live run 42253174816 at 3bf268a: success (the live step of `warp export --check` ran).

# T-075: The datagram socket of the vnet shim takes the same user only

**Source:** `docs/maintenance.md`, section "Open work" (the bullet of the vnet shim UDP);
`docs/virtual-network.md`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The datagram socket of the shim takes datagrams from any user. The stream sockets of the shim and
the sockets of the switch take the same user only.

## Premise

- `docs/virtual-network.md` says that the datagram socket of the shim has no check of the user.
- The switch drops a datagram of another user with `SO_PASSCRED` (`crates/podnet-vnet/src/switch.rs`).

## Approach

1. Set `SO_PASSCRED` on the datagram socket of the shim.
2. Read the credentials of each datagram. Drop a datagram of another user, with one log line.
3. Change `docs/virtual-network.md`.

## Decision

The owner, 2026-11-04 (`docs/decisions.md`, vnet sockets, #288 and #289): the unix sockets of
the shim and the switch take the same user only.

## Prove

```sh
cargo test --workspace -- shim_datagram_of_another_user_is_dropped
```

The test runs as root and sends from a child with another uid. It is skipped, with one line, when
it does not run as root.

## Done

2026-11-06, commit 55e8570 (`T-075: the datagram socket of the vnet shim takes the same user
only`). The shim sets `SO_PASSCRED` on each datagram socket, and before `recvfrom` and `recvmsg` it
drops each datagram of another user (`MSG_PEEK` of `SCM_CREDENTIALS`), with one line on stderr.
`recv` and `read` are not interposed (written in `docs/virtual-network.md`). The test is
`tests/shim_datagram_user.rs`: as root, uid 65534 sends first, then root; the server gets the
datagram of root. Control: with the shim before, the server gets the datagram of uid 65534.

# T-076: A CI job runs the tests with the loopback down

**Source:** `docs/maintenance.md`, section "Open work" (#217).
**Category:** chore
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

No CI job runs the tests with the loopback down. A new test that binds 127.0.0.1 with no
`need_loopback!()` fails on such a host, and CI does not see it.

## Premise

- `unshare -n` needs user namespaces, which the ubuntu-24.04 runner restricts.
- `docs/development.md` has the local command.
- The job `restricted-host` refuses `bind(AF_INET)` (`scripts/restricted-host.sh`).

## Approach

1. Add a job to `.github/workflows/ci.yml` that makes the network namespace as root and runs the
   tests as the runner user:
   `sudo unshare -n -- setpriv --reuid=$(id -u) --regid=$(id -g) --init-groups cargo test --workspace`,
   with `HOME` and `PATH` kept.
2. When `setpriv` cannot keep the environment, lower the AppArmor restriction for the job
   (`kernel.apparmor_restrict_unprivileged_userns=0`) and use `unshare -rn`.

## Prove

```sh
gh workflow run ci.yml
```

The new job passes. Plant: remove `need_loopback!()` from one test. The job must fail.

## Done

2026-11-06, commit 2ddb0ed (`T-076: a CI job runs the tests with the loopback down`). The job
`no-loopback` of `ci.yml` builds the tests, then runs them with `sudo unshare -n` and `setpriv` as
the runner user, offline; its plant step removes `need_loopback!()` from one test and must see the
test fail. `the_kernel_refuses_a_real_af_inet_bind` now skips on `EADDRNOTAVAIL`. CI run 42183709673
(head 00ca0a5): the job `no-loopback` passed, with its plant step.

# T-077: An ingress `path:` rule matches the decoded path, as cloudflared does

**Source:** `docs/maintenance.md`, section "Open work" (`path:`, #289).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

An ingress `path:` rule matches the path as it came, with its `%XX` escapes. cloudflared matches
the decoded path. A rule of a cloudflared configuration then matches other requests here.

## Premise

- `crates/podnet-serve/src/ingress.rs` matches the rule with the `regex` crate.
- The owner, 2026-11-04 (`docs/decisions.md`, `path:`, #269 item 3): a `path:` is a regular
  expression, as in cloudflared. The match has no anchor, as in cloudflared.
- Go gives the decoded path in `URL.Path`. Not read in the source of cloudflared here.

## Approach

1. Read the matcher of cloudflared to confirm the decoded path. Write the result in the
   Correction section when it differs.
2. Decode `%XX` of the path before the match.
3. Keep the match with no anchor. Write both facts in the manual and in `README.md`.

## Prove

```sh
cargo test --workspace -- a_path_rule_matches_the_decoded_path
```

The test asserts that `/a%20b` matches the rule `^/a b$`.

## Done

2026-11-06, commit 7fe9a2c (`T-077: an ingress path: rule matches the decoded path, as cloudflared
does`). Step 1: `FindMatchingRule` of cloudflared (`ingress/ingress.go`) gives `requestURL.Path`,
the decoded path of Go: the premise holds. `PathPattern::is_match` decodes `%XX` (a `%` with no
two hex digits stays; `+` stays) and keeps the match with no anchor. `README.md` says both.
`a_path_rule_matches_the_decoded_path` passes; on the code before, `/a%20b` does not match.

# T-078: `pod doctor` uses the verdict words of `doctor` and has a `schema` key

**Source:** `docs/maintenance.md`, section "Open work" (`podnet pod doctor`); the owner's ruling of
2026-11-04 (`docs/decisions.md`, Doctor).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`pod doctor` keeps its own words (`ok`, `no`, `available`) and has no `schema` key. The other
doctors and `status` use one vocabulary and a schema.

## Premise

- `crates/podnet-netns/src/doctor.rs` writes the words of the pod.
- `src/doctor.rs` and `crates/podnet-core/src/report.rs` write the vocabulary of the other doctors.

## Approach

1. Map each probe of the pod to the verdict words of `doctor`.
2. Add the key `schema` and the version to the JSON form.
3. Keep exit 3 when no backend can run.
4. Write the change of the JSON words in the `## Breaking` list of the next release note.

## Decision

The owner, 2026-11-04 (`docs/decisions.md`, Doctor): one verdict vocabulary for each check.

## Prove

```sh
cargo test --test cli -- pod_doctor_json_has_the_schema_and_the_verdicts
```

## Done

2026-11-06, commit 43b3559 (`T-078: pod doctor uses the verdict words of doctor and has a schema
key`). `crates/podnet-netns/src/doctor.rs`: `ok` or `fail` for each probe and backend; the JSON
form has `schema` 1, `verdict` for each capability and `verdicts`; the old keys stay; exit 3 stays.
The test is in `tests/commands.rs` (`tests/cli/main.rs` cannot grow) and passes. The `## Breaking` list
is in the draft note `docs/releases/v0.4.0-beta.1.md`.

# T-079: Read the numbers of #206 and #184 in a live run

**Source:** `docs/maintenance.md`, section "Open work" (#206 and #184).
**Category:** measurement
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Two reports wait for numbers. #206: small requests through `connect` can be slower than through
the URL. #184: `warp exit` can miss the IPv6 egress address.

## Premise

- `scripts/live-throughput.sh` prints the median time of small requests over the URL and over
  `connect`.
- `scripts/live-batch2.sh` prints whether `warp exit` found the IPv6 address. When the trace
  gives no IPv6 address, the fallback is the TXT record of `whoami.cloudflare`.

## Approach

1. Read both lines in the next run of `live.yml`.
2. When the median through `connect` is more than 1.5 times the median through the URL, write a
   defect entry with the numbers.
3. When `warp exit` finds no IPv6 address but the trace has one, write a defect entry.
4. Else write the numbers in the Done section.

## Prove

```sh
gh workflow run live.yml
gh run view RUN --log | grep -e median -e ipv6
```

## Done

2026-11-06, live run 42275332991 at 68098d1 (job 128090104282). #206: the median (p50) of 20 small
requests over the URL and through `connect`: quic+direct 88 ms and 131 ms (1.49 times), http2+direct
92 ms and 120 ms (1.30 times), quic+warp 101 ms and 127 ms (1.26 times). Each is under 1.5 times: no
defect entry. quic+direct is near the limit; read it again in a later run. #184: `warp exit` found
the IPv6 egress address (`ipv6 2606:4700:110:8d09:...` and `2606:4700:110:8b0b:...`), and the
batch reads `exit ipv6: yes, prefix 2a09::/16`: no defect entry.

# T-080: The errors of a config file name the line

**Source:** `docs/maintenance.md`, section "Open work" (#172 item 3).
**Category:** feature
**Milestone:** M5
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

An error of a config file names the rule and the key, not the line. In a long file the user
must search for the place.

## Premise

- `crates/podnet-core/src/yaml.rs` keeps no positions.
- `src/config.rs` writes the errors with the rule and the key.

## Approach

1. Keep the line and the column of each node in `crates/podnet-core/src/yaml.rs`.
2. Add the line to each error of `src/config.rs`, beside the rule and the key.

## Prove

```sh
cargo test --workspace -- a_config_error_names_its_line
cargo check --manifest-path fuzz/Cargo.toml --bins
```

The fuzz target `config_yaml` still builds.

## Done

2026-11-06, commit 7e3d835 (`T-080: an error of an ingress rule names the line of the rule`).
`yaml::item_lines` gives the line of each item of a top-level list; each error of an ingress rule
names `ingress rule N (line L)`. A full map of positions in `crates/podnet-core/src/yaml.rs` is not made: the errors
of the top-level keys name the key, which is unique in the file. `a_config_error_names_its_line`
passes; `cargo check --manifest-path fuzz/Cargo.toml --bins` passes.

# T-081: The tailnet front door warns when `tailscaled` is not logged in

**Source:** `docs/maintenance.md`, section "Open work" (#181 item 6);
`docs/research/podnet/podssh.md`, section 2.3, step 1.
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The tailnet front door dials through a `tailscaled` that is not logged in. Each dial fails after
its limit of 10 s, and the user gets no hint.

## Premise

- `crates/podnet-netns/src/tailscale.rs` has the dial limit (`DIAL_TIMEOUT`) and a log line.
- The LocalAPI of `tailscaled` gives `BackendState` at `/localapi/v0/status`.

## Approach

1. At start, and after a failed dial, read `/localapi/v0/status`.
2. When `BackendState` is not `Running`, warn once with the state and the hint `tailscale up`.
3. Show the state in the doctor of the door.

## Prove

```sh
cargo test --workspace -- tailscale_front_door_warns_when_logged_out
```

A fake LocalAPI on a unix socket answers `NeedsLogin`. The headscale harness of T-114 runs the
same check against a real `tailscaled`.

## Done

2026-11-06, commit 3807970 (`T-081: the tailnet front door warns when tailscaled is not logged
in`). `Dialer::backend_state` reads `BackendState` of `/localapi/v0/status` (HTTP/1.0, 3 s, 4 MiB).
The door reads it at its start and after each failed dial, and warns one time when it is not
`Running`, with `tailscale up`. `tailscale_front_door_warns_when_logged_out` passes with a fake
LocalAPI that answers `NeedsLogin`. Not done: step 3 (the state in a doctor of the door): the doctor
of `podnet-netns` has no row for the door. The check against a real `tailscaled` waits for T-114.

# T-135: `podnet pod` prints its own program name in each message and in `--version`

**Source:** the rename trial of 2026-11-05 (`docs/research/podnet/rename.md`); the ruling
"podnet-netns" of 2026-11-05 in `docs/decisions.md`.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The `podnet-netns` binary is dropped; `podnet pod` is the only CLI of the pod backends. The library
still writes its fixed prefix in each message, and `--version` prints the version line of the
library, not of the CLI. A user then sees a program name that does not exist.

## Premise

- `src/main.rs` calls `set_program_name` and `main_with_args` of the pod library.
- The library writes the prefix of its messages from a fixed text, not from the program name.

## Approach

1. Write each message prefix of the pod library from the program name that `set_program_name`
   gives.
2. Make `podnet pod --version` print the version line of the CLI.
3. Change the expected texts of the pod tests.

## Prove

```sh
cargo test --workspace pod
```

A test asserts that `podnet pod --version` starts with the name of the CLI and that an error of
`podnet pod` starts with `podnet pod:`.

## Start condition

T-019 and T-020 are done.

## Done

2026-11-10, commit 266e15a (`T-135: podnet pod prints its own program name in each message and in
--version`). Each message of the pod library starts with `program_name()` (34 sites; the default
is `podnet pod`). `set_version_line` gives the line of `--version`; `src/main.rs` passes the
version line of podnet. Test: `podnet_pod_prints_its_own_program_name` (`tests/pod_usage.rs`):
`podnet pod --version` equals `podnet --version`, and a usage error starts with `podnet pod: `.
`cargo test --workspace pod`: 23 passed, 0 failed. `TMPDIR=/tmp/tm7 sh scripts/agent/check.sh`:
each step 0. CI: at the merge.


# T-138: live: two runs in a row failed on the edge and WARP services (GitHub #316)

**Source:** GitHub #316 (the job report of `live`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The issue: "The workflow live failed in two runs in a row." The failed runs:

- 42060806281 (a2af228), the step "Throughput of the HTTP path and of a --forward WebSocket": the
  edge answered `Unauthorized: Tunnel not found` to the registration of a tunnel that the step
  made some seconds before; `the tunnel is not ready after 90 s`.
- 42062631365 (542f40b), the step "Throughput through WARP, 1 to 16 MiB": the origin server of
  the script got `OSError: [Errno 98] Address already in use`, and the POST of 1 MiB got HTTP 530
  from the edge.

## Premise

- a2af228 changed only `version` beside the git dependencies of `Cargo.toml`. 542f40b changed only
  documents and the record. The code is the code of a5894fe, whose live run 42058519485 passed.
- The next live run 42064698302 (66c9512) passed each step.
- Not measured: the cause on the side of the edge (the time until a new named tunnel is known to
  the edge) and the process that held the port of the origin server.

## Approach

1. Read the failed step of each run (`docs/maintenance.md`, The issue loop).
2. Compare with the runs before and after.
3. The watches of the services move to `pod-services` (`docs/decisions.md`, Services crate): the
   wait for a new tunnel goes there, with no change in podnet now.

## Prove

```sh
curl -qfsS "https://forge.web.pkgforge.dev/github/repos/PodUtils/podnet/actions/runs/42064698302?n=$RANDOM"
```

## Done

2026-11-05, the commit of this Done section. Two failures of services in the live job, not of
the code: the live run 42064698302 of 66c9512 passed with the same code. The podcrates session
noted the "Tunnel not found" for the watches of `pod-services`.

# T-139: The start of a session changes the global git identity and can drop commits of main

**Source:** the podcrates session, 2026-11-06 (a fact that the owner found in the same script of
`PodUtils/podcrates`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`.claude/session-start.sh` ran `git config --global user.name` and `user.email`: it changed the
global git identity of a local machine. On a branch other than `main`, it ran
`git checkout -q -B main origin/main`: it reset an existing local `main` to `origin/main` and
dropped its commits that were not pushed.

## Premise

- `.claude/settings.json` gives the identity to each git command (`GIT_AUTHOR_*`,
  `GIT_COMMITTER_*`); the identity of the repository is enough for the rest.
- `PodUtils/podcrates` fixed its copy of the script (`.claude/session-start.sh` on its `main`).

## Approach

1. Take the script of podcrates: the identity of the repository only; an existing `main` is
   checked out as it is and only fast-forwarded (`git merge --ff-only origin/main`); a new `main`
   tracks `origin/main`; a tree with changes stays on its branch.
2. Test the three cases in clones of a scratch repository.

## Prove

```sh
sh -n .claude/session-start.sh
```

In a clone: a `main` with a commit that is not pushed keeps it; a tree with changes on a branch
stays there; a `main` behind `origin/main` moves forward; `git config --global user.email` does
not change.

## Done

2026-11-06, the commit of this Done section. The three cases passed in scratch clones: the
local commit of `main` stayed; the tree with changes stayed on its branch `work`; `main` moved
forward to the new commit of the origin. The global identity did not change.

# T-140: `data_path` and the time of the last TCP probe in `status`, the ready document and `warp status`

**Source:** T-068, step 4 of its Approach.
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The TCP probe of T-068 marks a session `tcp-dead` and renews it. Only the log shows it. A user of
`warp status`, `podnet status` or the ready document of `warp proxy` cannot see the state of the
data path or the time of the last probe.

## Premise

- Read: `Live::data_path` gives `ok` or `tcp-dead` (`src/warp/live.rs`).
- Read: `warp status` runs in another process than `warp proxy`; it reads files and the socket of
  the instance, not the memory of the proxy.
- `src/warp/proxy.rs` and `src/main.rs` are in `scripts/size-allow.txt` and cannot grow.

## Approach

1. Keep the time of the last probe and its result in `Live`.
2. Put `data_path` and `last_probe` in the ready document of `warp proxy` and `warp up`.
3. Give them to `warp status` and `podnet status` through the record of the instance (or a
   status file in the run directory).

## Decision

2026-11-06: the record of the instance, not a status file. `warp status`, `podnet status`,
`podnet ps` and `warp up` read the record already, and a renewal changes no path. The first
`Live` of the process (the session of `warp proxy`) writes `data_path` and `last_probe` (Unix
seconds) in the ready document of its record after each probe and when a new session opens
(`src/warp/live/instance.rs`). The write reads the record from the disk first
(`detach::update_record`), as `planned_end` and `retry_after` do. A process that is not an
instance writes nothing.

## Prove

```sh
cargo test --workspace -- warp_status_names_the_data_path
```

The test starts a fake session whose probe fails and reads `tcp-dead`, then `ok` after the new
session.

## Done

2026-11-06, commit da611f5 (`warp: data_path and last_probe in the record, warp status and podnet
status (T-140)`). `Live` keeps a `Report` (`data_path`, `last_probe` in Unix seconds). The first
`Live` of a process writes it in the ready document of its instance record after each probe and
at each new session (`src/warp/live/instance.rs`). The ready document of `warp proxy` and `warp
up`, `warp status` (text and `--json`) and `podnet status` show the two keys.
`cargo test --workspace -- warp_status_names_the_data_path` passes; with
`the_ready_document_names_the_data_path` and `status_names_the_data_path_of_an_instance`. Control:
with the report and the record writes disabled, the three tests fail (no `data_path`).
`sh scripts/agent/check.sh`: each check 0.

CI run 42259197563 and live run 42259203680 at 0c4ff75: success.

# T-141: live: two runs in a row failed after the take of pod-dns and pod-services (GitHub #325)

**Source:** GitHub #325 (the job report of `live`).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The issue: "The workflow live failed in two runs in a row." The failed runs:

- 42210655807 (1301fb6, schedule), the step "Quick tunnel with no system DNS, then with 1.1.1.1
  blocked": the step read the lines `podnet::resolve` with `step=` and `server=`, which the take
  of `pod-dns` (bf0c7ca) removed.
- 42222905277 (61c70aa, a run by hand), the step "WARP run with the netns-raw backend": `warp run
  --backend netns-raw` panicked at `src/main.rs` (index 0 of an empty list): the nameservers of
  `cloudflare-dns` were empty after the take of `pod-services`.

## Premise

- The DNS step passed in run 42222905277 after c970da1 (it reads `source="doh"` of `pod_dns`).
- The endpoints `udp-1` and `udp-2` of `cloudflare-dns` give the address as the host, with no list
  of addresses; `Endpoint::ip_addrs` of `pod-services` reads the list only.

## Approach

1. Read the debug line of `pod_dns` in the DNS step (c970da1). Ask podcrates to name the DoH server
   (COORD-004, #322), and restore the check of the server with its release.
2. Read the addresses of an endpoint with `socket_addrs` (`services::ips_of`, 3db91c7), with a
   test.
3. Run `live.yml` at the head of `main` (#326).

## Prove

```sh
cargo test --workspace -- the_nameservers_of_the_service_are_1_1_1_1_and_1_0_0_1
```

The live run of #326 passes each step.

## Done

2026-11-06, commits c970da1 (the DNS step reads `pod_dns`) and 3db91c7 (`services::ips_of`: the
nameservers of `cloudflare-dns` are not empty). `the_nameservers_of_the_service_are_1_1_1_1_and_1_0_0_1`
passes; the control gives `[]`. Live run 42226303514 at 3db91c7: success, each step. CI run
42222918056 at 61c70aa: success. The check of the DoH server comes back with the next podcrates
tag (#322). GitHub #325 is closed with this evidence.


# T-142: seccomp: closing a UDP socket stops its relay and frees three descriptors (GitHub #347)

**Source:** GitHub #347 (https://github.com/PodUtils/podnet/issues/347) (defect 2)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

A program opens a UDP socket, sends one datagram to a port other than 53, and closes the socket. The
supervisor keeps the relay reader task. The task keeps three descriptors: the supervisor end of the
socketpair, the copy of the program end in the `Inbox`, and the relay `UdpSocket` (or, for the
unix-proxy form `UdpRelay::Stream`, the control connection). Observed by the reporter: 11 descriptors
at the start, 311 after 100 sockets, 311 after the close. A long-lived pod runs out of descriptors, and
`socket(2)` then fails with `EIO` (`seccomp.rs:840`).

## Premise

- Read: `ensure_udp_relay` spawns the readers (`seccomp.rs:1216`, `:1226`, `:1246`). They own strong
  clones of `pair`, `inbox` and `udp`. The UDP reader exits only on a `recv_from` error.
- Read: `handle_close` (`fdtable.rs:145`) and `sweep` (`fdtable.rs:85`) remove the table entry only.
  Nothing stops the relay.
- Read: the test `seccomp_closed_sockets_free_their_entries` (`tests/pod_seccomp_fds.rs:266`) makes
  5000 sockets that never send. It cannot see the leak.
- Not measured: the count of 3 per socket. No build ran in this triage.
- Existing tests have a SOCKS5 UDP-in-stream stand-in (`seccomp_backend_relays_udp_in_the_stream`, `tests/pod_seccomp/udp.rs`).

## Approach

1. Move `ensure_udp_relay`, `relay_datagram` and `UdpRelay` out of `seccomp.rs` into a new file
   (the file is at its cap and cannot grow).
2. Give each `Sock::Datagram` a stop signal (a `tokio::sync::watch` sender or a `CancellationToken`).
   Select on it in both reader loops. Drop of the entry sends it.
3. Send the stop from `handle_close`, `sweep` and the shutdown near `drain_relays` (`seccomp.rs:851`).
4. Correct `crates/podnet-netns/README.md:155-157` so that it separates `read` and `write` on TCP from
   the UDP path (the write part belongs to T-143).

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_udp_close_frees_the_relay_descriptors seccomp_udp_exit_frees_the_relay_descriptors seccomp_udp_stream_control_ends_on_close
```

The first test opens 100 UDP sockets, sends one datagram on each, closes each, and counts
`/proc/PID/fd` of the supervisor (not `/proc/self/fd`). It expects the count before the test. The
second test uses a child that exits (the `sweep` path) or `close_range`. The third test checks that the
control connection of the stream form ends.

## Done

2026-11-09, commit f974e6b (`T-142: the close of a UDP socket stops its relay`). The relay code is
in a new file, `crates/podnet-netns/src/udp.rs`. The entry `Sock::Datagram(Udp)` holds a
`tokio::sync::watch` sender; each reader task of the relay selects on it and stops when the entry
drops (`handle_close`, the sweep, and the end of the pod before `drain_relays`). The writer task of
the stream form stops when its channel sender drops with the entry. The tests are in
`tests/dgram/relay.rs`, a module of the test target `pod_seccomp_dgram`:
`seccomp_udp_close_frees_the_relay_descriptors`, `seccomp_udp_exit_frees_the_relay_descriptors`
(a child exits with 100 open sockets; new sockets of the parent start the sweep) and
`seccomp_udp_stream_control_ends_on_close`. Each test counts `/proc/PPID/fd` of the supervisor from
the program. On the tree before the fix the first two give 11 descriptors before and 311 after, and
the third test sees the control connection stay open: the three fail. With the fix the three pass,
on this host (user namespaces and seccomp user notification work here). `seccomp.rs`: 1516 to 1307
lines. `sh scripts/agent/check.sh` exits 0. CI: at the merge.

# T-143: seccomp: a datagram written with `write(2)` leaves the pod (GitHub #347)

**Source:** GitHub #347 (https://github.com/PodUtils/podnet/issues/347) (defect 1)
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

`write(2)` on an emulated datagram socket returns success, and the datagram never leaves the pod. The
reply never comes. `sendto`, `send`, `sendmsg` and `sendmmsg` work. Programs write datagrams with
`write` after `connect` (C code, Python `os.write`, some Go paths). The README says that `write` works
on the pair (`crates/podnet-netns/README.md:155-157`). That is true for TCP only.

## Premise

- Read: `write` is not in `intercepted_syscalls()` (`seccomp.rs:61-84`, 20 syscalls). The kernel
  writes the datagram into the socketpair. Nothing reads the supervisor end: the only use of `pair` is
  `try_send` in `Inbox::deliver` (`inbox.rs:79-110`). A reader on the supervisor end sees only what the
  program wrote, so it does not race with `deliver`.
- Read: `handle_connect` records `dest` for a datagram socket (`connect.rs:97-101`).
- A seccomp filter cannot test membership of the descriptor table, so intercepting `write` costs a
  notification for each write of each program. That cost is not measured.
- Not run: the Python repro of the issue (needs a SOCKS5 server with UDP ASSOCIATE).

## Approach

1. Start T-142 first: the reader selects on the same stop signal.
2. Spawn a task that reads the supervisor end of each datagram pair. It forwards `(dest, payload)` to
   the dispatcher over a channel. The dispatcher owns the table and calls the relay path.
   (`relay_datagram` takes `&mut` table, so the task cannot call it.) A datagram with no `dest` is
   dropped with one log line under `-v`.
3. Keep `dest` current: T-146 clears it on `AF_UNSPEC`, so share it in an `Arc<Mutex<_>>` or send it
   with the channel message.
4. Do not intercept `write` (cost). Write the limit in the README: an unconnected `write` gives no
   `EDESTADDRREQ`.

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_udp_write_after_connect_is_relayed seccomp_udp_sendto_still_works
```

The first test makes a `SOCK_DGRAM`, calls `connect`, calls `write`, and reads the echo from the
stand-in proxy. It fails on c52b8e0.

## Done

2026-11-09, commit eb4a8ba (`T-143: a datagram written with write(2) leaves the pod`). `write` is
not intercepted. `Udp::new` (`crates/podnet-netns/src/udp.rs`) starts a reader task on the
supervisor end of the socketpair. It sends each datagram, with the address of `connect`, on a
bounded queue (256) to the supervisor loop, and `relay_written` sends it through `send_payload`, the
path of `sendto` (port 53 gets the fake-IP answer). The address of `connect` is a cell of the inbox
(`Inbox::dest`), shared with the reader. A datagram with no destination is dropped, with one line
under `-v`. The README gives the limit (no `EDESTADDRREQ`). `handle_bind` moved to `sockopt.rs`,
so `seccomp.rs` does not grow (1287 lines). Tests in `tests/dgram/write.rs`:
`seccomp_udp_write_after_connect_is_relayed` (`write`, `writev`, a DNS query with `write`, and a
`write` with no `connect`; on both forms of the relay) and `seccomp_udp_sendto_still_works`
(`sendto`, `send`, `sendmsg`). On the tree before the fix the first test fails ("write: no
answer"); with the fix both pass on this host. `sh scripts/agent/check.sh` exits 0. CI: at the
merge.

# T-144: seccomp: `bind` on a datagram socket is recorded, and `getsockname` reports it (GitHub #348)

**Source:** GitHub #348 (https://github.com/PodUtils/podnet/issues/348)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`bind(2)` on an emulated UDP socket answers success and records nothing. `getsockname` then gives
`0.0.0.0:0`, also for `bind(0.0.0.0:5353)`. A program that publishes its port (RTP, mDNS, a DNS stub)
advertises port 0. A datagram sent to the bound port is never delivered: only replies to a relayed
`sendto` reach the inbox.

## Premise

- Read: `Sock::Datagram` has `dest` and no `bound` (`seccomp.rs:575-581`). The bind arm is
  `seccomp.rs:1338`. `getsockname` answers from `dest` for both `getsockname` and `getpeername`
  (`sockopt.rs:183`).
- Read: for `getpeername` on an unconnected datagram socket, the code answers success with 0.0.0.0:0.
  The kernel answers `ENOTCONN`. The issue says it "still fails with ENOTCONN": it does not fail today.
- Read: for a connected datagram socket, `getsockname` answers the peer address (`dest`). The kernel
  answers the local address. This is a second defect in the same line.
- Read: `ephemeral_port(fd)` exists (`sockopt.rs:59`) and gives a stable port per descriptor for TCP.
- Not measured: the output of the Python repro.

## Approach

1. Add `bound: Option<(IpAddr, u16)>` to `Sock::Datagram`. Record it in the bind arm. Port 0 gets
   `ephemeral_port(fd)`, as the connected TCP entry does (no new allocator).
2. `getsockname` answers `bound`, else the unspecified address with a stable port. `getpeername`
   answers `dest`, else `ENOTCONN`.
3. `Sock::Icmp` stays as it is. State this in the README.
4. State in the README (line 160 area) that a datagram sent to the bound port is not delivered.
   Keep the change independent of T-147.

## Decision

The lead, 2026-11-09: record the address of a `bind` on a datagram socket. Port 0 gets a stable port from `ephemeral_port(fd)`. The README says that no datagram arrives at a bound port. Do not refuse `bind`. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_udp_bind_is_read_back seccomp_udp_getpeername_is_enotconn_when_unconnected seccomp_udp_getsockname_of_a_connected_socket_is_local
```

## Done

2026-11-09, commit bd0db77 (`T-144: bind on a UDP socket is recorded, and getsockname reports it`), as
the Decision says. `Udp::bind` and `Udp::name` (`crates/podnet-netns/src/udp.rs`): the entry keeps
the address of `bind`; port 0 gets `ephemeral_port(fd)`; a second `bind` gets `EINVAL`.
`getsockname` gives the bound address, else the unspecified address of the family of the socket,
with an ephemeral port after a `connect` or a relayed datagram and port 0 before (as the kernel
binds at the first send). `getpeername` gives the address of `connect`, else `ENOTCONN`. A connected
socket gives the unspecified local address, as a connected TCP socket does: the kernel gives the
address of the route. The README states that no datagram arrives at a bound port, and that the ICMP
socket keeps only its identifier. Tests in `tests/dgram/addresses.rs`:
`seccomp_udp_bind_is_read_back`, `seccomp_udp_getpeername_is_enotconn_when_unconnected`,
`seccomp_udp_getsockname_of_a_connected_socket_is_local`. On the tree before the fix the three fail
(`bound: 0.0.0.0 0`, `unconnected: 0.0.0.0 0`, `local: 192.0.2.10 7777`); with the fix the three
pass on this host. The same C programs, run with no pod, give the same lines except the local
address of a connected socket (the kernel gives the address of the route). `sh
scripts/agent/check.sh` exits 0. CI: at the merge.

# T-145: fake DNS: the pool refuses at its end, and names match without case (GitHub #349)

**Source:** GitHub #349 (https://github.com/PodUtils/podnet/issues/349)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

1. The pool of 131072 addresses (198.18.0.0/15) wraps (`fakedns.rs:65`). Name 131073 gets the address
   of name 1. `by_ip` then names the newer host, so a connection to the older address goes to the
   wrong host.
2. Names match with case. `dns::normalize` (`dns.rs:19`) has no caller. `Example.COM` and
   `example.com` get two addresses and use two pool slots. A `--map` pin is stored as typed
   (`fakedns.rs:46-50`, `cli.rs:166-174`).
3. The AAAA address derives from the v4 address (`fakedns.rs:77-84`), so it has defect 1 too.

## Premise

- Read: all lines above. Wrap arithmetic: after 131071 more names, `next % FAKE_IP_COUNT` is 0 again.
  Not run.
- Read: `answer` has three call sites (`netns.rs:879`, `seccomp.rs:987`, `seccomp.rs:1160`).
  `build_response` takes any rcode (`dns.rs:129`). There is no SERVFAIL constant (`dns.rs:15-16`).
- `FakeDns` cannot know live flows, so LRU eviction needs the backend layer. It is out of this entry.

## Approach

1. Call `normalize` in `ParsedQuery::parse`, in `address_for`, in `name_for` and in `pin`. A pin whose
   name already holds a pool address keeps the pool address. State it in the README.
2. `address_for` returns `Option<Ipv4Addr>`. At `next >= FAKE_IP_COUNT` it returns `None`, with no
   insert. Add `RCODE_SERVFAIL = 2`. `answer` returns SERVFAIL on `None`.
3. Update `docs/proxies.md:55-60`: the pool bound, case-insensitivity, SERVFAIL at the end.

## Prove

```sh
cargo test -p podnet-netns --lib -- fake_pool_refuses_past_its_end fake_names_ignore_case fake_pin_does_not_evict_a_live_name servfail_answers_a_name_that_does_not_fit
```

## Done

2026-11-09, commit 0c3206c (`T-145: the fake-IP pool refuses at its end, and names match with no
case`). `FakeDns::address_for` (`crates/podnet-netns/src/fakedns.rs`) returns `Option<Ipv4Addr>`:
`None` when `next` is at `FAKE_IP_COUNT`, with no insert, so the pool does not wrap. `answer` gives
`RCODE_SERVFAIL` (2, new in `crates/podnet-netns/src/dns.rs`) to an A or AAAA query of a name that
does not fit; both backends use `answer`. `ParsedQuery::parse`, `address_for` and `pin` call
`dns::normalize`; the answer keeps the question bytes as the client wrote them. A pin of a name that
holds a pool address keeps the pool address. The README (DNS) gives the bound, the case rule and
SERVFAIL. The four tests of the Prove pass (`cargo test -p podnet-netns --lib`, 4 passed). On the
tree before the fix they do not compile (`address_for` returned an address), and the arithmetic of
the Premise holds: `next % FAKE_IP_COUNT` gave name 131073 the address of name 1. `sh
scripts/agent/check.sh` exits 0. CI: at the merge.

# T-146: seccomp: `connect(AF_UNSPEC)` dissolves the association of a datagram socket (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 1)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

On Linux, `connect` with `AF_UNSPEC` on a UDP socket disconnects it (DHCP renewal, QUIC migration). Here
the child gets `EAFNOSUPPORT` and `dest` stays.

## Premise

- Read: `handle_connect` parses with `parse_sockaddr`, which takes only `AF_INET` and `AF_INET6`
  (`seccomp.rs:411-429`). The error is at `connect.rs:95`. The datagram arm sets `dest`
  (`connect.rs:97-101`).
- Read: `handle_recv` serves the inbox only when `dest` is `None` (`inbox.rs:452`), so the socket
  works after the disconnect.
- Not run: the C snippet of the issue.

## Approach

1. In `handle_connect`, for a datagram socket whose family field is `AF_UNSPEC` (0) and whose length is
   at least 2: set `dest = None` and answer 0. No local port is minted.
2. A later `sendto` with no address then answers `EDESTADDRREQ` (as today for `dest` = `None`).
3. Order: after T-143, because the pair reader reads `dest`.

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_udp_af_unspec_disconnects
```

## Done

2026-11-09, commit d324710 (`T-146: connect(AF_UNSPEC) dissolves the association of a UDP socket`).
`handle_connect` (`crates/podnet-netns/src/connect.rs`) answers 0 to a datagram socket whose
family field is 0 (length 2 or more), and clears the address of `connect` (`Inbox::set_dest`). No
port is minted. The reader of T-143 reads the same cell, so a later `write` has no destination. The
test `seccomp_udp_af_unspec_disconnects` (`tests/dgram/addresses.rs`) connects, disconnects, and
checks `getpeername` (`ENOTCONN`), `send` with no address (`EDESTADDRREQ`) and the source of
`recvfrom`. On the tree before the fix it fails (`unspec: -1`, the peer stays); with the fix it
passes on this host. `sh scripts/agent/check.sh` exits 0. CI: at the merge.

# T-147: seccomp: a connected datagram socket takes datagrams from its peer only (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 2)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A connected UDP socket reads datagrams from any source that the relay returns. The kernel delivers only
the datagrams of the peer.

## Premise

- Read: `Inbox::deliver` (`inbox.rs:79`) sends into the pair with no check of the source. Both relay
  reader tasks call it (`seccomp.rs:1226`, `:1246`). `Inbox::new` has no `dest`.
- Read: the ruling of 2026-11-04 (`docs/decisions.md:37`): a read of a connected socket stays direct,
  so the filter must act at delivery.
- Open: the form of the filter key. The relay source is a `WireSource` (name or address), the `dest` is
  a fake or real address. Compare after `name_for_ip` mapping.

## Approach

1. Give `Inbox` a shared cell with the recorded `dest`. `deliver` drops a datagram whose source differs
   (no queue: a chatty source must not grow memory).
2. Unconnected sockets keep their behaviour. T-146 clears the cell.

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_connected_udp_drops_other_sources seccomp_unconnected_udp_keeps_all_sources
```

## Done

2026-11-09, commit f2568d4 (`T-147: a connected UDP socket takes datagrams from its peer only`).
The shared cell is the address of `connect` in the inbox (`Inbox::dest`, from T-143).
`Inbox::deliver` (`crates/podnet-netns/src/inbox.rs`) compares the `sockaddr` of the source, as
`Inbox::address` maps it (a name, or the host address of a proxy answer for a name, gives the fake
IP), with the `sockaddr` of `connect`, and drops a datagram of another source before the send into
the socketpair (no queue). A DNS answer has the resolver address as its source, so it passes. The
README states the filter. Tests in `tests/dgram/source.rs`: `seccomp_connected_udp_drops_other_sources`
(both forms of the relay) and `seccomp_unconnected_udp_keeps_all_sources`. On the tree before the
fix the first test fails (`first: NOISE`); with the fix both pass on this host. `sh
scripts/agent/check.sh` exits 0. CI: at the merge.

# T-148: seccomp: datagram and ICMP sockets keep their IP-level options (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 3)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`setsockopt(IP_TTL, 7)` then `getsockopt(IP_TTL)` gives 0 on a datagram socket. The kernel gives 7.

## Premise

- Read: `options_of` returns `None` for `Sock::Datagram` and `Sock::Icmp` (`sockopt.rs:64-69`).
  `setsockopt` answers 0 and drops the value (`sockopt.rs:93-100`). `getsockopt` writes zeros
  (`sockopt.rs:151`). T-063 did the same for TCP.
- `Sock::Icmp` is a struct in `icmp.rs`; it needs an `options` field.
- The value has no upstream effect (SOCKS5 carries no TTL). It is an echo only.

## Approach

1. Add an `options` map to `Sock::Datagram` and to `Icmp`. Return it from `options_of`.
2. State in the README that `IP_TTL` and `IP_MTU` are echoes.

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_udp_ip_ttl_round_trips seccomp_icmp_socket_keeps_its_options
```

## Done

2026-11-09, commit c6aa374 (`T-148: UDP and ICMP sockets keep their IP-level options`). `Udp`
(`crates/podnet-netns/src/udp.rs`) and `Icmp` (`crates/podnet-netns/src/icmp.rs`) have an `options`
map, and `options_of` (`crates/podnet-netns/src/sockopt.rs`) returns it. The README says that the
values are echoes and that `IP_MTU` has no value. Tests in `tests/dgram/options.rs`:
`seccomp_udp_ip_ttl_round_trips` (`IP_TTL`, `IP_TOS`, `IPV6_UNICAST_HOPS`) and
`seccomp_icmp_socket_keeps_its_options`. On the tree before the fix both fail (each value is 0);
with the fix both pass on this host. `sh scripts/agent/check.sh` exits 0. CI: at the merge.

# T-149: seccomp: decide and document the ICMP error and PMTU limit of the UDP relay (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 4)
**Category:** research
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The program never sees `ECONNREFUSED` from an ICMP port-unreachable, never sees `EMSGSIZE` with the
path MTU, and `IP_MTU` has no value. `recvmsg(MSG_ERRQUEUE)` returns `EAGAIN` from the empty queue of
the AF_UNIX pair.

## Premise

- Read: the relay peers with the SOCKS5 proxy, so an ICMP error at the target does not reach a
  supervisor socket. The SOCKS5 UDP header has no ICMP or TTL field. `handle_recv` passes
  `MSG_ERRQUEUE` to the kernel (`inbox.rs:459`).
- Options: (a) carry a signal in the relay protocol (needs a relay change); (b) make `ECONNREFUSED`
  after repeated send failures; (c) document that the fidelity stops at the proxy.

## Approach

1. Choose (c) first and write it in `crates/podnet-netns/README.md` and `pod doctor` text.
2. Answer `recvmsg(MSG_ERRQUEUE)` in the supervisor (`EAGAIN`) so that the answer does not depend on the
   AF_UNIX kernel socket.
3. Record (a) and (b) as later work only when a program needs them.

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_udp_errqueue_read_is_eagain
grep -n "ICMP error" crates/podnet-netns/README.md
```

## Done

2026-11-09, commit ba83a3e (`T-149: the fidelity of UDP stops at the proxy; MSG_ERRQUEUE gets
EAGAIN`). The result of the research: option (c). The relay peers with the proxy, and RFC 1928 gives
the UDP request header no ICMP, TTL or MTU field, so the supervisor cannot know a port-unreachable
or a path MTU of the target. Option (a) needs a new frame type in the relay protocol and in each
server of it (`podnet warp proxy`, `podnet pod --serve`, other SOCKS5 servers do not have it).
Option (b) guesses: a send failure at the proxy is not an ICMP error at the target, and a wrong
`ECONNREFUSED` breaks a program that retries. No program of the record needs (a) or (b) now: they
get an entry when a report names a program that needs them. The limit is in
`crates/podnet-netns/README.md` ("No ICMP error") and in the text form of `pod doctor`
(`crates/podnet-netns/src/doctor.rs`). `handle_recv` (`crates/podnet-netns/src/inbox.rs`) answers
`MSG_ERRQUEUE` on a UDP or ICMP socket with `EAGAIN`, connected or not, before the dispatch to the
inbox. The test `seccomp_udp_errqueue_read_is_eagain` (`tests/dgram/errqueue.rs`) reads the queue
of an empty socket, of a socket with a waiting datagram (the datagram stays), of a connected socket
and of an ICMP socket. On the tree before the fix the first read blocked on the socketpair until the
alarm of the program; with the fix the test passes on this host. `grep -n "ICMP error"
crates/podnet-netns/README.md` gives one line. `sh scripts/agent/check.sh` exits 0. CI: at the merge.

# T-154: seccomp: fuzz targets for the notification handlers (GitHub #353)

**Source:** GitHub #353 (https://github.com/PodUtils/podnet/issues/353) (part 1)
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Each handler parses bytes that the program controls: sockaddr, msghdr, cmsg (`handle_sendto`
`seccomp.rs:947`, `handle_sendmmsg` `:1019`, `sockopt.rs`, `recvmmsg.rs`). `fuzz/fuzz_targets/` has 15
parser targets and none for these handlers. Past bugs of this class: #301, #302, #303, #305.

## Premise

- Read: `read_mem` is `pread` on an fd at an offset (`seccomp.rs:285-300`). `write_mem` writes through
  the same fd (`:303-330`). A memfd with the child memory image is a drop-in `mem`: no `MemSource`
  trait is needed.
- Read: the replies are ioctls on the notify fd (`notif_respond` `:491`, `notif_continue` `:514`,
  `notif_addfd_*`). This is the seam to abstract, so that a target can read the reply.
- `seccomp.rs` is at 1516 lines and cannot grow. T-040 splits it and makes a memory-access module.
- `fuzz/` is its own workspace (nightly, glibc). Its targets call `podnet::fuzzing`.

## Approach

1. After T-040: put the reply primitives behind a small trait with one real implementation (ioctl) and
   one in-memory implementation.
2. Add targets `seccomp_sendto`, `seccomp_sendmsg`, `seccomp_sockopt` (fresh state for each input).
   The oracle: the handler always answers, never panics, never hangs.
3. Add each target to `fuzz/Cargo.toml` and `.github/workflows/fuzz.yml` (a fixed short budget).
4. A crash input becomes a unit test first (the rule of `fuzz/Cargo.toml`).

## Prove

```sh
cargo check --manifest-path fuzz/Cargo.toml --bins
cd fuzz && cargo +nightly fuzz run seccomp_sendto -- -max_total_time=60
```

## Start condition

T-040 is done.

## Done

2026-11-09, commit 6389505 (`T-154: fuzz targets for the seccomp handlers`). A trait `Answer`
(`crates/podnet-netns/src/seccomp/notif.rs`) holds `respond` and `proceed`: the notification
descriptor answers with the ioctl, and a target records the answers in memory. The send handlers
(`crates/podnet-netns/src/seccomp/send.rs`) and the four handlers of `crates/podnet-netns/src/sockopt.rs`
take an `Answer`; the ADDFD calls are not in these handlers and did not change. New module
`crates/podnet-netns/src/fuzzing.rs` with the targets `seccomp_sendto`, `seccomp_sendmsg` (also
`sendmmsg`) and `seccomp_sockopt` (`setsockopt`, `getsockopt`, `bind`, `getsockname`,
`getpeername`): a fresh state for each input, the memory of the program in a sealed memfd. The
oracle: no panic, an end in 5 s, one answer, or an error with no answer. `src/fuzzing.rs`,
`fuzz/Cargo.toml` and `fuzz/fuzz_targets/` have the three targets; `.github/workflows/fuzz.yml`
gives each 60 s. No crash input: a stable random run of 3 x 60 s (4.5 million inputs) found no fault,
and `fuzzing::tests::each_target_survives_mutations` runs the three targets. Tests:
`fuzzing::tests::{each_seed_gets_its_answer, a_burst_of_queries_in_one_sendmmsg_gets_an_answer,
an_empty_input_gets_an_answer}` of `podnet-netns`. `cargo check --manifest-path fuzz/Cargo.toml --bins`:
exit 0. `sh scripts/agent/check.sh`: exit 0. The libFuzzer run (nightly) is not run here: the host has
no nightly toolchain. CI: at the merge (`fuzz.yml` by hand).

# T-155: seccomp: a differential check of the socket calls against the host kernel (GitHub #353)

**Source:** GitHub #353 (https://github.com/PodUtils/podnet/issues/353) (part 2)
**Category:** chore
**Milestone:** M5
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

No test runs one scripted syscall sequence natively and in the pod and compares the errno, the returned
addresses and the cmsg. A rewrite of a handler can pass the hand-written C tests and still differ.

## Premise

- Read: the end-to-end tests are in `tests/pod_seccomp*.rs` (for example `:1588`, `:1657`, `:1717`,
  `:1810`). `tests/pod_seccomp/main.rs` is at its cap (2021).
- No packetdrill in the tree (grep).
- This work finds the gaps of T-143, T-144, T-146 to T-148 as a list. Start with the TCP surface.

## Approach

1. Write a table of C calls with the expected errno from the host (a small driver, run twice).
2. Run it natively and with `podnet pod --backend seccomp`. Fail on any difference, except an allowed
   list (each item names its P-A or T entry).
3. Add `diff-trace -- cmd` only when step 2 works (errno sequence only: no address normalization).
4. Run it in `ci.yml` (seconds).

## Prove

```sh
cargo test --test pod_seccomp_diff -- seccomp_errnos_match_the_host
```

## Done

2026-11-09, commit 46b130b (`T-155: a differential check of the socket calls against the host kernel`).
Steps 1, 2 and 4: `tests/pod_seccomp_diff.rs` compiles one C driver (32 lines: 16 TCP calls,
then 16 UDP calls of T-143 to T-149). It prints the result or the errno name of each call, and the
family, the address and the class of the port (0, fixed, ephemeral) of each address. The test runs
the driver on the host kernel and in `podnet pod --backend seccomp`, and compares the lines. A
difference fails the test, except an item of `ALLOWED`, which names its entry; an item that no
longer differs fails the test too. The check finds six TCP gaps; they are the new entry T-223, and
the six lines of `ALLOWED` name it. Each UDP line agrees. CI runs the test in `cargo test
--workspace` (`.github/workflows/ci.yml`); it takes less than one second here. Step 3 (`diff-trace`)
is not built: the entry builds it only when step 2 works, and nothing asks for it now. The test
passes on this host (user namespaces and seccomp user notification work here). `sh
scripts/agent/check.sh` exits 0. CI: at the merge.

# T-159: research documents are readings; fix the stale counts of the record (GitHub #356)

**Source:** GitHub #356 (https://github.com/PodUtils/podnet/issues/356)
**Category:** docs
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`docs/research/` and `docs/reports/` name the path crates/pod-netns/ (69 hits), the file src/proxy_auth.rs (moved to
`pod-proxy`) and old test paths. Counts differ: `netns.rs` 1199 (now 1200), `warp/up.rs` 1043 (now 847),
`warp/route.rs` 790 (now 799). No check reads these documents. The issue offers (1) a checker with an
allowlist, or (2) a statement that they are snapshots.

## Premise

- Read: `TODO/RULES.md:56` says that these directories "are readings at a time". `TODO/todo.toml:15`
  lists them as `readings`. `architecture.md:3` names its commit (`3bed023`, 2026-11-04).
  `tests/docs.rs:94-105` reads eight documents, not these.
- Read: the live plan is the record. T-040 (`TODO/split.md:810`) still says that `seccomp.rs` has 1,900
  lines, and "do this entry before T-063 starts, or after T-067 is done". Now `seccomp.rs` has 1516
  lines and T-063 to T-067 are done. This is a stale figure in the record, not in a reading.
- Measured: `grep -rn "crates/pod-netns/" docs/research docs/reports` gives 35, 8, 6, 6, 5, 5, 2, 2 hits
  in the eight files that the issue lists.

## Approach

1. Add a banner on top of each affected reading: "Reading of the tree at COMMIT. Paths and counts are
   history. The plan is the record (`TODO/`)." (the form of RULES.md).
2. Correct T-040 (a Correction section): the current counts and the done state of T-063 to T-067.
   Check the other entries that cite `architecture.md` for counts (`grep -n "lines" TODO/*.md`).
3. Do not add a checker (see Decision).

## Decision

The lead, 2026-11-09: mark the research documents as readings at a time with a banner (`TODO/RULES.md` already treats `docs/research/` as readings). Do not add a path checker. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo todo check
grep -L "Reading of the tree" docs/research/podnet/architecture.md docs/research/podnet/parity-checklist.md
```

The `grep -L` prints no file name after the fix.

## Done

2026-11-09, commit 9738eb9. Each research document and `docs/reports/aerrand-agent.md` has the banner `Reading of the tree at COMMIT`. T-040 has a Correction (the counts, T-063 to T-067 done). No checker (Decision). `scripts/agent/check.sh` exits 0. CI at dbfcfd6: ci 42638635724 and live 42638635442 success.

# T-160: tunnel verify: a name that this host cannot resolve yet keeps the tunnel up (GitHub #357)

**Source:** GitHub #357 (https://github.com/PodUtils/podnet/issues/357)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The verify loop retries for 180 s. At the end it exits 69 and shuts the tunnel down, also when each
attempt failed at name resolution, before the edge. The tunnel registered and the URL answers from
outside. Reported: Debian 13 container, egress only through an HTTP CONNECT proxy that resolves names
itself. The error: `could not verify https://NAME.trycloudflare.com in 180 s: ... the proxy ... refused
CONNECT ...: 502`, exit 69, and `curl --proxy ... URL` gives 200. `--no-verify` is all or nothing.

## Premise

- Read: `verify_fetch` (`src/tunnel.rs:1230-1311`). The Dns arm (`:1276-1279`) and the other `Err` arm
  (`:1280-1283`) only change the retry text. After the loop only `last_status` can demote the verdict
  (`:1301-1306`). Otherwise `fault(Unavailable)` (`:1307-1310`).
- Read: the three front ends each shut down and return the error (`src/cli/cmd/run.rs:17-198`).
  `run_demo` passes a body token, so the warn branch cannot apply; it prints PROOF from `check.dns`
  (`run.rs:153-160`). `wants_verify` (`tunnel.rs:1208`) has no caller. The ready document has
  `"verified": bool` (`run.rs:196-215`).
- Read: the test `a_failed_verify_exits_69_after_the_budget` (`tunnel.rs:1489`) uses HTTP 530. It must
  stay fatal.
- A refusal of CONNECT is also how a proxy says "not on my allow-list" (see Decision). The message must say
  "not proved".
- `src/tunnel.rs` is at its 1599 cap.

## Approach

1. Move `verify_fetch` and `Verified` into a new file of the tunnel module. Track `resolution_only`
   for each attempt: hop `Dns`, or a `Refusal` of CONNECT in the error chain (`err.chain()` downcast;
   `src/fetch.rs:71-83`). Any other `Err` clears it.
2. After the loop, when `resolution_only` holds: log a warning in STE ("the tunnel is registered; this
   host cannot resolve the name yet; the URL is not proved") and return `Ok(Verified { dns: "none" })`.
   Apply this also when a body token is set (the demo).
3. Ready document: keep `verified: false` and add `"unverified": "resolution"`. `run_demo` prints a
   line that does not say PROOF.
4. Keep `--no-verify`. Add a value for a strict mode (see Decision).

## Decision

The lead, 2026-11-09: demote the verify verdict also when the failure is a proxy refusal of CONNECT. The text says "not proved" and the tunnel stays up. Add a strict switch for the operator who wants exit 69 in that case. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet --lib -- a_name_that_does_not_resolve_keeps_the_tunnel_up a_failed_verify_exits_69_after_the_budget
cargo test --test cli -- tunnel_ready_document_marks_an_unresolved_name
```

## Done

2026-11-09, commit ebc94ea. `verify_fetch` (now `src/tunnel/verify.rs`) tracks failures before the edge: Dns hop, or a proxy `Refusal` of CONNECT. When each attempt failed so, it returns `Verified { unverified: Some("resolution") }` and the tunnel stays up (also for a body token). The ready document has `unverified`; `run_demo` prints NOT PROVED. `PODNET_VERIFY_STRICT=1` gives exit 69. HTTP 530 stays fatal. Tests: `a_name_that_does_not_resolve_keeps_the_tunnel_up` (skips when a proxy of the host answers), `a_proxy_refusal_is_a_resolution_failure`, `a_failed_verify_exits_69_after_the_budget`. The CLI test `tunnel_ready_document_marks_an_unresolved_name` is not written: it needs a live tunnel. CI: at the merge.

# T-164: Document `--no-verify` in the README, and say what it does not do (GitHub #361)

**Source:** GitHub #361 (https://github.com/PodUtils/podnet/issues/361)
**Category:** docs
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`--no-verify` (shared by `tunnel`, `serve`, `share` and `demo`) is not in the README. It skips the
fetch of the public URL after the registration. It does not change the check of an origin
certificate: `--no-tls-verify` does that (README.md:303). The name reads as "do not check TLS".
Operators of restricted hosts need the flag: the fresh `*.trycloudflare.com` name resolves late there.

## Premise

- `src/cli/opts.rs:59-61` declares it (`global = true`); `src/cli/opts.rs:174` sets `config.verify`.
- `src/cli/cmd/run.rs:23,69` read `config.verify` and gate `verify_accepting`; `src/tunnel.rs:1230`
  runs the fetch. `crates/podnet-core/src/net.rs` takes `no_tls_verify` from the ingress rule. The two flags share no state.
- `grep -n -e '--no-verify' README.md docs/*.md` finds nothing. `wants_verify` (`src/tunnel.rs:1220`) has no caller.
- Not measured: the runtime fetch on a restricted host.

## Approach

1. Add one bullet beside `--no-tls-verify` (README.md:303): what it skips and what it keeps.
2. Keep the name. Reword the help line: "Do not fetch the public URL to prove reachability. Origin TLS
   checks stay on." A rename waits for the third state of #357.
3. Optional: a test that each flag of `TunnelOpts` has a README mention.

## Prove

```sh
grep -c -e '--no-verify' README.md
cargo test --test cli -- help
```

## Done

2026-11-09, commit be7b624. The README has a `--no-verify` bullet beside `--no-tls-verify`. The help line says that origin TLS checks stay on. The name stays. `scripts/agent/check.sh` exits 0. CI at dbfcfd6: ci 42638635724 and live 42638635442 success.

# T-169: `tunnel`: try the MASQUE door of the host's address family first, and take `--masque-endpoint` (GitHub #366)

**Source:** GitHub #366 (https://github.com/PodUtils/podnet/issues/366)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** partial

## Problem

On a host with no IPv4 route, `tunnel --transport warp` tries the IPv4 door first (device record order),
fails with `Network unreachable`, then the IPv6 door, and can reach the WebSocket relay first. `warp`
has `--masque-endpoint`; `tunnel` has none.

## Premise

- `warp_targets` builds the client with no `with_masque` (`src/tunnel.rs:827-838`). Neither
  `TunnelOpts` (`src/cli/opts.rs:21-106`) nor `TunnelArgs` has the flag. `with_masque` exists
  (`src/warp/mod.rs:264`); `warp` uses it (`src/cli/cmd/warp/client.rs:67-74`).
- `open_on` (`src/warp/mod.rs:762-791`) uses `endpoint_order` (`:103-124`): the record order, IPv4 first.
  `has_default_route(true)` (v6) only decides whether the IPv6 door is included.
- The log lines of the issue are from beta.6 on one host. The silent-drop timing is read, not measured.

## Approach

1. Fix: in the `endpoint == None` branch, with no IPv4 default route (`has_default_route(false)`) and no
   won door, put the IPv6 door first. Dual-stack keeps the order. Log the family of a failed door.
2. Flag: `--masque-endpoint IP:PORT` in `TunnelOpts`, through `Config`, to `warp_targets`
   (as `--warp-identity`, `src/cli/opts.rs:173`, `src/tunnel.rs:157,262`).

## Decision

The lead, 2026-11-09: item 1 (the family order) is a fix and lands now. Item 2 (the flag `--masque-endpoint`) is a feature in code that moves to `pod-warp` and `pod-cloudflare`: send it as a COORD issue (to podcrates) and take it with the crates (T-050, T-052). The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet endpoint_order_puts_v6_first_without_a_v4_route
cargo test --test cli -- tunnel_takes_masque_endpoint
```

## Correction

2026-11-09, commit 243720f. Item 1 (the family order) is done: `endpoint_order` (now `src/warp/door.rs`) puts the IPv6 doors first with no IPv4 default route, no proxy, no relay and no door that won. Test: `warp::door::tests::endpoint_order_puts_v6_first_without_a_v4_route`. Item 2 (`--masque-endpoint` in `tunnel`) waits for pod-warp and pod-cloudflare (T-050, T-052) by the Decision; the COORD issue (to podcrates) is for the lead to open. The test `tunnel_takes_masque_endpoint` is not written. CI: at the merge.

# T-170: Quick-tunnel provisioning does not retry a DNS failure that cannot recover (GitHub #367)

**Source:** GitHub #367 (https://github.com/PodUtils/podnet/issues/367)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`provision` retries each transport error up to `PROVISION_ATTEMPTS = 5` (2, 4, 8, 16 s). Only `Hop::Proxy`,
`Hop::Tls` and `Hop::Network` end the loop; `Hop::Dns` falls into the retry arm. With no resolver, five
attempts repeat the same failure for about 30 s.

## Premise

- `src/quicktunnel.rs:258-260` (budget and attempts), `:292-312` (the arms; `Hop::Dns` goes to
  `ProvisionError::Http(_)` at `:312`).
- `crate::resolve::answering_step(host)` (`src/resolve.rs:150`) names the step that answers now.
- `src/quicktunnel.rs` is Cloudflare adapter code (pod-cloudflare, T-029/T-052): a fix is allowed.
- Not measured: the 30 s run of the issue.

## Approach

1. Retry a `Hop::Dns` failure once. Retry again only when `answering_step` of the API host changed.
2. End with the `Hop::Dns` hint (`crates/podnet-core/src/hop.rs:45`).

## Prove

```sh
cargo test -p podnet quicktunnel::tests::a_dns_failure_is_retried_once
```

## Done

2026-11-09, commit 99aa461. `provision` retries a `Hop::Dns` failure once, then again only when `answering_step` of the API host changed (`src/quicktunnel/retry.rs`). The error ends with the `Hop::Dns` hint through the hop of the error. Test: `quicktunnel::retry::tests::a_dns_failure_is_retried_once` (the decision, not a live run). CI: at the merge.

# T-171: The verify loop names the remedy when the public name does not resolve (GitHub #367)

**Source:** GitHub #367 (https://github.com/PodUtils/podnet/issues/367)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`verify_fetch` retries 180 s (`VERIFY_BUDGET`, `src/tunnel.rs:1315`). On a host with no resolver the
error text repeats `system resolver: no answer ... Network unreachable`. The final error
(`src/tunnel.rs:1307-1310`) names no remedy: `--resolve HOST=IP`, or `--no-verify`.

## Premise

- The loop keeps `last = "waiting for DNS: ..."` (`src/tunnel.rs:1276-1279`). `Verified.dns` records the
  step on success only. `Hop::Dns` has a hint (`crates/podnet-core/src/hop.rs:45`).
- `run_tunnel`, `run_share` and `run_demo` (`src/cli/cmd/run.rs`) wait for the verify.
- Overlap: #357 (exit 69 and teardown on an unresolved name). Do both in one pass; the verdict of the
  loop after the budget is #357's subject.

## Approach

1. After the provision, if `answering_step(host)` is `none`, print one line with the name, the chain
   state and the two remedies.
2. Add the hint to the final error.

## Prove

```sh
cargo test -p podnet tunnel::tests::verify_failure_names_the_remedy
```

## Done

2026-11-09, commit ebc94ea (with T-160). The final error of a verify that failed before the edge names `--resolve HOST=IP` and `--no-verify`; the first failed attempt logs the same remedy. Test: `tunnel::verify::tests::verify_failure_names_the_remedy` (skips when a proxy of the host answers). The line before the start of the verify is not added. CI: at the merge.

# T-173: A WARP identity of another uid, with a consent variable (GitHub #368)

**Source:** GitHub #368 (https://github.com/PodUtils/podnet/issues/368)
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** partial

## Problem

`check_private` refuses an identity file whose owner is another uid, also with `--identity PATH`. The
state directory has `PODNET_FOREIGN_HOME=allow`; the identity file has no override. Root cannot read the
device that a service user saved. A second device spends a registration (`docs/decisions.md`, WARP devices).

## Premise

- `src/warp/mod.rs:885-892` (owner and mode checks, bare `bail!`); readers: `load` (`:357`) and
  `src/warp/export.rs:68`. `register_or_load` (`src/warp/mod.rs:415-417`) refuses and does not write over the file.
- `crates/podnet-core/src/state.rs:67-118` has the model. `src/manual/facts.rs:136` lists `PODNET_FOREIGN_HOME`.
- `warp_client_inner` wraps errors in `Fault::Unavailable` (`src/cli/cmd/warp/client.rs:119-144`).
- Not measured: two real users (a registration is spent).

## Approach

1. Allow an owner mismatch with the consent variable (see Decision); keep the mode check; warn.
2. Give the refusal `Fault::Config` and name the variable. Name it in the state-directory error too.
3. Keep: a refused file never leads to a registration.
4. Update `src/manual/facts.rs`.

## Decision

The lead, 2026-11-09: the consent variable is a new `PODNET_FOREIGN_IDENTITY=allow`, not `PODNET_FOREIGN_HOME`: the existing word covers writes in a directory, and an identity is a credential. The exit code and the message (item 2) land now. The feature (item 1) goes as a COORD issue (to podcrates) and is taken with `pod-warp` (T-050). The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet warp::private_file_tests
```

The tests cover: refuse, allow with the variable, mode still checked, no registration after a refusal.

## Correction

2026-11-09, commit 5a21d81 (`T-173: a WARP identity of another uid is a Config error (exit 78)`). Done as the Decision says: item 2 (the exit code). `check_private` is in `src/warp/private.rs`; an owner of another uid gives `Fault::Config`. `warp_client_inner` and the carry and WireGuard peers keep a class that the library gave (the outer `Fault::Unavailable` hid it; this also gives a directory as `--identity` the exit 64 that its test names). A refused file leads to no registration. Tests: `warp::private_file_tests::foreign::{a_foreign_owner_is_a_config_error, a_refused_identity_leads_to_no_registration}` (the second one chowns the file, as root only), `cli::cmd::warp::client::tests::a_class_of_the_library_stays`. The message does not name `PODNET_FOREIGN_IDENTITY`: the variable has no effect until item 1 lands, and a message must not name a variable that does nothing. Item 1 (allow the owner mismatch with the variable, keep the mode check, warn) and the update of `src/manual/facts.rs` wait: a COORD issue (to podcrates) is to be opened by the lead (a worktree agent writes no issue); podnet takes it with `pod-warp` (T-050). Item 2 for the state-directory error: it names `PODNET_FOREIGN_HOME` already. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-182: `doctor --agent`: a row is `ok` only when the index host and the file host answer (GitHub #378)

**Source:** GitHub #378 (https://github.com/PodUtils/podnet/issues/378)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`podnet doctor --agent` checks one TLS handshake for each row. Three rows name the host of the index, and the tool gets its files from another host: `pip` (`pypi.org`, files from `files.pythonhosted.org`), `cargo` (`index.crates.io`, files from `static.crates.io`) and `docker pull` (`registry-1.docker.io`, blobs by a 307 to another host, `production.cloudfront.docker.com` at the time of the issue). A host that reaches the index and not the files shows `ok`. `npm`, `go modules` and `github.com` use one host.

## Premise

- `Task` has one `host` and one `port`, and `Check::Tls` is one handshake (`src/agent.rs:26-42`). The three rows are at `src/agent.rs:60-65`, `:74-79` and `:88-93`. The table prints one target (`src/agent.rs:541-561`).
- `registry-1.docker.io` is in pod-services as the endpoint `registry` of `docker-hub-registry`. The other hosts are not in pod-services (read of `services/` at podcrates v0.7.0).
- `src/agent.rs` is a taken source of pod-services (not frozen).
- Not measured: the three answers of the issue (curl of 2026-11-07).

## Approach

1. Add an optional second target to `Task` (the host of the files). A row is `ok` only when both answer. Name the failed host in the row: `index ok, files fail`. Add the second host to the JSON of `--agent`.
2. `docker pull`: read the host of the blob from the 307 of one blob request. This adds no hardcoded host. `cargo`: the `dl` key of `config.json` names `static.crates.io`.
3. `pip`: the host of the files is a new host of another party. Do not hardcode it (`docs/decisions.md`, Services crate): ask podcrates for service files (COORD issue `(to podcrates)`), then read them with `crate::services::endpoint`. Start condition: the file of `pod-services` exists.
4. Keep `npm`, `go modules` and the two `github.com` rows as they are.

## Decision

The lead, 2026-11-09: the docker and cargo file hosts come from the answers of the index (no hardcoded host). The pip file host comes from a file of `pod-services` that podcrates adds. The entry waits for that file. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet --lib agent::
cargo run -q -- doctor --agent --json
```

A unit test gives a row with a passing index and a failing file host and expects `fail` and the name of the host.

## Correction

2026-11-09: the blocker cleared. podcrates v0.8.0 has `pod_services::get("pypi-files")` (podcrates T-127); podnet takes v0.8.0 in 0df677d.

## Done

2026-11-09, commit 9c5eda7. A row with a file host is ok only when both hosts answer; the row names the failed host. Cargo: the dl key of config.json. Docker: the Location of the redirect of one blob request (manifest walk of library/hello-world). Pip: pod_services pypi-files. JSON rows get the key files. Tests: agent::tests::a_failing_file_host_fails_the_row_and_is_named, agent_files::tests. Not run: a live `doctor --agent --json` (no live host here). check.sh: exit 0. CI: at the merge.

# T-183: One port for an `http://` proxy URL with no port in podnet and podssh (GitHub #379)

**Source:** GitHub #379 (https://github.com/PodUtils/podnet/issues/379)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`HTTPS_PROXY=http://gw` gives `gw:8080` in podnet (`HTTP_PORT = 8080`, comment "as in curl") and `gw:80` in podssh (`pod-proxy` default). curl reads a missing proxy port as 1080 (manual, `--proxy`). The comment names a program whose value it is not. A change of the value also moves which `NO_PROXY` entries match (`no_proxy_ports`).

## Premise

- `src/egress.rs:45-46` sets 8080 and `rules()` passes it as `http_port` (`src/egress.rs:50-61`); `parse()` uses it (`:66`).
- `pod-proxy` `HTTP_PORT = 80` (`proxy/parse.rs:21`) is the default of `Rules` (`env.rs:141`). A consumer can set another value (`env.rs:124`).
- The tests assert the constant, not a number (`src/egress.rs:461,475`).
- `docs/decisions.md` has no ruling on this value. Not measured: curl behaviour (read from the issue).

## Approach

1. The value is 80 (see Decision): the default of `pod-proxy`; delete the override in `rules()` and `parse()`.
2. Write the reason once, with the true program names (Go and Python requests read 80). Record the ruling in `docs/decisions.md`.
3. Change the tests to assert the number, and add the same case with a comment to the podssh side as a COORD issue `(to podssh)`.

## Decision

The lead, 2026-11-09: the default port of an `http://` proxy URL with no port is 80 (the value of `pod-proxy`, podssh, Go and Python). Not 8080. Record the ruling in `docs/decisions.md`. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet --lib egress::
grep -n "HTTP_PORT" src/egress.rs
```

The test asserts `parse("http://127.0.0.1").unwrap().port == 80` (or the ruled number).

## Done

2026-11-09, commit cca2e0a. egress::rules and egress::parse use the default of pod-proxy (80); HTTP_PORT of egress.rs is removed; the ruling is in docs/decisions.md. Test: egress::tests (the chain test asserts 80 and 1080). COORD (to podssh): open for the lead. check.sh: exit 0. CI: at the merge.

# T-184: `doctor`: the relay row follows `--relay` and keeps a loopback relay (GitHub #380)

**Source:** GitHub #380 (https://github.com/PodUtils/podnet/issues/380)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet doctor` builds the relay row with `Relay::from_env(None)` and keeps it only when the URL starts with `https://` (`src/doctor.rs:159-170`). The user cannot pass the relay on the command line, and a loopback relay (`http://127.0.0.1:PORT`, allowed by `src/ws_relay.rs:113-165`) gives no row. `bug-report --doctor` has the same table. `--transport relay` is not seen either.

## Premise

- `--relay` and `--transport` are `global = true` in `TunnelOpts` (`src/cli/opts.rs:39-49`). `TunnelOpts` is flattened only into `tunnel` and `share` (`src/cli/cmd/tunnel.rs:17`, `src/cli/cmd/share.rs:22,62,114`). `DoctorArgs` has no `--relay` (`src/cli/cmd/host.rs:14-33`). A global flag does not reach a sibling command, so `doctor --relay X` most likely fails in clap. Not run (no build).
- `targets()` has no argument (`src/doctor.rs:111`). `doctor()` calls `Options::default()` with three fields set (`src/cli/cmd/host.rs:259-264`).
- `src/doctor.rs` has 1305 lines in `scripts/size-allow.txt`. It can only shrink: move code out in the same commit (T-033).

## Approach

1. Add `--relay URL` to `DoctorArgs` (env `PODNET_RELAY`). Add a `relay: Option<String>` to `doctor::Options` and pass it to `targets(relay)`.
2. Keep a loopback `http://` relay as a row with a plain check and the words `loopback relay, no TLS`.
3. Do not add a `--transport` flag to doctor. Name the relay row always when a relay is given.
4. Tests of `targets()` for: no relay, an `https://` relay, a comma list, a loopback relay.

## Prove

```sh
cargo test -p podnet --lib doctor::
python3 scripts/check-size.py
```

## Done

2026-11-09, commit aff90d5. podnet doctor --relay URL (env PODNET_RELAY) sets Options.relay; targets(relay) is in src/doctor/targets.rs; a loopback http:// relay gets the row with Tls::Plain and the words 'loopback relay, no TLS'. src/doctor.rs is smaller. Tests: doctor::targets::tests::the_relay_row_follows_the_given_relay, the_targets_hold_the_given_relay. check.sh: exit 0. CI: at the merge.

# T-185: A deadline for the client handshake of `net proxy`, `net tailscale` and `pod --serve` (GitHub #384)

**Source:** GitHub #384 (https://github.com/PodUtils/podnet/issues/384)
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

A client that connects and sends nothing, or one byte of a SOCKS5 greeting, holds its task and socket until the process ends. Each connection is unauthenticated. The accept loops have no cap. The WARP door has the fix (`REQUEST_LIMIT`, 30 s, `src/warp/proxy.rs:586`). `CLIENT_TIMEOUT` (300 s) wraps only the copy.

## Premise

- `crates/podnet-vnet/src/proxy.rs:84-96`: `fill_buf`, `socks5_read_request` and `http_read_connect` have no timeout. The copy is wrapped at `:118-125`. Accept loops: `:267-292`.
- `crates/podnet-netns/src/tailscale.rs:377,383,387`: same reads. The dial and the copy have a timeout (`:386`, `:406`).
- `crates/podnet-netns/src/frontdoor.rs:79,121`: same reads in the door of `pod --serve` (not in the issue).
- `src/warp/proxy.rs:628-630,657-661,703-706`: the precedent.
- Not run.

## Approach

1. One constant `REQUEST_LIMIT` (30 s) in `podnet-netns` (`socks.rs`). Wrap the first byte and the request read in each of the three doors. The warp door uses the same constant.
2. On expiry send a SOCKS5 reply `0x04` or an HTTP `504`, then close (`fail_with`, `crates/podnet-vnet/src/proxy.rs:161-180`).
3. Fix the comment of `CLIENT_TIMEOUT`: it bounds the copy only.
4. Count live client tasks (as `SocketCount::live`, `PodUtils/podcrates:crates/pod-netstack/src/stack/socket.rs`). A cap is a later choice.

## Prove

```sh
cargo test -p podnet --lib vnet::proxy::
cargo test -p podnet-netns tailscale
```

Tests drive `handle_connection` over a duplex with tokio paused time: no byte, one byte `0x05`, and a CONNECT head with no blank line. Each ends within the limit with an answer.

## Done

2026-11-09, commit e0daff1. The doors of net proxy, net tailscale and pod --serve read the opening with REQUEST_LIMIT (30 s, crates/podnet-netns/src/opening.rs); on expiry the client gets SOCKS5 code 4 or HTTP 504. Tests: opening::tests::no_byte_ends_with_a_504, one_socks_byte_ends_with_code_4, a_connect_head_without_a_blank_line_ends_with_a_504. check.sh: exit 0. CI: at the merge.

# T-186: `net tailscale` and the other unix listeners claim the socket path first (GitHub #385)

**Source:** GitHub #385 (https://github.com/PodUtils/podnet/issues/385)
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`bind_unix` of the tailnet door removes the path and binds (`crates/podnet-netns/src/tailscale.rs:354-357`). It deletes a regular file and takes over a live socket. The ruling "Sockets" (`docs/decisions.md:22`): a live socket is 69, a file that is not a socket is 64, a stale socket is removed. The default path is `unix:/tmp/podnet-socks.sock` (`src/cli/cmd/net/mod.rs:78`).

## Premise

- Five callers use `claim_socket_path` (`src/warp/proxy.rs:85`, `crates/podnet-vnet/src/switch.rs:433`, `crates/podnet-vnet/src/proxy.rs:313`, `src/cli/cmd/connect.rs:249`, `src/cli/cmd/relay.rs:194`). The function is in the root crate (`crates/podnet-core/src/net.rs:21-46`): `podnet-netns` cannot call it.
- Two more listeners do `remove_file` then `bind_private`: `crates/podnet-netns/src/frontdoor.rs:59` and `crates/podnet-netns/src/seccomp/accept.rs:45`.
- `crates/podnet-core/src/net.rs` has no test of `claim_socket_path` (tests start at `crates/podnet-core/src/net.rs:174`).
- `net_tailscale` opens the LocalAPI socket before the bind (`src/cli/cmd/net/shim.rs:70-76`).

## Approach

1. Move the claim logic to `podnet_netns::listen` (with `Fault` mapping kept in the root by a thin wrapper), or call `claim_socket_path` in `net_tailscale` before `tailscale::serve`. Use it in `bind_unix` of the tailnet door and of `frontdoor.rs`.
2. Tests for three cases: live socket (69), regular file (64), stale socket (removed).
3. Name the exit codes in the help of `net tailscale` and in `docs/virtual-network.md`. Do not edit `docs/releases/v0.3.0-beta.12.md` (a reading at a time).
4. The seccomp inbound path (`seccomp.rs:1368`) is a path that the supervisor makes: leave it, and say why in the commit.

## Prove

```sh
cargo test -p podnet --lib net::
cargo test -p podnet-netns tailscale
```

## Done

2026-11-09, commit 864fea6. net tailscale and the front door claim the path first (listen::claim_unix_path, listen::bind_claimed). Tests: listen::tests::a_claim_refuses_a_live_socket_and_a_file_and_removes_a_stale_socket, net::tests::a_claim_gives_69_for_a_live_socket_64_for_a_file_and_removes_a_stale_one. check.sh: exit 0. CI: at the merge.

# T-187: `net proxy --forward`: a real target may be a name, and the dial reports its hop (GitHub #386)

**Source:** GitHub #386 (https://github.com/PodUtils/podnet/issues/386)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`RealTarget::parse` takes only a `SocketAddr` (`src/cli/cmd/net/proxy.rs`, before this entry) and `localhost:3000` gives `bad real address`. The help and `docs/virtual-network.md:119-120` say `host:port`. The dial is a bare `TcpStream::connect` (same file): the proxy of the process, `NO_PROXY` and the resolver chain do not apply, and the warn has a bare `io::Error` (`crates/podnet-vnet/src/proxy.rs:210`). Measured by the issue: a dead `--proxy` changed nothing.

## Premise

- `connect_origin` (`crates/podnet-core/src/net.rs:103-128`) resolves names and `localhost`. `egress::dial` (`src/egress.rs:181-199`) uses the proxy and `NO_PROXY`. `split_host_port` is private (`crates/podnet-core/src/net.rs:94-98`).
- `rules()` sends the loopback through the proxy (`loopback_direct: false`). A forward to `127.0.0.1` would then use the proxy. The Decision rules it: the loopback stays direct.
- The `F: Fn() -> Fut` shape of `forward` stays (`crates/podnet-vnet/src/proxy.rs:185-189`).

## Approach

1. `RealTarget::Tcp` holds host and port. Accept a name and `[v6]:port`. Dial with a timeout.
2. Apply the Decision for the proxy. Log the target, the proxy label and the hop with `egress::error_of` words.
3. Keep `unix:` as it is. Update the help and the document in the same change.

## Decision

The lead, 2026-11-09: `--forward` uses the proxy of the process for a non-loopback target (with `NO_PROXY`). It dials direct for a loopback target and for `unix:`. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet --lib cli::cmd::net::proxy
```

Unit tests: a literal, a name, a unix path, and the error words without a network.

## Done

2026-11-09, commit 76d8993. RealTarget (src/cli/cmd/net/real.rs) accepts HOST:PORT, [V6]:PORT and unix:/PATH; a non-loopback target uses egress::dial (process proxy, NO_PROXY); loopback and unix dial direct; the error words name target, way and hop. Decision of the lead (2026-11-09) applied. Tests: real::tests::a_literal_a_name_a_v6_and_a_unix_path_parse, only_a_target_that_is_not_loopback_uses_the_process_proxy, the_error_words_name_the_target_the_way_and_the_hop. check.sh: exit 0. CI: at the merge.

# T-188: `net proxy --forward` reports a failed bind before it prints the forward (GitHub #387)

**Source:** GitHub #387 (https://github.com/PodUtils/podnet/issues/387)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`forward` binds inside a spawned task and returns a `JoinHandle` (`crates/podnet-vnet/src/proxy.rs:185-203`). `net_proxy` prints `forward VIRT -> REAL` at once (`src/cli/cmd/net/proxy.rs:120`). An address outside the subnet, a duplicate `--forward`, an ACL refusal or a collision with a shim bind give the line, a log warn and exit 0. `--import` binds before it prints (`crates/podnet-vnet/src/switch.rs:752-756`).

## Premise

- Refusals: `PodUtils/podcrates:crates/pod-netstack/src/stack/reactor.rs` (`AddrNotAvailable`, `PermissionDenied`, `AddrInUse`).
- `spawn_forwards` cannot return a result (`crates/podnet-vnet/src/proxy.rs:237-244`).
- The `--map` entry outside the subnet fails only at the first request (note of the issue: not verified).

## Approach

1. Bind first (async `forward`, or a `oneshot`). Return the result to `net_proxy`.
2. Map `AddrNotAvailable` to `Fault::Usage`, `AddrInUse` and `PermissionDenied` to `Fault::Unavailable`. Print the line after the bind.
3. Reject a repeated `VIRT` at parse time with its name.
4. Keep the per-connection warn at `crates/podnet-vnet/src/proxy.rs:210-212`.

## Prove

```sh
cargo test -p podnet --lib vnet::proxy::
```

Tests: a forward outside the subnet returns the error; a second forward of one address returns `AddrInUse`; a valid forward still serves.

## Done

2026-11-09, commit 6360717. vnet::proxy::forward is async and binds before it returns; net proxy prints the forward line after the bind and maps AddrNotAvailable to 64 and AddrInUse and PermissionDenied to 69; a repeated VIRT is a usage error. Tests: vnet::proxy::tests::a_forward_outside_the_subnet_returns_the_error, a_second_forward_of_one_address_returns_addr_in_use; cli::cmd::net::proxy::tests::a_repeated_forward_address_is_a_usage_error, a_refused_bind_has_its_class; tests/vnet_proxy_forward.rs serves a valid forward. check.sh: exit 0. CI: at the merge.

# T-189: The switch counts each packet that it drops (GitHub #388, part 1)

**Source:** GitHub #388 (https://github.com/PodUtils/podnet/issues/388)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`Table::forward` drops a packet when the queue of the peer is full and ignores the error (`let _ = peer.try_send(...)`, `crates/podnet-vnet/src/switch.rs:164,171`). The queue is 1024 packets (`:30`, `:463`). No counter, no log, no accessor. The `spoofed` counter (`:153`) has no accessor. The rule of the project: a drop is counted (`NetStats::dropped`, `PodUtils/podcrates:crates/pod-netstack/src/stack.rs`). The fault injector counts are not readable either (`crates/podnet-vnet/src/switch.rs:54-111`).

## Premise

- Read only. No run. `crates/podnet-vnet/src/switch.rs` has 711 lines.

## Approach

1. Count each failed `try_send` for each peer. Log the first drop of each peer at warn (peer id, addresses, depth).
2. A snapshot method of `Switch`: for each peer, sent and dropped; plus `spoofed` and the injector counts. Print the totals at stop.
3. Policy for a full queue stays a drop with a count (backpressure needs a change of the reader: later).

## Prove

```sh
cargo test -p podnet --lib vnet::switch::
```

A test fills the queue of a peer that does not read and reads a non-zero drop count.

## Done

2026-11-09, commit da010b5. Table::deliver counts sent and dropped for each peer; Switch::counts (crates/podnet-vnet/src/switch/counts.rs) gives them with the spoofed count; the stop logs the totals; FaultInjector logs its counts on drop. Test: vnet::switch::tests::a_full_queue_counts_its_drops. check.sh: exit 0. CI: at the merge.

# T-190: A stack over a switch link has bounded queues (GitHub #388, part 2)

**Source:** GitHub #388 (https://github.com/PodUtils/podnet/issues/388)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`NetStack::over_link` builds `LinkDevice::tunnel(config.mtu)`: both queues are unbounded and the stack does not hold when full (`NetStack::over_link`, `LinkDevice::tunnel`; now `PodUtils/podcrates:crates/pod-netstack/src/stack.rs`, `PodUtils/podcrates:crates/pod-netstack/src/device.rs`). A slow pump grows the heap. The WARP stack uses `tunnel_bounded` and `with_inbound_limit` (`src/warp/ip.rs:454-455`, 512 and 4096 packets).

## Premise

- `over_link` has one production caller (`src/cli/cmd/net/proxy.rs:58`) and the switch tests (`crates/podnet-vnet/src/switch/tests.rs`).
- `crates/podnet-vnet/src/stack.rs` has 2103 lines in `scripts/size-allow.txt` and can only shrink. Two new `StackConfig` fields grow it.

## Approach

1. Add `LinkDevice::link(mtu)` in `crates/podnet-vnet/src/device.rs` (639 lines): a bounded tunnel with the limits as named constants. In `stack.rs` replace the name `tunnel` by `link` (same line count). No `StackConfig` field now; fields wait for T-034.
2. Check that `device_stats()` shows the drops.
3. Existing switch tests, including the 64 KiB transfer with faults (`docs/virtual-network.md`), pass.

## Prove

```sh
cargo test -p podnet --lib vnet::
python3 scripts/check-size.py
```

## Done

2026-11-09, commit b40d290. LinkDevice::link (crates/podnet-vnet/src/device.rs) bounds both queues (LINK_OUTBOUND 512, LINK_INBOUND 4096) and counts an inbound drop in rx_dropped, so device_stats() shows it; stack.rs uses it with no new line. Test: vnet::device::tests::a_link_device_is_bounded_and_counts_its_inbound_drops; the switch tests including the 64 KiB transfer with faults pass. check.sh: exit 0. CI: at the merge.

# T-191: The listener table of `net doctor` names each command that listens (GitHub #389, part 1)

**Source:** GitHub #389 (https://github.com/PodUtils/podnet/issues/389)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`NEEDS` (`crates/podnet-vnet/src/doctor.rs:122-129`) lists `warp proxy`, `connect -L` and `net proxy` for `listen(AF_INET)`, and `warp proxy` and `connect -L` for `listen(AF_UNIX)`. Missing: `net switch` (`crates/podnet-vnet/src/switch.rs:413-415`), `net tailscale` (`tailscale.rs:321-322`) and `relay connect --listen` (`src/cli/cmd/relay.rs:177,190-192`). `net proxy`, `net switch` and `net tailscale` default to a unix path (`src/cli/cmd/net/mod.rs:65,138,196`). `docs/target-environment.md:126` repeats the short list.

## Premise

- Printed at `src/cli/cmd/net/mod.rs:307-308`. `tests/cli/main.rs` has `doctor_tells_listen_from_bind`.
- `docs/networks.md:39-41` uses `net switch --listen tcp://` as its example.

## Approach

1. Rewrite `NEEDS` with each command and the default form. Fix `docs/target-environment.md:126`.
2. A test reads the `--listen` flags of the command tree and fails when a command is not in `NEEDS` (a generated table is a later step).

## Prove

```sh
cargo test -p podnet --lib vnet::doctor::
cargo test -p podnet --test cli doctor_tells_listen_from_bind
```

## Done

2026-11-09, commit a1d4666. NEEDS (crates/podnet-vnet/src/doctor.rs) and docs/target-environment.md list each command that listens. Test: cli::listen_table::each_command_with_a_listen_flag_is_in_the_listener_table; tests/cli.rs doctor_tells_listen_from_bind passes. check.sh: exit 0. CI: at the merge.

# T-192: A refused TCP bind exits 69 in each listener command (GitHub #389, part 2; GitHub #385 step 5)

**Source:** GitHub #389 (https://github.com/PodUtils/podnet/issues/389)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`crates/podnet-core/src/exit.rs:115-127` promises 69 for "a service or a network path is not available". A TCP bind that fails gives exit 1: `crates/podnet-vnet/src/switch.rs:413-415`, `crates/podnet-netns/src/tailscale.rs:320-321`, `src/cli/cmd/relay.rs:192`, and also `crates/podnet-vnet/src/proxy.rs:262-264` and `src/warp/proxy.rs:71-73` (not named in the issue). `exit.rs:152` reads only `Faulted`.

## Premise

- Read only. The unix binds use `claim_socket_path` and give 69 for a live socket.

## Approach

1. One helper `bind_tcp(addr)` in the root crate that binds and maps `AddrInUse`, `PermissionDenied` and `AddrNotAvailable` to `Fault::Unavailable`. Use it at the five sites. For `podnet-netns`, wrap at the caller in `net_tailscale`.
2. A test binds twice on one port and expects 69.

## Prove

```sh
cargo test -p podnet --lib net::
cargo test -p podnet --test cli
```

## Done

2026-11-09, commit 56ee2fa. net::bind_tcp and net::bind_fault map a refused bind to exit 69 at net switch, net proxy, warp proxy, relay connect --listen, and net tailscale (wrapped at the caller). Test: net::tests::a_second_bind_on_one_port_exits_69. check.sh: exit 0. CI: at the merge.

# T-193: `net proxy --map` fills the gateway resolver and the shim table (GitHub #390)

**Source:** GitHub #390 (https://github.com/PodUtils/podnet/issues/390)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`--map NAME=ADDR` builds one `HashMap` for the door only (`src/cli/cmd/net/proxy.rs:94-106`). The stack gets `dns: None` (`StackConfig::default`, now its `gateway_service`: `PodUtils/podcrates:crates/pod-netstack/src/stack.rs`), so nothing listens on `10.66.0.1:53` (`PodUtils/podcrates:crates/pod-netstack/src/stack/reactor.rs`, `PodUtils/podcrates:crates/pod-netstack/src/stack/service.rs`). `net shim` has an empty `hosts` (`src/cli/cmd/net/shim.rs:27-38`, `crates/podnet-vnet/src/shim.rs:53,82-90`). `docs/virtual-network.md:241-246` and `net addresses` advertise the resolver.

## Premise

- `vnet::dns::Resolver` exists and is tested: `from_lines` (`crates/podnet-vnet/src/dns.rs:57`), `hosts_pairs` (`:168`), fields `upstream` and `catch_all` (`:26,29`) that nothing sets.
- `crates/podnet-vnet/src/stack.rs` allowlist (2103 lines): no new test in that file before T-034. A test can use the public `StackConfig.dns` from `crates/podnet-vnet/src/dns.rs` or `tests/`.

## Approach

1. Build one `Resolver` from `--map`. Pass it as `StackConfig.dns` (switch path) and to the door through `hosts_pairs()`.
2. `net shim --map NAME=ADDR` (repeatable) fills `ShimOptions::hosts`.
3. `--map-file PATH` with `Resolver::from_lines`. Validate each address against the subnet at parse time.
4. Update the help and `docs/virtual-network.md:241-246`. The use of `upstream` and `catch_all` is a separate ruling: do not wire them here.

## Prove

```sh
cargo test -p podnet --lib vnet::dns::
cargo test -p podnet --test vnet
```

A test answers a DNS query for a mapped name at the gateway address.

## Done

2026-11-09, commit 96c43d0. Resolver::from_maps (crates/podnet-vnet/src/dns.rs) reads --map and --map-file and checks the subnet; net proxy passes it as StackConfig.dns and as the door table; net shim fills ShimOptions::hosts; docs/virtual-network.md updated. Tests: tests/vnet.rs a_mapped_name_is_answered_at_the_gateway, a_map_outside_the_subnet_or_with_a_bad_form_is_refused. check.sh: exit 0. CI: at the merge.

# T-194: A relay `close` aborts the session task of the node (GitHub #391)

**Source:** GitHub #391 (https://github.com/PodUtils/podnet/issues/391)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The `close` control message removes the entry of the session and drops its `AbortHandle` (`src/relay_reverse.rs` lines 448-452 at `013b755`). A dropped handle does not stop the task. If the task waits in `write_all` to a target that does not read (lines 645-653 at `013b755`), it stays with its socket and its queued frames. The binary-frame arm aborts (lines 473-477 at `013b755`).

## Premise

- The comment at `:523-524` says that dropping the sender ends the task. It does not, for a blocked write.
- The same removal at `:573-575` (`OutKind::End`) is safe: the task has returned.
- The test `a_local_side_that_does_not_read_stops_no_other_session` (`tests/relay_reverse.rs:322`) reaches only the binary-frame arm. The fake relay sends `close` at `tests/relay_reverse/fake.rs:176`.
- Trade: an abort drops the target connection with no FIN (reset instead of half-close).

## Approach

1. In the `close` arm take the entry and call `abort()`. Fix the comment.
2. A helper `end_session(&mut sessions, id)` for each removal, so that a removal always means aborted or done.
3. New test `a_relay_close_ends_a_session_whose_target_does_not_read`.

## Prove

```sh
cargo test --test relay_reverse -- a_relay_close_ends_a_session_whose_target_does_not_read
cargo test --test relay_reverse
```

## Done

2026-11-09, commit 699d002 (`T-194: a relay close aborts the session task`). The `close` arm calls `end_session` (src/relay_reverse/session.rs, which went to `pod-relay` of PodUtils/podcrates): it removes the entry and aborts the task. The binary-frame arm and the `End` arm use the same helper. The session code of the node moved to src/relay_reverse/session.rs; `src/relay_reverse.rs` has 973 lines (was 1139). Test: `a_relay_close_ends_a_session_whose_target_does_not_read` (`tests/relay_reverse.rs`, a unix target with a fixed buffer): it fails with no abort (the target reads 3145728 of 3145728 bytes), and passes with it. `scripts/agent/check.sh` exits 0. CI: at the merge.

# T-195: `relay stop` checks the pair before it stops the local node (GitHub #392)

**Source:** GitHub #392 (https://github.com/PodUtils/podnet/issues/392)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`relay stop NAME` stops each local instance of the pair, then calls the relay (`src/cli/cmd/relay.rs:233-257`). `reverse::stop` refuses an empty stop token (`src/relay_reverse.rs:146-148`). The command then prints one line per stopped instance and exits 69. The pair file stays and the relay keeps the pair for up to 72 h. A pair with no stop token is a valid state (`src/relay_reverse.rs:55,111`, fixture at `tests/relay_reverse.rs:24-33`).

## Premise

- The local-first order has a good reason (`src/cli/cmd/relay.rs:242-243`): a stopped node does not connect again.
- Read only. No run.

## Approach

1. Before the first local stop, check that `pair.stop_token` is not empty and that the relay URL parses. Same faults and messages as now.
2. When the relay call fails after the local stop, say that the relay side is live, give the time left (`relay_reverse::left`) and the pair file path.
3. A retry of the POST on a transport error or 5xx is optional (short, capped). Skip it if it grows the file.

## Prove

```sh
cargo test --test relay_reverse relay_stop
```

A test: a pair with an empty stop token stops no instance, exits 69 and keeps the file.

## Done

2026-11-09, commit 77a70f6 (`T-195: relay stop checks the pair before it stops the local node`). `relay stop` parses the relay URL (78) and checks the stop token (69) before the first local stop (`src/cli/cmd/relay.rs`). A failed relay call after the local stop says that the relay side is live, gives the time left and the pair file path. The retry of the POST is skipped (optional). Test: `relay_stop_checks_the_pair_before_it_stops_an_instance` (`tests/relay_reverse.rs`; a live planted instance and a pair with no stop token): it fails before the fix (the instance is stopped), and passes after it. `scripts/agent/check.sh` exits 0. CI: at the merge.

# T-196: The relay node drains and waits after a local end of stream (GitHub #393)

**Source:** GitHub #393 (https://github.com/PodUtils/podnet/issues/393)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`session` ends when the local read returns 0 (`src/relay_reverse.rs` line 632 at `1a1e729`, select at lines 654-657 at `1a1e729`) and sends `close` at once (line 659 at `1a1e729`). The operator side keeps the session open after its input ends (`EOF_IDLE`, lines 535 and 623 at `1a1e729`). A target that shuts down its write side and still reads loses the rest of the input of the operator. The operator exits 0 (a close with code 1000 is success, line 655 at `1a1e729`). The forward leg has the right shape (`crates/podnet-serve/src/relay.rs:273-307`).

## Premise

- The queued frames in `data_rx` are dropped when the first half ends.
- Whether the relay forwards the operator bytes after the `close` of the node is not known (not in this repository).
- A wait after every target close would add delay to ordinary sessions: the Decision sets a short grace time.

## Approach

1. When the local read returns 0, send no `close`. Keep `to_local` running until the relay closes the id, a write fails, or the grace time of the Decision passes with no frame.
2. Before `close`, write the queued frames to the target.
3. Log the reason (`target closed its write side`).
4. T-087 rewrites this framing: do T-194 to T-197 first, and port the result.

## Decision

The lead, 2026-11-09: the node ends when the relay closes the id, when a write to the target fails, or after about 2 s with no frame from the operator. It does not wait the 15 s of `EOF_IDLE`. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test --test relay_reverse -- a_half_closing_target_gets_the_whole_input
cargo test --test relay_reverse
```

A fake relay and a target that writes, shuts down its write half, then reads to the end: it reads each byte of the operator.

## Done

2026-11-09, commit 0682f98 (`T-196: the relay node waits after a local end of stream`). When the local read returns 0, the session sends no `close` and logs `the target closed its write side`. The write side goes on until the relay closes the id (abort, T-194), a write to the target fails, or no frame comes for 2 s (`HALF_CLOSE_IDLE`, the Decision; src/relay_reverse/session.rs). The queued frames go to the target first. Test: `a_half_closing_target_gets_the_whole_input` (`tests/relay_reverse.rs`): it fails before the fix (the target reads 0 of 300000 bytes), and passes after it. `cargo test --workspace` passes; the other checks of `scripts/agent/check.sh` exit 0. CI: at the merge.

# T-197: The pair file is always mode 0600, and `load` checks the file (GitHub #394)

**Source:** GitHub #394 (https://github.com/PodUtils/podnet/issues/394)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`save` opens the pair file with `.mode(0o600)` (`src/relay_reverse.rs` lines 149-154 at `ed79896`). The kernel applies the mode only to a new file. An old file of mode 0644 keeps it, and `relay pair` prints `mode 0600` (`src/cli/cmd/relay.rs:105`). `save` follows a symlink. `load` uses `read_to_string` (line 172 at `ed79896`) with no check of type, owner, mode or size. The file holds three tokens. The token cache has all four checks and a rename write (src/relay_pool.rs lines 163-195 at `ed79896`; the file went to `pod-relay` of PodUtils/podcrates).

## Premise

- `state::replace_file(path, bytes, mode)` writes through a rename, sets the mode and replaces a symlink (`crates/podnet-core/src/state.rs:344-363`).
- `read_private` returns `Option` and logs the reason: `load` needs an error with a `Fault::Config` class (`src/relay_reverse.rs` lines 151-188 at `ed79896`).
- `DirBuilder::mode(0o700)` applies only to a new directory (lines 143-146 at `ed79896`).
- The test `a_pair_file_is_private_and_the_newest_is_chosen` is at `src/relay_reverse.rs` line 871 at `ed79896`.

## Approach

1. `save` calls `state::replace_file(.., 0o600)`.
2. `load` applies the four checks of `read_private` (share one function) and fails with a message that names the reason.
3. Refuse a `relay` directory that the group or others can write.

## Prove

```sh
cargo test -p podnet --lib relay_reverse::
```

Tests: a 0644 file becomes 0600 after `save`; a symlink is replaced; `load` refuses a 0644 file, a symlink and an oversize file.

## Done

2026-11-09, commit 7efd1d8 (`T-197: the pair file is always mode 0600, and load checks the file`). `save` writes through `state::replace_file` (rename, mode 0600; a symlink is replaced). `load` reads through `relay_pool::read_private_checked` (shared with `read_private`): type, owner, mode and size; a fault is exit 78 with the reason. `save` and `load` refuse a `relay` directory that others can write. The code is in `src/relay_reverse/pairfile.rs` (`src/relay_reverse.rs`: 899 lines). Tests (`cargo test -p podnet --lib relay_reverse::`): `a_file_of_mode_0644_becomes_0600_after_save`, `save_replaces_a_symlink_and_does_not_write_through_it`, `load_refuses_a_file_that_is_open_a_symlink_or_large`, `a_directory_that_others_can_write_is_refused`. `scripts/agent/check.sh` exits 0. CI: at the merge.

# T-198: `docs/protocol.md` and the test agree on the number of seed alternates (GitHub #395)

**Source:** GitHub #395 (https://github.com/PodUtils/podnet/issues/395)
**Category:** docs
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`docs/protocol.md:209` says the alternates are the first 3 hosts of the pool. The text after it says the default relay uses 6 hosts until a pool is cached. The code takes 3 (`MAX_ALTERNATES`, src/relay_pool.rs lines 39 and 119 at `7c5f87e`; the file went to `pod-relay` of PodUtils/podcrates). The seed has 6 endpoints (`ajam-relay.toml`). A connect with no pool tries the primary and 3 hosts. The stagger is 2 s each (`src/ws_relay.rs:200`).

## Premise

- The test pins the count of alternates only (src/relay_pool.rs line 347 at `7c5f87e`).
- `docs/decisions.md` (2026-11-04, Relay) says only that the alternates are the hosts of the podssh seed.

## Approach

1. Call: 3 stays (code, test and the delay of 6 s at most). Record it in `docs/decisions.md`.
2. Fix the paragraph: the seed holds 6 hosts, the client takes the first 3. Name `MAX_ALTERNATES`.
3. Test: `seed().len() >= MAX_ALTERNATES` (not `== 6`: the seed is data of pod-services).

## Prove

```sh
cargo test -p podnet --lib relay_pool::
cargo todo check
```

## Done

2026-11-09, commit be7b624. `docs/protocol.md` says that the seed holds 6 hosts and the client takes 3 (`MAX_ALTERNATES`). The test `the_alternates_come_from_the_cache_else_the_seed` checks `seed().len() >= MAX_ALTERNATES`. `scripts/agent/check.sh` exits 0. CI at dbfcfd6: ci 42638635724 and live 42638635442 success.

# T-199: `relay connect --listen` has the instance surface of `connect -L` (GitHub #396)

**Source:** GitHub #396 (https://github.com/PodUtils/podnet/issues/396)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`podnet relay connect NAME --listen LISTEN` is a server. It has no `--detach`, `--name`, `--replace`, `--no-restart` or
`--ready-timeout`. `podnet ps` and `podnet stop` cannot see it. An accept error ends it with exit 1 (no supervisor can
restart it). The unix socket file stays after exit. The refusal text of `claim_socket_path` names `podnet ps`,
`podnet stop NAME` and `--replace`, and none of them works for this command. `connect -L` has all of this (issues 202,
207).

## Premise

- `RelayCommand::Connect` has no `DetachArgs`: `src/cli/cmd/relay.rs:49-70`. `RelayCommand::Node` has it: `:44-45`.
- `detach_of` has no arm for it: `src/cli/opts.rs:260-269`. `connect` has one: `:251`.
- Both accept loops use `accepted?`: `src/cli/cmd/relay.rs:188`, `:200`. `connect -L` logs and returns:
  `src/cli/cmd/connect.rs:208-211`.
- The unix branch calls `claim_socket_path(&path, false)` with no guard: `src/cli/cmd/relay.rs:194`.
  `SocketFile` is in `src/cli/cmd/connect.rs:219-239` (`pub(crate)`).
- Message: `crates/podnet-core/src/net.rs:27-33`.
- `src/main.rs:141` allows `--replace` without `--detach` only for `Command::Connect`.

## Approach

1. Flatten `DetachArgs` into `Connect`. Add the arm to `detach_of`. Allow `--replace` as for `connect -L` in `main.rs`.
   The flags have no effect without `--listen`.
2. On accept error: warn and return (as `connect -L`), so a supervisor restarts the process.
3. Use `SocketFile` for the unix listener. Pass the real `replace` to `claim_socket_path`.
4. Change the `net.rs` message to name only remedies that exist for the caller.
5. Do not change the stdin and stdout mode.

## Prove

`cargo test -p podnet --test cli relay_connect_listen_detach` (new test: `--detach --name fwd` starts, `ps --json` lists
`fwd`, `stop fwd` ends it).
`cargo test -p podnet relay_connect_unix_socket_removed` (new test: the socket file is gone after a clean stop).
`cargo run -q -- relay connect x --listen tcp://127.0.0.1:0 --detach --help` exits 0 (flag exists).

## Done

2026-11-09, commit 456c7d9 (`T-199: relay connect --listen has the instance surface of connect -L`). `relay connect` flattens `DetachArgs` (the positional is the field `pair`: the id `name` clashed with `--name`); `detach_of` has an arm that applies only with `--listen`; `--replace` works as for `connect -L` (`replaces_a_socket` in `src/cli/opts.rs`). An accept error ends the command with exit 69 (a supervisor starts it again). The unix listener uses `connect::unix_listener` (`SocketFile` removes the file at the end; the real `replace` reaches `claim_socket_path`). The ready document has `pair` and `listen`. A detached start moves `--token` of `relay connect` to `PODNET_RELAY_CONNECT_TOKEN` in the secret file (`split_secrets`, `SECRET_VARIABLES`); before, it went to `TUNNEL_TOKEN` and the child had no token. The message of `claim_socket_path` stays: `podnet ps`, `podnet stop NAME` and `--replace` now exist for this caller. The tests are in the new file `tests/relay_listen.rs` (`tests/cli/main.rs` is in the size allowlist): `relay_connect_listen_detach` (start with `--detach --name fwd`, `ps --json` lists `fwd`, no token in the record, `stop fwd` ends it) and `relay_connect_unix_socket_removed` (the socket file is gone after SIGTERM); both fail before the change. `relay connect x --listen tcp://127.0.0.1:0 --detach --help` exits 0. `cargo test --workspace --no-fail-fast` passes; the other checks of `scripts/agent/check.sh` exit 0. CI: at the merge.

# T-201: `WebSocketTunnel` dials through the egress layer and parses IPv6 (GitHub #398)

**Source:** GitHub #398 (https://github.com/PodUtils/podnet/issues/398)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`WebSocketTunnel::connect` opens TCP with `Direct::new().connect`. `Direct` uses the system resolver only: it ignores
`--proxy`, `NO_PROXY`, `--resolve` and DoH. `parse_ws_url` has no bracket handling, so `wss://[::1]:8443/ws` fails. The
TLS config is `util::tls::client_config(..)` with no ALPN pin. The trait arguments `host` and `port` are dropped. No
command builds the type (latent).

## Premise

- `src/transport/websocket.rs:181-185` (direct dial, `host` and `port` dropped), `:142-150` (`rsplit_once(':')`),
  `:193-197` (TLS).
- The precedent: `src/ws.rs:120-123`, `:155` (`crate::egress::dial`), `:161` (ALPN `http/1.1`);
  `src/ws_relay.rs` lines 605-619 at `98d8a71`; `Host::parse`, lines 137-149 at `98d8a71`.
- `grep -rn WebSocketTunnel src` finds only the module and one test (`src/transport/mod.rs:474`).

## Approach

1. Replace the direct dial with `crate::egress::dial(&url_host, url_port, None, timeout)`. Map the error to
   `TransportError::Unreachable`.
2. Strip the brackets of an IPv6 host for the dial and the server name. Add them back in the `Host` header.
3. Build the TLS config from `crate::trust::context()` with ALPN `http/1.1`, as `ws.rs` does.
4. Document in the type that the destination is its URL (the addressing mode is #377, outside this entry).

## Prove

`cargo test -p podnet --lib transport::websocket` (new tests: `wss://[::1]:8443/ws` gives host `::1`; a loopback server
with a refusing proxy shows that the proxy is asked).
`cargo test -p podnet --lib websocket_transport_reports_its_name`.

## Done

2026-11-09, commit 35d2a3b. `WebSocketTunnel::connect` dials with `crate::egress::dial` (errors map to `Unreachable`), takes the TLS config of `crate::trust::context()` with ALPN `http/1.1`, parses `wss://[::1]:8443/ws` (host `::1`) and brackets an IPv6 host in the Host header. Tests: `transport::websocket::tests::parses_an_ipv6_host`, `websocket_transport_reports_its_name`. The test with a refusing proxy is not written: the proxy choice of the egress layer is process-global state. CI: at the merge.

# T-202: `serve` of a directory with no `index.html` logs no WARN for the 404 at `/` (GitHub #399)

**Source:** GitHub #399 (https://github.com/PodUtils/podnet/issues/399)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`Show::root_404` makes the verify accept a 404 at `/` as the normal answer (issue 119, commit 0775407). No code sets it
to true. Each `serve --dir DIR` of a directory with no `index.html` logs `WARN ... the origin answered HTTP 404`.

## Premise

- `grep -rn root_404 src` gives 3 lines: `src/cli/opts.rs:115` (field), `:125` (`root_404: false`),
  `src/cli/cmd/run.rs:44` (read).
- `serve` passes `args.common.show()`: `src/cli/cmd/serve.rs:201`.
- The warn branch: `src/tunnel.rs:1266-1270`. `static_router` gives 404 at `/` with no index: `crates/podnet-serve/src/files/mod.rs:73-137`.
- `src/config.rs` is in the size allowlist: do not add the field there.

## Approach

1. In `serve()` (share.rs), set `show.root_404 = !args.spa && !args.dir.join("index.html").is_file()` on a local copy of
   `Show`. Pass it to `preflight` and `run_tunnel`.
2. Do not touch `config.rs`. (`tunnel --config` with a static catch-all stays out of scope.)

## Prove

`cargo test -p podnet --lib serve_show_root_404` (new test: a helper that builds the `Show` for an empty directory gives
true; for a directory with `index.html` gives false).
`cargo test -p podnet --lib verify_accepts_404_at_info` (new test: a local server with status 404 and `accepted=&[404]`
returns `Ok`).

## Done

2026-11-09, commit 7074262. `serve_show` (`src/cli/cmd/share.rs`) sets `root_404` for a directory with no `index.html` and no `--spa`; `serve` and `--check` use it. Test: `serve_show_root_404` (bin tests). The test `verify_accepts_404_at_info` is not added: `tunnel::verify::tests::an_accepted_status_of_the_origin_verifies` covers it. CI: at the merge.

# T-203: The ready line of `share --json` has `schema` and `podnet` (GitHub #400)

**Source:** GitHub #400 (https://github.com/PodUtils/podnet/issues/400)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Each JSON document of podnet has the keys `schema` (1) and `podnet` (`status::stamped`, `src/status.rs:119-127`).
`run_share` prints its ready document with a bare `serde_json::json!`.

## Premise

- `src/cli/cmd/run.rs:95-105` (no `stamped`). The only `stamped` call of `run.rs` is `:292` (`announce`).
- `grep -rn "status::stamped" src/cli` gives 8 hits, none in `run_share`.
- The `verified` line is unstamped in every command (`run.rs:119`, `:167`). It stays as it is.

## Approach

1. Move the document to a helper `share_ready_doc(..) -> Value` and wrap it with `podnet::status::stamped`.
2. Add a unit test of the helper. Keep the `verified` line as it is.

## Prove

`cargo test -p podnet --lib share_ready_doc_is_stamped` (new test: `schema == 1`, non-empty `podnet`).
`cargo test -p podnet` (the other documents keep both keys).

## Done

2026-11-09, commit 6197ff3. `share_ready_doc` (`src/cli/cmd/run.rs`) wraps the ready line with `status::stamped`. Test: `share_ready_doc_is_stamped` (bin tests). CI: at the merge.

# T-204: `tunnel --config` refuses the origin flags (GitHub #401)

**Source:** GitHub #401 (https://github.com/PodUtils/podnet/issues/401)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

With `--config FILE`, the code skips the flag-built ingress. `--forward`, `--url`, `--unix`, `--port`, `--tcp`,
`--hello-world`, `--no-tls-verify`, `--http-host-header` and `--strip-accept-encoding` have no effect, and nothing says
so. The README (`:307-362`) documents the precedence of the token and protocol flags only.

## Premise

- Guard: `src/cli/cmd/tunnel.rs:136` (`if args.config.is_none()`). The flags are read at `:138-162`, `:245`.
  The three per-origin flags are declared at `:62-73`.
- The token case already has a usage error: `src/cli/cmd/tunnel.rs:111-127` (issue 172).
- Existing tests with `--config`: `a_bad_configuration_exits_78` and `bad_credentials_exit_78` (`tests/cli/exit_codes.rs`), `check_mode` and `the_config_file_gives_the_protocol_and_the_connections` (`tests/cli/startup.rs`) (none gives an origin flag).

## Approach

1. Make the combination a usage error (exit 64) that names each flag given and points to `ingress:` of the file.
   (see Decision.)
2. Say in the help text of each flag that it does not apply with `--config`. Add one sentence to the README section.

## Decision

The lead, 2026-11-09: `tunnel --config` with an origin flag is refused with exit 64. A silent drop is the defect. The owner can rule otherwise (`docs/decisions.md`).

## Prove

`cargo test -p podnet --test cli tunnel_config_with_origin_flag_is_usage_error` (new test: `--config F --port 8080`
exits 64 and names `--port`; same for `--forward` and `--no-tls-verify`).
`cargo test -p podnet --test cli` (the `--config` tests without an origin flag still pass).

## Done

2026-11-09, commit 71a0afe. `tunnel --config` with an origin flag exits 64 and names each flag given and `ingress:`. The help of each flag and the README section say it. Tests: `tunnel_config_with_origin_flag_is_usage_error` (`tests/tunnel_config.rs`: `--port`, `--forward`, `--no-tls-verify`), `origin_flags_are_named` (bin tests). The existing `--config` tests pass. CI: at the merge.

# T-205: A nested detached start does not inherit `PODNET_DETACH_SUPERVISED` (GitHub #402)

**Source:** GitHub #402 (https://github.com/PodUtils/podnet/issues/402)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`start_inner` copies each `PODNET_*` variable of its environment into the child. The copy includes
`PODNET_DETACH_SUPERVISED`. A detached child that starts a detached instance with `--no-restart` makes a grandchild that
takes the child lock and writes a record with `supervisor: None`. `targets()` then returns an empty list. `podnet stop`
sends no SIGTERM, waits the full timeout and sends SIGKILL.

## Premise

- `crates/podnet-core/src/detach.rs:166-192` (`child_begin` reads the variable). `:534-541` sets 4 detach variables and not this one.
  `crates/podnet-core/src/detach/supervise.rs:120` is the only writer (the supervisor).
- `crates/podnet-core/src/autostart.rs:178-182` copies each `PODNET_` variable except `PODNET_ERROR_PROGRAM`.
- `crates/podnet-core/src/detach.rs:712-719` (`targets()` gives `Vec::new()` for this record).
- The default start (restart on) runs `--detach-supervise`: the supervisor sets the variable for its own child, so that
  path is right. The defect needs `--no-restart`. I did not run it.
- `crates/podnet-core/src/detach.rs` is in the size allowlist (1210 lines): add no net line.

## Approach

1. Add `.env_remove(SUPERVISED_VARIABLE)` in `start_inner` (offset by removing a line).
2. Exclude `PODNET_DETACH_SUPERVISED` in `split_secrets` (as `PODNET_ERROR_PROGRAM`).
3. Add a test of the argv and environment of the child, and a test of `targets()` for "no supervisor, instance lock free,
   child lock held".

## Prove

`cargo test -p podnet --lib detach::` (new tests: `child_env_has_no_supervised_variable`,
`split_secrets_skips_supervised_variable`).
`cargo test -p podnet --test detach` (existing tests pass).

## Done

2026-11-09, commit e4ad540. `child_command` (`crates/podnet-core/src/detach/spawn.rs`, moved out of `start_inner`) calls `env_remove(PODNET_DETACH_SUPERVISED)`; `kept_variable` keeps it out of an autostart entry (`split_secrets`). `crates/podnet-core/src/detach.rs` is smaller by 19 lines. Tests: `detach::spawn::tests::child_env_has_no_supervised_variable`, `split_secrets_skips_supervised_variable`. The test of `targets()` for a record with no supervisor is not written. CI: at the merge.

# T-206: `agent status --json` (GitHub #403)

**Source:** GitHub #403 (https://github.com/PodUtils/podnet/issues/403)
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`agent status` prints text only. `agent up` and `doctor --agent --json` print stamped JSON. `Status` has no fields, so
`podnet agent status --json` exits 64. The report type already derives `Serialize`.

## Premise

- `src/cli/cmd/agent.rs:14-35` (`Status` with no fields), `:205-211` (`instances_ps(false)`, `agent::text`).
- `src/cli/cmd/instances.rs:41-47` (`instances_ps(json)` prints its own document).
- `src/cli/cmd/host.rs:245-255` (stamped report). `src/agent.rs:143-145` (`Serialize`). `docs/proxies.md:61-67`.
- `T-011` (`TODO/record.md:220`) says `agent status` has no JSON form; its test is `tests/commands.rs:143`.

## Approach

1. Add `#[arg(long)] json: bool` to `Status`.
2. With `--json`, print one stamped document `{"instances": [...], "report": {...}}`. Share the list builder with
   `instances_ps` (do not call it: it prints). The text form stays as it is.
3. Exit code stays 0 (see Decision).
4. Add the flag to the README.

## Decision

The lead, 2026-11-09: `agent status --json` exits 0. The state, with the result of each task, is in the JSON. A change of the exit code can come later when a user asks. The owner can rule otherwise (`docs/decisions.md`).

## Prove

`cargo test -p podnet --lib agent_status_accepts_json_flag` (new test: `Cli::try_parse_from(["podnet","agent","status","--json"])`).
`cargo test -p podnet --test commands agent_status` (new JSON test: `schema == 1`, keys `instances` and `report`; the text test passes).

## Done

2026-11-09, commit 25435fb. `agent status --json` prints `{schema, podnet, instances, dir, report}` and exits 0; `instances_doc` (`src/cli/cmd/instances.rs`) is shared with `ps --json`. The text form stays. The README names the flag. Tests: `agent_status_accepts_json_flag` (bin tests), `agent_status_json_has_the_instances_and_the_report` (`tests/commands.rs`). CI: at the merge.

# T-207: The library static path decodes `+` as a literal plus (GitHub #404)

**Source:** GitHub #404 (https://github.com/PodUtils/podnet/issues/404)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`origin::static_files::percent_decode` turns `+` into a space. `service::percent_decode_path` (the CLI), and
`ingress::decode_path`, keep `+`. A `+` in a URL path is a literal plus. The README says both paths follow the same rules.

## Premise

- `crates/podnet-serve/src/origin/static_files.rs:256-258` (the `b'+'` arm), `:84-89` (`resolve`), test `:603-607` asserts `"a b"`.
- `crates/podnet-serve/src/files/mod.rs:257-272`, `:170`. `crates/podnet-serve/src/ingress.rs:200-203`, test `:635-648` (`"a + stays a +"`).
- `docs/serve.md:54-55`.
- One more divergence (not in the issue): a bad escape (`%zz`) passes through in the library decoder
  (`crates/podnet-serve/src/origin/static_files.rs:242-260`, test `:616-618`). The CLI decoder returns `None` (`crates/podnet-serve/src/files/mod.rs:262-265`), so the request is refused. A short escape at the
  end (`i + 2 < len`) also differs.

## Approach

1. Remove the `+` arm. Change the test to assert `"a+b"` and `"a b"` for `%20`.
2. Add a test that sends the same path to `StaticDir::respond` and `static_router` over one file set (a file `a b.txt`
   and a file `a+b.txt`).
3. Decide the bad-escape case in the same test. Recommendation: the library refuses it as the CLI does.

## Prove

`cargo test -p podnet --lib origin::static_files::tests::percent_decoding` (changed test).
`cargo test -p podnet --lib static_paths_agree_on_plus` (new test).

## Done

2026-11-09, commit 76ec99a. `origin::static_files::percent_decode` keeps `+`; `resolve` refuses a bad escape (`%zz`, a short `%`), as `service::percent_decode_path` does. Tests: `origin::static_files::tests::percent_decoding`, `a_plus_in_a_path_is_a_plus` (the files `a b.txt` and `a+b.txt`), `service::tests::static_paths_agree_on_plus` (the two decoders). CI: at the merge.

# T-208: `warp up --tun` forwards `--replace` and refuses the flags it cannot forward (GitHub #405)

**Source:** GitHub #405 (https://github.com/PodUtils/podnet/issues/405)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet warp up --tun` builds the argv of `warp tun --default-route --detach`. It does not pass `--replace`. A second
`warp up --tun --replace` with other arguments gets the refusal "use --replace" and exit 69. `--env-d`, `--dns`,
`--proxy-user` and `--proxy-password-file` are also dropped with exit 0 (`--persist` has `conflicts_with`).

## Premise

- `src/cli/cmd/warp/up.rs:19-29` (the `tun: true` arm matches `..`), `:425-437` (`warp_up_tun_args`), `:229-233` (the
  proxy path pushes `--replace`).
- `src/cli/cmd/warp/args.rs:391-435` (the flags), `:404` (`conflicts_with = "persist"` on `tun`), `:529-544` (`Tun` has
  `DetachArgs`, so `--replace` exists there, and has no `--dns`).
- `crates/podnet-core/src/detach.rs:509-516` (the refusal text).

## Approach

1. Pass `replace` into `warp_up_tun_args`. Push `--replace` when set.
2. Add `conflicts_with = "tun"` to `env_d`, `dns`, `proxy_user` and `proxy_password_file` (they describe a proxy; the
   tunnel has none). Do not add a feature to forward them.
3. Test the full argv of `warp up --tun` for each flag set.

## Prove

`cargo test -p podnet --lib warp_up_tun_args` (new tests: `--replace` present only when set; `Cli::try_parse_from` of
`warp up --tun --env-d` is a clap error).

## Done

2026-11-09, commit dd02a91 (`T-208: warp up --tun forwards --replace and refuses the proxy flags`).
`warp_up_tun_args` takes `replace` and pushes `--replace` when set. `--env-d`, `--dns`,
`--proxy-user` and `--proxy-password-file` of `warp up` have `conflicts_with = "tun"`. Tests (in
`src/cli/cmd/warp/up_tests.rs`): `warp_up_tun_args_replace_only_when_set`,
`warp_up_tun_refuses_the_proxy_flags`. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-209: `warp status` names the `warp-tun` instance (GitHub #406)

**Source:** GitHub #406 (https://github.com/PodUtils/podnet/issues/406)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`warp up --tun` starts the instance `warp-tun`. `warp status` reads the instance `warp` only. On a host that runs the
tunnel, it prints `warp up     not running (podnet warp up starts it)` and `instance: {"running": false}`. A script
then restarts a working tunnel. `agent status` has the same text (`src/agent.rs:538`).

## Premise

- `src/warp/live/instance.rs:70-76` (`instance_state` reads `INSTANCE`), `:88-105` (`instance_state_in` takes a name),
  `:131-158` (the line). `src/warp/up.rs:24-26` (two names), `:529-534` (`down` knows both).
- `src/cli/cmd/warp/account.rs:32-45` (the arm `Status` of the account file).
- `warp tun` has no `Live` and so no `data_path` (the issue notes it; not part of this entry).

## Approach

1. Add `instance_state_tun()` (name `TUN_INSTANCE`). Add a key `tun_instance` to the JSON beside `instance` (additive).
2. The text line names each running instance. When only the tunnel runs, it points to `podnet warp down`.
3. Do the change after T-031, or keep the edit in `main.rs` to two lines.

## Prove

`cargo test -p podnet --lib warp::live::instance` (new tests: `instance_state_in(dir, "warp-tun")` is running with a tun
record; the line for neither, proxy only, tun only).

## Done

2026-11-09, commit 57ff81c (`T-209: warp status names the warp-tun instance`). `instance_state_tun`
(`src/warp/live/instance.rs`) reads the instance `warp-tun`. `warp status --json` has the key
`tun_instance` beside `instance`. `instances_line` names each running instance and, with only the
tunnel, points to `podnet warp down`. Test: `warp_status_names_the_tunnel_instance`. `agent
status` keeps its text (not part of this fix). `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-210: The WARP+ label names the paid account types (GitHub #407)

**Source:** GitHub #407 (https://github.com/PodUtils/podnet/issues/407)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** blocked

## Problem

`warp_plus_label` gives `yes` for each non-empty `account_type` that is not exactly `free`, or when `premium_data > 0`.
`warp status`, `warp status --json` (`warp_plus_active`) and `warp license` use it. The issue says a free account with
another type is shown as WARP+.

## Premise

- `src/warp/identity.rs:157-171`. Users: `src/cli/cmd/warp/account.rs:35`, `:53-56`, `:88-91`. Test: `src/cli/cmd/warp/cli_tests.rs:128-131`.
- The issue says the fixture type `limited` is a free type. This is not shown: `src/warp/register.rs:627` is a test of
  the redaction of a licence key, and the test at `main.rs:894` uses `limited` with `premium_data` 10 as WARP+.
  The tree has no list of the types that the service sends. A closed list of paid types could hide a real paid account.
- Earlier report: `docs/reports/aerrand-agent.md:639` (the raw `warp_plus` field).

## Approach

1. Measure first: run `podnet warp status --json` on a free device and, when possible, on one with a licence. Record the
   `account_type` and `premium_data` values in `docs/reports/`.
2. Then name the paid types from the measurement. Keep `premium_data > 0` as sufficient. Add a test for each value
   measured and one for an unknown type (decide its result with the measurement).

## Prove

`cargo test -p podnet --lib warp_plus_label` (new cases from the measured values).
`podnet warp status --json | jq '.account_type, .premium_data, .warp_plus_active'` on a live device (the measurement).

## Blocker

The owner, 2026-11-11 (`docs/decisions.md`, Deferred and blocked work): the measure needs a device with a paid licence
(one `podnet warp status --json` of a free device and one of a licensed device). A mock cannot give the
`account_type` values of the live service.

## Correction

2026-11-09: the measurement of Approach 1 is not possible in this session. No record of the `account_type` values of the live service exists in the tree: `grep -rn account_type docs tests src` finds the report line `docs/reports/aerrand-agent.md:639` (type `free` with `warp_plus` true), the fixtures `src/warp/register.rs:627` and `src/cli/cmd/warp/cli_tests.rs:130` (`limited`, made up), and no list of the paid types. A registration spends a device and needs the live service (`docs/decisions.md`, WARP devices). The code stays as it is. Start condition: one `podnet warp status --json` of a free device and one of a device with a licence, recorded in `docs/reports/`.

# T-211: `warp down` and `warp up --persist` own the fish file only when podnet wrote it (GitHub #408)

**Source:** GitHub #408 (https://github.com/PodUtils/podnet/issues/408)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`warp down` removes `~/.config/fish/conf.d/podnet-warp.fish` with no content check, and prints "removed". `persist`
overwrites that path with `std::fs::write`. The environment.d file is checked (`env_d_is_ours`), the shell files keep the
text outside the markers.

## Premise

- `src/warp/up.rs:551-553` (remove), `:463-470` (write), `:424-430` (`fish_file`), `:434-444` (`fish_block` starts with
  the shared `BEGIN` marker), `:495-497` and `:554-567` (the model).
- `src/warp/up.rs` has 847 lines (allowlist): put the new code in a new small file, or remove lines elsewhere.

## Approach

1. A fish file is ours when its first line is `BEGIN` (already true for each file that earlier builds wrote: no new mark,
   no stale file). The issue proposes a new mark. That mark would leave the old files on disk.
2. `down`: remove only such a file. Else print that it is not a file of podnet and keep it.
3. `persist`: refuse to overwrite a file that is not ours, with an error that names the path. Run the check before any
   other write (as `check_env_d`).

## Prove

`cargo test -p podnet --lib warp::up` (new tests: `fish_file_is_ours` for the block text, an empty text, a user text).
A shell probe with `HOME=$(mktemp -d)`: put a user file at the fish path, run `podnet warp down`, and check the file stays.

## Done

2026-11-09, commit 9ece848 (`T-211: warp down and warp up --persist own the fish file only when podnet wrote it`). A fish file is ours when its first line is the marker. `down` keeps another file and says so. `persist` refuses it, with the path, before any write. The code is in `src/warp/up/fish.rs` (`src/warp/up.rs` 847 to 817 lines). Tests: `warp::up::fish::tests::{the_fish_block_loads_the_env_file, fish_file_is_ours_by_its_first_line, a_user_fish_file_stays}`. Probe with a temporary HOME: a user file at the fish path stays after `podnet warp down`. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-212: The README states that the rules apply to `warp proxy --l4` (GitHub #409)

**Source:** GitHub #409 (https://github.com/PodUtils/podnet/issues/409)
**Category:** docs
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`docs/warp.md:95-100` says: "UDP (SOCKS5 UDP ASSOCIATE) is not served, and the rules do not apply." The code applies
`-r`, `--rules-file` and `--rule-default` to `--l4`. The `--help` text says so.

## Premise

- `docs/warp.md:91-100`. `src/warp/proxy.rs:507-527` (the dial closure: `rules::current()`, `decide_static`, three actions).
- `src/cli/cmd/warp/args.rs:340-344` (help). `src/warp/rules.rs:304-309`, tests `:316-331`.
- `TODO/deferred.md:91` (T-002) says the rules apply from commit db6dbe6.
- No other file says that the rules do not apply: `grep -rn "rules do not apply" README.md docs` finds none.

## Approach

1. Change the README sentence: UDP is not served; the rules apply; a `cidr:` rule decides an address, a name with no
   other rule takes `--rule-default`.
2. Do not change T-002 (the feature stays blocked).

## Prove

`grep -n "rules do not apply" README.md` prints nothing.
`cargo test -p podnet --lib warp::rules` (the existing `decide_static` test passes).

## Done

2026-11-09, commit be7b624. The README says that the rules apply to `warp proxy --l4`. `grep -n "rules do not apply" README.md` prints nothing. Test: `warp::rules` lib tests. `scripts/agent/check.sh` exits 0. CI at dbfcfd6: ci 42638635724 and live 42638635442 success.

# T-213: The WireGuard endpoint of a device record has one derivation (GitHub #410)

**Source:** GitHub #410 (https://github.com/PodUtils/podnet/issues/410)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`warp export` reads `host`, then `v4` (`record_endpoint`). The WireGuard leg reads `v4` only (`Peer::from_exported`).
A record with only `host` exports and does not connect. A record with only `v6` fails in both, though MASQUE dials it.
`Endpoint.ports` is read nowhere.

## Premise

- `src/warp/register.rs:185-194` (`Endpoint`; frozen file: no edit). `src/warp/export.rs:245-263`, `:30` (`WG_PORT`).
- `src/warp/wg.rs:61-68` (v4 only; the message `:64-66`).
- MASQUE reads `v4` and `v6`: `src/warp/register.rs:558-597`.
- `grep -rn "\.ports" src/warp` finds no reader.
- Test to extend: `src/warp/export.rs:402-437`.

## Approach

1. One function `wireguard_endpoint(&Endpoint, Option<SocketAddr>)` in `export.rs`. Order: `host`, `v4`, `v6` (brackets
   for `v6`). Port 2408. Both callers use it. The failure names the fields that were present.
2. Do not read `ports` in this entry. Measure first what a real record holds (a list of ports, and which one WireGuard
   uses). Open a new entry if it differs from 2408.
3. Test: `Config::of` and `Peer::from_exported` give the same address for 4 record shapes.

## Prove

`cargo test -p podnet --lib warp::export` (new tests: host only, v4 only, v6 only, none).
`cargo test -p podnet --lib warp::wg` (the existing tests pass).

## Done

2026-11-09, commit 23cb347 (`T-213: the WireGuard endpoint of a device record has one derivation`). `src/warp/export.rs` has one list of candidates: `host`, `v4`, `v6` (port 2408; brackets for `v6`). `wireguard_endpoint` (`warp export`) takes the first; `wireguard_socket_addr` (`Peer::from_exported`) takes the first literal address, else the first name that the system resolver gives. A record with only `host` or only `v6` now connects. A record with a name and a `v4` keeps its old results (the export names the host; the leg takes the address with no lookup). A failure names the fields that the record has. `Endpoint.ports` is not read: not measured (no live record of the service in this session); no new entry opened. Test: `warp::export::tests::the_two_wireguard_readers_agree_on_the_record_shapes`; `cargo test -p podnet --lib warp::wg` passes. `src/warp/register.rs` is unchanged. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-214: `warp ssh-config` prints a `ProxyCommand` with the same quote as `--write` (GitHub #411)

**Source:** GitHub #411 (https://github.com/PodUtils/podnet/issues/411)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`nc::ssh_config` (the printed block) puts the binary path in `ProxyCommand` with no quote. `ssh_config::block` (`--write`)
quotes it with `shell_quote`. A path with a space works with `--write` and fails when pasted.

## Premise

- `src/warp/nc.rs:301-310` (raw `{exe}` at `:292`). `src/warp/ssh_config.rs:34-36`, `:65-73`.
- The only test of the printed block: `src/warp/nc.rs:363-367` (plain path).
- The issue says the double quotes of the `GIT_SSH_COMMAND` comment are right. They are not: the shell that runs
  `ProxyCommand` splits the path at a space inside `"..."` of the outer option too. The comment needs the quote of the
  path inside the value (`ProxyCommand='...'` with the `'\''` form).

## Approach

1. Move the `ProxyCommand` line into one function that both builders call (`shell_quote` of the path).
2. Fix the `GIT_SSH_COMMAND` comment for a path with a space or a quote.
3. Keep the printed block without markers.

## Prove

`cargo test -p podnet --lib warp::nc` (new test: for `/b/podnet`, `/opt/c f/podnet` and a path with a quote, the printed
`ProxyCommand` line equals the line of `ssh_config::block`).

## Done

2026-11-09, commit 3f9c205 (`T-214: warp ssh-config prints a ProxyCommand with the quote of --write`). `ssh_config::proxy_command` gives the command line to both builders (`--write` and the printed block). The `GIT_SSH_COMMAND` comment is quoted in two levels (the shell that reads the line, and the shell that runs `ssh`). Test: `warp::nc::tests::the_printed_block_quotes_the_path_as_the_written_block` (`/b/podnet`, `/opt/c f/podnet`, `/o'q/podnet`; the export line runs through `sh` and gives back the path). `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-215: `warp proxy --dns` has a bound and a counter (GitHub #412)

**Source:** GitHub #412 (https://github.com/PodUtils/podnet/issues/412)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`serve_dns` spawns one task for each datagram: no limit, no counter, and the length check is inside the task. While no
session is up, each task waits in `live.get()` for up to 20 s. `--max-connections` and the health line do not cover it.

## Premise

- `src/warp/proxy.rs:193-215` (`serve_dns`), `:137-148` (`admit`), `:158-185` (`health_log`), `:108-123` (`Stats`),
  `:381-383` (length check in `forward_dns_to`).
- `src/warp/live.rs:25` (`WAIT_FOR_NEW` 20 s).
- `src/warp/proxy.rs` has 1902 lines (allowlist): put the new code in a new file.

## Approach

1. A separate constant bound for DNS tasks (a few hundred), with a semaphore. Drop the excess. Count the drops in `Stats`
   and print them in the health line.
2. Drop a datagram under 12 bytes before the spawn, with a debug line.
3. Keep one task for each query and the first answer wins.

## Prove

`cargo test -p podnet --lib warp::proxy::dns_bound` (new test: a burst larger than the bound gives at most the bound in
flight and a count of drops; a 1-byte datagram spawns nothing).

## Done

2026-11-09, commit be1c8dc (`T-215: warp proxy --dns has a bound and a counter`). `serve_dns` moved to `src/warp/proxy/dns_bound.rs` (`src/warp/proxy.rs` 1902 to 1878 lines). A semaphore of 256 places (`MAX_DNS_TASKS`) bounds the queries in flight; an excess datagram is dropped and counted in `Stats.dns_dropped`, which the health line prints. A datagram under 12 bytes starts no task. One task for each query; the first answer wins. Tests: `warp::proxy::dns_bound::tests::{a_burst_gives_at_most_the_bound_in_flight, a_short_datagram_starts_nothing}`. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-225: The send poller of the WARP QUIC socket keeps its wake-up

**Source:** The podcrates session (PodUtils/podcrates T-019, pod-quic), 2026-11-10: a finding in
`src/warp/quic.rs`, read from the behaviour of tokio.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`SendPoller::poll_writable` (`src/warp/quic.rs`) makes a new `Notify::notified()` future on each call and
drops it when the call returns `Pending`. tokio removes a dropped `Notified` from the list of waiters, so the
`notify_waiters` of the writer task wakes nobody. When the send queue of the QUIC socket is full, quinn waits for
a wake-up that does not come: the QUIC send path of a tunnel over WARP can stall until some other event polls it.

## Premise

- Read: `src/warp/quic.rs`, `SendPoller::poll_writable` (the `notified` future is local to the call; the
  re-check of the capacity covers only a drain between the check and the registration) and the writer task of
  `WarpUdpSocket::new` (`send_ready.notify_waiters()` after each send).
- The receive side of the same socket uses an `AtomicWaker` that the reader task wakes; it has no such fault.
- Not measured: a stall in a live run.

## Approach

1. A test: fill the queue, poll `poll_writable` until `Pending` with a waker that counts its wakes, drain one
   slot as the writer task does, and assert one wake.
2. Replace the `Notify` of the send side with an `AtomicWaker`: the poller registers the waker of the task,
   then checks the capacity again; the writer task wakes it after each send and at its end.
3. Tell podcrates (pod-quic takes this socket later): a COORD issue (to podcrates), as for each fix of code
   that a crate will hold.

## Prove

```sh
cargo test -p podnet --lib warp::quic::tests
sh scripts/agent/check.sh
```

## Done

2026-11-10, commit 8bd5331. `SendPoller` waits on `SendReady`, an `AtomicWaker` (as the receive side): the
poller registers the waker of the task, then checks the capacity again (and a closed queue ends the wait). The
writer task wakes it when it takes a datagram from the queue and at its end. Test:
`warp::quic::tests::a_drain_after_pending_wakes_the_send_poller`; it failed on the old poller ("the poller was
not woken") and passes now. `sh scripts/agent/check.sh`, `unshare -n -- cargo test --workspace` and
`sh scripts/restricted-host.sh`: pass. podcrates knows the fix through the message of its session; pod-quic
(podcrates T-019) takes the socket with it. CI: at the push.

# T-226: The netns backend moves a large TCP download at full speed with a bounded buffer

**Source:** The measure of T-151 (2026-11-10): the netns backend through a TCP SOCKS5 proxy moved
1 MiB at 9 MB/s, 8 MiB in 17 s, and 64 MiB did not end in 120 s; for 1 GiB its process grew to 1 GB.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The netns backend (`crates/podnet-netns/src/netns/backend.rs`) is slow on a large TCP download, and its
memory grows with the size of the download.

## Premise

- Read: `Backend::shuttle` reads the upstream until `WouldBlock` with no limit, into `to_app`. Each
  `drain(..n)` at the front of that buffer moves all of the bytes after it.
- Read: `Backend::tick` calls `Interface::poll` one time. In smoltcp 0.12, one `poll_egress` sends at
  most one segment for each socket. So the backend sends one segment on each tick of its loop (a TUN
  input or 5 ms).
- Measured: `netns_backend_moves_a_large_download` (48 MiB) failed at the `alarm(60)` of its client.

## Approach

1. A test: a SOCKS5 proxy on a unix socket sends 48 MiB; a client in the pod counts the bytes under
   `alarm(60)`.
2. Hold at most `TCP_BUF` bytes in each direction of a flow: read the upstream only when the socket
   takes the held bytes, and read the socket only when the upstream takes them.
3. After `shuttle`, call `poll_egress` again while `poll_at` says that a socket has a segment now,
   at most 64 times in one tick.

## Prove

```sh
cargo test -p podnet --test pod_seccomp netns_backend_moves_a_large_download
sh scripts/agent/check.sh
```

## Done

2026-11-10, commit f81e7e6. `Backend::egress` calls `poll_egress` while `poll_at` is now (at most
`EGRESS_ROUNDS`, 64). `send_held` and the bound `HELD` (`TCP_BUF`, 64 KiB) stop the read of the upstream
and of the socket while the other side holds 64 KiB. The bound alone did not change the speed (the test
failed at 60 s); the egress loop did. Debug build, the same test: 1 MiB in 1.2 s, 4 MiB in 8.1 s and
8 MiB in 17.4 s before the egress loop; 8 MiB in 0.37 s and 48 MiB in 1.9 s after. The memory of a flow
is at most 64 KiB held in each direction, plus the smoltcp buffers. `sh scripts/agent/check.sh`,
`unshare -n -- cargo test --workspace` and `sh scripts/restricted-host.sh`: pass. CI: at the push.

# T-227: Publish `v0.4.0-beta.2`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The fixes after `v0.4.0-beta.1` (T-150, T-151, T-157, T-158, T-181, T-225, T-226 and others) are on
`main` only. A tester of a release binary does not get them.

## Premise

- `v0.4.0-beta.1` is the newest release (T-023).
- `main` is green at `99014d5` (ci 42801224616, live 42801224536, secrets 42801224821).

## Approach

1. Set the version `0.4.0-beta.2` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`.
2. Write `docs/releases/v0.4.0-beta.2.md`, with `## Breaking`.
3. Push, and wait until `ci` and `live` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.2`.
5. Check the files of the release and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-10, commit 5465007 (the version and the note). The dry run 42815610031 and the release run
42833480545 passed; the tag `v0.4.0-beta.2` is at 5465007. The release is a prerelease with the 8
binaries, `podnet.1`, `SHA256SUMS`, `SHA256SUMS.sigstore.json`, `provenance.sigstore.json` and
`THIRD-PARTY-LICENSES.md`. `release-proofs.yml` 42836050242: success. ci 42808911135 and live
42810147582 at 5465007: success.

# T-228: Each sender that waits on the WARP QUIC socket gets its wake-up

**Source:** The podcrates session (PodUtils/podcrates T-159, pod-quic), 2026-11-10: a second finding in
`src/warp/quic.rs`, after T-225.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The HA connections of the quic+warp leg share one WARP socket. quinn polls `poll_writable` from the task
of each connection, and each poller of the socket shares one `SendReady`. With T-225, `SendReady` is one
`AtomicWaker`: a second sender that waits on a full queue overwrites the waker of the first. The first
sender misses its wake-up, and its connection can stall while the queue is full.

## Premise

- Read: `src/warp/quic.rs`, `WarpUdpSocket::create_io_poller` (each poller gets a clone of the same
  `SendReady`) and `SendReady` (one `AtomicWaker`).
- The receive side has one poller (the driver of the endpoint): its `AtomicWaker` is sufficient.
- Not measured: a stall in a live run.

## Approach

1. A test: two pollers wait on a full queue; one drain must wake both.
2. `SendReady` keeps a list of wakers (one for each waiting task); the writer task wakes each one, and
   each one checks the queue again.
3. Tell podcrates: pod-quic (podcrates v0.14.0) has the same fix.

## Prove

```sh
cargo test -p podnet --lib warp::quic::tests
sh scripts/agent/check.sh
```

## Done

2026-11-10, commit 1d556fb. `SendReady` is a list of wakers under a mutex: `register` adds the waker of a
task when the list has none that `will_wake` it, and `wake` takes the list and wakes each waker. Test:
`warp::quic::tests::two_senders_on_a_full_queue_each_wake`; it failed on the one `AtomicWaker` ("sender 0
was not woken") and passes now. `a_drain_after_pending_wakes_the_send_poller` (T-225) passes. CI: at the
push. The take of `pod-quic` (b7b0aef) replaces the socket; `pod-quic` has the same fix
(`two_senders_on_a_full_way_each_wake`).

# T-216: Freeze `doctor --json` as schema 1, with `--quick` (PodUtils/podcrates#35, A-N-04)

**Source:** PodUtils/podcrates#35 (cfb), ask A-N-04, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

Ask A-N-04 (P1): "Freeze `podnet doctor --json` as schema 1 for the legs and hops, with a quick mode."
cfb reads the document to qualify a host. The keys must not change within schema 1, and a run must end soon.

## Premise

- Read: `src/doctor.rs:245` (`to_json`) writes `ok`, `roads`, `legs` (`leg`, `verdict`, `status`, `reason`) and `names`, and `status::stamped` adds `schema` 1 and `podnet`. No test pins the key set.
- Read: `podnet doctor` has `--samples` and `--timeout` (`src/cli/cmd/host.rs`). It has no `--quick`. The default budget of the samples is 10 s (`Options::budget`).
- Not measured: the time of a full run on a host with no egress.

## Approach

1. Write the schema in `docs/protocol.md`: each key, its type, and the verdict words. Say that a key is added, never removed or renamed, within schema 1.
2. Add a test that compares the set of keys of each object (top level, `roads`, `legs`, the hops) to a fixed list.
3. Add `--quick`: one sample for each path, a limit of 15 s for the whole run, and a leg that did not end in time gets the verdict `unknown` with a reason. `--quick` keeps the same keys.
4. Test the limit with a fake host that never answers: the run ends in 15 s.

## Prove

```sh
cargo test -p podnet --test doctor -- doctor_json_keys_are_fixed doctor_quick_ends_in_the_limit
cargo todo check
```

## Done

2026-11-11, commit 7045593. `docs/protocol.md` section 9 gives schema 1: each key, its type and the verdict words. `podnet doctor --quick` makes one sample for each path and ends the run in 15 s (`Options::quick`, `Options::deadline`); a path, the UDP probe or the clock that is late gets `unknown` with the reason `no answer in the time limit of --quick`. The key `quick` (bool) is new. The JSON code moved to `src/doctor/json.rs`. Tests: `doctor_json_keys_are_fixed` and `doctor_quick_ends_in_the_limit` (`tests/doctor.rs`, fail-first: with the limit off the run took 18.26 s and the test failed), `doctor::tests::doctor_json_dns_keys_are_fixed`. check.sh: exit 0. CI: at the merge. Not proven: the time of a full run on a host with no egress (the fake host is a proxy that is slow, not a real network).

# T-217: `doctor --json`: a fixed block for the egress lanes (PodUtils/podcrates#35, A-C-10)

**Source:** PodUtils/podcrates#35 (cfb), ask A-C-10, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

Ask A-C-10 (P1): "A fixed JSON schema of the egress lanes for the node probe of cfb: `tunnel_quic`, `tunnel_http2`, `tunnel_quic_warp`, `masque_tcp443`, `masque_udp443`, `ws_upgrade` and `proxy`, each `ok`, `fail` or `unknown`, with the time."

## Premise

- Read: `src/doctor.rs:223` (`roads`) groups the legs by road. The legs have their own names, not the seven lane names. A leg has a time (`ms`) in the paths, not in the legs.
- Read: the verdict words of the doctor are `ok`, `no` and others (`Verdict::name`). The lanes use `ok`, `fail` and `unknown`, so a map is necessary.
- `ws_upgrade` has no probe now. `proxy` is the row of the proxy (T-184).

## Approach

1. Add the key `lanes` to the JSON of `doctor`: an object with the seven names, each `{"verdict": "ok|fail|unknown", "ms": N or null}`. Map each leg and probe to one lane in one table.
2. Add a probe for `ws_upgrade` (a WebSocket upgrade to the edge host), or give `unknown` with a reason until it exists.
3. Write the block in the schema of T-216. Pin its keys with the same test.
4. Do the work after T-216 (one schema) and after T-184 (the proxy row).

## Prove

```sh
cargo test -p podnet --test doctor -- doctor_json_has_the_seven_lanes
cargo todo check
```

## Done

2026-11-11, commit 78c1e6a. `lanes` in `doctor --json` has the seven lanes, each `{verdict, ms, reason}`; the table of the sources is in `src/doctor/json.rs` (`lanes`) and in `docs/protocol.md` section 9. `masque_udp443` and `ws_upgrade` have no probe and give `unknown` with a reason. `proxy` is the verdict of the paths that go through a proxy. Tests: `doctor_json_has_the_seven_lanes` (`tests/doctor.rs`, fail-first: without the key it panicked on `doctor["lanes"]`), `doctor::tests::the_lanes_follow_the_legs_and_paths`; `doctor_json_keys_are_fixed` pins the keys of `lanes`. check.sh: exit 0. CI: at the merge. Not proven: a real `ok` of `masque_tcp443` and `tunnel_quic_warp` (no network here). Not done: the probes of `masque_udp443` and `ws_upgrade`.

# T-222: `llms.txt`, `llms-full.txt` and `--help --json` (PodUtils/podcrates#35, A-X-19)

**Source:** PodUtils/podcrates#35 (cfb), ask A-X-19, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Ask A-X-19 (P2): "Machine-readable docs for agents: `llms.txt` and `llms-full.txt` for each tool, and `--help --json` generated from the source."

## Premise

- Read: `podnet man --json` prints the manual as JSON from the clap tree (`src/cli/cmd/host.rs`, `man`). `--help --json` does not exist.
- No `llms.txt` exists in this repository.

## Approach

1. Make `--help --json` (global, at each level) print the same document as `podnet man --json` for that command.
2. Generate `llms.txt` (an index of the commands and documents) and `llms-full.txt` (the manual and the documents) from the clap tree and `docs/`, with a script in `scripts/`. Publish both with each release.
3. A test makes sure that the files agree with the tree.

## Prove

```sh
cargo test -p podnet --test cli -- help_json_matches_man_json
cargo todo check
```

## Done

2026-11-11, commit 981ee40 (`T-222: --help --json, llms.txt and llms-full.txt`). `--help --json` (or
`-h --json`) at each level prints the JSON of `podnet man --json` for the command that the words of
the line name (`manual::help_json_request`, called in `src/main.rs` before the parse). The root
gives the whole manual. `pod` keeps its own help. `scripts/gen-llms.py BINARY` writes `llms.txt` (the
summary, one line for each command, the documents) and `llms-full.txt` (`podnet man --no-pager` and
the text of README.md and three documents) in the repository root. The script cuts the commit from
the version, so the files do not change with each build. `--out DIR` writes to another directory.
`release.yml` builds both files from the musl binary, adds them to `SHA256SUMS`, and attaches them;
`release-proofs.yml` checks their lines. Tests (`tests/cli/llms.rs`): `help_json_matches_man_json`
failed before the change (the output of `--help --json` was the text of the help) and passes now;
`llms_files_are_current` runs `gen-llms.py --check` and fails when a file is old.
`sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test all 0.
Not proven: the release steps (`release.yml`, `release-proofs.yml`) did not run; the files hold the
version of `Cargo.toml`, so a version change needs a new run of the script.

# T-223: seccomp: the TCP gaps that the differential check finds

**Source:** T-155 (the differential check `tests/pod_seccomp_diff.rs`), 2026-11-09
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

The differential check of T-155 runs one series of socket calls on the host kernel and in a seccomp
pod. Six TCP lines differ. Each one is an item of `ALLOWED` in `tests/pod_seccomp_diff.rs`:

| Label | Host kernel | Seccomp pod |
| --- | --- | --- |
| `tcp shutdown unconnected` | `ENOTCONN` | 0 |
| `tcp getsockname bound` (bind to port 0) | an ephemeral port | port 0 |
| `tcp bind again` | `EINVAL` | 0 |
| `tcp listen unbound` | 0 (the kernel binds a port) | `EINVAL` |
| `tcp SO_ERROR after refused` (a blocking connect) | 0 | `ECONNREFUSED` |
| `tcp nonblocking connect` (to a closed loopback port) | `EINPROGRESS` | `ECONNREFUSED` |

The UDP lines of the same series agree after T-143 to T-149.

## Premise

- Measured: `cargo test --test pod_seccomp_diff` on 2026-11-09, on a host with user namespaces and
  seccomp user notification (the table above).
- Read: `shutdown` is not in the filter (`crates/podnet-netns/README.md`, seccomp). The socketpair
  is connected, so the kernel answers 0.
- Read: `handle_bind` (`crates/podnet-netns/src/sockopt.rs`) keeps the address of a stream as the
  program gives it, also port 0 and a second `bind`. `handle_listen` needs a bound port.
- Read: the pending error of a refused connect stays for one read of `SO_ERROR` (issue 302). The
  kernel clears it when `connect` itself returns the error.
- Not measured: the effect of each gap on a real program.

## Approach

1. `bind` of a stream: port 0 gets `ephemeral_port(fd)`; a second `bind` gets `EINVAL` (as
   `Udp::bind` of T-144).
2. `listen` with no `bind`: bind the unspecified address and an ephemeral port first.
3. `SO_ERROR`: keep the error only when the `connect` did not return it.
4. `shutdown` and a nonblocking `connect`: decide whether to stop `shutdown` (one more system call
   in the filter) and whether to give `EINPROGRESS`; write the result here.
5. Remove each fixed line from `ALLOWED`. The check fails on a line that no longer differs.

## Decision

2026-11-09: each of the six gaps closes; `ALLOWED` is empty.

- `shutdown` goes into the filter. A TCP socket with no connection, no listener and no dial gets
  `ENOTCONN` (`EINVAL` for a bad `how`). Each other socket goes to the kernel, as before. The cost
  is one notification for each `shutdown` call of the program.
- A nonblocking `connect` gives `EINPROGRESS` and the dial goes on. The socket pair is writable
  before the dial ends, so `poll` does not wait. Thus `getsockopt(SO_ERROR)` during the dial
  waits for the end of the dial and gives its result. This keeps the result that T-067 (Correction)
  found lost. A program that reads `SO_ERROR` after `poll` (curl, Go, tokio, libuv, Python) gets
  the correct result. A second nonblocking `connect` during the dial gives `EALREADY`; a blocking
  one waits. At the end of the dial the connection stays when no call waits. One waiting
  `SO_ERROR` can take a plain upstream (T-065).
- Not matched: a `read` or `write` after a failed nonblocking dial does not give the error (the
  socket pair stays open), and `poll` gives no `POLLERR`. The program must read `SO_ERROR`. The
  driver does not test this.
- The driver has a new line `tcp getsockname listening` (the address of `listen` with no `bind`).

## Prove

```sh
cargo test --test pod_seccomp_diff -- seccomp_errnos_match_the_host
```

## Done

2026-11-09, commit d68a51c (`T-223: seccomp: close the six TCP gaps of the differential check`).
The six TCP lines agree with the host kernel; `ALLOWED` of `tests/pod_seccomp_diff.rs` is empty.
New code: `crates/podnet-netns/src/tcp.rs` (`shutdown`, `listen` with no `bind`, a waiting
`SO_ERROR`) and the nonblocking dial of `crates/podnet-netns/src/connect.rs`. Tests:
`seccomp_errnos_match_the_host` (with the new line `tcp getsockname listening`),
`seccomp_a_nonblocking_connect_gives_einprogress` (`tests/pod_seccomp_nonblock.rs`: `EINPROGRESS`,
`EALREADY`, `SO_ERROR` waits for a slow dial, the handoff of T-065, a write before the dial ends),
and `so_error_gives_the_error_of_a_failed_connect` (now `EINPROGRESS`, then `ECONNREFUSED` once).
`sh scripts/agent/check.sh` exits 0. `unshare -n -- cargo test --offline --test pod_seccomp_diff
--test pod_seccomp --test pod_seccomp_dgram` and `sh scripts/restricted-host.sh` with the same
tests exit 0. CI: at the merge.

# T-235: Publish `v0.4.0-beta.3`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The work after `v0.4.0-beta.2` (T-055, T-056, T-058 to T-060, T-088 to T-094, T-109, T-110, T-112, T-113,
T-228 to T-231, T-233, T-234, the takes of `pod-quic` and `pod-hold`) is on `main` only. A tester of a
release binary does not get it.

## Premise

- `v0.4.0-beta.2` is the newest release (T-227).
- The checks of `sh scripts/agent/check.sh` pass on `main` at f9da9e6.

## Approach

1. Set the version `0.4.0-beta.3` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`.
2. Write `docs/releases/v0.4.0-beta.3.md`, with `## Breaking`.
3. Push, and wait until `ci` and `live` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.3`.
5. Check the files of the release and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-11, commits 05bcf44 (the version and the notes) and 649c8e3 (the head of the release: ci
42942255003, live 42942255041, deny and secrets: success). The dry run 42946309363 and the release run
42949492119 passed; the dispatch 42948701660 made the tag `v0.4.0-beta.3` at 649c8e3. The release is a
prerelease with the 8 binaries, `podnet.1`, `SHA256SUMS`, `SHA256SUMS.sigstore.json`,
`provenance.sigstore.json` and `THIRD-PARTY-LICENSES.md`. `release-proofs.yml` 42952184194: success.


# T-238: `listen::bind_private`: bind in a private 0700 directory, chmod, then rename into place, so the socket never has a wider mode under its name (GitHub #429)

**Source:** GitHub #429 (https://github.com/PodUtils/podnet/issues/429); donor review, satmux.md block 6
(Th0rgal/satmux at 2922ebb, MIT)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet_netns::listen::bind_private` binds a unix socket and then sets the mode 0600. Between those two
calls the socket has the mode that the umask of the process gives it. On a shared host with the umask
002 or 000, a local user who connects in that interval reaches an unauthenticated SOCKS5 door that opens
connections as the owner. `podnet net proxy` listens on the default `unix:/tmp/podnet-vnet.socks`. With
the umask 022, other users cannot connect in the interval, so the window matters only for the umasks
above. The check to satisfy is: no other user can connect, at any instant.

## Premise

- `crates/podnet-netns/src/listen.rs:127-170` (`bind_private`) calls `UnixListener::bind(path)`, then
  `set_permissions(path, 0o600)`.
- The callers that bind in a directory that other users can reach: `src/cli/cmd/connect.rs:250`,
  `src/warp/proxy.rs:86`, `crates/podnet-vnet/src/proxy.rs:314`, `crates/podnet-vnet/src/switch.rs:434`,
  and, through `bind_claimed` (`crates/podnet-netns/src/listen.rs:250-253`),
  `crates/podnet-netns/src/tailscale.rs:356` and `crates/podnet-netns/src/frontdoor.rs:59`.
- `crates/podnet-netns/src/seccomp/accept.rs:46` is not exposed: it calls `private_dir` first
  (`crates/podnet-netns/src/seccomp/accept.rs:43`), and it removes the old file before the bind (`crates/podnet-netns/src/seccomp/accept.rs:44`).
- The defaults in `/tmp`: `src/cli/cmd/net/mod.rs:78`, `src/cli/cmd/net/mod.rs:95` and
  `src/cli/cmd/net/mod.rs:173` (and `/tmp/podnet-vnet.switch` at `src/cli/cmd/net/mod.rs:237`).
- `libc` is a dependency (`Cargo.toml:132`, `crates/podnet-netns/Cargo.toml:24`). No code calls
  `renameat2` yet.
- T-186 (done, GitHub #385) added `claim_unix_path` and `bind_claimed`. They do not cover the interval.
  GitHub #306 (closed) fixed the inbound directory of `accept.rs` only.
- The same shape is in `bind_unix` of `pod-broker` (PodUtils/podcrates). That is a separate ask to the
  session of podcrates (a COORD issue).
- Not re-run: the umask measurements of the issue (the author's). The issue reports the modes 0755, 0775
  and 0777 right after the bind for the umasks 022, 002 and 000.

## Approach

1. Keep the claim step first (`claim_socket_path`, `claim_unix_path`).
2. In `bind_private`: make the directory `PARENT/.podnet-bind-XXXXXX` (random name, create-exclusive, mode
   0700), bind `s` inside it, `chmod` it to 0600, then move it to the final path with
   `renameat2(RENAME_NOREPLACE)`, then remove the directory. An `EEXIST` gives `AddrInUse`.
3. Where the file system has no `RENAME_NOREPLACE`, fall back to `link` and `unlink` (`link` fails on an
   existing name).
4. A crash between the bind and the rename leaves an empty 0700 directory. A start-up sweep may remove
   the directories with the prefix `.podnet-bind-` that the same user owns.
5. Do not use `umask`: it is process-wide and races with each thread that creates a file. A simpler fix,
   to require a 0700 parent in the callers, is possible. The rename is the more general one.
6. Verify `renameat2` in the `libc` crate on the musl targets of the static builds.

## Prove

```sh
cargo test -p podnet-netns --lib -- bind_private_is_never_wider_than_0600 bind_private_gives_0600_under_each_umask
cargo test -p podnet-netns --lib -- two_bind_private_calls_on_one_path_give_one_winner a_claim_refuses_a_live_socket_and_a_file_and_removes_a_stale_socket
```

A test seam runs a closure after the bind and before the chmod. Inside it, assert that the parent
directory of the socket has the mode 0700 and that the final path does not exist yet. A planted defect
(bind at the final path) must fail this test. Run with the umask 000, 002 and 022 in a child process: the
final path has the mode 0600 when `bind_private` returns. Two concurrent calls on one path: one succeeds,
the other gives `AddrInUse`, and neither removes the socket of the other. A crash after the bind and
before the rename leaves no socket under the final name.

## Correction

2026-11-11: the moved socket keeps the name of its bind (`getsockname`, the name of an accepted socket):
the staging path. The seccomp inbound listener needs the name of its path (T-150), and it binds in its
own directory of mode 0700 (`private_dir`), so it takes `bind_in_private_dir` (a bind at the path, then
0600) and no staging. A path too long for the staging name falls back to that bind too. The test of the
umasks runs in a new process (the test binary again, `PODNET_TEST_UMASK`), not a fork: a fork copied the
socket of a parallel test. For the same reason the probe child of `pod doctor` (T-231) closes each
inherited descriptor but its pipe.

## Done

2026-11-11, commit 3c6173f. `crates/podnet-netns/src/listen.rs`: `bind_private` binds in `.pnb-HEX/s` (mode 0700)
beside the path, sets 0600, moves the socket with `renameat2(RENAME_NOREPLACE)` (else `link` and `unlink`) and
removes the directory; an existing path is `AddrInUse`. Tests: `cargo test -p podnet-netns --lib -- bind_private
a_claim_refuses` (4: the seam asserts the 0700 directory and no final path before the chmod; the umasks 000, 002
and 022 give 0600; two binds of one path give one winner and `AddrInUse`; the claim). `sh scripts/agent/check.sh`:
each step 0. Not done here: the start-up sweep of item 4 (a crash leaves an empty 0700 directory `.pnb-HEX`),
and the same fix of `bind_unix` of `pod-broker` (a COORD issue to podcrates).

# T-239: `install.sh` checks the release tag and reads SHA256SUMS strictly, `podnet update` does the same, and the install tests assert the downloader flags (GitHub #430)

**Source:** GitHub #430 (https://github.com/PodUtils/podnet/issues/430); donor review, tailsplit.md block 3
(idy/tailsplit at 0bd3a74, GPL-3.0-or-later)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`scripts/install.sh` builds download URLs from `PODNET_VERSION` and from the tags that it reads out of
`releases.atom`, without the tag rule that `podnet update` already applies. Its reader of SHA256SUMS
takes any number of matching lines, and `podnet update` takes the first. The install tests use a fake
`curl` that ignores its flags, so removing `--proto '=https'` from the script fails no test. Example:
`PODNET_VERSION='v1.2.3?x=y' sh install.sh` requests
`https://github.com/PodUtils/podnet/releases/download/v1.2.3?x=y/SHA256SUMS`, and a tag with `/../`
reaches another path. `podnet update` refuses the same text with "not a release tag". A release with a
duplicated SHA256SUMS line fails closed today, but the message is misleading: it prints "the SHA256 of X is
H, not H" with the same hash twice.

## Premise

- Implement from the description, not from the donor source (the donor is GPL-3.0-or-later).
- `scripts/install.sh:327-351` (`resolve_tag`): the branches `v*) echo "$1"` and `*) echo "v$1"` pass the
  text through. The line `scripts/install.sh:84` builds `base` from it.
- `scripts/install.sh:401-413` (`check_sum`): the `awk` prints each matching hash, and the compare is
  against the joined text.
- The rule that exists: `src/update.rs:167-174` (`valid_tag`: 1 to 64 characters, first `v` or a digit, no
  `..`), used by `src/update.rs:185`, `src/update.rs:271` and `src/update.rs:348`.
- `src/update/cosign.rs:160-174` (`sum_of`) returns the first matching line and does not check the length
  of the hash. SHA256SUMS is signed, so this is a hardening gap, not a hole.
- `tests/install_script.rs:102-112` (`Lab::curl`): the argument loop skips each word that starts with `-`,
  so no flag can change a result. `Lab::run` clears the environment and sets no `TMPDIR`; one test sets it
  (`tests/install_script.rs:408`). The script passes `--proto` at `scripts/install.sh:272` and
  `scripts/install.sh:292`.
- Closed issues that touch the same files and add no such rule: GitHub #227 (`valid_tag` for `update`
  only), #290 (seven defects of the script), #216, #246, #168, #169, #2, #17, #19. No entry of the record
  covers it.
- Not re-run: the exit codes and the messages for the hostile versions of the issue.

## Approach

1. Add `valid_tag` to `scripts/install.sh` with the same rule as `src/update.rs`. Call it in `resolve_tag`
   for the three branches (the `latest` result, `v*`, a bare version), and fail with "not a release tag"
   before any download. Check the shape `OWNER/NAME` of `PODNET_REPO` in the same place.
2. One strict reader of SHA256SUMS in both places: exactly one line names the asset, the line has two
   fields, and the hash has 64 hex digits. Otherwise the error says "SHA256SUMS must have exactly one valid
   line for NAME (found N)". In Rust, count the matches before the return, and fail on 0 or on more than
   1, and fail on a hash that does not decode to 32 bytes.
3. Tests: `Lab::curl` records its full argument list, and a test asserts `--proto =https` and
   `--proto-redir =https` for each call. `Lab::run` sets `TMPDIR` to a directory in the lab, and a shared
   `assert_clean` checks that it is empty after each run (success, failure and TERM). A table test for
   hostile versions (`v1/../x`, `v1?x=y`, `v1 2`, `$(touch injected)`, a newline): each exits non-zero,
   makes zero `curl` calls and creates no file `injected`. A table test for SHA256SUMS: no line, a short
   hash, a non-hex hash, an extra field, two lines for the asset (the correct one first, the wrong one
   first). A planted-defect check: a copy of `install.sh` without the `--proto` words must fail the suite.

## Decision

Do not add `curl -q` (the donor passes it first, so that `~/.curlrc` cannot change the options) without a
decision of the owner. A host that keeps its proxy only in `~/.curlrc` would lose it. The archive checks of
the donor do not apply: the releases of podnet ship one static binary for each target, not an archive.

## Prove

```sh
cargo test -p podnet --test install_script
cargo test -p podnet --lib -- update::
```

`PODNET_VERSION='v1.2.3?x=y' sh scripts/install.sh` exits non-zero with "not a release tag", and the log
of the fake `curl` is empty. Both orders of a duplicated line give "exactly one valid line (found 2)", and
no binary is installed. The suite fails when `--proto '=https'` is removed from `download()`. The suite
leaves no file in the `TMPDIR` of the lab. `podnet update --version v1.2.3` with a SHA256SUMS that lists
the asset twice fails with the same wording.

## Done

2026-11-11, commit be5259e. `scripts/install.sh` has `valid_tag` (the rule of `src/update.rs`) in
`resolve_tag` for the `latest` result, `v*` and a bare version; it fails with "not a release tag" before
any download. It checks `PODNET_REPO` as OWNER/NAME. `check_sum` needs exactly one two-field line with a
64-hex hash, else "SHA256SUMS must have exactly one valid line for NAME (found N)". `sum_of` in
`src/update/cosign.rs` does the same. Tests: `strict::curl_gets_https_only_flags`,
`strict::a_script_without_proto_fails_the_flag_test`, `strict::hostile_versions_stop_before_any_download`,
`strict::a_bad_repo_stops_before_any_download`, `strict::sha256sums_needs_exactly_one_valid_line`
(`tests/install_script/strict.rs`), and `update::tests::sha256sums_must_have_exactly_one_valid_line`. The fake
`curl` logs its full arguments; `Lab::run` sets `TMPDIR` and asserts it is empty after each run. Against
the old script 3 of the new tests fail; against the new one all 28 pass. Not proven: the other downloaders
(wget, aria2c) get no flag test; the TERM case relies on the existing test with the clean check; no live
run.

# T-240: `warp tun --default-route`: keep the resolver backup on disk, restore it after a kill, set the mode of `resolv.conf`, and test the route table with `ip route get` (GitHub #431)

**Source:** GitHub #431 (https://github.com/PodUtils/podnet/issues/431); donor review, tailsplit.md block 4
(idy/tailsplit at 0bd3a74, GPL-3.0-or-later)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`podnet warp tun --default-route` replaces `/etc/resolv.conf` and keeps the original text only in memory.
If the process is killed (SIGKILL, the OOM killer, a power cut), the original is lost, and the next run
reads the replaced file as its "original". A clean stop then "restores" the file that podnet wrote, and
the original resolver list is gone. On a network where only the internal resolver answers, each lookup
fails in the meantime. A second symptom: `replace_file` writes with `std::fs::write`, so the mode follows
the umask. Under the umask 077 the new `/etc/resolv.conf` has the mode 0600, and the programs that are not
root lose name resolution. Persist the original before the first change. Restore it at the next start and
at `warp down` when the marker line shows that the file is still ours. Give the replaced file an explicit
mode. Add a privileged CI test that compares `ip route get` before and after.

## Premise

- Implement from the description, not from the donor source (the donor is GPL-3.0-or-later).
- This is a defect of the WARP code (`src/warp/`). That code waits for `pod-warp` (T-050), and it takes
  fixes. `src/warp/route.rs` has 799 of 800 lines (`scripts/check-size.py`): a change must not add lines to
  it without a split.
- `src/warp/route.rs:364-370`: `enum Dns { Resolved, File { backup: String, link: Option<PathBuf> } }`.
  `src/warp/route.rs:574-596` (`set_dns`) reads the backup from whatever file is there
  (`src/warp/route.rs:592-593`), with no check of the marker. Only `Drop for Routes`
  (`src/warp/route.rs:508-538`) restores. `src/warp/route.rs:618-629` (`replace_file`) uses
  `std::fs::write`; no code sets the mode. `src/warp/route.rs:566-572` (`resolv_text`) already writes the
  marker line `# podnet warp tun --default-route: DNS through WARP`.
- The call of `Routes::setup` is in `src/cli/cmd/warp/tun.rs:101`. The name of the instance is
  `TUN_INSTANCE` (`src/warp/up.rs:26`), and `instances::is_alive` answers from the held lock
  (`crates/podnet-core/src/instances.rs:327`). The detached child holds the lock under that name, so the
  liveness test must be a lock on a file of its own.
- No test touches a kernel route table: the unit tests give a closure in place of the route calls
  (`install_all`).
- Closed issues: GitHub #298 (item 2: `set_dns` writes by rename and keeps the backup in `Routes`), #77
  (a saved copy; it landed as the in-memory `Dns::File`), #138 (the ledger of the system proxy; no code on
  `main`). T-012 (blocked: the WARP stand-ins) is context only. The privileged test does not need the
  stand-ins.
- Not run: the `unshare` measurements of the issue (iproute2 6.15.0, kernel 6.12.113: the selected route
  moves to the TUN while `ip route show default` does not change). That the kernel removes the `/1` routes
  with the TUN is Inferred in the issue, not reproduced. A kill of a real `warp tun` needs `CAP_NET_ADMIN`
  and a WARP session.

## Approach

1. `set_dns` takes a lock on `STATE/resolv.lock` (flock, held until `Routes` drops). If it fails, refuse
   with "another warp tun runs". Then it calls `reconcile_held()`: the lock is ours, so any record is from a
   dead process.
2. Write a record (durable, mode 0600) BEFORE the first change: `{ version: 1, path, kind: file|symlink|
   absent, text | link_target }`. Then replace the file with the marker text, with the mode 0644 or the mode
   of the replaced file (not the umask).
3. `reconcile()` at the start of `warp tun` and at `warp down`: try the lock. If a live tun holds it,
   return. Else, with a record: if the current file starts with the marker line, restore from the record
   (symlink or text; in place when the rename fails) and remove the record. If not, remove the record, log
   "resolv.conf was changed by someone else; left as found".
4. A record with an unknown `version` is left in place and reported, never guessed. A missing
   `/etc/resolv.conf` at the start is recorded as `kind: absent` and removed at the restore. A bind-mounted
   `/etc/resolv.conf` keeps the in-place path (the rename over it fails).
5. Confirm with the privileged test that the `resolvectl` per-link settings disappear with the device.
6. Split `src/warp/route.rs` first (a new module for the resolver code), so that the file stays within
   800 lines.
7. Privileged test (a disposable runner only; it refuses otherwise): `unshare --user --map-root-user --net
   --mount`, a dummy device with a default route, a second dummy device for the TUN, a bind mount over
   `/etc/resolv.conf`. Take `ip -4 route get 203.0.113.5` as `before`. Run `Routes::setup` and `up`. Take the
   snapshot again and assert that it differs (the positive control: it proves that the snapshot sees the
   `/1` routes). Drop the value and assert equality with `before`. Then the kill case: a child process runs
   `setup` and `up`, gets SIGKILL, the test asserts that `/etc/resolv.conf` is the marker file, calls
   `reconcile()`, and asserts that the original text is back.

## Prove

```sh
cargo test -p podnet --lib -- warp::route::
cargo test -p podnet --lib -- resolv_backup_survives_a_kill resolv_marker_guards_the_restore replace_file_sets_the_mode
unshare --user --map-root-user --net --mount cargo test -p podnet --test warp_routes -- --ignored routes_lifecycle
```

After `kill -9` of `warp tun --default-route`, the next `warp tun` or `warp down` restores the original text
(or the original symlink) and removes the record. A second start after a kill never records the marker file
as the original. If the file no longer starts with the marker line, `reconcile` leaves it unchanged. The
function `replace_file` gives the mode 0644 under the umask 077 and under the umask 0. A record with
`version: 2` is not applied and is named in the error.

## Done

2026-11-11, commit 57eaa34 (`T-240: warp tun --default-route keeps the resolver record on disk, restores
it after a kill, sets the mode`). The resolver code moved to the new file `src/warp/route/resolv.rs`
(`src/warp/route.rs`: 799 to 727 lines). `set_file` takes the flock `STATE/resolv.lock`
(refusal: "another warp tun runs"), restores a record of a dead process, writes the record
`STATE/resolv.record.json` (version 1, mode 0600, synced; kind file, symlink or absent) before the
first change, and replaces the file with the mode of the original (0644 for a new file) under any
umask. `reconcile` runs at `Routes::setup` and at `warp down` (after the stop); it restores only when
the file starts with the marker line, else it drops the record and logs. A record with another
version stays and is named in the error. Tests (`cargo test -p podnet --lib -- warp::route::resolv::`):
`resolv_backup_survives_a_kill`, `resolv_backup_restores_a_symlink_and_an_absent_file`,
`resolv_marker_guards_the_restore`, `resolv_record_of_another_version_is_not_applied`,
`replace_file_sets_the_mode`, `a_live_tun_blocks_a_second_one`: pass. Control with the record write,
the mode call, the marker check and the version check disabled: four of them fail. The privileged test
`tests/warp_routes.rs` (`routes_lifecycle`, `--ignored`) compares `ip -4 route get` before and after;
it skips here (no `ip`, no mount namespace) and did not run. Not proven: that test, the kill of a real
`warp tun` process, the `resolvectl` per-link settings.

# T-241: `warp`: register under a lock, save the device before the enrolment step, and fail over the registration POST only when no response came (GitHub #432)

**Source:** GitHub #432 (https://github.com/PodUtils/podnet/issues/432); donor review, tailsplit.md block 5
(idy/tailsplit at 0bd3a74, GPL-3.0-or-later)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Registering a WARP device is two remote calls (`POST /reg` creates it, `PATCH /reg/{id}` enrols its MASQUE
key), followed by a local save. The code checks only that the file is writable before it starts, and seven
call sites run the check, the registration and the save with no lock around them. Two processes can
register two devices, and a kill between the calls loses a device that the service has already counted.
The rule of the project is "Do not register WARP devices when it is not necessary" (the service limits new
devices for each address). Take a lock around the whole sequence. Save the device and its key as soon as
the first call answers, so that a later run resumes the enrolment. The check that proves the fix: two
parallel `register_or_load` calls against a fake API make exactly one `POST /reg`, and a kill after the
create call makes the next run send `PATCH` for the same device id and no second `POST`.

## Premise

- Implement from the description, not from the donor source (the donor is GPL-3.0-or-later).
- This is a defect of the WARP code (`src/warp/`). That code waits for `pod-warp` (T-050), and it takes
  fixes. The record needs a `state` field in `warp.json`: a format change. Breaking changes are allowed
  (beta).
- `src/warp/mod.rs:415-417` (`register_or_load`): `check_file_writable` at `src/warp/mod.rs:450`, the
  registration at `:451`, the save at `:452`, with no lock between. A stored device that is not enrolled
  for MASQUE counts as "register again", not as "resume" (`src/warp/mod.rs:438-446`).
- `src/warp/mod.rs:202-241` (`register_with`): `create_device`, then `enrol_masque`; the token stays in
  memory.
- The call sites with no lock: `src/cli/cmd/warp/client.rs:153`, `src/tunnel.rs:828` (only with
  `--warp-identity PATH`; without it `tunnel` registers a new device on each run and saves nothing),
  `src/cli/cmd/warp/front.rs:188`, and the WireGuard `load_or_register` (`src/warp/export.rs:75`; the
  callers `src/cli/cmd/warp/client.rs:38` and `src/cli/cmd/warp/client.rs:57`).
- `src/warp/register.rs:67-95` (`send_api_to`): a status of 500 or more, or a transport error, goes to the
  next API host for each request, the registration POST included (`src/warp/register.rs:77`).
- `instances::lock` and `try_lock` exist in `crates/podnet-core/src/instances.rs`; `delete_device` exists
  in `src/warp/register.rs`. `docs/decisions.md:35` has the rule of the registration.
- T-173 (done, GitHub #368) changed the refusal for a file of another uid in the same function and keeps
  "a refused file never leads to a registration". It does not add a lock. GitHub #368 is open on GitHub
  while T-173 is done. A close of #368 may be due. Closed issues #50, #187, #141 and #150 do not cover a
  lock or a resumable enrolment.
- Not run: the race and the kill case (Inferred in the issue; the registration needs the live API: use a
  local fake). Unverified: that the WARP API applies a create before it answers 5xx.

## Approach

1. `register_or_load(path)` takes `path + ".lock"` (flock, a deadline of 60 s, mode 0600, never removed)
   and holds it to the end of the function. A lock held longer than the deadline gives "another podnet is
   registering" with exit 75.
2. With a stored file: an enrolled device returns. A device with the state `created` goes to
   `finish_enrolment` (a `PATCH` with the stored token, no new `POST`). On 404 or 401 (the device is gone),
   remove the partial record and register anew (one `POST`).
3. For a new device: `POST /reg`, then save the partial record (`state: "created"`, the device id, the token,
   the key; durable, mode 0600, the same `check_private` as the complete file), then `PATCH`, then save the
   complete record.
4. A reader that does not know the `state` value refuses the file, and names the value.
5. If the `PATCH` fails and the process stays alive, call `delete_device` for the new device when the
   failure is not a network error (best effort; do not hide its own failure).
6. For the registration POST only: fail over to the second API host when no response arrived (a connect
   failure). After any HTTP response, 5xx included, return the error.
7. Use the same lock and the same two-step save in the WireGuard export path.
8. A kill between the answer of the POST and the first save still loses that device. The window shrinks to
   one durable write.
9. `src/warp/mod.rs` and `src/warp/register.rs` are WARP code: keep the change small, and send nothing new
   to the future `pod-warp` except as the fix.

## Prove

```sh
cargo test -p podnet --lib -- warp::register
cargo test -p podnet --test warp_register -- two_register_or_load_calls_make_one_post a_kill_after_the_post_resumes_with_a_patch
cargo test -p podnet --test warp_register -- a_patch_404_registers_anew a_503_on_the_post_is_not_sent_to_the_second_host
```

Two tasks call `register_or_load` on one path with a fake API: the fake counts one `POST /reg`, and both
get the same device id. A child process killed with `kill -9` after the POST and before the PATCH: the next
call sends `PATCH` for the same id, sends no second `POST`, and the final file has `tunnel_type`
`masque`. A partial file with an unknown `state` is refused with its name in the message. A connect failure
on the first host still falls over.

## Done

2026-11-11, commit f08840c. `src/warp/enrol.rs` (new): `register_or_load` takes `PATH.lock` (flock, 60 s, mode
0600, kept), saves the record with `state: "created"` after `POST /reg`, sends `PATCH` and saves the complete
record (no `state`). A stored `created` record resumes with `PATCH` and no `POST`; a 401 or 404 removes it and
registers anew. A failed `PATCH` with an HTTP answer deletes the new device (best effort) and the record.
`load` refuses a `created` record; an unknown `state` is refused with its name. `src/warp/register.rs`: a `POST`
with any HTTP answer is not sent to the second host; `check` returns `ApiError`. `src/warp/export.rs`: the
WireGuard registration takes the same lock and checks the file first. `DeviceHints` moved to
`src/warp/hints.rs` (size allowlist lowered). Tests: `cargo test -p podnet --test warp_register` (6, five of
them fail on the old code: `two_register_or_load_calls_make_one_post`, `a_kill_after_the_post_resumes_with_a_patch`,
`a_patch_404_registers_anew`, `a_failed_patch_deletes_the_new_device`,
`a_partial_file_with_an_unknown_state_is_refused`; the sixth is the child of the kill test) and
`cargo test -p podnet --lib -- warp::register warp::enrol` (`a_503_on_the_post_is_not_sent_to_the_second_host`
fails with the old condition; `a_held_lock_gives_exit_75_after_the_wait`). `a_503_...` is a lib test, not a
`--test warp_register` test, as the fallover needs two API hosts. `sh scripts/agent/check.sh`: fmt, clippy,
record, size, doc, layers 0. Not proven: the live WARP service; that a create is applied before a 5xx answer;
a kill between the answer of `POST` and the first save still loses that device (item 8). The WireGuard export
has one remote call, so it has no `created` state. `tunnel` and `warp --identity`-less runs still register
with no save. Frozen files changed: `src/warp/mod.rs`, `src/warp/register.rs`, `src/warp/export.rs`; new
`src/warp/enrol.rs`, `src/warp/hints.rs`.

# T-245: Publish `v0.4.0-beta.4`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The work after `v0.4.0-beta.3` (T-097 to T-100, T-118 to T-121, T-165, T-174, T-178, T-179, T-246, T-250, T-253, T-254, T-177, T-178, T-216 to T-218, T-221, T-222,
T-239 to T-243, the takes of podcrates v0.21.0 and v0.22.0) is on `main` only. A tester of a release binary does not
get it.

## Premise

- `v0.4.0-beta.3` is the newest release (T-235).
- `sh scripts/agent/check.sh` passes on `main` at 54bd958.
- The release adds `llms.txt` and `llms-full.txt` to the files and to `SHA256SUMS` (T-222). No release
  run made them yet.

## Approach

1. Set the version `0.4.0-beta.4` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.4.md`, with `## Breaking`.
3. Push, and wait until `ci` and `live` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.4`.
5. Check the files of the release and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-11, 9df5438 (the head of the release: ci 43068690494, live 43068690426, deny and secrets: success). The dry
run 43099493695 and the release run 43104434114 passed; the dispatch 43104388861 made the tag `v0.4.0-beta.4` at
9df5438. The release is a prerelease with the 8 binaries, `podnet.1`, `llms.txt` and `llms-full.txt` (the first
release with them, T-222), `SHA256SUMS`, `SHA256SUMS.sigstore.json`, `provenance.sigstore.json` and
`THIRD-PARTY-LICENSES.md`. `release-proofs.yml` 43107969792: success.


# T-254: Housekeeping: one footnote for cfrs, no narrative history

**Source:** the owner, 2026-11-11: "we still mention cfrs in many places rather than just one brief footnote and
done with it and never mention it again"; `CLAUDE.md` (Writing: keep only technical content).
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`git grep -i cfrs` finds the former name in 47 files: the documents, the manual, code comments and old
compatibility paths. The documents also hold history and stories ("before", "used to", dates of changes) that
are not technical content.

## Premise

- The release notes (`docs/releases/`), the reports (`docs/reports/`) and the record (`TODO/`,
  `docs/decisions.md`) are records of the past: they keep their text.
- The frozen files (`sh scripts/agent/podcrates.sh`; the freeze of PodUtils/podnet#449) take fixes only.

## Approach

1. Keep one brief footnote about the former name in README.md. Remove each other mention outside the records
   and the frozen files, and remove the compatibility aliases of the old names (beta: breaking is allowed).
2. Remove history, stories and opinions from the documents and the comments; keep the facts and the rules.
3. List each mention left in a frozen file for a COORD issue (to podcrates).

## Prove

```sh
git grep -n -i cfrs -- . ':!docs/releases' ':!docs/reports' ':!TODO' ':!docs/decisions.md'
sh scripts/agent/check.sh
```

The grep finds the footnote of README.md, the source repositories of `CLAUDE.md` (License) and the frozen files
only.

## Start condition

None.

## Correction

2026-11-11, fbfd6da and 32a66e5: the first pass. One footnote stays at the end of README.md. The other mentions
are gone from README.md, SECURITY.md, CLAUDE.md, `docs/maintenance.md`, `docs/target-environment.md`,
`scripts/`, `live.yml` (the A/B step against the binary of v0.3.0-beta.6 is gone) and the code; the fixtures of
v0.3.0-beta.1 (signed by the old repository) are gone, and the test of `podnet update` runs on the bundle of
v0.4.0-beta.1. No alias of an old name existed in the code. Kept: the source repositories of the License line
of `CLAUDE.md`, and `-e CFRS_BUILD_COMMIT` of `scripts/rebuild.sh` (a rebuild of a tag up to v0.3.0-beta.12
needs it). No frozen file named cfrs. The narrative of README.md, `docs/target-environment.md`, `src/update.rs`
and `src/status.rs` is cut; dead code is gone (`net.rs`, `run.rs`, `socks.rs`). Left: `docs/research/` (a
second pass).


## Done

2026-11-11, commits 14719c7 and the commit after it: the second pass. `docs/research/podnet/` writes `podnet`,
`PODNET_*`, `libpodnet-vnet.so`, `libpodnet-socks.so`, `crates/podnet-vnet/shim/`, `/__podnet/ws` and `podnet_pin`
where it named the old binary, variables, shims and paths. Each command of the studies was run with `--help` on
the build of this tree: all exist except proposals of the studies (`router`, `send`, `export`, `relay serve`,
`route status`); `export` is now `warp export`. `rename.md` is cut from 447 lines to the names of podnet by
kind, the self-update chain and the rules for a change of a name, with the current values; the entries of the
record cite the file, so it stays (`cargo todo check` checks each cited path). `architecture.md` has no section
of renamed names, no compatibility questions and no text of the rename step. The `README.md` of the studies says
that paths and line numbers are those of commit 3bed023. `git grep -c -i cfrs -- docs/research`: 9 files with
463 lines before, 1 file with 3 lines after. The 3 lines name the attachment directories and files of the field
study (`att/562a2c99-cfrs-standalone/cfrs/`, `att/522011b9-cfrs-field-report/`, `cfrs-field-report.md`,
`cfrs-field-evidence-seccomp-rst.log`) in `mwan-field.md`: they are names of source files, as the source
repositories are. Not done: the line references of the studies (for example main.rs:5446) are those of 3bed023 and
are not updated one by one.

# T-256: Publish `v0.4.0-beta.5`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The work after `v0.4.0-beta.4` (T-132, T-165, T-255, the take of podcrates v0.25.0) is on `main` only.
PodUtils/podrunner#64 waits for T-255 in a release.

## Premise

- `v0.4.0-beta.4` is the newest release (T-245).

## Approach

1. Set the version `0.4.0-beta.5` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.5.md`, with `## Breaking`.
3. Push, and wait until `ci` and `live` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.5`.
5. Check the files of the release and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-12, 8f53288 (the head of the release: ci 43113581495; live 43113581673, attempt 2: attempt 1 failed at
"The commands of the second batch" on HTTP 429 of the WARP service, error 1015). The dry run 43120242252 and the
release run 43122020833 passed; the dispatch 43121998362 made the tag `v0.4.0-beta.5` at 8f53288. The release is a
prerelease with the 8 binaries, `podnet.1`, `llms.txt`, `llms-full.txt`, `SHA256SUMS`,
`SHA256SUMS.sigstore.json`, `provenance.sigstore.json` and `THIRD-PARTY-LICENSES.md`. `release-proofs.yml`
43123633932: success.


# T-258: Docs: stale or misleading comments and records (GitHub #459)

**Source:** GitHub #459.
**Category:** docs
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Stale or misleading comments and records, found at `76e8b60`:

- `crates/podnet-core/src/net.rs:13-14`: a doc comment ("Install the ring crypto provider once per process...") sits
  above the doc comment of `claim_socket_path`, so rustdoc merges the two. No item `install_crypto_provider` exists.
- `src/bug_report.rs` `tail`: the doc says "The last `n` lines of the file". The code reads the whole file with
  `read_to_string`, so one invalid UTF-8 byte prints "(the log ... cannot be read)".
- `TODO/PROGRESS.md`: "the donor review (GitHub #429 to #434): done." The row of #433 is open and T-242 is partial.
- `TODO/issues.md`: #441, #442 and #443 are COORD issues; no entry describes them, as the rules require.
- Two lists of secret variables disagree: `crates/podnet-core/src/instances.rs` (`TUNNEL_TOKEN`, `PODNET_PROXY`,
  `PODNET_RELAY_CONNECT_TOKEN`) and `crates/podnet-core/src/report.rs` (`TUNNEL_TOKEN`, `PODNET_RELAY_TOKEN`,
  `PODNET_RELAY_CONNECT_TOKEN`, with the proxy variables in `PROXY_VARIABLES`).
- The text of #442 (airhand 9) cites `is_ours` at the wrong line, and #443 names `podwatch` (now `podstat`). These
  are the bodies of GitHub issues; the lead amends them.

## Premise

- Read: the files above at `76e8b60`. `withheld` and `secrets_of` are the only users of the two lists.
- A union of the two lists loses no secret. Effect: `PODNET_RELAY_TOKEN` stays out of the record of a detached
  instance, and `PODNET_PROXY` is always withheld from the error program (it was withheld only with a user part).
- Not measured: the rendered rustdoc.

## Approach

1. Delete the stray doc comment in `net.rs`.
2. `tail` reads bytes and uses `String::from_utf8_lossy`; the doc comment says what the code does.
3. `instances::SECRET_VARIABLES` is the one list (the union of four names); `report::withheld` uses it.
4. Correct the lines of `TODO/PROGRESS.md` and the rows of `TODO/issues.md`.

## Prove

```sh
cargo test -p podnet-core --lib each_secret_variable_stays_out_of_the_record
cargo test -p podnet-core --lib the_error_program_withholds_the_list_of_the_instances
cargo test -p podnet --lib tail_reads_a_log_with_an_invalid_byte
sh scripts/agent/check.sh
```

## Done

2026-11-11, 05af4d3. `net.rs`: the stray doc comment is gone. `bug_report::tail` reads bytes and uses
`String::from_utf8_lossy`; its doc says that it reads the whole file and keeps the last `n` lines.
`instances::SECRET_VARIABLES` is the one list (`TUNNEL_TOKEN`, `PODNET_PROXY`, `PODNET_RELAY_TOKEN`,
`PODNET_RELAY_CONNECT_TOKEN`); `report::withheld` uses it, and `PODNET_PROXY` left `PROXY_VARIABLES` there.
`TODO/PROGRESS.md` and the rows of #441 to #443 of `TODO/issues.md` are true. Tests (each failed on the old
code): `each_secret_variable_stays_out_of_the_record`, `the_error_program_withholds_the_list_of_the_instances`,
`tail_reads_a_log_with_an_invalid_byte`. `sh scripts/agent/check.sh`: all 0. Not proven: the rendered rustdoc;
the corrections of the bodies of #442 and #443 (the lead amends the issues).


# T-259: The user documentation by topic: a short README and one document for each area

**Source:** the owner, 2026-11-12
**Category:** docs
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The owner: "the readme for instance is too long, and doesn't even talk about half the capabilities we
have and also doesn't describe podnet accurately since now it is a swiss army knife, move dedicated to
docs/ by topic".

`README.md` has 1618 lines. It does not cover the router, the kernel mode, the forwards, `bench`,
`ssh-relay`, NetBird, the network registry, `--egress`, `--help --json`, the llms files or the JSON of
`doctor`. It describes podnet as the work of three tools.

## Premise

- `README.md` was read in full. The help of the binary at this commit is the source for the new text.
- `tests/docs.rs` pins README sections: the cell of `notebooks/install.ipynb`, the `--detach` sentence, the
  flags of `warp`, the release tags, and the paths in backticks.
- The README of a crate page cannot have relative links (`tests/docs.rs`): a link is a full URL.

## Approach

1. Make `README.md` short: what podnet is, install, quick start, a map of the capabilities with a link to
   each document, the list of the documents, the license and the note on the former name.
2. Move the detail of the README to topic documents in `docs/` and add the missing capabilities, each
   checked against the help of the binary.
3. Fix the links, the tests that pin README text (move each pin to the new document), the manual, the
   notebook, `CLAUDE.md` and `scripts/gen-llms.py`; regenerate `llms.txt` and `llms-full.txt`.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/gen-llms.py --check $CARGO_TARGET_DIR/debug/podnet
```

## Done

2026-11-12, 28da113. `README.md` has 145
lines: what podnet is, install, quick start, a map of the capabilities and the list of the documents. The detail
is in 16 topic documents in `docs/`: install, tunnels, serve, warp, pods, networks, proxies, relay, background,
doctor, cli, library, build (moved) and router, forward, egress (new). The capabilities that the README did not
cover (router, kernel mode, `--check --to`, `--status`, mwan-json, forward and the DNS door, `expose`,
`ssh-relay`, NetBird, the network registry and `--egress`, `bench`, `bug-report`, `--help --json`, the llms files,
the doctor JSON keys, the rules and the audit of `pod`) are written from the help of the binary and from the code.
`tests/docs.rs` reads the install cell, the `--detach` sentence and the warp flags in the new documents, and has
two new tests: the README is at most 250 lines and links each topic document, and each relative link of the topic
documents resolves. `scripts/gen-llms.py` lists the new documents; `llms.txt` and `llms-full.txt` are regenerated.
The proxy example with a user and a password in the old README is now `PODNET_PROXY="$PROXY_URL"` in `docs/proxies.md`.
Not proven: the new text was read against the help and the code, not run against each live service. GitHub #89
(the test guide) could not be read from the sandbox (HTTP 403).



# T-260: doctor: probes for the lanes `masque_udp443` and `ws_upgrade` (GitHub #461, A-C-10)

**Source:** GitHub #461 (https://github.com/PodUtils/podnet/issues/461), the tracker of the asks of cfb; ask A-C-10.
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

T-217 gives the seven lanes of `doctor --json`, but two lanes have no probe and always give `unknown`
(`src/doctor/json.rs`, `lanes`): `masque_udp443` ("no probe sends a datagram to MASQUE on UDP 443") and
`ws_upgrade` ("no probe makes a WebSocket upgrade to the edge host"). Ask A-C-10 wants `ok`, `fail` or
`unknown` with the time for each lane.

## Premise

- Read: `src/doctor/json.rs` (`LANES`, `lanes`), the Done section of T-217.
- `src/doctor.rs` is frozen (`pod-services` block of `sh scripts/agent/podcrates.sh`); `src/doctor/json.rs` is not.
- The hosts and ports of the MASQUE endpoint and of the edge are facts that podnet already has; add no new host.

## Approach

1. `masque_udp443`: a QUIC Initial to the MASQUE endpoint on UDP 443 with a short timeout; `ok` on any QUIC
   answer, `fail` on a refusal or a timeout with UDP open elsewhere, else `unknown` with the reason.
2. `ws_upgrade`: an HTTP/1.1 `Upgrade: websocket` request to the edge host over TLS (through the proxy when one
   is set); `ok` on `101`, `fail` on another status or a TLS failure.
3. Tests against local stand-ins (a UDP socket that answers, a TLS server that answers `101`), each failing first.
4. Update `docs/protocol.md` section 9 and `docs/doctor.md`.

## Prove

```sh
cargo test -p podnet doctor -- lanes
```

## Done

2026-11-12, commit 9814f2d. `doctor --json` gives a probe for each of the lanes `masque_udp443` and `ws_upgrade`. The code is in `src/doctor/lane_probes.rs`. `src/doctor.rs` (frozen) has only the field `Report.probes` and two calls in `run`.

- `masque_udp443`: one QUIC packet (`udp::probe`) to the MASQUE endpoint on UDP 443. `ok` on any answer (`ms` is the time). `fail` on a refusal, and on silence when the probe of UDP 7844 answers. `unknown` on silence when UDP 7844 does not answer. With a proxy for the endpoint no packet is sent: `unknown`, reason "no proxy carries UDP".
- `ws_upgrade`: an HTTP/1.1 `Upgrade: websocket` request over TLS to the target `edge-name` (the edge host on 7844, SNI `h2.cftunnel.com`, the roots of the edge), through the proxy when one is set. `ok` on `101`. `fail` on another status, a TLS fault, a refusal or a close with no status line. `unknown` when no answer comes in twice `--timeout`.
- No new host. Both probes run beside the paths. With `--quick`, a probe that is late gets `unknown` ("no answer in the time limit of --quick"). The keys of a lane stay `{verdict, ms, reason}`.
- Docs: `docs/protocol.md` section 9, `docs/doctor.md`, `llms-full.txt`.

Tests (each failed first against a stub that returned `unknown`): `doctor::lane_probes::tests::lanes_ws_upgrade_is_ok_on_101`, `lanes_ws_upgrade_fails_on_another_status`, `lanes_ws_upgrade_fails_on_a_tls_fault`, `lanes_ws_upgrade_fails_on_a_closed_port`, `lanes_masque_udp_is_ok_on_any_answer`, `lanes_masque_udp_fails_on_a_refusal`, `lanes_masque_udp_silence_is_a_fail_only_when_udp_is_open_elsewhere`, `lanes_masque_udp_with_a_proxy_is_unknown_and_sends_nothing`, `doctor::tests::the_lanes_hold_the_probes`. `doctor_json_has_the_seven_lanes` (`tests/doctor.rs`) checks both lanes through a test proxy. Not proven: the live edge (a real `ok` or `fail` of both lanes; no network here). The stand-ins answer with a version negotiation packet, not a full QUIC handshake. The edit of the frozen `src/doctor.rs` is a hand-off for podcrates (`pod-services`).

# T-312: tests/signals.rs: the test sends a stop signal before the ready of the command (flaky under load)

**Source:** the lead, 2026-11-12.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The test `tests/signals.rs::sighup_and_sigterm_stop_with_run_for_and_exit_0` failed under load in two full runs of `cargo test --workspace --no-fail-fast` on 2026-11-12 (the process did not stop; the `wait` limit). It passes when it runs alone.

## Premise

- Read: `tests/signals.rs`, `src/cli/signals.rs`, `crates/podnet-core/src/detach.rs` (`arm_stop`, `ready`), `src/cli/cmd/net/proxy.rs`.
- A command arms its stop signals at `detach::ready` or at its first wait. Before, each stop signal has the default action (issue 91). `net proxy` makes its socket before the ready.
- Measured: a SIGTERM sent when the socket file exists ends the process by the signal (rc 143) in 4 of 8 runs.

## Approach

1. Make the test wait for the ready line `podnet: proxy` on stdout, not for the socket and a fixed sleep.
2. Use a limit of 30 s for the stop.
3. Do not change what the test checks (exit 0, the log line).

## Prove

```sh
cargo test -q --test signals
```

## Done

2026-11-12, commit 78dec0d. Cause: the test. `start` waited for the socket file and then slept 300 ms. The socket comes before `detach::ready`, which arms the stop signals. On a loaded host the ready came after the 300 ms, and the stop signal had the default action (exit by the signal, not 0). The code is as designed (issue 91: the default action before the ready). Fix: `start` reads stdout and waits for the line `podnet: proxy`, then waits 200 ms; the stop limit is 30 s; the checks are the same.

Failure rate: before, 0 of about 500 runs under load (40 busy loops, 200 parallel runs; 24 busy loops with 150 runs; one-CPU pinning) failed, so the failure of the full run was not reproduced on demand. The window was shown with a signal at the socket: 4 of 8 runs ended by SIGTERM (rc 143). After: 0 of 200 runs under the same load. `cargo test -q --test signals`: 3 passed. Not proven: the failure message "did not stop" (a hang) was not reproduced; the measured fault is an exit by the signal.


# T-313: Publish `v0.4.0-beta.6`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The work after `v0.4.0-beta.5` (T-061, T-097, T-098, T-115, T-117, T-166, T-176, T-180, T-237, T-248, T-249,
T-258, T-259, T-260, T-312, and the takes of podcrates v0.27.0 to v0.29.0) is on `main` only.

## Premise

- `v0.4.0-beta.5` is the newest release (T-256).

## Approach

1. Set the version `0.4.0-beta.6` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.6.md`, with `## Breaking`.
3. Push, and wait until `ci` and `live` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.6`.
5. Check the files of the release and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-12, 64f1ed0 (the head of the release: ci 43183892180, live 43183892212, deny 43183892189). The dry run
43186824289 and the publish dispatch 43190711812 passed; the dispatch made the tag `v0.4.0-beta.6` at 64f1ed0, and the
release run 43190862473 passed. The release is a prerelease with the 8 binaries, `podnet.1`, `llms.txt`,
`llms-full.txt`, `SHA256SUMS`, `SHA256SUMS.sigstore.json`, `provenance.sigstore.json` and `THIRD-PARTY-LICENSES.md`.
`release-proofs.yml` 43193362865: success.


# T-314: Publish `v0.4.0-beta.7`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The work after `v0.4.0-beta.6` (T-043, T-049, T-083, T-162, T-163, T-315, T-316, T-317, and the takes of podcrates v0.30.0 to v0.32.0) is on `main`
only.

## Premise

- `v0.4.0-beta.6` is the newest release (T-313).

## Approach

1. Set the version `0.4.0-beta.7` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.7.md`, with `## Breaking`.
3. Push, and wait until `ci` and `live` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.7`.
5. Check the files of the release and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-12, b8514a3 (the head of the release: ci 43243606655, live 43243604963, deny 43243604316). The dry run
43246835031 and the publish dispatch 43249788105 passed; the dispatch made the tag `v0.4.0-beta.7` at b8514a3, and the
release run 43249820078 passed. The release is a prerelease with the 8 binaries, `podnet.1`, `llms.txt`,
`llms-full.txt`, `SHA256SUMS`, `SHA256SUMS.sigstore.json`, `provenance.sigstore.json` and `THIRD-PARTY-LICENSES.md`.
`release-proofs.yml` 43252551977: success.


# T-315: live: `relay connect` hangs on the runner after the take of pod-relay (GitHub #483)

**Source:** GitHub #483 (https://github.com/PodUtils/podnet/issues/483), the issue of `live.yml` for two failed runs.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

After the take of `pod-relay` (podcrates v0.30.0, 83e0c93), the step "SSH through the reverse relay" of `live.yml`
fails: the node connects to the relay, then `ssh` with `ProxyCommand=podnet relay connect NAME` waits about 2 minutes
and gets "Connection closed by UNKNOWN port 65535" (live 43203358263, attempts 1 and 2, at 59c0313). The step passed
at 64f1ed0 (v0.4.0-beta.6). The run 43209202451 at a2c1750 failed earlier, at "Quick tunnel through the WebSocket
relay": the quick-tunnel service answered HTTP 429 with Retry-After 3600 s (a rate limit of the service).

## Premise

- In this sandbox (egress through an HTTP proxy), `relay pair`, `relay node --to 127.0.0.1:PORT` and `relay
  connect NAME` against the live relay pass: the answer of the target comes back. The runner has a direct egress.
- podnet gives `pod-relay` no `h2` and no `quic` setting (`src/ws_relay.rs`, `settings`): the crate takes `auto`.
  `quic_road` races QUIC only to a relay host that the user named; the default relay is not one.
- a2c1750 makes the step print the log of one `relay connect -v` before `ssh`.

## Approach

1. Run `live` after the rate limit of the quick-tunnel service ends; read the log of the probe of a2c1750.
2. Find the road or the step of the open that waits on a direct egress. Fix it in podnet (the settings of the
   crate, the copy of the stream) or write the evidence to podcrates in a COORD issue.
3. Release `v0.4.0-beta.7` (T-314) only when the step passes.

## Prove

```sh
# live.yml, step "SSH through the reverse relay": relay-ssh-ok, parallel-1, parallel-2 and the banner.
```

## Correction

2026-11-12: the cause (PodUtils/podcrates#92): `pod_relay::reverse::operator` up to v0.31.0 writes
each frame of the node to the local side and flushes it only at the end of the session. `tokio::io::stdout()` keeps
bytes until a newline or a flush, so the binary KEXINIT of sshd stays in the buffer; `ssh` and sshd wait for each
other until sshd's `LoginGraceTime` (120 s). In podnet's sandbox the test answers ended in a newline. podcrates fixed
the crate on its `main` (f891d89; the next tag). podnet's glue wraps the output of `relay connect` in a writer that
flushes after each write (`src/relay_reverse/flush_each.rs`, test
`relay_connect_output_reaches_the_reader_with_no_newline`: with the flush off it fails). The entry is done when the
step of `live.yml` passes.

## Done

2026-11-12, f539ccd (the writer that flushes each frame; 0ff53a5 the rustfmt). `live` 43226426806 at 0ff53a5: the
step "SSH through the reverse relay" passed (`relay-ssh-ok`, the two parallel sessions, the banner), and `ci`
43226426336 passed. The fix of the crate (podcrates f891d89) comes with its next tag; podnet keeps the writer.


# T-316: A local stand-in of the quick-tunnel service and the edge, so that CI runs the quick tunnel with no call to Cloudflare

**Source:** the owner, 2026-11-12 ("can the quick tunnel test be stubbed/emulated with a self hosted one? for
instance headscale is used for stand in for tailscale"); the study in `.work/cf-edge-standins.md`.
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** L
**Status:** partial

## Problem

`live.yml` runs 25 steps of the quick tunnel against `api.trycloudflare.com` and the Cloudflare edge. The service
answers HTTP 429 (error 1015, Retry-After 3600 s) after many runs (T-315), and the steps run only in `live`, not on
each push.

## Premise

- No open-source, self-hostable Cloudflare Tunnel edge exists. `flolep2607/cctop` (MIT OR Apache-2.0) has a fake edge
  in its tests (`flolep2607/cctop:crates/tunnel/src/cloudflare/fake_edge/`: QUIC and HTTP/2 listeners, `registerConnection`, visitor
  streams, a fake quick-tunnel API). `cloudflare/cloudflared` has Go test fakes only.
- podnet has `--service-url` (the provision endpoint), `PODNET_EDGE` (replaces the discovery of the edge,
  `src/tunnel.rs`) and an in-memory fake edge (`src/cloudflare/rpc.rs`, `fake_edge`).
- The roots of the edge are built in (`src/cloudflare/`); no override exists. `src/cloudflare/` is frozen for
  `pod-cloudflare`: a test hook there is a change to report to podcrates.
- A stand-in tests podnet against its own reading of the protocol, not against a change of Cloudflare.

## Approach

1. A test support crate or module with: a fake provision API (`POST /tunnel`, the body of the service, a 429 mode);
   an edge listener over TLS on HTTP/2 that answers `registerConnection` and opens visitor streams; a front that
   maps the Host of a visitor to its tunnel. QUIC after HTTP/2.
2. A hook for the trust roots of the edge in tests (an environment variable read only with a test CA), written as a
   COORD issue (to podcrates) because `src/cloudflare/` is frozen.
3. Move the quick-tunnel steps of `live.yml` that test podnet (the proxy, no DNS, dropped 7844, `--detach`, `ps`,
   `--json`, `--auth-user`, the leg fallback) into `ci` against the stand-in. Keep one real quick tunnel per leg in
   `live`.
4. Move the study to docs/research/podnet (cf-edge-standins.md). Keep the notice of cctop with each part of its code.

## Prove

```sh
cargo test --workspace -- quick_tunnel_standin
```

## Correction

2026-11-12. Step 2 of the Approach (a hook for the trust roots of the edge) is not needed. `SSL_CERT_FILE`
(and `--ca-file`) gives the only platform roots of podnet, and the edge adds its own roots to them
(`crates/podnet-core/src/trust.rs`, `edge_tls_config` in `src/cloudflare/mod.rs`). A test CA in
`SSL_CERT_FILE` makes the stand-in edge valid. `src/cloudflare/` is not changed, and no COORD issue to
podcrates is needed. The QUIC leg is not built (step 1, "QUIC after HTTP/2"). The move of the steps of
`live.yml` (step 3) is not done in this pass.

## Done

2026-11-12, commit 8f7b1d4 (`T-316: a stand-in of the quick-tunnel service and of the HTTP/2 edge for
tests`). Status `partial`. `tests/standin/` holds the fake provision API (`POST /tunnel`, a script of
answers with a 429 and `Retry-After`), the HTTP/2 edge (TLS on TCP with no ALPN, the control stream,
`registerConnection` with the check of the account tag and the secret, `unregisterConnection`, visitor
streams), the front (`Host` to tunnel, 530 for an unknown host), a CONNECT proxy and a test CA. The tests
are in `tests/quick_tunnel_standin.rs`: `cargo test --workspace -- quick_tunnel_standin` passes (13
tests, 2.6 s). The study is `docs/research/podnet/cf-edge-standins.md`. Covered: the visitor request over
HTTP/2, 429 with `Retry-After` and exit 75, no retry of a 403, `--json` lines, SIGTERM after the ready
line, `--detach` with `ps`, `logs` and `stop`, `--auth-user`, the CONNECT proxy, the fallback from QUIC to
HTTP/2, a refused registration, and the refusal of the edge certificate without the test CA. No fail-first
run: podnet has no change. Not proven: the QUIC leg, the verify of the public URL, the WARP and relay
legs, and a change of the real edge. Open: the QUIC leg; the move of the steps of `live.yml` into `ci`
(the lead); one real quick tunnel for each leg stays in `live`.


# T-322: doctor: resolve the ssh-relay probe through the configured proxy (GitHub #504)

**Source:** GitHub #504 (https://github.com/PodUtils/podnet/issues/504)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet doctor --ssh-relay HOST:PORT --ssh-user USER` builds the target of the flag path with `proxy: None`
(`src/cli/cmd/host.rs`). The probe honors a proxy when the target has one, and the path of a running instance fills the
field from `--connect-proxy`. The flag path never does. On a host whose only egress is an HTTP CONNECT proxy, the
probe dials directly and fails before the relay answers. `podnet ssh-relay --connect-proxy HOST:PORT` works on the
same host. The row prints no proxy, so the output does not show why the probe failed.

The issue asks:
- Split `HOST:PORT` and call `crate::egress::proxy_for(&host, port)`. `None` is a direct dial.
- `RelayConfig.proxy` is an HTTP CONNECT authority (`HOST:PORT`). Convert an HTTP proxy only. For a SOCKS5 proxy, a
  unix carrier or an exec carrier, the row says that the probe needs an HTTP CONNECT proxy and does not dial.
- `RelayConfig` has no proxy credentials and the relay uses `connect_tunnel`, which sends no `Proxy-Authorization`.
  A resolved HTTP proxy with credentials gives a refusal that says the probe needs an unauthenticated HTTP CONNECT
  proxy. A proxy that answers 407 appears as a refusal, not a timeout.
- A flag `--connect-proxy HOST:PORT` (requires `--ssh-relay`, validated as `ssh-relay --connect-proxy`) overrides the
  proxy. Precedence: the flag, then the global `--proxy` and `PODNET_PROXY` (`egress::set_proxy`), then the
  environment resolver. `NO_PROXY` applies. No loopback exception: follow `egress::rules()`.
- The text row prints the `HOST:PORT` authority or `proxy: none`. `--json` has the key `proxy` (`Proxy::label()`, no
  credentials; `null` for none) in the row `ssh_relay`.
- The target of a running instance stays as it is.

## Premise

- `src/cli/cmd/host.rs` (`doctor`) sets `proxy: None` for the flag path. `src/doctor/ssh_relay_check.rs` has
  `SshRelayTarget` (field `proxy`), `ssh_relay`, `judge`, `to_json` and `target_of_record`.
- `crate::egress::proxy_for(host, port) -> Option<Proxy>` exists (`src/egress.rs`). `egress::rules()` has
  `loopback_direct: false`: a loopback relay goes through the proxy too. `pod_proxy::Proxy` has `kind()`,
  `carrier()`, `has_credentials()`, `address()` and `label()`.
- `podnet_core::connect::connect_tunnel` returns a `ConnectionRefused` error with the status line for a status that is
  not 200, so a 407 is a refusal at the stage `connect`.
- `tests/doctor.rs` passes `--proxy http://USER:PASS@127.0.0.1:PORT` to all its tests and probes a loopback relay
  (`doctor_ssh_relay_row_probes_the_relay_of_the_flags`). With this change that proxy has credentials, so the test
  needs `NO_PROXY` for the loopback relay.
- `src/egress.rs` is frozen for `pod-egress`: this entry only calls `proxy_for`.
- Not measured: a live relay behind a proxy.

## Approach

1. In `ssh_relay_check.rs`: a function that chooses the proxy (flag, else the resolved `Proxy`) and returns the
   authority or the refusal text. `SshRelayTarget` gets a field `refusal`. The row of a refusal is `fail`, has no
   probe, and does not dial.
2. `host.rs`: the flag `--connect-proxy` (requires `ssh_relay`), validated with `relay::check_address`; the flag path
   calls the function with `egress::proxy_for`.
3. The text row and `--json` (`proxy`) carry the proxy. Update `docs/doctor.md`, `docs/relay.md` and the help.

## Prove

```sh
cargo test -p podnet --lib doctor::tests::the_ssh_relay
cargo test -p podnet --test doctor_ssh_relay
cargo test -p podnet --test doctor doctor_json_keys_are_fixed
```

Tests: `the_ssh_relay_proxy_is_the_flag_then_the_resolved_proxy`,
`the_ssh_relay_proxy_refuses_socks5_credentials_unix_and_exec`, `the_ssh_relay_row_shows_the_proxy`,
`the_ssh_relay_row_of_a_refused_proxy_does_not_dial`, `doctor_ssh_relay_row_dials_through_the_proxy`,
`doctor_ssh_relay_row_uses_the_proxy_of_the_environment`,
`doctor_ssh_relay_row_shows_a_proxy_that_answers_407_as_a_refusal`,
`doctor_ssh_relay_row_refuses_a_proxy_with_credentials`, `doctor_ssh_relay_row_refuses_a_socks5_proxy`,
`doctor_connect_proxy_needs_ssh_relay`.

## Done

2026-11-12, commit eba9df5. `SshRelayTarget::of_flags` (`src/doctor/ssh_relay_check.rs`) takes the flag
`--connect-proxy`, else the proxy of `egress::proxy_for(host, port)`, else none. `choose_proxy` gives an HTTP
proxy on TCP with no credentials as `HOST:PORT`; a SOCKS5, unix or exec proxy, and a proxy with credentials, are a
refusal that names the proxy with no credentials (row `fail`, no dial). A 407 is `fail` at the stage `connect`.
The text row has `proxy HOST:PORT` or `proxy none`; the JSON row has `proxy` (`http://HOST:PORT` or `null`).
The flag `--connect-proxy` requires `--ssh-relay` and is checked as `HOST:PORT` (exit 64). The existing test
`doctor_ssh_relay_row_probes_the_relay_of_the_flags` now sets `NO_PROXY=127.0.0.1`: the process proxy of the tests
has credentials, and `egress::rules()` sends loopback through the proxy.
- Fail first: with the resolver disabled (the old `proxy: None`), `doctor_ssh_relay_row_uses_the_proxy_of_the_environment`,
  `doctor_ssh_relay_row_refuses_a_proxy_with_credentials` and `doctor_ssh_relay_row_refuses_a_socks5_proxy` fail.
  The unit tests and the test of `--connect-proxy` did not compile before the change (no function, no flag).
- Pass: `cargo test -p podnet --lib doctor::tests` 17 passed; `cargo test -p podnet --test doctor_ssh_relay` 8 passed (the tests of the
  SSH relay row moved from `tests/doctor.rs`, the size limit); `--test doctor` 6 passed.
- Not proven: a live relay behind a proxy that answers 407, and an exec proxy (the parser takes it from a trusted
  origin only; unix and SOCKS5 are tested).


# T-318: ssh-relay: try the authentication methods the relay offers, and verify an RSA host key (GitHub #505)

**Source:** GitHub #505 (https://github.com/PodUtils/podnet/issues/505)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`podnet ssh-relay` and the probe of `doctor` offer one method: `authenticate_publickey` with an ephemeral Ed25519 key.
Three relays of `pod-services` fail with it:

| Relay | What the server offers | What podnet reports |
| --- | --- | --- |
| Serveo | publickey, keyboard-interactive | `relay rejected publickey auth: Failure { remaining_methods: MethodSet([PublicKey, KeyboardInteractive]), partial_success: false }` |
| localhost.run | none, password, publickey, hostbased, keyboard-interactive | `relay rejected publickey auth: Failure { remaining_methods: MethodSet([None, Password, PublicKey, HostBased, KeyboardInteractive]), partial_success: false }` |
| Pinggy | publickey, password, any key accepted | `handshake failed: Wrong server signature` |

The first two need a client that tries the methods the server offers. Pinggy presents an RSA host key. podnet depends
on `russh` with `default-features = false` and `features = ["ring"]`; the feature `rsa` is missing, and the
likely cause is that the RSA signature check is not compiled in. The client key stays Ed25519.

The issue asks:
- In `relay.rs` and `relay/probe.rs`, try in the order of `PodUtils/podcrates`,
  the file forward.rs of the crate pod-services-watch (`authenticate`): publickey with the ephemeral key; `none`, to read the
  list of methods; keyboard-interactive with an empty answer to each of up to four information requests, when the
  list names it. Stop at the first success. When all fail, one error names the methods that the server offered and the
  last failure. Do not retry a method that the server did not offer.
- Add `rsa` to the features of `russh` in `crates/podnet-serve/Cargo.toml` and in the root `Cargo.toml`.
- Fix the comment in `relay.rs` that says the relay "rejects none".
- No key store, no known-hosts file, no password attempt.
- Acceptance: a local server that offers keyboard-interactive only; a local server with an RSA host key (4096 bits);
  without the feature `rsa` the error `Wrong server signature` shows, with it the error is gone; `--service serveo`,
  `--service localhost-run` and `--service pinggy` reach the banner; `doctor --ssh-relay` is `ok` for the same three
  local servers; a server with no usable method gives an error that names every offered method.
- Blocked (for the `--service serveo` acceptance only): the verification of the Serveo URL rule in `pod-services`.

## Premise

- `crates/podnet-serve/src/relay.rs` (`open`) and `crates/podnet-serve/src/relay/probe.rs` (`stages`) call
  `authenticate_publickey` once. `russh = { version = "0.65", default-features = false, features = ["ring"] }` is in
  `Cargo.toml` (root) and `crates/podnet-serve/Cargo.toml`.
- The sequence is in the podcrates tree (v0.32.0, the file forward.rs of the crate pod-services-watch, `authenticate`). It
  needs no fact of `pod-services`: it invents no secret and it reads the method list from the server.
  `pod-services` v0.32.0 holds no fact about authentication, so the order stays in code.
- `tests/doctor.rs` has `fake_ssh_relay` (a russh server that takes any key).
- Not measured: a live relay (Serveo, localhost.run, Pinggy), and the cause `rsa` (the entry measures it with a test).

## Approach

1. A new file a new module auth of the relay with `authenticate(handle, user, key)`; `open` and the probe
   call it. Its error names the offered methods and the last failure.
2. Add the feature `rsa` to both manifests.
3. Tests with a local russh server: keyboard-interactive only; an RSA 4096 host key; no usable method. Show the
   failure of each test before the change, and of the RSA test without the feature.
4. Update the comment of `relay.rs`, `docs/relay.md` and `docs/doctor.md`.

## Prove

```sh
cargo test -p podnet-serve --lib relay::auth_tests
cargo test -p podnet --test doctor_ssh_relay
```

Tests: `a_relay_that_offers_keyboard_interactive_only_takes_the_empty_answer`,
`a_relay_that_accepts_none_takes_no_key`, `a_relay_with_an_rsa_host_key_completes_the_handshake`,
`a_relay_with_no_usable_method_names_the_offered_methods`,
`the_probe_names_the_offered_methods_of_a_relay_that_takes_none`, `the_probe_logs_in_with_keyboard_interactive`,
`the_probe_completes_the_handshake_with_an_rsa_host_key`.

## Done

2026-11-12, commit 27f0808. `relay::auth::authenticate` (`crates/podnet-serve/src/relay/auth.rs`) tries the key
of the run, then `none` (it reads the list), then keyboard-interactive with an empty answer (4 rounds) when the list
names it; `open` and the probe call it. A refusal names the offered methods and the last failure. The feature `rsa`
of `russh` is on in both manifests. The probe row has the key `auth_method`. `doctor` shows the method.
- Fail first (before the change): the keyboard-interactive, `none`, no-method and probe tests failed with
  `relay rejected publickey auth: Failure { remaining_methods: MethodSet([KeyboardInteractive]) ... }`.
- RSA: the test server and the client share one build, so without the feature `rsa` the server cannot sign and the
  client sees `handshake failed: early eof`, not `Wrong server signature`. The test fails without the feature and
  passes with it (key: a 4096-bit throwaway key, `crates/podnet-serve/src/relay/rsa4096.test-key`).
- Pass: `cargo test -p podnet-serve --lib relay::` 35 passed; `cargo test -p podnet --test doctor_ssh_relay` 8 passed.
- Not proven: a live relay (Serveo, localhost.run, Pinggy), and that `Wrong server signature` of Pinggy is the
  missing feature `rsa` (the cause is likely, not measured). The acceptance `--service serveo` waits for the
  verification of the Serveo URL rule in `pod-services`.


# T-319: ssh-relay: a remote port from a flag, and from the service data (GitHub #506)

**Source:** GitHub #506 (https://github.com/PodUtils/podnet/issues/506)
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet ssh-relay` always asks for the remote port 0: `handle.tcpip_forward("0.0.0.0", 0)`. The Sandhole demo
refuses port 0 and takes port 80. The service data has no `remote-port` fact, and `RelayConfig` has no field for one.

The issue asks:
- `RelayConfig::remote_port: Option<u16>`.
- `relay/facts.rs` reads the optional fact `remote-port` through the alias `podnet_core::services::forward::Relay`
  and puts it in `Facts`.
- `relay.rs` calls `tcpip_forward("0.0.0.0", u32::from(config.remote_port.unwrap_or(0)))`.
- A flag `--remote-port PORT` (1 to 65535; 0 is a usage error) overrides the fact. Port 0 only when neither is set.
  `--server` with no `--service` has no fact. The `--json` key `remote_port` keeps the port that the server returns.
- Blocked (the fact): the `pod-services` fact and accessor `remote-port` of Sandhole.

## Premise

- `crates/podnet-serve/src/relay.rs` (`open`) asks for port 0. `Facts` in `crates/podnet-serve/src/relay/facts.rs`
  has no port. `src/cli/cmd/ssh_relay.rs` builds the `RelayConfig`.
- `pod_services::forward::Relay` of podcrates v0.32.0 has `endpoints`, `user`, `url_rule`, `session_limit` and
  `keepalive`: no `remote_port`. The file the data file of Sandhole in pod-services has no such fact. A relay fact
  is data of `pod-services`, not code of podnet (T-166): the fact is an ask to podcrates.
- Not measured: the Sandhole demo (a live relay).

## Approach

1. The field `RelayConfig::remote_port`, the call with the port, the flag `--remote-port` (range 1 to 65535), the
   docs. A local russh server that refuses port 0 and takes port 80 tests it.
2. The fact: ask podcrates (COORD, to podcrates) for `remote-port` in `sandhole.toml` and the accessor
   `forward::Relay::remote_port`. Then `facts.rs` reads it and the flag overrides it. The entry is `partial` until
   then.

## Prove

```sh
cargo test -p podnet-serve --lib relay::auth_tests::a_relay_that_refuses_port_0
cargo test -p podnet --bin podnet ssh_relay_remote_port
```

Tests: `a_relay_that_refuses_port_0_takes_the_remote_port_of_the_config`,
`ssh_relay_remote_port_is_one_to_65535`, `ssh_relay_remote_port_reaches_the_config`.

## Correction

2026-11-12, commit d32d8d6: Approach 1 is done. `RelayConfig::remote_port` and the flag `--remote-port PORT`
(clap range 1 to 65535: `0` and `70000` are usage errors that name `1..=65535`) exist. `open` asks for that port,
else 0, and `TunnelHandle::remote_port` is the requested port (the reply of the relay holds a port only for a
request of port 0). Fail first: `a_relay_that_refuses_port_0_takes_the_remote_port_of_the_config` failed with the
field present and unused (`remote_port` 0, not 80). Passed: `relay::` 35 tests; the two CLI tests.
Left (Approach 2): the fact. `pod-services` v0.32.0 has no `remote-port` fact and `forward::Relay` has no accessor,
and podnet adds no relay fact in code (T-166). The ask for podcrates: the fact `remote-port` in the data file of
Sandhole (80) and `forward::Relay::remote_port() -> Option<u16>`. Then `facts.rs` puts it in `Facts` and
`ssh_relay.rs` uses it when no flag is given. Not proven: a live Sandhole demo.

2026-11-12: podcrates did the fact (PodUtils/podcrates#101): `sandhole.toml` has `remote-port` = 80, and `pod_services::forward::Relay::remote_port() -> Option<u16>` reads it. It comes with the next tag; the take reads it into the facts of the relay and lets `--remote-port` override it.

## Done

2026-11-12, commit 3e4c999 (`T-319: ssh-relay: the remote-port fact of the relay`). Approach 2 is done with podcrates
v0.33.0 (main takes it). `relay/facts.rs`: `Facts::remote_port` is `forward::Relay::remote_port()`. `ssh_relay.rs`:
the port of the config is `--remote-port`, else the fact, else port 0 (`--server` with no `--service` has no fact).
`docs/relay.md` and the help of `--remote-port` say it. Tests: `relay::tests::the_remote_port_is_a_fact_of_the_relay_that_needs_one`
(sandhole 80; pinggy and serveo none), `relay::auth_tests::a_relay_that_refuses_port_0_takes_the_remote_port_fact` (the fake
SSH relay that refuses port 0 takes the port of the fact: 80), and
`cli::cmd::ssh_relay::tests::ssh_relay_takes_the_remote_port_fact_and_the_flag_overrides_it` (`--service sandhole`: 80; the flag
8080 replaces it; pinggy, the default and `--server` give none). Fail first: with `Facts::remote_port` always `None`, the three
tests failed (`left: None, right: Some(80)`, and `remote forward refused`); with the fact they pass. Not proven: a live
Sandhole demo (a live relay).


# T-323: live: the WebSocket step leaves its echo server, and waits 30 s for the DNS of a new name (GitHub #511)

**Source:** GitHub #511 (https://github.com/PodUtils/podnet/issues/511), the issue of `live.yml` for two failed runs.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Two `live` runs failed in a row. Run 43262551243 at 8ce65ba failed at "WebSocket through a --url origin":
`scripts/live-ws-url.sh` runs twice in the step (QUIC, then HTTP/2) on the fixed port 18095, and its EXIT trap
(`kill $(jobs -p)`) does not see the jobs in `sh`, so the echo server of the first run stays and the second one gets
"address already in use"; the client then got "Name or service not known" for the new tunnel name 10 times in 30 s.
Run 43259640036 at b17b907 failed at "The commands of the second batch": `warp scan` found no front door of WARP
answering (162.159.198.1, 162.159.198.2, 2606:4700:103::2), a state of the WARP service.

## Premise

- Read: `scripts/live-ws-url.sh`, the logs of the two jobs.

## Approach

1. The echo server binds port 0 and writes its port to a file; the trap kills the PIDs that the script records.
2. The client tries 30 times (90 s), as the wait of the tunnel's ready line.

## Prove

```sh
sh -n scripts/live-ws-url.sh
# live.yml, step "WebSocket through a --url origin": "ws-url quic" and "ws-url http2" echo.
```

## Done

2026-11-12. `scripts/live-ws-url.sh`: a free port for each run, the PIDs killed at the end, 30 tries of the client.
The echo server with port 0 was run here with `websockets` (the client got its echo). Not proven here: the step in
`live` (the next run). The WARP door scan is a state of the service, not of podnet.


# T-324: doctor: the test of a refused direct dial holds its port

**Source:** the lead, 2026-11-12: `doctor::tests::a_direct_refusal_is_an_answer` failed once in a full run (the run of
T-261 and T-263) and passed alone.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The test bound a listener on port 0, dropped it, then dialed the port and expected a refusal. Between the drop and
the dial, another test of the same run can bind the free port, and the dial then connects.

## Premise

- Read: `src/doctor/tests.rs`, `a_direct_refusal_is_an_answer`.
- On Linux a connect to a port that a socket holds with `bind` and no `listen` is refused.

## Approach

1. Hold the port with a `TcpSocket` that is bound and does not listen, for the whole test.

## Prove

```sh
cargo test -p podnet --lib a_direct_refusal_is_an_answer
```

## Done

2026-11-12. The test holds the port with a bound `tokio::net::TcpSocket` and no `listen`; the dial gets "connection
refused". 5 runs in a row passed. Not proven: the race under the load of a full run (it was seen once).


# T-325: Publish `v0.4.0-beta.8`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The work after `v0.4.0-beta.7` (T-261 to T-267, T-274, T-275, T-307, T-309, T-311, T-316, T-318, T-319, T-321 to T-324, and
the take of podcrates v0.33.0) is on `main` only.

## Premise

- `v0.4.0-beta.7` is the newest release (T-314).

## Approach

1. Set the version `0.4.0-beta.8` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.8.md`, with `## Breaking`.
3. Push, and wait until `ci` and `live` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.8`.
5. Check the files of the release and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-12, commits fb47f09 (`T-325: version 0.4.0-beta.8 and its release notes`) and 97687dc (T-326: the ignore of
RUSTSEC-2023-0071; the first `deny` run 43295725441 failed on it). On 97687dc: `ci` 43321245519, `live` 43321245503 and
`deny` 43321245726 passed. `release.yml`: the dry run 43324008185 passed, the publish 43326653552 passed. The release
`v0.4.0-beta.8` is a prerelease with 15 files. `release-proofs.yml`: run 43328735562 passed.


# T-326: Remove the ignore of RUSTSEC-2023-0071 when russh takes `rsa` 0.10.0

**Source:** the `deny` run 43295725441 on fb47f09 (2026-11-12): `advisories FAILED` on `rsa` 0.10.0-rc.18.
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

RUSTSEC-2023-0071 (the Marvin attack: a timing leak of the private key in RSA decryption and
signing) now names `rsa` 0.10.0-rc.18, the fix is `rsa` 0.10.0. Each russh release up to 0.65.1 pins
`rsa = "=0.10.0-rc.18"`, so `cargo update -p rsa` changes nothing. The `deny` check fails, and the
release of `v0.4.0-beta.8` waits for it.

## Premise

- podnet makes Ed25519 keys (`src/transport/ssh.rs`, `crates/podnet-serve/src/relay.rs`,
  `crates/podnet-serve/src/git/ssh.rs`).
- An RSA private key is used only when the user gives one: the host key of the git server
  (`PrivateKey::read_openssh_file`) or a key of the SSH relay.

## Approach

1. Now: `deny.toml` ignores RUSTSEC-2023-0071 with its reason and date; `docs/decisions.md` has the call.
2. When a russh release takes `rsa` 0.10.0: update russh, remove the ignore, and run
   `cargo deny --locked check advisories`.

## Decision

The lead's call (2026-11-12, `docs/decisions.md`, Advisories): ignore the advisory until russh moves.
No fixed version is reachable, and podnet does not make RSA keys.

## Prove

```sh
cargo deny --locked check advisories bans sources
```

## Blocker

Each russh release up to 0.65.1 pins `rsa = "=0.10.0-rc.18"`. A russh release that takes `rsa` 0.10.0 ends the block.

# T-331: Publish `v0.4.0-beta.9`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md` (Releases, 2026-11-09): cut the next prerelease when a batch of fixes is green;
`docs/maintenance.md`, section "Release".
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The work after `v0.4.0-beta.8` (T-268, T-285 to T-289, T-291, T-293, T-306, T-308, T-310, T-320, T-327, T-329, T-330, and
the takes of podcrates v0.34.0 to v0.36.0) is on `main` only.

## Premise

- `v0.4.0-beta.8` is the newest release (T-325). `ci`, `live` and `deny` passed on 4f79aa6.
- The release adds a file: `pod-report.schema.json` (T-330).

## Approach

1. Set the version `0.4.0-beta.9` in `Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.9.md`, with `## Breaking`.
3. Push, and wait until `ci`, `live` and `deny` pass on that commit.
4. Dispatch `release.yml` on `main` with `publish` empty (the dry run), then with
   `publish=v0.4.0-beta.9`.
5. Check the files of the release (with `pod-report.schema.json`) and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next
prerelease unattended when a batch is green.

## Prove

```sh
cargo test -p podnet --test docs
sh scripts/agent/check.sh
```

## Done

2026-11-13, commits 6be9e76 (`T-331: version 0.4.0-beta.9 and its release notes`) and f566e3b (the notes name T-289
and T-329). On f566e3b: `ci` 43363112482, `deny` 43363112416 and `secrets` 43363112643 passed; `live` 43363112516
failed once at the WARP door scan (an external flake) and passed on its rerun. `release.yml`: the dry run 43368235597
passed, the publish 43370009549 passed. The release `v0.4.0-beta.9` is a prerelease with 16 files, with
`pod-report.schema.json` (T-330). `release-proofs.yml`: run 43371748745 passed.
