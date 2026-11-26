This file holds the entries of milestone M5: the defects and the small gaps that the field report
of 0.3.0-beta.12 (`docs/reports/field-report-2026-11-05.md`), the lab kit, the section "Open
work" of `docs/maintenance.md` and the seccomp ideas of the vn kit found.
`docs/research/podnet/mwan-field.md`, section 4, ranks them. The lab kit's reports are
attachments, not files of the repository: `docs/research/podnet/mwan-field.md`, section 2.3,
quotes them. These entries can start at any time from M0. An entry that changes a file of a split
of M2 lands before that split starts, or after it ends. Names: before T-016 a variable is
`CFRS_X`; after it, `PODNET_X`. A fix in a module that podcrates already holds, or that
T-137 plans to move (`src/warp/`, `src/resolve.rs`, `src/egress.rs`, `PodUtils/podnet@7fe60e0:src/cloudflare/`), lands in
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
src/warp/tun/rp_filter.rs at `9c1fc4b`: 0 for the interface after the create, at each session (`tun::run`) and
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
`src/warp/proxy.rs` to src/warp/proxy/names.rs at `9c1fc4b`. The text of `ps` moves from `src/main.rs` to
`crates/podnet-core/src/instances/text.rs`. The test of `--list` is `warp_forward_list_prints_the_forwards_of_ps`.

## Done

2026-11-06, commit bbfe527 (`warp: the pins of --resolve come first, ps prints each forward, warp
forward --list (T-073)`). `pinned(host)` in `src/resolve.rs` gives the pins of `--resolve` and
`PODNET_RESOLVE`. `resolve_all` and `resolve` (src/warp/proxy/names.rs at `9c1fc4b`) return the pins before
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
- Read: the test `seccomp_closed_sockets_free_their_entries` (`tests/pod_seccomp_fds.rs:267`) makes
  5000 sockets that never send. It cannot see the leak.
- Not measured: the count of 3 per socket. No build ran in this triage.
- Existing tests have a SOCKS5 UDP-in-stream stand-in (`seccomp_backend_relays_udp_in_the_stream`, `tests/pod_seccomp/udp.rs`).

## Approach

1. Move `ensure_udp_relay`, `relay_datagram` and `UdpRelay` out of `seccomp.rs` into a new file
   (the file is at its cap and cannot grow).
2. Give each `Sock::Datagram` a stop signal (a `tokio::sync::watch` sender or a `CancellationToken`).
   Select on it in both reader loops. Drop of the entry sends it.
3. Send the stop from `handle_close`, `sweep` and the shutdown near `drain_relays` (`seccomp.rs:851`).
4. Correct `crates/podnet-netns/README.md:157-159` so that it separates `read` and `write` on TCP from
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
on the pair (`crates/podnet-netns/README.md:157-159`). That is true for TCP only.

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
3. Update `docs/proxies.md:58-63`: the pool bound, case-insensitivity, SERVFAIL at the end.

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
- Read: the ruling of 2026-11-04 (`docs/decisions.md:40`): a read of a connected socket stays direct,
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
  `tests/docs.rs:95-106` reads eight documents, not these.
- Read: the live plan is the record. T-040 (`TODO/split.md:827`) still says that `seccomp.rs` has 1,900
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

- Read: `verify_fetch` (`PodUtils/podnet@7fe60e0:src/tunnel.rs:1182-1263`). The Dns arm (`PodUtils/podnet@7fe60e0:src/tunnel.rs:1228-1231`) and the other `Err` arm
  (`:1232-1235`) only change the retry text. After the loop only `last_status` can demote the verdict
  (`:1253-1258`). Otherwise `fault(Unavailable)` (`:1259-1262`).
- Read: the three front ends each shut down and return the error (`PodUtils/podnet@7fe60e0:src/cli/cmd/run.rs:17-198`).
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

2026-11-09, commit ebc94ea. `verify_fetch` (now `PodUtils/podnet@7fe60e0:src/tunnel/verify.rs`) tracks failures before the edge: Dns hop, or a proxy `Refusal` of CONNECT. When each attempt failed so, it returns `Verified { unverified: Some("resolution") }` and the tunnel stays up (also for a body token). The ready document has `unverified`; `run_demo` prints NOT PROVED. `PODNET_VERIFY_STRICT=1` gives exit 69. HTTP 530 stays fatal. Tests: `a_name_that_does_not_resolve_keeps_the_tunnel_up` (skips when a proxy of the host answers), `a_proxy_refusal_is_a_resolution_failure`, `a_failed_verify_exits_69_after_the_budget`. The CLI test `tunnel_ready_document_marks_an_unresolved_name` is not written: it needs a live tunnel. CI: at the merge.

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

- `src/cli/opts.rs:61-63` declares it (`global = true`); `src/cli/opts.rs:177` sets `config.verify`.
- `src/cli/cmd/run.rs:23,69` read `config.verify` and gate `verify_accepting`; `PodUtils/podnet@7fe60e0:src/tunnel.rs:1182`
  runs the fetch. `crates/podnet-core/src/net.rs` takes `no_tls_verify` from the ingress rule. The two flags share no state.
- `grep -n -e '--no-verify' README.md docs/*.md` finds nothing. `wants_verify` (`PodUtils/podnet@7fe60e0:src/tunnel.rs:1172`) has no caller.
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
**Status:** done

## Problem

On a host with no IPv4 route, `tunnel --transport warp` tries the IPv4 door first (device record order),
fails with `Network unreachable`, then the IPv6 door, and can reach the WebSocket relay first. `warp`
has `--masque-endpoint`; `tunnel` has none.

## Premise

- `warp_targets` builds the client with no `with_masque` (`PodUtils/podnet@7fe60e0:src/tunnel.rs:827-838`). Neither
  `TunnelOpts` (`src/cli/opts.rs:21-109`) nor `TunnelArgs` has the flag. `with_masque` exists
  (src/warp/mod.rs:264 at `9c1fc4b`); `warp` uses it (`src/cli/cmd/warp/client.rs:67-74`).
- `open_on` (src/warp/mod.rs:762-791 at `9c1fc4b`) uses `endpoint_order` (`:103-124`): the record order, IPv4 first.
  `has_default_route(true)` (v6) only decides whether the IPv6 door is included.
- The log lines of the issue are from beta.6 on one host. The silent-drop timing is read, not measured.

## Approach

1. Fix: in the `endpoint == None` branch, with no IPv4 default route (`has_default_route(false)`) and no
   won door, put the IPv6 door first. Dual-stack keeps the order. Log the family of a failed door.
2. Flag: `--masque-endpoint IP:PORT` in `TunnelOpts`, through `Config`, to `warp_targets`
   (as `--warp-identity`, `src/cli/opts.rs:176`, `src/tunnel.rs:157,262`).

## Decision

The lead, 2026-11-09: item 1 (the family order) is a fix and lands now. Item 2 (the flag `--masque-endpoint`) is a feature in code that moves to `pod-warp` and `pod-cloudflare`: send it as a COORD issue (to podcrates) and take it with the crates (T-050, T-052). The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet endpoint_order_puts_v6_first_without_a_v4_route
cargo test --test cli -- tunnel_takes_masque_endpoint
```

## Correction

2026-11-09, commit 243720f. Item 1 (the family order) is done: `endpoint_order` (now src/warp/door.rs at `9c1fc4b`) puts the IPv6 doors first with no IPv4 default route, no proxy, no relay and no door that won. Test: `warp::door::tests::endpoint_order_puts_v6_first_without_a_v4_route`. Item 2 (`--masque-endpoint` in `tunnel`) waits for pod-warp and pod-cloudflare (T-050, T-052) by the Decision; the COORD issue (to podcrates) is for the lead to open. The test `tunnel_takes_masque_endpoint` is not written. CI: at the merge.

## Done

2026-11-13, commit 59f6b60. Item 2: `tunnel` (and `share`, `serve`, `demo`: the option is global) takes `--masque-endpoint IP:PORT`. It goes through `Config::masque_endpoint` and `warp_targets` (now `PodUtils/podnet@7fe60e0:src/tunnel/warp_targets.rs`) to `MasqueOptions::endpoint`, which is `Door::Pinned` of `pod-warp` v0.37.0 (a wrong value is exit 64). The doc and the profile (`docs/protocol.md`, section 10) list the flag. Item 1 was done on 2026-11-09 (see Correction). Test: `cargo test --test cli -- warp::tunnel_takes_masque_endpoint` (help, a bad value, a good value with `demo --check`; not run on the old code: the old binary has no such flag and clap exits 2). Not proven: a live WARP session on a host with no IPv4 route.

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

- `PodUtils/podnet@7fe60e0:src/quicktunnel.rs:258-260` (budget and attempts), `PodUtils/podnet@7fe60e0:src/quicktunnel.rs:292-312` (the arms; `Hop::Dns` goes to
  `ProvisionError::Http(_)` at `:312`).
- `crate::resolve::answering_step(host)` (`src/resolve.rs:146`) names the step that answers now.
- `src/quicktunnel.rs` is Cloudflare adapter code (pod-cloudflare, T-029/T-052): a fix is allowed.
- Not measured: the 30 s run of the issue.

## Approach

1. Retry a `Hop::Dns` failure once. Retry again only when `answering_step` of the API host changed.
2. End with the `Hop::Dns` hint (`crates/podnet-core/src/hop.rs:49`).

## Prove

```sh
cargo test -p podnet quicktunnel::tests::a_dns_failure_is_retried_once
```

## Done

2026-11-09, commit 99aa461. `provision` retries a `Hop::Dns` failure once, then again only when `answering_step` of the API host changed (`PodUtils/podnet@7fe60e0:src/quicktunnel/retry.rs`). The error ends with the `Hop::Dns` hint through the hop of the error. Test: `quicktunnel::retry::tests::a_dns_failure_is_retried_once` (the decision, not a live run). CI: at the merge.

# T-171: The verify loop names the remedy when the public name does not resolve (GitHub #367)

**Source:** GitHub #367 (https://github.com/PodUtils/podnet/issues/367)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`verify_fetch` retries 180 s (`VERIFY_BUDGET`, `PodUtils/podnet@7fe60e0:src/tunnel.rs:1267`). On a host with no resolver the
error text repeats `system resolver: no answer ... Network unreachable`. The final error
(`PodUtils/podnet@7fe60e0:src/tunnel.rs:1259-1262`) names no remedy: `--resolve HOST=IP`, or `--no-verify`.

## Premise

- The loop keeps `last = "waiting for DNS: ..."` (`PodUtils/podnet@7fe60e0:src/tunnel.rs:1228-1231`). `Verified.dns` records the
  step on success only. `Hop::Dns` has a hint (`crates/podnet-core/src/hop.rs:49`).
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
**Status:** done

## Problem

`check_private` refuses an identity file whose owner is another uid, also with `--identity PATH`. The
state directory has `PODNET_FOREIGN_HOME=allow`; the identity file has no override. Root cannot read the
device that a service user saved. A second device spends a registration (`docs/decisions.md`, WARP devices).

## Premise

- src/warp/mod.rs:885-892 at `9c1fc4b` (owner and mode checks, bare `bail!`); readers: `load` (`:357`) and
  `src/warp/export.rs:68`. `register_or_load` (`src/warp/mod.rs:421-423`) refuses and does not write over the file.
- `crates/podnet-core/src/state.rs:69-127` has the model. `src/manual/facts.rs:163` lists `PODNET_FOREIGN_HOME`.
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

2026-11-09, commit 5a21d81 (`T-173: a WARP identity of another uid is a Config error (exit 78)`). Done as the Decision says: item 2 (the exit code). `check_private` is in src/warp/private.rs at `9c1fc4b`; an owner of another uid gives `Fault::Config`. `warp_client_inner` and the carry and WireGuard peers keep a class that the library gave (the outer `Fault::Unavailable` hid it; this also gives a directory as `--identity` the exit 64 that its test names). A refused file leads to no registration. Tests: `warp::private_file_tests::foreign::{a_foreign_owner_is_a_config_error, a_refused_identity_leads_to_no_registration}` (the second one chowns the file, as root only), `cli::cmd::warp::client::tests::a_class_of_the_library_stays`. The message does not name `PODNET_FOREIGN_IDENTITY`: the variable has no effect until item 1 lands, and a message must not name a variable that does nothing. Item 1 (allow the owner mismatch with the variable, keep the mode check, warn) and the update of `src/manual/facts.rs` wait: a COORD issue (to podcrates) is to be opened by the lead (a worktree agent writes no issue); podnet takes it with `pod-warp` (T-050). Item 2 for the state-directory error: it names `PODNET_FOREIGN_HOME` already. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

## Done

2026-11-13, commit 59f6b60. Item 1: `PODNET_FOREIGN_IDENTITY=allow` gives `pod-warp` `Foreign::Allow` (`src/warp/foreign.rs`) for an identity file that the user named (`--identity`, `--warp-identity`). The default file of the state directory is never covered. The mode check stays (`pod-warp`); podnet logs a warning, and writes no door file and no leg file next to the file. The refusal (exit 78) names the variable. `src/manual/facts.rs`, `docs/warp.md` and `docs/tunnels.md` are updated. Tests (as root; they skip with a line otherwise): `warp::foreign::tests::{a_foreign_identity_needs_the_consent_variable, the_mode_is_checked_with_the_consent, the_consent_is_for_a_named_path_only}`; with the consent disabled in the code, the three fail (`belongs to user 65534 ... it is not used`). Not covered: the reader of the exported device `warp-wg.json` (`src/warp/export.rs`) keeps refusing a foreign owner; `pod-warp` takes the lock of the file (`PATH.lock`) before the check of the owner, so a refusal leaves a lock file next to the foreign file (ask for podcrates). Not proven: two real users.

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

- `Task` has one `host` and one `port`, and `Check::Tls` is one handshake (`src/agent.rs:26-42`). The three rows are at `src/agent.rs:60-65`, `:74-79` and `:88-93`. The table prints one target (`src/agent.rs:542-562`).
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

- `src/egress.rs:45-46` sets 8080 and `rules()` passes it as `http_port` (`src/egress.rs:50-61`); `parse()` uses it (`:67`).
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

`podnet doctor` builds the relay row with `Relay::from_env(None)` and keeps it only when the URL starts with `https://` (`src/doctor.rs:163-174`). The user cannot pass the relay on the command line, and a loopback relay (`http://127.0.0.1:PORT`, allowed by `src/ws_relay.rs:119-171`) gives no row. `bug-report --doctor` has the same table. `--transport relay` is not seen either.

## Premise

- `--relay` and `--transport` are `global = true` in `TunnelOpts` (`src/cli/opts.rs:41-51`). `TunnelOpts` is flattened only into `tunnel` and `share` (`src/cli/cmd/tunnel.rs:17`, `src/cli/cmd/share.rs:22,62,114`). `DoctorArgs` has no `--relay` (`src/cli/cmd/host.rs:14-33`). A global flag does not reach a sibling command, so `doctor --relay X` most likely fails in clap. Not run (no build).
- `targets()` has no argument (`src/doctor.rs:115`). `doctor()` calls `Options::default()` with three fields set (`src/cli/cmd/host.rs:286-305`).
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

A client that connects and sends nothing, or one byte of a SOCKS5 greeting, holds its task and socket until the process ends. Each connection is unauthenticated. The accept loops have no cap. The WARP door has the fix (`REQUEST_LIMIT`, 30 s, src/warp/proxy.rs:586 at `9c1fc4b`). `CLIENT_TIMEOUT` (300 s) wraps only the copy.

## Premise

- `crates/podnet-vnet/src/proxy.rs:84-96`: `fill_buf`, `socks5_read_request` and `http_read_connect` have no timeout. The copy is wrapped at `:118-125`. Accept loops: `:272-297`.
- `crates/podnet-netns/src/tailscale.rs:377,383,387`: same reads. The dial and the copy have a timeout (`:386`, `:406`).
- `crates/podnet-netns/src/frontdoor.rs:79,121`: same reads in the door of `pod --serve` (not in the issue).
- `src/warp/proxy.rs:628-630,657-661,703-706`: the precedent.
- Not run.

## Approach

1. One constant `REQUEST_LIMIT` (30 s) in `podnet-netns` (`socks.rs`). Wrap the first byte and the request read in each of the three doors. The warp door uses the same constant.
2. On expiry send a SOCKS5 reply `0x04` or an HTTP `504`, then close (`fail_with`, `crates/podnet-vnet/src/proxy.rs:162-181`).
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

`bind_unix` of the tailnet door removes the path and binds (`crates/podnet-netns/src/tailscale.rs:354-357`). It deletes a regular file and takes over a live socket. The ruling "Sockets" (`docs/decisions.md:22`): a live socket is 69, a file that is not a socket is 64, a stale socket is removed. The default path is `unix:/tmp/podnet-socks.sock` (`src/cli/cmd/net/mod.rs:80`).

## Premise

- Five callers use `claim_socket_path` (src/warp/proxy.rs:85 at `9c1fc4b`, `crates/podnet-vnet/src/switch.rs:437`, `crates/podnet-vnet/src/proxy.rs:319`, `src/cli/cmd/connect.rs:250`, `src/cli/cmd/relay.rs:283`). The function is in the root crate (`crates/podnet-core/src/net.rs:20-51`): `podnet-netns` cannot call it.
- Two more listeners do `remove_file` then `bind_private`: `crates/podnet-netns/src/frontdoor.rs:95` and `crates/podnet-netns/src/seccomp/accept.rs:60`.
- `PodUtils/podnet@7fe60e0:crates/podnet-core/src/net.rs` has no test of `claim_socket_path` (tests start at `PodUtils/podnet@7fe60e0:crates/podnet-core/src/net.rs:174`).
- `net_tailscale` opens the LocalAPI socket before the bind (`src/cli/cmd/net/shim.rs:98-104`).

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

`RealTarget::parse` takes only a `SocketAddr` (`src/cli/cmd/net/proxy.rs`, before this entry) and `localhost:3000` gives `bad real address`. The help and `docs/virtual-network.md:180-181` say `host:port`. The dial is a bare `TcpStream::connect` (same file): the proxy of the process, `NO_PROXY` and the resolver chain do not apply, and the warn has a bare `io::Error` (`crates/podnet-vnet/src/proxy.rs:211`). Measured by the issue: a dead `--proxy` changed nothing.

## Premise

- `connect_origin` (`PodUtils/podnet@7fe60e0:crates/podnet-core/src/net.rs:103-128`) resolves names and `localhost`. `egress::dial` (`src/egress.rs:243-269`) uses the proxy and `NO_PROXY`. `split_host_port` is private (`PodUtils/podnet@7fe60e0:crates/podnet-core/src/net.rs:94-98`).
- `rules()` sends the loopback through the proxy (`loopback_direct: false`). A forward to `127.0.0.1` would then use the proxy. The Decision rules it: the loopback stays direct.
- The `F: Fn() -> Fut` shape of `forward` stays (`crates/podnet-vnet/src/proxy.rs:186-190`).

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

`forward` binds inside a spawned task and returns a `JoinHandle` (`crates/podnet-vnet/src/proxy.rs:186-204`). `net_proxy` prints `forward VIRT -> REAL` at once (`src/cli/cmd/net/proxy.rs:118`). An address outside the subnet, a duplicate `--forward`, an ACL refusal or a collision with a shim bind give the line, a log warn and exit 0. `--import` binds before it prints (`crates/podnet-vnet/src/switch.rs:722-726`).

## Premise

- Refusals: `PodUtils/podcrates:crates/pod-netstack/src/stack/reactor.rs` (`AddrNotAvailable`, `PermissionDenied`, `AddrInUse`).
- `spawn_forwards` cannot return a result (`crates/podnet-vnet/src/proxy.rs:238-245`).
- The `--map` entry outside the subnet fails only at the first request (note of the issue: not verified).

## Approach

1. Bind first (async `forward`, or a `oneshot`). Return the result to `net_proxy`.
2. Map `AddrNotAvailable` to `Fault::Usage`, `AddrInUse` and `PermissionDenied` to `Fault::Unavailable`. Print the line after the bind.
3. Reject a repeated `VIRT` at parse time with its name.
4. Keep the per-connection warn at `crates/podnet-vnet/src/proxy.rs:211-213`.

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

`Table::forward` drops a packet when the queue of the peer is full and ignores the error (`let _ = peer.try_send(...)`, `crates/podnet-vnet/src/switch.rs:164,171`). The queue is 1024 packets (`:30`, `:468`). No counter, no log, no accessor. The `spoofed` counter (`:153`) has no accessor. The rule of the project: a drop is counted (`NetStats::dropped`, `PodUtils/podcrates:crates/pod-netstack/src/stack.rs`). The fault injector counts are not readable either (`crates/podnet-vnet/src/switch.rs:54-111`).

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

`NetStack::over_link` builds `LinkDevice::tunnel(config.mtu)`: both queues are unbounded and the stack does not hold when full (`NetStack::over_link`, `LinkDevice::tunnel`; now `PodUtils/podcrates:crates/pod-netstack/src/stack.rs`, `PodUtils/podcrates:crates/pod-netstack/src/device.rs`). A slow pump grows the heap. The WARP stack uses `tunnel_bounded` and `with_inbound_limit` (src/warp/ip.rs:454-455 at `9c1fc4b`, 512 and 4096 packets).

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

`NEEDS` (`crates/podnet-vnet/src/doctor.rs:122-132`) lists `warp proxy`, `connect -L` and `net proxy` for `listen(AF_INET)`, and `warp proxy` and `connect -L` for `listen(AF_UNIX)`. Missing: `net switch` (`crates/podnet-vnet/src/switch.rs:413-415`), `net tailscale` (`tailscale.rs:321-322`) and `relay connect --listen` (`src/cli/cmd/relay.rs:177,190-192`). `net proxy`, `net switch` and `net tailscale` default to a unix path (`src/cli/cmd/net/mod.rs:65,138,196`). `docs/target-environment.md:156` repeats the short list.

## Premise

- Printed at `src/cli/cmd/net/mod.rs:318-319`. `tests/cli/main.rs` has `doctor_tells_listen_from_bind`.
- `docs/networks.md:40-42` uses `net switch --listen tcp://` as its example.

## Approach

1. Rewrite `NEEDS` with each command and the default form. Fix `docs/target-environment.md:156`.
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

`PodUtils/podnet@7fe60e0:crates/podnet-core/src/exit.rs:122-139` promises 69 for "a service or a network path is not available". A TCP bind that fails gives exit 1: `crates/podnet-vnet/src/switch.rs:413-415`, `crates/podnet-netns/src/tailscale.rs:320-321`, `src/cli/cmd/relay.rs:281`, and also `crates/podnet-vnet/src/proxy.rs:263-265` and src/warp/proxy.rs:71-73 at `9c1fc4b` (not named in the issue). `exit.rs:152` reads only `Faulted`.

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

`--map NAME=ADDR` builds one `HashMap` for the door only (`src/cli/cmd/net/proxy.rs:92-104`). The stack gets `dns: None` (`StackConfig::default`, now its `gateway_service`: `PodUtils/podcrates:crates/pod-netstack/src/stack.rs`), so nothing listens on `10.66.0.1:53` (`PodUtils/podcrates:crates/pod-netstack/src/stack/reactor.rs`, `PodUtils/podcrates:crates/pod-netstack/src/stack/service.rs`). `net shim` has an empty `hosts` (`src/cli/cmd/net/shim.rs:43-60`, `crates/podnet-vnet/src/shim.rs:53,82-90`). `docs/virtual-network.md:336-341` and `net addresses` advertise the resolver.

## Premise

- `vnet::dns::Resolver` exists and is tested: `from_lines` (`crates/podnet-vnet/src/dns.rs:57`), `hosts_pairs` (`:168`), fields `upstream` and `catch_all` (`:26,29`) that nothing sets.
- `crates/podnet-vnet/src/stack.rs` allowlist (2103 lines): no new test in that file before T-034. A test can use the public `StackConfig.dns` from `crates/podnet-vnet/src/dns.rs` or `tests/`.

## Approach

1. Build one `Resolver` from `--map`. Pass it as `StackConfig.dns` (switch path) and to the door through `hosts_pairs()`.
2. `net shim --map NAME=ADDR` (repeatable) fills `ShimOptions::hosts`.
3. `--map-file PATH` with `Resolver::from_lines`. Validate each address against the subnet at parse time.
4. Update the help and `docs/virtual-network.md:336-341`. The use of `upstream` and `catch_all` is a separate ruling: do not wire them here.

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
- The test `a_local_side_that_does_not_read_stops_no_other_session` (`tests/relay_reverse.rs:326`) reaches only the binary-frame arm. The fake relay sends `close` at `tests/relay_reverse/fake.rs:226`.
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

`relay stop NAME` stops each local instance of the pair, then calls the relay (`src/cli/cmd/relay.rs:322-346`). `reverse::stop` refuses an empty stop token (`src/relay_reverse.rs:153-155`). The command then prints one line per stopped instance and exits 69. The pair file stays and the relay keeps the pair for up to 72 h. A pair with no stop token is a valid state (`src/relay_reverse.rs:55,111`, fixture at `tests/relay_reverse.rs:24-37`).

## Premise

- The local-first order has a good reason (`src/cli/cmd/relay.rs:331-332`): a stopped node does not connect again.
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

`session` ends when the local read returns 0 (`src/relay_reverse.rs` line 632 at `1a1e729`, select at lines 654-657 at `1a1e729`) and sends `close` at once (line 659 at `1a1e729`). The operator side keeps the session open after its input ends (`EOF_IDLE`, lines 535 and 623 at `1a1e729`). A target that shuts down its write side and still reads loses the rest of the input of the operator. The operator exits 0 (a close with code 1000 is success, line 655 at `1a1e729`). The forward leg has the right shape (`crates/podnet-serve/src/relay.rs:279-313`).

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

`save` opens the pair file with `.mode(0o600)` (`src/relay_reverse.rs` lines 149-154 at `ed79896`). The kernel applies the mode only to a new file. An old file of mode 0644 keeps it, and `relay pair` prints `mode 0600` (`src/cli/cmd/relay.rs:147`). `save` follows a symlink. `load` uses `read_to_string` (line 172 at `ed79896`) with no check of type, owner, mode or size. The file holds three tokens. The token cache has all four checks and a rename write (src/relay_pool.rs lines 163-195 at `ed79896`; the file went to `pod-relay` of PodUtils/podcrates).

## Premise

- `state::replace_file(path, bytes, mode)` writes through a rename, sets the mode and replaces a symlink (`crates/podnet-core/src/state.rs:427-442`).
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

`docs/protocol.md:210` says the alternates are the first 3 hosts of the pool. The text after it says the default relay uses 6 hosts until a pool is cached. The code takes 3 (`MAX_ALTERNATES`, src/relay_pool.rs lines 39 and 119 at `7c5f87e`; the file went to `pod-relay` of PodUtils/podcrates). The seed has 6 endpoints (`ajam-relay.toml`). A connect with no pool tries the primary and 3 hosts. The stagger is 2 s each (`src/ws_relay.rs:206`).

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

- `RelayCommand::Connect` has no `DetachArgs`: `src/cli/cmd/relay.rs:66-112`. `RelayCommand::Node` has it: `:61-62`.
- `detach_of` has no arm for it: `src/cli/opts.rs:265-274`. `connect` has one: `:256`.
- Both accept loops use `accepted?`: `src/cli/cmd/relay.rs:277`, `:289`. `connect -L` logs and returns:
  `src/cli/cmd/connect.rs:208-211`.
- The unix branch calls `claim_socket_path(&path, false)` with no guard: `src/cli/cmd/relay.rs:283`.
  `SocketFile` is in `src/cli/cmd/connect.rs:219-239` (`pub(crate)`).
- Message: `crates/podnet-core/src/net.rs:26-32`.
- `src/main.rs:157` allows `--replace` without `--detach` only for `Command::Connect`.

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

- src/transport/websocket.rs:181-185 (direct dial, `host` and `port` dropped), `:142-150` (`rsplit_once(':')`),
  `:193-197` (TLS).
- The precedent: `PodUtils/podnet@7fe60e0:src/ws.rs:120-123`, `PodUtils/podnet@7fe60e0:src/ws.rs:155` (`crate::egress::dial`), `PodUtils/podnet@7fe60e0:src/ws.rs:161` (ALPN `http/1.1`);
  `src/ws_relay.rs` lines 605-619 at `98d8a71`; `Host::parse`, lines 137-149 at `98d8a71`.
- `grep -rn WebSocketTunnel src` finds only the module and one test (src/transport/mod.rs:474).

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

- `grep -rn root_404 src` gives 3 lines: `src/cli/opts.rs:118` (field), `:128` (`root_404: false`),
  `src/cli/cmd/run.rs:43` (read).
- `serve` passes `args.common.show()`: `src/cli/cmd/serve.rs:225`.
- The warn branch: `PodUtils/podnet@7fe60e0:src/tunnel.rs:1218-1222`. `static_router` gives 404 at `/` with no index: `crates/podnet-serve/src/files/mod.rs:73-137`.
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

- `src/cli/cmd/run.rs:94-104` (no `stamped`). The only `stamped` call of `run.rs` is `:291` (`announce`).
- `grep -rn "status::stamped" src/cli` gives 8 hits, none in `run_share`.
- The `verified` line is unstamped in every command (`run.rs:119`, `:166`). It stays as it is.

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

- Guard: `src/cli/cmd/tunnel.rs:192` (`if args.config.is_none()`). The flags are read at `:194-218`, `:301`.
  The three per-origin flags are declared at `:62-73`.
- The token case already has a usage error: `src/cli/cmd/tunnel.rs:167-183` (issue 172).
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

- `crates/podnet-core/src/detach.rs:166-192` (`child_begin` reads the variable). `:536-543` sets 4 detach variables and not this one.
  `crates/podnet-core/src/detach/supervise.rs:120` is the only writer (the supervisor).
- `crates/podnet-core/src/autostart.rs:177-181` copies each `PODNET_` variable except `PODNET_ERROR_PROGRAM`.
- `crates/podnet-core/src/detach.rs:714-721` (`targets()` gives `Vec::new()` for this record).
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

- `src/cli/cmd/agent.rs:14-35` (`Status` with no fields), `:207-213` (`instances_ps(false)`, `agent::text`).
- `src/cli/cmd/instances.rs:41-47` (`instances_ps(json)` prints its own document).
- `src/cli/cmd/host.rs:272-282` (stamped report). `src/agent.rs:143-145` (`Serialize`). `docs/proxies.md:64-70`.
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
- `crates/podnet-serve/src/files/mod.rs:257-272`, `:170`. `PodUtils/podnet@7fe60e0:crates/podnet-serve/src/ingress.rs:200-203`, test `PodUtils/podnet@7fe60e0:crates/podnet-serve/src/ingress.rs:635-648` (`"a + stays a +"`).
- `docs/serve.md:76-77`.
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

- `src/cli/cmd/warp/up.rs:19-29` (the `tun: true` arm matches `..`), `:434-446` (`warp_up_tun_args`), `:238-242` (the
  proxy path pushes `--replace`).
- `src/cli/cmd/warp/args.rs:421-465` (the flags), `:434` (`conflicts_with = "persist"` on `tun`), `:559-574` (`Tun` has
  `DetachArgs`, so `--replace` exists there, and has no `--dns`).
- `crates/podnet-core/src/detach.rs:511-518` (the refusal text).

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
then restarts a working tunnel. `agent status` has the same text (`src/agent.rs:539`).

## Premise

- src/warp/live/instance.rs:70-76 at `9c1fc4b` (`instance_state` reads `INSTANCE`), :88-105 at `9c1fc4b` (`instance_state_in` takes a name),
  :131-158 at `9c1fc4b` (the line). `src/warp/up.rs:24-26` (two names), `:529-534` (`down` knows both).
- `src/cli/cmd/warp/account.rs:51-96` (the arm `Status` of the account file).
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

- src/warp/identity.rs:157-171 at `9c1fc4b`. Users: `src/cli/cmd/warp/account.rs:57`, `:105-108`, `:150-153`. Test: `src/cli/cmd/warp/cli_tests.rs:128-131`.
- The issue says the fixture type `limited` is a free type. This is not shown: src/warp/register.rs:627 at `9c1fc4b` is a test of
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

2026-11-09: the measurement of Approach 1 is not possible in this session. No record of the `account_type` values of the live service exists in the tree: `grep -rn account_type docs tests src` finds the report line `docs/reports/aerrand-agent.md:639` (type `free` with `warp_plus` true), the fixtures src/warp/register.rs:627 at `9c1fc4b` and `src/cli/cmd/warp/cli_tests.rs:130` (`limited`, made up), and no list of the paid types. A registration spends a device and needs the live service (`docs/decisions.md`, WARP devices). The code stays as it is. Start condition: one `podnet warp status --json` of a free device and one of a device with a licence, recorded in `docs/reports/`.

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

`docs/warp.md:99-104` says: "UDP (SOCKS5 UDP ASSOCIATE) is not served, and the rules do not apply." The code applies
`-r`, `--rules-file` and `--rule-default` to `--l4`. The `--help` text says so.

## Premise

- `docs/warp.md:94-104`. src/warp/proxy.rs:507-527 at `9c1fc4b` (the dial closure: `rules::current()`, `decide_static`, three actions).
- `src/cli/cmd/warp/args.rs:365-369` (help). src/warp/rules.rs:304-309 at `9c1fc4b`, tests :316-331 at `9c1fc4b`.
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

- src/warp/register.rs:185-194 at `9c1fc4b` (`Endpoint`; frozen file: no edit). src/warp/export.rs:245-263 at `9c1fc4b`, `:30` (`WG_PORT`).
- `src/warp/wg.rs:62-69` (v4 only; the message `:65-67`).
- MASQUE reads `v4` and `v6`: src/warp/register.rs:558-597 at `9c1fc4b`.
- `grep -rn "\.ports" src/warp` finds no reader.
- Test to extend: src/warp/export.rs:402-437 at `9c1fc4b`.

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

- src/warp/nc.rs:301-310 at `9c1fc4b` (raw `{exe}` at :292 at `9c1fc4b`). `src/warp/ssh_config.rs:34-36`, `:65-73`.
- The only test of the printed block: src/warp/nc.rs:363-367 at `9c1fc4b` (plain path).
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

- `src/warp/proxy.rs:194-216` (`serve_dns`), `:138-149` (`admit`), `:159-186` (`health_log`), `:109-124` (`Stats`),
  `:382-385` (length check in `forward_dns_to`).
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

2026-11-09, commit be1c8dc (`T-215: warp proxy --dns has a bound and a counter`). `serve_dns` moved to src/warp/proxy/dns_bound.rs at `9c1fc4b` (`src/warp/proxy.rs` 1902 to 1878 lines). A semaphore of 256 places (`MAX_DNS_TASKS`) bounds the queries in flight; an excess datagram is dropped and counted in `Stats.dns_dropped`, which the health line prints. A datagram under 12 bytes starts no task. One task for each query; the first answer wins. Tests: `warp::proxy::dns_bound::tests::{a_burst_gives_at_most_the_bound_in_flight, a_short_datagram_starts_nothing}`. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

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

- Read: `src/doctor.rs:249` (`to_json`) writes `ok`, `roads`, `legs` (`leg`, `verdict`, `status`, `reason`) and `names`, and `status::stamped` adds `schema` 1 and `podnet`. No test pins the key set.
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

- Read: `src/doctor.rs:227` (`roads`) groups the legs by road. The legs have their own names, not the seven lane names. A leg has a time (`ms`) in the paths, not in the legs.
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
gives the whole manual. `pod` keeps its own help. scripts/gen-llms.py BINARY writes `llms.txt` (the
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

- `crates/podnet-netns/src/listen.rs:127-240` (`bind_private`) calls `UnixListener::bind(path)`, then
  `set_permissions(path, 0o600)`.
- The callers that bind in a directory that other users can reach: `src/cli/cmd/connect.rs:251`,
  src/warp/proxy.rs:86 at `9c1fc4b`, `crates/podnet-vnet/src/proxy.rs:320`, `crates/podnet-vnet/src/switch.rs:438`,
  and, through `bind_claimed` (`crates/podnet-netns/src/listen.rs:375-378`),
  `crates/podnet-netns/src/tailscale.rs:356` and `crates/podnet-netns/src/frontdoor.rs:95`.
- `crates/podnet-netns/src/seccomp/accept.rs:61` is not exposed: it calls `private_dir` first
  (`crates/podnet-netns/src/seccomp/accept.rs:58`), and it removes the old file before the bind (`crates/podnet-netns/src/seccomp/accept.rs:59`).
- The defaults in `/tmp`: `src/cli/cmd/net/mod.rs:80`, `src/cli/cmd/net/mod.rs:97` and
  `src/cli/cmd/net/mod.rs:177` (and `/tmp/podnet-vnet.switch` at `src/cli/cmd/net/mod.rs:248`).
- `libc` is a dependency (`Cargo.toml:197`, `crates/podnet-netns/Cargo.toml:24`). No code calls
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
- `scripts/install.sh:260-291` (`resolve_tag`): the branches `v*) echo "$1"` and `*) echo "v$1"` pass the
  text through. The line `scripts/install.sh:90` builds `base` from it.
- `scripts/install.sh:371-383` (`check_sum`): the `awk` prints each matching hash, and the compare is
  against the joined text.
- The rule that exists: `PodUtils/podnet@749da3d:src/update.rs:167-174` (`valid_tag`: 1 to 64 characters, first `v` or a digit, no
  `..`), used by `PodUtils/podnet@749da3d:src/update.rs:185`, `PodUtils/podnet@749da3d:src/update.rs:271` and `PodUtils/podnet@749da3d:src/update.rs:348`.
- `PodUtils/podnet@749da3d:src/update/cosign.rs:160-174` (`sum_of`) returns the first matching line and does not check the length
  of the hash. SHA256SUMS is signed, so this is a hardening gap, not a hole.
- `tests/install_script.rs:105-115` (`Lab::curl`): the argument loop skips each word that starts with `-`,
  so no flag can change a result. `Lab::run` clears the environment and sets no `TMPDIR`; one test sets it
  (`tests/install_script.rs:408`). The script passes `--proto` at `scripts/install.sh:1048` and
  `scripts/install.sh:1072` (after T-360).
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
`PodUtils/podnet@749da3d:src/update/cosign.rs` does the same. Tests: `strict::curl_gets_https_only_flags`,
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
- src/warp/route.rs:364-370 at `9c1fc4b`: `enum Dns { Resolved, File { backup: String, link: Option<PathBuf> } }`.
  src/warp/route.rs:574-596 at `9c1fc4b` (`set_dns`) reads the backup from whatever file is there
  (src/warp/route.rs:592-593 at `9c1fc4b`), with no check of the marker. Only `Drop for Routes`
  (src/warp/route.rs:508-538 at `9c1fc4b`) restores. src/warp/route.rs:618-629 at `9c1fc4b` (`replace_file`) uses
  `std::fs::write`; no code sets the mode. src/warp/route.rs:566-572 at `9c1fc4b` (`resolv_text`) already writes the
  marker line `# podnet warp tun --default-route: DNS through WARP`.
- The call of `Routes::setup` is in `src/cli/cmd/warp/tun.rs:101`. The name of the instance is
  `TUN_INSTANCE` (`src/warp/up.rs:26`), and `instances::is_alive` answers from the held lock
  (`crates/podnet-core/src/instances.rs:311`). The detached child holds the lock under that name, so the
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
- `src/warp/mod.rs:421-423` (`register_or_load`): `check_file_writable` at `src/warp/mod.rs:456`, the
  registration at `:457`, the save at `:458`, with no lock between. A stored device that is not enrolled
  for MASQUE counts as "register again", not as "resume" (`src/warp/mod.rs:444-452`).
- src/warp/mod.rs:202-241 at `9c1fc4b` (`register_with`): `create_device`, then `enrol_masque`; the token stays in
  memory.
- The call sites with no lock: `src/cli/cmd/warp/client.rs:153`, `PodUtils/podnet@7fe60e0:src/tunnel.rs:828` (only with
  `--warp-identity PATH`; without it `tunnel` registers a new device on each run and saves nothing),
  `src/cli/cmd/warp/front.rs:323`, and the WireGuard `load_or_register` (`src/warp/export.rs:75`; the
  callers `src/cli/cmd/warp/client.rs:38` and `src/cli/cmd/warp/client.rs:57`).
- src/warp/register.rs:67-95 at `9c1fc4b` (`send_api_to`): a status of 500 or more, or a transport error, goes to the
  next API host for each request, the registration POST included (src/warp/register.rs:77 at `9c1fc4b`).
- `instances::lock` and `try_lock` exist in `crates/podnet-core/src/instances.rs`; `delete_device` exists
  in `src/warp/register.rs`. `docs/decisions.md:38` has the rule of the registration.
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

2026-11-11, commit f08840c. src/warp/enrol.rs at `9c1fc4b` (new): `register_or_load` takes `PATH.lock` (flock, 60 s, mode
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
src/warp/enrol.rs at `9c1fc4b`, `src/warp/hints.rs`.

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

- `crates/podnet-core/src/net.rs:12-13`: a doc comment ("Install the ring crypto provider once per process...") sits
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
   notebook, `CLAUDE.md` and scripts/gen-llms.py; regenerate `llms.txt` and `llms-full.txt`.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/gen-llms.py --check $CARGO_TARGET_DIR/debug/podnet
```

## Correction

2026-11-15: scripts/gen-llms.py is removed (T-359). The check of the llms files is now
`cargo test --bin podnet the_llms_files_equal_the_model` (`src/cli/manual_tests.rs`).

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
documents resolves. scripts/gen-llms.py lists the new documents; `llms.txt` and `llms-full.txt` are regenerated.
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
  `src/tunnel.rs`) and an in-memory fake edge (`PodUtils/podnet@7fe60e0:src/cloudflare/rpc.rs`, `fake_edge`).
- The roots of the edge are built in (`PodUtils/podnet@7fe60e0:src/cloudflare/`); no override exists. `PodUtils/podnet@7fe60e0:src/cloudflare/` is frozen for
  `pod-cloudflare`: a test hook there is a change to report to podcrates.
- A stand-in tests podnet against its own reading of the protocol, not against a change of Cloudflare.

## Approach

1. A test support crate or module with: a fake provision API (`POST /tunnel`, the body of the service, a 429 mode);
   an edge listener over TLS on HTTP/2 that answers `registerConnection` and opens visitor streams; a front that
   maps the Host of a visitor to its tunnel. QUIC after HTTP/2.
2. A hook for the trust roots of the edge in tests (an environment variable read only with a test CA), written as a
   COORD issue (to podcrates) because `PodUtils/podnet@7fe60e0:src/cloudflare/` is frozen.
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
(`crates/podnet-core/src/trust.rs`, `edge_tls_config` in `PodUtils/podnet@7fe60e0:src/cloudflare/mod.rs`). A test CA in
`SSL_CERT_FILE` makes the stand-in edge valid. `PodUtils/podnet@7fe60e0:src/cloudflare/` is not changed, and no COORD issue to
podcrates is needed. The QUIC leg is not built (step 1, "QUIC after HTTP/2"). The move of the steps of
`live.yml` (step 3) is not done in this pass.

2026-11-14, step 3, the part that is open and not frozen (calls of a session, the owner is away). Read: `live.yml`
has no step of the stand-in. The stand-in runs in ci on each push already: `tests/quick_tunnel_standin.rs` is a test
target of `cargo test --locked --workspace` in the job `test` of `.github/workflows/ci.yml`; a6d4428 moved the steps of
`--detach`, `ps`, `logs`, `stop` and `--auth-user` out of `live.yml`. Each stand-in test runs with no call to
Cloudflare and no secret, and with `--no-verify`: the verify of the public URL is not in the stand-in. The calls:
(1) The step "Quick tunnel with UDP 7844 dropped (HTTP/2 fallback)" and its cleanup leave `live.yml`. The new test
`quick_tunnel_standin_falls_back_to_http2_when_udp_is_dropped` holds a UDP socket on the port of the edge that takes
each QUIC packet and answers none, as the `DROP` rule of the step. (2) The part of issue 129 of the step "Quick tunnel
through a local CONNECT proxy" (`--proxy 'http+exec:nc ...'`) leaves `live.yml`. The new test
`quick_tunnel_standin_goes_through_an_exec_proxy` gives a pipe in Python as the command. The rest of the step stays: it
checks the verify of the public URL through the proxy. (3) These steps stay in `live.yml`: they need the real edge,
the real service or the verify of the public URL: the QUIC, HTTP/2, QUIC through WARP and relay legs (one real quick
tunnel for each leg), `--json` (the event `verified` and its resolver), SIGTERM during the verify, the UDP probe, no
system DNS, the wrong `--resolve` pin and the staggered legs with WARP. (4) The QUIC leg of the stand-in stays out:
it needs `PodUtils/podnet@7fe60e0:src/cloudflare/` (frozen). No frozen file changed. Commit 3654b05. `cargo test --test quick_tunnel_standin`: 15 passed, three
runs in a row. The entry stays partial for the QUIC leg.

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

CI: ci 43590942248, deny 43590945244, secrets 43590947996 and live 43590951009 on 9610316 passed (GitHub #554): the two stand-in tests of 3654b05 passed in `ci`.

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
**Status:** done

## Problem

RUSTSEC-2023-0071 (the Marvin attack: a timing leak of the private key in RSA decryption and
signing) now names `rsa` 0.10.0-rc.18, the fix is `rsa` 0.10.0. Each russh release up to 0.65.1 pins
`rsa = "=0.10.0-rc.18"`, so `cargo update -p rsa` changes nothing. The `deny` check fails, and the
release of `v0.4.0-beta.8` waits for it.

## Premise

- podnet makes Ed25519 keys (src/transport/ssh.rs, `crates/podnet-serve/src/relay.rs`,
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

## Correction

2026-11-17: the block ended. Each russh release up to 0.65.1 pinned `rsa = "=0.10.0-rc.18"`; russh 0.65.2 takes `rsa`
0.10.0.

## Done

2026-11-17, the commit of this entry (with the take of podcrates `v0.68.0`, T-420). russh 0.65.2 takes `rsa` 0.10.0
(`cargo update -p russh --precise 0.65.2`: `rsa` 0.10.0-rc.18 -> 0.10.0, `pkcs1` 0.8.0-rc.4 -> 0.8.0, `ssh-key`
0.7.0-rc.11 -> 0.7.0-rc.12). The advisory names `>= 0.10.0` patched. `deny.toml` has `ignore = []`. The proof is the
`deny` workflow of the push (`cargo deny` is not installed in this container).

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

# T-333: link door: --resolve-at local tries only the first address of a name

**Source:** the `ci` runs 43438649213 on b4c8b23 and 43440773144 on db8c1b6: the jobs `test`, `features`,
`no-loopback` and `restricted-host` fail in `tests/link_door.rs`.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

On the runners of GitHub, `localhost` resolves to `::1` first. Two tests of `tests/link_door.rs` fail:
`a_name_goes_to_the_peer_and_resolves_there_or_here` (`--resolve-at local`: the peer gets `::1`, the echo listens on
`127.0.0.1`, the reply code is 1) and `the_resolve_choice_decides_the_address_type_the_peer_gets` (the peer gets the
address type 4, the test expects 1).

## Premise

- `Way::connect` of `src/link_door.rs` (T-291) with `Resolve::Local` resolves the name and sends only the first address
  to the peer.
- A client that resolves a name tries each address of the name until one connects.

## Approach

1. `src/link_door.rs`: with `Resolve::Local`, send a CONNECT for each address of the name, in the order of the
   resolver, until the peer answers 0. Give the last failure when none answers.
2. `tests/link_door.rs`: the local choice gives the peer an address, of type 1 or 4.

## Prove

```sh
cargo test -p podnet --test link_door
sh scripts/agent/check.sh
# the ci run of the commit: the jobs test, features, no-loopback and restricted-host pass
```

## Correction

2026-11-13: the ci run 43445945197 on b1e2b80 passed `tests/link_door.rs` and the jobs `no-loopback` and
`restricted-host`. The jobs `test` and `features` failed in `tests/link_socks5.rs`:
`a_udp_datagram_goes_through_the_peer_to_an_echo_and_back` waited 300 s for the answer to a datagram for the name
`localhost`. The same cause on the side of the peer: `resolve` of `crates/podnet-netns/src/door/udp.rs` took the first
address (`::1`), and the UDP socket of the service binds `0.0.0.0`. `resolve` now takes an address of the family of
the socket when the name has one; a socket on `[::]` keeps the order of the resolver. Test:
`door_udp_name_takes_the_family_of_the_socket`.

## Done

2026-11-13, commits a48d3e7 (`Way::connect` of `src/link_door.rs` tries each address of a name) and 2122bd4 (`resolve` of
`crates/podnet-netns/src/door/udp.rs` takes the family of the socket; test `door_udp_name_takes_the_family_of_the_socket`).
The ci run 43452891413 on 7d73ec7 passed the jobs `test` and `features` (`tests/link_door.rs`, `tests/link_socks5.rs`)
on a runner that resolves `localhost` to `::1` first. The ci run 43459872995 on bf01ac6 passed each job.


# T-332: QUIC over WARP after the take of pod-warp: the socket binds one address family (GitHub #523)

**Source:** GitHub #523 (live: two runs in a row failed): runs 43380951617, 43382313993 and 43383091796, step
"Throughput through WARP, 1 to 16 MiB".
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

After the take of `pod-warp` (f6abdc3), the tunnel leg `quic+warp` fails: "QUIC handshake with the Cloudflare edge: no
edge answered: QUIC handshake with 198.41.192.67:7844 timed out" for each edge address. The run before the take (8fc8bb4)
passed. The origin of `scripts/live-throughput.sh` also failed once with "Address already in use" on the fixed port 18090.

## Premise

- Before the take, `src/warp/quic.rs` bound a UDP socket of the stack on the IPv4 address of the device and gave it to
  `pod_quic::queued::QueuedDatagrams`.
- After the take, it takes `pod_core::Dial::datagrams` of `WarpNet`. That socket binds `[::]:port` (`bind_udp_any`).
  noq sends an IPv4 destination on an IPv6 socket as an IPv4-mapped IPv6 address. The stack does not route it.

## Approach

1. `src/warp/quic.rs` (a fix of a frozen file): bind the socket on one address of the device (the IPv4 address, else the
   IPv6 address) with `NetStack::bind_udp`, and give its datagrams to `pod_quic::over`.
2. `scripts/live-throughput.sh`: take free ports of the loopback.
3. Report the fix to podcrates as a COORD issue: `WarpNet`'s datagrams must not bind `[::]` for a consumer that sends to
   IPv4.

## Prove

```sh
sh scripts/agent/check.sh
# the live run: the step "Throughput through WARP, 1 to 16 MiB" passes
```

## Correction

2026-11-13: the live run 43394125862 on 917a0ae passed the step "Throughput through WARP" (the QUIC fix holds), then
failed at "WARP account and probe": `warp nc -u 1.1.1.1 53` with no `warp up` got no answer. The same socket of `WarpNet`
(`[::]:port`) is the way of `pod_warp::nc::udp_via_net`, which keeps an answer only when its source equals the IPv4 target.
src/warp/one_family.rs at `ca53a72` now holds the socket of one family for both: `quic.rs` takes it, and `nc.rs` resolves the host
first and gives `udp_via_net` a way (`OneFamily`) whose datagrams bind the family of the target.

2026-11-13: the live run 43397473033 on ca53a72 failed at the same step: `warp nc -u` ended 40 ms after its line "opens a
WARP session of its own", with no answer and no error. A second cause: `pod_warp::nc::udp_via_net` keeps the socket in its
tasks, not the session, and `nc_session` (`src/cli/cmd/warp/up.rs`) gave its only handle of the session to
`udp_via_stack`, so the session was dropped when that function returned. A stopped stack ends each read with
`ConnectionReset` (pod-netstack since podcrates v0.34.0): the receive task ended, the channel of the answers closed, and
the pipe ended at once. `nc_session` now keeps the session until the pipe ends.

2026-11-13: the live run 43401730194 on ef240bb passed "WARP account and probe" (`warp nc -u` answers). It failed at "WARP
probe through the WebSocket relay": the probe worked, but the step looks for the line "MASQUE goes through the WebSocket
relay", which `pod-warp` writes as "MASQUE goes through the relay" with the target `pod_warp`, and the job's `RUST_LOG`
(`podnet=info`) hid that target. `.github/workflows/live.yml`: `RUST_LOG` is `podnet=info,pod_warp=info`, and the step
looks for the new line.

2026-11-13: podcrates `v0.39.0` (PodUtils/podcrates#137, its entry 508) fixes the datagrams of `WarpNet` for both
paths. The take drops src/warp/one_family.rs at `ca53a72`: `quic.rs` and `nc.rs` take the datagrams of the session again. The
fix of `nc_session` stays: it is podnet's own.

2026-11-13: the live run 43407944050 on dfc1695 passed the steps of WARP through QUIC, nc and the relay (the fix of
podcrates `v0.39.0` holds). It failed at "warp tun --default-route (root)": the step looks for "the routes and the
resolver are as before" in the log of the instance, a line that `pod-warp` writes with the target `pod_warp`. The default
log filter of podnet (`podnet_core::report::log_filter`) named six targets and hid each line of `pod-warp` and of the other
crates of podcrates. The filter now names each `pod-*` crate that podnet runs. Test:
`the_log_filter_names_each_pod_crate_of_podnet` (it reads the root `Cargo.toml`).

2026-11-13: the live run 43434773421 on 1765d1c passed "warp tun --default-route (root)" (the log filter holds) and
failed at "WARP run": the step looks for the debug line "a UDP association in the stream", which `pod-warp` writes
(`pod_warp::proxy::udp`); the step sets `RUST_LOG=podnet=debug`, which hid the target `pod_warp`. Each `RUST_LOG` of
`podnet=debug` in `.github/workflows/live.yml` now adds `pod_warp=debug`. The later steps grep no other line that only a
crate of podcrates writes, except the relay line, which the `RUST_LOG` of the job covers.

2026-11-13: the live run 43438649254 on b4c8b23 passed "WARP run" (the `RUST_LOG` holds) and each step to "The commands
of the second batch". It failed at "a nested session" of `scripts/live-batch2.sh`: the WARP service refused the new
device of the nested session with HTTP 429 (error 1015, "wait 39 s"). The session ended and the port did not open. The
step now waits the time that the service asks for, plus 5 s, and starts the session again, at most 3 times.

2026-11-13: the live run 43445966561 on b1e2b80 passed "The commands of the second batch" (the wait for HTTP 429
holds). It failed at "An IPv6-only host": `warp tun --default-route` in `scripts/v6only/up.sh` ended with "detached
instances need a run directory". Since the take of `pod-warp` (f6abdc3), `--default-route` keeps the record of the
resolver file in the state directory, else the run directory; the lab sets `PODNET_STATE_DIR=none`, and `sudo` drops
`XDG_RUNTIME_DIR`. `scripts/v6only/up.sh` and `scripts/v6only/acceptance.sh` now give each tun a run directory
(`PODNET_RUN_DIR`), and `scripts/v6only/down.sh` removes them.

2026-11-13: the live run 43452912977 on 7d73ec7 failed earlier, at "File server through a quick tunnel": the tunnel was
up (its check through DNS over HTTPS got the answer 401), but the resolver of the runner did not resolve the new name, and
curl stopped after 5 tries. The steps of the file server and of `git serve` now run `scripts/live-wait-name.sh` (issue
119) before the first request, as the step of the user service does.

## Done

2026-11-13. The fixes, in order: the datagrams of `WarpNet` (podcrates v0.39.0, GitHub #524; the take 306d80b), the
session of `nc_session` (ef240bb), the log filters of the job and of podnet (e1c5b34, 6fec3df, ecbd88b), the wait for
HTTP 429 in `scripts/live-batch2.sh` (a48d3e7), a run directory for each `warp tun` of `scripts/v6only/` (2122bd4), and
the wait for the new name of the quick tunnel in the steps of the file server and `git serve` (098ff8d). The live run
43459893595 on bf01ac6 passed each step, among them "Throughput through WARP, 1 to 16 MiB", "WARP run", "The commands
of the second batch" and "An IPv6-only host". ci 43459872995, deny 43459879363 and secrets 43459886257 on bf01ac6 passed.

# T-336: Publish `v0.4.0-beta.10`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease unattended when a batch of fixes is green. The
batch since `v0.4.0-beta.9`: ci 43521791090, deny 43521797247, secrets 43521803788 and live 43521809728 on 3061c7c passed.
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.9` has none of the work after it: the transfer by code, the new carriers of `podnet link`, the keys from end
to end, WARP as a network, the resumable relay and the take of podcrates `v0.40.0`.

## Premise

- The steps are `docs/maintenance.md`, "Release". A push of the cloud session starts no workflow run: each dispatch goes
  to the owner as a `[gh action] requested` issue.

## Approach

1. Set the version `0.4.0-beta.10` in `Cargo.toml`, `crates/podnet-capi/Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.10.md`, with `## Breaking`.
3. Push, and ask for `ci`, `deny` and `live` on that commit.
4. Ask for `release.yml` on `main` with `publish` empty (the dry run), then with `publish=v0.4.0-beta.10`.
5. Check the files of the release (with `pod-report.schema.json`) and `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next prerelease unattended when a
batch is green.

## Prove

```sh
cargo test --test docs
# the runs of release.yml (dry run, publish) and release-proofs.yml pass
```

## Correction

2026-11-14: the request GitHub #543 stopped at `ci` on 43c53a6 (ci 43535662399 failed in the job `restricted-host`; deny
43535667510, secrets 43535672932 and live 43535679488 passed); nothing was published. The failure:
`service_serve_ingress::a_grant_that_does_not_name_the_service_refuses_the_stream` panicked with `BrokenPipe` at the write
of its request: the refused stream can close before the write. The helper `get` of `tests/service_serve_ingress.rs` now
takes a failed write as an empty answer (the test still asks for an empty answer). The test passed three runs in a row;
`sh scripts/agent/check.sh` passed. A new request follows (GitHub #544).

2026-11-14: the request GitHub #544 on 59cf78c passed ci 43539190755, deny 43539195145, secrets 43539199370 and live
43539203095, and stopped at the dry run 43541138625: the jobs `i686-unknown-linux-musl`, `armv7-unknown-linux-musleabihf`
and `arm-unknown-linux-musleabihf` did not compile `crates/podnet-netns/src/nfnl.rs` (T-153): `tv_sec` of `libc::timeval` is
an `i32` there, and the code gave an `i64`. The value is now clamped to `i32::MAX` and converted to the type of the field.
`cargo check -p podnet-netns` passes for the three targets here (clang as CC); CI checks x86-64 only, so the dry run of
`release.yml` is the first build of these targets. A new request follows (GitHub #545).

## Done

2026-11-14, commits 43c53a6 (`T-336: version 0.4.0-beta.10 and its release notes`), 59cf78c (the race of
`service_serve_ingress`) and e99eba6 (`nfnl.rs` on the 32-bit targets; the take of podcrates v0.41.0, 095955c). On
e99eba6 (GitHub #545): ci 43544154056, deny 43544158796, secrets 43544163293 and live 43544168415 passed; `release.yml`:
the dry run 43544173658 passed, the publish 43547344329 passed (the tag `v0.4.0-beta.10` at e99eba6), the release on the
tag 43547356755 passed, `publish.yml` 43547358531 passed. The release `v0.4.0-beta.10` is a prerelease with 16 files:
eight binaries, `podnet.1`, `llms.txt`, `llms-full.txt`, `pod-report.schema.json`, `SHA256SUMS`,
`SHA256SUMS.sigstore.json`, `provenance.sigstore.json` and `THIRD-PARTY-LICENSES.md`. `release-proofs.yml`: run
43549615088 passed.


# T-339: Fuzz targets for the parsers of peer and file input that came after 2026-11-12

**Source:** the lead session, 2026-11-14 (the 18 targets of `fuzz/fuzz_targets/` cover no parser that came after
2026-11-12).
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`docs/build.md` says that `fuzz/` has a libFuzzer target for each parser of bytes from the network. The 18 targets
are older than the wg-quick file (`wg:FILE`, `podnet wg serve`), the chains of networks (`TOP over BELOW`), the router
file, the transfer (the messages and hints of magic-wormhole, `podnet-relay-v1`, `piping-v1`, the zip of a directory,
the records of the transit) and the route announcements of the link mesh.

## Premise

- Read: `src/fuzzing.rs`, `fuzz/Cargo.toml`, `.github/workflows/fuzz.yml` (it runs each target of
  `cargo fuzz list`: no list to change), `crates/podnet-vnet/src/wg_quick.rs`, `src/networks/wg_serve.rs`,
  `src/networks.rs`, `src/networks/chain.rs`, `src/router.rs`, `src/transfer/messages.rs`, `src/transfer/zip.rs`,
  `src/transfer/land.rs`, `src/transfer/transit.rs`, `src/transfer/piping.rs`, `src/mesh_announce.rs`,
  `src/mesh_table.rs`, `src/link_query.rs`.
- Skipped: the query keys of a link carrier address (`src/link_query.rs`). Only the command line gives an address
  (`link listen`, `link dial`, `link connect`); no peer and no file gives one.
- The network specs come from the router file and from the control socket of the router, and the target
  `proxy_url` parses a spec already, with seeds of proxy URLs only.
- A `wg:` spec reads a file. `wg_quick::read_file` read the whole file, with no bound: `wg:/dev/zero` filled the
  memory.
- `Records::receive` of the transit made a buffer of the length of a record (at most 64 MiB) before the bytes came: a
  peer that sends 4 bytes made this end allocate 64 MiB.
- `Plan::new` of the kernel mode of the router added the priority (u32) to a `usize`: on a 32-bit target a priority
  near `u32::MAX` wraps and passes the check.
- Not measured before the change: a run of the new targets under libFuzzer.

## Approach

1. Add the targets `wg_quick`, `network_spec`, `router_config`, `transfer_message`, `transfer_zip`,
   `transit_record`, `mesh_announce` and `mesh_table` (`src/fuzzing/peers.rs`, `fuzz/fuzz_targets/`,
   `fuzz/Cargo.toml`), with seeds. No target opens a file of the input: a spec with `wg:` is not parsed (also in
   `proxy_url`). The hooks of crate-private code: `wg_serve::check` and `wg_serve::fuzz_packet`,
   `mesh_announce::fuzz`.
2. Bound the read of a wg-quick file to 1 MiB. Grow the buffer of a record with the bytes that come. Add the
   priority of the kernel mode in u64.
3. Give the step of the targets of `fuzz.yml` 80 minutes (26 targets).
4. Run each new target for 60 s under libFuzzer; fix each crash in podnet's code.

## Prove

```sh
cargo test -p podnet --lib -- fuzzing::tests::each_target_survives_mutations
cargo test -p podnet-vnet -- wg_quick_file_read_has_a_bound
cargo test -p podnet --lib -- kernel_config_errors_name_the_key
```

Then, in `fuzz/` with a nightly toolchain, `cargo +nightly-2026-10-15 fuzz run TARGET corpus/TARGET --
-max_total_time=60 -malloc_limit_mb=64 -rss_limit_mb=2048 -timeout=10` for each new target.

## Done

2026-11-14, commit ce6352c. `src/fuzzing/peers.rs`, eight files of `fuzz/fuzz_targets/`, `fuzz/Cargo.toml`, the hooks
`wg_serve::check`, `wg_serve::fuzz_packet` and `mesh_announce::fuzz`, the guard of `wg:` in `proxy_url`, the bound of
1 MiB of `wg_quick::read_file`, the buffer of `Records::receive`, the u64 sum of `Plan::new`, `fuzz.yml` (the step
of the targets: 80 minutes; the job: 120), `docs/build.md`. `cargo test -p podnet --lib -- fuzzing`: 1 passed (each
of the 26 targets on its seeds and on 2000 mutations, 100 for `mesh_announce`); `wg_quick_file_read_has_a_bound` and
`kernel_config_errors_name_the_key` passed; `cargo test --workspace --no-fail-fast`: 0 failed; the 32-bit check
(`i686-unknown-linux-musl`): 0. With nightly-2026-10-15 and cargo-fuzz 0.13.1, each new target for 60 s from its
seeds (`-malloc_limit_mb=64 -rss_limit_mb=2048 -timeout=10`), no crash, no timeout, no large allocation:
`wg_quick` 1855804 runs, `network_spec` 785725, `router_config` 367505, `transfer_message` 801393, `transfer_zip`
1189058, `transit_record` 1130339, `mesh_announce` 88883, `mesh_table` 380834. The three defects of the Premise were
found by reading, before the runs: the targets keep them fixed. CI: `fuzz.yml` runs the 26 targets each week; no run
yet.

CI: ci 43590942248, deny 43590945244, secrets 43590947996 and live 43590951009 on 9610316 passed (GitHub #554). `fuzz.yml` 43590953876 on 9610316 ran the 26 targets (120 s each) and failed with two crashes: `capnp_message` (T-348, 54637d9) and `proxy_url` (T-349, 243eb48). After the fixes each of the two targets ran 120 s here with its crash input in the corpus, no crash.

# T-340: The parity checklist after the take of pod-warp, the router, the transfer and the carriers

**Source:** `CLAUDE.md` ("tick `docs/research/podnet/parity-checklist.md` areas after each large refactor"); the lead
session, 2026-11-14 (the checklist was last changed on 2026-11-12).
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The take of `pod-warp` (2026-11-13: `src/warp/` is glue over the crate), the router (T-097, T-120, T-248), the
transfer and the carriers came after the last walk of `docs/research/podnet/parity-checklist.md`. No feature or
capability of the code may be lost.

## Premise

- Read: the checklist (41 of 620 boxes ticked), `podnet man --json` of ce6352c, `.github/workflows/live.yml`,
  `scripts/live-batch2.sh`, the tests of `tests/` and of `src/cli/cmd/warp/`, the files of `pod-warp` v0.43.0.
- The router, the transfer and the carriers are new: the checklist (the CLI at 3bed023) names none of them.

## Approach

1. Compare each command and flag item of `podnet warp` with `podnet man --json` (the default, the variable, the
   values, required and repeatable), with a script of the scratchpad.
2. Name the test or the live step that runs each `warp` command; run a command that has neither.
3. Check the WARP variables, files and documents, the steps of `live.yml` and the fuzz targets.
4. Tick what holds; write an entry for each item that does not hold.

## Prove

```sh
cargo test --workspace
target/debug/podnet man --json
python3 scripts/check-workflows.py
```

## Done

2026-11-14, commit 5046de6. Ticked: 41 before, 232 after (of 620).
Each of the 116 items of the 21 sections `podnet warp ...` agrees with `podnet man --json` of ce6352c; one difference:
`warp forward -L` is required unless `--list` (a new flag); `podnet warp forward` with neither exits 64. Each section
has a line `checked 2026-11-14` with its tests and live steps; `warp shell` has no test, and a run with no `warp up`
started the shell with the line of the missing proxy (exit 0). The WARP variables are in `podnet man --json` and in
the code (`POD_NETNS_QUIET` is `PODNET_NETNS_QUIET` since the rename); the WARP files, the WARP documents, the steps of
section 12 (three are replaced by tests of the stand-in in ci: a6d4428 and 3654b05) and the fuzz targets (`doh_json`
is in `pod-dns`) hold. `cargo test --workspace` on ce6352c: 0 failed. No item failed: no defect entry.

CI: ci 43590942248, deny 43590945244, secrets 43590947996 and live 43590951009 on 9610316 passed (GitHub #554).

# T-341: podnet receive: a directory can land more bytes and entries than the offer that the person accepted

**Source:** the security review of the code of 2026-11-13 and 2026-11-14 that takes peer input (2026-11-14).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

A hostile sender offers a directory with `numbytes` 1 KiB and `numfiles` 1. The person accepts the offer. The zip
that comes holds deflated entries of up to 64 GiB (the default `--max-bytes`) and up to 100000 entries (the default
`--max-files`). A deflate stream of 1 MiB can give 1 GiB. The receiver writes all of it: the disk of the host fills,
and the person did not accept that size.

## Premise

- `check` of `src/transfer/receive.rs` compares `numbytes`, `zipsize` and `numfiles` of the offer with the caps.
- `land_offer` gave `how.caps` (the flags) to `land::unzip`, not the values of the offer. `plan` and `Capped` of
  `src/transfer/land.rs` hold the flags only.
- `numfiles` of magic-wormhole (`cmd_send.py`) counts the files of the zip, and `numbytes` is the sum of their sizes.
  The zip of podnet holds its directories as entries, and its `numfiles` counts them. Thus the entries of a good zip
  are `numfiles`, and the sum of their sizes is `numbytes`.

## Approach

1. `land_zip` of `src/transfer/receive.rs`: the caps of the unzip are the caps of the flags, narrowed to `numbytes`
   and `numfiles` of the accepted offer. A refusal names the offer.
2. A test of the lib: a zip with more bytes, or more entries, than its offer is refused and writes nothing; a zip that
   agrees with its offer lands.

## Decision

2026-11-14: no slack over the offer. A Python sender stats each file after it writes it into the zip; a file that
grows in between makes the zip larger than `numbytes`, and the receiver refuses it. The person accepted the size of
the offer, not more.

## Prove

```sh
cargo test -p podnet --lib transfer_receive_a_zip_lands_no_more_than_its_accepted_offer
cargo test -p podnet --test transfer --test transfer_land
sh scripts/agent/check.sh
```

## Done

2026-11-14, commit ab0cff7. `land_zip` of `src/transfer/receive.rs` narrows the caps to the accepted offer.
`cargo test -p podnet --lib transfer_receive_a_zip_lands_no_more_than_its_accepted_offer`: 1 passed; with the caps of
the flags only (the code before the fix) it failed at the first `unwrap_err`. `cargo test -p podnet --test transfer
--test transfer_land`: 9 and 7 passed.


# T-342: wg serve: a peer can send with the address of another peer when their AllowedIPs overlap

**Source:** the security review of the code of 2026-11-13 and 2026-11-14 that takes peer input (2026-11-14).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The file of `podnet wg serve` has peer A with `AllowedIPs = 10.9.0.0/25` and peer B with `AllowedIPs = 10.9.0.2/32`.
Peer A sends packets with the source 10.9.0.2. The server takes them: the stack opens flows and dials targets with
the address of peer B, and the answers go to peer B. Peer A can open flows in the name of peer B, collide with the
flows of peer B, and send answers to peer B that it did not ask for.

## Premise

- `incoming` of `src/networks/wg_serve.rs` admitted a packet when a prefix of the `AllowedIPs` of its own peer held
  the source.
- `outgoing` sends a packet to the peer whose `AllowedIPs` has the longest prefix that holds the destination.
- `check` does not refuse overlapping `AllowedIPs`. The cryptokey routing of WireGuard admits a packet only when its
  source routes back to the peer that sent it.

## Approach

1. `incoming`: admit a packet only when `route` of its source gives its own peer.
2. A test on the loopback: the overlap above; the server dials nothing for the source of peer B, and serves peer A
   with an address of its own.

## Prove

```sh
cargo test -p podnet --lib wg_serve
sh scripts/agent/check.sh
```

## Done

2026-11-14, commit 9c98cce. `incoming` of `src/networks/wg_serve.rs` admits a packet only when `route` of its source
gives its own peer. `cargo test -p podnet --lib wg_serve`: 7 passed. With the old admit rule,
`wg_serve_drops_a_source_that_routes_to_another_peer` failed: the server dialed for the source of the other peer.


# T-343: link record: a middle node makes an end take 16 MiB for each record header that it sends

**Source:** the security review of the code of 2026-11-13 and 2026-11-14 that takes peer input (2026-11-14).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A sealed stream of the link passes a middle node (a mesh hop, T-287). The middle node sends a record header of 24
bytes that says a body of 16 MiB (`MAX_LENGTH` of `pod-e2e`), and then no byte. The end that reads the stream takes
a buffer of 16 MiB at once, before a byte of the body comes and before the check of the record. One header for each
stream: 16 MiB of memory for 24 bytes of input.

## Premise

- `read_record` of `src/link_record/pump.rs` called `record.resize(len, 0)` with the length of the header, then
  `read_exact`.
- `record_len` of `pod-e2e` refuses a length above 16 MiB only.
- The transit of the transfer (`src/transfer/transit.rs`) already grows its buffer with the bytes that come.

## Approach

1. `read_record`: reserve at most 64 KiB (`BODY_STEP`) for the body, and read the body with `take` and
   `read_to_end`: the buffer grows with the bytes that come. A stream that ends in a record is still a cut.
2. A test of the lib: a reader that gives a header of 16 MiB and then no byte is offered a buffer of at most
   `BODY_STEP`.

## Prove

```sh
cargo test -p podnet --lib link_record
cargo test -p podnet --test link_record --test link_seal --test link_pad --test link_compress --test link_replay --test link_record_mesh
sh scripts/agent/check.sh
```

## Done

2026-11-14, commit b8c1d24. `read_record` of `src/link_record/pump.rs` reserves at most `BODY_STEP` (64 KiB) and
reads the body with `take` and `read_to_end`. `cargo test -p podnet --lib link_record`: 7 passed. Before the fix,
`link_record_a_long_header_with_no_body_takes_no_buffer_of_its_length` failed: "a read offered 16777232 bytes for a
body that did not come". The six test targets of the Prove passed (3, 2, 8, 2, 8, 5). `cargo test --workspace
--no-fail-fast` on cbb8579: each target passed.


# T-344: podnet receive and send: the other end of a transfer can make this end dial thousands of hosts

**Source:** the security review of the code of 2026-11-13 and 2026-11-14 that takes peer input (2026-11-14).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The `transit` message of the other end holds a list of hints. A hostile end (it has the code) sends a message with
thousands of `direct-tcp-v1` hints, or a `relay-v1` hint with thousands of ends, with the addresses of hosts of the
network of the receiver. This end dials each one at once, and writes the handshake of the transit to each that
answers, for the time of the transit wait (120 s): a scan of ports and a flood of connections from the host of this
end, and many tasks and descriptors.

## Premise

- `Transit::parse` of `src/transfer/messages.rs` kept each hint of a known kind, with no bound.
- `connect` of `src/transfer/connect.rs` spawns one dial for each direct hint and for each relay end, and one dial
  loop (again each 1 s) for each podnet relay hint.
- A Python end sends one direct hint for each address of its host, and its relay hints. Not measured: the most
  addresses of a real host; 64 holds each host that the tests and the lab kit use.

## Approach

1. `Transit::parse`: keep the first `MAX_HINTS` (64) hints of a known kind, and the first `MAX_RELAY_ENDS` (16)
   ends of each relay hint.
2. A test of the lib: a message with 1001 hints and a relay hint of 1000 ends gives 64 hints and 16 ends.
3. `docs/transfer.md`: the caps.

## Prove

```sh
cargo test -p podnet --lib transfer_hints_a_long_list_of_hints_is_cut_at_its_cap
cargo test -p podnet --lib transfer
sh scripts/agent/check.sh
```

## Done

2026-11-14, commit 4f7dfc5. `Transit::parse` of `src/transfer/messages.rs` keeps `MAX_HINTS` hints and
`MAX_RELAY_ENDS` ends of each relay hint. `cargo test -p podnet --lib transfer`: 18 passed. Before the fix,
`transfer_hints_a_long_list_of_hints_is_cut_at_its_cap` failed: 1001 hints, where 64 are expected.


# T-345: wg serve: one peer can take each flow of the server from the other peers

**Source:** the security review of the code of 2026-11-13 and 2026-11-14 that takes peer input (2026-11-14).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Each peer of `podnet wg serve` goes to one guest of the router, and the guest has one pool of flows: `MAX_FLOWS`
(2048) TCP flows and 512 UDP flows. A peer with a valid key opens 2048 TCP flows (or sends 2048 SYNs to targets that
do not answer, which stay held while the dial runs), or 512 UDP flows. Each SYN and each new datagram of the other
peers is then dropped. The cap keeps the host up; the other peers lose the service until the flows end.

## Premise

- `syn` of `src/router/guest/tcp.rs` drops a SYN when `tcp_flows` (the held SYNs and the open flows of all sources)
  is at `MAX_FLOWS`. `deliver_new` of `src/router/guest/udp.rs` refuses a new flow at `MAX_FLOWS` of UDP.
- The guest sees packets, not peers: a peer with `AllowedIPs` of a /25 has 128 sources.
- Not measured: the time that a peer holds a flow (it depends on the target and on the timeouts of the stack).

## Approach

1. The guest counts the flows (held, open, UDP) of each source address, and refuses a new flow of a source at its
   share. The share is a setting of `RouterGuest` (default: `MAX_FLOWS`, the behaviour of today).
2. `podnet wg serve` gives each peer a share of `MAX_FLOWS / peers` (at least 64), counted over the sources of its
   `AllowedIPs`: `RouterGuest` takes a function from a source to its group (`route` of `src/networks/wg_serve.rs`).
3. A test on the loopback: peer A holds its share of SYNs to a target that does not answer; peer B still opens a
   flow.

## Decision

2026-11-14, the calls of the work:

- The shares and the totals are settings. `podnet wg serve`: `--flows` (2048), `--udp-flows` (512), `--peer-flows`
  and `--peer-udp-flows` (the total divided by the peers, at least 64 TCP and 16 UDP flows). A guest of the
  router: `flows`, `udp_flows`, `source_flows` and `source_udp_flows`; the share of a source is the total by
  default, so a guest with one host keeps the behaviour of before.
- A released SYN (the dial succeeded, the stack has not accepted the stream) keeps its place in the share until
  its flow ends. A stream that the stack accepts with no released SYN is counted, also over the share: the
  handshake has ended.
- One counter, `share_refused`, for TCP and UDP. The log gets one line for each group in each 10 s
  (`LOG_EVERY`); the line gives the count of the flows refused since the last line.

## Prove

```sh
cargo test -p podnet --lib wg_serve
cargo test -p podnet --lib router::guest
sh scripts/agent/check.sh
```

## Done

2026-11-14, commit f7ba889. `src/router/guest/share.rs` counts the flows of each group of sources;
`syn` of `src/router/guest/tcp.rs` and `deliver_new` of `src/router/guest/udp.rs` refuse a new flow over the
share. `podnet wg serve` groups the sources of each peer (`PeerGroups` of `src/networks/wg_serve.rs`).
`cargo test -p podnet --lib wg_serve`: 8 passed; `wg_serve_one_peer_cannot_take_the_flows_of_another` (total 16,
share 4: peer A holds 16 SYNs to a target that does not answer, peer B still gets an echo). `cargo test -p podnet
--lib router::guest`: 16 passed, with `guest_wire_udp_a_source_has_at_most_its_share_of_flows` and
`guest_wire_share_logs_one_line_for_each_group_in_each_interval`. The plant: with the share check of `take`
off (one group takes the total, the behaviour before the fix), the wg test failed ("no answer" for peer B), and
the UDP test failed ("share: 0 of 1"). `cargo test --workspace --no-fail-fast`: each target passed, except
`llms_files_are_current` (fixed by scripts/gen-llms.py, then 2 passed). `CC_i686_unknown_linux_musl=clang cargo
check --workspace --target i686-unknown-linux-musl`: no error.


# T-346: link mesh: a neighbour can claim a direct link to each origin that it hears, and attract its routes

**Source:** the security review of the code of 2026-11-13 and 2026-11-14 that takes peer input (2026-11-14).
**Category:** defect
**Milestone:** M8
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Node X gets the announcement of origin O with the path `[O, A]` from its neighbour A. X removes the hop of A, signs its own hop over the signature of the origin, and sends the path `[O, X]` with the cost 0. Its neighbours
accept it: X is now one hop from O at the lowest cost, and the routes to O go through X. When X is fast, its copy
comes first, and the honest copy of the same `seq` is dropped (`SeenSeq`). X then sees each stream to O: a stream
with no `--seal` is plain text at X; a sealed stream can still be dropped or delayed.

## Premise

- `verify` of `src/mesh_announce.rs` checks the signature of the origin over the body, and the chain of hop
  signatures: each hop signs `hop_bytes(prev, key, cost)`. A hop names itself, not the node that it sends to.
- The body that the origin signs names no neighbour. Thus each node that holds the signature of the origin can
  start a new chain at the origin.
- `receive_stream` of `src/link_announce.rs` checks only that the last node of the path is the peer of the link.
- `Receiver::note` keeps one newest `seq` for each origin, over all neighbours.

## Approach

1. A new version of the wire: the origin signs one hop record for each neighbour that it sends to (its key and the
   key of the neighbour), and each hop signs `prev`, its key, its cost and the key of the next node. `verify` checks
   that the next key of each hop is the key of the next node of the path, and that the last one is this node.
2. A node that is not in the signed chain cannot shorten the path: a chain that starts at the origin names the first
   neighbour.
3. Tests of `tests/mesh_announce.rs`: a path with a hop removed, or with a new first hop, is refused; the fuzz target
   `mesh_announce` follows the new form.

## Decision

2026-11-14: recorded, not fixed in the review. The change is a new version of the wire of the mesh (both ends must
change), and an effort of some days. Until it is done, a mesh that carries plain streams trusts each neighbour with
the routes; `docs/link.md` says to use `--seal` through hops.

2026-11-14, the fix: the hop of the origin is one more hop of the path (key, cost 0, next key, signature over the
signature of the body), so each hop has one form. A node refuses version 1 with no fallback (a fallback keeps the attack
open); the log gets one line for each stream. The breaking change is in `docs/link.md` (Breaking) and in
`docs/decisions.md` (2026-11-14). The cost: one signature for each neighbour that a node sends or forwards to.

## Prove

```sh
cargo test -p podnet --test mesh_announce --test mesh_table --test mesh_three_hops
cargo test -p podnet --test link_announce --test mesh_processes --test mesh_acl --test mesh_status --test link_dump
sh scripts/agent/check.sh
```

## Done

2026-11-14, commit 637921e. `src/mesh_announce.rs` is version 2: each hop names its next node, and `verify` refuses a
hop that names another node (`BadNextHop`) and a last hop that does not name this node (`NotForMe`). `Mesh::make` and the
forward of `src/link_announce.rs` sign one hop for each neighbour; `receive_stream` logs a version once for each stream.
`cargo test -p podnet --test mesh_announce`: 33 passed, with `a_neighbour_cannot_strip_hops_and_claim_the_origin` (the
stripped path `[O, X]` is `BadNextHop`; with the next key of O changed to X it is `BadHopSignature`),
`dropped_hop_is_refused`, `a_valid_path_of_3_hops_is_taken`, `version_1_is_refused_and_named` and
`an_announcement_for_another_node_is_refused`. `cargo test -p podnet --test link_announce`: 8 passed, with
`a_peer_of_version_1_is_refused_with_one_line` (3 frames of version 1, one log line). `mesh_table` 22, `mesh_three_hops`
11, `mesh_processes` 4, `mesh_acl` 3, `mesh_status` 4, `link_dump` 11 passed. The plants: with the next-key check of
`verify` off, 5 tests of `mesh_announce` failed (the stripped path was taken); with one line for each frame, the log test
failed (3 lines). `cargo test --workspace --no-fail-fast`: each target passed. `CC_i686_unknown_linux_musl=clang cargo
check --workspace --target i686-unknown-linux-musl`: no error.


# T-347: Hardening: the low findings of the security review of 2026-11-14

**Source:** the security review of the code of 2026-11-13 and 2026-11-14 that takes peer input (2026-11-14).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

The review found these items of low severity. Each one is checked against the code; the last two are not measured.

1. `entries` of `src/transfer/zip.rs` reads the whole central directory into memory: up to 4 GiB, as the zip says.
   The zip is on the disk first, so the memory is at most the bytes that the sender sent.
2. `receive` of `src/cli/cmd/transfer.rs` prints the text of a sender, and the path of the transfer, with no escape:
   a control sequence of a terminal in a text or a name reaches the terminal of the receiver.
3. The listener of a direct transit (`connect` of `src/transfer/connect.rs`) takes each connection that comes for
   the transit wait, each one for up to 20 s: a host of the network can hold many descriptors.
4. `Landing::undo` of `src/transfer/land.rs` removes by path from the output directory. A local user who changes a
   directory of the transfer into a symbolic link between the write and the undo makes the undo remove a file there.
5. `Frames::learn` of `src/router/guest/veth.rs` learns the MAC address of each source of the net from each frame. On
   a veth wire that a bridge shares among guests, a guest takes the address of another guest, and a guest that sends
   from 4096 addresses empties the table.
6. The transit compares the handshake of the other end byte by byte (`expect` of `src/transfer/transit.rs`), not in
   constant time. The records have their own key, so a match of the handshake gives no data.
7. Not measured: the kernel mode of the router (`src/router/kernel/guard.rs`) guards IPv4 only. On a host that
   forwards IPv6, the clients go out over IPv6 past the kill switch.
8. Not measured: `receive_stream` of `src/link_announce.rs` has no rate limit; each frame costs up to 17 signature
   checks.

## Premise

- Each item comes from a reading of the code at 4f7dfc5. Items 7 and 8 are suspicions: no host with IPv6 forwarding
  and no load test was used.

## Approach

1. Bound the central directory to the cap of entries times (46 + `PATH_MOST` + 1024) bytes.
2. Print a text and a name with the control characters escaped (as `{:?}` does for the question).
3. Hold at most 64 handshakes of the listener at a time; close each later connection.
4. Undo with the descriptors of `Landing` (`unlinkat` under the directory that the write opened).
5. Learn a MAC address from the ARP of a guest only, and keep the first MAC of an address until it is idle.
6. Compare the handshake with a constant-time compare.
7. Measure: a host with `net.ipv6.conf.all.forwarding` 1; when the clients go out, add an IPv6 unreachable route, or
   say it in `docs/router.md`.
8. Measure the cost of a flood of frames; add a token bucket for each stream when one core is busy.

## Decision

2026-11-14, the calls of the work:

- Item 2: the text of a sender is escaped when stdout is a terminal; a pipe and `--json` get it as it came, so a
  script keeps the exact text. A text keeps its line feeds and tabs; a name escapes them too.
- Item 3: the bound is 64 handshakes at one time, not 64 connections in all: a slot that ends takes a new
  connection.
- Item 5: the MAC address is learned from each frame, as before (a guest with a static neighbour entry for the
  gateway sends no ARP), but an address in use keeps its first MAC address until it is idle for 60 s (`MAC_IDLE`).
  A full table drops its idle addresses; with none idle, a new address waits (its frames are dropped). A guest that
  changes its MAC address waits 60 s.
- Item 7: a warning line and `docs/router.md`, not an IPv6 rule: a drop of the forwarded IPv6 of the host would
  change IPv6 forwarding that the router does not own.
- Item 8: the bucket counts signature checks, not frames: 2048 each second and 8192 at most for each stream (the
  announcements of 4096 origins with paths of 5 nodes in each 10 s). The checks also moved out of the lock of the
  route table.

## Prove

```sh
cargo test -p podnet --lib transfer
cargo test -p podnet --lib router::guest
cargo test -p podnet --test transfer_land --test link_announce
cargo test -p podnet --lib kernel_guard
sh scripts/agent/check.sh
```

## Correction

2026-11-14: item 7 holds by the code: `src/router/kernel.rs` and `src/router/kernel/guard.rs` write rules, routes
and a table of nf_tables of the family IPv4 only (no `AF_INET6`), and the kernel keeps the rules of IPv6 apart, so an
IPv6 packet of a client takes the IPv6 routes of the host. Not measured on a host with IPv6 forwarding: the container
has no `ip` program. Item 8 holds by a measurement: a replay of a valid announcement of 16 nodes (5582 bytes) took
885 µs of one core in a release build (`Receiver::accept`, 2000 rounds, a test that is not kept): 1129 frames each
second, 6.3 MB/s, fill one core. `Mesh::receive` ran the checks under the lock of the route table.

## Done

2026-11-14, commits f6b765c (items 1 to 4 and 6) and 9090a0c (items 5, 7 and 8). Each item has a test; the plant of
each test (the fix off) failed it.

1. `entries` of `src/transfer/zip.rs` takes the cap of entries and refuses a central directory over `ROW_MOST` bytes
   for each entry before it reads it. `transfer_land_a_central_directory_over_its_bound_is_refused`: a directory of
   64 MiB for 1 entry is refused; the plant read it ("a bad row of the central directory").
2. `for_terminal` of `src/transfer/mod.rs`; `receive` of `src/cli/cmd/transfer.rs` prints a text and a path with it.
   `transfer_for_terminal_escapes_each_control_character`.
3. `accept_direct` of `src/transfer/connect.rs` runs `LISTEN_MOST` (64) handshakes at one time.
   `wormhole_transfer_the_listener_holds_at_most_its_bound_of_handshakes`: with a bound of 4 the fifth connection is
   closed with no byte; the plant sent it the handshake.
4. `Landing::undo` of `src/transfer/land.rs` removes with `unlinkat` under the descriptor of `--out`.
   `transfer_land_the_undo_does_not_follow_a_swapped_directory`: the plant (a removal by path) removed the file at the
   end of the link.
5. `learn_at` of `src/router/guest/veth.rs`. `guest_wire_veth_a_guest_cannot_take_the_address_of_another`: the plant
   (a new MAC address replaces the old one) failed it.
6. `expect` of `src/transfer/transit.rs` reads the whole handshake and compares with `same`.
   `transfer_transit_the_handshake_is_compared_in_constant_time`: the plant (the early return) read 1 byte of 29.
7. `ipv6_warning` of `src/router/kernel/guard.rs`; the first apply logs it. `kernel_guard_warns_when_the_host_forwards_ipv6`.
8. `src/link_announce/limit.rs`, `signature_checks` of `src/mesh_announce.rs`; `receive_stream` drops a frame over
   the bucket and `Mesh::receive` verifies before the lock. `a_flood_of_frames_is_bounded_by_the_checks_of_the_stream`:
   20 frames with a bucket of 17 checks: 1 taken, 12 limited; the plant limited none.

`cargo test -p podnet --lib transfer`: 21 passed. `cargo test -p podnet --lib router::guest`: 17 passed. `cargo test
-p podnet --test transfer_land --test link_announce`: 10 and 9 passed. `cargo test --workspace --no-fail-fast` on the
tree of 9090a0c: each target passed. `CC_i686_unknown_linux_musl=clang cargo check --workspace --target
i686-unknown-linux-musl`: no error.


# T-348: capnp: a list in a segment that does not exist panics the reader of a message of the edge

**Source:** the fuzz run 43590953876 of `fuzz.yml` on 9610316, target `capnp_message` (2026-11-14); GitHub #554
and #555 (COORD-079, item 2).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The fuzz run 43590953876 stopped the target `capnp_message` with a panic: `index out of bounds: the len is 1 but
the index is 10`. The crash input (base64):
`AAAAAAsAAAAAAAAAAQAHAAMIAAAAAABdAgAAAAAAAAA2AAAAAAAAAAA/ACAAAAIAAAAAAAAABZECAAAACgAAAAUAAAACAAAAAAAAADYAAAAAAAAAAD8AIAAAAgAAAAAAAAAAAAAAAAA=`.
The input comes from the Cloudflare edge (the RPC of the tunnel), so a peer can stop the process.
`PodUtils/podnet@7fe60e0:src/cloudflare/capnp.rs` is frozen for `pod-cloudflare` of PodUtils/podcrates; a fix is allowed.

## Premise

- Read at 7f22571: `check_range` of `PodUtils/podnet@7fe60e0:src/cloudflare/capnp.rs` gave a length of 0 to a segment that does not exist,
  so a list of 0 words in segment 10 passed the check. `Reader::bytes` then indexed `self.msg.segments[segment]`
  with no check: the segment index 10 in a message of 1 segment panics. A double-far pointer names the segment of
  the content.
- `data_word` indexed the segment with no check too; the struct pointers pass `check_range` first.
- Measured: the crash input panics `fuzzing::capnp_message` at 7f22571.

## Approach

- `check_range` refuses a segment that does not exist, also for 0 words.
- `bytes` reads its words with `get` over the range of the list, and `data_word` with `get`: no reader of a list
  indexes a segment.
- The test `fuzz_a_list_in_a_missing_segment_is_an_error` of `src/fuzzing.rs`: the crash input, and a double-far
  pointer to each kind of list in segment 10. The file `capnp.rs` is at its count of the size allowlist, so the
  test is in `src/fuzzing.rs`.
- Give the commit to podcrates (PodUtils/podcrates#156), so that `pod-cloudflare` takes it (their T-029).

## Prove

```sh
cargo test -p podnet --lib fuzzing
cargo test -p podnet --lib cloudflare
cd fuzz && cargo +nightly-2026-10-15 fuzz run capnp_message corpus/capnp_message -- -max_total_time=120
```

## Done

2026-11-14, commit 54637d9. `check_range` of `PodUtils/podnet@7fe60e0:src/cloudflare/capnp.rs` refuses a segment that does not exist (also
for 0 words); `bytes` and `data_word` read their words with `get`. The file keeps its count of the size allowlist
(944 lines). `fuzz_a_list_in_a_missing_segment_is_an_error` of `src/fuzzing.rs`: the crash input, and a double-far
pointer to a list of each kind in segment 10; at 7f22571 it panicked ("the len is 1 but the index is 10").
`cargo test -p podnet --lib -- fuzzing capnp cloudflare`: 71 passed. `cargo test --workspace --no-fail-fast` on the
tree of 54637d9: 1986 passed, 0 failed (120 targets). `CC_i686_unknown_linux_musl=clang cargo check --workspace
--target i686-unknown-linux-musl`: 0. Fuzz, nightly-2026-10-15 and cargo-fuzz 0.13.1, on the tree of 243eb48, with
the seeds and the crash input in the corpus (`-max_total_time=120 -malloc_limit_mb=64 -rss_limit_mb=2048
-timeout=10`): `capnp_message` 172099 runs in 121 s, no crash. The fuzz corpus is not in the repository
(`fuzz/.gitignore`), so the crash input is in the test only. The commit goes to PodUtils/podcrates#156 for
`pod-cloudflare` (their T-029).

# T-349: proxy: a password in the URL form of a Unix proxy shows in each line that names the proxy

**Source:** the fuzz run 43590953876 of `fuzz.yml` on 9610316, target `proxy_url` (2026-11-14); GitHub #554 and
#555 (COORD-079, item 1).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The fuzz target `proxy_url` asserts that the label of a parsed proxy holds no `@`. Its crash input is
`socks5+unix://ss@proxy:2]:/`. `Proxy::parse("socks5h+unix://user:hunter2@x")` of `pod-proxy` succeeds with the Unix
path `//user:hunter2@x`, and `Proxy::label()` prints the path whole. Measured with podnet `0.4.0-beta.10`:

```
$ podnet --proxy 'socks5h+unix://user:hunter2@x' doctor
proxy  socks5h+unix://user:hunter2@x (credentials: none)
    api.trycloudflare.com:443 through the proxy socks5h+unix://user:hunter2@x: could not reach the proxy unix://user:hunter2@x: ...
```

The fix of the crate is entry 612 of PodUtils/podcrates (PodUtils/podcrates#156). podnet guards its own parse until a
tag carries it.

## Premise

- Read at 54637d9: podnet hands a proxy spec to `pod-proxy` in `parse` and `set_proxy` of `src/egress.rs` (also the
  variables and the files of tools that `way` reads), and in `parse` of `src/networks.rs` (`parse_trusted`).
  `podnet doctor`, `podnet status` and each dial take the proxy from these.
- Measured: `doctor_a_unix_proxy_with_user_info_shows_no_password` failed at 54637d9: exit 1, and the JSON of the
  doctor held `hunter2` in each lane.
- Not measured: the info line of `pod-proxy` for the proxy of a tool's file (`or_detected`) prints the label before
  podnet can see it.

## Approach

- `check_unix_path` of `src/egress.rs`: the forms of a Unix proxy of `pod-proxy` (`SCHEME+unix:PATH`,
  `SCHEME://unix:/PATH`) and `unix:PATH` of the pod hops. A path that starts with `//HOST` or holds `@` is a usage
  error (exit 64). The text names the form, never the path.
- `parse` and `set_proxy` call it first. `way` and `set_proxy` read the variables through `guarded_var`, which passes
  over a refused value with a warning. `without_unix_user_info` drops such a proxy from a tool's file.
- `parse` of `src/networks.rs` calls it before each proxy form (`--egress`, `link --via`, the pod hops).
- The fuzz target `proxy_url` takes podnet's `egress::parse`.

## Decision

2026-11-14: `///PATH` (an empty authority) stays valid: `socks5+unix:///run/proxy.sock` is a form of the pod hops that
a test of `src/networks/tests.rs` holds. Only `//HOST` and `@` are refused.

2026-11-14, the take of podcrates v0.45.0 (their entry 612; the owner is away): `pod-proxy` refuses a Unix
path that starts with `//HOST` or holds `@`, and its text names the form, not the path. `SCHEME+unix:///PATH` stays
valid in the crate. The call: `check_unix_path` stays as a second check. It gives the exit 64 of a usage fault (the
error of `Proxy::parse` has no exit class), and it refuses `unix:PATH` of the pod hops, which `pod-proxy` does not
parse. `guarded_var` stays with it. `without_unix_user_info` goes: the proxy of a tool's file with such a path is now
an `Err` of the crate, so `or_detected` writes the warning "a tool's file names a proxy that cannot be used" with no
path, and no longer the info line with the label. This closes the gap of the Premise.

## Prove

```sh
cargo test -p podnet --lib egress
cargo test -p podnet --lib fuzzing
cargo test -p podnet --test doctor
cd fuzz && cargo +nightly-2026-10-15 fuzz run proxy_url corpus/proxy_url -- -max_total_time=120
```

## Done

2026-11-14, commit aded9d1 (the take of v0.45.0: T-137, e84ebdf). `without_unix_user_info` of `src/egress.rs` is gone; `check_unix_path`
stays (the Decision). New tests: `pod_proxy_refuses_a_unix_path_with_user_info` (`src/egress.rs`: four specs refused by
`Proxy::parse` with no secret in the text, `socks5+unix:///run/x.sock` parsed as a Unix proxy, and a `~/.gitconfig`
with `socks5h+unix://user:hunter2@x` gives no proxy and one skipped entry with no password) and
`a_tool_file_with_a_unix_proxy_password_shows_no_password` (`tests/doctor.rs`: `podnet status --json` with that
`~/.gitconfig` and `RUST_LOG=info`; no line holds `hunter2`, and the warning names `.gitconfig`). The tests of T-349
stay green: `a_unix_proxy_with_user_info_is_refused`, `fuzz_a_unix_proxy_label_holds_no_user_info`,
`doctor_a_unix_proxy_with_user_info_shows_no_password`.

2026-11-14, commit 243eb48. `check_unix_path`, `guarded_var` and `without_unix_user_info` of `src/egress.rs`; the
check in `parse` of `src/networks.rs`; `proxy_url` of `src/fuzzing.rs` takes `egress::parse`; `docs/proxies.md`.
Tests: `a_unix_proxy_with_user_info_is_refused` (six specs, the crash input among them: exit 64 from `parse` and
`set_proxy`, a usage error from `networks::parse`, and no text holds the secret; `socks5h+unix:/run/x.sock`,
`socks5+unix:///run/x.sock` and `unix:/run/x.sock` pass), `fuzz_a_unix_proxy_label_holds_no_user_info`, and
`doctor_a_unix_proxy_with_user_info_shows_no_password` of `tests/doctor.rs` (`--proxy` exits 64; `HTTPS_PROXY` and
`ALL_PROXY` with the spec: no line of `podnet doctor --json` and of `podnet status --json` holds the password; a normal
Unix path is in the output). At 54637d9 the doctor test failed: exit 1, and each lane held `hunter2`.
`cargo test --workspace --no-fail-fast`: 1988 passed, 1 failed (`llms_files_are_current`: `llms-full.txt` after the
change of `docs/proxies.md`; scripts/gen-llms.py wrote it, and the test then passed). `sh scripts/agent/check.sh
notest`: each step 0. The 32-bit check: 0. Fuzz (as T-348): `proxy_url` 1355113 runs in 121 s with the crash input in
the corpus, no crash. Left: the guard stays until a tag of podcrates carries their entry 612; then the take removes it
or keeps it as a second check.

CI: secrets 43603461673 on 3ad5315 failed: TruffleHog marked the test vector `http://user:hunter2@proxy:3128` of the tests
of `check_unix_path` (src/egress.rs:582 at 243eb48) as `unknown` (it tried the host `proxy`). It is no credential:
`.github/secrets-allow.txt` names it.

# T-350: pod: a packet socket opens in a seccomp pod, and the raw rule of T-334 is in the supervisor

**Source:** the take of podcrates `v0.44.0` (T-137; PodUtils/podcrates#154, their entries 574 and 558, the answer to
note 1 of GitHub #550) (2026-11-14).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The notes of podcrates `v0.44.0`: "`pod-hold`: `Calls::allow_raw()` no longer opens the ping sockets, and
`Calls::allow_ping()` does. `AF_PACKET`, `AF_XDP` and `SOCK_PACKET` are in the raw part. Under `--allow-raw icmp`,
podnet's handler must refuse them." The filter of the pod was `Calls::sockets().allow_raw()`
(`crates/podnet-netns/src/seccomp/filter.rs`): with the tag it refuses each ping socket, so the ping to the gateway of
T-066 is lost. With `allow_raw` the supervisor gets the packet sockets, and `handle_socket` gives each socket that is
not of `AF_INET` or `AF_INET6` to the kernel (`notif_continue`): a program with `CAP_NET_RAW` opens an `AF_PACKET`
socket and sends frames past the upstream of the pod. podnet asked to move its raw rule of T-334 into the kernel filter
(GitHub #550, note 1).

## Premise

- Read at c48f2ae and in PodUtils/podcrates:crates/pod-hold/src/filter.rs at `v0.44.0`: `Calls::sockets()`
  refuses the raw part and the ping part; `allow_raw` and `allow_ping` open one part each.
- Measured as root in this container: with the filter of the tag and the old rule of `raw_refused`, under
  `--allow-raw icmp` the sockets `AF_PACKET` (raw and datagram), `AF_XDP` and `SOCK_PACKET` opened. With
  `Calls::sockets()` alone, five tests of `tests/pod_seccomp_dgram.rs` failed (the ping socket gets `EPERM`).

## Approach

- `calls(allow_raw)` of `crates/podnet-netns/src/seccomp/filter.rs`: `Calls::sockets().allow_ping()`, and
  `.allow_raw()` with `--allow-raw icmp`. The raw rule of T-334 is in the kernel filter by default.
- `raw_refused` of `crates/podnet-netns/src/socket.rs`: each socket of `AF_PACKET` and `AF_XDP` and `SOCK_PACKET` of
  `AF_INET` and `AF_INET6` gets `EPERM`, also with `--allow-raw icmp`; `SOCK_RAW` unless `--allow-raw icmp`.
- Tests: the C program `RAW` of `tests/pod_seccomp_dgram.rs` tries the packet sockets in both modes; unit tests of
  `calls` and `raw_refused`.
- `docs/pods.md`, `crates/podnet-netns/README.md`, the help of `pod`, the `not_held` list of `pod doctor`.

## Decision

2026-11-14 (`docs/decisions.md`, A raw and a packet socket in a seccomp pod): the default rule moves into the kernel
filter. The supervisor keeps `raw_refused`: under `--allow-raw icmp` the filter lets the whole raw part through, so the
supervisor is the only place that can refuse the packet sockets; by default it is a second check.

## Prove

```sh
cargo test -p podnet-netns
cargo test --test pod_seccomp_dgram
cargo test --test pod_seccomp_rules
```

## Done

2026-11-14, commit 6e9e9ee (the take of v0.44.0, T-137). `calls(allow_raw)` of
`crates/podnet-netns/src/seccomp/filter.rs` and `raw_refused` of `crates/podnet-netns/src/socket.rs`. Tests, as root
here (no skip line): `cargo test --test pod_seccomp_dgram`: 20 passed, among them `seccomp_raw_socket_gets_eperm_by_default`
(raw ICMP of each family, `AF_PACKET` raw and datagram, `AF_XDP` and `SOCK_PACKET` get `EPERM`; the ping socket
opens), `seccomp_raw_socket_opens_with_allow_raw_icmp` (the raw ICMP socket opens; each packet socket gets `EPERM`)
and `seccomp_icmp_echo_to_the_gateway`. `cargo test -p podnet-netns`: 147 passed, among them
`the_raw_part_is_refused_in_the_kernel_unless_allow_raw` and `packet_sockets_are_refused_with_allow_raw_too`. Plants:
the old `raw_refused`: `seccomp_raw_socket_opens_with_allow_raw_icmp` failed (each packet socket opened), and
`seccomp_raw_socket_gets_eperm_by_default` passed, as the kernel filter refuses the raw part first; the filter
`Calls::sockets()` with no `allow_ping`: 5 tests failed (the echo to the gateway among them). `sh scripts/agent/check.sh`:
each step 0 (`cargo test --workspace`: 1991 passed, 0 failed, 120 targets). The 32-bit check: exit 0.


# T-351: Publish `v0.4.0-beta.11`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease unattended when a batch of fixes is green. The
batch since `v0.4.0-beta.10`: deny 43610669298, live 43610676971 and fuzz 43610680236 on 7860081 passed (GitHub #561);
ci 43617081662 and secrets 43617085112 on 3b3b9be passed (GitHub #562). The request of the release is GitHub #563.
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.10` has none of the work after it: the fixes of the security review (T-341 to T-347), the two fuzz crashes
(T-348, T-349), `podnet wg serve`, the transfer with the Python client, the router fixes and the take of podcrates
`v0.45.0`.

## Premise

- The steps are `docs/maintenance.md`, "Release". A push of the cloud session starts no workflow run: each dispatch goes
  to the owner as a `[gh action] requested` issue.
- The run of `release.yml` on the tag is a `workflow_dispatch` run on the branch of the tag (`--branch TAG`).

## Approach

1. Set the version `0.4.0-beta.11` in `Cargo.toml`, `crates/podnet-capi/Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.11.md`, with `## Breaking`.
3. Push, and ask in one request for `ci`, `deny`, `secrets` and `live` on that commit, the dry run of `release.yml`, the
   publish with `publish=v0.4.0-beta.11`, and `release-proofs.yml`.
4. Push nothing between the request and the tag.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next prerelease unattended when a
batch is green.

## Prove

```sh
cargo test --test docs
# the runs of release.yml (dry run, publish) and release-proofs.yml pass
```

## Done

2026-11-14, 6491f9d (the tag `v0.4.0-beta.11`). GitHub #563 on 6491f9d: the dry run 43619124468, ci 43619108933, deny
43619113053, secrets 43619116496 and live 43619120794 passed; the publish 43621310830 passed (the same run is the run on
the tag: `--branch v0.4.0-beta.11` finds it); release-proofs 43623174074 passed.

# T-352: Publish `v0.4.0-beta.12`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease unattended when a batch of fixes is green. The
batch since `v0.4.0-beta.11`: ci 43632069484, deny 43632072542, secrets 43632076272 and fuzz 43632083716 on b7c3e90
(GitHub #565), live 43638808808 on 5db015c (GitHub #566) passed. The request of the release is GitHub #567.
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.11` has none of the work after it: the take of podcrates `v0.46.0`, the `s3` carrier of the link (T-279)
and the split session (T-302).

## Premise

- The steps are `docs/maintenance.md`, "Release". A push of the cloud session starts no workflow run: each dispatch goes
  to the owner as a `[gh action] requested` issue.
- The run of `release.yml` on the tag is a `workflow_dispatch` run on the branch of the tag (`--branch TAG`).

## Approach

1. Set the version `0.4.0-beta.12` in `Cargo.toml`, `crates/podnet-capi/Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.12.md`, with `## Breaking`.
3. Push, and ask in one request for `ci`, `deny`, `secrets` and `live` on that commit, the dry run of `release.yml`, the
   publish with `publish=v0.4.0-beta.12`, and `release-proofs.yml`.
4. Push nothing between the request and the tag.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next prerelease unattended when a
batch is green.

## Prove

```sh
cargo test --test docs
# the runs of release.yml (dry run, publish) and release-proofs.yml pass
```

## Done

2026-11-14, b6e7959 (the tag `v0.4.0-beta.12`). GitHub #567 on b6e7959: the dry run 43641676226, ci 43641661040, deny
43641664659, secrets 43641667797 and live 43641671768 passed; the publish 43643432345 passed (the same run is the run on
the tag); release-proofs 43645375545 passed.

# T-355: Publish `v0.4.0-beta.13`: the dry run, the publish and the release proofs

**Source:** `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease unattended when a batch of fixes is green. The
batch since `v0.4.0-beta.12`: ci 43708707139 and deny 43708710128 on f1327d6 (GitHub #578); ci, deny and secrets on 08802d8
(GitHub #576); live 43654446240 and fuzz 43654449404 on 6536905 (GitHub #569). The request of the release is GitHub #579.
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.12` has none of the work after it: `devtunnel://` with a tunnel access token or no token (T-354), the
fix of a lost half of a split session (T-353, podcrates `v0.51.0`) and the takes of podcrates `v0.47.0` to `v0.51.0`.

## Premise

- The steps are `docs/maintenance.md`, "Release". A push of the cloud session starts no workflow run: each dispatch goes
  to the owner as a `[gh action] requested` issue.
- The run of `release.yml` on the tag is a `workflow_dispatch` run on the branch of the tag (`--branch TAG`).

## Approach

1. Set the version `0.4.0-beta.13` in `Cargo.toml`, `crates/podnet-capi/Cargo.toml`, `crates/podnet-serve/Cargo.toml`,
   `crates/podnet-vnet/Cargo.toml`, `Cargo.lock` and `fuzz/Cargo.lock`, and regenerate the llms files.
2. Write `docs/releases/v0.4.0-beta.13.md`, with `## Breaking`.
3. Push, and ask in one request for `ci`, `deny`, `secrets` and `live` on that commit, the dry run of `release.yml`, the
   publish with `publish=v0.4.0-beta.13`, and `release-proofs.yml`.
4. Push nothing between the request and the tag.

## Decision

The owner (`CLAUDE.md`, Releases, 2026-11-09): betas are allowed, no crates.io; cut the next prerelease unattended when a
batch is green.

## Prove

```sh
cargo test --test docs
# the runs of release.yml (dry run, publish) and release-proofs.yml pass
```

## Done

2026-11-15, 0a0f3e5 (the tag `v0.4.0-beta.13`). GitHub #579 on 0a0f3e5: the dry run 43711413173, ci 43711402086, deny
43711405265, secrets 43711407779 and live 43711410272 passed; the publish 43713471880 and the run on the tag 43714213695
passed; release-proofs 43716868782 passed.

# T-356: vnet shim: an IPv4-mapped IPv6 address gets the IPv4 abstract name

**Source:** PodUtils/podrunner#170 (COORD-058 of podrunner), data from a peer session.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

podrunner#170: "`to_abstract` in `podnet-vnet.c` does not map an IPv4-mapped IPv6 address: a connect to
`[::ffff:127.0.0.1]:7001` becomes `\0podnet-vnet/6/::ffff:127.0.0.1/7001`, not `\0podnet-vnet/4/127.0.0.1/7001`."
.NET's `SocketsHttpHandler` uses dual-mode `AF_INET6` sockets and connects to the mapped form (PodUtils/podrunner#69,
T-317 for the seccomp backend). Thus a .NET client under the shim does not meet a listener on `127.0.0.1:PORT`.

## Premise

- `crates/podnet-vnet/shim/podnet-vnet.c`: `route_match` already reads a mapped address as IPv4. `to_abstract`,
  `ctl_register`, `finish_accept`, `recvfrom` and `recvmsg` do not.
- An `AF_INET6` descriptor must get an `AF_INET6` address back (the mapped form) from `accept`, `recvfrom` and
  `recvmsg`, and an `AF_INET` descriptor an `AF_INET` address.
- Not measured: the official GitHub runner in a podvm unit.

## Approach

1. In `to_abstract`, make a mapped address (and `::1` with `PODNET_VNET_MAP_LOOPBACK`) a `sockaddr_in` first. Thus
   bind, connect, sendto and sendmsg agree.
2. Send the IPv4 form to the control socket.
3. On the reverse path, give the address in the family of the descriptor.

## Prove

```sh
cargo test --test shim_direct
cargo test --test shim_forward a_mapped_ipv6_connect_meets_an_ipv4_listener
```

## Done

2026-11-15, d794352. `tests/shim_forward.rs`, `a_mapped_ipv6_connect_meets_an_ipv4_listener`: before the fix it failed
(the client connect to `podnet-vnet/6/::ffff:127.0.0.1/17001` met no listener, and the server waited 30 s); after the fix
it passes. An `AF_INET6` client on `::ffff:127.0.0.1` meets an `AF_INET` listener of `127.0.0.1` and the other way, and
each side gets addresses in the family of its descriptor. `::1` with `PODNET_VNET_MAP_LOOPBACK` now gets the name
`podnet-vnet/4/10.66.0.2/PORT` (`shim_direct`, `ipv6_loopback_maps_to_the_local_ipv4_name`). `sh scripts/agent/check.sh`
passed (2016 tests), and `cargo clippy --workspace --all-targets --features full -- -D warnings` passed. Not measured: the
GitHub runner in a podvm unit.

# T-357: vnet shim: `PODNET_VNET_FORWARD` sends a TCP destination to a unix socket path

**Source:** PodUtils/podrunner#170 (COORD-058 of podrunner), data from a peer session.
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

podrunner#170: on a host that refuses each `AF_INET`/`AF_INET6` `bind` and `connect` with `EACCES` (loopback too)
but allows `AF_UNIX`, the GitHub runner (a dynamic .NET program) must reach the broker at `http://127.0.0.1:PORT/` and
an egress proxy at `http://127.0.0.1:PROXYPORT`. podrunner serves both on unix socket paths (for example
`/run/podrunner/broker.sock`). Ask: "`PODNET_VNET_FORWARD`, a comma list of `ADDR:PORT=unix:/PATH`. A stream `connect`
to a listed destination connects the descriptor to that AF_UNIX path, with no SOCKS step and no other process.
Precedence: the forward list first, then the SOCKS routes, then the abstract name."

## Premise

- The shim already sends a listed destination to a SOCKS5 door on a unix path (T-167, `connect_through_door`). A
  forward is the same connect with no SOCKS step.
- A bad entry: the C shim ignores it and logs it (`PODNET_VNET_LOG`); `podnet net shim --forward` exits 64.
- Not measured: a run inside a podvm unit next to the `LD_PRELOAD` interposer of podvm.

## Approach

1. Parse `PODNET_VNET_FORWARD` in the C shim: 16 entries at most, an absolute path that fits `sun_path`.
2. In `connect`, match the forward list before the SOCKS routes. A mapped IPv6 address matches its IPv4 entry.
3. Add `--forward ADDR:PORT=unix:/PATH` to `net shim`, with a check in `crates/podnet-vnet/src/shim.rs`.
4. Document it in `docs/virtual-network.md`, the manual and the help.

## Prove

```sh
cargo test --test shim_forward
cargo test -p podnet-vnet --lib shim
cargo test --test cli net::
```

## Done

2026-11-15, 21593c2. `PODNET_VNET_FORWARD` and `net shim --forward ADDR:PORT=unix:/PATH`. `tests/shim_forward.rs`,
`a_forward_connects_to_a_unix_socket_path`: a client of `AF_INET` and `AF_INET6` reaches a `UnixListener` of the test at
`127.0.0.1:7001`, `[::ffff:127.0.0.1]:7001` and `[::1]:3128`; the descriptor is `AF_UNIX` (`SO_DOMAIN` 1); `getpeername`
gives the TCP address; the forward comes before a SOCKS route of `127.0.0.0/8`; a missing path gives `ECONNREFUSED`; a
bad entry is ignored and logged. `a_destination_with_no_forward_keeps_its_abstract_name` passes. `net::shim_takes_forwards`:
exit 64 at a bad forward. `forwards_are_checked` (the unit test of the forms). `sh scripts/agent/check.sh` passed (2016
tests), and clippy with `--features full` passed. Not measured: a run in a podvm unit next to the `LD_PRELOAD`
interposer of podvm.

# T-358: The release gate: each beta passes a scenario run of its own binary on restricted hosts

**Source:** the owner, 2026-11-15: podrunner, a sibling that uses podnet, did not survive its first contact with real
testers, and came off as a toy. "How are you planning to catch these bugs before a release is pushed; the beta testers
won't be always available or willing to test." `docs/decisions.md`, 2026-11-15, The release gate.
**Category:** chore
**Milestone:** M5
**Priority:** P1
**Effort:** L
**Status:** partial

## Problem

A beta of podnet ships when `ci`, `deny`, `secrets` and `live` pass on a commit. Three betas (`v0.4.0-beta.11` to `beta.13`)
shipped on 2026-11-14 and 2026-11-15 that way. Each set of notes has a growing list "Not measured against a live service".
The checks run `cargo test` against stand-ins, inside one runner, as the authors of the code imagined the use. A user runs
the released static binary, on a host with its own limits, from the documents. Bugs between those two places reach the
users first: PodUtils/podrunner#69 (a dual-mode IPv6 connect of .NET, podnet T-317) and PodUtils/podrunner#170 (the same
class in the vnet shim, podnet T-356) came from a consumer, not from a check of podnet.

## Premise

- Read on 2026-11-15: `.github/workflows/live.yml` runs the release-mode binary against live services, one runner, with
  root. `ci.yml` has the jobs `restricted-host` and `no-loopback`. No job runs the documented commands of `docs/` as a
  user types them, and no job runs a consumer's real use (a dynamic .NET or Python client under the shim, a podvm unit).
- podcrates has `pod-testkit` and `pod-netlab`. podrunner proposes a shared harness for a lab (the owner, 2026-11-15).
  Nothing was built or measured for this entry.

## Approach

1. **The scenario lab.** A workflow `gate.yml` that takes the release binary of the dry run (not `cargo test`) into
   containers that copy the target hosts of `docs/target-environment.md`: no `CAP_NET_ADMIN`, egress through an HTTP proxy
   only, a seccomp filter that refuses `AF_INET`/`AF_INET6` with `EACCES` (the host of PodUtils/podrunner#170), IPv6 only,
   a read-only root, no `/dev/net/tun`. Build it on the shared harness when podrunner's proposal lands; take `pod-netlab`
   if it fits.
2. **The documents as tests.** Each `sh` block of `README.md` and `docs/*.md` that a user runs is a scenario: it runs in
   the lab as written (a marked list of the blocks that need a live service or a second host). A block that fails is a
   defect of the code or of the document.
3. **The consumers as tests.** The real use of each sibling: a .NET `HttpClient` and a Python `requests` client under
   the shim with AF_INET refused; `podnet pod` around a dynamic program; the relay and the tunnel as podssh and podrunner
   call them. A new consumer report adds a scenario before its fix.
4. **The gate.** `release.yml` publishes only after `gate.yml` passed on the same commit. The notes of a beta list each
   scenario and its result; a feature with no passing scenario is marked experimental in its help and in the notes.
5. Record each scenario failure as a defect entry; a flake is a defect, not a re-run.

## Decision

A call of the lead (2026-11-15), recorded in `docs/decisions.md`: no beta after `v0.4.0-beta.13` until the gate runs.
The owner can rule otherwise.

## Prove

```sh
gh workflow run gate.yml -R PodUtils/podnet --ref main
# the gate passes on the head; a planted defect (the vnet shim without T-356) fails it
```

## Correction

2026-11-15: podcrates takes the shared lab as `pod-lab` (their entry 692, PodUtils/podcrates#180, the owner's ruling "Discipline"). Part 1 (the tag after `v0.53.0`): the profiles as data, `podlab check` and `podlab run PROFILE -- CMD` on a host with no root. Part 2: the step of GitHub Actions, the machine tier of podvm, and the network profiles, which podnet holds (`proxy-only`, `bind-denied`). Step 1 of the Approach runs in the profiles of `pod-lab`, not in a lab of podnet's own; podnet writes the network profiles for part 2 (COORD-091, GitHub #585).

2026-11-15, step 2, the first slice. `scripts/doc-commands.py` reads each fenced `sh` and `bash` block of `README.md`
and `docs/*.md` (not `docs/releases/`, `docs/reports/`, `docs/research/`). `scripts/doc-commands.txt` gives each
block a class (`run`, `live`, `root` or `skip`) and a reason, by the hash of its text (`docs/development.md`,
Documented commands). `--check` fails on a block with no class and on a stale line; `check.sh` and the job
`workflows` of `ci.yml` run it. `--run` runs each `run` block with `sh -e` in a new HOME and working directory with
the built binary; the job `test` of `ci.yml` runs it (about 2 s). The count on 2026-11-15: 97 blocks, 4 `run`, 57
`live`, 1 `root`, 35 `skip`. Here 3 `run` blocks passed; `docs/networks.md` (the passwd shim and `ssh-keygen`) did not
run, as this host has no `ssh-keygen`. Single lines of `skip` and `live` blocks, run by hand, gave T-361, T-362 and
T-363.

What remains of step 2: most blocks are reference lists that mix runnable lines with placeholders, foreground servers
and second hosts, so few blocks run as written. Next: (a) a scenario form for a block that needs a server in the
background and a client (the `--local` blocks of `docs/serve.md`), or blocks that are written to run; (b) the `live`
blocks in `live.yml`; (c) the `root` block and the blocks that need a restricted host in the profiles of `pod-lab`
(step 1).

2026-11-16, step 1, the first run. `podlab` of podcrates `v0.56.0` (release, musl) and the release binary of podnet
(musl) on this container (uid 0, all capabilities, kernel 6.18, user namespaces allowed, `/dev/net/tun` of mode 0600).
`scripts/lab-lane.sh PODLAB PODNET [PROFILE...]` runs the lane (`docs/development.md`, The lane in the profiles of
pod-lab): `--version`, `man --no-pager`, `doctor`, `net shim --forward` with a dynamic Python client and an echo server
outside the lab (and a negative control with no shim), `link listen` and `link dial` over the unix carrier, `pod
--backend seccomp` and `--backend netns` around `/bin/true`, `net proxy --listen unix:PATH` with a SOCKS5 greeting.

- `podlab check PROFILE` (the host itself): only `no-kvm` holds; 14 profiles fail (the host allows what they refuse),
  19 are unseen (no row of `pod-hostprobe` measures them).
- `podlab run PROFILE -- /bin/true`: as uid 65534, 25 of the 34 labs hold. As root, 24 hold: `nonroot-nocaps` does
  not (a lab cannot drop root), and each other lab runs as root. The 9 labs that never hold here: `proxy-only`,
  `arm64-tcg`, `cgroup-v1` and the 6 network profiles of podcrates (their entry 716) (part 2 of `pod-lab`).
- As uid 65534, in the 25 labs: each `--version`, `man`, `doctor` and `shim-forward` passed (100 of 100). `link-unix`
  failed in the 5 Landlock labs (`root-unreadable`, `no-kvm`, `proc-masked`, `no-tun`, `no-net-admin`): T-380.
  `net-proxy-unix` failed there too: T-379, fixed (it passes after the fix). `pod-netns` failed in `udp-denied`: T-381.
  `link-unix` and `net-proxy-unix` are n.a. in `listen-denied` (the lab refuses `listen(2)` of AF_UNIX; podnet stops
  with `Permission denied`, and says `bind` for a refused `listen`). The pod backends are n.a. where the lab takes what
  they need by design: `pod-netns` in `userns-off`, `nonroot-nocaps`, `seccomp-nonewprivs`, `no-proc`, `proc-masked`
  and `raw-denied` (no TUN for this user and no AF_PACKET); `pod-seccomp` in `no-proc` and `proc-masked`. Each other
  row passed.
- As root: the same, except `pod-netns` passes in `userns-off` and `no-proc`, and is n.a. in `no-net-admin` (since
  Linux 5.12 uid 0 with no `CAP_SETFCAP` cannot map itself).
- `docs/lab-profiles/`: the six network profiles of podcrates (their entry 716), in the form of `pod-lab` at `93bf133a`, with the
  way a lab can hold each part and the parts that are assumed.

What remains of step 1: the network labs (part 2 of `pod-lab`: `proxy-only` and the six profiles; the entry 716 of podcrates takes
`docs/lab-profiles/`); the commands of the lane that need them (`tunnel --quick`, `warp proxy`, `relay`, `send` and
`receive`, `doctor` against a stand-in proxy); `link` over `icmp` and `pod --allow-raw icmp` in `raw-denied`; a host where
`podlab` runs as a user that is not root in CI (`gate.yml`); T-380 and T-381 (fixes
of podcrates).

2026-11-16, step 3, the first consumer: the .NET `HttpClient` of the runner of PodUtils/podrunner#170.
`tests/consumers/dotnet-http` (.NET 8) sends GET to `http://127.0.0.1:PORT/` with the default `SocketsHttpHandler`.
`sh scripts/consumer-dotnet.sh PODNET [PODLAB [PROFILE]]` (`docs/development.md`, The consumer .NET HttpClient) runs
it with the release musl podnet, `podnet net shim --forward 127.0.0.1:PORT=unix:PATH` and a broker on a unix socket.
Measured here with dotnet 8.0.134 (Ubuntu's `dotnet-sdk-8.0`) and `podlab` of podcrates `v0.56.0`:

- No lab, no shim: the client gets the body of a TCP server on `127.0.0.1:PORT` (the control).
- No lab, the shim: exit 0 and the body of the broker. The shim log: `socket(AF_INET6) -> AF_UNIX`, `setsockopt
  level=41 optname=26 swallowed` (`IPV6_V6ONLY` off: the dual-mode socket of T-356), `connect -> forward PATH`. The
  broker reads the peer of its AF_UNIX connection with `SO_PEERCRED`: `comm=dotnet`.
- `podlab run bind-denied` as uid 65534, no shim: `HttpRequestException: Permission denied (127.0.0.1:7105)`. The lab
  refuses AF_INET.
- `podlab run bind-denied` as uid 65534, the shim: exit 0 and the body of the broker; the peer is `uid=65534
  comm=dotnet`.
- The plant: a shim with no match of the IPv4-mapped address of a forward (T-356 removed, `CONSUMER_SHIM`) fails both
  shim cases with `Connection refused`.

No defect of podnet. The four cases take about 3 s. The job `static` of `ci.yml` (x86_64) runs the cases with no lab
on its release binary, and its plant; CI does not build `podlab`. What remains of step 3: a Python `requests` client;
`podnet pod` around a dynamic program; the relay and the tunnel as podssh and podrunner call them.

2026-11-16, step 3, the second and the third consumer. Measured with the release musl podnet, `podlab` of podcrates
`v0.56.0`, Python 3.13.18, requests 2.34.2 and curl, as uid 65534 in the labs.

- `sh scripts/consumer-python.sh PODNET PODLAB` (`docs/development.md`, The consumer Python HTTP client): `urllib.request`
  and `requests` send GET to `127.0.0.1:PORT` under the shim with `PODNET_VNET_FORWARD`, and through `HTTP_PROXY` forwarded
  to a proxy on a unix socket (the egress path of PodUtils/podrunner#170, ask 4). Each of the 12 cases passes: `direct`
  gets the decoy; `shim`, `proxy`, `lab-shim` and `lab-proxy` get the body of the unix server, the peer is `python3`
  (uid 65534 in the lab), and the proxy gets `GET http://consumer.test/`; `lab-bare` in `bind-denied` fails with
  `[Errno 13] Permission denied`. A planted shim with no match of a forward fails the 8 shim cases with `Connection
  refused`. About 3 s; the job `static` of `ci.yml` runs the cases with no lab, and the plant.
- `sh scripts/consumer-pod.sh PODNET PODLAB [PROFILE...]` (`docs/development.md`, The consumer pod): python3 and curl in
  `podnet pod -x socks5://unix:PATH`, with `auto`, `netns`, `seccomp` and `netns-kernel`, in each lab of `podlab list`
  (26 labs hold here; 9 do not). The pod gives the exit 42 of the program in each pass. With no pod, the program fails
  in `bind-denied` with `Permission denied`. `CONSUMER_PLANT=decoy` (the upstream sends the name to the decoy) fails each
  pod row. The result: 148 rows pass. n.a. with exit 3 and a reason: `netns` and `netns-kernel` where the lab has no user
  namespace (`userns-off`, `nonroot-nocaps`, `seccomp-nonewprivs`); each backend in `no-proc` and `proc-masked`; `netns`
  in `raw-denied` (no TUN for this user and no `AF_PACKET`); `netns` and `netns-kernel` in `bind-denied` (Landlock
  refuses each TCP connect of the program in each namespace); `netns-kernel` in `listen-denied` and `udp-denied`.
  Failures: `auto` and `netns` in `bind-denied` (T-382, fixed: `auto` takes seccomp, `--backend netns` exits 3);
  `netns` in `udp-denied` (T-381, `pod-hold`); `auto` and `seccomp` in `udp-denied`, no name resolves with no UDP
  socket (T-383, open); `netns-kernel` in `getsockname-denied` (T-384, open).

What remains of step 3: the relay and the tunnel as podssh and podrunner call them; T-381, T-383 and T-384; the
consumers in the network labs (part 2 of `pod-lab`), and in `gate.yml`.

2026-11-16, step 1, the row `proxy-only`. podcrates `v0.58.0` ships `podlab` as a release asset and has `podlab run
--allow-connect PORT PROFILE`. `sh scripts/get-podlab.sh VERSION [DIR]` downloads the asset of this machine and
`SHA256SUMS` into a new directory and checks the SHA-256; `scripts/lab-lane.sh` takes that directory as PODLAB and checks
the asset again. The release has no signature of its assets (their entry 717 of podcrates): no signature was checked. The asset
`podlab-x86_64-unknown-linux-musl` of `v0.58.0` has the SHA-256
`1de3b930907efd833566f1779d492a55f234a7d3f0843d26b90719af3a89e9de`, which is the line of `SHA256SUMS` and the digest that
the GitHub API gives for the asset.

The lane runs the row `proxy-only` in `podlab run --allow-connect PORT bind-denied` (`docs/development.md`, The row
proxy-only), with the stand-in HTTP CONNECT proxy `tests/consumers/connect-proxy.py` on `127.0.0.1:PORT` outside the lab.
Measured with the release musl podnet of `947096d`, as uid 65534:

- `proxy-direct`: the GET of `127.0.0.1:PORT+1` with no podnet fails with `[Errno 13] Permission denied`: pass.
- `proxy-port`: a CONNECT through the proxy with no podnet gets the body: pass (the lab opens the proxy port only).
- `proxy-pod-auto`, `proxy-pod-seccomp` (`podnet pod -x http://127.0.0.1:PORT -- python3 ...`) and `proxy-pod-env`
  (`HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY=localhost,127.0.0.1`, no `-x`): exit 0, the body, and the proxy logged `CONNECT
  proxy.test:PORT+1 HTTP/1.1`: pass.
- `proxy-doctor`: exit 1 (no leg can work), the proxy logged the CONNECT of each probe, and the doctor names `proxy 403`
  for each leg: pass.
- The plants: `LANE_PLANT=refuse` fails `proxy-port` and the three pod rows (the program gets `Connection refused`);
  `LANE_PLANT=port` fails the three pod rows and `proxy-doctor` (podnet: `the upstream 127.0.0.1:PORT+2 does not answer
  (Permission denied)`).

No defect of podnet. The lane gave `pod-netns` in `bind-denied` as a failure: since T-382 the backend exits 3 with a
reason there. The lane now gives it n.a., as `scripts/consumer-pod.sh` does. The whole lane with `podlab` `v0.58.0` as
uid 65534: 181 rows pass; the failures are the known ones (`link-unix` in the 5 Landlock labs, T-380; `pod-netns` in
`udp-denied`, T-381); 9 labs do not hold here (`noexec-home` among them: the lab gives `home-noexec` fails for uid
65534). What remains of the row: a lab where the
direct egress is refused by the network and not by Landlock (`proxy-only` of `pod-lab` part 2), and `tunnel --quick`,
`warp proxy`, `relay`, `send` and `receive` against a stand-in that carries their protocols.

2026-11-16, step 1, the network labs. `podlab` of podcrates `v0.60.0` (`sh scripts/get-podlab.sh v0.60.0
.work/podlab-v0.60.0`: SHA-256 OK) has the network labs of their entry 716. `scripts/lab-lane.sh` runs the rows
`version`, `doctor`, `net-curl`, `net-pod-auto`, `net-pod-seccomp` and `net-pod-netns` in `proxy-only`,
`proxy-tls-reterminate`, `proxy-per-name`, `ws-upgrade-refused`, `sni-allowlist`, `no-proxy-no-dns` and `ipv6-only`
(`docs/development.md`, The network labs). The old row `proxy-only` of the lane (`podlab run --allow-connect`) is now
the row `allow-connect`: the name `proxy-only` is the real lab. Measured as uid 65534 with the release musl podnet of
ad8abd4: 24 rows pass; each `podnet pod` backend gets HTTP 200 from `https://lab.test/` through the proxy of the lab
(with its credential in `proxy-only`, with TLS terminated again in `proxy-tls-reterminate`); the doctor names the proxy
and its credential, and the HTTP 403 of `proxy-only` on port 7844. 8 rows are n.a. in the two labs with no proxy
(`podnet pod` refuses with `no proxy: give -x PROXY`). `ipv6-only` does not hold here (no IPv6 in this kernel). The 6
rows of `allow-connect` pass. The plant `LANE_PLANT=net` (a proxy that does not listen) fails each `net-pod` row. No
defect of podnet.

The full lane with `podlab` v0.60.0 as uid 65534 (each profile of `podlab list` and `allow-connect`): 223 rows pass,
22 are n.a., none fails. `arm64-tcg`, `cgroup-v1` and `ipv6-only` do not hold here. A first run put the podnet binary
in HOME: each row of `noexec-home` failed (the lab mounts HOME noexec). The binary and `LANE_DIR` must not be in HOME.

2026-11-16, `gate.yml` (01487d8): the lane on a GitHub runner (ubuntu-24.04, kernel 7.0, uid 1001, the AppArmor limit of
user namespaces off). Run 43820928202 passed: each row passes or is n.a.; `arm64-tcg` and `cgroup-v1` do not hold;
`ipv6-only` holds there (`version` and `doctor` pass; the pod rows are n.a., no proxy). The plant step passed (the 3
`net-pod` rows of `proxy-only` fail with the plant). `gate.yml` runs by hand; step 4 (`release.yml` waits for it) is
open.

2026-11-16, step 4, the first part. `release.yml` calls `gate.yml` as its job `gate` (in the dry run and on the tag), and
the job `publish` needs it: a release is not published with no passing gate. `gate.yml` passed on 89a6e1e with podlab
v0.61.0 (run 43831896685). The dry run of `release.yml` on 5fbb2e3 (run 43839899756) passed: its job `gate / lane`
passed, then `publish` ran (a dry run: no tag, no proofs).

2026-11-16, step 4, the notes. `gate.yml` keeps the rows of the lane as the artifact `gate`; `scripts/gate-notes.sh`
writes the section "The release gate" (the counts, and each row in a `<details>` block), and the job `publish` of
`release.yml` adds it to the body of the release. It exits 1 on a file with no row and on a failed row. Measured on the
lane of this container (`/var/tmp/lane/full.txt`, 245 rows): with the 8 rows of `noexec-home` of the HOME fault it exits
1; with them removed it gives 215 pass, 0 fail, 22 n.a. and 237 table rows. The dry run of `release.yml` on 81d2e73
(run 43849173659): `gate / lane` passed, and the step "the body of the release" of `publish` passed with the section; the
artifact `gate` of that run gives 226 pass, 0 fail, 24 n.a. (250 rows). What remains of step 4: a feature with no
passing scenario is marked experimental in its help and in the notes.

What remains of step 1: a row that uses WebSocket through the proxy (`ws-upgrade-refused`: the relay and the tunnel);
`ipv6-only` on a host with IPv6; the `icmp` rows.

2026-11-16, step 4, the experimental commands. `src/cli/scenarios.txt` has one line for each command with no
subcommand, and its scenarios: a step of `live.yml`, a step of the job `static` of `ci.yml`, or a case of the lane
(`docs/development.md`, The scenarios of each command). `scripts/gate-scenarios.py` finds each call of the binary in the
steps and in the scripts that they run; the cases of the lane have a map (`LANE_MAP`), checked against the calls of
`scripts/lab-lane.sh`. `--check` runs in `check.sh` and in the job `workflows` of `ci.yml`. The help of a command with no
scenario starts with "Experimental:" (`src/cli/help.rs`; `src/cli/help_tests.rs` checks that each command has a line and
the mark). `release.yml` adds the section "The scenarios of each command" (`--notes LANE.TXT`) after the section of the
gate; a lane scenario counts only when a row of its case passed. A new lane case `local` runs ten commands that need no
network (`completions`, `components`, `status`, `ports`, `qr`, `bug-report`, `net addresses`, `link carriers`, `autostart
list`, `agent status`); as uid 65534 it passed in each lab of `podlab` v0.63.0 that holds here, except `noexec-home`
(the binary was in HOME, the known fault of the setup). The count: 81 commands, 59 with a scenario, 22 experimental
(T-398 gives them scenarios). Step 4 is done.

2026-11-16, the row `way-out` (T-402): `podlab run --way-out proxy-only` runs the case `way-tunnel` (a quick tunnel of
`podnet demo --exit-on-verify` from a host whose only way out is a proxy). gate 43910086958 on ff7fbad failed in two rows:
`way-out` (the lab sets `SSL_CERT_FILE` to the roots of its stand-in servers, which replace the roots of podnet: the case
unsets it) and `wg-serve` in `listen-denied` (the lab refuses the unix listener of the forward: now n.a., as the other
unix listeners). Each other row passed.

What remains: step 1 (above), step 3 (the relay and the tunnel as podssh and podrunner call them), T-398 (T-384 is done).

2026-11-17, step 1, the row with WebSocket through the proxy (T-421): the profile `way-ws` of the lane
(`podlab run --way-out ws-upgrade-refused`) runs `way-relay-refused`, and the profile `way-out` runs `way-relay` beside
`way-tunnel`. Both run `relay connect` to the public relay with a token that it refuses. The row found T-421 (the
WebSocket paths did not take `--ca-file`; a proxy's 403 read as a bad token). With podlab v0.68.0 the three rows pass.
What remains of step 1: `ipv6-only` on a host with IPv6 (the GitHub runner has no IPv6 route); the `icmp` rows. The
release binary is built with no optional feature (`release.yml`: `cargo zigbuild --release`), so its `icmp` carrier is
"not built" (exit 69): a row of it checks nothing of ICMP. The `icmp` rows wait for a release build with the feature.


# T-359: One shared crate for the help and the manual of each PodUtils command line

**Source:** the owner, 2026-11-15: "it is time everyone shared a single crate for cli help/man".
`docs/decisions.md`, 2026-11-15, Help and manual.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Each command line of PodUtils (podnet, podssh, podvm, podrunner) writes its own help, manual page and plain-text manual.
The owner rules that they share one crate.

## Premise

- podnet has `src/manual/` (about 1,970 lines on 2026-11-15): a model of the command tree from `clap`
  (`model.rs`), the roff writer of `podnet.1` (`roff.rs`), the plain-text manual (`text.rs`), the pager (`pager.rs`), the
  facts of the environment variables and exit codes (`facts.rs`), the component inventory (`inventory.rs`), and
  scripts/gen-llms.py for `llms.txt` and `llms-full.txt`. The tests `tests/cli/manual.rs` check it.
- No crate of podcrates holds help or manual code (read at `v0.52.0`).

## Approach

1. A COORD issue to podcrates (to all): the ruling, and `src/manual/` of podnet as a source for a crate (for example
   `pod-manual`): the model, the roff and text writers, the pager, a typed table of variables and exit codes.
2. When the crate is at a tag: podnet takes it (T-137), `src/manual/` becomes glue (the facts of podnet), and the tests
   stay.

## Prove

```sh
cargo test --test cli manual
sh scripts/agent/check.sh
```

## Correction

2026-11-15: podcrates takes `src/manual/` and scripts/gen-llms.py of podnet at `78a3911` as `pod-manual` (their entry 700, PodUtils/podcrates#181); they are frozen (fixes only) until the take. podssh (PodUtils/podssh#267) and podvm (PodUtils/podvm#425) need a model that each program fills (neither uses `clap`); podnet's reader of `clap` becomes one way to fill it (GitHub #584). The owner's ruling "Discipline" asks for a test that fails when the code gives an exit code that has no fact: podnet has the test of the variables (`src/manual/facts.rs`, `each_variable_in_the_source_is_in_the_manual` and its reverse) but none of the exit codes; it comes with the take.

2026-11-15: the blocker (podcrates builds `pod-manual`, their entry 700) ended with the tag `v0.54.0`.

2026-11-15, the take of `pod-manual` (podcrates `v0.54.0`): three parts of the crate do not fit podnet, and the glue of
`src/manual/mod.rs` keeps the old behaviour with a note.

- The reader of clap (`pod_manual::clap::read`) leaves `Flag::env` empty. The glue fills it from the clap tree, so
  that the manual names the variable of each flag (`Variable: NAME.`), as before.
- The reader of clap gives a rule `Excludes` for a conflict with a positional (its value name), and `check::model`
  of the crate refuses a rule that names no flag (7 rules of serve, send, autostart add and stop). The glue keeps
  the rules of flags only.
- `help::render` has no groups of commands and no "Start here" block. The text of `podnet --help` and of
  `podnet COMMAND --help` stays the help of clap (`src/cli/help.rs`), from the same command tree as the model.
  `--help --json` comes from the crate.
- `scan::variables_read` takes each quoted word of the shape of a variable: the test exempts the names of C
  constants, a crate name and a hash name (`EXEMPT` of `src/cli/manual_tests.rs`).
- `PodUtils/podcrates:scripts/check-man.sh` of podcrates: its plant check fails for podnet, as the planted flag (`--add-door`) is
  also in the prose of other flags.

## Done

2026-11-15, 9217b0b, 13b1514 and the commit of this record. `podnet man`, `podnet man --roff`, `podnet man --json`,
`podnet COMMAND --help --json`, the pager and the llms files come from `pod-manual` (feature `clap`). `src/manual/`
keeps the facts of podnet (`facts.rs`: the variables, the files, the exit codes, the examples; `inventory.rs`: the
component inventory, also a table of the manual) and the glue (`mod.rs`). scripts/gen-llms.py is removed; the test
`the_llms_files_equal_the_model` checks the files, and the writer of the crate gives the same `llms.txt` as the
script did. The tests of `src/cli/manual_tests.rs` call the checks of the crate: the text rules, the model, the
flags against the parser both ways, the variables both ways, the exit codes both ways, the llms files, and the
JSON help of each command.

What the checks found and the fixes: the exit codes 4 (`pod --fail-on-violation`), 128+N (a signal) and the code of
the program of `pod` and `warp run` had no fact (now facts; 4 is also in `docs/protocol.md`). The variables of the
CGI of `git serve`, `GIT_PROTOCOL` and `GIT_DIR`, the CA bundles and the npm proxy of `warp up` had no fact (now
facts). 43 sentences of the help and the facts had more than 25 words (each is split; one is in
`src/warp/export/command.rs`, a frozen file: a text fix only). `PodUtils/podcrates:scripts/check-strings.py` of podcrates on a release
binary found work ids in the C source of the shims and in two messages (`src/carriers.rs`,
`crates/podnet-core/src/network/split.rs`): removed. It still finds 84 runs in the data of `pod-services`, 12 runs
of PKCS of a library, and one byte run that is no text. `PodUtils/podcrates:scripts/check-man.sh`: groff and mandoc give no warning,
and each of the 248 flag spellings is in both renderings.

For a user: `podnet man` has the form of `pod-manual`. The first line is the name, the version and the about line
(no `PODNET(1)` header and no NAME section). Each command section shows Usage, Arguments, Options, Rules,
Commands and Examples. ENVIRONMENT has a group of the pager, INVENTORY shows `Kind:` and `Meaning:`, EXIT STATUS
has the rows 4, 128+N and N, and SEE ALSO is a section (`see-also`). The topic `pod` is gone: `podnet man pod` gives
the command with the usage of podnet-netns. An unknown section gives `NAME: no such section; fix: ...` and the
list of sections (exit 64). `--roff` is the man(7) page of the crate. `--json` is the model of the crate (schema
1). `--help --json` at the root gives the root command, not the whole manual. `podnet --help` is not changed,
except the split sentences. `sh scripts/agent/check.sh` passed (2013 tests), clippy with `--features full` passed,
and the i686 check passed.

# T-360: install.sh: a second run with the same version changes nothing and says so

**Source:** the owner's ruling "Installer", 2026-11-15 (PodUtils/podcrates#182; `docs/decisions.md`, 2026-11-15, Installer).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The ruling asks for an installer that is hermetic, idempotent and fast: "a second run changes nothing and says so. An
interrupted run leaves no half install". `scripts/install.sh` downloads and replaces the binary at each run.

## Premise

- Read on 2026-11-15 (`scripts/install.sh`, 501 lines): it checks `SHA256SUMS` and moves the new binary into place with
  `mv -f` after it runs it, so an interrupted run leaves the old binary. It has no check of the installed version, no
  `--dry-run`, no offline install from a file, and issue numbers in its comments. The tests are `tests/install_script.rs`
  and `tests/install_script/strict.rs`. Not measured: the time of a run, and `dash`, `busybox sh`, `shellcheck --shell=sh`.
- podcrates builds a shared POSIX core (their entry 702). If it comes first, podnet takes it in place of this fix.

## Approach

1. Before the download: when the installed binary prints the version of the release (`podnet --version`), and the
   binary's SHA256 matches the line of `SHA256SUMS`, print that the version is installed and exit 0 with no change.
2. A test: two runs against a local release; the second downloads only `SHA256SUMS` and changes no file (the mtime and
   the inode of the binary stay).
3. Remove the issue numbers from the comments and the text of the script.
4. Run it under `dash` and `busybox sh` in the test, and `shellcheck --shell=sh` in CI.

## Prove

```sh
cargo test --test install_script
shellcheck --shell=sh scripts/install.sh
```

## Correction

2026-11-16: ci 43796824990 on 1852fcc failed in the jobs `restricted-host` and `no-loopback`.
`the_install_test_passes_in_each_shell` and `an_installer_with_no_pin_check_fails_the_install_test` run
`scripts/install-test.sh`, which serves the fake release on `127.0.0.1`. The two tests now skip with `need_loopback!()`
on a host with no loopback or with `bind(AF_INET)` refused. `sh scripts/restricted-host.sh --test install_script`:
2 tests skipped, 33 passed, exit 0.

## Done

2026-11-16, 0529753. `scripts/install.sh` is the installer core of podcrates v0.58.0 (`scripts/install/core.sh` and
`scripts/install/pins.py`, unchanged, pinned by SHA-256 in `scripts/install/SOURCES`, checked by
`scripts/check-install.py`) with the hooks of podnet. The copy of the tree holds the core inline (the documented line
runs it with `sh -c "$s"`) and takes the pin of the binary from `SHA256SUMS` after the cosign check; `release.yml`
publishes `podnet-install.sh` with the pins (`pins.py stamp`) and installs from it twice. A second run says "podnet
VERSION is already installed", downloads `SHA256SUMS` only, and the binary keeps its inode and its time
(`a_second_run_changes_nothing`, `scripts/install-test.sh` case 2). New: `--dry-run`, `--offline DIR` (the copy of a
release), `--prefix`, `--add-path`, `PODNET_MIRROR`, `PODNET_TARGET`, the noexec and free-space checks. Each old
variable, downloader (`aria2c`, `http`, `python`, `perl` through the part of podnet) and check stays; the comments have
no issue numbers. `scripts/install-test.sh` passes (11 cases) in `dash`, `bash --posix` and `busybox sh`, and in the
podlab profiles small-tmp, noexec-tmp, proc-masked, inherited-fds, root-unreadable, no-passwd-entry, noexec-home,
no-proc, ro-etc-ssl and seccomp-nonewprivs; a copy with no pin check fails it (planted defect). `cargo test --test
install_script`: 33 passed. `shellcheck -S warning` passes; `shellcheck --shell=sh` gives info SC2317 only, on two
hooks of the inline core. `sh scripts/agent/check.sh` passed, and clippy with `--features full` passed.

# T-361: git init: make the root when it does not exist

**Source:** T-358 step 2, the documented command at docs/serve.md:197 (`podnet git init --root ./repos app`), run on
2026-11-15.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`docs/serve.md` gives `podnet git init --root ./repos app` as the first command: "the bare repository ./repos/app.git".
The manual has the same example. In a new directory, the command exits 64 with `podnet: --root ./repos: not a
directory`, and makes nothing. A user who types the documented commands stops at the first one.

## Premise

- Measured on 2026-11-15 with `target/debug/podnet` in a new HOME and a new working directory: exit 64 and the message
  above. `src/cli/cmd/git.rs`, `init`, calls `real_root`, which refuses a root that is not a directory.
- `git serve` and `git urls` keep the refusal: they read a root, they do not make one.
- The block of `docs/serve.md` has the class `live` in `scripts/doc-commands.txt` (its other lines open a quick tunnel
  or the relay), so `doc-commands.py --run` does not run it. The line ran alone.

## Approach

1. In `init`, make the root (`create_dir_all`) when it does not exist, then go on as before.
2. `tests/cli/git.rs`, `git_serve_init_and_urls`: start with no root.

## Prove

```sh
cargo test --test cli git::git_serve_init_and_urls
d=$(mktemp -d) && cd "$d" && podnet git init --root ./repos app && test -f repos/app.git/HEAD
```

## Done

2026-11-15, eeac027. `git init` makes the root with `create_dir_all` when it does not exist. `git_serve_init_and_urls`
starts with no root and passes. In a new directory, `podnet git init --root ./repos app` exits 0 and writes
`repos/app.git/HEAD`. `sh scripts/agent/check.sh` passed (2016 tests), and clippy with `--features full` passed.

# T-362: docs/cli.md: the completion command writes to a directory that can be absent

**Source:** T-358 step 2, the documented command at docs/cli.md:36
(`podnet completions bash > ~/.local/share/bash-completion/completions/podnet`), run on 2026-11-15.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

In a new HOME, the shell refuses the redirection: `~/.local/share/bash-completion/completions/podnet: No such file or
directory`. The document gives no step that makes the directory.

## Premise

- Measured on 2026-11-15 in a new HOME: the shell exits 1. `podnet completions bash` alone exits 0. The defect is in the
  document, not in the code.
- The block has the class `skip` (its last three lines have the placeholder `...`), so `--run` does not run it. The
  line ran alone.

## Approach

1. Add `mkdir -p ~/.local/share/bash-completion/completions` before the line in `docs/cli.md`.

## Prove

```sh
HOME=$(mktemp -d) sh -ec 'mkdir -p ~/.local/share/bash-completion/completions; podnet completions bash > ~/.local/share/bash-completion/completions/podnet'
```

## Done

2026-11-15, eeac027. `docs/cli.md` has `mkdir -p ~/.local/share/bash-completion/completions` before the line. The Prove
command exits 0 in a new HOME.

# T-363: net passwd-shim: the home of the entry is $HOME, not /state/home

**Source:** T-358 step 2, the documented command at docs/networks.md:338
(`eval "$(podnet net passwd-shim)"`, then `ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519 -N ''`), read on 2026-11-15.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet net passwd-shim` writes the entry `NAME:x:UID:GID:NAME:/state/home:/bin/sh` on each host. `/state/home` is the
home of one sandbox, not of the user. `ssh` and `ssh-keygen` find `~/.ssh` by the home of the entry, not by `$HOME`.
With the shim, `ssh` reads its keys and `known_hosts` from `/state/home/.ssh`, and `ssh-keygen -f ~/.ssh/id_ed25519`
does not make `~/.ssh` (it makes the directory only when the file is under the `.ssh` of the entry).

## Premise

- Measured on 2026-11-15: `HOME=/x/y podnet net passwd-shim --out DIR` writes `u0:x:0:0:u0:/state/home:/bin/sh`.
  `crates/podnet-vnet/src/tailscale_ssh.rs`, `LocalUser::new`, has the fixed home; `src/cli/cmd/net/shim.rs`,
  `write_user_shims`, takes it.
- Not measured: the documented block itself. This host has no `ssh-keygen` (`doc-commands.py --run` prints MISS for
  it). The `ssh-keygen` behaviour is from its source (`ssh-keygen.c`, the `.ssh` directory of `pw_dir`). CI has
  `ssh-keygen`: the step "The documented commands" of `ci.yml` runs the block.

## Approach

1. In `write_user_shims`, set the home of the entry to `$HOME` when it is an absolute path. The library default
   (`LocalUser::new`) stays.
2. `tests/cli/net.rs`, `passwd_shim_prints_the_preload`: the entry ends with `:$HOME:/bin/sh`.
3. `docs/networks.md`: say that the home of the entry is `$HOME`.

## Prove

```sh
cargo test --test cli net::passwd_shim_prints_the_preload
python3 -I scripts/doc-commands.py --run --only docs/networks.md
```

## Correction

2026-11-15, eeac027: the code and its test are done. `write_user_shims` takes `$HOME` when it is absolute;
`HOME=/x/y podnet net passwd-shim` writes `u0:x:0:0:u0:/x/y:/bin/sh`; `passwd_shim_prints_the_preload` passes. Open:
the documented block with `ssh-keygen` has not run (this host has no `ssh-keygen`). It runs in the step "The
documented commands" of the job `test` of `ci.yml`; set the entry `done` when that step passes.

## Done

2026-11-15, eeac027 (the fix). ci 43774698575 on f2d96fc ran the documented block `docs/networks.md:356` (`net passwd-shim`,
then `ssh-keygen`) in the step "The documented commands": `pass docs/networks.md:337`; `doc-commands: 4 passed, 0 failed,
0 not run`.

# T-364: live: two runs in a row failed at the quick tunnel; the rerun passed (GitHub #516)

**Source:** GitHub #516 ("live: two runs in a row failed").
**Category:** measurement
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Two runs of `live.yml` in a row failed. The issue asked why, and whether podnet has a defect there.

## Premise

- Read on 2026-11-15, from the row of GitHub #516 in `TODO/issues.md` (written when the issue closed): both failures were
  at the quick tunnel of Cloudflare's edge, and the rerun 43333498755 passed. The record kept no entry for it then; the
  rules of the issues (PodUtils/podcrates#192, the owner's ruling "Issues") ask for one.
- Not measured again here. A quick-tunnel step that does not depend on Cloudflare is T-316 (the stand-in in `ci`).

## Approach

1. Record the finding: an outage or a flake of the edge, with no change of podnet; the rerun passed.

## Prove

```sh
gh run view 43333498755 -R PodUtils/podnet --json conclusion
```

## Done

2026-11-15, no code commit (the record of the finding). The rerun 43333498755 of `live.yml` passed; the issue was closed
with that evidence.

# T-379: A unix listener of podnet fails with EXDEV in a Landlock domain

**Source:** T-358 step 1, podlab run no-tun (also root-unreadable, no-kvm, proc-masked, no-net-admin), 2026-11-15.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

In each lab of `pod-lab` that hides a path with Landlock, `podnet net proxy --listen unix:PATH` exits with `podnet:
binding unix:PATH: Cross-device link (os error 18)`. A Python server binds a unix socket in the same directory with no
error. Each listener of podnet that calls `podnet_netns::listen::bind_private` has the defect: `net proxy`, `net
switch`, `warp proxy`, `connect`, `route`, the router, and the control socket of `link`. Sandboxes of agents use
Landlock, so a user meets it.

## Premise

- Measured on 2026-11-15 with the release binary and `podlab` of podcrates `v0.56.0`, as uid 65534.
- `bind_private` (`crates/podnet-netns/src/listen.rs`, T-238) binds the socket in a new directory beside the path and
  moves it to the path with `renameat2`. A Landlock domain that does not handle the right `REFER` refuses each move to
  another directory with `EXDEV` (the Landlock documentation). The labs of `pod-lab` handle file rights, not `REFER`.

## Approach

1. When the move gives `EXDEV`, bind at the path with the mode 0600 from the bind on: `fchmod(0600)` on the socket
   before `bind(2)` (the mode of the new file is the mode of the socket less the umask), then `chmod 0600`. The file is
   never wider than 0600.
2. A unit test with a seam that makes the move give `EXDEV`.

## Prove

```sh
cargo test -p podnet-netns --lib listen::
sh scripts/lab-lane.sh PODLAB target/x86_64-unknown-linux-musl/release/podnet no-tun
```

## Done

2026-11-16, d788b51. `bind_private` falls back to `bind_with_mode` on `EXDEV`. The test
`bind_private_binds_at_the_path_when_the_move_is_refused` passes. The lane as uid 65534: `net-proxy-unix` passes in
root-unreadable, no-kvm, proc-masked, no-tun and no-net-admin (it failed in each before).

# T-380: link listen over the unix carrier fails with EXDEV in a Landlock domain (pod-link)

**Source:** T-358 step 1, podlab run no-tun (also root-unreadable, no-kvm, proc-masked, no-net-admin), 2026-11-15.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

In each lab of `pod-lab` that hides a path with Landlock, `podnet link listen PATH --allow FILE` exits with `podnet: the
unix carrier: bind PATH: Cross-device link (os error 18)`. The class `bind-denied` of PodUtils/podrunner#170 runs
podnet under Landlock, and the unix carrier is the way of `link` there.

## Premise

- Measured on 2026-11-15 with the release binary and `podlab` of podcrates `v0.56.0`, as uid 65534.
- The unix carrier is in the crate `pod-link` of podcrates (`PodUtils/podcrates:crates/pod-link/src/unix.rs`, `bind_private`, at
  `93bf133a`). It stages the socket in a new directory and moves it with `renameat2`, as podnet did before T-379. A
  Landlock domain that does not handle `REFER` refuses the move with `EXDEV`.

## Approach

1. podcrates: in `pod-link`, on `EXDEV` of the move, bind at the path with `fchmod(0600)` before `bind(2)`, as
   `podnet_netns::listen::bind_with_mode` (T-379).
2. podnet takes the tag; the lane passes `link-unix` in the Landlock profiles.

## Prove

```sh
sh scripts/lab-lane.sh PODLAB PODNET no-tun no-kvm
```

## Done

2026-11-16, commit 1f5088f (T-386) and the commit of this record. podcrates v0.59.0 fixes `pod-link` (their entry 742:
on `EXDEV` the unix carrier binds at the path). The lane with `podlab` v0.59.0 and the release musl binary of 1f5088f, as
uid 65534 (`sh scripts/lab-lane.sh .work/podlab-v0.59.0 PODNET`): `link-unix` passes in root-unreadable, no-kvm,
proc-masked, no-tun and no-net-admin (it failed in each with v0.56.0 and v0.58.0). The lane exits 0: 187 rows pass, 0
fail, 20 n.a.

# T-381: The netns backend of podnet pod needs an AF_INET datagram socket (pod-hold)

**Source:** T-358 step 1, podlab run udp-denied, 2026-11-15.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

In the lab `udp-denied` (seccomp refuses `socket(AF_INET or AF_INET6, SOCK_DGRAM)` with `EPERM`), `podnet pod
--backend netns -- /bin/true` exits 3 with `socket() for the interfaces: Operation not permitted`. The backend sends no
UDP: it opens the socket only for the ioctls of the interfaces. `--backend auto` goes to the seccomp backend, and its
reason is wrong: `unshare ok, but lo cannot be set up (no capability)`.

## Premise

- Measured on 2026-11-15 with the release binary and `podlab` of podcrates `v0.56.0`, as uid 65534. The backend passes
  in the other labs where a user namespace is allowed.
- `pod-hold` (`PodUtils/podcrates:crates/pod-hold/src/netns/tun.rs`, `control`, at `93bf133a`) opens `socket(AF_INET, SOCK_DGRAM)`.
  Measured here: `SIOCGIFFLAGS` on an `AF_UNIX` datagram socket works (the kernel sends each interface ioctl of a socket
  to `dev_ioctl`).
- The reason text is in `crates/podnet-netns/src/doctor.rs` (`PROBE_NO_LOOPBACK`).

## Approach

1. podcrates: `pod-hold` opens `socket(AF_UNIX, SOCK_DGRAM)` (or a netlink socket) for the interface ioctls, with
   `AF_INET` as the fallback.
2. podnet takes the tag. The reason of the probe names the call that failed, not "no capability".

## Prove

```sh
sh scripts/lab-lane.sh PODLAB PODNET udp-denied
```


## Correction

2026-11-16: podnet's part is done: the probe of `pod doctor` (`crates/podnet-netns/src/doctor/child.rs`) opens the
`AF_INET` datagram socket of the interface ioctls first, and a refused socket gives its own reason (`doctor.rs`,
`PROBE_NO_IOCTL_SOCKET`): "unshare ok, but the socket for the interface ioctls is refused: a filter refuses an AF_INET
datagram socket", not "no capability". Measured with the release binary as uid 65534: `podlab run udp-denied -- podnet
pod doctor` gives that reason for `unshare(NEWUSER|NEWNET)` and the veth wire; `podlab run no-kvm` still gives the netns
backend `ok`. The backend itself still needs the socket: the fix is in `pod-hold` (COORD-095, GitHub #592).

## Done

2026-11-16, commit 1f5088f (T-386) and the commit of this record. podcrates v0.59.0 fixes `pod-hold` (their entry 743:
`socket(AF_INET, SOCK_STREAM)` for the interface ioctls, else `SOCK_DGRAM`). The probe of `pod doctor` opens the same
sockets in the same order, and fails only when the two are refused ("a filter refuses an AF_INET socket (stream and
datagram)"). Measured with the release musl binary of 1f5088f and `podlab` v0.59.0 as uid 65534:

- `sh scripts/lab-lane.sh .work/podlab-v0.59.0 PODNET`: `pod-netns` passes in `udp-denied` (it failed with v0.56.0 and
  v0.58.0). The lane exits 0: 187 rows pass, 0 fail, 20 n.a.
- `podlab run udp-denied -- podnet pod --backend netns -- /bin/true` exits 0. `podnet pod doctor` there gives
  `unshare(NEWUSER|NEWNET)` ok (the veth wire) and `backend netns: ok`.
- `sh scripts/consumer-pod.sh PODNET PODLAB udp-denied`: `auto` now takes netns. `auto` and `netns` start the program,
  and it fails only on name resolution (T-383).

Not measured: a host that refuses both socket types (the reason text of the probe).

# T-382: podnet pod takes the netns backend where each TCP connect of the program is refused (Landlock)

**Source:** T-358 step 3, podnet pod in bind-denied, 2026-11-16.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

In the lab `bind-denied` of `pod-lab` (Landlock refuses each TCP bind and connect of `AF_INET` and `AF_INET6` with
`EACCES`: the host of PodUtils/podrunner#170), `podnet pod -x socks5://unix:PATH -- PROGRAM` with `--backend auto` takes
the netns backend, and the program fails with `Permission denied` at its first TCP connect. A Landlock domain holds in
each network namespace, so the program cannot reach the stack of the pod. `--backend seccomp` works there: the
supervisor answers the connect, and the kernel does not run it.

## Premise

- Measured on 2026-11-16 with the release musl binary and `podlab` of podcrates `v0.56.0`, as uid 65534:
  `sh scripts/consumer-pod.sh PODNET PODLAB bind-denied` gave `fail (exit 1)` for `auto` and `netns` with python3
  (`URLError: [Errno 13] Permission denied`) and curl (exit 7); `seccomp` passed.
- The choice of `auto` is in `crates/podnet-netns/src/lib.rs` (`netns_probe`). It probes `unshare` only.

## Approach

1. A probe: a TCP connect to `127.0.0.1:1` with `SOCK_NONBLOCK`. `EACCES` or `EPERM` means that each TCP connect of the
   program is refused (a refused port or a listener means that the connect is allowed).
2. `auto` takes the seccomp backend and names the reason. `--backend netns` exits 3 with the reason.

## Prove

```sh
cargo test --test pod_backend_line netns_is_refused_where_tcp_connect_is_refused
sh scripts/consumer-pod.sh PODNET PODLAB bind-denied
```

## Done

2026-11-16, the commit of this record. The test `netns_is_refused_where_tcp_connect_is_refused` of
`tests/pod_backend_line.rs` runs `podnet pod` under a Landlock domain that handles `BIND_TCP` and `CONNECT_TCP` (ABI 4;
it skips with no ABI 4). It failed on the code before the fix (`--backend netns` exited 0) and passes after it (6 of 6
tests of the file). `sh scripts/consumer-pod.sh` with the release binary in `bind-denied`: `auto` passes with python3
and curl (the seccomp backend, with the reason `auto takes netns first, but netns cannot run here (a TCP connect to
127.0.0.1 is refused: Permission denied (os error 13))`); `--backend netns` exits 3 with `the netns backend cannot carry
a TCP connect here`. Not done: `podnet pod doctor` still gives the netns backend `ok` there.

# T-383: podnet pod: no name resolves when the program cannot make a UDP socket (udp-denied)

**Source:** T-358 step 3, podnet pod in udp-denied, 2026-11-16.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

In the lab `udp-denied` of `pod-lab` (seccomp refuses `socket(AF_INET or AF_INET6, SOCK_DGRAM)` with `EPERM`), a program
in `podnet pod --backend seccomp` (and `auto`, which takes seccomp there, as T-381 refuses netns) cannot resolve a name:
python3 gives `gaierror: [Errno -3] Temporary failure in name resolution`, curl exits 6. The filter of the lab runs
before the filter of the pod, so the fake DNS of the pod never sees the query.

## Premise

- Measured on 2026-11-16 with the release musl binary and `podlab` of podcrates `v0.56.0`, as uid 65534
  (`sh scripts/consumer-pod.sh PODNET PODLAB udp-denied`).
- With an address in place of a name, the TCP connect of the same program goes through the pod to the SOCKS5 upstream.
- With `RES_OPTIONS=use-vc`, glibc asks over TCP: the pod sends the connect to `8.8.8.8:53` (the nameserver of
  `/etc/resolv.conf`) through the upstream, not to the fake DNS. The seccomp backend answers DNS over UDP only.
- Not measured: a musl program; the netns backend (T-381 refuses it in this lab).

## Approach

1. The seccomp backend answers a TCP connect to port 53 of a nameserver of the program from the fake DNS (DNS over
   TCP, RFC 7766 framing), as it answers a datagram.
2. When the probe finds the UDP socket refused, the pod adds `use-vc` to `RES_OPTIONS` of the program (as
   `with_single_request` adds `single-request`).

## Prove

```sh
sh scripts/consumer-pod.sh PODNET PODLAB udp-denied
```

## Correction

2026-11-16: since podcrates v0.59.0 (T-386) the netns backend runs in `udp-denied`, and `auto` takes it there.
`sh scripts/consumer-pod.sh PODNET PODLAB udp-denied` with the release musl binary of 1f5088f: `netns` and `auto` start
the program; python3 gives `gaierror: [Errno -3] Temporary failure in name resolution` and curl exits 6, as with
`seccomp`. The program itself cannot make the UDP socket of its DNS query in each backend.

## Done

2026-11-16, 55908f6. The fake DNS answers DNS over TCP (RFC 7766 framing, `crates/podnet-netns/src/dnstcp.rs`): the
seccomp backend answers each TCP connect to port 53 (`connect.rs`), and the netns backend answers port 53 of the gateway
`10.66.255.254` (`netns/backend.rs`). When the pod cannot make an `AF_INET` UDP socket (`EPERM`, `EACCES` or `ENOSYS`;
the seccomp supervisor probes before the start, the netns child probes in the namespace), it adds `use-vc` to
`RES_OPTIONS` of the program and prints one line on stderr. `netns-kernel` does not change: it needs a UDP socket and
exits 3 in `udp-denied`.

- The test `a_name_resolves_where_udp_sockets_are_refused` (`tests/pod_seccomp/udp_denied.rs`) runs `podnet pod` under a
  seccomp filter that refuses `socket(AF_INET or AF_INET6, SOCK_DGRAM)` with `EPERM`, with a glibc C program
  (`getaddrinfo` with `AF_UNSPEC`, then a connect). It failed on the code before the fix (seccomp: `GAI:-3`) and passes
  after it for `seccomp` and `netns`.
- `sh scripts/consumer-pod.sh` with the release musl binary of the fix, `podlab` of podcrates `v0.59.0`, as uid 65534,
  over the 34 profiles of `podlab list`: `udp-denied` passes for `auto`, `netns` and `seccomp` with python3 and curl
  (before: 6 rows `fail, known: T-383`). The pod rows: 148 pass before, 154 after; 42 n.a. before and after; the two
  failures that stay are `netns-kernel` in `getsockname-denied` (T-384). No other row changed. 10 labs do not hold here.
  `CONSUMER_PLANT=decoy` in `udp-denied`: the 6 pod rows fail, as planted.
- musl: its resolver makes a UDP socket for each query and reads no `use-vc` (the files res_msend.c and resolvconf.c
  of musl). A static musl program (Rust `ToSocketAddrs`) in `podlab run udp-denied` gives `Operation not
  permitted` with both backends; it resolves outside the lab. No fix of podnet can help it there; `docs/pods.md` says so.

# T-384: podnet pod --backend netns-kernel reads the ports of its listeners with getsockname (getsockname-denied)

**Source:** T-358 step 3, podnet pod in getsockname-denied, 2026-11-16.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

In the lab `getsockname-denied` of `pod-lab` (seccomp refuses `getsockname` with `EOPNOTSUPP`: a host of podrunner),
`podnet pod --backend netns-kernel` exits 3 with `local address: Not supported (os error 95)`. The child binds its TCP,
UDP and DNS listeners to port 0 in the new namespace and reads each port with `getsockname`; the parent reads the UDP
port again (`kernel_serve.rs`). The other backends pass in this lab.

## Premise

- Measured on 2026-11-16 with the release musl binary and `podlab` of podcrates `v0.56.0`, as uid 65534
  (`sh scripts/consumer-pod.sh PODNET PODLAB getsockname-denied`).
- The code is in `crates/podnet-netns/src/kernel.rs` (`child_setup`) and `kernel_serve.rs`. The namespace is new, so
  each port is free in it.

## Approach

1. Bind the listeners to fixed ports of the new namespace when `getsockname` is refused (or always), and send the
   ports to the parent with the descriptors.

## Prove

```sh
sh scripts/consumer-pod.sh PODNET PODLAB getsockname-denied
```

## Done

2026-11-16, the commit of this entry. The child binds its TCP, UDP and DNS listeners to the fixed ports 61001, 61002
and 61003 of 127.0.0.1 in the new namespace (`Ports::FIXED` of `crates/podnet-netns/src/nfrules.rs`): the namespace is
new, so each port is free, and the ports are above the ephemeral range (32768-60999). Neither the child nor the parent
calls `getsockname` now. `docs/pods.md` names the ports.

`sh scripts/consumer-pod.sh PODNET PODLAB getsockname-denied` (debug podnet, `podlab` v0.63.0, uid 65534): each of the 8
pod rows passes, `netns-kernel` with python and with curl (before: exit 3, `local address: Not supported`). `no-kvm`:
each row passes; `bind-denied`: as before. `cargo test -p podnet-netns` and `--test pod_netns_kernel` pass.

# T-390: vnet shim: fcntl reads the struct flock pointer of the lock commands as an int

**Source:** GitHub #599 (Nemo-010, outside the team; verified and labelled `confirmed` on 2026-11-16).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

GitHub #599: "`fcntl` reads a `struct flock *` as an `int`. `:1401-1409` lists the pointer-taking commands the shim
knows; `:1412-1419` forwards every other command through `int arg = va_arg(ap, int)`. `F_GETLK`, `F_SETLK` and
`F_SETLKW` take a `struct flock *` and are \"other\", so on x86-64 the pointer is truncated to 32 bits and the kernel
answers `EFAULT`. Qt takes a `QLockFile` on the profile, so the client never binds a listener." Measured with
`qbittorrent-nox` 5.1.0 under the shim.

## Premise

- Reproduced on 2026-11-16 at 5fbb2e3: a C program with `fcntl(fd, F_SETLK, &fl)` and `F_GETLK` gives rc 0 with no
  shim, and `-1 Bad address` for both under `LD_PRELOAD` of the shim of `podnet net shim --out DIR`.
- `F_OFD_GETLK`, `F_OFD_SETLK`, `F_OFD_SETLKW`, `F_GETOWN_EX` and `F_SETOWN_EX` also take a pointer.

## Approach

1. Move the lock commands (`F_GETLK`, `F_SETLK`, `F_SETLKW`, the `F_OFD_*` forms under `#ifdef`) and `F_GETOWN_EX`,
   `F_SETOWN_EX` into the arm that reads a pointer.
2. A test under the shim: a lock and its query on a file give rc 0; a planted shim with the old arm fails it.

## Prove

```sh
cargo test -p podnet-vnet
```

## Done

2026-11-16, the commit of this entry. `fcntl` of `crates/podnet-vnet/shim/podnet-vnet.c` reads a pointer for `F_GETLK`,
`F_SETLK`, `F_SETLKW`, `F_OFD_GETLK`, `F_OFD_SETLK`, `F_OFD_SETLKW` (under `#ifdef`), `F_GETOWN_EX` and `F_SETOWN_EX`.
`tests/shim_daemon.rs`, `a_record_lock_passes_its_struct_flock`: each lock command and `F_GETOWN_EX` gives rc 0 under the
shim and with no shim. The plant (the shim source of 5419c25) fails it with `Bad address` for each command.
`docs/virtual-network.md` 3.2 lists the commands.

# T-391: vnet shim: a bind to the wildcard address gets a name that no peer dials

**Source:** GitHub #599 (Nemo-010, outside the team; verified and labelled `confirmed` on 2026-11-16).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

GitHub #599: "A wildcard bind registers a name no peer can dial. `bind_one` passes the caller's address to
`to_abstract` verbatim, so `bind(0.0.0.0, 19101)` registers `\0podnet-vnet/4/0.0.0.0/19101`, while a peer dialling
`10.66.0.2` builds `\0podnet-vnet/4/10.66.0.2/19101`. Different names, connection refused. `INADDR_ANY` is what a
daemon binds when nothing told it otherwise, so this is most servers, not a corner." The proposal: `INADDR_ANY` and
`in6addr_any` become the local address of the shim (`PODNET_VNET_LOCAL_ADDR`) before the bind.

## Premise

- Reproduced on 2026-11-16 at 5fbb2e3: `python3 -m http.server 19101 --bind 0.0.0.0` under the shim, and curl under
  the shim to `http://10.66.0.2:19101/`: `000`. With `--bind 10.66.0.2`: `200`.
- `to_abstract` maps `127.0.0.0/8` and `::1` to the local address when `map_loopback` is on; it leaves `0.0.0.0` and
  `::`.
- `getsockname` after a wildcard bind: Linux gives the wildcard. Decide whether the shim gives the wildcard or the local
  address, and say it in `docs/virtual-network.md`.

## Approach

1. In `bind_one`, a wildcard address of either family becomes the local address (IPv4; for `AF_INET6` the mapped
   form), as for a loopback address.
2. A test under the shim: a server bound to `0.0.0.0` and to `::` answers a client that dials the local address and
   `127.0.0.1`; a planted shim with no rewrite fails it.

## Prove

```sh
cargo test -p podnet-vnet
```

## Done

2026-11-16, the commit of this entry. `bind_one` of the shim gives a bind to `0.0.0.0` the local address, and a bind to
`::` the mapped form of the local address (`wildcard_to_local`); the bind of port 0 goes through it too.
`tests/shim_daemon.rs`, `a_wildcard_bind_takes_the_local_address`: a server bound to `0.0.0.0` and to `::` answers a
client that dials `10.66.0.2`, and a server bound to `0.0.0.0` answers a client that dials `127.0.0.1` with
`PODNET_VNET_MAP_LOOPBACK=1`; the control binds `10.66.0.2`. The plant (the shim source of 5419c25) fails it.
`getsockname` gives the local address after a wildcard bind (`docs/virtual-network.md` 3.1).

# T-392: vnet shim: answer netlink route dumps with the virtual interfaces

**Source:** GitHub #599 (Nemo-010, outside the team; verified and labelled `confirmed` on 2026-11-16).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

GitHub #599: "Interface enumeration reports the host's interfaces. `socket()` virtualises `AF_INET` and `AF_INET6`
and forwards every other domain to the host, so `AF_NETLINK` dumps are answered by the real kernel. [...] libtorrent
2.0.11 imports neither symbol [`getifaddrs`, `if_nameindex`] and asks netlink directly — `RTM_GETLINK`, `RTM_GETADDR`,
`RTM_GETROUTE`. Under the shim the client listens on and advertises the host's addresses [...] inside the
networkspace." The proposal: answer `AF_NETLINK` (protocol `NETLINK_ROUTE`) in the process over a
`socketpair(AF_UNIX, SOCK_DGRAM)`: `RTM_GETLINK` gives `lo` and `vnet0`, `RTM_GETADDR` gives `127.0.0.1/8`, `::1/128`
and the local address `/24`, each other type `NLMSG_DONE`. "`getsockname` on a netlink socket writes `nl_pid =
getpid()`, and libtorrent passes that value as the expected `nlmsg_pid`, discarding any reply that carries another."

## Premise

- Read on 2026-11-16 at 5fbb2e3: `socket()` of `crates/podnet-vnet/shim/podnet-vnet.c` (`:1419`) gives each other domain
  to the host. Not measured here with libtorrent.
- `getifaddrs` of glibc reads netlink too: the answer also changes what `getifaddrs` gives a program.
- A program that needs the host's netlink (for example `ip`) under the shim: decide whether a variable turns the answer
  off.

## Approach

1. A fake `NETLINK_ROUTE` socket in the shim, with the dumps of the Problem, the `nl_pid` of `getpid()` in
   `getsockname` and in each reply, and `NLM_F_MULTI` then `NLMSG_DONE`.
2. Tests under the shim: `getifaddrs` gives `lo` and the local address only; `ip -br addr` (when there) shows the same;
   a planted shim that forwards netlink fails the test.

## Prove

```sh
cargo test -p podnet-vnet
```

## Done

2026-11-16, the commit of this entry. `crates/podnet-vnet/shim/podnet-vnet.c`: `socket(AF_NETLINK, *, NETLINK_ROUTE)`
is a `socketpair(AF_UNIX, SOCK_DGRAM)`; the shim answers each request at `send`, `sendto` and `sendmsg` (and a request of
`write` at the next receive) with `lo` and `vnet0`, the port id of `getsockname` in `nlmsg_pid`, `NLM_F_MULTI` and
`NLMSG_DONE`; `bind`, `connect`, `getpeername`, `recvfrom`, `recvmsg` and `SOL_NETLINK` options are answered for the
socket. `getifaddrs`, `if_nameindex`, `if_nametoindex` and `if_indextoname` give the same view (glibc's own `getifaddrs`
asks the kernel by internal calls, which a preload does not reach, so the shim interposes it). `PODNET_VNET_HOST_NETLINK=1`
gives the host's view again. `docs/virtual-network.md` 3.4.

`tests/shim_netlink.rs`: a dump in the way of libtorrent (`send`, `recvfrom`, the port id and the sequence checked) gives
exactly `lo` and `vnet0` with `10.66.0.7/24`, an empty route dump, and the same by `getifaddrs`, `if_nameindex` and
`if_nametoindex`; the control with `PODNET_VNET_HOST_NETLINK=1` (the host's kernel answers, as the stock shim does) fails
the check. iproute2 6.x under the shim: `ip -br addr` gives `lo` and `vnet0` only, `ip link set lo down` fails with
"Operation not permitted". `cargo test --test shim_netlink --test shim_daemon --test shim_direct`: 15 pass.

Not proven: libtorrent and qBittorrent were not run here; a program that reads a netlink socket with `read(2)` after
`write(2)` gets no answer (neither is interposed).

# T-395: live: the check of agent up fails when the resolver of the runner caches NXDOMAIN for the new name

**Source:** live 43881448907 on 7954152 (2026-11-16), failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The step "agent up and down" of `.github/workflows/live.yml` failed with `curl: (6) Could not resolve host`. podnet had
verified the URL of the quick tunnel through public DNS ("verified public URL ... dns="doh"") and said: "the system
resolver of this host does not resolve the new name yet; public DNS does. A visitor whose resolver cached NXDOMAIN gets
an error for some seconds more". The step then ran one `curl` through the resolver of the runner.

## Premise

- live passed on the same code three times on 2026-11-16 (b6c4ef7 twice, 62816a8). The fault is a race of the check,
  not of podnet: podnet names the case in its own output.
- The step of line 156 of `live.yml` already retries its curl (`--retry 5 --retry-all-errors`).

## Approach

1. The curl of the step retries for 60 s, also on a name that does not resolve (`--retry 20 --retry-all-errors
   --retry-delay 3`).

## Prove

```sh
python3 scripts/check-workflows.py
# live passes on the next push
```

## Done

2026-11-16, the commit of this entry. The curl of "agent up and down" retries for 60 s with `--retry-all-errors` (exit 6,
the name does not resolve, is retried). The next push runs live.

# T-397: live: two quick tunnels over WARP within a minute register two devices, and WARP answers HTTP 429

**Source:** live 43894127680 on defa106 (2026-11-16), failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The step "Quick tunnel with UDP and TCP 7844 dropped (staggered legs)" of `.github/workflows/live.yml` failed: "quic+warp:
registering with WARP: registering a WARP device: the WARP service limits new devices for each address (HTTP 429, error
1015); the service asks to wait 60 s". The step before it ("Quick tunnel, QUIC through WARP (TCP 443)") had registered a
device 30 s earlier.

## Premise

- A quick tunnel with no `--warp-identity` registers a new WARP device for each run (`PodUtils/podnet@7fe60e0:src/tunnel/warp_targets.rs:28`,
  `docs/tunnels.md`). podnet names the limit and the wait in its error. The fault is in the check, not in podnet.
- live passed on 3f203cc with the same steps: the limit holds at some times only.

## Approach

1. The two WARP steps of the quick tunnel give `--warp-identity "$RUNNER_TEMP/tunnel-warp.json"`: the first run
   registers the device, the second loads it.
2. The step "Remove the WARP device" also deletes that device.

## Prove

```sh
python3 scripts/check-workflows.py
# live passes on the next push
```

## Done

2026-11-16, the commit of this entry. The two steps share one device file, and the last step deletes the device. The
next push runs live.

# T-398: The release gate: a scenario for each experimental command

**Source:** T-358 step 4 (2026-11-16): 22 commands have no scenario of the release binary.
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The help of a command with no scenario says "Experimental:" (`src/cli/scenarios.txt`). On 2026-11-16 these commands
have none: `autostart run`, `bench`, `exit`, `expose`, `forward`, `link connect`, `link mesh`, `link close`, `link
reconnect`, `link rotate`, `net demo`, `net switch`, `net tailscale`, `net socksify`, `net passwd-shim`, `net
ts-shims`, `ssh-relay`, `wait`, `warp license`, `warp rotate`, `warp shell` and `wg serve`.

## Premise

- `python3 -I scripts/gate-scenarios.py --check` gives the list. Each command has tests with stand-ins in `cargo test`;
  no step runs its release binary as a user does.

## Approach

1. A lane case for each command that runs in a lab with no network service of another party: `link` over the unix
   carrier (`connect`, `close`, `reconnect`, `rotate`, `mesh`), `net switch` and `net socksify` with a local server,
   `forward` and `exit` with a local SOCKS5 peer, `bench` against a local server, `wait`, `net demo`, `net passwd-shim`
   and `net ts-shims`.
2. A step of `live.yml` for the commands that need a live service: `expose`, `ssh-relay`, `warp license`, `warp
   rotate`, `warp shell`, `wg serve`, `autostart run`.
3. Write `src/cli/scenarios.txt` again; the mark leaves the help of each command that gets a scenario.

## Prove

```sh
python3 -I scripts/gate-scenarios.py --check
grep -c ' |$' src/cli/scenarios.txt   # 0 when each command has a scenario
```

## Correction

2026-11-16, the first part. The lane case `local` also runs `net demo` (the line `PROOF:`), `net passwd-shim` (then
`whoami` under the preload gives the user `labuser`), `net ts-shims` (its `getent passwd labuser`) and `net socksify`
(the shim is built). A new case `link-control` runs `link listen --detach`, `wait`, `link connect --detach`, `link
status`, `forward -L unix:PATH=peer:FP/raw` (a client gets `echo:ping` from the echo server behind the listener),
`link reconnect` (link 1 to 2), `link rotate` (2 to 3) and `link close` of both instances. As uid 65534 in
`podlab` v0.63.0: both cases pass in `bind-denied`, `no-kvm` and `noexec-tmp`, `local` in `no-passwd-entry` and
`proc-masked`; `link-control` is n.a. in `listen-denied` (the lab refuses `listen(2)` of AF_UNIX). 12 commands stay
experimental: `autostart run`, `bench`, `exit`, `expose`, `link mesh`, `net switch`, `net tailscale`,
`ssh-relay`, `warp license`, `warp rotate`, `warp shell` and `wg serve`.

2026-11-16, the second part. `link-control` also runs `link mesh` on the control socket of the dialer (both ends with
`--announce`; the view has the edge to the peer). The network labs run two new cases: `net-bench` (`podnet bench
https://lab.test/ --duration 2 --json` through the proxy of the lab: about 1000 requests, 0 errors) and `net-exit` (a
client runs SOCKS5 CONNECT `lab.test:443`, then TLS and `GET /` over the pipe of `podnet exit --stdio`: `HTTP/1.1 200
OK`). As uid 65534 with `podlab` v0.63.0, both pass in `proxy-only`, `proxy-tls-reterminate`, `proxy-per-name` and
`ws-upgrade-refused`, and are n.a. in the labs with no proxy. A new case `net-switch` runs `net switch` on a unix socket and two stacks
(`net proxy --switch`): a SOCKS5 client of stack B connects `10.66.0.2:8080`, which stack A forwards to the echo server
(`echo:ping`). It passes in `bind-denied`, `no-kvm` and `udp-denied`, and is n.a. in `listen-denied`. 8 commands stay
experimental, each with a live service: `autostart run`, `expose`, `net tailscale`, `ssh-relay`, `warp license`, `warp
rotate`, `warp shell` and `wg serve`.

2026-11-16, the third part (not run yet: the next `live`). `live.yml`: the step "WARP API through its second host" runs
`warp rotate` on its own device, then a DNS query through `warp nc -u` with the new key; the step "warp up, env, exec,
--persist and down" runs `warp shell` with `SHELL=/bin/sh` and a `curl` of the trace on stdin (`warp=on`, and no line
"with no pod"). In the lane case `net-switch`, stack A is now `podnet expose unix:ECHO --via switch --as
10.66.0.2:8080 -- --switch ...` (it passes in `bind-denied` and `udp-denied`). 5 commands stay experimental: `autostart
run`, `net tailscale` (a `tailscaled`), `ssh-relay` (an SSH relay of another party), `warp license` (a WARP+ key) and `wg
serve` (WireGuard keys for a client; a key in the repository would need an exception of the secret scan).

2026-11-16, the fourth part. A new lane case `wg-serve` makes the keys of the server and of the client for each run (X25519
of RFC 7748 in Python: no key in the repository), runs `podnet wg serve` on UDP with `--egress socks5h+unix:` to a SOCKS5
stub that echoes, and `podnet forward --network wg:CLIENT.conf -L unix:PATH=192.0.2.10:7`: a client of PATH gets
`echo:ping`. The public keys agree with the module `cryptography`. As uid 65534: pass in `bind-denied` and `no-kvm`, n.a. in
`udp-denied` and `no-loopback` (no UDP on the loopback). The live step "Autostart with a systemd user unit" checks that
the unit runs `podnet autostart run` (`VIA` of `scripts/gate-scenarios.py`: a command that a step runs through another
program, with the text that the step must hold). 3 commands stay experimental: `net tailscale` (a `tailscaled` in a
tailnet), `ssh-relay` (an SSH relay of another party) and `warp license` (a WARP+ key).

2026-11-17, the fifth part. A new live step "A unix socket through a public SSH relay"
(`scripts/live-ssh-relay.sh`): an HTTP origin listens on a unix socket, `podnet ssh-relay --service S --json` publishes
it, and curl must get the body of the origin through the URL. The script tries `localhost-run`, `pinggy`, `serveo` and
`tunnl-gg` in order and passes on the first that serves the origin (the relays are services of other parties). The
step "WARP API through its second host" runs `warp license -` with a key that is not a WARP+ key on stdin: the API
refuses it, exit 64 (exit 69 with HTTP 429 is a limit of the API). The proxy of a cloud session refuses SSH, so the
step is not run here. 1 command stays experimental: `net tailscale` (it needs a `tailscaled` in a tailnet and an auth
key; no secret of the repository gives one).
Live 44005215136 on `73b3e28` passed both steps: `pinggy: the origin answered through the relay (try 1)`, and `warp
license` gave exit 64 (`HTTP 400`, code 1046, "Invalid license"). localhost.run refused the remote forward: its facts in
`pod-services` have no `remote-port` (80); COORD-100 (#607) asks podcrates for the fact.

2026-11-26, the fourth part, the commit of this entry. Commands added since 2026-11-16 had no scenario. A new lane case
`local-files` runs the commands of local files only: `host probe`, `rules check`, `init generate`, `init check`, `ci
check`, `offline spool add`, `list`, `check` and `take`, `offline bundle build` (a bundle and a delta), `verify` and
`apply`, `offline pair offer` and `accept`, and `forge clone` of a local seed. Each step checks the exit 0 and its words.
As uid 65534 with `podlab` `v0.84.0` it passes in `bind-denied`, `no-loopback`, `proc-masked`, `listen-denied` and
`nonroot-nocaps`. `scripts/gate-scenarios.py` reads a third level of a command (`offline spool add`). 15 commands leave
the experimental mark. Still experimental: `cover`, `harness`, `link p2p`, `net tailscale` (T-114), `offline discover
announce` and `listen`, `offline capture merge`, `offline replica merge`, `init enroll`, `stat`, and the `sidecar`
commands (feature `sidecar`, not in a release binary).

2026-11-26, the sixth part. The lane case `local-files` adds `offline replica merge` (two logs of two actors that write
one key: one conflict, one winner, the same bytes in each order) and `offline capture merge` (a secret of a
`--secret-file` is redacted, and is not in the merged file). A new lane case `local-udp` runs `offline discover
announce` to a unicast `offline discover listen` on the loopback (the listener names the peer), and `stat` to a `stat
--serve` peer on the loopback (`received 20`). As uid 65534 with `podlab` `v0.84.0`: both pass in `bind-denied`,
`proc-masked`, `listen-denied` and `nonroot-nocaps`; `local-udp` is n.a. in `no-loopback` and `udp-denied`. 5 commands
leave the experimental mark. Still experimental: `cover`, `harness`, `link p2p`, `net tailscale` (T-114), `init
enroll`, and the `sidecar` commands.
2026-11-26, the seventh part. `local-files` adds `init enroll --offline` (the state directory with its key, and
`"registered": false`) and `harness` over `tests/data/harness` (the lane copies the fixtures and the cover script of
`tests/data/cover` beside the cases). A new lane case `local-tcp` runs `link p2p` on the direct path to an echo peer on
the loopback (`--check --json` names `"path":"direct"`, and the stream carries `echo:ping`), and `cover` with a P-256
certificate of `openssl` to a TLS client with no switch key (`HTTP/1.1 200` and the page). The lane found three
defects: T-590, T-591 and T-592. As uid 65534 with `podlab` `v0.84.0`: `local-files` passes in each of the six
profiles; `local-tcp` passes in `proc-masked`, `nonroot-nocaps` and `udp-denied`, and is n.a. in `bind-denied`,
`no-loopback` and `listen-denied`. 4 commands leave the experimental mark. Still experimental: `net tailscale` (T-114)
and the `sidecar` commands (feature `sidecar`, not in a release binary).

## Done

2026-11-26, the commit of this entry. Each command of a release binary but `net tailscale` has a scenario
(`python3 scripts/gate-scenarios.py --check`: 0 faults). The scenario of `net tailscale` is the work of T-114 (blocked
on the shared headscale harness, COORD-130 #760): its Prove runs the command. The `sidecar` commands are not in a release
binary (feature `sidecar`).

# T-399: live: the WebSocket client of a --url origin dials a name that has no IPv4 answer yet

**Source:** live 43898114234 on c591bab (2026-11-16), failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The step "WebSocket through a --url origin" of `.github/workflows/live.yml` failed on the QUIC leg: the tunnel registered
(`the leg won the race leg=quic`), and each of 30 tries of the Python client failed with `OSError: [Errno 101] Network
is unreachable` at the connect. The runner has no IPv6 route.

## Premise

- `scripts/live-ws-url.sh` dials the new quick-tunnel name at once, with the system resolver, and does not wait for the
  name as `scripts/live-share.sh` and `scripts/live-throughput.sh` do (`scripts/live-wait-name.sh`).
- `scripts/live-wait-name.sh` took `getent hosts`, which an IPv6 answer alone satisfies.
- The same step passed on defa106 and on 3f203cc. The fault is in the check, not in podnet.

## Approach

1. `scripts/live-wait-name.sh` waits for an IPv4 answer (`getent ahostsv4`), else the DoH answer of type A in
   `/etc/hosts`.
2. `scripts/live-ws-url.sh` calls it before the client.

## Prove

```sh
shellcheck -S warning scripts/live-wait-name.sh scripts/live-ws-url.sh
# live passes on the next push
```

## Done

2026-11-16, the commit of this entry. Both scripts are changed as in the Approach. The next push runs live.

# T-400: Publish `v0.4.0-beta.14`: the gate, the dry run, the publish and the release proofs

**Source:** `CLAUDE.md`, Releases (2026-11-09) and the release gate (T-358, 2026-11-15): cut the next prerelease when a
batch of fixes is green and the gate passes. The batch since `v0.4.0-beta.13`: T-356, T-357, T-379 to T-384, T-390 to
T-392, T-396, the takes of podcrates `v0.52.0` to `v0.63.0`, the installer core (T-360), `pod-manual` (T-359) and the gate
(T-358, T-398).
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.13` has none of this work. It is the first release under the gate: `release.yml` runs `gate.yml`, and the
notes list each row of the lane and the experimental commands.

## Premise

- The steps are `docs/maintenance.md`, "Release". The session dispatches `release.yml` with the MCP tool
  `actions_run_trigger`.

## Approach

1. Set the version `0.4.0-beta.14` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`), write the llms files again.
2. Write `docs/releases/v0.4.0-beta.14.md`, with `## Breaking`.
3. Push; wait until `ci` and `live` pass on that commit.
4. Dispatch the dry run of `release.yml`; then `publish=v0.4.0-beta.14`; then check `release-proofs.yml`.

## Prove

```sh
gh release view v0.4.0-beta.14 -R PodUtils/podnet   # the files, the notes with the gate and the scenarios
```

## Done

2026-11-17, the tag `v0.4.0-beta.14` at `77ed861`. The dry run of `release.yml` 43991044357 passed (with the gate). The
release run 43995458679 published the release, `publish.yml` 43995461740 passed, and `release-proofs.yml` 44002218263
passed on `17fa8e6`.

# T-401: --detach and autostart stop on a host with no /proc: podnet cannot find its own file

**Source:** gate 43902956923 on c92438d (2026-11-16): the lane case `link-control` failed in the lab `no-proc`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

In the lab `no-proc` of `pod-lab` (no `/proc` is mounted), `podnet link listen --detach` exits with "podnet: --detach:
podnet could not run itself again: " (no path was tried). After a first fix of the parent, the supervisor stopped with
"supervisor: no path of this binary".

## Premise

- `binaries()` of `crates/podnet-core/src/detach.rs` tries `/proc/self/exe`, then `std::env::current_exe`, which reads
  the same link. With no `/proc`, the list is empty.
- The child of `--detach` runs with `argv[0]` "podnet" (`arg0` of `spawn.rs`), so it cannot find its file from `argv[0]`.
- `autostart add`, `autostart run` and the commands of `warp` that run the binary again use `current_exe` too.

## Approach

1. A module `self_exe` of `podnet-core`: `current()` is `current_exe`, else the file of `PODNET_SELF_EXE`, else the file
   of `argv[0]` (a path, or a name in `PATH`).
2. `--detach` adds that file to its list, and gives the child `PODNET_SELF_EXE`.
3. `autostart`, the CA bundle beside the binary, `net shim` (the shim beside the binary) and the commands of `src/cli/cmd`
   use it. `src/warp/` is frozen: it keeps `current_exe`.

## Prove

```sh
cargo test -p podnet-core self_exe
sh scripts/lab-lane.sh PODLAB PODNET no-proc   # link-control passes
```

## Done

2026-11-16, the commit of this entry. `crates/podnet-core/src/self_exe.rs` (with a test of the forms of `argv[0]`);
`--detach` and its supervisor find the binary with no `/proc`. As uid 65534 in the lab `no-proc` of `podlab` v0.63.0:
`link-control` passes (before: fail), and `local` passes. `PODNET_SELF_EXE` is in the manual.

# T-402: a quick tunnel over quic+warp does not recover when its MASQUE session closes

**Source:** the lane row `way-out` (`podlab run --way-out proxy-only`), 2026-11-16, this container.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`podnet demo --exit-on-verify` in the lab `proxy-only` with `--way-out`: the direct MASQUE path failed ("TLS handshake
with the MASQUE front door 162.159.198.2:443: tls handshake eof"), the relay path worked, and the leg `quic+warp`
registered both connections. Three seconds later `pod_warp::session` logged "the MASQUE HTTP/2 connection closed". Then
for 180 s the public URL gave HTTP 502 and 530, and podnet logged no reconnect: exit 69. A second run passed.

## Premise

- The reactor of `src/tunnel.rs` sees a lost connection only when `accept_bi` of the QUIC connection fails. The QUIC idle
  timeout is 30 s (`PodUtils/podnet@7fe60e0:src/cloudflare/quic.rs`). No loss was logged in 180 s: the timers of the QUIC connection seem not to
  run after the WARP stack under its endpoint closed.
- `WarpLink::endpoint` opens a new session when the reactor asks for a reconnect; the reactor never asked.
- `src/tunnel.rs` and `src/warp/` are frozen: a fix goes to podcrates as a COORD issue too.
- The container here has a proxy that re-signs TLS (the lane used `LANE_CA_FILE`); not measured on a GitHub runner yet.

## Approach

1. Reproduce: close the MASQUE session of a running quick tunnel (the relay path) and watch the reactor.
2. The reactor also watches the WARP session of its endpoint: a closed session is a lost connection, so the reconnect
   opens a new session at once.
3. A test with the stand-in WARP edge of `pod-testkit`: close the session, the tunnel reconnects in a few seconds.

## Prove

```sh
sh scripts/lab-lane.sh PODLAB PODNET way-out
```

## Done

2026-11-16, the commit of this entry. `EndpointSource::closed` (now in `PodUtils/podnet@7fe60e0:src/tunnel/warp_targets.rs`, with `WarpLink`)
reads the WARP session of a `quic+warp` leg each 2 s; the reactor of `src/tunnel.rs` takes a closed session as a lost
connection ("the WARP session of the edge leg closed; a reconnect follows"), and the reconnect opens a new session. The
move makes `src/tunnel.rs` smaller (1370 to 1322 lines; `scripts/size-allow.txt`).

Measured here: `podnet tunnel --hello-world --protocol quic --transport warp` (debug build, MASQUE through the relay),
then `ss -K` of each TCP connection of the process: the leg logged the closed session at 16:35:25 and "reconnected
conn_index=0" at 16:35:28, and the URL answered HTTP 200. The lane row `way-out` passed twice; it is now a default row of
the lane (the gate). Not proven: a test with stand-ins (the QUIC leg has no stand-in, T-316).

# T-406: live: the relay keeps a lost socket of a node for minutes after a reset (409), and SSH through it hangs

**Source:** live 43912914825 on ce9456d (2026-11-16), the step "SSH through the reverse relay", failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

The node connected to the relay (17:12:56). Two seconds later the link ended with "Connection reset by peer (os error
104)" and no Close. From then on, each new connect of the node got HTTP 409 ("the relay still holds the node's lost
socket"), for the 150 s of the step. The SSH client of the operator reached the relay and stopped in the key exchange
(`expecting SSH2_MSG_KEX_ECDH_REPLY`): the relay sent it to the lost socket.

## Premise

- The node is `pod_relay::reverse::node` of podcrates (taken crate): it retries a 409 for `rejoin` (600 s) after a lost
  socket, as designed. podnet does not change it.
- The relay server (the service of the relay pool) keeps the socket of a node until it notices that it is dead. The
  same step passed on 725214f and earlier on this day.
- live 43918102449 on 6369056 (18:00 UTC): the step "200 MiB each way through the reverse relay with --resume" failed.
  Each lost leg ended with `NeverReady { code: Some(1013), reason: "node open timeout" }` from the relay, and each new
  leg "ended before the far end greeted", for 600 s. The relay did not reach the node in that time.

## Approach

1. Ask podcrates (COORD): the relay server drops a node socket after a reset or a failed write at once, and the node
   asks for a new pair after a time of 409 shorter than the step of an operator.
2. Until then, the step of `live.yml` gives the error with the 409 lines (as now).

## Prove

```sh
# live passes the step "SSH through the reverse relay" each day for a week
```

## Blocker

The relay server and `pod-relay` of PodUtils/podcrates. Ask 1 waits for the relay role of podssh.

## Correction

2026-11-17: ask 2 is done in `pod-relay` v0.65.0 (their entry 805, `lost_hold`, 60 s); podnet takes it in T-410. Ask 1
(the relay drops a dead socket at once) is for the relay role of podssh (PodUtils/podcrates#214).
2026-11-17, more evidence: `live` 44125425938 (3654383) and 44172865442 (27e3b7b) failed in the resume step with
`1013 node open timeout` for each new leg for 600 s after one reset; `live` 44147054834 (76f7031) had a hung SSH
session through the node (T-478). About one run of six fails in this class. The state line of COORD-098 (#604) asks
podcrates who owns ask 1 now and when.
2026-11-20: `live` 44733644822 (schedule, `3f9a8e6`) failed in the step "Throughput through WARP, 1 to 16 MiB" at
the 16 MiB GET of `connect`: WARP's MASQUE door showed a certificate of Let's Encrypt in place of the pinned key, so the
tunnel took the public relay for MASQUE; the MASQUE HTTP/2 connection through the relay closed with no `close_notify`
10 s later. The edge connection reconnected (T-402), and the stream in flight was lost (`curl: (18) transfer closed`).
The same code passed `live` 44713609259 an hour before. A fault of the public relay and of WARP's door, not of podnet.
2026-11-23: `live` 45162920304 (`91a53fb`) failed in "warp scan" of `scripts/live-batch2.sh`: in two scans 10 s apart,
162.159.198.1:443 and 162.159.198.2:443 showed a certificate of Let's Encrypt (CN=YE1) in place of the pinned MASQUE key.
The steps before it opened WARP sessions to 162.159.198.2:443 some seconds earlier. The step passed on `af42f0d`
(`live` 45157147093), and `91a53fb` changed no code of WARP. A fault of WARP's door, not of podnet.
2026-11-23: `live` 45215638823 (`9d34eb7`; GitHub #688, the second failed run in a row after 45186065944, T-488) failed in
"Throughput through WARP, 1 to 16 MiB": the door 162.159.198.2:443 showed a certificate of Let's Encrypt; the MASQUE
session took the public relay; its HTTP/2 connection closed with no `close_notify` 18 s later, and the WebSocket of
`podnet connect` in flight was lost. The same class as the run of 2026-11-20. A fault of WARP's door and of the relay.
2026-11-25: `live` 45517951377 (`a6a6ccb`) failed in "warp scan" of `scripts/live-batch2.sh`: both doors showed a
certificate of Let's Encrypt (CN=YE1) in two scans 10 s apart, as on 2026-11-23. `live` 45518282050 (`1bf4161`)
failed in "WARP proxy" with resets of the SOCKS5 handshake: T-546 (the proxy closes a client with no session).


# T-407: podnet connect resets a local connection when one WebSocket dial fails

**Source:** live 43912914825 on ce9456d, attempt 2 (2026-11-16): the step "Throughput of the HTTP path and of a --forward
WebSocket" failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

"latency quic+direct connect: 1 of 20 requests FAILED": one `curl` through `podnet connect` got `(56) Recv failure:
Connection reset by peer`. The step printed the log of the tunnel only, not the log of `podnet connect`.

## Premise

- `ws::connect_bridge` (`src/ws.rs`) dials the WebSocket of the forward once for each local connection; a failed dial
  drops the local stream (a reset at the client). No byte of the local stream went out before the dial, so a new dial
  loses nothing.

## Approach

1. Dial again after a failure that can pass (a reset, a time limit, HTTP 5xx), 3 tries with a backoff of 250 ms and 500 ms.
   A URL fault and HTTP 4xx are final.
2. `scripts/live-throughput.sh` prints the log of `podnet connect` with the log of the tunnel at a failure.

## Prove

```sh
cargo test --lib ws::tests
# live passes the step on the next push
```

## Done

2026-11-16, the commit of this entry. `dial_again` of `src/ws.rs`, with one warning line for each new try. The test
`a_refused_upgrade_is_tried_again`: a server that answers the first upgrade with HTTP 503 and accepts the second gives a
session; a server that answers HTTP 403 gets one connection and the error names 403.

# T-408: a detached tunnel stops at 60 s while the check of its new public name may take 180 s

**Source:** live 43926742960 on c448b49 (2026-11-16): the step "File server through a quick tunnel" failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet serve DIR --detach` registered its tunnel at once, then waited for the check of the public URL: "this host does
not resolve the new name yet", and "Cloudflare and Google say that ... does not exist" for each try. At 60 s: "--detach:
files was not ready in 60 s; it is stopped" (exit 124). The check of a new name allows 180 s
(`VERIFY_BUDGET` of `PodUtils/podnet@7fe60e0:src/tunnel/verify.rs`).

## Premise

- The default of `--ready-timeout` was 60 s (`src/cli/opts.rs`), shorter than the time that the command itself gives
  its check. In the foreground the same command waits up to 180 s and works.
- The step wraps the start in `timeout 120`.

## Approach

1. The default of `--ready-timeout` is 240 s: the 180 s of the check, and the provision of the tunnel.
2. The steps of `live.yml` that start a detached tunnel wrap it in `timeout 300`.

## Prove

```sh
cargo test --test cli
# live passes the step on the next push
```

## Done

2026-11-16, the commit of this entry. `--ready-timeout` defaults to 240 s (`docs/background.md`, the help); a command
that fails still ends at once with its own exit code. `live.yml`: the file server and git serve steps use `timeout 300`.
The notes of `v0.4.0-beta.14` name it.

# T-411: live: speed.cloudflare.com answers HTTP 429 to the shared address of WARP, and the rate check fails

**Source:** live 43983078556 on 7d8edf2 (2026-11-17), the step "WARP proxy", failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Each of the four downloads of 20 MB through `podnet warp proxy` gave "1 429 (bytes, HTTP status)". The step needs 1 MB/s
from the best default run, so it failed. Each step before it through WARP passed (`warp=on`, the DNS forwarder, 30 of 30
UDP answers).

## Premise

- The 429 came through the proxy: the path through WARP works; the server of the measure limits the address of the
  WARP exit, which many users share.
- The best default rate also took the rate of a 429 answer (a few bytes each second).

## Approach

1. Each download tries again after a 429 (3 tries, waits of 10 s and 20 s).
2. Only a 200 counts for the best default rate. When no default run got a 200 and the server answered 429, the step
   gives a warning ("the rate is not measured") in place of the threshold.
3. The time limit of the step is 14 minutes.

## Prove

```sh
python3 scripts/check-workflows.py
# live passes the step on the next push
```

## Done

2026-11-17, the commit of this entry. `.github/workflows/live.yml`, the step "WARP proxy", as in the Approach.

# T-413: live: warp scan gets no answer from a front door once, and the step fails

**Source:** live 44002295224 on 060147b (2026-11-17), the step of `scripts/live-batch2.sh`, failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet warp scan --candidate 162.159.198.1 --json` gave "no front door answered": the doors 162.159.198.1:443,
162.159.198.2:443 and [2606:4700:103::2]:443 failed. In the same second, a WARP session to 162.159.198.2:443 was open,
and `warp exit` through it passed. The script printed no error of a door.

## Premise

- The same step passed on 17fa8e6 (live 43996999160). The door works: a single try of 3000 ms missed.
- The scan is code of `pod-warp`; the check is podnet's script.

## Approach

1. The script prints the error of each failed door.
2. When no door answers, the script scans once more after 10 s with `--timeout-ms 10000`; then the step fails.

## Prove

```sh
shellcheck -S warning scripts/live-batch2.sh
# live passes the step on the next push
```

## Done

2026-11-17, the commit of this entry. `scripts/live-batch2.sh`, the step "warp scan", as in the Approach.
Live 44014496432 on 404440d passed the step: 162.159.198.2:443 answered (TLS 16 ms); 162.159.198.1 showed a certificate of
Let's Encrypt, not the pinned MASQUE key (it is an extra candidate of the script, not a door of `pod-services`).

# T-414: live: after a kill of its MASQUE connection, the WARP proxy opens no new session in 60 s

**Source:** live 44007046520 on 2bd37f6 (2026-11-17), the step "WARP proxy", failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** partial

## Problem

The step kills the TCP connection of the MASQUE session (`ss -K dst 162.159.198.0/24 dport = 443`) and waits 60 s for the
log line "a new WARP session is open". In live 44007046520 the line did not come, and the log of the proxy had no line
after its start. Before the kill, speed.cloudflare.com answered 429 to each download (T-411), so the session had no
traffic for about 4 minutes. In live 44005215136 on 73b3e28 (downloads with 200), the new session opened 0.4 s after the
kill.

## Premise

- `ss -K` closed the connection: the fallback with the TCP reset rule did not run (the socket was not established
  after the kill).
- The proxy is code of `pod-warp` (podcrates); the check is podnet's step.
- One run is not enough to find the cause: an idle session that the service closed before the kill, a session on
  another transport, or a reader that does not see the abort.

## Approach

1. The step prints the TCP sockets to the front doors before and after the kill.
2. When no new session comes in 60 s, a client request goes through the proxy, the step gives a warning (T-414), and
   waits 60 s more; then it fails with the log of the proxy.
3. When the warning shows again, write the evidence (the sockets, the log) as a COORD issue to podcrates (`pod-warp`).

## Prove

```sh
python3 scripts/check-workflows.py
# live: the step passes; a warning "T-414" names a recovery that needed a client
```

## Correction

2026-11-17, steps 1 and 2 in `.github/workflows/live.yml`, the step "WARP proxy". Step 3 waits for the next warning.

2026-11-17, a second cause, in commit 5c1eb22: `live` 44105534873 (2d66915) failed with `no new WARP session after 120
s`, and `curl` to 127.0.0.1:40000 got `Couldn't connect`. The proxy had ended at its `--run-for 150`: three downloads
got HTTP 429 and each waited 60 s, so the kill came after the end. The log of the proxy said nothing of the end. Now the
proxy runs for 600 s, the step stops it at its end, and a planned end of `--run-for` logs `--run-for N s ended: the
command stops` (checked with `podnet serve` and `podnet warp proxy` and `--run-for 2`).

2026-11-18, a third form: `live` 44286677011 attempt 1 (`60db06b`), the step "The commands of the second batch" (`scripts/live-batch2.sh`, the
upload of 16 MiB with `--cc cubic`): the public endpoint reset one upload (`curl: (97) Recv failure`); then the
`warp proxy` ended with `the MASQUE session is open, and a DNS query to 1.1.1.1:53 through it got no answer`, so the
tries 2 and 3 got `Couldn't connect`. The proxy ends on a failed health check of its session, where it could open a new
session. The code is `pod-warp` (frozen); the evidence goes to podcrates with step 3.
2026-11-24: `live` 45322152685 on `cfbc215` (the release commit of `v0.4.0-beta.18`), attempt 2, failed in "The commands of
the second batch": the upload of 16 MiB with the congestion controller `cubic` got a reset; then each connect to the
WARP proxy at 127.0.0.1:40012 was refused; podnet's last line: "the MASQUE session is open, and a DNS query to
1.1.1.1:53 through it got no answer (1.1.1.1:53: no answer in 5000 ms)". The proxy ended on the failed health check.
Attempt 1 of the same run failed at the WARP API (HTTP 429, error 1015, T-397). The same code passed `live`
45317818764 on `1634d0a`.

# T-415: live: the first request through the WARP proxy gets a reset 5 s after the start

**Source:** live 44019959524 on ba6cce4 (2026-11-17), the step "WARP proxy", failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** partial

## Problem

The step starts `podnet warp proxy --listen tcp://127.0.0.1:40000 --dns 127.0.0.1:15353`, waits until the port
listens, and runs `curl -x socks5h://127.0.0.1:40000 https://www.cloudflare.com/cdn-cgi/trace`. curl gave `(97) Recv
failure: Connection reset by peer` 5 s after the start, and the step printed no line of the log of the proxy. The same
step passed on 404440d (live 44014496432).

## Premise

- Exit 97 is a failure of the proxy handshake: the proxy closed the SOCKS5 connection before its answer.
- The proxy is code of `pod-warp` (podcrates); the step is podnet's.
- One run gives no cause: the log of the proxy was not printed.

## Approach

1. When the first request fails, the step prints the last 30 lines of the log of the proxy, gives a warning (T-415),
   and tries once more after 5 s; then a failure ends the step.
2. When the warning shows, write the evidence (the log) as a COORD issue to podcrates (`pod-warp`).

## Prove

```sh
python3 scripts/check-workflows.py
# live: the step passes; a warning "T-415" gives the log of the failed first request
```

## Correction

2026-11-17, step 1 in `.github/workflows/live.yml`, the step "WARP proxy". Step 2 waits for the next warning.

# T-418: ssh-relay asks for a remote forward on 0.0.0.0, and localhost.run reads it as a custom domain

**Source:** live 44025607469 on 7fe60e0 (2026-11-17), the step "A unix socket through a public SSH relay" (T-398).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

With the fact `remote-port` = 80 of podcrates v0.67.0 (COORD-100, #607), localhost.run took the forward, but its banner
said: "SSH key is required for custom domains but no SSH key was sent", and it published no URL. The step then passed
on pinggy. `podnet ssh-relay` asked for the remote forward with the bind address `0.0.0.0`
(`crates/podnet-serve/src/relay.rs`).

## Premise

- OpenSSH sends the bind address `localhost` for `ssh -R PORT:HOST:HOSTPORT` with no bind address. The documents of
  localhost.run (`ssh -R 80:localhost:8080 nokey@localhost.run`) and of pinggy (`ssh -R0:localhost:8000 a.pinggy.io`)
  use that form.
- localhost.run reads another bind address as the custom domain of the tunnel.

## Approach

1. The remote forward asks for the bind address `localhost` (`FORWARD_BIND`).
2. The fake relay of the tests refuses another bind address, as localhost.run does.

## Prove

```sh
cargo test -p podnet-serve --lib relay::
# live: the step "A unix socket through a public SSH relay" passes on localhost-run
```

## Done

2026-11-17, the commit of this entry. With the fake that refuses another address and the old `0.0.0.0`,
`the_supervisor_reconnects_and_announces_the_new_url` and `the_supervisor_sees_a_connection_that_the_relay_ends` fail;
with `localhost`, the 37 tests of `relay::` pass. Live 44031540675 on 3d85179: `localhost-run: the origin answered
through the relay (try 1)`.

# T-419: Publish `v0.4.0-beta.15`: the gate, the dry run, the publish and the release proofs

**Source:** `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease when a batch of fixes is green. The batch since
`v0.4.0-beta.14`: T-403, T-404, T-405, T-410, T-417, T-418, the WARP half of T-242, the takes of podcrates `v0.64.0` to
`v0.67.0` (T-409, T-410, T-412, T-416) and the live steps of T-398.
**Category:** release
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The fixes and features of the batch are on `main` (ci and live passed on `3248174`), and no release has them.

## Premise

- The steps are `docs/maintenance.md`, "Release". The session dispatches `release.yml` with the MCP tool
  `actions_run_trigger`.

## Approach

1. Set the version `0.4.0-beta.15` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`), write the llms files again.
2. Write `docs/releases/v0.4.0-beta.15.md`, with `## Breaking`.
3. Push; wait until `ci` and `live` pass on that commit.
4. Dispatch the dry run of `release.yml`; then `publish=v0.4.0-beta.15`; then check `release-proofs.yml`.

## Prove

```sh
gh release view v0.4.0-beta.15 -R PodUtils/podnet   # the files, the notes with the gate and the scenarios
```

## Done

2026-11-17, the tag `v0.4.0-beta.15` at `4a8c52a`. ci 44046524678 and live 44046524676 passed on it. The dry run of
`release.yml` 44048483522 passed (with the gate). The release run 44053595040 published the release (17 files),
`publish.yml` 44053599483 passed, and `release-proofs.yml` 44056728232 passed.


# T-421: The WebSocket and relay connections ignore `--ca-file`, and a proxy's refusal of the upgrade reads as a bad token

**Source:** T-358 step 1, the lane row with WebSocket through the proxy (2026-11-17): `podnet relay connect` in
`podlab run --way-out ws-upgrade-refused` (podlab v0.68.0).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

1. In the lab, the proxy terminates TLS with its own CA, which `SSL_CERT_FILE` holds. `podnet bench` of the relay host
   passed; `podnet relay connect` failed with `invalid peer certificate: UnknownIssuer`, also with `--ca-file` naming the
   same file, and its message said to use `--ca-file`.
2. With the CA trusted, the proxy refused the WebSocket upgrade with its own 403. podnet said "the relay refused the
   connect token (403)" and exited 78: a user looks at the token, not at the proxy.

## Premise

- Each WebSocket and relay connection of podnet passed `pod_tls::Trust::Default`: the extra bundle, the system bundles
  and the compiled-in roots. `Context::roots_for(Trust::Default)` does not read the CA file of the context. The calls:
  `src/relay_reverse.rs` (pair, node, connect), `src/relay_reverse/resume.rs`, `src/ws_relay.rs` (the relay leg of the
  tunnel), `src/ws.rs` (the WebSocket client of `connect` and `--url`), `src/networks/devtunnel.rs`. A host behind a
  proxy that re-signs TLS could not use them.
- `pod_relay::way_refusal::upgrade_refused` tells a refusal on the way (a proxy's page, a 426) from the relay's own
  (`reverse: forbidden`).

## Approach

1. `podnet_core::trust::ws_trust()`: the configuration of the context (`Trust::caller`), which holds the roots of
   `--ca-file` or `SSL_CERT_FILE`. When the roots do not load, the CA file alone (`Trust::File`): no fall back to other
   roots. Each call above takes it; `src/resolve.rs` takes it in place of its own copy.
2. `connect_error` and the resume layer: a refusal on the way gives "a proxy or another hop on the way refused the
   WebSocket upgrade to the relay", exit 69, before the mapping of 401 and 403 to the token.
3. Lane rows (T-358): `way-relay` (`--way-out proxy-only`: the relay refuses the token, exit 78) and `way-relay-refused`
   (`--way-out ws-upgrade-refused`: exit 69 with the text of the hop).

## Prove

```sh
cargo test -p podnet-core --lib trust::
cargo test -p podnet --lib relay_reverse::
LANE_DIR=/var/tmp/lane/out sh scripts/lab-lane.sh PODLAB PODNET way-out way-ws
```

## Done

2026-11-17, the commit of this entry. Tests: `ws_trust_is_the_configuration_of_the_context`,
`ws_trust_keeps_a_bad_ca_file`, `a_refusal_on_the_way_is_not_the_token`. The lane with podlab v0.68.0 and the debug
build: `way-out way-tunnel`, `way-out way-relay` and `way-ws way-relay-refused` pass. The control: the release binary
of `v0.4.0-beta.15` fails `way-relay-refused` with `UnknownIssuer` (exit 69, the text of the hop missing).
`docs/relay.md` says it.

# T-422: Publish `v0.4.0-beta.16`: the gate, the dry run, the publish and the release proofs

**Source:** `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease when a batch of fixes is green. The batch since
`v0.4.0-beta.15`: T-421 (the WebSocket and relay connections take `--ca-file`; a proxy's refusal of the upgrade), T-420
(podcrates `v0.68.0`), T-326 (rsa 0.10.0) and the lane rows of T-358.
**Category:** release
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

beta.15 cannot reach the relay from a host behind a proxy that re-signs TLS (T-421). The fix is on `main`.

## Premise

- The steps are `docs/maintenance.md`, "Release".

## Approach

1. Set the version `0.4.0-beta.16` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`), write the llms files again.
2. Write `docs/releases/v0.4.0-beta.16.md`, with `## Breaking`.
3. Push; wait until `ci` and `live` pass on that commit.
4. Dispatch the dry run of `release.yml`; then `publish=v0.4.0-beta.16`; then check `release-proofs.yml`.

## Prove

```sh
gh release view v0.4.0-beta.16 -R PodUtils/podnet
```

## Done

2026-11-17, the release `v0.4.0-beta.16` (commit 3ab4831): a prerelease with 17 assets, published at 14:07 UTC. The
publish run passed; the run of `release.yml` 44091086685 passed; `release-proofs.yml` 44100847097 passed.
`docs/releases/v0.4.0-beta.16.md` has a section `Known problems` for the QA round (GitHub #610 to #648) and for
PodUtils/podrunner#213 and #142. No release follows until T-450 to T-455 are done (`docs/decisions.md`, 2026-11-17).


# T-424: `git init` of an existing bare repository fails (GitHub #614)

**Source:** GitHub #614 (https://github.com/PodUtils/podnet/issues/614), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A second `podnet git init demo --root R` said `the path "demo" exists and is not a repository` (exit 64) while `R/demo.git`
was the bare repository of the first run. A script that runs `git init` twice failed.

## Premise

- `create_repo` (`crates/podnet-serve/src/git/mod.rs`) refused a path when `NAME` or `NAME.git` existed, with one text.
  `git init` of git itself passes on an existing repository.

## Approach

1. When `NAME.git` is a bare repository (a directory, not a link, with `HEAD`, `objects` and `refs`), `create_repo`
   gives its directory and changes nothing. Another file or directory at `NAME` or `NAME.git` is refused, and the text
   names that path.
2. The CLI test and `docs/serve.md` say it.

## Prove

```sh
cargo test -p podnet --test cli -- git::
```

## Done

2026-11-17, the commit of this entry. Reproduced with the debug build (the second run: exit 64). `git_serve_init_and_urls`
checks that a second init by `team/app` and `team/app.git` passes and prints the same path, and that a file at the path,
`../x` and `a/.git` exit 64: it passes.

# T-425: `warp ssh-config` prints a `GIT_SSH_COMMAND` line with three layers of quotes (GitHub #615)

**Source:** GitHub #615 (https://github.com/PodUtils/podnet/issues/615), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** chore
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The line `# export GIT_SSH_COMMAND='ssh -o '\''ProxyCommand='\''\'\'''\''PATH'\''\'\'''\'' warp nc %h %p'\'''` is hard to
read. The report says that it does not work.

## Premise

- Measured: the line works. After the shell reads it, git's `sh` gives ssh the option
  `ProxyCommand='PATH' warp nc %h %p`, and ssh's `sh -c` runs `PATH warp nc HOST PORT` (a stand-in `ssh` printed each
  argument). The `'\''` of the value is quoting that the shell of git removes.

## Approach

1. A path of safe characters (letters, digits, `/._-+@%:,=`) gives the short form
   `export GIT_SSH_COMMAND="ssh -o 'ProxyCommand=PATH warp nc %h %p'"`. Another path keeps each layer of quotes.

## Prove

```sh
cargo test -p podnet --lib warp::nc::
```

## Done

2026-11-17, the commit of this entry (`src/warp/nc.rs`). `the_ssh_config_uses_warp_nc` checks the short form;
`the_printed_block_quotes_the_path_as_the_written_block` runs the line in `sh` for three paths (with a space and a
quote) and gets the path: both pass.

# T-426: The unlock flow of a PIN is not documented (GitHub #618)

**Source:** GitHub #618 (https://github.com/PodUtils/podnet/issues/618), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** docs
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`serve --pin` and `share --password` lock the files with a PIN. The documents do not say how a client sends it.

## Premise

- The unlock is a POST of a form to `/__podnet/unlock` with the fields `password` and `next`; the answer is 303 with the
  cookie `podnet_pin` (`crates/podnet-serve/src/gate.rs`).

## Approach

1. `docs/serve.md` gives the flow with a `curl` example, and says that the PIN is not HTTP Basic and not a header.

## Prove

```sh
grep -n '__podnet/unlock' docs/serve.md
```

## Done

2026-11-17, the commit of this entry: `docs/serve.md`, under `podnet share`; `serve --pin` points to it.

# T-427: `route` writes `router-usage.json` to the global state directory, not to `state_dir` (GitHub #619)

**Source:** GitHub #619 (https://github.com/PodUtils/podnet/issues/619), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The usage of a door with a quota went to `router-usage.json` of podnet's state directory, also when the router file has
`state_dir`. Two routers with two state directories shared one count, and a new router started with a spent quota.

## Premise

- `door_limit::usage_file` read the state directory of podnet only (`src/router/door_limit.rs`).

## Approach

1. `usage_file(state_dir)`: the `state_dir` of the router file, else the state directory of podnet. A door of the file
   and a door that the control socket adds take the `state_dir` of the router.
2. `docs/router.md` names both places.

## Prove

```sh
cargo test -p podnet --lib router::
```

## Done

2026-11-17, the commit of this entry. Read in the code (the call took no `state_dir`). The test
`the_usage_file_follows_the_state_dir_of_the_router` passes; `cargo test -p podnet --lib router::`: 74 passed.

# T-428: The release binaries have no optional link carrier, and the notes do not say it (GitHub #620)

**Source:** GitHub #620 (https://github.com/PodUtils/podnet/issues/620), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** docs
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The binaries of `v0.4.0-beta.15` have the link carriers `unix` and `memory` only: `release.yml` builds the default
features. `docs/build.md` says so; the release notes and `docs/link.md` do not.

## Premise

- `docs/build.md`: "The static musl build of the release set uses the default features. Build a feature on every target
  of the release set before it joins that set."

## Approach

1. `docs/link.md` and the notes of each release say that a release binary has the carriers `unix` and `memory` only.
2. A feature joins the release set when it builds and passes its tests on each target of the release (`docs/build.md`).
   A second binary for each target with `full` is the other way; it needs a call on the size and the risk of each
   target.

## Prove

```sh
grep -n 'default builds' docs/link.md
```

## Correction

2026-11-17: step 1 is done (`docs/link.md`; the notes of the next beta). Step 2 waits for the build of each feature on
each target of the release set.
2026-11-25: step 2 takes the first way: the feature `release` (`tcp`, `ws`, `h2`, `quic`, `udp`, `poll`, `dns`) for
each release-mode build (`release.yml`, `rebuild.sh` for a tag that has the feature, `gate.yml`, `live.yml`, the job
`static` of `ci.yml`). The size, measured on x86-64 musl: 42.6 MB default, 45.1 MB with `release` (+5.8 %), 45.9 MB
with `full`. The other features (`s3`, `icmp`, `ntp`, `smtp`, `imap`, `ss`, `trojan`, `sidecar`, `c-abi`) stay out. The
proof is the dry run of `release.yml` (each target builds) after `v0.4.0-beta.19`.

## Done

2026-11-26, `3ef7d0f` and `5e67a23`. Step 1: `docs/link.md` and the notes name the carriers of a release binary. Step
2: the feature `release` (`tcp`, `ws`, `h2`, `quic`, `udp`, `poll`, `dns`) for each release-mode build. The dry run
45731591597 of `release.yml` on `49baa57` built each of the eight targets with it (x86-64 and ARM64 in the build image,
the others with cargo-zigbuild), passed the lane, and checked that each binary is static. `cargo test --features release
--test link_features`: 11 passed. The notes of the next beta (`v0.4.0-beta.20`) list the carriers.
# T-429: Small output faults of the QA round (GitHub #622)

**Source:** GitHub #622 (https://github.com/PodUtils/podnet/issues/622), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Eight small faults of output: the header `x-robots-tag` twice, two spaces in `stopped  NAME`, `origin   Compiled`, no
`schema` in the JSON of `send`, `receive` and `relay pair`, the WARP error twice, `autostart add` exits 69 and keeps the
entry, the text of `ssh-keygen` in the passwd shim, and no carrier hint for `link listen tcp://...`.

## Premise

- Measured and read: (1) a local `serve` sends no `x-robots-tag`, and no code of podnet or podcrates sets it: the edge of
  Cloudflare adds it on a quick tunnel. (2, 3) the two spaces are the column of each status line (a label of a fixed
  width), as in `podnet: ssh-relay  URL`. (4) a JSON document has `schema`; an event line (`tunnel`, `ssh-relay`,
  `serve`, `send`, `receive`) has `event` and no `schema`. (5) the line printed the cause after a text that held it.
  (6) the entry stays on purpose: the line for `~/.profile` runs it. (7) `ssh-keygen` exits 0 or 255, by its build.

## Approach

1. Item 5: `podnet::exit::text` joins the causes of an error and skips a cause whose text the line holds (the rule of
   `pod_core::Failure::chain`); the error lines of `src/main.rs` take it.
2. Item 6: the text says that the entry is kept and how to remove it (`--manual`: GitHub #630).
3. Item 7: `docs/networks.md` and `docs/target-environment.md` say "exits 0 or 255".
4. Item 8: with the link work (the hint of `--carrier`).
5. Items 1 to 4: no change, for the reasons of the Premise.

## Prove

```sh
cargo test -p podnet-core --lib exit::
```

## Done

2026-11-17: items 5, 6 and 7 in bb8a1209 (`text_names_each_cause_once`). Item 8 in ee5e2ad5:
`carriers::check_address` refuses an address of another carrier on the `unix` carrier (exit 64) and names the
carrier (`a_network_address_on_the_unix_carrier_names_the_carrier`, and the CLI test
`a_network_address_on_the_unix_carrier_exits_64_and_names_the_carrier`); before, `link listen 127.0.0.1:18084` made a
socket file of that name. `text_names_each_cause_once` passes; it also shows that
`{err:#}` named the cause two times.

# T-430: `pod -x` with a dead upstream: a non-blocking connect() looks connected, then hangs (GitHub #610)

**Source:** GitHub #610 (https://github.com/PodUtils/podnet/issues/610), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M7
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

With the seccomp backend and an upstream that does not answer (`-x socks5h+unix:/tmp/no-such.sock`,
`-x socks5h://127.0.0.1:9`), `curl` waited for its own time limit, and the warning said that each connect() is refused.

## Premise

- Reproduced where podnet cannot write the memory of the program (the test hook `PODNET_NETNS_TEST_MEM=none`; the host
  of the report has no `AF_INET` bind and no netns). A non-blocking connect got `EINPROGRESS`
  (`crates/podnet-netns/src/connect.rs`), and `getsockopt(SO_ERROR)` went to the kernel, which gave 0 for the socket
  pair: the program read a connected socket that never got a byte.

## Approach

1. On such a host, a non-blocking connect waits for the end of the dial and gives 0 or `ECONNREFUSED`. It can block
   for the time of the dial (at most 30 s).
2. The warning says what happens: each connect() of the program through the upstream fails with `ECONNREFUSED`.
3. `docs/pods.md` and `crates/podnet-netns/README.md` say both.

## Prove

```sh
cargo test -p podnet --test pod_seccomp -- a_dead_upstream_refuses_a_nonblocking_connect_without_memory_writes
```

## Done

2026-11-17, commit 8a0b0f19. Before: `connect_ex 0 after 0.003` and `curl -m 5` waited 5 s; after: `connect_ex 111`, and
curl ends in 0.05 s with exit 7. The test passes; with the fix reverted it fails (`CONNECT:-1:115`, `SOERROR0:0:0`).
`cargo test -p podnet-netns`: 151 passed; `--test pod_seccomp`: 44 passed.
The test: `a_dead_upstream_refuses_a_nonblocking_connect_without_memory_writes` (`tests/pod_seccomp/tcp.rs`).

# T-431: `pod -r 'cidr:...=reject'` does not refuse a name (GitHub #611)

**Source:** GitHub #611 (https://github.com/PodUtils/podnet/issues/611), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** docs
**Milestone:** M7
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

With `-r 'cidr:140.82.0.0/16=reject'`, a connect to `api.github.com` gave `connect_ex 0`, then a hang.
`--default-action reject` gave 111.

## Premise

- Measured: a connect to the address `140.82.112.6` gives 111 (`refused by the rule cidr:...`). A `cidr:` rule matches
  addresses only. The fake DNS of the pod answers a name with an address of its own and does not look up the real one,
  so a `cidr:` rule cannot match `api.github.com`. The flow went to the dead upstream of the report: the hang is T-430.

## Approach

1. `docs/pods.md`: a `cidr:` rule does not match a name; a `domain:` rule does.

## Prove

```sh
grep -n 'does not match a name' docs/pods.md
```

## Done

2026-11-17, commit eff5dcea. After T-430 the command of the report gives 111.

# T-432: `pod -x direct` prints a false warning (GitHub #621)

**Source:** GitHub #621 (https://github.com/PodUtils/podnet/issues/621), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`podnet pod -x direct -- curl ...` printed `the upstream :0 does not answer ...`, then the program worked.

## Premise

- The check of each upstream at the start (`crates/podnet-netns/src/lib.rs`) also dialled `direct`, which has no host
  and port 0. `docs/pods.md` makes `-x` and `--egress` one option, and `--egress direct` becomes `-x direct`.

## Approach

1. The check skips `direct`. `docs/pods.md` lists `direct` as an upstream.

## Prove

```sh
cargo test -p podnet-netns --lib -- only_a_dead_upstream_gets_the_warning
```

## Done

2026-11-17, commit 8a0b0f19. No warning for `-x direct`; the test passes and fails with the fix reverted.
The test: `only_a_dead_upstream_gets_the_warning` (`crates/podnet-netns/src/lib.rs`).

# T-433: `net proxy` and a target outside the virtual network (GitHub #609)

**Source:** GitHub #609 (https://github.com/PodUtils/podnet/issues/609), Nemo-010 (outside: verified, not a defect).
**Category:** defect
**Milestone:** M7
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The report says that a SOCKS5 CONNECT to `140.82.112.6:443` through `podnet net proxy` gets success (`0x00`), then EOF.

## Premise

- The reply of the report is `05 01 00 01 ...`: its second byte is `0x01`, the SOCKS5 answer "general failure", not
  success. An HTTP CONNECT gets `502 Bad Gateway`: `140.82.112.6:443 is outside the subnet of the stack`.
- `net proxy` is a door into the virtual network (`docs/networks.md`, `docs/virtual-network.md`); a real target goes out
  through `--forward` or the egress of the process. The timeout of `pod -x` through it was T-430.

## Approach

1. No change of the code. The issue closes with the reason.

## Prove

```sh
cargo test -p podnet-vnet --lib -- a_target_outside_the_subnet_fails_at_once_on_each_door
```

## Done

2026-11-17: verified with the debug build: the same reply bytes (`05 01`) and the 502 of HTTP CONNECT.

2026-11-20 (T-450): the test `a_target_outside_the_subnet_fails_at_once_on_each_door`
(`crates/podnet-vnet/src/proxy.rs`) pins the behaviour of the report: the SOCKS5 reply `05 01` and the 502 of HTTP
CONNECT, each in less than 1 s. There was no fix, so there is no control on old code. `cargo test -p podnet-vnet --lib
proxy::`: 8 passed.

# T-434: `serve` drops a client that half-closes before the answer (GitHub #613)

**Source:** GitHub #613 (https://github.com/PodUtils/podnet/issues/613), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`printf 'GET / ...' | podnet link dial ...` to a listener with `--to unix:SOCK` gave no answer; with stdin kept open, the
answer came. The report names `link dial`.

## Premise

- Measured: the link is correct. On the end of stdin the dialer sends the close of its half (`Frame::Close` of
  `pod-link`, the reads go on), and the listener half-closes the target and sends the rest of its answer
  (`tests/link_raw.rs`). With a Python HTTP server as the target, 100 of 100 runs got the answer.
- The fault is in `podnet serve` as the target: `axum::serve` keeps the default of hyper, `half_close(false)`, so hyper
  drops a connection that reads the end of the request stream before its answer is ready. A WebSocket of
  `connect --stdio` or `relay connect` has no half close, so the target never saw the end there.

## Approach

1. `podnet_serve::road::serve`: `axum::serve` with `http1().half_close(true)`. `serve --local`, `share --local`, the local
   road and the private socket of a road take it.
2. `docs/serve.md` and `docs/link.md` say how a half close goes.

## Prove

```sh
cargo test -p podnet-serve --lib -- local_road_answers_a_client_that_half_closes
cargo test -p podnet --test cli -- serve_local_answers_a_client_that_half_closes
```

## Done

2026-11-17, commit 1bd1a99f. Both tests pass; each fails without the fix (`HTTP/1.0: ""`). `cargo test -p podnet-serve`:
171 passed.
The tests: `local_road_answers_a_client_that_half_closes` (`crates/podnet-serve/src/road/tests.rs`) and
`serve_local_answers_a_client_that_half_closes` (`tests/cli/serve.rs`).

# T-435: `expose ... -- --detach` stays in the foreground (GitHub #612)

**Source:** GitHub #612 (https://github.com/PodUtils/podnet/issues/612), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The help of `expose` gives `--detach` as an argument after `--`. `podnet expose ... -- --detach --name X` stayed in the
foreground, and `podnet ps` showed no X, for each `--via`.

## Premise

- `main` does the step of `--detach` and `--name` with `detach_of(&cli.command)`, which gave none for `expose`; `expose`
  read its inner command later, in the runtime (`src/cli/cmd/expose.rs`, `src/cli/opts.rs`).

## Approach

1. `expose::resolve` turns `expose` into the command that it stands for (tunnel, relay node or net proxy) right after the
   parse, before the step of `--detach`. A global option after `--` takes priority over the same option before it.

## Prove

```sh
cargo test -p podnet --bin podnet -- expose_gives_its_command_line_to_main
cargo test -p podnet --test cli -- net::expose_detaches_with_the_arguments_after_the_separator
```

## Done

2026-11-17, commits dfb836e6 and 6ee419ff. `expose 127.0.0.1:9 --via switch --as 10.66.0.2:8080 -- --detach --name X612`
prints the detached document and exits 0; `podnet ps` lists X612. The CLI test fails without the change of `src/main.rs`
(the 60 s limit). A detached instance has the name of the inner command by default.
The test: `expose_detaches_with_the_arguments_after_the_separator` (`tests/cli/net.rs`).

# T-436: A syntax error of the command line names `podnet update --check` (GitHub #616)

**Source:** GitHub #616 (https://github.com/PodUtils/podnet/issues/616), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

After clap's error for a value, an extra positional or a flag of another command, podnet added "podnet VERSION does not
know 'TOKEN'; a newer release can: podnet update --check".

## Premise

- `usage_hints` (`src/cli/help.rs`) added the line for each unknown argument.

## Approach

1. The line comes only for a flag (`--word` or `-w`) that no command of this binary knows.

## Prove

```sh
cargo test -p podnet --bin podnet -- only_a_flag_that_no_command_knows_names_the_update
```

## Done

2026-11-17, commit fd1c13a9. The test covers a value, an extra positional, a flag of another command, `-L`, `-5` and
`--bogus` (which keeps the line); it fails without the check. The exit code stays 64.
The test: `only_a_flag_that_no_command_knows_names_the_update` (`src/cli/help_tests.rs`).

# T-437: A command that ends at once prints the line "runs in the foreground" (GitHub #617)

**Source:** GitHub #617 (https://github.com/PodUtils/podnet/issues/617), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`serve --hash-password`, `tunnel --hello-world --check` and `serve DIR --check` printed the INFO line "runs in the
foreground until a stop signal". With `2>&1`, `HASH=$(... --hash-password 2>&1)` held the line before the hash.

## Premise

- `foreground_hint` (`src/cli/argv.rs`) skipped the line only for `--detach`, `--run-for`, `--exit-on-verify` and the help.

## Approach

1. No line for a mode that ends alone: `--check`, `--hash-password`, `--stdio`, `--stop-after` and `--idle-secs`.

## Prove

```sh
cargo test -p podnet --bin podnet -- a_mode_that_ends_alone_gets_no_foreground_line
```

## Done

2026-11-17, commit 9e279378. The test also checks that `serve www`, `tunnel --hello-world` and `connect -L` keep the
line; it fails with the old `src/cli/argv.rs`.
The test: `a_mode_that_ends_alone_gets_no_foreground_line` (`src/cli/argv_tests.rs`).

# T-438: `exit` needs `--stdio`, its only mode (GitHub #623)

**Source:** GitHub #623 (https://github.com/PodUtils/podnet/issues/623), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`podnet exit` exited 64 with no `--stdio`, and an empty session exited 1.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `--stdio` is optional (kept as a no-op). A client that closes stdin before its first byte exits 0.

## Prove

```sh
cargo test -p podnet -- exit_without_flag_serves_stdio
```

## Done

2026-11-17, commit 45d8d520. Reproduced on the code before the change. The test (`tests/exit_stdio.rs::exit_without_flag_serves_stdio`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-439: `autostart run` needs `--dir`, whose default is known (GitHub #624)

**Source:** GitHub #624 (https://github.com/PodUtils/podnet/issues/624), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`podnet autostart run NAME` with no `--dir` exited 64.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `--dir` defaults to `STATE/autostart`; the unit and the crontab line still give `--dir`.

## Prove

```sh
cargo test -p podnet -- autostart_run_defaults_to_the_state_dir
```

## Done

2026-11-17, commit 04ff0c5c. Reproduced on the code before the change. The test (`tests/commands.rs::autostart_run_defaults_to_the_state_dir`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-440: `forward --name` needs `--alias-port` (GitHub #625)

**Source:** GitHub #625 (https://github.com/PodUtils/podnet/issues/625), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`--name NAME=HOST:PORT` with no `--alias-port` exited 64.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. With no `--alias-port`, each name listens on the port of its target (each name has its own address). `--alias-port` still sets one port for all names.

## Prove

```sh
cargo test -p podnet -- forward_name_listens_on_the_port_of_its_target
```

## Done

2026-11-17, commit 3647ef92. Reproduced on the code before the change. The test (`tests/commands.rs::forward_name_listens_on_the_port_of_its_target`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-441: `warp env` gives nothing for a unix socket of `warp up` (GitHub #626)

**Source:** GitHub #626 (https://github.com/PodUtils/podnet/issues/626), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

With no TCP listener of `warp up`, `warp env` exited 69 and named no other way.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `warp env --unix` prints `PODNET_WARP_SOCKET` and `PODNET_PROXY=socks5h+unix:SOCKET`, or exits 69 when the socket is absent. Plain `warp env` keeps exit 69 (issue 255: `eval "$(podnet warp env)" && CMD` never runs CMD with no proxy), and its error names `--unix`.

## Prove

```sh
cargo test -p podnet -- warp_env_unix_gives_the_socket_proxy
```

## Done

2026-11-17, commit cb80a7d7. Reproduced on the code before the change. The test (`tests/commands.rs::warp_env_unix_gives_the_socket_proxy`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-442: `warp scan` names a command that does not exist (GitHub #627)

**Source:** GitHub #627 (https://github.com/PodUtils/podnet/issues/627), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The hint was `podnet warp --masque-endpoint ...`, which does not parse.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. The hint is `podnet warp up --masque-endpoint IP:PORT` (or proxy, run, tun, probe). The JSON is the same.

## Prove

```sh
cargo test -p podnet -- the_scan_hint_is_a_command
```

## Done

2026-11-17, commit 6f219c8c. Reproduced on the code before the change. The test (`cli_tests::the_scan_hint_is_a_command`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-443: The output of `serve --pin` does not say how to unlock (GitHub #628)

**Source:** GitHub #628 (https://github.com/PodUtils/podnet/issues/628), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The ready document and the text did not name the unlock flow.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. Ready documents and `--json` lines have `unlock` (`/__podnet/unlock`, else null); the text has a line `podnet: unlock` with the fields and the cookie (serve and share, local, tunnel, relay, `--at`).

## Prove

```sh
cargo test -p podnet -- serve_local_checks_users_and_the_pin
```

## Done

2026-11-17, commit 67dcd972. Reproduced on the code before the change. The test (`tests/cli/serve.rs::serve_local_checks_users_and_the_pin` (a real POST and the cookie), `share_local_serves_a_directory`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-444: `serve --list --tar`: the listing has no tar link (GitHub #629)

**Source:** GitHub #629 (https://github.com/PodUtils/podnet/issues/629), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The HTML listing had no link to the tar of the directory.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. With `--tar`, each HTML listing links to `?tar`. The JSON listing is the same.

## Prove

```sh
cargo test -p podnet -- a_listing_with_tar_links_to_the_tar
```

## Done

2026-11-17, commit e5f2ffae. Reproduced on the code before the change. The test (`crates/podnet-serve/src/files/tests.rs::a_listing_with_tar_links_to_the_tar`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-445: `autostart`: no `--json` for `list`, no `--manual` for a host with no manager (GitHub #630)

**Source:** GitHub #630 (https://github.com/PodUtils/podnet/issues/630), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`autostart list` had no JSON, and `add` on a host with no manager could only exit 69.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `autostart list --json` (`entries`: name, args, the names of the variables, way, line; never a value). `autostart add --manual` writes the entry, prints the line for login and exits 0. The tests of the core moved to `crates/podnet-core/src/autostart/tests.rs`.

## Prove

```sh
cargo test -p podnet -- autostart_manual_entry_lists_as_json_and_runs_with_no_dir
```

## Done

2026-11-17, commit c279914a, 8b6ea1e4. Reproduced on the code before the change. The test (`tests/commands.rs::autostart_manual_entry_lists_as_json_and_runs_with_no_dir`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-446: `relay node` has no `--json` (GitHub #631)

**Source:** GitHub #631 (https://github.com/PodUtils/podnet/issues/631), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Only `--detach` printed the ready document of a node.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `relay node --json` prints the ready document (`event`, `pair`, `relay`, `target`, `expires`, `expires_in`, `schema`) as one line when the node connects; the node runs on (`detach::foreground::on_ready`).

## Prove

```sh
cargo test -p podnet -- 
```

## Done

2026-11-17, commit 4793d647. Reproduced on the code before the change. The test (`tests/relay_node_json.rs`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-447: `ssh-relay` has no `--run-for`, and `-q` prints the banner (GitHub #632)

**Source:** GitHub #632 (https://github.com/PodUtils/podnet/issues/632), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`--run-for` was refused, and `-q` still printed the `[relay]` banner on stderr.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `ssh-relay --run-for SECONDS` exits 0 after that time; the banner prints only at the log level info. The URL line on stdout stays.

## Prove

```sh
cargo test -p podnet -- ssh_relay_run_for_ends_and_quiet_hides_the_banner
```

## Done

2026-11-17, commit 5ee06c68. Reproduced on the code before the change. The test (`tests/ssh_relay.rs::ssh_relay_run_for_ends_and_quiet_hides_the_banner`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-448: No way to add a user to the users file of `serve` (GitHub #633)

**Source:** GitHub #633 (https://github.com/PodUtils/podnet/issues/633), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

A user wrote the line `USER:ROLE:HASH` by hand.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `serve --add-user USER:ROLE --users FILE` reads the password from stdin and adds the line (a new file: mode 0600; an existing file must be private; replaced atomically). A user in the file exits 64 unless `--replace-user` (not `--replace`: the option of detach).

## Prove

```sh
cargo test -p podnet -- serve_add_user_writes_the_users_file
```

## Done

2026-11-17, commit 905972fd. Reproduced on the code before the change. The test (`tests/cli/serve.rs::serve_add_user_writes_the_users_file`, unit tests in `src/cli/cmd/serve_users.rs`) passes, and it fails on the
code before the change. The topic document, the help and the llms files say it.

# T-449: live: four checks read the log lines of the tunnel before the take of `pod-cloudflare`

**Source:** live 44092810296 on db8597c (2026-11-17), the step of the UDP probe, failed.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The tunnel of the step registered on the leg `http2` and verified its URL; `grep -q 'leg=http2' udpreject.err` failed. The
crate logs `the edge leg http2 won after N ms`, with no field `leg=`. Three more checks read the old lines (`the leg won
the race`, `leg=quic+warp`, `leg=http2+relay`), and the check `! grep -q 'tries the next host'` read a line that
`pod-warp` writes as `the next host is tried`: that check passed for each output.

## Premise

- T-423 took `pod-cloudflare`: the log lines of the tunnel are the crate's. The take did not search `live.yml` for the
  texts of the old lines.

## Approach

1. Each of the five checks takes the old and the new line (`grep -E`), so it reads the output of both versions.
2. At each take of a crate, search `.github/workflows/live.yml`, `scripts/live*.sh` and `scripts/lab-lane.sh` for each
   text of a log line that moved, and check that each text is still in the source.

## Prove

```sh
python3 scripts/check-workflows.py
# live: the steps of the UDP probe, the race, the relay and the second API host pass
```

## Done

2026-11-17, the commit of this entry. The new line of the run (`the edge leg http2 won after 413 ms`) and the line of
the run of the race (`the edge leg quic+warp won after 38111 ms`) match the new checks. The other texts of the tunnel that
`live.yml` reads (`no proxy carries UDP`, `registered over HTTP/2`, `the UDP probe was refused`, `moved to a new leg`,
`lost its leg`, `NAME: stopping`) are in the source.
2026-11-17, a sixth grep: `live` 44099840388 (617f071) failed on `registered over HTTP/2` in the relay step. The step
now also accepts `registered with the edge at NAME (http2`. A scan of each log grep of `live.yml` against the sources
of podnet and of podcrates v0.69.0 finds no other old text.
2026-11-17, a seventh grep: `live` 44130542899 (96c97c1) failed in `scripts/test_in_box.sh`, `box step warp: FAIL (demo
passed, but not on quic+warp)`, while the log said `the edge leg quic+warp won`. The script read `leg=quic+warp`; it now
also accepts the new line. The first scan read `live.yml` only; a second scan reads each script under `scripts/` and each
workflow, and finds no other old text (the other hits are texts that the code builds at run time, or texts of
`tests/`).


# T-450: Rule: an issue closes only with a test that reproduces it

**Source:** the owner's review of the QA round, 2026-11-17 (`docs/decisions.md`, The QA round and the release freeze).
**Category:** chore
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

An issue of a defect closed with a Done section that names a test, but no rule asked that the test reproduces the
report and fails on the code before the fix.

## Premise

- Most fixes of the QA round (T-424 to T-448) have such a test and a control. The rule is not written.

## Approach

1. `TODO/RULES.md`: the Done section of a defect names the test that reproduces the report, and the result of the test
   on the code before the fix (the control).
2. `cargo todo check` refuses a defect that is `done` with no test name in its Done section (a name of a test function,
   or a path under `tests/`).

## Prove

```sh
cargo todo check   # a done defect with no test is a fault
```

## Correction

2026-11-20: step 1 is done. `TODO/RULES.md`, section "A defect closes with a test that reproduces it", has the rule;
it applies to each defect done on or after 2026-11-17. The Done sections of T-430, T-432 to T-437 and T-474 now name
their tests. T-433 (GitHub #609, not a defect) got the test
`a_target_outside_the_subnet_fails_at_once_on_each_door` (`crates/podnet-vnet/src/proxy.rs`). Measured with the form
of COORD-110: 152 done defects; 0 of those done on or after 2026-11-17 name no test (8 before this change); 26 older
ones name no test.

Step 2 is a check of `pod-todo` (PodUtils/podcrates): `crates/podnet-todo` has no own code, and the rule "Shared code
first" puts the check in the crate. The ask is COORD-110 (GitHub #657): an opt-in key `defect_tests = "DATE"` in
`todo.toml`. When a tag of podcrates has it, take the tag, set the key in `TODO/todo.toml`, and close this entry.

## Done

2026-11-20, the commit of T-501. Step 2: podcrates `v0.79.0` has the key (PodUtils/podcrates@T-979, the answer to
COORD-110, #657); `TODO/todo.toml` has `defect_tests = "2026-11-17"`. `cargo todo check` agrees, and refuses a
planted done defect with no test name: `TODO/fixes.md:LINE: T-498: a done defect names no test in its Done section`.
Step 1 is the rule of `TODO/RULES.md` (2026-11-20, `177336b`).


# T-451: Contract tests from the manual: each flag of the help exists, each example parses, each JSON document has one schema

**Source:** the owner's review of the QA round, 2026-11-17; GitHub #612, #641, #642, #647.
**Category:** chore
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

The help named flags that the command refused (`ssh-relay --no-restart` names `--stop-after`, #647), arguments that had
no effect (`expose -- --detach`, #612), and `--json` documents with keys of their own (#641, #642). No test reads the
promises of the help.

## Premise

- The help and the manual come from one model (`pod-manual`, `podnet man --json`): each command, flag and example.

## Approach

1. A test walks the model: each flag that a long help names (`--word`) exists on that command or on a command that the
   text names; each example of the help parses with clap.
2. A JSON schema for each kind of document (`ready`, `detached`, `status`, `doctor`, event lines), in `docs/schema/`, and
   a test that each command with `--json` gives a document of its kind.
3. The release gate runs the tests on the release binary.

## Prove

```sh
cargo test -p podnet --test contracts
```

## Correction

2026-11-17: step 1 is a unit test of the binary, not tests/contracts.rs: the integration tests cannot build the
command tree. `src/cli/help_tests.rs::each_flag_that_a_help_names_exists` walks the tree. Each `--flag` of a long help
must be a flag of the command, of a command above it (a global flag) or below it, or of a command that the same
sentence names before the flag; a flag of another program ("magic-wormhole's --code-length") is allowed. The examples
of the manual parse already (`each_example_of_the_manual_parses`). Commit 43e53c0b: the test found 69 false references
(the global `--egress` named `--transport` and `--relay` on about 50 commands; the shared edge flags named `--config`;
`--no-restart` named `--stop-after` and `--idle-secs`; 6 small texts). Each text now names the command that owns the
flag, or names no flag; the test passes. Steps 2 and 3 (the schemas of the JSON documents, the run on the release
binary) are open; the Prove names tests/contracts.rs for step 2.

2026-11-17, step 2, commit 256b424d (an agent; merged on `main`). `tests/contracts.rs` (one table test,
`each_json_document_keeps_the_contract`, over 40 command lines with `--json` or `--detach` that run with no network)
checks four rules, written in `docs/cli.md`, "JSON documents": one JSON object on stdout (an event stream: one object
for each line, with a string `event`); a stamp (`schema`, `podnet`) on each object but the documents of the
environment; the endpoint keys (`url`, `listen`, `listens`) on each `ready` event and `detached` document; no secret
value. On `main` before the merge the test gives 20 faults (run by the lead with the new test on the old code); after
it, it passes. The fixes: the envelope of `crates/podnet-core/src/instances.rs` stamps each detached document; `ports`,
`autostart list`, `net addresses`, `man --json` and `--help --json` are stamped; each `ready` event has the endpoint
keys (also tunnel, serve, share, demo, relay node); `git serve` lists its listeners; `link listen` and `link connect`
print a `ready` event. The keys are added, none is removed. Not covered with no network: tunnel, share, demo,
ssh-relay, send, receive, relay pair, relay node, warp status, exit and scan, doctor, connect (their ready events take
the shared stamp code that the test runs); `link mesh` (it needs a mesh node). `link listen` writes its carrier address
in `listen` (not `unix:PATH`), as `docs/cli.md` says.

2026-11-17, step 3, commits e4cab496 and b2160d4f: `tests/contracts.rs` takes the binary of `PODNET_TEST_BINARY`
(else the binary that cargo built), and `gate.yml` runs it on the static release binary at its build path (`autostart
add` refuses a binary in a temporary directory such as `/var/tmp`). Here: the debug and the release binary pass, and
`/bin/false` fails at once.

## Done

2026-11-17, commits of steps 1 to 3 (see the Correction). `gate` 44198354501 on b2160d4 passed: the lane, its plant
and `cargo test --test contracts` on the static release binary. `ci` passed on the commits of each step.


# T-452: Error-path rows of the lane: dead, refused, rejected, unreachable, no DNS

**Source:** the owner's review of the QA round, 2026-11-17; GitHub #610, #611, #635, #639, PodUtils/podrunner#142.
**Category:** chore
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

The rows of the lane run one success path of each command. A dead upstream made a program hang (#610); an error line
named its cause twice (#635); `bench` exited 0 when each request failed (#639). No row checks a failure.

## Premise

- The labs of `pod-lab` give the host classes; `scripts/lab-lane.sh` runs the rows.

## Approach

1. For each command that dials (`pod`, `connect`, `forward`, `net proxy`, `relay connect`, `warp proxy`, `bench`,
   `tunnel`): five cases: a dead upstream, a refused connect, a reject rule, an unreachable target, a name that does not
   resolve. Each row checks: the end within a limit (no hang), the documented exit code, one error line that names each
   cause once.
2. The rows run in each lab where the case applies.

## Prove

```sh
LANE_DIR=/var/tmp/lane/out sh scripts/lab-lane.sh PODLAB PODNET
```

## Correction

2026-11-20, measured with `0.4.0-beta.17` (the commands of the Approach with a dead proxy `127.0.0.1:1`, a refused
port, a name under `.invalid`, a reject rule):

- `bench`: each fault ends at `--duration` with exit 1 (its help says so) and one line. No fault.
- `pod`: a name with no answer and a reject rule gave no line at the default level; a dead upstream on netns-kernel got
  a false line (T-496, done). The rows run on each push in `tests/pod_error_lines.rs`
  (`each_failed_connect_names_its_cause_once`), on each backend of the host.
- `tunnel` and `warp proxy` with a dead proxy: exit 69 at once, but a cause two or three times in the line (T-497,
  blocked on podcrates: COORD-111, GitHub #658).
- `relay connect` with no pair ends at once with exit 69 and one line before any dial.
- `forward`: each fault resets the client at once (the unreachable target after the limit of 29 s) and logs one line
  for each connection. The unreachable target named its step two times (T-498, done).
- `connect` checks its remote at the start: each fault ends at once with exit 69, but the line repeats its step
  (T-497, item 3, in `pod-ws`).
- `net proxy`: the unit tests `refusals_have_a_status_and_a_body` and
  `a_target_outside_the_subnet_fails_at_once_on_each_door` (`crates/podnet-vnet/src/proxy.rs`) hold its rows.
- Step 2 (2026-11-20): the case `errors` of the lane (`scripts/lab-errors.sh`, in CASES and NET_CASES) runs the rows
  that podnet passes now: `bench` (a refused port, a dead proxy, a name with no answer, an unreachable target) and
  `pod` (a name with no answer, a reject rule). A row checks the end within a limit, the exit code, one line of
  podnet with the cause, and no part of a line said two times. Run as uid 65534 with pod-lab `v0.68.0` on each
  profile: pass in each lab that holds here, n.a. for the pod rows in `proc-masked` and `no-proc` (the lab refuses
  the pod, with the reason); the two pod rows failed in `no-proxy-no-dns` (T-499, done: they pass now). `arm64-tcg`, `cgroup-v1` and
  `ipv6-only` do not hold on this host.
- 2026-11-20 (T-501, T-497): the rows of `connect` (`--stdio`), `tunnel` and `warp proxy` are in the case `errors`;
  each lab passes them.
- 2026-11-20: the rows of `relay connect` (exit 69, one line), `forward` (`forward_row`: the client is reset at once,
  one log line with the cause) and `net proxy --forward` (`net_proxy_row`: the CONNECT is answered and the stream is
  closed at once, as the near side of `ssh -L` does; one log line with the cause) are in the case `errors`. The rows
  found T-502 (done).

## Done

2026-11-20. The case `errors` of the lane (`scripts/lab-errors.sh`) has 20 rows: `bench` (a refused port, a dead
proxy, a name with no answer, an unreachable target), `connect --stdio`, `relay connect`, `forward` and `net proxy
--forward` (a refused port, a name with no answer, a dead proxy), `tunnel` and `warp proxy` (a dead proxy), and `pod`
(a name with no answer, a reject rule; `tests/pod_error_lines.rs` has the dead upstream and the refused port on each
backend). Each row checks the end within a limit, the documented exit code, one line of podnet with the cause, and no
part of a line said two times. Here all 20 pass. Run as uid 65534 with pod-lab `v0.68.0` in each profile and in the row
`testers`: each row passes or is n.a. with the reason (the lab refuses the listener, the pod, or the loopback);
`arm64-tcg`, `cgroup-v1` and `ipv6-only` do not hold on this host. The rows found T-496, T-497, T-498, T-499 and T-502
(each done). `ci` 44737373446 on `79b8331` passed. `gate` 44742237281 on `c9e394a` (the GitHub runner) failed in 7 labs: the words of Rust for an unreachable
network (`Network unreachable`) and a client limit of 10 s on an IPv6-only host (faults of the rows, fixed), and T-503
(the loopback nameserver of the runner, fixed). `gate` 44782773447 on `3836971` passed: each lab on the GitHub runner,
with the case `errors` and the row `testers`.


# T-453: A lab of the host class of the testers: no AF_INET bind, no netns, no write of the memory of a program

**Source:** GitHub #610 (it showed only where podnet cannot write the memory of the program).
**Category:** chore
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The lane has no lab where the seccomp backend cannot write the memory of the program. The defect of #610 hid there.

## Premise

- The test hook `PODNET_NETNS_TEST_MEM=none` gives the class to the tests of the seccomp backend.
- `pod-lab` has `bind-denied`; a lab of the three parts together may not exist.

## Approach

1. Ask podcrates (COORD) for a profile of `pod-lab` with the three parts, or make the lane give the hook in `bind-denied`.
2. The pod rows and the error-path rows (T-452) run there.

## Prove

```sh
LANE_DIR=/var/tmp/lane/out sh scripts/lab-lane.sh PODLAB PODNET testers
```

## Done

2026-11-20. Option 2 of the Approach: the lane has the row `testers` (`scripts/lab-lane.sh`, TESTER_CASES): the lab
`bind-denied` of pod-lab with `PODNET_NETNS_TEST_MEM=none` (the test hook of the seccomp backend: no write of the
memory of the program; a lab cannot take that away). It runs `pod-seccomp`, `pod-netns` and the error rows `errors`
(T-452). The lane runs it with each profile when no profile is given, so `gate.yml` runs it. Run as uid 65534 with
pod-lab `v0.68.0`: `pod-seccomp` and `errors` pass; `pod-netns` is n.a. with podnet's reason (exit 3: the lab refuses
each TCP connect of the program, in each namespace). `podlab run` keeps the variable: `hook=none` in the lab.

# T-454: A black-box QA pass before each beta

**Source:** the owner's review of the QA round, 2026-11-17.
**Category:** chore
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

External testers found 40 issues in one hour with the documents and the help only. podnet ran no such pass.

## Premise

- The faults were in the promises of the help and the documents, the error paths and the output.

## Approach

1. `docs/maintenance.md`, Release: before the dry run, an agent with the release binary, the documents and the help only
   (no source) runs each command in the labs and writes each finding as an issue. The release waits until each finding
   is an entry.
2. The prompt of the pass is a file of the repository (a new file `qa-pass.md` in `scripts/agent/`), so each pass is
   the same.

## Prove

```sh
ls scripts/agent/ | grep -x qa-pass.md
```

## Done

2026-11-20. `scripts/agent/qa-pass.md` is the prompt of the pass: the inputs (the binary of the version, `README.md`
and `docs/`, PODLAB), what the agent must not use (no source, no record, no issue), the host classes (the labs, as a
user that is not root), what to check (the help, each promise of the topic documents, the five error paths of each
command that dials, the JSON outputs and the exit codes, the background, the stop signals), and the form of a finding
(one issue for each fault, titled `[qa VERSION] COMMAND: ...`). `docs/maintenance.md`, Release, step 0, gives the
pass: a new agent with the binary and the documents only; the release waits until each finding is an entry and each
P1 and P2 finding is done. The first pass runs before the next beta.

# T-455: Readiness: the promises with a test, in place of the commands with a scenario

**Source:** the owner's review of the QA round, 2026-11-17.
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`scripts/gate-scenarios.py` marks a command as tested when one step calls it. The release notes gave that count as a
measure of maturity.

## Premise

- A command has more promises than one call: its flags, its examples, its JSON, its error paths, its exit codes.

## Approach

1. For each command: the count of its promises (from the model of T-451 and the rows of T-452) and the count with a
   test. The notes of a release give both.
2. The help of a command whose promises have no test says "Experimental".

## Prove

```sh
python3 -I scripts/gate-scenarios.py --check
python3 -I scripts/promises.py --report target/debug/podnet
```

## Done

2026-11-20. `scripts/promises.py`: a promise is a flag or an argument of a command (`podnet man --json`; the global
flags once; the flags that each subcommand of a group shares once, as `GROUP (each)`). A promise has a test when a
scenario of the release binary (live, the job `static` of ci, the lane with `scripts/lab-errors.sh`) gives it to the
command, or when a Rust test does (a statement with the words of the command and the flag; a helper file; a test
target named after a command of one word; the tests of `crates/podnet-netns` for `pod`; the text after
`#[cfg(test)]`). On the head: 384 of 651 promises have a test; `pod` 20 of 21, `bench` 5 of 5, the shared flags of
`warp` 8 of 19. `--notes` prints the section "The promises of each command" of the notes; `release.yml` prints it
after the scenarios, and the job `test` of `ci` runs `--report`. Step 2: the call of `docs/decisions.md` (2026-11-20,
The measure of readiness): the help keeps the rule of the command level (no scenario: "Experimental"). The measure
found T-500 (done).

# T-456: `pod --backend seccomp`: a connect to a loopback port gives EOPNOTSUPP (PodUtils/podrunner#213)

**Source:** PodUtils/podrunner#213 (2026-11-17, a member of the team): under `podnet pod --backend seccomp --egress
direct`, the official runner of GitHub gets `Runner connect error: Operation not supported (127.0.0.1:PORT)` to the
broker on the loopback.
**Category:** defect
**Milestone:** M7
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

A program in a seccomp pod cannot connect to a port of the loopback that a program of the host serves.

## Premise

- The report is podrunner's, with podnet `0.4.0-beta.13`; T-357 (`PODNET_VNET_FORWARD`) is in beta.14 and later.

## Approach

1. Reproduce with podnet of the head: a .NET or a Python client in `pod --backend seccomp --egress direct` to a TCP
   server of the host on 127.0.0.1, with and without the hook `PODNET_NETNS_TEST_MEM=none`.
2. Fix the path that gives EOPNOTSUPP, with a test of `tests/pod_seccomp`.

## Prove

```sh
cargo test -p podnet --test pod_seccomp -- loopback
```

## Correction

2026-11-17: part 1 does not fix the report. Its test used a Python client; the reporter's program is .NET. With the
.NET 8 `HttpClient` in the pod and `PODNET_NETNS_TEST_MEM=none`, a request to 127.0.0.1 still fails with
`Operation not supported (127.0.0.1:PORT)`, the text of the report. `strace` shows the cause:
`getsockopt(fd, SOL_IPV6, IPV6_V6ONLY)` before the `connect` gives EOPNOTSUPP. The pod cannot write the value into
the memory of the program, so the call goes to the kernel, and the kernel answers for the AF_UNIX socket pair.

Part 2 (open): where the pod cannot write the memory of the program, `socket` must give the program a TCP socket of
the kernel. Then the kernel answers each socket option before the connect. A loopback `connect` goes to the kernel;
any other `connect` waits for the dial (issue 610) and the pod then puts the upstream at the descriptor. Prove it with
the .NET program of the reproduction (a test that skips when no `dotnet` is installed) and with a C program that reads
`IPV6_V6ONLY` before `connect`; each must fail on the code before the fix.

## Done

2026-11-17, commit aaf9e2c (`T-456: seccomp: a loopback TCP connect gives the kernel socket when the pod cannot write
the memory`). Reproduction: a Python client in `pod --backend seccomp` to a TCP listener on 127.0.0.1 of the host. With
`PODNET_NETNS_TEST_MEM=none`, `getsockname` failed with errno 95 (EOPNOTSUPP); the .NET runner reads `LocalEndPoint`
and `RemoteEndPoint`, so it failed as in the report. The connect itself worked. The fix: with no write to the memory
and no `--flow-log`, the loopback dial gives the TCP socket of the kernel (`Link::Kernel`,
`crates/podnet-netns/src/handoff.rs`); the handoff puts it at the descriptor of the program, and the entry is marked
`kernel` (`crates/podnet-netns/src/sockopt.rs`). The kernel then answers `getsockname` and `getpeername`. A dial to
the program's own inbound listener keeps the relay. Test: `tests/pod_seccomp/loopback_name.rs`
(`a_loopback_connect_has_its_names_with_no_memory_write`): it passes; on the code before the fix it fails with
`NAME-ERROR:95`. `cargo test --test pod_seccomp`: 45 passed.

2026-11-17, part 2, commit 58fd0db8. On a host that refuses writes to the memory of the program, `socket` of a TCP
socket gives the program a socket of the kernel (`Sock::Kernel`, `crates/podnet-netns/src/socket.rs`). Each call on it
goes to the kernel (`crates/podnet-netns/src/seccomp/mod.rs`), but `connect`: a loopback target goes to the kernel; any
other target puts the socket pair of the pod at the descriptor (`pair_in_place`, `crates/podnet-netns/src/connect.rs`)
and the dial goes on as before. A host with no IPv6 refuses an `AF_INET6` socket of the kernel; then the pair stays.
Proof with the reporter's kind of program: the .NET 8 `HttpClient` in `pod --backend seccomp` with
`PODNET_NETNS_TEST_MEM=none`, to `http://127.0.0.1:PORT/`: 3 requests of 3 pass (before: `Operation not supported
(127.0.0.1:PORT)`, the text of the report). This host has no IPv6, so that run sets `DOTNET_SYSTEM_NET_DISABLEIPV6=1`;
a host with IPv6 gets an `AF_INET6` socket of the kernel. A target through the proxy still fails where .NET reads
`LocalEndPoint` (the limit of `crates/podnet-netns/README.md`). Test: `tests/pod_seccomp/loopback_name.rs` now reads
`TCP_NODELAY` before `connect`; it passes; on the code before part 2 it fails with `OPTION-ERROR:95`. Each
`tests/pod_seccomp*` target passes.


# T-457: `pod --backend seccomp`: the runner times out to github.com under the hold (PodUtils/podrunner#142)

**Source:** PodUtils/podrunner#142 (2026-11-15, a member of the team), podnet `0.4.0-beta.8`.
**Category:** defect
**Milestone:** M7
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

Under the seccomp hold (advisory) with podrunner's egress module, each `GET .../_apis/connectionData` of the runner
timed out after 60 s; `curl` and Node to a plain CONNECT proxy reached github.com.

## Premise

- T-430 (a non-blocking connect that looked connected where podnet cannot write the memory of the program) is a likely
  cause; the report is of beta.8.

## Approach

1. Reproduce with a .NET client (`HttpClient`, non-blocking connects) in a seccomp pod with an HTTP CONNECT upstream, on
   the head; with and without `PODNET_NETNS_TEST_MEM=none`.
2. If the head passes, ask podrunner (COORD) to run the case again with the next release.

## Prove

```sh
cargo test -p podnet --test pod_seccomp -- dotnet
```

## Done

2026-11-17, commit 040f89d (`T-457: seccomp: no handoff of a nonblocking descriptor (epoll)`). Reproduction on the
head with a .NET 8 `HttpClient` program (`SocketsHttpHandler`, no proxy of its own) in `pod --backend seccomp` with an
HTTP CONNECT upstream on a Unix socket: request 0 passed, requests 1 and 2 timed out after 20 s. `-v` showed `the
program holds the upstream socket (no relay)` for each connect. Cause: the handoff (T-065) replaced a nonblocking
descriptor after the program had put it in its epoll set; the close of the old socket removes it from the set, and
epoll never reports the new one. T-430 was not the cause. The fix: `hand_off` (`crates/podnet-netns/src/handoff.rs`)
takes a blocking descriptor only; a nonblocking one keeps the relay. After the fix the .NET program passes 3 requests
of 3 (45 ms, 7 ms, 4 ms). Test: `tests/pod_seccomp/epoll.rs` (`an_epoll_program_sees_its_socket_after_the_connect`, a
C program that adds the socket to epoll before `connect`): it passes; on the code before the fix it fails with
`EPOLL-TIMEOUT:read`. `tests/pod_seccomp_nonblock.rs` now expects the relay for a nonblocking dial.
`cargo test --test pod_seccomp` (46), `pod_seccomp_nonblock`, `pod_seccomp_handoff`, `pod_seccomp_slow_dial`,
`pod_seccomp_tcp`: each passes. podrunner runs its case again with the next release.

# T-458: `link`: the address scheme does not select the carrier; a `unix:` prefix is taken as a file name (GitHub #634)

**Source:** GitHub #634 (https://github.com/PodUtils/podnet/issues/634), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet link` took `unix:/path` as a file name `unix:/path`, and the scheme of the address did not select the carrier.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. The scheme of the address (`unix:`) selects the carrier, and the path is the part after the scheme.

## Prove

```sh
cargo test -p podnet --test cli -- the_unix_carrier_takes_unix_path
```

## Done

2026-11-17, commit 12f81ef0 and 31656579 (test). Reproduced on the code before the fix. The test `tests/cli/link.rs::the_unix_carrier_takes_unix_path` passes; on the code with `src/cli/cmd/link/mod.rs` and `src/carriers.rs` of before the fix (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-459: Error messages repeat the cause twice (GitHub #635)

**Source:** GitHub #635 (https://github.com/PodUtils/podnet/issues/635), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

An error line named its cause twice (for example `link` and `connect`).

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. Each cause is named once in an error line.

## Prove

```sh
cargo test -p podnet-core -- text_names_each_cause_once
```

## Done

2026-11-17, fixed before this issue by commit bb8a1209 (T-429, GitHub #622). On the head, the commands of the issue (`link`, `connect`) name each cause once. The test `crates/podnet-core/src/exit.rs::text_names_each_cause_once` passes; it failed on the code before bb8a1209 (T-429).

# T-460: `serve --local --pin`: a plain-HTTP listener sets a Secure cookie, so the PIN unlock does not persist (GitHub #636)

**Source:** GitHub #636 (https://github.com/PodUtils/podnet/issues/636), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

On a plain HTTP listener, the PIN cookie had the attribute `Secure`; a browser does not keep it, so each request asked for the PIN again.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. The cookie of a plain TCP listener has no `Secure` attribute; a TLS or tunnel listener keeps it.

## Prove

```sh
cargo test -p podnet-serve -- the_pin_cookie_of_a_plain_tcp_listener_is_not_secure
```

## Done

2026-11-17, commit fd90c4e8. Reproduced on the code before the fix. The test `crates/podnet-serve/src/road/tests.rs::the_pin_cookie_of_a_plain_tcp_listener_is_not_secure` passes; on the code with the serve and road files of before the fix (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-461: `route`: a door quota can be overshot by a full 64 KiB buffer per flow (GitHub #637)

**Source:** GitHub #637 (https://github.com/PodUtils/podnet/issues/637), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A door of `podnet route` with a byte quota let each flow pass a full buffer of 64 KiB after the quota.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. A read stops at the quota: no byte passes after it.

## Prove

```sh
cargo test -p podnet -- a_read_stops_at_the_quota
```

## Done

2026-11-17, commit ce428588. Reproduced on the code before the fix. The test `src/router/door_limit_tests.rs::a_read_stops_at_the_quota` passes; on the code with `door_limit.rs` and `door_stats.rs` of before the fix (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-462: `forward --dns`: the DNS door is printed as up before it binds; a bind failure exits 1 after the success line (GitHub #638)

**Source:** GitHub #638 (https://github.com/PodUtils/podnet/issues/638), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet forward --dns` printed the DNS door as up before the bind; a bind failure then exited 1 after the line of success.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. The line comes after the bind; an address in use exits 69 with no line of success; an upstream that cannot be reached gives SERVFAIL.

## Prove

```sh
cargo test -p podnet --test commands -- forward_dns_at_an_address_in_use_exits_69
```

## Done

2026-11-17, commit 80e482c7 and 31656579 (test). Reproduced on the code before the fix. The test `tests/commands.rs::forward_dns_at_an_address_in_use_exits_69` and `dns_door_answers_servfail_when_the_upstream_cannot_be_reached` passes; on the code with the forward and udp files of before the fix (exit 1, not 69; with SERVFAIL off, the SERVFAIL test fails) (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-463: `bench` exits 0 when each request fails (GitHub #639)

**Source:** GitHub #639 (https://github.com/PodUtils/podnet/issues/639), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet bench` exited 0 when each request failed.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `bench` counts the HTTP errors and exits 1 when no request passes; the help says it.

## Prove

```sh
cargo test -p podnet --test commands -- bench_counts_http_errors
```

## Done

2026-11-17, commit 34bbe792 and 31656579 (test). Reproduced on the code before the fix. The test `tests/commands.rs::bench_counts_http_errors` passes; on the code with the help and code of before the fix (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-464: `autostart add` exits 69 but still creates the entry (GitHub #640)

**Source:** GitHub #640 (https://github.com/PodUtils/podnet/issues/640), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet autostart add` on a host with no manager exited 69 and kept the files of the entry.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. An `add` that exits 69 leaves no entry; `--manual` (T-445) writes the entry and exits 0.

## Prove

```sh
cargo test -p podnet --test cli -- autostart_with_cron_and_with_nothing
```

## Done

2026-11-17, commit f695188b and 84cfd9c0. Reproduced on the code before the fix. The test `tests/cli/autostart.rs::autostart_with_cron_and_with_nothing` passes; on the code with `autostart.rs` of before the fix (the files of the entry stay after exit 69) (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-465: Detached `--json` documents do not share a schema (`url` and `http`, `doors`, `forward`) (GitHub #641)

**Source:** GitHub #641 (https://github.com/PodUtils/podnet/issues/641), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The ready documents of detached commands had keys of their own for one thing (`url`, `http`, `doors`, `forward`).

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. Each ready document has one envelope with the keys of the endpoint.

## Prove

```sh
cargo test -p podnet-core -- the_envelope_has_the_keys_of_the_endpoint
```

## Done

2026-11-17, commit 3d8d827c and 045aeb04. Reproduced on the code before the fix. The test `crates/podnet-core/src/instances.rs::the_envelope_has_the_keys_of_the_endpoint` passes; on the code with the envelope as a no-op (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-466: `--json` is accepted by some commands and refused by others (GitHub #642)

**Source:** GitHub #642 (https://github.com/PodUtils/podnet/issues/642), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`--json` worked with some commands that print a document and was refused by others (`route --detach`).

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `route --detach --json` prints the ready document.

## Prove

```sh
cargo test -p podnet --test router_control -- route_detach_json_prints_the_document
```

## Done

2026-11-17, commit da3abf2c. Reproduced on the code before the fix. The test `tests/router_control.rs::route_detach_json_prints_the_document` passes; on the code with `route.rs` of before the fix (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-467: `serve --check` refuses `--local`, so a plan of a local file server cannot be checked (GitHub #643)

**Source:** GitHub #643 (https://github.com/PodUtils/podnet/issues/643), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet serve --check --local ADDR` exited 64.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `serve --check` takes `--local`.

## Prove

```sh
cargo test -p podnet --test commands -- serve_check_takes_local
```

## Done

2026-11-17, commit 704e79b8 and 50bb9fa9. Reproduced on the code before the fix. The test `tests/commands.rs::serve_check_takes_local` passes; on the code with `conflicts_with = "check"` put back (exit 64) (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-468: The default log level prints an INFO line on stderr for `status` (GitHub #644)

**Source:** GitHub #644 (https://github.com/PodUtils/podnet/issues/644), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet status` (documented to open no connection) printed an INFO line on stderr with a proxy set. `update --check` also named the step of its error twice.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. `status` is quiet; the error of `update` names its step once.

## Prove

```sh
cargo test -p podnet --test cli -- status_is_quiet_with_a_proxy update_error_names_its_step_once
```

## Done

2026-11-17, commit be6673bd and b1176e32. Reproduced on the code before the fix. The test `tests/cli/log.rs::status_is_quiet_with_a_proxy` and `update_error_names_its_step_once` passes; on the code with the log line at INFO again, and `update.rs` of before the fix (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-469: `podnet-install.sh --dry-run` prints the proxy line twice; the default directory can be noexec (GitHub #645)

**Source:** GitHub #645 (https://github.com/PodUtils/podnet/issues/645), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`podnet-install.sh --dry-run` printed the proxy line twice when `https_proxy` and `http_proxy` are the same, and the
default install directory can be on a noexec mount.

## Premise

- Verified on the head: `inst_proxy` of `scripts/install/core-net.sh` prints each variable. The noexec text names
  `--prefix DIR` already.
- `scripts/install/core-net.sh` and `scripts/install/core.sh` are copies of PodUtils/podcrates v0.69.0, pinned by
  SHA-256 (`scripts/install/SOURCES`): frozen in podnet.

## Approach

1. podcrates prints one proxy line when the two variables are the same, and checks that the default directory allows
   exec (a fallback directory, or a clear text). podnet takes the new copy at the next take.

## Prove

```sh
sh scripts/install-test.sh
```

## Done

2026-11-18, the commit of T-480. podcrates fixed the core in `v0.70.0` (their PodUtils/podcrates@T-883: one proxy line for one proxy; a
default prefix on a noexec mount gives way to the next default, with podcrates' own test). podnet took the core (T-480).
The new case 3b of `scripts/install-test.sh` ("one proxy in two variables gives one proxy line") fails on the core of
`v0.69.0` (two lines `proxy: http://proxy.invalid:3128`) and passes on the core of `v0.70.0`.

# T-470: `expose --via switch` refuses the `http://` target that `expose` documents (GitHub #646)

**Source:** GitHub #646 (https://github.com/PodUtils/podnet/issues/646), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet expose --via switch` refused a target `http://HOST:PORT`, the syntax of the help of `expose`.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. A URL target on a TCP road gives its host and port.

## Prove

```sh
cargo test -p podnet -- a_url_on_a_tcp_road_names_the_host_and_port
```

## Done

2026-11-17, commit 485052ab. Reproduced on the code before the fix. The test `src/cli/cmd/expose.rs::a_url_on_a_tcp_road_names_the_host_and_port` passes; on the code with the check of the URL removed (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-471: `ssh-relay --no-restart`: the help names `--stop-after`, `--idle-secs` and `--run-for`, which the command refuses (GitHub #647)

**Source:** GitHub #647 (https://github.com/PodUtils/podnet/issues/647), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A hint of the help of `ssh-relay` named flags that the command refused.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. Each server command takes the flags that its hints name.

## Prove

```sh
cargo test -p podnet -- each_server_command_takes_the_flags_of_its_hints
```

## Done

2026-11-17, commit dc278d7c. Reproduced on the code before the fix. The test `src/cli/argv_tests.rs::each_server_command_takes_the_flags_of_its_hints` passes; on the code with `relay.rs` and `opts.rs` of before the fix (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-472: `serve --list`: the HTML listing omits the modified time and the upload limit (GitHub #648)

**Source:** GitHub #648 (https://github.com/PodUtils/podnet/issues/648), Nemo-010 (outside: verified, labelled `confirmed`).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The HTML listing of `podnet serve --list` had no time of change and did not say the upload limit.

## Premise

- Verified on the head before the change (the command of the issue, its code and its help); the issue is right.

## Approach

1. The listing shows the time of change; the upload form names the limit.

## Prove

```sh
cargo test -p podnet-serve -- a_listing_shows_the_modified_time a_form_uploads_into_a_directory
```

## Done

2026-11-17, commit 003d92a9. Reproduced on the code before the fix. The test `crates/podnet-serve/src/files/tests.rs::a_listing_shows_the_modified_time` and `a_form_uploads_into_a_directory` passes; on the code with the time column and the upload limit removed (the new test kept) it fails. Branch of the QA round merged at main; `cargo test --workspace` of the branch after the merge with main: the `cli` target 113 of 113.

# T-473: A survey of podnet's copies of shared code against podcrates and the siblings (PodUtils/podcrates#221)

**Source:** PodUtils/podcrates#221 (COORD-131 of podcrates, 2026-11-17, a member of the team): its ask 3, to name each
copy of shared code that podnet holds and that the survey of podcrates did not list.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

podcrates' survey names three copies in podnet: `crates/podnet-core/src/detach.rs` (their entry 879), `src/update.rs`
with `PodUtils/podnet@749da3d:src/update/cosign.rs` (their entry 880) and the instance lock (their entry 877). Other copies of podnet are not
named.

## Premise

- The three named copies wait for crates of podcrates that do not exist yet; podnet takes each at its tag (T-137).
- The rulings "Shared code" and "Quality first" that COORD-131 cites are on podcrates' `main` at `0fefea74c`. podnet
  has them in `CLAUDE.md` and in `docs/decisions.md` (2026-11-17).

## Approach

1. Read each module of podnet against the design document and the `README.md` of each crate of podcrates, and against the
   crates of podssh, podvm and podrunner.
2. For each copy: remove it where a crate of podcrates covers it; else name it in one COORD issue `(to podcrates)`.
3. The survey of 2026-11-17 (an agent; each claim to check against the code before the change) found 23 places:
   - Remove now (podcrates covers it): src/transport/ (1380 lines, no caller; checked: nothing uses it and no
     document promises it); the CONNECT client of `crates/podnet-core/src/connect.rs` (its last caller is the
     `--connect-proxy` path of `ssh-relay`); the HTTP head reader of `crates/podnet-netns/src/tailscale.rs` and of
     `src/router/control.rs` (`pod_proxy::http`); the retry waits of `src/cli/cmd/warp/run.rs`, `warp/tun.rs` and
     `crates/podnet-serve/src/relay/supervise.rs` (`pod_core::backoff`, with jitter); the date code of
     `src/doctor/clock_check.rs`, `crates/podnet-serve/src/files/listing.rs` and `src/router/door_limit.rs`
     (`pod_core::clock`); the CA list of `src/warp/up.rs` (T-475); the file writes of
     `crates/podnet-netns/src/report.rs`, `src/warp/ssh_config.rs` and `src/cli/cmd/serve_at.rs` (`pod_core::files`);
     the groups and the lead of the help (`pod-manual`); the request parser of `crates/podnet-core/src/util/http.rs`.
   - Ask podcrates (a part is missing): the SOCKS5 and CONNECT handshake on a stream that the caller holds (the hops
     of `crates/podnet-netns/src/upstream.rs` and `src/link_chain.rs`); the check of a user by a callback in the
     SOCKS5 and HTTP server of `pod-proxy` (`crates/podnet-netns/src/socks.rs`; podssh has a server too); a DNS answer
     with an RCODE (`crates/podnet-netns/src/dns.rs`); rows of `pod-hostprobe` for the probes of the pod backends; the
     downloaders and the lookup of the newest tag of the installer core (`scripts/install.sh`); a WebSocket request
     with no token header (`src/doctor/lane_probes.rs`).
   - Keep (podnet-specific): the lock of the fake-DNS file, the reader of `resolv.conf` of `src/link_dns.rs`.
   - Not a copy: `scripts/retry.sh` had 3 tries, the copy of podcrates 5; podnet took the copy of podcrates
     v0.69.0 (2026-11-17).
4. Done on 2026-11-17: src/transport/ removed (commit c3fb7da6; nothing called it). The asks went to podcrates as
   COORD-105 (GitHub #652). Their answer (PodUtils/podcrates#221, 2026-11-17): their entries 896 (the handshake of a
   hop on a stream of the caller), 897 (the check of a user by a callback; one server for podnet and podssh), 898 (the
   RCODE answers), 899 (the probes of the doctors; the user and net namespace row and the row of `process_vm_writev`
   are in their next tag), 900 (the downloaders and the newest tag), 901 (the upgrade request with no token).
   `pod_core::lock::FileLock` (their entry 877) and `pod_core::clock::Clock` (their entry 878) are on their `main`
   for the next tag. Keep the lock of the fake-DNS file and the reader of `resolv.conf`.
5. 2026-11-21: the date code takes `pod_core::clock` (podcrates `v0.80.0`): `civil_from_days` and `days_from_civil`
   in `src/router/door_limit.rs` and `crates/podnet-serve/src/files/listing.rs`, and `parse_http_date` behind
   `podnet::doctor::parse_imf_fixdate` (`src/doctor/clock_check.rs`; the name stays in the API). The shared parser
   refuses a day, an hour or a zone out of range, which the copy took as a date: the test `the_date_header_gives_the_skew` of
   `src/doctor/tests.rs` has three such rows (`ci` 44846557642 passed on `0948be4`). The retry waits of `src/cli/cmd/warp/run.rs` and `warp/tun.rs` take
   `pod_core::backoff::backoff` (the same band of 1 s to 30 s, with jitter; the log field is `next_try_ms`; `ci` 44850063694 passed on
   `00ceb2f`). The
   `Backoff` of `crates/podnet-serve/src/relay/supervise.rs` stays: its tests give it short waits, and it starts again
   after a stable session. The file writes: `pod_core::files` of `v0.80.0` keeps private files in a list of
   directories and has no replace of a file at a path, so the claim of the survey does not hold. The two copies of a
   replace (`write_atomic` of `crates/podnet-netns/src/report.rs`, `save` of `src/warp/ssh_config.rs`; the second has
   no `create_new` and no `sync_all`) are an ask: COORD-112 (GitHub #677). `write_private` of
   `src/cli/cmd/serve_at.rs` makes a new file in a private directory: not a copy. podcrates `v0.81.0` answered
   COORD-112 (PodUtils/podcrates@T-1066): both copies take `pod_core::files` (T-505). What remains
   (`pod_core::files`), the CONNECT client of `crates/podnet-core/src/connect.rs`, the HTTP head readers, the request
   parser of `crates/podnet-core/src/util/http.rs`, and the groups and the lead of the help.

## Prove

```sh
sh scripts/agent/podcrates.sh
```

## Done

2026-11-23, commits 3611d22b, ef372745, 89b075e4 and 001b0bd7 (podcrates `v0.82.0`). The last four copies take the
shared code; `pod_core::files` was taken by T-505.

- The head readers: the answer of the tailscaled local API (`crates/podnet-netns/src/tailscale.rs`) takes
  `pod_proxy::http::read_head_exact` (64 KiB, no byte past the empty line) and `parse_head`. The control socket of the
  router (`src/router/control.rs`) takes `pod_proxy::http::read_request` (16 KiB, was 8 KiB; the strict parser of
  `pod_core::http1`); a refused head gets its status (400, 431 or 505, was 400). Its client takes
  `pod_proxy::http::read_response`.
- The request parser: `RequestHead::parse` of `crates/podnet-core/src/util/http.rs` takes `find_request` and
  `parse_request` of `pod_core::http1`. It now takes a bare LF and empty lines first, and refuses a bare CR, a control
  byte, a folded line, a space before a colon, a line with no colon and another version of HTTP. `HttpParseError` has
  `TooLarge` and `Malformed { status, why }`. It has no caller in the binary.
- The CONNECT client: `ssh-relay --connect-proxy` dials with `pod_proxy::Dialer` and the proxy of the flag
  (`podnet_core::connect::dialer_via`): Basic, or Digest when the proxy asks; a refusal keeps its status, its reason and
  the first line of its body. `connect_tunnel` and `connect_tunnel_with_auth` keep their API (the parity checklist) on
  `pod_proxy::blocking`, on a thread of their own (a call from a tokio task does not panic). A refusal stays
  `ConnectionRefused`. The head limit is 16 KiB (was 8 KiB). No byte past the head is read, so the bytes of the tunnel
  stay in the socket (`leftover` is empty).
- The help: the groups and the "Start here" lines are `podnet::manual::GROUPS` and `LEAD`, and the model of
  `pod-manual` has them as `Command::groups` and `Command::lead` of the root. The man page, the JSON and
  `llms-full.txt` show them; the top help of clap renders the same data, with the same text. `pod_manual::check::help`
  runs in `each_text_of_the_manual_keeps_the_rules`.
- Tests (each a table for its class): `the_head_reader_stops_at_the_blank_line_and_names_each_fault` (podnet-netns),
  `route_control_answers_each_bad_head_with_its_status`, `the_shared_reader_takes_and_refuses_each_kind_of_head`,
  `connect_tunnel_takes_each_answer_of_the_proxy` (podnet-core; also from a tokio runtime). `cargo test --test
  doctor_ssh_relay` (8), `--test ssh_relay` (4), `--test router_control` (4), `cargo test -p podnet-serve relay` (37),
  the help and manual tests of the binary (20) pass. The block "Start here" of `podnet --help` is pinned as text.

# T-474: `ssh-relay` ignores `--proxy`, `--egress`, `--resolve` and the proxy variables

**Source:** the survey of T-473 (2026-11-17), finding 2: `connect_address` of `crates/podnet-serve/src/relay.rs`
connected directly when `--connect-proxy` was not given.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`docs/relay.md` says that `--proxy`, `--egress` and `--resolve` apply to `podnet ssh-relay` as to the other commands.
With no `--connect-proxy`, the relay connected with `TcpStream::connect` and ignored each of them. On a host whose only
way out is a proxy, `ssh-relay` could not reach the relay.

## Premise

- `podnet::egress::egress(None)` is the dial of the process: the proxy, the egress and the resolver of the flags and
  of the environment.

## Approach

1. `RelayConfig::dial` (`crates/podnet-serve/src/relay.rs`) takes the dial of the process; `ssh-relay` and the doctor
   row give it. `--connect-proxy` keeps its path. `PrefixedStream` holds any stream.

## Prove

```sh
cargo test -p podnet --test ssh_relay -- ssh_relay_takes_the_proxy_of_the_process
```

## Done

2026-11-17, commit 6f54eca9. The test runs `ssh-relay --proxy http://127.0.0.1:PORT --server relay.invalid:443`
against a recording CONNECT proxy that joins a fake relay: it passes, and the proxy saw `CONNECT relay.invalid:443
HTTP/1.1`. On the code before the fix it fails (the direct connect to `relay.invalid`). `cargo test --test ssh_relay`
(4), `cargo test --test doctor_ssh_relay` (8) and `cargo test -p podnet-serve relay` (37) pass.
The test: `ssh_relay_takes_the_proxy_of_the_process` (`tests/ssh_relay.rs`).

# T-475: `warp up` reads its own list of 4 system CA bundles; the manual and `pod_tls` have 6

**Source:** the survey of T-473 (2026-11-17), finding 12.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`SYSTEM_BUNDLES` of `src/warp/up.rs` has 4 paths in its own order. The manual (`src/manual/facts.rs`) documents the 6
paths of `pod_tls::SYSTEM_BUNDLES`. On a host whose bundle is only at one of the 2 other paths, `warp up` finds none.

## Premise

- `pod_tls` (`trust.rs`) has the list of 6 and a count of the PEM certificates (`bundle.rs`).

## Approach

1. `warp up` takes `pod_tls::SYSTEM_BUNDLES` and the count of `pod_tls`; `ca_fix` (the merged bundle for WARP) stays.
2. A test checks that each path that the manual names is a path that `warp up` reads.

## Prove

```sh
cargo test -p podnet -- warp::up
```

## Done

2026-11-17, commit fa3092d3. `SYSTEM_BUNDLES` of `src/warp/up.rs` is now `pod_tls::SYSTEM_BUNDLES` (6 paths). The count of
the certificates stays (a count of the PEM markers; the bundles of `ca_fix` need no parse). Test:
`src/warp/up/tests.rs::the_system_bundles_are_the_bundles_of_the_manual` reads the paths of the manual
(`src/manual/facts.rs`) and compares them with the list, in order. It passes; with the old list of 4 it fails (the
manual has 6). `cargo test -p podnet --lib warp::up`: 17 passed.


# T-476: `transfer_over_the_relay_crosses_the_cap` fails under load: a session meets the cap before its move

**Source:** `sh scripts/agent/check.sh` on 2026-11-17 (the full `cargo test --workspace`, 17:42 UTC).
**Category:** defect
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

`tests/transfer_relay.rs::transfer_over_the_relay_crosses_the_cap` failed once in the full run: a relay session carried
1049609 bytes, 1033 over the cap of 1 MiB of the fake relay. The file landed with the right SHA-256 (the resume after
the close of 1009 carried it). Alone, the test passed 3 times of 3.

## Premise

- The fake relay counts the frame that crosses the cap, then closes the session with 1009
  (`tests/relay_reverse/fake.rs`, `count`). So the failure means that the move to a new leg came after the cap.
- In the test the receiver moves at 256 KiB (`move_bytes`), a margin of 768 KiB. The real relay: a cap of 64 MiB and
  `MOVE_BYTES` 48 MiB, a margin of 16 MiB. When the old leg carries bytes while the new leg opens, a slow open on a
  loaded host or a fast flow can meet the cap.

## Approach

1. Measure the bytes that the old leg carries between the decision of the move and the switch, under load.
2. If the old leg carries bytes during the open of the new leg: stop the writes on the old leg at the margin, or start
   the open of the new leg earlier (a margin in time and in bytes). Else make the test hold the property that the code
   guarantees, and name the guarantee.
3. The test runs 20 times under a parallel load and passes each time.

## Prove

```sh
cargo test -p podnet --test transfer_relay -- transfer_over_the_relay_crosses_the_cap
```

## Blocker

`resume::move_under` of `pod-session` (COORD-126, #756, to podcrates). The test moves at 160 KiB until the tag.

## Correction

2026-11-23, H-19 (`docs/research/podnet/hard-problems-results.md`): worst-case bytes before a move are `move_bytes + PAIR_WINDOW (768 KiB) + 2 * CHUNK (64 KiB) + 64`. The default 48 MiB move under a 64 MiB cap has 15.2 MiB of room, while the test's 256 KiB move under a 1 MiB cap overshot by 65,600 bytes. `tests/transfer_relay.rs` now moves at 160 KiB (`<= 191 KiB`, leaving 32,704 bytes of room) and passes `transfer_over_the_relay_crosses_the_cap`. Adding `resume::move_under(cap, window)` in `pod-session` remains the follow-up.

# T-477: A refusal for now (HTTP 429, exit 75) gets the hint "does not change with a retry"

**Source:** `live` 44140229441 (2026-11-17): the quick-tunnel service answered HTTP 429 with `Retry-After: 3600`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The error line of the refusal was right, but its second line was the hint of the hop `edge-register`: "a refused
token or tunnel does not change with a retry; check the token, or use a quick tunnel". The tunnel was a quick tunnel,
and a retry after the wait works. The line was not true (the ruling "Quality first": one true line).

## Premise

- `error_line` (`src/report.rs`) chose the hint by the hop alone, not by the class of the failure.

## Approach

1. A failure of the class TempFail (exit 75) gets its own second line: try again after the time that the service asks
   for. Other classes keep the hint of their hop.

## Prove

```sh
cargo test -p podnet --lib -- the_hint_follows_the_class_of_the_failure
```

## Done

2026-11-17, commit b0c1ec3a. The table test (`src/report.rs`, `the_hint_follows_the_class_of_the_failure`: TempFail and
Refused at the stage `EdgeRegister`) passes. On the code before the fix it fails for TempFail. The lines of the job
log that came twice are of the workflow (`scripts/live-run.sh` writes stderr to the log and to the file, and the step
prints the tail of the file), not of podnet.

# T-478: Two parallel SSH sessions through one relay node: one hung with no line

**Source:** `live` 44147054834 (76f7031, 2026-11-17), the step "SSH through the reverse relay".
**Category:** defect
**Milestone:** M6
**Priority:** P1
**Effort:** M
**Status:** open

## Problem

The first SSH session through `relay node` and `relay connect` worked (`relay-ssh-ok`). Then two sessions at the same
time through the one node: the step waited until its limit of 5 minutes, with no line. The same step passed in
`live` of da53b1a 20 minutes before. A wait with no limit and no line breaks the ruling "Quality first".

## Premise

- The two sessions had no `timeout` and no log of their own, so the run shows neither which session hung nor where.
- The node serves each session of an operator on its own stream of the relay.

## Approach

1. The step gives each parallel session a limit (60 s) and a debug log of `relay connect`, and prints the logs of the
   sessions and of the node when one fails (commit of this entry).
2. On the next failure: read which side waits (the node, the connect, the relay) and fix it there; each wait of the
   node and of the connect has a limit and a line.
3. A test with the fake relay (`tests/relay_reverse/fake.rs`): two sessions at the same time through one node, each
   ends within a limit, 20 times.

## Prove

```sh
cargo test -p podnet --test relay_reverse
```

## Correction

2026-11-17: step 3 is written: `tests/relay_reverse.rs::two_sessions_at_the_same_time_through_one_node` (two sessions
at the same time through one node to a TCP echo, each within 20 s). It passed 20 rounds of 20: it does not reproduce
the hang. Each session gave its answer in 3 to 46 ms and ended after 15 s: the fake relay sends no Close when the
node ends a session, so the operator ends at `EOF_IDLE` of `pod-relay` (15 s of no byte after the end of the input,
by design, podnet issue 109). The real relay sends the Close: the first ssh of the live step ended in about 5 s. The
test keeps 3 rounds. The cause of the hang waits for the logs of step 1.
2026-11-17, the cause is most likely T-406 (the relay keeps the lost socket of a node after a reset): in the first
case of T-406 an SSH session through the node stopped in the key exchange, as here. Keep the logs of step 1: the next
failure shows the 409 lines of the node, or not.


# T-479: Tests free a port to get a refused address; another test can take the port

**Source:** `ci` 44200604223 (793f55f, 2026-11-17), job `features`:
`relay::tests::the_supervisor_tries_again_when_the_relay_is_not_there` failed with "SSH session failed: handshake
failed: Connection reset by peer".
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A test bound a port, read it and dropped the listener, to get an address where each connect is refused. Tests run in
parallel: another test (a fake relay) bound the freed port. The connect then reached that stranger, and the test saw
a reset handshake in place of a refused connect.

## Premise

- A socket that is bound and does not listen gives a refused connect and keeps the port: no other socket can take it.
- `pod_testkit::listener::free_port` (PodUtils/podcrates) frees its port too; it has no helper of a refused port.

## Approach

1. The three cases of `crates/podnet-serve/src/relay/tests.rs` keep a socket bound with no listen (commit b4154e4d).
2. Ask podcrates (COORD-106, GitHub #653) for a helper of a refused address in `pod-testkit` (one copy for each project, the ruling "Shared
   code first"); then replace the `dead_port` helpers of `tests/commands.rs`, `tests/warp_stand_in.rs` and
   `tests/doctor_ssh_relay.rs`.

## Prove

```sh
cargo test -p podnet-serve --lib -- relay
```

## Correction

2026-11-17: step 1 is done in commit b4154e4d: `cargo test -p podnet-serve --lib relay`, 37 passed; the failing test passed 3
times of 3. The failure was not reproduced here (it needs the other test on the same port at the same time); the
cause follows from the reset in place of the refusal. Step 2 is open.

## Done

2026-11-18, the commit of T-483 (podcrates `v0.71.0`, PodUtils/podcrates@T-910). Each copy of a refused address takes
`pod_testkit::refused::refused()` (feature `refused`): `dead_port` and the port of the connect stage of
`tests/doctor_ssh_relay.rs`, and the three cases of `crates/podnet-serve/src/relay/tests.rs`. The guard keeps the port
bound with no listen for the life of the test. `cargo test -p podnet-serve --lib relay`: 37 passed;
`cargo test --test doctor_ssh_relay`: 8 passed. The helpers `free_port` of `tests/commands.rs` and
`tests/warp_stand_in.rs` give a port to listen on, not a refused address: they stay. podnet builds for Linux only, so
the gap of PodUtils/podcrates@T-936 (macOS) does not apply.


# T-481: Publish `v0.4.0-beta.17` for podrunner: the fixes of T-456 and T-457, and the QA round

**Source:** `docs/decisions.md`, 2026-11-18, podrunner first; the coordinator's call C782 (PodUtils/podcrates#227):
a fix that podrunner needs may go out in a beta under the release freeze of 2026-11-17. PodUtils/podrunner#213
(T-456) and #142 (T-457, podrunner's own T-179 waits for the next podnet release) are fixed on `main` and in no release.
**Category:** release
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.16` was tagged before the fixes of T-456 and T-457. podrunner pins a release of podnet, so the fixes do
not reach its units.

## Premise

- The steps are `docs/maintenance.md`, "Release". Step 0 (the gate of T-450 to T-455) does not hold; call C782 lets
  this beta out with fixes only. The notes say what the freeze leaves open.
- The beta carries the fixes of the QA round (GitHub #609 to #648) and the take of podcrates `v0.70.0` (T-480).

## Approach

1. Set the version `0.4.0-beta.17` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`), write the llms files again.
2. Write `docs/releases/v0.4.0-beta.17.md`, with `## Breaking`.
3. Push; wait until `ci` and `live` pass on that commit.
4. Dispatch the dry run of `release.yml`; then `publish=v0.4.0-beta.17`; then check `release-proofs.yml`.
5. Tell podrunner (PodUtils/podnet#654, and a state line on PodUtils/podnet#649).

## Prove

```sh
gh release view v0.4.0-beta.17 -R PodUtils/podnet
```

## Correction

2026-11-18. The dry run 44295134663 on `60db06b` failed in the gate: the gate takes podlab of the podcrates tag of
`Cargo.toml` (`v0.70.0`), whose new profile `bind-netns-vm-denied` refuses each network namespace by design. The row
`pod-netns` got exit 3 with the reason, and `refusal_expected` of `scripts/lab-lane.sh` had no line for the profile.
The line is added. The second line of that refusal is T-482. `live` on `60db06b` passed at its third attempt: attempt 1
failed at the public WARP endpoint (T-414), attempt 2 at a POST of 1 MiB through a new quick tunnel (HTTP 530 of the
edge, the class of T-138).

## Done

2026-11-18, the release `v0.4.0-beta.17` (tag at commit `cfc0924`). `ci` 44298151244, `live` 44298151291 and the dry run
44298164375 (with the gate) passed on `cfc0924`; the publish dispatch 44302346584 made the tag; the release run
44302556124 and `publish` 44302560923 passed; `release-proofs` 44306857480 passed (the published files, SHA256SUMS,
provenance, the signature, `podnet update`, the rebuild of x86_64). podrunner is told in COORD-108 (PodUtils/podnet#655).


# T-482: `pod --backend netns` prints a second, false cause after a refused network namespace

**Source:** the release gate of `v0.4.0-beta.17` (release run 44295134663, the row `pod-netns` of the profile
`bind-netns-vm-denied` of pod-lab `v0.70.0`), 2026-11-18. The ruling "Quality first" (2026-11-17): each failure gives
one true line.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Where `clone(CLONE_NEWUSER|CLONE_NEWNET)` is EPERM, `podnet pod --backend netns` prints the true cause and then a
second line that is not a cause:

```
podnet pod: unshare(CLONE_NEWNET) failed (EPERM) and unshare(CLONE_NEWUSER|CLONE_NEWNET) failed (EPERM)
podnet pod: write uid/gid maps: the child died before it named its namespace
```

## Premise

- Read: `crates/podnet-netns/src/netns/mod.rs:96` and `crates/podnet-netns/src/kernel.rs:60` add the context
  `write uid/gid maps` to the error of `pod_hold::netns::answer`, which says that the child died. The child died because
  its `unshare` failed; that line was printed first.
- The exit code (3) and the first line are right.

## Approach

1. When the child reported its `unshare` failure, end with that failure alone; do not print the error of `answer`.
2. A table test of the refusals of the netns backend: one line for each.

## Prove

```sh
cargo test --test pod_error_lines each_refusal_of_the_netns_backend_is_one_line
```

## Done

2026-11-20. `answer_child` (`crates/podnet-netns/src/netns/mod.rs`), for the netns backend and the netns-kernel
backend: when the answer of the parent fails, the parent reaps the child with no blocking wait; a child that ended with
exit 3 printed its cause, so the parent ends with 3 and no second line. A child that lives (the id maps failed) is not
waited for, and the error of the answer stays the cause. The table test `each_refusal_of_the_netns_backend_is_one_line`
(`tests/pod_error_lines.rs`) has three rows: the namespaces are refused (in `unshare -rU`, with
`/proc/sys/user/max_net_namespaces` and `max_user_namespaces` at 0), a program that does not exist (exit 3), and
`--inbound-dir` with netns (exit 64); each gives one line of podnet with its cause. It passes. Control: with the two
files of the commit before (touched), the first row fails with the second line `write uid/gid maps: the child died
before it named its namespace`. `ci` 44799344725 on `ac67710` passed.


# T-483: Take podcrates v0.71.0

**Source:** PodUtils/podcrates#228 (COORD-137, 2026-11-18): the tag `v0.71.0` at `6dff7fc`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.70.0`. The tag `v0.71.0` has `pod_testkit::refused` (T-479, COORD-106), `podlab run
--expect-exit` (T-452), and fixes of `pod-cloudflare` on a 32-bit target.

## Premise

- The tag has no hand-off patch for podnet. The installer core of the tag has the same SHA-256 as at `v0.70.0`.

## Approach

1. Each podcrates tag is `v0.71.0`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.
2. T-479 step 2: the copies of a refused address take `pod_testkit::refused`.

## Prove

```sh
sh scripts/agent/check.sh
```

## Done

2026-11-18, the commit of this entry. The tag `v0.71.0` (commit `6dff7fc`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `sh scripts/agent/check.sh` passes.

# T-484: The version line is a contract that podrunner reads

**Source:** PodUtils/podrunner@T-495 (githome's G-17, done 2026-11-18): `serve` puts the second word of
`podnet --version` in the `hello` of each source. The ruling "Schema ids" (2026-11-18): a change of an interface that
podrunner reads needs a new id and a COORD issue `(to all)`.
**Category:** docs
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The tests of podnet held only the start of the version line (`podnet `, `tests/pod_usage.rs`). A change of its form
would empty the field `podnet` of podrunner's `hello` with no failure here.

## Premise

- Read: `PodUtils/podrunner:crates/podrunner/src/serve/host.rs` (`version_word`: the second word of `podnet X (COMMIT)`).
- `src/main.rs` and `build.rs` print `podnet VERSION (COMMIT)`; `COMMIT` is `unknown` for a local build.

## Approach

1. A test of the form in `tests/contracts.rs` (the gate runs it on the release binary).
2. `docs/cli.md`, "The version line": the form, and the rule for a change.

## Prove

```sh
cargo test -p podnet --test contracts the_version_line_keeps_its_form
```

## Done

2026-11-18, the commit of this entry. `the_version_line_keeps_its_form` passes (version `0.4.0-beta.17`, commit
`unknown` here); it fails for a line with no commit, a second word that is not `CARGO_PKG_VERSION`, or no newline.

# T-485: Take podcrates v0.72.0

**Source:** PodUtils/podcrates#229 (COORD-138, 2026-11-18): the tag `v0.72.0` at `0ea12df`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.71.0`. The tag `v0.72.0` moves podrunner's status and source wire into `pod-job`, adds
`pod_core::http1` (one reader of an HTTP/1.1 head), and gives `pod_testkit::refused` a form for macOS.

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.71.0`.
- The replacement of podnet's readers of an HTTP/1.1 head by `pod_core::http1` is copy replacement: T-473. It waits
  behind the asks of podrunner.

## Approach

1. Each podcrates tag is `v0.72.0`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.

## Prove

```sh
sh scripts/agent/check.sh
```

## Done

2026-11-18, the commit of this entry. The tag `v0.72.0` (commit `0ea12df`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `sh scripts/agent/check.sh` passes.

# T-486: Take podcrates v0.73.0

**Source:** PodUtils/podcrates#231 (COORD-140, 2026-11-18): the tag `v0.73.0` at `1f1fd3f`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.72.0`. The tag `v0.73.0` adds three optional fields of `status.v1`, `pod-fake-github` and
`pod-fake-githome` for the CI of podrunner, and a longer `session.stall` line of the far end (PodUtils/podcrates@T-945).

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.72.0`.
- No test, workflow or script of podnet matches the text of `session.stall` (a search of `src`, `tests`, `crates`,
  `scripts` and `.github`).

## Approach

1. Each podcrates tag is `v0.73.0`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.

## Prove

```sh
sh scripts/agent/check.sh
```

## Done

2026-11-18, the commit of this entry. The tag `v0.73.0` (commit `1f1fd3f`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `sh scripts/agent/check.sh` passes.

# T-487: The image on ghcr.io: an attestation of its provenance, and its digests in the release notes

**Source:** the owner's ruling "ghcr.io" (2026-11-19), relayed by podrunner in PodUtils/podrunner#312 (COORD-102 of
podrunner): each project publishes its CLIs and assets on ghcr.io, signed or attested as the release assets are, in a
form that plain OCI tools read; the release notes name the digest of each pushed reference.
**Category:** release
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet publishes `ghcr.io/podutils/podnet:<tag>` since issue 160 (`FROM scratch`, the static binary at
`/usr/local/bin/podnet`, one image for each of the eight architectures under one manifest list). Two parts of the
ruling were missing: the image had no attestation, and the release notes did not name its digests. podrunner pins
podnet by ghcr.io digest beside the SHA-256 of the file.

## Premise

- Read: the job `image` of `.github/workflows/release.yml` and `images/podnet/Dockerfile`. The form is the form (2)
  of the ruling: `docker pull`, `podvm pull` and `crane export` take it with no other tool.
- Measured on 2026-11-19: the binary of `ghcr.io/podutils/podnet:v0.4.0-beta.17-amd64`
  (`sha256:7ea8d5d4...`) has the SHA-256 of `podnet-x86_64-unknown-linux-musl` in `SHA256SUMS` (`d0b72b9b...`).

## Approach

1. The job `image` gives the digest of the manifest list, attests its provenance in the registry (`actions/attest`,
   `push-to-registry`), checks the attestation (`gh attestation verify oci://`), and appends the section "The image,
   by digest" (the manifest list and each image of one architecture) to the notes of the release.
2. `docs/install.md`, "Container image": pin by digest, and check the attestation.

## Prove

```sh
python3 -I scripts/check-workflows.py
gh attestation verify oci://ghcr.io/podutils/podnet@sha256:DIGEST --repo PodUtils/podnet   # at the next release
```

## Correction

2026-11-19. The coordinator took podnet's form for each project (call C789, PodUtils/podcrates#236, COORD-142). Its
item 4 is new for podnet: after the push, the job `image` pulls each image of one architecture back by its digest and
compares its `/usr/local/bin/podnet` with the file of the release by SHA-256 (a manual check of
`v0.4.0-beta.17-amd64` matched on 2026-11-19). Its item 5: the hand-off COORD issue of a release names the digest of
the index beside the SHA-256 of each file. The digest lines of the job (`docker buildx imagetools inspect ...
--format '{{json .Manifest}}' | jq -r .digest`) give the digests of `v0.4.0-beta.17` that the registry API gives; the
pull and `docker cp` run first at the next release (no docker daemon here).

## Done

2026-11-19, the commit of this entry. `scripts/check-workflows.py`: 11 files, 0 faults. The new steps run only on a
tag; the first proof is the next release. The digests of `v0.4.0-beta.17` are in COORD-109.

# T-488: A quick tunnel with two edge connections gets HTTP 530 for some requests

**Source:** `live` 44444118882 (`e9bf11c`, 2026-11-19) and `live` 44286677011 attempt 2 (`60db06b`, 2026-11-18), the
step "Throughput through WARP, 1 to 16 MiB" (`scripts/live-throughput.sh`); the question of PodUtils/podcrates@T-931
(cloudflared opens one edge connection for a quick tunnel; podnet opens two).
**Category:** research
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

A quick tunnel through WARP registers its first connection (`msp01`) and, about 0.25 s later, connection 1 (`ord11`).
Some requests through the public URL get HTTP 530 from the edge: on 2026-11-19 the first POST of 1 MiB (1.4 s after
the first registration), and 2 of 20 requests of the latency check after the throughput passed. On 2026-11-18 the
first POST of 1 MiB, the same way. Each other request of the step passed.

## Premise

- `pod_cloudflare::Config` gives `ha_connections: 2` to each tunnel, a quick tunnel too (the answer to
  PodUtils/podcrates@T-931 in PodUtils/podnet#654). cloudflared opens one connection for a quick tunnel.
- Not measured: whether the edge sends a request of a quick tunnel to a connection that is not ready, or to a
  connection whose registration it does not keep for a quick tunnel.

## Approach

1. Run the step with `--ha-connections 1` and with 2, 20 times each, through WARP and direct; count the 530s.
2. If one connection gives no 530: a quick tunnel takes one connection by default, as cloudflared (a change of
   `pod-cloudflare`: a COORD issue `(to podcrates)`); a named tunnel keeps 2.

## Prove

```sh
sh scripts/live-throughput.sh target/release/podnet quic "1 4 16" warp
```

## Correction

2026-11-20: `live` 44712072334 on `ab0612d` (the take of podcrates `v0.79.0`, T-501) failed in the step "Throughput
of the HTTP path and of a --forward WebSocket", not in the WARP step: the quick tunnel registered on QUIC (iad10, then
connection 1 at iad23), and six GETs of its public name over 19 s each got HTTP 530. The tag changed the socket of
`pod-quic` (PodUtils/podcrates@T-990 clears `IPV6_V6ONLY`); the QUIC leg registered at once, so the edge had the
connection. The second run, `live` 44713609259 on `3f14a38` (the same code), passed: a flake, not a fault of the
take.
2026-11-23: `live` 45186065944 (schedule, `f63b449`) failed in the step "Throughput of the HTTP path and of a --forward
WebSocket": "latency http2+direct http: 2 of 20 requests FAILED". The capture of H-11 gave each answer: HTTP 530,
`cf-ray` `a5f0a38f2dc5a375-SEA` and `a5f0a3929924ba48-SEA`, the page "Origin DNS error | <tunnel>.trycloudflare.com".
The tunnel had registered at sea10 and sea09; the GETs and POSTs of 16 MiB before passed. The same code passed `live`
45179728390 on `3448b93`. The edge did not find the new quick tunnel for 2 requests: a fault of the edge.
2026-11-24: `live` 45309573101 (`9f3c8a2`, the release commit of `v0.4.0-beta.18`, T-533) failed in the same step: the
POST of 16 MiB over `http2+direct` got HTTP 530, `cf-ray` `a5f4e5494c61e3c6-IAD`, "Origin DNS error". The code of the
step is the code of `cca0b3d`, whose `live` 45289544910 passed. The failed job runs again.

# T-489: Take podcrates v0.74.0

**Source:** PodUtils/podcrates#237 (COORD-141, 2026-11-19): the tag `v0.74.0` at `dd228ef`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.73.0`. The tag `v0.74.0` changes two timings of podnet's link: a split session waits up to
500 ms for the `LOST` frame of the far end, so both ends name the same lost half (PodUtils/podcrates@T-950); the gate of
E2-1 at 20 % drop divides the medians of five runs (PodUtils/podcrates@T-949).

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.73.0`.
- The tests of these behaviours in podnet: `tests/link_split.rs` (the lost half) and `tests/link_udp.rs` (feature
  `udp`; the gate of E2-1 is a test of `pod-link`).

## Approach

1. Each podcrates tag is `v0.74.0`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.
2. Run `tests/link_split.rs` three times and `tests/link_udp.rs` with its feature.

## Prove

```sh
cargo test --test link_split
cargo test --features udp --test link_udp
sh scripts/agent/check.sh
```

## Done

2026-11-19, the commit of this entry. The tag `v0.74.0` (commit `dd228ef`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `cargo test --test link_split`: 8 passed, three times of three; `cargo test --features udp --test link_udp`: 11
passed. `sh scripts/agent/check.sh` passes.

# T-490: Take podcrates v0.75.0

**Source:** PodUtils/podcrates#238 (COORD-143, 2026-11-19): the tag `v0.75.0` at `930255d`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.74.0`. The tag `v0.75.0` changes the move of the relay layer: a client that moves the
layer holds the old link while the new one greets (PodUtils/podcrates@T-958). The wire does not change. That is the
area of T-476 (the cap test of the relay transfer fails under load).

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.74.0`.
- The gate (`gate.yml`) takes the podlab of the podcrates tag of `Cargo.toml`: a new tag can bring new profiles of
  the lane (T-481 found one).

## Approach

1. Each podcrates tag is `v0.75.0`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.
2. Run `tests/transfer_relay.rs` three times and the relay tests of `podnet-serve`.
3. Dispatch `gate.yml` on the head, so that a new profile of podlab shows before the next release.

## Prove

```sh
cargo test --test transfer_relay
cargo test -p podnet-serve --lib relay
sh scripts/agent/check.sh
```

## Done

2026-11-19, the commit of this entry. The tag `v0.75.0` (commit `930255d`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `cargo test --test transfer_relay`: 4 passed, three runs of three (`transfer_over_the_relay_crosses_the_cap`
among them); `cargo test -p podnet-serve --lib relay`: 37 passed. `sh scripts/agent/check.sh` passes.

# T-491: Take podcrates v0.76.0

**Source:** PodUtils/podcrates#239 (COORD-144, 2026-11-19): the tag `v0.76.0` at `fee6f8a`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.75.0`. The tag `v0.76.0` changes the move of the relay layer again: a client that moves the
layer holds the old link from the threshold on (PodUtils/podcrates@T-962). The wire does not change. The one breaking
change of the tag (`operator_told`) is for podssh; podnet does not use it.

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.75.0`.
- No code of podnet names `operator_told` (a search of `src`, `crates` and `tests`).

## Approach

1. Each podcrates tag is `v0.76.0`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.
2. Run `tests/transfer_relay.rs` three times and the relay tests of `podnet-serve`; dispatch `gate.yml` on the head.

## Prove

```sh
cargo test --test transfer_relay
cargo test -p podnet-serve --lib relay
sh scripts/agent/check.sh
```

## Done

2026-11-19, the commit of this entry. The tag `v0.76.0` (commit `fee6f8a`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `cargo test --test transfer_relay`: 4 passed, three runs of three; `cargo test -p podnet-serve --lib relay`: 37
passed. `sh scripts/agent/check.sh` passes.

# T-492: Tests free a port and then bind it; another test can take the port first

**Source:** `ci` 44498923514 (`a3386ee`, 2026-11-19), the job `features`:
`road::tests::the_pin_cookie_of_a_plain_tcp_listener_is_not_secure` failed with `--local 127.0.0.1:40811: Address
already in use (os error 98)`. The class of T-479 (a freed port is free for each other test of the run).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A test bound `127.0.0.1:0`, read the port, dropped the socket, and then gave the port to the local road. Another test
of the run bound the same port in that window, and the road got `EADDRINUSE`.

## Premise

- Read: the local road binds `tcp://HOST:PORT` and reports the bound address in `Published::url`
  (`crates/podnet-serve/src/road.rs`), so a test can listen on port 0.
- A search of `src`, `crates` and `tests` for a port that a test frees: `src/preflight.rs`
  (`a_closed_port_fails_with_its_address`, a freed port as a refused address: the class of T-479), and `free_port` of
  `tests/commands.rs` and `tests/warp_stand_in.rs` (a port for a child command to bind; `forward --name` must listen
  on the port of its target, so port 0 does not fit there).

## Approach

1. The road test listens on `tcp://127.0.0.1:0` and connects to the address of `Published::url`.
2. `a_closed_port_fails_with_its_address` takes `pod_testkit::refused`.
3. Later (P3): the users of `free_port` take a port that the child reports, where the command can report one.

## Prove

```sh
cargo test -p podnet-serve --lib the_pin_cookie
cargo test --lib a_closed_port_fails_with_its_address
```

## Correction

2026-11-19, steps 1 and 2 in the commit of this entry: `the_pin_cookie_of_a_plain_tcp_listener_is_not_secure` passes
three runs of three; `a_closed_port_fails_with_its_address` passes. Step 3 is open.

2026-11-24, step 3 in part: `proxy` of `tests/warp_stand_in.rs` gives `warp proxy` the listener `tcp://127.0.0.1:0`
and reads the port from its line `podnet: warp proxy  socks5h://127.0.0.1:PORT` (a thread reads stdout to its end).
`cargo test -p podnet --test warp_stand_in through_the_edge`: 6 passed, three runs of three. Open: `warp forward -L`
and `warp up --port` of the same file, and `free_port` of `tests/commands.rs` (`forward --name` listens on the port
of its target).

## Done

Step 3, 2026-11-25, the commit of this entry: `tests/warp_stand_in.rs`
`warp_stand_in_forward_carries_a_local_port_through_the_edge` binds `-L 127.0.0.1:0=...` and reads the port of the
listen line of `warp forward` (3 runs of 3 pass). The other users of a freed port need a fixed port by the meaning of
the test, and the command reports no port before its work: the config file of a reference server
(`tests/quic_tuic_reference.rs`, `tests/quic_juicity_reference.rs`), a name forward on the port of its target
(`tests/commands.rs`, on 127.0.0.2 and 127.0.0.3, which no other test binds), `warp up --port` (a port that the
pointer file names) and `offline discover listen --bind` (UDP; the command prints only its last JSON).

# T-493: Take podcrates v0.77.0

**Source:** PodUtils/podcrates#240 (COORD-145, 2026-11-19): the tag `v0.77.0` at `7cbe674`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.76.0`. The tag `v0.77.0` adds the crate `pod-release` (a signed release index, its
verifier, the self-update of a binary: PodUtils/podcrates@T-880), changes two calls of `pod-extract` (for podrunner
and podvm), and moves the second Nostr relay of the pairing to `nos.lol` (PodUtils/podcrates@T-963).

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.76.0`.
- podnet builds against the changed `pod-extract` with no change of code.
- No code, test, workflow or script of podnet names `offchain.pub` (`docs/research/` names `nos.lol` already).
- The take of `pod-release` in place of `src/update.rs` and `PodUtils/podnet@749da3d:src/update/cosign.rs` is T-494.

## Approach

1. Each podcrates tag is `v0.77.0`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.

## Prove

```sh
sh scripts/agent/check.sh
```

## Done

2026-11-19, the commit of this entry. The tag `v0.77.0` (commit `7cbe674`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `sh scripts/agent/check.sh` passes.

# T-494: Take `pod-release` in place of podnet's own update

**Source:** PodUtils/podcrates#240 (COORD-145): `pod-release` (PodUtils/podcrates@T-880) holds the update of a running
binary, from podssh's `update` with podnet's strict checks; PodUtils/podcrates@T-966 holds the adoption trial and the
take. The ruling "Shared code first" (2026-11-17).
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`src/update.rs` and `PodUtils/podnet@749da3d:src/update/cosign.rs` are podnet's own copy of the update of a binary (the check of SHA256SUMS,
of the cosign bundle and of the certificate identity; the trial run). `pod-release` now holds the same capability for
each project.

## Premise

- The Done section of PodUtils/podcrates@T-880 lists each part of podnet that goes, and each call that takes its place.
- The rulings "podrunner first" (2026-11-18): this is podnet's own work. It waits behind the asks of podrunner.
- The ruling on `podnet update` (`CLAUDE.md`, Decisions): SHA256SUMS, the certificate identity and the
  signature of the cosign bundle are checked with own code; the Rekor inclusion proof is not. `pod-release` must keep
  that, or the decision changes.

## Approach

1. Read the trial of PodUtils/podcrates@T-966 and the Done section of PodUtils/podcrates@T-880.
2. Take the crate; remove the copy; the tests of `podnet update` (`tests/`, the `update` step of `live.yml`) stay green.

## Prove

```sh
cargo test --workspace update
```

## Done

2026-11-23, the commit of this entry. The ruling of 2026-11-23 (`docs/decisions.md`, `podnet update`, the owner):
podnet takes the shared verifier of `pod-release`, with the check of Rekor; podnet keeps no own verifier.

- `pod-release` (tag `v0.82.0`, feature `fetch`) is a dependency of the root crate. `PodUtils/podnet@749da3d:src/update/cosign.rs` and
  `PodUtils/podnet@749da3d:src/sigstore-fulcio.pem` are removed. Of `src/update.rs`, `verify_bundle`, `sum_of`, `atom_tags`,
  `newest_in_atom`, `version_order`, `choose`, `Release`, `plan`, `trial`, `install`, `rollback`, `same_file`,
  `keep`, `sync_dir` and the test hook `AFTER_TRIAL` are removed. Their places: `pod_release::verify_release` with
  `Identity::github("PodUtils/podnet", "release.yml", TAG)` and `Roots::sigstore()`, `Verified::check`,
  `version::{valid_tag, tags_of_api, tags_of_atom, newest, release_target, asset}`, `Version`,
  `update::plan`, `update::install` with `trial::LIMIT`, `keep::rollback`, `keep::prev_of`, and
  `replace::running`. `rustls-webpki` is no longer a direct dependency.
- podnet keeps its own fetch (`crate::fetch`: its TLS roots, its proxy, its `--resolve` pins, its error lines),
  `run`, `Outcome`, the `PODNET_REPO` warning and the lines of each step. Each flag stays: `--check`,
  `--version TAG`, `--force`, `--rollback`. Each failure exits 69; a text that is not a tag exits 64.
- Changes of behaviour: a bundle with no entry of Rekor, or with an entry or a timestamp that does not verify, is
  refused. A tag is `v` and a version of semver with no build part (`0.3.0`, `v1`, `v1.0.0+b` now exit 64). A
  binary that is newer than the newest release says so (`Outcome::Newer`). The new binary takes the mode of the
  old binary with no write for others and no set-user-ID bit. A `podnet.prev` that cannot be written gives a
  warning, not a failure.
- Tests (`src/update/tests.rs`, 12): `the_entry_of_rekor_and_the_timestamps_are_checked` (a table of six changes
  of the bundle of `v0.4.0-beta.1`: no promise and no inclusion proof, a changed promise, a changed hash and root
  of the inclusion proof, an unknown log, a changed RFC 3161 timestamp). Each change passed the verifier before
  this commit (measured with the removed `cosign.rs`) and is refused now. The bundle tests, the trial table (with
  a file that another file replaces during its trial), the rollback, the plan table, the tags, the lists of the
  API and of the feed, and `the_kept_binary_is_the_fallback_of_the_supervisor`. The real bundles of
  `v0.4.0-beta.17` (accepted) and `v0.3.0-beta.1` (refused: the leaf names Azathothas/cfrs, so the `live.yml` grep
  holds) were checked by hand with the new verifier.
- `log_filter` of `crates/podnet-core/src/report.rs` names `pod_release`
  (`report::tests::the_log_filter_names_each_pod_crate_of_podnet`).
- `cargo test --workspace update`: passes. `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers,
  claims, doc-commands, scenarios, install and workflows give 0. `cargo test --workspace --no-fail-fast` with
  `TMPDIR=/tmp`: each target passes on this host except four that need what the host does not have: the network
  lab and `link_p2p_namespaces` (no `ip`), `pod_seccomp::kernel_listen::seccomp_kernel_socket_refuses_listen_v6`
  (no IPv6), and the timing test `defer::the_generator_reaches_the_cache_with_the_times_of_the_profile` (passes
  alone three times). `cargo clippy --workspace --all-targets --features full -- -D warnings`: passes.

# T-495: Take podcrates v0.78.1

**Source:** PodUtils/podcrates#241 (COORD-146, 2026-11-19): the tag `v0.78.1` at `edbc86b`. The tag `v0.78.0` has no
GitHub release (its run stopped after the signature, PodUtils/podcrates@T-974); `v0.78.1` holds its content.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.77.0`. The tag `v0.78.1` has the TLS end of `pod-broker`, the static command `pod-release`,
and three small breaking changes (`pod_broker::Tunnel`, `pod_release::update::plan`, `pod_release::verify`).

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.77.0`.
- No code of podnet uses `pod_broker` or `pod_release` (a search of `src`, `crates` and `tests`): the three breaking
  changes do not reach podnet. The take of `pod-release` is T-494.

## Approach

1. Each podcrates tag is `v0.78.1`: the manifests, both lock files, `scripts/install/SOURCES` and `issue-sync.yml`.

## Prove

```sh
sh scripts/agent/check.sh
```

## Done

2026-11-20, the commit of this entry. The tag `v0.78.1` (commit `edbc86b`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`. podnet builds against the tag with no change of
code. `sh scripts/agent/check.sh` passes.

# T-496: `pod`: a failed connect of the program gives no line at the default level

**Source:** T-452 (the error rows), measured 2026-11-20 with `0.4.0-beta.17` at `177336b4`.
**Category:** defect
**Milestone:** M7
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

In a pod, each name resolves (the upstream resolves it). A name with no answer, a reject rule and a refused target all
reach the program as `ECONNREFUSED` (or a reset on netns-kernel): `curl` says "Couldn't connect" (exit 7), not "Could
not resolve" (exit 6). podnet printed the cause only with `-v`. On netns-kernel the warning of a dead upstream said
"fails with ECONNREFUSED", but the program sees a reset.

## Premise

- `pod -x direct -- curl http://nowhere.invalid/` and `pod -x direct -r domain:lab.test=reject -- curl
  http://lab.test/` printed only the line of curl, on seccomp, netns and netns-kernel.
- The three upstream paths (`connect.rs`, `netns/backend.rs`, `kernel_serve.rs`) and `rules::note_refusal` printed
  their line only with `verbose`.

## Approach

1. One line for the first failure of each target at the default level (at most 64 targets, then one line that names
   `-v`); `-v` prints each connect; `podnet -q` prints none (`crates/podnet-netns/src/notice.rs`).
2. No second line for a cause that is already named: a failure through an upstream that the warning at the start named
   as dead, and a direct dial that the target refused (the program sees `ECONNREFUSED`, the true cause).
3. The warning of a dead upstream says "is reset" on netns-kernel.

## Prove

```sh
cargo test --test pod_error_lines
cargo test -p podnet-netns notice
```

## Done

2026-11-20. The test `each_failed_connect_names_its_cause_once` (`tests/pod_error_lines.rs`) runs four rows (a name
with no answer, a reject rule, a dead upstream, a refused connect on the loopback) on each backend of the host
(seccomp, netns, netns-kernel here): 12 rows pass. Control: with `crates/podnet-netns` of `177336b4` (the new test
kept), 6 rows fail (no line for the name with no answer and the reject rule, on each backend). The unit tests
`each_target_gets_one_line_up_to_the_limit`, `only_a_direct_refusal_is_the_cause_of_the_program` (`notice.rs`) and
`only_a_dead_upstream_gets_the_warning` (`lib.rs`, the reset of netns-kernel) pass. `docs/pods.md` has the line.
`ci` 44664720872 on `0dc34d0` failed in the jobs `restricted-host` (the test bound a refused port with
`pod_testkit::refused`, and that host refuses each bind of AF_INET) and `no-loopback` (with the loopback down, the
refused row gets a true line `Network is unreachable`). The test now takes `127.0.0.1:1` (no bind), and the refused row
refuses only a second line that says "Connection refused". It passes here as it is, under
`scripts/restricted-host.sh`, and in `unshare -n` (the loopback down); the control still fails 6 rows. `ci` 44667637115 on `826475c` passed (13 of 13 jobs).

# T-497: The error lines of `tunnel`, `warp proxy` and `connect` name one cause two times (code of podcrates)

**Source:** T-452 (the error rows), measured 2026-11-20 with `0.4.0-beta.17` at `177336b4`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

1. `podnet tunnel --hello-world --proxy http://127.0.0.1:1` (a dead proxy) ends with exit 69 in 0.1 s, but its line
   names the request two times: `provisioning quick tunnel: request to the quick-tunnel service failed: POST
   https://api.trycloudflare.com/tunnel: POST https://api.trycloudflare.com/tunnel: api.trycloudflare.com:443 through
   the proxy ...`.
2. `podnet warp proxy --proxy http://127.0.0.1:1` ends with exit 69, but its line names the dead proxy once for each
   host of the WARP API (`the WARP API at https://api.devices.cloudflare.com (before: https://api.cloudflareclient.com:
   POST ...: could not reach the proxy 127.0.0.1:1 ...): POST ...: could not reach the proxy 127.0.0.1:1 ...`), and an
   INFO line names it a third time.
3. `podnet connect -L tcp://127.0.0.1:0 wss://127.0.0.1:1/__podnet/ws` ends with exit 69 at once, but its line says
   `could not connect to 127.0.0.1:1: connecting to 127.0.0.1:1: Connection refused`; a name with no answer says
   `could not resolve nowhere.invalid: connecting to nowhere.invalid:443: could not resolve nowhere.invalid: ...`.

## Premise

- 1: `pod_proxy::Client::send` names the request (`PodUtils/podcrates@edbc86b:crates/pod-proxy/src/client.rs`, line 335; the tag `v0.78.1`), and
  `pod_cloudflare::Host::send` names it again (`PodUtils/podcrates@edbc86b:crates/pod-cloudflare/src/host.rs`, `send` and `sent`).
- 2: the fallback of the hosts of the WARP API is in `pod-warp` (`pod_warp::api`).
- 3: `pod_ws::way::dial_error` puts the text of podnet's dial (`connecting to HOST:PORT: ...`) into a `DialError`
  whose Display of `pod-proxy` adds `could not connect to HOST:PORT: ` in front.
- Both crates are of podcrates; podnet takes them by the tag.

## Approach

1. Ask podcrates (COORD-111, GitHub #658): `Host::send` does not add the request when the failure already starts with it; the
   fallback of `pod-warp` names a cause once when each host failed for the same cause (the proxy).
2. Take the tag, and add the rows of `tunnel`, `warp proxy` and `connect` to the error rows of T-452.

## Prove

```sh
cargo test --test pod_error_lines
```

## Done

2026-11-20, the commit of T-501 (podcrates `v0.79.0`, PodUtils/podcrates@T-985, PodUtils/podcrates@T-986 and PodUtils/podcrates@T-987). The lines now: `POST
https://api.trycloudflare.com/tunnel: api.trycloudflare.com:443 through the proxy ...` (once); `the WARP API: each host
failed: 2 hosts (...): could not reach the proxy 127.0.0.1:1: ...`; `TCP to 127.0.0.1: could not connect to
127.0.0.1:1: Connection refused`. The rows `connect-refused`, `connect-no-name`, `connect-dead-proxy`,
`tunnel-dead-proxy` and `warp-proxy-dead-proxy` of `scripts/lab-errors.sh` pass with the binary of `v0.79.0`.
Control: with the binary of `v0.78.1`, `connect-no-name`, `tunnel-dead-proxy` and `warp-proxy-dead-proxy` fail (`the
part 'POST https://api.trycloudflare.com/tunnel' comes two times`, and the like). `connect` takes `--stdio` (no
listener: the labs `bind-denied` and `listen-denied` refuse a listener). Each lab of pod-lab `v0.68.0` that holds here,
and the row `testers`, passes the case `errors` as uid 65534.


# T-498: A direct dial that timed out names its step two times

**Source:** T-452 (the error rows), measured 2026-11-20 with `0.4.0-beta.17` at `177336b4`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet forward -L 127.0.0.1:PORT=192.0.2.1:80 --network direct` logs, after the time limit: `a forward did not reach
its target: connecting to 192.0.2.1:80: connecting to 192.0.2.1:80 timed out after 29s`. Each direct dial of podnet
that times out has the same words two times.

## Premise

- `failed` of `src/egress.rs` adds `connecting to HOST:PORT` to each direct failure. The `DialError::Timeout` of
  `pod-proxy` already starts with the step `connecting to HOST:PORT`.

## Approach

1. `failed` adds the words only when the error does not start with them.

## Prove

```sh
cargo test -p podnet --lib egress::tests
```

## Done

2026-11-20. The test `a_timed_out_dial_names_its_step_once` (`src/egress.rs`) passes; `cargo test -p podnet --lib
egress::tests::`: 6 passed. Control: on the `src/egress.rs` of `177336b4` with the new test, it fails with `left:
"connecting to h:1: connecting to h:1 timed out after 29s"`.

# T-499: `pod --backend netns`: no name resolves when `/etc/resolv.conf` is a mount of a deleted file

**Source:** T-452, the case `errors` of the lane in the lab `no-proxy-no-dns` of pod-lab `v0.68.0`, 2026-11-20.
**Category:** defect
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

In the lab `no-proxy-no-dns`, `podnet pod --backend netns -- PROGRAM` prints `warning: the program keeps the resolver
of the host (bind mount over /etc/resolv.conf: No such file or directory (os error 2))`, and each name of the program
fails with `Temporary failure in name resolution`: the pod resolves nothing, also a name that a rule refuses. The
seccomp and netns-kernel backends resolve the names in the same lab.

## Premise

- In the lab, `/etc/resolv.conf` is a bind mount of a deleted file (`/proc/self/mountinfo`:
  `/tmp/podlab-resolv.conf-PID//deleted /etc/resolv.conf`). Linux refuses a mount over a deleted mount point with
  `ENOENT`. A container whose resolv.conf was replaced while a bind mount kept the old file has the same state.
- The file has no `nameserver` line, so glibc asks `127.0.0.1:53`: the loopback of the namespace of the pod, where
  nothing answers. With a `nameserver` of another address, the UDP query goes into the TUN, and the pod answers it
  (`crates/podnet-netns/src/netns/packet.rs` checks the port 53 only); a query over TCP to that address is a flow of
  the upstream. So the case to fix is a host file with no nameserver, and DNS over TCP to a nameserver of the host
  file.
- `private_resolv_conf` (`crates/podnet-netns/src/netns/mod.rs`) is the one way of the netns backend to give the
  program its resolver. netns-kernel redirects port 53 in the kernel, so the same failure costs it nothing.

## Approach

1. A second path when the bind mount fails: the pod answers DNS at `127.0.0.1:53` in its namespace (a forwarder in
   the namespace to the gateway), and DNS over TCP at each nameserver of the host file, as netns-kernel does with its
   redirect.
2. The warning then names the second path, or the backend refuses with the cause when neither path works.

## Prove

```sh
cargo test --test pod_error_lines
LANE_DIR=/var/tmp/lane/out sh scripts/lab-lane.sh PODLAB PODNET no-proxy-no-dns
```

## Done

2026-11-20. `crates/podnet-netns/src/netns/loopdns.rs`: when the bind mount fails, the child binds UDP and TCP
`0.0.0.0:53` in the namespace and passes both sockets to the parent after the TUN (one byte `0` or `1` first); the
parent answers them from the fake DNS (`Asker::ask` for a datagram, `Asker::connect` for a connection). The warning
names the second path. The test `a_resolv_conf_of_a_deleted_file_still_resolves` (`tests/pod_error_lines.rs`) makes
the state of the lab in `unshare -rm` (a deleted file with no nameserver mounted over `/etc/resolv.conf`) and resolves
`lab.test` on each backend: it passes (netns, seccomp, netns-kernel here). Control: with `crates/podnet-netns` of the
commit before, it fails for netns (`Temporary failure in name resolution`). The case `errors` of the lane passes in
`no-proxy-no-dns`, `proxy-only`, `bind-denied` and `userns-off` as uid 65534 with pod-lab `v0.68.0`. `docs/pods.md`
has the second path. `ci` 44671079409 on `1413960` passed.

# T-500: `podnet pod --help --json` prints text, and the manual of `pod` has no flag of `pod`

**Source:** T-455 (the measure of the promises), 2026-11-20.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

1. `podnet pod --help --json` printed the text help; each other command prints the JSON of `man --json COMMAND`
   (`docs/cli.md`: "`podnet COMMAND --help --json` prints the JSON of `podnet man --json COMMAND` for one command, at
   each level").
2. `podnet man --json pod` (and the llms files) listed only the global flags of podnet for `pod`, and none of the 22
   flags of `pod` (`--proxy`, `--rule`, `--backend`, `--report`, ...). The global flags do not apply to `pod`.

## Premise

- `src/main.rs` gave the line after `pod` to podnet-netns before the answer of `--help --json`.
- `pod` has its own parser and help (podnet-netns, no clap); the command of clap in podnet is a stub that takes the
  rest of the line, so `pod_manual::clap::read` saw no flag of `pod`.

## Approach

1. `--help --json` is answered before the dispatch of `pod`; for `pod`, only the words before `--` count (the words
   after it are the program's).
2. The manual reads the flags of `pod` from its one source, the text of `podnet_netns::usage_for`
   (`src/manual/pod_flags.rs`).

## Prove

```sh
cargo test -p podnet --lib manual::pod_flags
cargo test --test cli llms::
```

## Done

2026-11-20. The tests `the_help_json_of_pod_has_its_flags` and `help_json_matches_man_json` (a row of `pod`) in
`tests/cli/llms.rs`, and `the_manual_has_each_flag_of_the_help_of_pod` (`src/manual/pod_flags.rs`: each flag of the
first column of the help is a flag of the manual, and the reverse) pass. Control: with `src/main.rs` and
`src/manual/mod.rs` of the commit before, both CLI tests fail (`valid JSON: Error("expected value", line: 1, column:
1)`; `["pod", "--help", "--json"] differs from ["man", "--json", "pod"]`). `podnet pod -x direct -- python3 -c ...
--help --json` still gives `--help --json` to the program. The llms files have the flags of `pod`.

`ci` 44701681199 on `1c7df7d` failed in `test`, `features`, `restricted-host` and `no-loopback`: with the flags of
`pod` in the manual, `the_manual_agrees_with_the_parser` compared them with the stub of clap (28 findings), and
`each_text_of_the_manual_keeps_the_rules` found two sentences of the help of `pod` with more than 25 words, and two
rules of the global flags (`--no-proxy`, `--quiet`). The local check had passed on a stale build (a control moved
`src/manual/pod_flags.rs` out and back with `mv`, which keeps its old time). The fix: the test takes the flags of
`pod` from its help, and the new test `each_flag_of_the_help_is_a_flag_of_the_parser`
(`crates/podnet-netns/src/cli_tests.rs`) proves that the parser of pod takes each of them; the two sentences are
split; the manual clears the rules of `pod`. A fresh check (each changed file touched) passes. `ci` 44707736066 on `4559204`
(the commit after the fix) passed.

# T-501: Take podcrates v0.79.0

**Source:** PodUtils/podcrates#245 (COORD-149, 2026-11-20): the tag `v0.79.0` at `68fbc96`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.78.1`. The tag `v0.79.0` has the answers to COORD-110 (#657: the key `defect_tests` of
`pod-todo`) and COORD-111 (#658: each error line of `pod-cloudflare`, `pod-warp`, `pod-ws` and `pod-proxy` names each
cause once), Rekor v2 in `pod-release`, and the clients of a job through `pod-broker`.

## Premise

- The tag has no hand-off patch for podnet (`adoption/podnet` has `commit` and `repo` only). The installer core has the
  same SHA-256 as at `v0.78.1`.
- The breaking change of `pod_release::sigstore::Roots` does not reach podnet (podnet does not use `pod_release`; T-494).

## Approach

1. Each podcrates tag is `v0.79.0`: the manifests, both lock files, `scripts/install/SOURCES` (the tag and the commit)
   and `issue-sync.yml` (the commit and `podcrates_ref`).
2. `TODO/todo.toml`: `defect_tests = "2026-11-17"` (T-450).
3. The rows of `connect`, `tunnel` and `warp proxy` in the case `errors` of the lane (T-497).

## Prove

```sh
sh scripts/agent/check.sh
cargo todo check
```

## Done

2026-11-20, the commit of this entry. The tag `v0.79.0` (commit `68fbc96`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`; the three installer files have the same SHA-256.
podnet builds against the tag with no change of code; no test of podnet pins an old text. `sh scripts/agent/check.sh`
passes. The key `defect_tests` holds: `cargo todo check` agrees, and refuses a planted done defect with no test name
(`T-498: a done defect names no test in its Done section`). `ci` 44707736066 on `4559204` passed.

# T-502: `forward` ends with a bare `Operation not supported` where the host refuses getsockname

**Source:** T-452, the case `errors` of the lane in the lab `getsockname-denied` of pod-lab `v0.68.0`, 2026-11-20.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

In the lab `getsockname-denied`, `podnet forward -L 127.0.0.1:PORT=TARGET` bound its listener and then ended at once
with the one line `podnet: Operation not supported (os error 95)`: no step, no address, and the forward did not run.

## Premise

- `src/cli/cmd/forward.rs` read the bound address for the line of the listener with `local_addr()?` (getsockname(2))
  for each TCP, UDP and DNS door, with no context and no second path. The listener works with no known address.
- `bind_dns` (`src/forwards/udp.rs`) needs the bound address: the TCP door takes the port of the UDP door.

## Approach

1. The line of a listener takes the address of the command line and the reason when getsockname is refused
   (`bound` in `src/cli/cmd/forward.rs`); the forward runs.
2. `bind_dns` names its step when getsockname is refused.

## Prove

```sh
cargo test -p podnet --bin podnet a_refused_getsockname_shows_the_asked_address
LANE_DIR=/var/tmp/lane/out sh scripts/lab-lane.sh PODLAB PODNET getsockname-denied
```

## Done

2026-11-20. The test `a_refused_getsockname_shows_the_asked_address` (`src/cli/cmd/forward.rs`) passes. In the lab
`getsockname-denied` as uid 65534: `podnet: forward  127.0.0.1:17411 (the host refused getsockname: Operation not
supported (os error 95))  ->  127.0.0.1:1  via direct`, exit 0 at the end of `--run-for`; the rows `forward-*` of the
case `errors` pass there. Control: with the binary before the fix, the three rows `forward-*` fail in that lab (`forward
did not start: podnet: Operation not supported (os error 95)`). `ci` 44737373446 on `79b8331` passed.

# T-503: `pod --backend netns`: the second path of the resolver does not answer a loopback nameserver of the host

**Source:** `gate` 44742237281 on `c9e394a` (#668), the case `errors` of the lane on the GitHub runner, 2026-11-20.
**Category:** defect
**Milestone:** M7
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

On the GitHub runner, in five labs (`root-unreadable`, `no-kvm`, `userns-mount-denied`, `no-tun`, `no-net-admin`), the
netns backend could not make its mounts private (`mount --make-rprivate /: Operation not permitted`), so the second
path of T-499 was on. No name of the program resolved for 20 s, and podnet printed no line: the rows `pod-no-name`
and `pod-reject` failed.

## Premise

- The resolv.conf of the runner is that of systemd-resolved: `nameserver 127.0.0.53`, `options edns0 trust-ad`.
- T-499 bound UDP and TCP `0.0.0.0:53`. A datagram to 127.0.0.53 reached the socket, and the answer went out from
  127.0.0.1. glibc takes an answer only from the address that it asked: it dropped each one.
- The test of T-499 used a file with no nameserver (glibc asks 127.0.0.1), so it did not show this.

## Approach

1. Bind UDP and TCP port 53 on 127.0.0.1 and on each loopback nameserver of the host file (`127.0.0.53`, `::1`); the
   answer comes from the address that was asked.
2. The test of T-499 runs both host files.

## Prove

```sh
cargo test -p podnet-netns --lib loopdns
cargo test --test pod_error_lines a_resolv_conf_of_a_deleted_file_still_resolves
```

## Done

2026-11-20. `crates/podnet-netns/src/netns/loopdns.rs`: one pair of sockets for each address of `addresses` (127.0.0.1
and each loopback nameserver of the host file); the child sends their count, then the pairs. The unit test
`the_pod_answers_at_each_loopback_nameserver` and the test `a_resolv_conf_of_a_deleted_file_still_resolves`
(`tests/pod_error_lines.rs`, now a table of two host files: no nameserver, and `nameserver 127.0.0.53` with `edns0`)
pass. Control: with `crates/podnet-netns` of `c9e394a` (each file touched), the row of 127.0.0.53 fails for netns. A
deleted file of the runner's form mounted over `/etc/resolv.conf` in `unshare -rm`: `lab.test` resolves at once
(before: `Temporary failure in name resolution`, or no answer for 40 s with `edns0`). The case `errors` passes in each
lab here as uid 65534. `ci` 44782773504 on `3836971` passed. `gate` 44782773447 on `3836971` passed (the pod rows in the 5 labs of the runner).

# T-504: Take podcrates v0.80.0

**Source:** PodUtils/podcrates#250 (COORD-150, 2026-11-20): the tag `v0.80.0` at `62b8039`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.79.0`. The tag `v0.80.0` has three new crates (`pod-control`, `pod-unit`, `pod-beneath`)
and says that podnet needs no change of code.

## Premise

- The tag has no hand-off patch for podnet. The installer core has the same SHA-256 as at `v0.79.0`.
- The one breaking change (`pod_enter::Names::write_table`) does not reach podnet: no crate of podnet uses `pod-enter`.
- `pod-unit` (for `src/cli/cmd/autostart.rs`) and `pod-control` (a status socket) are optional; they are later work.

## Approach

1. Each podcrates tag is `v0.80.0`: the manifests, both lock files, `scripts/install/SOURCES` (the tag and the commit)
   and `issue-sync.yml` (the commit and `podcrates_ref`).

## Prove

```sh
sh scripts/agent/check.sh
```

## Done

2026-11-20, the commit of this entry. The tag `v0.80.0` (commit `62b8039`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`; the three installer files have the same SHA-256.
podnet builds against the tag with no change of code. `sh scripts/agent/check.sh` passes. `ci` 44803822897 on
`058059f` passed.

# T-505: Take podcrates v0.81.0

**Source:** PodUtils/podcrates#254 (COORD-151, 2026-11-21): the tag `v0.81.0` at `0707cfa`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.80.0`. The tag `v0.81.0` has the answer to COORD-112 (#677): one atomic replace of a file
at a path (`pod_core::files::replace` and `Replace`, PodUtils/podcrates@T-1066).

## Premise

- The tag has no hand-off patch for podnet, and says that podnet needs no change of code. Its breaking changes reach
  podrunner and podvm only. The installer core has the same SHA-256 as at `v0.80.0`.

## Approach

1. Each podcrates tag is `v0.81.0`: the manifests, both lock files, `scripts/install/SOURCES` (the tag and the commit)
   and `issue-sync.yml` (the commit and `podcrates_ref`).
2. The two copies of a replace take the shared one (T-473): `write_atomic` of `crates/podnet-netns/src/report.rs`
   (`replace(path, bytes, Mode::Private)`) and `save` of `src/warp/ssh_config.rs` (`Replace::new(Mode::Keep)
   .follow_link(true).make_dir(true)`). A commit whose directory sync failed gives one warning.

## Prove

```sh
sh scripts/agent/check.sh
```

## Done

2026-11-21, the commit of this entry. The tag `v0.81.0` (commit `0707cfa`) in each manifest, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES` and `issue-sync.yml`; the three installer files have the same SHA-256.
The two copies of a replace take `pod_core::files`: now each file is synced, its directory too, and the temporary file
of `warp ssh-config` is new (before: no `create_new`, no sync). `cargo test --lib warp::ssh_config` (9, with
`save_makes_a_private_file_and_keeps_a_link`), `cargo test -p podnet-netns --lib report` (6) and `cargo test --test
pod_report` (5) pass. `sh scripts/agent/check.sh` passes. `ci` 44871490037 on `9a9a0a9` passed.

# T-506: Take podcrates v0.82.0

**Source:** `PodUtils/podcrates` tag `v0.82.0` (`51c09390`, 2026-11-23).
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.81.0`. The tag `v0.82.0` adds `pod-cover`, `pod-harness`, `pod-offline`, `pod-shape`, optional `pod-sidecar`, `pod-link` `ntp`/`smtp`/`imap`, `pod-turn` RFC 6062 TCP relay, the HTTP/3 control-stream fix in `pod-warp`, hardened 8-word PBKDF2 pairing in `pod-offline`, and `url` 2.5.9 (`RUSTSEC-2026-0592`).

## Premise

- All manifests (`Cargo.toml`, `crates/*/Cargo.toml`), `Cargo.lock`, `fuzz/Cargo.lock`, `scripts/install/SOURCES`, and `.github/workflows/issue-sync.yml` pin the `podcrates` tag.
- `pod-offline` v0.82.0 raises pairing codes from 4 words to 8 words (`WORDS_COUNT = 8`) and adds `Error::CodeExpired`.

## Approach

1. Update every `pod-*` dependency across `Cargo.toml` and `crates/*/Cargo.toml` to `tag = "v0.82.0", version = "0.82.0"`, adding `pod-cover`, `pod-harness`, `pod-offline`, `pod-shape`, optional `pod-sidecar`, `noq` `qlog`, `toml`, and features `ntp`, `smtp`, `imap`, `sidecar`, and `full`.
2. Update `Cargo.lock` and `fuzz/Cargo.lock` (including `url` >= 2.5.9), `scripts/install/SOURCES`, and `.github/workflows/issue-sync.yml`.
3. Align `src/cli/cmd/offline/{mod,pair}.rs`, `tests/offline_pair.rs`, `docs/offline.md`, `src/warp/h3/serve.rs`, and `docs/quic.md` with `v0.82.0`.

## Prove

```sh
sh scripts/agent/check.sh
cargo clippy --workspace --all-targets --features full -- -D warnings
cargo test --features full --test sidecar
cargo test --test offline_pair
```

## Done

2026-11-23, consolidated onto `main`. Updated all workspace manifests, `Cargo.lock` (`url` 2.5.9), `fuzz/Cargo.lock`, `scripts/install/SOURCES`, and `.github/workflows/issue-sync.yml` to `podcrates` `v0.82.0` (`51c09390`). Aligned `offline pair` with 8-word codes and `Error::CodeExpired` (`tests/offline_pair.rs`: 2 passed), updated `docs/quic.md` and `src/warp/h3/serve.rs` for the `v0.82.0` control-stream fix, and verified `sh scripts/agent/check.sh`, `cargo clippy --workspace --all-targets --features full -- -D warnings`, and `cargo test --features full --test sidecar` (18 passed).

# T-507: `pod --backend seccomp`: refuse `listen(2)` on a kernel socket with `EPERM` (H-02)

**Source:** `docs/research/podnet/hard-problems.md` (H-02) and `docs/research/podnet/hard-problems-results.md`.
**Category:** defect
**Milestone:** M7
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

When `pod --backend seccomp` runs on a host where program memory cannot be written, `socket(AF_INET, SOCK_STREAM)` falls back to a real host TCP socket (`Sock::Kernel`). `supervise_seccomp` continued every call on `Sock::Kernel` except `connect`, so `listen(2)` reached the kernel and an unbound `listen` autobound a wildcard port on the host.

## Premise

- `crates/podnet-netns/src/seccomp/mod.rs` bypassed `handle_listen` for `Sock::Kernel`.
- `crates/podnet-netns/src/seccomp/accept.rs` handled only `Sock::Stream`.

## Approach

1. Stop continuing `SYS_listen` on `Sock::Kernel` in `crates/podnet-netns/src/seccomp/mod.rs`.
2. Return `EPERM` with a once-only warning in `crates/podnet-netns/src/seccomp/accept.rs`.
3. Add 3 regression tests (bare IPv4 listen, bind then listen, bare IPv6 listen) checking both `EPERM` and `/proc/<pid>/fd` inodes against `/proc/net/tcp{,6}` in `tests/pod_seccomp/kernel_listen.rs`.

## Prove

```sh
cargo test -p podnet --test pod_seccomp -- kernel_listen
```

## Done

2026-11-23, consolidated onto `main`. Updated `crates/podnet-netns/src/seccomp/mod.rs` and `crates/podnet-netns/src/seccomp/accept.rs` to refuse `listen` on `Sock::Kernel` with `EPERM`. Reproducing tests in `tests/pod_seccomp/kernel_listen.rs`: `seccomp_kernel_socket_refuses_listen`, `seccomp_kernel_socket_refuses_listen_after_bind`, and `seccomp_kernel_socket_refuses_listen_v6` (3 passed). Control before the fix failed all 3 cases with `listen 0 0 ok` and a host LISTEN socket matching the program inode.

# T-508: `pod --backend netns`: reassemble IPv4 UDP fragments up to 65,507-byte payloads (H-04)

**Source:** `docs/research/podnet/hard-problems.md` (H-04) and `docs/research/podnet/hard-problems-results.md`.
**Category:** defect
**Milestone:** M7
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

In `pod --backend netns`, UDP datagrams larger than 1472 bytes arrive from the guest kernel as IPv4 fragments. Because `udpflow` did not reassemble fragments before passing non-UDP packets to `smoltcp`, `smoltcp` answered fragments with a false ICMP port unreachable and `NETNS_UDP_LIMIT` was capped at 1500 bytes.

## Premise

- `crates/podnet-netns/src/netns/udpflow.rs` called `parse_udp` on raw TUN frames and forwarded unmatched frames to the TCP stack.
- `crates/podnet-netns/src/report.rs` reported `NETNS_UDP_LIMIT = 1500`.

## Approach

1. Add bounded IPv4 UDP fragment reassembly (`Fragments`, `MAX_FRAGMENTS = 64`, `MAX_PIECES = 1024`, `FRAGMENT_IDLE = 30s`, `MAX_REASSEMBLED_BYTES = 65_515`, overlap drop, oldest-set eviction) in `crates/podnet-netns/src/netns/udpflow.rs`.
2. Raise `NETNS_UDP_LIMIT` to `65_507` in `crates/podnet-netns/src/report.rs` and update `crates/podnet-netns/src/cli.rs`, `crates/podnet-netns/README.md`, `docs/pods.md`, and `tests/pod_report.rs`.
3. Add unit tests (including `a_full_fragment_table_evicts_the_oldest_set`) and an end-to-end 3000-byte UDP test (`netns_backend_reassembles_a_datagram_over_the_mtu`).

## Prove

```sh
cargo test -p podnet-netns --lib netns::udpflow
cargo test -p podnet --test pod_seccomp -- netns_udp::
cargo test -p podnet --test pod_report
```

## Done

2026-11-23, consolidated onto `main`. Implemented `Fragments` in `crates/podnet-netns/src/netns/udpflow.rs` and raised `NETNS_UDP_LIMIT` to `65_507`. Reproducing tests: `netns_backend_reassembles_a_datagram_over_the_mtu` in `tests/pod_seccomp/netns_udp.rs` (4 passed in `netns_udp::`), `a_full_fragment_table_evicts_the_oldest_set` in `crates/podnet-netns/src/netns/udpflow/tests.rs` (17 passed in `netns::udpflow`), and `tests/pod_report.rs` (5 passed). Control before the fix failed `netns_backend_reassembles_a_datagram_over_the_mtu` (exit 101 after timeout on false ICMP port unreachable) and planted eviction cap `>` failed with `left: 65 right: 64`.

# T-509: `net shim`: per-run `PODNET_VNET_PREFIX`, 59-byte cap, pre-build validation, and stderr warning (H-05)

**Source:** `docs/research/podnet/hard-problems.md` (H-05) and `docs/research/podnet/hard-problems-results.md`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Every `podnet net shim` run used the fixed abstract-socket prefix `podnet-vnet`, causing two shims in one network namespace to collide with `EADDRINUSE` or talk to each other. In addition, `sun_path` names truncated at 106 bytes instead of filling 107, invalid or non-UTF-8 `PODNET_VNET_PREFIX` values fell back silently, and `net shim` printed notes to stdout breaking `eval`.

## Premise

- `crates/podnet-vnet/src/addr.rs` hardcoded `ABSTRACT_PREFIX` without reading `PODNET_VNET_PREFIX`.
- `crates/podnet-vnet/shim/podnet-vnet.c` copied `PODNET_VNET_PREFIX` without validation and reserved an extra NUL byte in `to_abstract`.

## Approach

1. Cap `MAX_PREFIX = 59`, add `prefix()`, `valid_prefix()`, `abstract_name_with()`, and `from_abstract_name_with()` in `crates/podnet-vnet/src/addr.rs`, and `unique_prefix()` plus `Result`-returning `ShimOptions::environment` in `crates/podnet-vnet/src/shim.rs`.
2. Validate `PODNET_VNET_PREFIX` (including non-UTF-8) before build in `src/cli/cmd/net/shim.rs` with exit 64, and write notes to stderr so stdout holds only exports.
3. Validate the prefix in `crates/podnet-vnet/shim/podnet-vnet.c`, print ignored-prefix warnings unconditionally to stderr, and copy full 107-byte abstract names in `to_abstract`.

## Prove

```sh
cargo test -p podnet-vnet --lib
cargo test -p podnet --test shim_sun_path
cargo test -p podnet --test cli -- shim_prefixes_differ_per_run
```

## Done

2026-11-23, consolidated onto `main`. Updated `crates/podnet-vnet/src/{addr,control,shim}.rs`, `crates/podnet-vnet/shim/podnet-vnet.c`, `src/cli/cmd/net/{mod,shim}.rs`, `src/manual/facts.rs`, and `docs/virtual-network.md`. Reproducing tests: `shim_prefixes_differ_per_run` in `tests/cli/net.rs` (1 passed, covering per-run unique prefix, invalid and non-UTF-8 exit 64 before build, and clean stdout exports), `a_long_prefix_and_a_full_ipv6_name_fit_sun_path` in `tests/shim_sun_path.rs` (1 passed, covering 107-byte abstract names and unconditional stderr warning without `PODNET_VNET_LOG`), and `a_prefix_that_cannot_build_a_name_is_not_exported` in `crates/podnet-vnet/src/shim.rs` (`podnet-vnet --lib`: 101 passed). Control before the fix failed `shim_prefixes_differ_per_run` (both runs printed `podnet-vnet`), exported 60-byte prefixes, returned exit 0 on non-UTF-8, and produced empty stderr on invalid prefix when quiet.

# T-510: live: the check of agent up cannot connect to the address that the resolver of the runner gives

**Source:** live 45144945104 on 75f69f51 (2026-11-23), failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The step "agent up and down" of `.github/workflows/live.yml` failed. podnet verified the URL of the quick tunnel
through public DNS ("verified https://...: HTTP 200 at attempt 15, DNS step doh"). The curl of the step then got
`curl: (6) Could not resolve host` 9 times, and `curl: (7) Failed to connect to ... port 443 after 0 ms` 12 times,
until its retries ended. The resolver of the runner gave an address that the runner could not reach.

## Premise

- The same step passed on 1325d5bb (live 45146226467) with the same binary code. The fault is in the check, not in
  podnet: podnet names the case in its own output (T-395).
- The rule "Quality first" (2026-11-17): each edge has a second path.

## Approach

1. The curl of the step keeps its 60 s of retries through the resolver of the runner.
2. When it fails, a second curl resolves the name through DNS over HTTPS (`--doh-url`), as podnet does.

## Prove

```sh
python3 scripts/check-workflows.py
# the live step "agent up and down" passes on the next push
```

## Done

2026-11-23, the commit of this entry. The test is the live step "agent up and down" of `.github/workflows/live.yml`:
its curl has a second path through DNS over HTTPS. `python3 scripts/check-workflows.py` passes. The next push runs live.

# T-511: ci: the test of the HTTP/3 qlog reads the QUIC trace of noq before noq flushes it

**Source:** ci 45163142934 on ef189d2 (2026-11-23), failed.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The job `test` of ci 45163142934 failed in `tests/quic_qlog.rs::the_qlog_shows_the_http3_control_stream`:
"no QUIC qlog with a stream frame of stream 3: [.../client-...-client.qlog, .../podnet-h3-...-server.qlog]".
The test failed in 6 of 100 runs on an idle host, and in 5 of 100 runs on a host with load.

## Premise

- noq-proto 1.5.0 (`QlogFileFactory`) writes each QUIC trace through a `BufWriter` of 8 KiB. n0-qlog 0.2.0
  (`QlogStreamer`) flushes it only in `finish_log`, at the drop of the connection state: after the drain period of
  the close, and after the drop of each handle.
- The test read the traces after `closed()`, with the handle of the client connection still alive. When the
  events of the trace did not fill the buffer, the stream frame of stream 3 was not on disk yet.
- The fault is in the test. pod-quic and the code of podnet do not read the QUIC traces.
- The rule "Quality first" (2026-11-17): each edge has a check and a second path.

## Approach

1. `get()` gives back the endpoint of the client. The test drops the connection, waits for `wait_idle()` of the
   endpoint (5 s at most), drops the endpoint, and waits for the end of the server task (5 s at most).
2. The test reads the QUIC traces again each 50 ms, until a trace has a stream frame of stream `id` or 5 s end.
   The assertion and its message do not change.

## Prove

```sh
for i in $(seq 100); do cargo test -q --test quic_qlog || echo FAIL; done | grep -c FAIL
# the same loop with 6 busy loops of the shell and a release build of podnet-netns
```

## Done

2026-11-23, the commit of this entry. The test is `tests/quic_qlog.rs::the_qlog_shows_the_http3_control_stream`.
Before: 6 of 100 runs failed on an idle host, 5 of 100 under load. After: 0 of 100 on an idle host, 0 of 100 under
load (6 busy loops of the shell and a release build of podnet-netns on 4 CPUs).

# T-512: ci: the new loopback tests of 4df28952 fail on a host with no bind(AF_INET) and with the loopback down

**Source:** ci 45157147187 on af42f0d (2026-11-23), failed.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The jobs `restricted-host` and `no-loopback` of `.github/workflows/ci.yml` failed. In `tests/cover.rs`, 10 tests
stopped at `tests/cover_harness/mod.rs:106`: `TcpListener::bind("127.0.0.1:0")` gave `PermissionDenied`. Each ci run
after 4df28952 was cancelled, and cargo stops at the first test binary that fails. Thus the other new test files of
4df28952 that bind 127.0.0.1 did not run in these jobs. With the loopback down, some of them wait with no time limit.

## Premise

- The rule of the two jobs (`ci.yml`, T-076, issue 217): a test that binds or connects 127.0.0.1 skips with
  `test_host::need_loopback!()`. It does not fail, and it does not wait for a time limit.
- The skip changes no assertion. A test that does not need the loopback does not skip.

## Approach

1. Run both jobs on a local host with `--no-fail-fast`, to see each test that fails.
2. Add `need_loopback!()` at the start of each test that binds or connects 127.0.0.1.
3. A test with `should_panic` cannot skip: a skip is a pass with no panic. Write its assertion directly.
4. The sample of the listeners of `tests/pod_seccomp/kernel_listen.rs` can be empty in a network namespace. A flag,
   not the rows, says that the sample was taken; else the test reads `/proc` after the program stopped.

## Prove

```sh
sh scripts/restricted-host.sh --no-fail-fast
unshare -n -- cargo test --offline --workspace --no-fail-fast
```

## Done

2026-11-23, commit da0003f1. These tests now skip with `need_loopback!()` when the host refuses bind(AF_INET) or
has no loopback:

- `tests/cover.rs`: the 10 tests on the loopback (all but the 3 tests of the script file).
- `tests/defer.rs`: 7 tests (all but the 2 tests of the generator with no origin). The control test
  `the_old_prepared_bytes_accounting_undercounts_the_wire` asserts the two numbers directly; it has no `should_panic`.
- `tests/e2e.rs`: `one_gib_loopback_transfer_rekeys_and_completes`,
  `the_relay_dump_holds_no_plaintext_destination_service_or_key`.
- `tests/egress_refusal.rs`: the 10 tests.
- `tests/offline_discover.rs`: `a_listener_takes_the_signed_record_and_ignores_a_changed_one`,
  `an_announcement_goes_to_one_address_as_one_txt_record`.
- `tests/stat_measure.rs`: the 3 tests.
- `tests/quic_juicity.rs` (2), `tests/quic_qlog.rs` (1), `tests/quic_tuic.rs` (1), `tests/quic_tuic_cli.rs`
  (`the_tuic_option_of_warp_nc_carries_stdin_to_stdout`), `tests/quic_h3_stock.rs` (1).
- `tests/pod_netns_kernel.rs`: `netns_kernel_relays_tcp_through_a_proxy_on_the_loopback`,
  `netns_kernel_resets_a_connect_when_the_upstream_is_dead`.
- `tests/pod_seccomp/kernel_listen.rs`: `seccomp_kernel_socket_refuses_listen_after_bind`. The test
  `seccomp_kernel_socket_refuses_listen` failed in a network namespace with no listener: the sample of the listeners
  was empty, so the test read `/proc` after the program stopped. A flag now says that the sample was taken.

Before the fix, `sh scripts/restricted-host.sh --no-fail-fast` failed in 41 tests of 12 test files of this list, and
`unshare -n -- cargo test --offline --workspace --no-fail-fast` failed in most of these files and waited with no time
limit in `tests/e2e.rs` and `tests/stat_measure.rs`. After the fix, no test of this list fails in either run, and
no test waits. The plants of both jobs still fail with `PermissionDenied` and `test result: FAILED`. The tests that
still fail on the local host are of the host, not of CI: `tests/network_lab.rs` and `tests/link_p2p_namespaces.rs`
(no `ip` program), `seccomp_kernel_socket_refuses_listen_v6` (no IPv6), and, as root, the `router::kernel` and
`router::guest` unit tests and `a_resolv_conf_of_a_deleted_file_still_resolves` (CI skips them: its runner refuses
the user namespace).

# T-513: live: the edge did not know a new quick tunnel, and the second line said that a retry does not help

**Source:** live 45169011007 on c298684 (2026-11-23), failed (job 137677158556).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The step "Large share uploads, QUIC and HTTP/2" of `.github/workflows/live.yml` ran
`sh scripts/live-share.sh target/release/podnet quic "20 64"` (`podnet share --protocol quic --transport direct` on
a quick tunnel). The edge answered "Unauthorized: Tunnel not found" with retry: yes to the registration of the quick
tunnel that the service made some seconds before. pod-cloudflare tried 3 times in about 2 s. podnet exited 69 with:

```text
podnet: no edge leg worked: quic: the edge refused the registration: Unauthorized: Tunnel not found (retry: yes, after 1 s; podnet speaks cloudflared 2026.11.1)
podnet: the edge-register hop failed; a refused token or tunnel does not change with a retry; check the token, or use a quick tunnel
```

The second line is false: the edge asked for a retry, and the tunnel was a quick tunnel. The same class failed in
T-138 (live 42060806281, a named tunnel made some seconds before).

## Premise

- pod-cloudflare v0.82.0 (`PodUtils/podcrates:crates/pod-cloudflare/src/tunnel/plan.rs`, `register_retrying_with`)
  tries a refusal with retry: yes 3 times (`REGISTER_TRIES`), with 1 s to 5 s between two tries.
- pod-cloudflare gives the class TempFail to a refusal with retry: yes (`From<Refused> for Failure`). When each leg
  failed, it makes a new failure "no edge leg worked: ..." with the class Unavailable and no cause
  (`PodUtils/podcrates:crates/pod-cloudflare/src/tunnel.rs`). The typed refusal and its class are lost: podnet saw
  the stage EdgeRegister only, and gave the one hint of that hop.
- A quick tunnel cannot register again with the same name from podnet: `Credentials::Explicit` has no public URL.
  A wait for the same quick tunnel belongs to pod-cloudflare. podnet can start again, with a new quick tunnel.
- The files of podnet here are podnet's own (`sh scripts/agent/podcrates.sh`: each import of pod-cloudflare is
  `taken`). The glue `src/tunnel.rs` calls pod-cloudflare; `crates/podnet-core/src/hop.rs` and `src/report.rs` give
  the hint.
- The rule "Quality first" (2026-11-17): each edge has a second path, and each failure gives one true line.

## Approach

1. `crates/podnet-core/src/hop.rs`: `refusals` reads each refusal of the registration: the typed
   `pod_cloudflare::fail::Refused` first, else the text of pod-cloudflare in the chain. `read_refusal` wraps the
   error in `EdgeRefusal` (the kind of the tunnel, retry, "Tunnel not found"). A refusal with retry: yes, or
   "Tunnel not found" for a quick tunnel, is transient: exit 75. Each kind has its own hint.
2. `src/report.rs`: the second line of a refused registration is the hint of its kind.
3. `src/tunnel.rs`: `start` runs `again`: after a transient refusal, it waits 2 s to 6 s (`pod_core::backoff`,
   round 3) and starts one more time. For a quick tunnel this makes a new quick tunnel: a second request to the
   quick-tunnel service only after a refusal.
4. The wait for the same tunnel, and the class of a refusal through "no edge leg worked", go to pod-cloudflare
   (a COORD issue to podcrates). When pod-cloudflare keeps the refusal as the cause, the text path of `refusals`
   is the second path only.
5. `docs/tunnels.md` (A refused registration) and the fact of exit 75 (`src/manual/facts.rs`).

## Prove

```sh
cargo test -p podnet --lib a_refused_registration_gives_the_hint_and_the_code_of_its_kind
cargo test -p podnet --lib a_refusal_that_a_new_start_can_fix_gets_one_more_start
cargo test -p podnet --test quick_tunnel_standin a_quick_tunnel_that_the_edge_does_not_know_gets_one_new_start
```

## Correction

2026-11-24 (T-539): podcrates `v0.82.1` does item 4 of the approach. pod-cloudflare tries the same tunnel again for
30 s at retry: yes, and gives the class TempFail through "no edge leg worked" when each leg asked for a retry, with the
typed refusal of the last leg in its chain. `refusals` reads the typed refusal first and then the text of the chain,
so that the refusals of the other legs stay in the count. `again` of `src/tunnel.rs` stays as the second path.

## Done

2026-11-23, the commit of this entry. The tests are
`a_refused_registration_gives_the_hint_and_the_code_of_its_kind` (`src/report.rs`: 11 kinds of refusal, each with
its second line and its exit code), `a_refusal_that_a_new_start_can_fix_gets_one_more_start` (`src/tunnel.rs`: a
stand-in of the start that refuses N times, then accepts), and `tests/quick_tunnel_standin.rs`,
`a_quick_tunnel_that_the_edge_does_not_know_gets_one_new_start` (the binary against the stand-in of pod-cloudflare:
"Tunnel not found" once, then accept: ready on the second quick tunnel, 2 provisions; "Tunnel not found" always:
exit 75; another refusal: exit 69 and 1 provision). Control: on the code before the fix the stand-in test fails at
its first case: podnet exits 69 with "a refused token or tunnel does not change with a retry; check the token, or use
a quick tunnel", and prints no ready line. After the fix the three tests pass. The wait for the same tunnel inside
pod-cloudflare, the class TempFail through "no edge leg worked", and a stand-in refusal with retry: yes go to
podcrates as a COORD issue.


# T-514: gate: get-podlab.sh fails when the podcrates tag of Cargo.toml has no GitHub release

**Source:** gate 45212474279 on 1947a8e (2026-11-23), failed (job `lane`).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The step "Get podlab of the podcrates tag of Cargo.toml" of `.github/workflows/gate.yml` ran
`sh scripts/get-podlab.sh v0.82.0 "$RUNNER_TEMP/podlab"`. The download of SHA256SUMS gave
`curl: (22) ... 404`, and the script stopped with
`get-podlab: cannot download https://github.com/PodUtils/podcrates/releases/download/v0.82.0/SHA256SUMS`.
The tag v0.82.0 of podcrates exists, but it has no GitHub release. The job `publish` of `release.yml` needs the
job `gate` (`gate.yml`). Thus the failure blocks the release.

## Premise

- Not each tag of podcrates gets a GitHub release. The newest release is v0.81.0. Each release from v0.58.0 has
  `SHA256SUMS` and `podlab-ARCH-unknown-linux-musl`.
- podcrates publishes `podlab-install.sh` with each release. It holds the pins of its own release only: it cannot
  install podlab for a tag with no release. It is no second path here.
- The rule "Quality first" (2026-11-17): each edge has a second path, and each failure gives one true line.

## Approach

1. `scripts/get-podlab.sh`: when the release of VERSION does not exist or does not have the asset (a download gives
   404), read the list of releases of podcrates (GitHub REST API, `GET_PODLAB_API`, `GH_TOKEN` to api.github.com
   only). Take the newest release at or below VERSION, not a draft or a prerelease, that has `SHA256SUMS` and the
   asset. Write one line that names VERSION and the release that it took. The SHA-256 check does not change.
2. A VERSION that is not vX.Y.Z is a usage fault: exit 64. When no release at or below VERSION has the asset, or a
   download fails, write one line and exit 1. On a failure, remove DIR, so that a second run can use it.
3. `.github/workflows/gate.yml`: the step gives `GH_TOKEN` (`github.token`) to the script.
4. `docs/development.md`: the fallback.

## Prove

```sh
sh -n scripts/get-podlab.sh
python3 scripts/check-workflows.py
GET_PODLAB_API=https://forge.web.pkgforge.dev/github sh scripts/get-podlab.sh v0.82.0 DIR1   # takes v0.81.0
GET_PODLAB_API=https://forge.web.pkgforge.dev/github sh scripts/get-podlab.sh v0.81.0 DIR2   # v0.81.0 directly
GET_PODLAB_API=https://forge.web.pkgforge.dev/github sh scripts/get-podlab.sh v0.0.1 DIR3    # exit 1, one line
# the next run of gate.yml passes the step
```

## Done

2026-11-23, the commit of this entry. The local runs (the API of the sandbox is a mirror of the GitHub REST API):
v0.82.0 printed "get-podlab: podcrates v0.82.0 has no release with podlab-x86_64-unknown-linux-musl; took release
v0.81.0", "podlab-x86_64-unknown-linux-musl: OK", "podlab 0.81.0", exit 0. v0.81.0 printed no fallback line, "podlab
0.81.0", exit 0. v0.0.1 printed "get-podlab: podcrates v0.0.1 has no release with podlab-x86_64-unknown-linux-musl,
and no release at or below v0.0.1 has it", exit 1, and left no DIR. `0.81` gave exit 64. With the API refused
(HTTP 403), the script printed one line with "(HTTP 403)" and exit 1. `python3 scripts/check-workflows.py`: 0
faults. The next run of `gate.yml` runs the step on the runner.


# T-515: connect: a TCP or unix listener that cannot bind exits 1; net proxy, forward and warp proxy exit 69 (GitHub #689)

**Source:** GitHub #689 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

Lab `bind-denied` (a TCP bind gives EACCES; AF_UNIX holds), and a plain host with port 18080 in use:

```
podnet connect wss://nowhere.invalid/__podnet/ws -L tcp://127.0.0.1:18083 --no-check --run-for 2
podnet: binding 127.0.0.1:18083: Permission denied (os error 13)      exit 1
podnet net proxy --listen tcp://127.0.0.1:18082 --run-for 2
podnet: binding tcp://127.0.0.1:18082: Permission denied (os error 13)   exit 69
podnet forward -L 18081=127.0.0.1:18080 --run-for 2
podnet: -L 127.0.0.1:18081: Permission denied (os error 13)           exit 69
podnet connect wss://nowhere.invalid/__podnet/ws -L tcp://127.0.0.1:18080 --no-check --run-for 2
podnet: binding 127.0.0.1:18080: Address in use (os error 98)         exit 1
```

`net proxy`, `forward` and `warp proxy` give exit 69 for the same cause. The promise: `podnet man exit-status`,
"69 (unavailable) A service or a network path is not available"; `podnet --help`, "69 unavailable: a carrier,
listener or relay disabled by policy, or not answering"; "1 (failure): A failure that has no other code."

## Premise

- Read: `crates/podnet-core/src/net.rs:162` has `bind_fault`. It gives `Fault::Unavailable` (69) to `AddrInUse`,
  `PermissionDenied` and `AddrNotAvailable`. `forward`, `net proxy`, `serve`, `route`, `link` and `cover` call it.
- Read: `src/cli/cmd/connect.rs:109` binds the TCP listener of `-L` and adds only a context (`:110`). It does not
  call `bind_fault`, so the exit code is 1. `unix_listener` (`src/cli/cmd/connect.rs:246`, `:249`) has the same
  fault for a unix listener. `git serve` and other commands use `unix_listener`.
- Read: `src/cli/cmd/e2e.rs:79` and `:83` bind the data and control listeners of `e2e` with a context only. The
  tester did not run `e2e`. It has the same fault by the same reading.
- Not measured: the exit code of `e2e`, and of a unix listener whose directory is not writable.

## Approach

1. Give each bind of a listener the class of `bind_fault`: `connect` (TCP and unix), `unix_listener`, `e2e`. Prefer
   `podnet::net::bind_tcp` where it fits.
2. One table-driven test (a new file tests/listener_bind.rs, after the pattern of
   `src/cli/listen_table.rs`): a row for each command that has a listener (`connect -L tcp://`, `connect -L unix:`,
   `net proxy --listen`, `forward -L`, `serve --local`, `git serve --ssh`, `e2e --listen`, `link dial --listen`).
   Each row binds the address first (`need_loopback!` skips a host with no loopback), runs the command with
   `--run-for 2`, and asserts exit 69 and one line on stderr. The rows `connect` and `e2e` fail before the fix (exit 1).
3. A row for the lab `bind-denied` goes into `scripts/lab-lane.sh` (the case `errors`): each listener command exits
   69 there.
4. The text of the line (the next step) belongs to T-523.

## Prove

```sh
cargo test -p podnet --test listener_bind
sh scripts/lab-lane.sh --help
python3 scripts/check-size.py
```

## Done

2026-11-23, the commit of this entry. `connect -L tcp://` (`src/cli/cmd/connect.rs`), `unix_listener`, the two
listeners of `tunnel --e2e` (`src/cli/cmd/e2e.rs`), `git serve --ssh`, the unix listeners of `net proxy`, `net switch`,
`warp proxy`, `route` (its control socket and a `unix:` wire) and the control socket of a link go through
`podnet_core::net::bind_fault` (`bind_fault_unix` for a unix socket). A refused bind exits 69 in each command with a
listener. A unix socket in a directory that does not exist is a refused bind too (69, with the next step). The test
`a_listener_that_cannot_bind_exits_69_with_one_line_and_a_next_step` (tests/listener_bind.rs) has 17 rows: connect,
net proxy, forward, serve, git serve (`--ssh`, `--local`), tunnel --e2e, warp proxy and warp forward, with a TCP address
in use, a live unix socket and a unix socket with no directory. Before the fix (src and crates of `main`, the same test):
"podnet connect ... -L tcp://127.0.0.1:43195 --no-check: exit Some(1), not 69 / podnet: binding 127.0.0.1:43195:
Address already in use (os error 98)"; the same exit 1 for `tunnel --e2e --listen` and for `connect`, `net proxy` and
`serve --local` with `unix:DIR/nodir/x.sock`. The unit test `a_refused_bind_is_69_with_one_line_and_a_next_step`
(crates/podnet-core/src/net.rs) holds the class of each kind. The lab rows `listen-*` of `scripts/lab-errors.sh`
(lane case `errors`) pass on this host; the lab `bind-denied` runs them with EACCES. `link dial --listen` has no row: it
binds after the link to a peer is up.


# T-516: pod: with the seccomp backend, a program that cannot run exits 127 with no line (GitHub #690)

**Source:** GitHub #690 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

Lab `userns-off` (the backend is seccomp); lab `bind-denied` and a plain host with `--backend seccomp` give the same:

```
podnet pod -x direct -- /nonexistent/prog      (no output)   exit 127
podnet pod -x direct -- /etc/passwd            (no output)   exit 127
podnet pod -x direct -- nonexistentprog
podnet pod: the program "nonexistentprog" is not in PATH      exit 3
```

Plain host, backend netns: `podnet pod -x direct -- /nonexistent/prog` gives
`podnet pod: execvpe /nonexistent/prog: No such file or directory (os error 2)` and exit 3.

The promise: `podnet man exit-status`, "3 (no-backend) ... the program is not in PATH (or cannot run), or the setup of
the backend failed."; `podnet pod --help`; `docs/pods.md`, "Exit status". A path that does not exist, and a file with
no execute permission, must give exit 3 and one line with the seccomp backend too.

## Premise

- Read: `crates/podnet-netns/src/seccomp/mod.rs:85` resolves a bare name (`pod_hold::spawn::resolve`). A name with a
  `/` is used as it is, with no check. `crates/podnet-netns/src/seccomp/mod.rs:90` starts the program with
  `pod_hold::spawn::start`.
- Read: `PodUtils/podcrates:crates/pod-hold/src/spawn.rs` (v0.82.0): the child of the fork calls `execve` and then
  `_exit(127)`. The parent never reads the errno of `execve`. The netns backend (`crates/podnet-netns/src/raw.rs:36`,
  `ExecError`) reads it through a pipe that closes at `execve`, and exits 3.
- The fault is in a crate of podcrates (`pod-hold`) and in the check of podnet before the start.
- Not measured: the exit code of `--backend netns-kernel` with a program that cannot run.

## Approach

1. COORD ask (to podcrates, the lead files it): `pod_hold::spawn::start` reports the errno of the `execve` of the
   program as a `Failure` (the pipe with `FD_CLOEXEC` that `raw.rs` uses), with the class of the errno (`ENOENT`,
   `EACCES`, `ENOEXEC`), and `resolve` checks a path with a `/` as a bare name is checked (a regular file with an
   execute bit).
2. podnet: until the tag that has it, `supervise_pod` checks the path after `resolve_program` (it exists, it is a
   regular file, `access(X_OK)`), and exits 3 with the line `execvpe PATH: REASON`, the form of the netns backend.
   Remove the check when podnet takes the tag.
3. One table-driven test (`tests/pod_error_lines.rs` has the form): for each backend that `podnet pod doctor` finds
   (netns, seccomp), for each program (a path that does not exist, a directory, a file with no execute bit, a bare name
   that is not in PATH): exit 3, one line on stderr that names the program and the reason, and no 127. The seccomp rows
   fail before the fix.

## Prove

```sh
cargo test -p podnet --test pod_error_lines
cargo test -p podnet-netns
```

## Done

2026-11-23, the commit of this entry (the part of podnet; the part of podcrates is open). Before a backend starts
the program, `podnet pod` checks it (`crates/podnet-netns/src/program.rs`, `can_run`): a path with a `/` must be a
regular file with an execute bit that `access(X_OK)` accepts; a bare name must be in `PATH`
(`pod_hold::spawn::resolve`). A program that cannot run gives one line (`execvpe PATH: REASON`, or `the program
"NAME" is not in PATH`) and exit 3 on each backend. The tests: `a_program_that_cannot_run_exits_3_with_one_line`
(`tests/pod_error_lines.rs`: the programs `/nonexistent/prog`, a directory, a file with no execute bit and a bare
name not in PATH, on each backend that `podnet pod doctor` finds: seccomp, netns and netns-kernel on this host), and
`a_program_that_cannot_run_gives_one_line` (the unit table of `can_run`). `cargo test -p podnet --test
pod_error_lines`: 4 passed. `cargo test -p podnet-netns`: 173 passed. Control: on the code before the fix the test
fails at each seccomp row: `seccomp, /nonexistent/prog: exit Some(127), want 3 and one line with "No such file or
directory"` (no line of the cause); the netns rows give exit 3 and the line, and a bare name gives `execvpe
podnet-no-such-program: No such file or directory`. Left: a file that the kernel cannot run (`ENOEXEC`, a binary of
another architecture) still exits 127 with no line on the seccomp backend. The COORD ask (to podcrates):
`pod_hold::spawn::start` reports the errno of the `execve` of the program as a `Failure` (a pipe with `FD_CLOEXEC`,
as `crates/podnet-netns/src/raw.rs` does), with the class of the errno, and `resolve` checks a path with a `/`. When
podnet takes that tag, the check of a path with a `/` in `program.rs` goes away.

2026-11-24 (T-542, the take of podcrates `v0.83.0`): the rest is done. `pod_hold::spawn::start` reports the errno of
`execve`, and `resolve` checks a path with a `/` (PodUtils/podcrates@T-1125). `can_run` (`crates/podnet-netns/src/program.rs`)
calls `resolve` for each name; podnet's own check of a path is gone. The netns backends (`netns/mod.rs`, `raw.rs`)
start the program with `execve` of the resolved path (`program::exec_path`), not `execvpe`: `execvpe` gave a file
that the kernel refuses (ENOEXEC) to `/bin/sh`, and the pod exited 127 with no line. The test: a row "a file with an
execute bit that the kernel cannot run" in `a_program_that_cannot_run_exits_3_with_one_line`
(`tests/pod_error_lines.rs`; the test clears the proxy variables of the host, which give a line of their own).
Control: on `v0.82.1` the row gives exit 127 and no line on each backend; after the take, the seccomp backend gives
exit 3 and `the program "PATH" could not be started: execve: ENOEXEC: Exec format error (os error 8)`, and the netns
backends give exit 3 and one line too. `cargo test -p podnet --test pod_error_lines`: 4 passed. `cargo test -p
podnet-netns`: 173 passed. `docs/pods.md` says that a script needs a `#!` line.

# T-517: pod: SIGTERM, SIGINT or SIGHUP to podnet ends podnet and leaves the program running (GitHub #691)

**Source:** GitHub #691 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** done

## Problem

Plain host, backend netns, and the same with `--backend seccomp`:

```
podnet pod --backend netns -x direct -- sleep 37      # in one shell
kill -TERM PID_OF_PODNET                               # from another shell
```

podnet exits at once with exit 143 (SIGINT: 130, SIGHUP: 129) and prints no line. `pgrep -af '^sleep 37'` still shows
the `sleep` process 1 s later. The promise: the pass prompt, check 6, "A command that waits: a stop signal (SIGTERM,
SIGINT) ends it at once and cleanly."; `podnet man exit-status`, "128+N (signal): A second stop signal during the
shutdown, or a signal that ended a program of podnet pod"; `docs/pods.md`, "The pod returns when the program exits".
A user of `kill`, or a service manager that stops only the main process, leaves an orphan with no pod around it (no
supervisor for the seccomp backend, no network stack for the netns backend).

## Premise

- Read: `podnet pod` goes to `podnet_netns::main_with_args` before the command line is parsed
  (`src/main.rs:104`). The signal handlers of `src/cli/signals.rs` do not run for it.
- Read: the backends install no handler for SIGTERM, SIGINT or SIGHUP. The netns reaper only ignores SIGINT and SIGQUIT
  for itself (`crates/podnet-netns/src/netns/mod.rs:198`). The seccomp supervisor (`supervise_pod`,
  `crates/podnet-netns/src/seccomp/mod.rs:70`) has none. The default action ends podnet and the program stays.
- The code is podnet's own (`crates/podnet-netns`). The start of the program in the seccomp backend is
  `pod-hold`, but the signal belongs to the caller.
- Not measured: `--backend netns-kernel`, and a program that ignores SIGTERM.

## Approach

1. The pod takes SIGTERM, SIGINT and SIGHUP in each backend (`run`, `run_seccomp`, `kernel::run`). A signal goes to the
   program (to the process group of the program, as a terminal does). The pod then waits for the end of the program as
   today, and returns its exit status (128+N when the signal ended it).
2. A second stop signal, or a program that does not end in a bounded time (10 s), gets SIGKILL
   and the exit 128+N of the signal that came. Nothing stays running: the pod stops each orphan of its namespace.
3. One line on stderr names the signal that came (`podnet pod: SIGTERM: stopping the program`).
4. One table-driven test (a new file tests/pod_signals.rs): for each signal (SIGTERM, SIGINT, SIGHUP) and each backend that `podnet pod doctor` finds,
   the program is `sleep 37`; the test signals podnet, then asserts that podnet exits in 5 s with 128+N, that the
   `sleep` process no longer exists, and that no process of the pod stays. Before the fix, the `sleep` stays.
5. A row of the lane (`scripts/lab-lane.sh`) does the same in the labs `userns-off` and `bind-denied`.

## Prove

```sh
cargo test -p podnet --test pod_signals
cargo test -p podnet-netns
```

## Done

2026-11-23, the commit of this entry. A new module `crates/podnet-netns/src/stop.rs`: the front of the pod
(`outlive::front`) takes SIGTERM, SIGINT and SIGHUP and gives a signal that a process sent (`si_code` 0 or less) to the
back; the back gives it to its child (the reaper of the namespace in `netns` and `netns-kernel`, the program in
`seccomp`); the reaper gives it to the program. A signal of the terminal (`SI_KERNEL`) is not given again: the program
has it. A signal that the caller ignores (`nohup`) stays ignored. The front prints one line (`podnet pod: SIGTERM:
stopping the program`) and returns the status of the program. After the code of a stopped program, the front (a
subreaper) gives the back 1 s, then kills each process under it. At a second stop signal, or after 10 s, each process of
the pod gets SIGKILL and podnet exits 128+N. The test `a_stop_signal_ends_the_program_and_each_process_of_the_pod`
(tests/pod_signals.rs): for each backend that `podnet pod doctor` finds (seccomp, netns, netns-kernel here) and each
signal, the program (`sleep 37`, with a `sleep 41` of its own) ends, podnet exits 128+N in 5 s, and no process stays.
Before the fix: "seccomp SIGTERM: podnet ended with signal: 15 (SIGTERM), not exit 143; [("the program", 4812), ("its
process", 4817)] stays; not one line that names SIGTERM", the same for each of the 9 rows. A manual run: a program that
ignores SIGTERM ends with SIGKILL at a second SIGTERM (0.5 s) or after 10.05 s, exit 143. Unit tests
`only_a_signal_of_a_process_goes_down_the_chain` and `the_descendants_of_a_process_include_its_grandchildren`. The lab
rows `pod-sigterm-auto` and `pod-sigterm-seccomp` of `scripts/lab-errors.sh` pass on this host. Not measured: a
service manager that signals the whole control group (each process then forwards; the program can get the signal more
than one time).


# T-518: rules check: a policy file that cannot be read or parsed exits 1; PODNET_POLICY gives 78 (GitHub #700)

**Source:** GitHub #700 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

```
echo '{"default":"refuse","rules":[' > pol2.json
podnet rules check pol2.json
podnet: pol2.json: the rules are not JSON of the policy: EOF while parsing a list at line 2 column 0     exit 1
podnet rules check /nonexistent.json
podnet: /nonexistent.json: No such file or directory (os error 2)                                         exit 1
PODNET_POLICY=pol2.json podnet bench --duration 1 http://127.0.0.1:18080/
podnet: pol2.json: the rules are not JSON of the policy: EOF while parsing a list at line 2 column 0     exit 78
```

The promise: `docs/host-runtime.md:134`, "A file that does not parse exits 78."; `podnet man environment`
(`PODNET_POLICY`), "A file that cannot be read or parsed is a configuration error (exit 78)."; `podnet man
exit-status`, "78 (config) A configuration that cannot be used". A script cannot tell a bad file from a failed check.
Measured on 2026-11-23 with the debug binary of beta.17: the same three exit codes (1, 1, 78); a policy with an
unknown action (`"default":"allow"`) also exits 1.

## Premise

- Read: `src/cli/cmd/runtime.rs:165` calls `Policy::load(&file)?` and gives the error no class: exit 1. The same
  function loads `PODNET_POLICY` in `install_from_env` (`src/policy.rs:511`), which `src/main.rs:58` turns into exit
  78. Thus one loader has two classes.
- Read: the `--help` of `rules check` says it exits 78 when a rule is unreachable. That case is right
  (`src/cli/cmd/runtime.rs:205` gives `Fault::Config`). The bad file is the other case.
- The code is podnet's own.

## Approach

1. `Policy::load` gives the class itself: a file that cannot be read or parsed is `Fault::Config` (78), for each
   caller (`rules check`, `PODNET_POLICY`, `PODNET_NODE_CONFIG`). The call of the issue: a missing file is 78 too,
   as `podnet man environment` says; the exit 66 is for a file that an option names and that must exist.
2. One table-driven test (a new file tests/rules_check.rs): for each input (missing file, a directory, an empty file, bad JSON, an unknown action, a
   duplicate rule name) and each entry (`rules check FILE`, `rules check FILE --json`, `PODNET_POLICY=FILE podnet
   bench ...`): exit 78 and one line that names the file. The `rules check` rows fail before the fix.
3. `docs/host-runtime.md` and `src/manual/facts.rs` list `rules check` where they list the 78 of a bad file.

## Prove

```sh
cargo test -p podnet --test config_input
cargo test -p podnet policy
```

## Done

2026-11-23, the commit of this entry. `Policy::load` (`src/policy.rs`) gives a file that cannot be read or parsed
the class `Fault::Config` (78), with the line `FILE: CAUSE`; `PODNET_POLICY` uses the same loader. A missing file is 78
too (the call of the triage). The help of `rules check`, `docs/host-runtime.md` and `podnet man environment` say it. The
test `a_bad_configuration_input_exits_78_with_one_line_that_names_it` (tests/config_input.rs): for each input (missing,
a directory, empty, not JSON, an unknown action, a rule name two times) and each entry (`rules check FILE`, with
`--json`, `PODNET_POLICY` with `bench`, `pod`, `pod --backend seccomp`): exit 78 and one line that names the file.
Before the fix: "None podnet rules check /tmp/.tmpTSVIeP/missing.json: exit Some(1), not 78", and exit 1 for each of the
12 `rules check` rows.


# T-519: JSON documents without the stamp: warp status, pod doctor and rules check (GitHub #693, #694)

**Source:** GitHub #693 and #694 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

#693, plain host:

```
podnet warp status --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print("schema" in d, "podnet" in d, list(d)[:6])'
False False ['id', 'account_type', 'created', 'updated', 'premium_data', 'quota']
```

#694, plain host:

```
podnet pod doctor --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print("schema" in d, "podnet" in d, list(d)[:5])'
True False ['schema', 'capabilities', 'verdicts', 'backend_netns', 'backend_seccomp']
```

The report `pod --report` has `version` and no `podnet` (that file has its own schema). The promise: `docs/cli.md:81`,
"Stamp: each object has `schema` (a positive integer) and `podnet` (the version line of the binary)."; `tests/contracts.rs`
is said to check each command that runs with no network. The documents of `status`, `ports`, `components`, `doctor`,
`bench`, `link status`, `net doctor` and `ps` have both keys.

Found while the entry was written (not in the issues): `podnet rules check FILE --json` prints `"schema":
"podnet.rules-check/1"` (a string, not a positive integer) and has no `podnet` key (measured on 2026-11-23 with the
debug binary of beta.17).

## Premise

- Read: `podnet::status::stamped` (`src/status.rs:123`) adds `schema` (1 unless the document has one) and `podnet`.
  `warp status --json` prints the account record with `serde_json::to_string_pretty(&account)` and does not call it
  (`src/cli/cmd/warp/account.rs:74`). `rules check` does the same (`src/cli/cmd/runtime.rs:181`, with its own
  string `schema`).
- Read: `podnet-netns` writes the document of `pod doctor --json` itself. It has `schema` from `SCHEMA`
  (`crates/podnet-netns/src/doctor.rs:426`) and no version. The library takes the version line of the caller
  (`podnet_netns::cli::set_version_line`, `crates/podnet-netns/src/cli.rs:87`).
- Read: `tests/contracts.rs` has a table of 40 rows. It has no row for `warp status`, `pod doctor` or `rules check`.
  Its header says that `warp status` needs the network; a stored `warp.json` in `PODNET_STATE_DIR` and a stand-in
  (`tests/warp_stand_in.rs`) can give it a row.
- The code is podnet's own. Not measured: `ci check --json` and the other documents that call
  `to_string_pretty` (`src/cli/cmd/ci.rs`, `src/cli/cmd/net/shim.rs`).

## Approach

1. `warp status --json`: print `podnet::status::stamped(account)`. The record keeps its keys; a key `schema` or
   `podnet` of the provider record (if one comes) does not replace the stamp.
2. `pod doctor --json`: add `podnet` from the version line that the caller sets (`VERSION_LINE`); with no line, the
   name and the version of the library.
3. `rules check --json`: `schema` 1 (a positive integer) and `podnet`. The string `podnet.rules-check/1` moves to a
   key `kind`. The change is breaking and allowed (beta); `docs/host-runtime.md` and `docs/cli.md` say it.
4. The table of `tests/contracts.rs` gets the rows `warp status --json` (a stored record), `pod doctor --json`
   (accept exit 0 and 3) and `rules check FILE --json`. The rows fail before the fix (no `podnet` key).
5. A table row of the same test, or a read of `ci check --json`, covers each other document that `src/cli/`
   prints with `to_string_pretty`; the test lists each one that has no stamp.

## Prove

```sh
cargo test -p podnet --test contracts
cargo test -p podnet-netns doctor
```

## Done

2026-11-23, the commit of this entry.

- Cause: `warp status --json`, `rules check --json`, `init check --json`, `ci check --json` and
  `init enroll --json` printed their document with `to_string_pretty` and no call of `podnet::status::stamped`;
  `pod doctor --json` (podnet-netns) had `schema` and no `podnet`. `offline spool --json` and `warp probe --json`
  printed `json!` with no stamp.
- Change: each of the seven documents has the stamp. `warp status --json` sets `schema` 1 over a key of the record.
  `pod doctor --json` adds `podnet`, the version line that `set_version_line` gives with no first word
  (`podnet_netns::cli::stamp_version`), as the other documents have it. It is an added optional key: its schema id
  stays 1 (`docs/decisions.md`, 2026-11-18), and the reader of podrunner is not affected. `rules check`,
  `init check` and `ci check`: `schema` is 1 and the string id moves to the key `kind` (`podnet.rules-check/1`,
  `podnet.init-check/1`, `podnet.ci-check/1`). This is a breaking change (beta): a reader of the string `schema`
  reads `kind`. `docs/cli.md` (JSON documents, rule 2) and `docs/host-runtime.md` say it; `tests/ci.rs` reads
  `kind`.
- Test: `each_json_document_keeps_the_contract` (`tests/contracts.rs`) has the rows `rules check --json`,
  `init check --json`, `ci check --json`, `init enroll --offline --json`, `pod doctor --json` (exit 0 or 3) and
  `warp status --json` (a device that `warp register` stored against the API stand-in of `pod-testkit` on the
  loopback). A row has its own variables and exit codes (`Row::env`, `Row::codes`).
- Control: on the code before the fix the test fails with 6 faults: `rules check`, `init check` and `ci check`:
  `schema: {"schema":"podnet.rules-check/1",...}` (and the two others); `init enroll`: `schema: {"node":"a",...}`;
  `pod doctor --json`: `podnet: {"schema":1,"capabilities":...}`; `warp status --json`:
  `schema: {"account_type":"free",...}`.
- `cargo test -p podnet --test contracts`: 2 passed. `cargo test -p podnet --test ci`: 6 passed.
  `cargo test -p podnet-netns doctor`: 8 passed.
- Not covered by a row: `offline spool --json` (needs a spool of a pair) and `warp probe --json` (needs WARP).

# T-520: send: the timeout line gives the wrong time, "within its 1 minutes" for --timeout 5 (GitHub #687)

**Source:** GitHub #687 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Plain host:

```
podnet send --text hello --timeout 5
Wormhole code is: 8-therapist-glucose-trombonist
On the other computer, run: podnet receive (and type the code)
podnet: nobody took the code within its 1 minutes; it is spent
```

Exit 124 after 5.6 s. `--timeout 1` prints "within its 1 minutes" (exit 124 after 2.5 s). `--timeout 90` prints "within its
2 minutes" (exit 124 after 92 s). The promise: `podnet send --help`, "`--timeout <TIMEOUT>  Seconds to wait for a
receiver to take the code`"; `podnet man exit-status`, "124 (timeout) ... podnet send: no receiver took the code in
--timeout seconds." The line states a time that is not the value of `--timeout`. It rounds up to whole minutes, and it
writes "1 minutes" (a wrong plural).

## Premise

- Read: `src/transfer/send.rs:87` returns `Failed::Expired(settings.wait)`: the value of `--timeout`, as a
  `Duration`.
- Read: `PodUtils/podcrates:crates/pod-wormhole/src/failed.rs:57` (v0.82.0) writes `life.as_secs().div_ceil(60)`
  followed by " minutes". The test of that file pins "within its 10 minutes" for 600 s. The words are in a crate of
  podcrates; podnet cannot change them.
- The exit code (124) is right.

## Approach

1. COORD ask (to podcrates, the lead files it): `Failed::Expired(life)` writes the time exactly: whole minutes as
   "N minute" or "N minutes" (singular for 1), else seconds as "N seconds" (singular for 1). A test of the crate
   pins 1 s, 5 s, 60 s, 90 s and 600 s.
2. podnet: a test of `podnet send --timeout N` for each of these values (a stand-in mailbox, `pod-wormhole`
   feature `standin`): the line has the value of `--timeout` in the unit of its form. It fails before the fix for 5 s,
   60 s and 90 s. The test passes when podnet takes the tag.
3. Until the tag, podnet may write the line itself in `src/transfer/send.rs` (it holds `settings.wait`).

## Prove

```sh
cargo test -p podnet --test transfer send_timeout
cargo test -p podnet transfer
```

## Done

2026-11-23, the commit of this entry. The call: podnet writes the line itself, from the value that it has
(`Failed::Expired(life)` holds the `--timeout` of `settings.wait`), because the words of pod-wormhole are wrong for
each value that is not a whole number of minutes, and the fix of the crate comes with a later tag. `failed` of
`src/transfer/mod.rs` gives `nobody took the code within its TIME; it is spent`, with `exact_time`: whole minutes as
"N minute" or "N minutes", else "N second" or "N seconds". The exit code stays 124. The tests:
`send_timeout_line_gives_the_exact_time` (`src/transfer/mod.rs`: 1 s, 5 s, 60 s, 90 s, 600 s, 1800 s) and
`send_timeout_names_the_value_of_timeout` (`tests/transfer.rs`: a sender at the stand-in mailbox with `--timeout` 1
and 2, nobody takes the code: exit 124 and "within its 1 second;", "within its 2 seconds;"). `cargo test -p podnet
--test transfer send_timeout`: 1 passed. Control: on the code before the fix the test fails with `1 s: nobody took
the code within its 1 minutes; it is spent`. The COORD ask (to podcrates): `Failed::Expired(life)` of
`pod-wormhole` (`failed.rs`) writes the time exactly (whole minutes as "N minute(s)", else "N second(s)", singular
for 1), with a test of 1 s, 5 s, 60 s, 90 s and 600 s. podnet keeps its own line after the take (the line is
podnet's own text).


# T-521: error lines: a hint of the wrong hop (quick tunnel), and no hint for a dead proxy or a name that does not resolve (GitHub #695, #697, #698)

**Source:** GitHub #695, #697 and #698 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23). The rule that they break:
the pass prompt, check 3, "one error line that names each cause once and says what to do."
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

#695, lab `proxy-only` (the proxy answers `POST /tunnel` with the text `pod-lab stand-in`):

```
podnet tunnel --url unix:/var/tmp/qa/work/none.sock --run-for 20
podnet: provisioning quick tunnel: could not decode the quick-tunnel response: expected value at line 1 column 1; body was: pod-lab stand-in

podnet: the edge-register hop failed; when the edge says retry: no, check the token or the credentials file; when it says retry: yes, start again after the wait that it gives
```

Exit 69. The failure is in provisioning (the answer of `api.trycloudflare.com` is not JSON). No edge registration ran. A
quick tunnel has no token. The hint does not say that a proxy or a middlebox answered with a page that is not the API.
The same lab: `podnet warp proxy --ephemeral --listen unix:/var/tmp/qa/work/w.sock` gives `podnet: registering a WARP
device: the provider's API did not answer: POST /reg: the WARP API answer does not decode (17 bytes)`, with no body text
and no hint.

#697, plain host (a dead proxy `127.0.0.1:1`):

```
podnet connect --stdio --proxy http://127.0.0.1:1 wss://example.com/__podnet/ws </dev/null
podnet: the remote wss://example.com/__podnet/ws does not answer: TCP to example.com: could not connect to example.com:443: example.com:443 through the proxy http://127.0.0.1:1: could not reach the proxy 127.0.0.1:1: could not connect to 127.0.0.1:1: Connection refused (os error 111)
podnet relay connect --proxy http://127.0.0.1:1 --token x qa-nonexist
podnet: the session to qa-nonexist: could not connect to tcp.ssh.relay.ajam.dev:443: tcp.ssh.relay.ajam.dev:443 through the proxy http://127.0.0.1:1: could not reach the proxy 127.0.0.1:1: could not connect to 127.0.0.1:1: Connection refused (os error 111)
podnet bench --proxy http://127.0.0.1:1 http://example.com/
podnet: bench: no request worked: GET http://example.com/: example.com:80 through the proxy http://127.0.0.1:1: could not reach the proxy 127.0.0.1:1: could not connect to 127.0.0.1:1: Connection refused (os error 111)
```

The cause is the proxy. The line of `connect` says that the remote does not answer, and `relay connect` and `bench` have no
next step. Only `tunnel --proxy http://127.0.0.1:1 --url http://127.0.0.1:8080` gives the second line: `podnet: the
proxy hop failed; check the proxy URL, its user and password, and the allow-list of the proxy; --no-proxy goes around it`.

#698, lab `no-proxy-no-dns` and a plain host: `connect --stdio wss://nowhere.invalid/__podnet/ws` and `tunnel` print
`podnet: the dns hop failed; give the address with --resolve HOST=IP, or let a proxy resolve names (socks5h://)`.
`bench`, `relay connect --relay https://nowhere.invalid`, `send --text hi` and `warp proxy` print no such line.

## Premise

Measured on 2026-11-23 with the debug binary of beta.17: the `connect`, `relay connect` and `bench` lines of #697 and the
`tunnel` hint are as the issue says.

- #695. Read: `pod-cloudflare` gives a failure of the provisioning the stage `EdgeRegister`
  (`PodUtils/podcrates:crates/pod-cloudflare/src/quicktunnel/error.rs`: `Decode`, `Service` and `MissingResult` are
  `(Stage::EdgeRegister, Class::Protocol)`). `podnet_core::hop::of` maps the stage to `Hop::EdgeRegister`
  (`crates/podnet-core/src/hop.rs:104`). The hint of that hop is for the registration with a token or a credentials
  file (`crates/podnet-core/src/hop.rs:48`). `src/report.rs:51` appends it for any failure of the stage. The hop is
  too coarse: a provisioning has no token. The form of that hint ("when the edge says retry: no, ...") reads as a
  condition that the user cannot test.
- #695, WARP. Read: `PodUtils/podcrates:crates/pod-warp/src/register.rs:89` and `:140` write "the WARP API answer does
  not decode (N bytes)". They keep no text of the body; `pod-cloudflare` keeps one safe line of it.
- #697. Read: `src/ws.rs:108` turns the dial failure of `connect` into a hop when its stage is `Proxy` or `Dns`. The
  failure of the dialer of podnet has the stage `Proxy` (`src/dialers/mod.rs:41`, `failure`). `pod-ws` then writes it as
  `DialError::Connect` with the text only and the stage `Connect` (`PodUtils/podcrates:crates/pod-ws/src/way.rs`,
  `dial_error`), so the stage is gone when `src/ws.rs:111` reads it. No hop, no hint.
- #697, #698. Read: `bench` keeps the first error of the workers as a string (`src/cli/cmd/bench.rs:77`,
  `format!("{err:#}")`) and `bail!` gives that string (`src/cli/cmd/bench.rs:176`): the hop of `src/fetch.rs` is lost.
  `src/relay_reverse/resume.rs:422` and `:435` write `anyhow!("the session to {name}: {e}")`: the hop is lost too.
- The rule that works: `report::error_line` (`src/report.rs:9`) gives the hint of `hop::of(err)`. A command whose error
  keeps the typed chain gets the hint. Not read: `send` and the `warp proxy` path of #698 (the tester names no cause).

## Approach

1. COORD ask (to podcrates, the lead files it): `pod-ws` keeps the stage of a failure of a dialer (`Proxy`, `Dns`) when
   it makes a `DialError` of it, as `pod-proxy` types it (`DialError::ProxyUnreachable`, `DialError::Resolve`); and
   `pod-warp` writes one safe line of the body in the line "does not decode".
2. podnet: a failure of the provisioning (`ProvisionError::Decode`, `Service`, `MissingResult`) gets its own hint in
   `src/report.rs`: "the quick-tunnel service did not answer with its API: a proxy or a gateway can answer with its own
   page; check the proxy and its allow-list for api.trycloudflare.com; a named tunnel (--token) does not use this
   service". The edge-register hint keeps a form with no "when ... retry" condition: one sentence for each case
   (a refused token, a retry after a wait).
3. podnet: `bench` keeps the first error as an `anyhow::Error` (not a string) and returns it; `relay connect` keeps the
   cause with `.context`, not `{e}`. Each command that dials gets its hint from `hop::of`.
4. One table-driven test: for each command (`connect --stdio`, `relay connect`, `bench`, `send`, `warp proxy
   --ephemeral` with the stand-in, `tunnel`) and each cause (a dead proxy `127.0.0.1:1`, a name that does not resolve
   with `--resolve` unset and no resolver, a proxy that answers a page that is not JSON): the second line of stderr is
   the hint of the true hop, and no hint of another hop. Each row that has a hint today stays a row. The rows `connect`,
   `relay connect` and `bench` fail before the fix.

## Prove

```sh
cargo test -p podnet --test error_words
cargo test -p podnet --lib report::
cargo test -p podnet-core hop::
```

## Correction

2026-11-23: the test of step 4 is `tests/error_words.rs` (one table with the rows of T-522), not a
file hop_hints. A refused port, an address with no answer, `send` and `warp proxy` are rows too. A refused port
had no second line: the stage `Connect` of `pod_core` had no hop. It is now the hop `connect`.

## Done

2026-11-23, the commit of this entry.

- The hint of the quick-tunnel service: `podnet_core::hop::quick_service_answer` reads a `ProvisionError` `Decode`,
  `Service` or `MissingResult` in the chain, and `src/report.rs` gives the second line "the quick-tunnel service did
  not answer with its API: a proxy or a gateway on the way can answer with its own page; ...". The hint of the hop
  `edge-register` has no "when the edge says retry" condition.
- A new hop `connect` (`crates/podnet-core/src/hop.rs`) for the stage `Connect`: "check the host and the port of the
  target, and that a service listens there; a host that reaches out only through a proxy needs --proxy".
  `src/fetch.rs` takes the hop of each stage (`hop::of_stage`).
- The stage of the proxy through `pod-ws`: the dialer of podnet gives a failure at the proxy its `DialError` as the
  cause (`src/egress.rs`, `dial_failure`), and `pod_ws::way::dial_error` keeps a cause of that type as it is.
  `connect` and `relay connect` get the hint `proxy`. The lead "the remote URL does not answer" is "the remote URL"
  for a failure of the proxy or of a name (`podnet::ws::remote_lead`).
- The second path for a crate that keeps only the text of a failed dial (pod-warp `EnrolError::Unreachable`,
  pod-wormhole `Failed::Unreachable`): `hop::of` reads the words of `pod_proxy::DialError` ("could not reach the
  proxy", "could not resolve") before the stage `Connect`. `send` and `warp proxy` get the hint `proxy` and `dns`.
- `bench` keeps its first error with its chain and returns its hop (exit 1, as its help says). `relay connect` keeps
  the cause of a failed connect (`src/relay_reverse.rs`, `connect_error`; `src/relay_reverse/resume.rs`).

Tests: `tests/error_words.rs`, `each_error_line_names_its_cause_once_and_says_what_to_do` (13 rows: `connect`,
`relay connect`, `bench`, `send`, `warp proxy` with a dead proxy 127.0.0.1:1; `connect`, `relay connect`, `bench`
on a refused port and on a name under `.invalid`; `bench` on 192.0.2.1; `tunnel` with a stand-in service that
answers 200 and a text page). Each row checks the exit code, the second line (the hint of the true hop) and the
words of the first line. Unit tests: `an_answer_that_is_not_the_api_has_the_hint_of_the_service` (`src/report.rs`),
`the_words_of_a_dial_give_the_hop_when_its_stage_is_lost` (`crates/podnet-core/src/hop.rs`).
`cargo test -p podnet --test error_words`: 2 passed. `cargo test -p podnet --lib report::`: 9 passed.

Control: the test binary against the binary of the code before the fix fails with 23 faults, among them:
"connect, a dead proxy: the second line is None, not "podnet: the proxy hop failed; "", the same for
`relay connect`, `bench`, `send` and `warp proxy`; "connect, a refused port: the second line is None"; "bench, a
name that does not resolve: the second line is None"; "tunnel, a service page that is not its API: the second line
is Some("podnet: the edge-register hop failed; when the edge says retry: no, check the token or the credentials
file; ...")".

COORD asks (to podcrates; the parts of #695 and #697 that are in the crates):

1. `pod-ws`, `PodUtils/podcrates:crates/pod-ws/src/way.rs`, `dial_error`: keep the stage `Proxy` of a failure of a dialer whose cause is
   not a `DialError` (a refusal of the proxy: 403, 407, 502), for example as `DialError::ProxyRefused` or with the
   stage in `DialError::Connect`. podnet gives a `DialError` cause only for a proxy that does not answer. Test: a
   `Dial` that fails with `Failure::caused(Stage::Proxy, Class::Refused, "...", io-error)` gives a `ConnectError`
   whose `stage_and_class().0` is `Stage::Proxy`.
2. `pod-warp`, `PodUtils/podcrates:crates/pod-warp/src/provider.rs:98` and `:234`: `EnrolError::Unreachable` keeps the failure of the
   API (its stage `Proxy` or `Dns`) as the source, not its text. `PodUtils/podcrates:crates/pod-warp/src/register.rs:89` and `:97`: an
   answer that is not JSON and does not start with `{` gets one safe line of its body (as
   `pod_cloudflare::quicktunnel::ProvisionError::Decode`). Test: an enrol through a dead proxy gives a chain with the
   stage `Proxy`; a 200 answer `pod-lab stand-in` gives a text with `pod-lab stand-in`.
3. `pod-wormhole`, `PodUtils/podcrates:crates/pod-wormhole/src/failed.rs:54`: `Failed::Unreachable` keeps the failure of each meeting
   point (its stage) as the source. Test: a meeting point behind a dead proxy gives a chain with the stage `Proxy`.


# T-522: error lines and rows name one cause two to six times, with the Debug text of a Rust struct, and one line holds three resolvers (GitHub #696, #697, #698)

**Source:** GitHub #696, #697 and #698 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23). The rule that they break:
the pass prompt, check 3, "one error line that names each cause once and says what to do."
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

#696, plain host (the system resolver has no SRV records for the edge name). `podnet serve --check --dir ./public --spa`,
row `edge` (shortened here):

```
  warn  edge          the system resolver cannot find the edge (SRV lookup of _v2-origintunneld._tcp.argotunnel.com: DNS error: no records found for Query { name: Name("_v2-origintunneld._tcp.argotunnel.com."), query_type: SRV, query_class: IN }: no records found for Query { name: Name("_v2-origintunneld._tcp.argotunnel.com."), query_type: SRV, query_class: IN }: DNS error: ... ); the run tries DNS over HTTPS, then the static list
```

The text `no records found for Query { name: Name(...), query_type: SRV, query_class: IN }` appears 6 times.
`podnet tunnel --check --json --url http://127.0.0.1:18080` has the same text in `checks[].detail`. Exit 0. Rows of
the origin repeat a cause too:

```
podnet tunnel --check --url http://127.0.0.1:1
  fail  origin 1      http://127.0.0.1:1: connecting to 127.0.0.1:1: Connection refused (os error 111): Connection refused (os error 111)
podnet tunnel --check --url http://nowhere.invalid:80
  fail  origin 1      http://nowhere.invalid:80: resolving the origin nowhere.invalid: failed to lookup address information: Name does not resolve: failed to lookup address information: Name does not resolve
```

#697: a dead proxy gives `podnet: the remote wss://example.com/__podnet/ws does not answer: TCP to example.com: could not
connect to example.com:443: example.com:443 through the proxy http://127.0.0.1:1: could not reach the proxy 127.0.0.1:1:
could not connect to 127.0.0.1:1: Connection refused (os error 111)`. "could not connect" and the target appear three
times. `relay connect` and `bench` have the same shape.

#698, lab `no-proxy-no-dns`: `podnet bench --duration 1 --timeout 3 http://nowhere.invalid/` gives one line of 567 bytes
that joins the failures of three resolvers: `... could not resolve nowhere.invalid: system resolver: failed to lookup
address information: Try again; DNS over HTTPS: 4 resolvers (2606:4700:4700::1111, ...): could not connect to [the
address of the resolver]:443: Address family not supported by protocol (os error 97); 4 resolvers (1.1.1.1, ...): could
not connect to t...`. On a plain host the line adds `TLS: invalid peer certificate: certificate not valid for name "the
address of the resolver"; certificate is only valid for DnsName("default.domain")` (a Rust debug form).

## Premise

Measured on 2026-11-23 with the debug binary of beta.17: the `edge` row (six copies), both `origin` rows and the
`connect` line are as the issues say.

- Read: `src/preflight.rs:45`, `:215`, `:263`, `:287` and `:478` print an error with `{err:#}` (anyhow joins each cause
  with `: `). A failure of podcrates holds the text of its cause in its own text (`pod_core::Failure::caused`), so the
  cause comes twice. `src/report.rs:62` has `chain_text`, which names a cause that is already in the line once. The
  rows of `--check` do not call it. The `origin` rows are the double text of `io_fail`
  (`PodUtils/podcrates:crates/pod-cloudflare/src/origin.rs:155`, `:370`, `:380`).
- Read: the `edge` row comes from the system resolver of podnet (`system_srv`, `src/tunnel.rs:282`). hickory writes
  `no records found for Query { ... }` with the Debug form of its `Query`. The row shows the error of hickory as it is.
- Read: `src/egress.rs:343` adds `{at} through the proxy {label}` to the error of a dial. `pod-ws` adds `could not
  connect to {at}` (`PodUtils/podcrates:crates/pod-ws/src/way.rs`, `dial_error`, which strips only
  `connecting to {target}` and `could not connect to {target}`). `pod-proxy` writes `could not reach the proxy P: could
  not connect to P`. The target and the proxy come two or three times.
- Read: `PodUtils/podcrates:crates/pod-dns/src/lookup.rs:86` (`fold`) groups the failures that read alike, but it
  keeps each group in the line. The line of #698 has three groups. The `DnsName("default.domain")` text is rustls's
  `Display` (`pod-tls`). The line of #698 is not cut anywhere: no bound for the length of an error line exists.
- Fault of the words: in podnet (`src/preflight.rs`, `src/tunnel.rs`, `src/egress.rs`) and in podcrates (`pod-ws`,
  `pod-proxy`, `pod-dns`, `pod-tls`).
- Not measured: the `--json` form of the rows after the fix (the key `detail`).

## Approach

1. podnet: one function gives the text of an error for a row or a line (`report::chain_text`, or a new `words(&err)` that
   calls it). `src/preflight.rs` uses it for each row. A `{err:#}` in a line for the user is a fault: step 5 covers
   each row.
2. podnet: `system_srv` (`src/tunnel.rs:282`) maps `NoRecordsFound` to one sentence: `no SRV records for NAME at the
   system resolver`. It keeps no Debug form of a struct. Before that, read `pod-dns` for a lookup of SRV records
   (shared code first).
3. podnet: `src/egress.rs:343` names the target once. The line of a dial is
   `TARGET through the proxy PROXY: could not reach the proxy PROXY: Connection refused (os error 111)`, or shorter.
4. COORD ask (to podcrates, the lead files it): (a) `pod-ws` `dial_error` strips the target lead of a dialer that writes
   `TARGET through the proxy`, and `pod-proxy` names the proxy once in `could not reach the proxy`; (b) `pod-dns`
   gives one short line for a name that does not resolve: one phrase for each source (the system resolver, DNS over
   HTTPS) and a bound of 200 bytes; (c) `pod-tls` writes the name of a certificate without the Debug form
   (`DnsName("...")`).
5. One table-driven test over the rows of `--check` and the error lines of the commands that dial: for each cause
   (no SRV records, a refused origin, an origin name that does not resolve, a dead proxy, a name that no resolver
   answers) the text names the cause once, has no `Query {`, `DnsName(` or `Name(`, and is shorter than 300 bytes. The
   rows `edge` and `origin` and the line of `connect` fail before the fix.

## Prove

```sh
cargo test -p podnet --test tunnel_config
cargo test -p podnet --test error_words
cargo test -p podnet-core exit::
cargo test -p podnet --lib tunnel::tests::a_failed_srv_lookup_has_no_debug_form
cargo test -p podnet --lib preflight
```

## Done

2026-11-23, the commit of this entry. Partial: the part of podnet is done; the length of a line for a name that no
resolver answers waits for `pod-dns` (ask 1 below).

- One function gives the words of an error for a line and a row: `podnet_core::exit::text` (and `text_of` for an
  error of a crate), which `podnet::report::chain_text` calls. It names a cause that the line holds once, says a
  part that comes again at once one time (`exit::tidy`: `A: B: B` is `A: B`), and writes the Debug form of a name of
  a certificate of webpki as the name (`DnsName("default.domain")` is `default.domain`).
- Each row of `--check` (`src/preflight.rs`) uses it; no `{err:#}` is left there. The failures of the glue
  (`src/tunnel.rs`, `failure`) and of the dialers (`src/egress.rs`, `dial_failure`) use it too.
- `system_srv` (`src/tunnel.rs`) writes a failed SRV lookup in one phrase (`srv_words`: "no SRV records", "the name
  does not exist (NXDOMAIN)", "the resolver answered CODE", "no answer in time"). `pod-dns` has no lookup of SRV
  records by the system resolver; `pod-cloudflare` asks the consumer for it (`SystemSrv`).
- `src/egress.rs` names the proxy once: `TARGET through the proxy URL: could not reach the proxy P: Connection
  refused (os error 111)` (`proxy_named_once`).

Measured after the fix on this host (no SRV records at the system resolver): the `edge` row is "the system resolver
cannot find the edge (SRV lookup of _v2-origintunneld._tcp.argotunnel.com: no SRV records); the run tries DNS over
HTTPS, then the static list" (915 bytes before); the `origin` rows name their cause once; the line of `connect`
through a dead proxy is "podnet: the remote wss://example.com/__podnet/ws: TCP to example.com: could not reach the
proxy 127.0.0.1:1: Connection refused (os error 111)".

Tests: `tests/error_words.rs`, `each_row_of_a_check_names_its_cause_once` (`tunnel --check` with a refused origin
and an origin name under `.invalid`, `serve --check --spa`: each row says no part two times, has no Debug form, and
is shorter than 300 bytes) and `each_error_line_names_its_cause_once_and_says_what_to_do` (the rows of T-521: the
first line says no part two times, says "could not connect to" and its target at most once, has no Debug form, and is
shorter than 300 bytes except for a name under `.invalid`). Unit tests: `tidy_says_each_part_once_and_no_debug_form`
(`crates/podnet-core/src/exit.rs`, 7 cases), `a_failed_srv_lookup_has_no_debug_form` (`src/tunnel.rs`, 4 cases).

Control: on the binary of the code before the fix, `each_row_of_a_check_names_its_cause_once` fails: "tunnel --check
--url http://127.0.0.1:1: has the Debug form "Query {"", "a row of 915 bytes", "says "Connection refused (os error
111)" two times", "tunnel --check --url http://nowhere.invalid:80: says "failed to lookup address information: Name
or service not known" two times", and the same `edge` faults for `serve --check`. The row test of T-521 fails with
"connect, a dead proxy: says "example.com:443" 2 times", "says "could not connect to" 2 times", "send, a dead proxy:
a line of 575 bytes", and "has the Debug form "DnsName("" for each row of a name under `.invalid`.

COORD asks (to podcrates):

1. `pod-dns`, `PodUtils/podcrates:crates/pod-dns/src/lookup.rs:62` (`fold`): one short line for a name that does not resolve, with a
   bound of 200 bytes: one phrase for each source (the system resolver, DNS over HTTPS), the count of the resolvers,
   and the full list at the debug level. Test: the failures of #698 (the system resolver, 4 resolvers by IPv4 with a
   TLS failure, 4 by IPv6 with EAFNOSUPPORT) give a line shorter than 200 bytes that names each source once. The line
   of `bench` on this host is 631 bytes after the fix of podnet (642 before).
2. `pod-tls` (the words of a TLS failure): write the names of a certificate plain. rustls-webpki gives
   `DnsName("x")` in `CertificateError::NotValidForNameContext`. Test: a failure with the presented name
   `DnsName("default.domain")` gives a text with `default.domain` and no `DnsName(`. podnet's second path is
   `podnet_core::exit::tidy`.
3. `pod-proxy`, `PodUtils/podcrates:crates/pod-proxy/src/stream.rs:222` and `:235`: `DialError::ProxyUnreachable` holds the text of the
   connect as its detail ("could not connect to P: ..."), and its words name the proxy again ("could not reach the
   proxy P: could not connect to P: ..."). Strip the lead `could not connect to P`. Test: the text of a refused
   proxy names the proxy once.
4. `pod-ws`, `PodUtils/podcrates:crates/pod-ws/src/way.rs:218` (`dial_error`): strip the lead `TARGET through the proxy P` of a dialer
   too, not only `connecting to TARGET` and `could not connect to TARGET`. Test: a failure with the text
   `x:443 through the proxy http://p:1: refused` gives a text that names `x:443` once.
5. `pod-cloudflare`, `PodUtils/podcrates:crates/pod-cloudflare/src/origin.rs:155` and `PodUtils/podcrates:crates/pod-cloudflare/src/fail.rs:90`
   (`io_fail`): the failure holds the text of its error of I/O and the error as its source, so `{:#}` says the cause
   two times. podnet's rows use `chain_text`. Test: `format!("{:#}", anyhow::Error::new(failure))` of a refused
   origin says "Connection refused" once.

2026-11-24 (T-542, the take of podcrates `v0.83.0`): the rest is done. `pod-dns` gives one short line for a name that
no resolver answers (asks 1 to 5 of COORD-115, PodUtils/podcrates@T-1129 to PodUtils/podcrates@T-1134). `tests/error_words.rs` holds each
row to 300 bytes; the exemption of a name under `.invalid` (the field `short`) is gone. Control: `podnet --no-proxy
connect --stdio wss://nowhere.invalid/__podnet/ws` gives a line of 612 bytes on `v0.82.1` and of 273 bytes on
`v0.83.0` ("DNS over HTTPS: 8 resolvers failed, 4 with: TLS: ..."). `cargo test -p podnet --test error_words`: 2
passed.

# T-523: a listener that cannot bind: two lines for one cause, no next step, and the bind comes after the WARP session (GitHub #705, #706)

**Source:** GitHub #705 and #706 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23). The rule that they break: the
pass prompt, check 3, "one error line that ... says what to do."
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

#705, plain host (port 18080 in use by another process):

```
podnet warp proxy --listen tcp://127.0.0.1:18080 --run-for 2
podnet: warp proxy: tcp://127.0.0.1:18080 is not available: binding tcp://127.0.0.1:18080: Address in use (os error 98)
podnet: warp proxy: a listener did not start: binding tcp://127.0.0.1:18080: Address in use (os error 98)
```

Exit 69. Time: 2.3 s (the registration and the tunnel start first). `net proxy`, `forward` and `serve --local` print one
line for the same cause. `podnet warp proxy --detach --name wu --ephemeral --listen tcp://127.0.0.1:18080` prints the
same two lines. Two lines name the same bind error and the same address. Neither says what to do (use another port, or a
unix socket). The proxy opens the WARP session (2 s) before it finds that no listener binds; the bind error could come
first. The help of `--listen` says: "A listener that the host refuses is reported, and the proxy starts with the others;
with none, the exit code is ...".

#706, lab `bind-denied` (TCP bind and connect of AF_INET give EACCES; AF_UNIX holds):

```
podnet forward -L 18081=127.0.0.1:18080 --run-for 2
podnet: -L 127.0.0.1:18081: Permission denied (os error 13)          exit 69
podnet net proxy --listen tcp://127.0.0.1:18082 --run-for 2
podnet: binding tcp://127.0.0.1:18082: Permission denied (os error 13)   exit 69
podnet serve ./public --local 127.0.0.1:18090 --run-for 4
podnet: --local 127.0.0.1:18090: binding 127.0.0.1:18090: Permission denied (os error 13)   exit 69
podnet agent up --expose 18080
... "exposed":[{"port":18080,"state":"origin-unreachable","error":"nothing accepts a connection on 127.0.0.1:18080"}] ...
```

The same commands with `unix:/path` listeners work in the lab. `podnet net doctor` (same lab) knows the fix:
"listen(AF_UNIX): warp proxy --listen unix:, connect -L unix://, net proxy, ...". `podnet agent up` prints the advice
"TCP on 127.0.0.1 does not work here: use unix sockets (podnet tunnel --unix, podnet net)". The line of the bind is the
bare errno: it does not say that the host refuses TCP and does not name the unix form. In `agent up`, the origin check
says "nothing accepts a connection" when the cause is EACCES of the connect.

## Premise

- Read: `src/cli/cmd/warp/front.rs:144` prints one line for each listener that did not bind, and `:156` fails with a
  second line that joins the same errors. With one listener, the two lines are equal in cause.
- Read: `run` of `src/cli/cmd/warp/front.rs:206` takes a `client` that the caller has made
  (`src/cli/cmd/warp/mod.rs:156`, `warp_client`, which registers a device for `--ephemeral`): the registration comes
  before `bind_all` (`src/cli/cmd/warp/front.rs:134`). The comment of issue 97 says that the listeners
  bind before the session; that is true of the session of the proxy, not of the registration.
- Read: `podnet_core::net::bind_fault` (`crates/podnet-core/src/net.rs:162`) is the one place that gives the class to a
  refused bind. It adds no words. `src/cli/cmd/local.rs:65` writes its own line (`serve --local`).
- Read: `src/cli/cmd/agent.rs:104` calls `podnet::ports::reachable(*p, ...)`, which gives a bool: a refused connect and
  an EACCES give the same answer. The advice with the unix sockets is in `src/agent.rs:529` and is not used here.
- Not read: the `warp forward` lines (`src/cli/cmd/warp/front.rs:276`, `:291`) have the same form by the same reading.

## Approach

1. One place writes the line of a refused bind: `bind_fault` (and `bind_tcp`) adds the next step by the kind of the
   error: `PermissionDenied` on a TCP address: "this host refuses a TCP listener; give a unix socket
   (unix:PATH)"; `AddrInUse`: "the address is in use; give another port (:0 gives a free one) or a unix socket". The
   commands that write their own line (`serve --local`, `forward`, `connect`) go through it. The exit code stays 69
   (T-515).
2. `warp proxy` and `warp forward` print the list of the listeners that did not start once (one line with each
   address), not one line for each and then the joined line.
3. `warp proxy` binds its listeners before the registration of the device and before the tunnel (a bind error then
   costs no 2 s and no registration). The `--check` path does not bind.
4. `agent up --expose`: the check of the origin keeps the error of the connect. EACCES gives `the host refuses a TCP
   connection to 127.0.0.1:PORT: use unix sockets (podnet tunnel --unix, podnet net)`; a refusal gives the text of today.
5. One table-driven test (the rows of tests/listener_bind.rs of T-515): for each command with a listener and each
   cause (the address in use; EACCES, in the lab row of `scripts/lab-lane.sh`, case `errors`): one line on stderr that
   names the cause and the address once, and one next step; `warp proxy` with the stand-in checks the order (no request
   to the stand-in before the bind error). The two lines of `warp proxy` and the bare errno fail before the fix.

## Prove

```sh
cargo test -p podnet --test listener_bind
cargo test -p podnet --test warp_stand_in
```

## Done

2026-11-23, the commit of this entry. `podnet_core::net::bind_fault` writes the one line of a refused bind: the
cause, the address one time, `; ` and the next step by the kind of the error and of the listener (`bind_step`: another
port or a unix socket for an address in use; a port above 1023 or a unix socket, and `podnet net doctor`, for EACCES;
another path for a unix socket). `serve --local`, `git serve --ssh` and `connect` use it. `warp proxy` and `warp
forward` bind in `front::bind_early`, before the registration of a device (`warp_client`) and before the session, and
fail with one line that lists each listener that did not start. `agent up --expose` names the cause of a failed origin
check (`podnet::ports::origin`): EACCES gives "the host refuses a TCP connection to 127.0.0.1:PORT: use unix sockets
(podnet tunnel --unix, podnet net)". The test of T-515 (tests/listener_bind.rs) checks one line, the address one time,
a next step and no request to the stand-in of the WARP API. Before the fix: "serve ... --local 127.0.0.1:43195: the
line names 127.0.0.1:43195 2 times"; "net proxy --listen tcp://127.0.0.1:43195: the line gives no next step" (also
forward and git serve --ssh); "warp proxy --ephemeral --listen tcp://127.0.0.1:43195: the line names 127.0.0.1:43195 0
times / podnet: registering a WARP device: the provider's API did not answer" (the registration came first; also warp
forward). The unit test `the_origin_line_follows_the_cause_of_the_connect` (crates/podnet-core/src/ports.rs) holds the
origin lines. `cargo test -p podnet --test warp_stand_in` passes.


# T-524: net doctor: a host with no loopback gets "not needed", and a TCP loopback listener starts and cannot be reached (GitHub #692)

**Source:** GitHub #692 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Lab `no-loopback` (no address on the loopback; AF_UNIX holds). `podnet net doctor` shows

```
bind(AF_INET, 127.0.0.1:0)    ok       allowed
listen(AF_INET, 127.0.0.1:0)  ok       allowed (SO_ACCEPTCONN=1)
...
podnet-vnet: not needed: this host allows AF_INET bind and listen, so programs use real addresses (...)
```

and exits 0. Then:

```
podnet net proxy --listen tcp://127.0.0.1:1081 --run-for 4 &
curl -sS -m 3 -x socks5h://127.0.0.1:1081 http://10.66.0.2/
```

podnet prints `podnet: proxy    tcp://127.0.0.1:1081` with no warning. curl prints `curl: (7) Failed to connect to 127.0.0.1
port 1081 after 0 ms: Couldn't connect to server`. In this lab a connect to 127.0.0.1 fails (`Network unreachable`, as
`podnet doctor` shows for each address). The promise: `docs/networks.md`, "`podnet net doctor` calls `bind` and then
`listen` for each stream socket ... It prints the commands that need each listener." The lab profile `no-loopback` says
"no address on the loopback; AF_UNIX holds". The doctor tests bind and listen only. It never tests a connect to the
loopback. It says that no virtual network is needed and that `net proxy --listen tcp://` works. The doctor has no row and
no advice to use a unix socket.

## Premise

- Read: `crates/podnet-vnet/src/doctor.rs:153` (`verdict`) returns "not needed" when the two rows `bind(AF_INET,
  127.0.0.1:0)` and `listen(AF_INET, 127.0.0.1:0)` are `ok`. The probes (`crates/podnet-vnet/src/doctor.rs:539`) have
  no connect.
- Read: `podnet agent` has the probe that is missing: `localhost()` of `src/agent.rs` binds a listener on 127.0.0.1 and
  connects to it. `src/agent.rs:527` turns its failure into the advice "TCP on 127.0.0.1 does not work here: use unix
  sockets". The two doctors disagree on the same host.
- The code is podnet's own (`crates/podnet-vnet`). Not measured: a loopback that has `127.0.0.1` but drops the
  packets.

## Approach

1. `net doctor` adds the rows `connect(AF_INET, 127.0.0.1)` and `connect(AF_UNIX, path)` (a listener and a connect to
   it, as `localhost()` does; take one function for both doctors).
2. `verdict` says "not needed" only when the connect works. With a failed connect and a working AF_UNIX it says
   `podnet-vnet: TCP on the loopback does not work here: use unix sockets (warp proxy --listen unix:, net proxy, connect
   -L unix://)`, with the commands of `NEEDS` (`crates/podnet-vnet/src/doctor.rs:122`).
3. One table-driven test of `verdict` and of the probe set: the rows (bind and listen ok with connect ok; with connect
   failed; bind denied; nothing works) and the lab `no-loopback` in `scripts/lab-lane.sh` (case `doctor`). The
   row with a failed connect fails before the fix ("not needed").

## Prove

```sh
cargo test -p podnet-vnet doctor
cargo test -p podnet --test doctor
```

## Done

2026-11-23, the commit of this entry. `podnet_core::ports::loopback_connect` (a TCP listener on 127.0.0.1 and a
connect to it, with a check of the address before the connect) and `unix_connect` are the one probe of both doctors:
`podnet agent` (`localhost()` of `src/agent.rs`, with the egress policy as its check) and `podnet net doctor` (the new
rows `connect(AF_INET, 127.0.0.1)` and `connect(AF_UNIX, path)`). `verdict` says "not needed" only when the TCP
connect works; with bind and listen ok and the connect refused it says `podnet-vnet: TCP on the loopback does not
work here: use unix sockets (warp proxy --listen unix:, net proxy, connect -L unix://)`, and with no connect at all
"not ready". `docs/networks.md` names the rows; `scripts/lab-lane.sh` (case `doctor`) fails when `net doctor` says
"not needed" with a failed loopback connect. The tests: `doctor_verdict_needs_a_loopback_connect`
(`crates/podnet-vnet/src/doctor/tests.rs`: connect works; no loopback; no connect at all; bind denied; and the probe
set has both rows) and `loopback_connect_names_the_failed_step` (`crates/podnet-core/src/ports.rs`). `cargo test -p
podnet-vnet doctor`: 4 passed. `cargo test -p podnet --test doctor`: passed. Control: with the verdict before the fix
the test fails with `no loopback: podnet-vnet: not needed: ...`. The lab `no-loopback` was not run in this session.


# T-525: net shim: the documented `--out /tmp/podnet-vnet` example prints a working LD_PRELOAD line on a noexec /tmp (GitHub #699)

**Source:** GitHub #699 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Lab `noexec-tmp` (`/tmp` is a tmpfs with `noexec`). The example of `docs/networks.md` (the first `sh` block):

```
podnet net shim --out /tmp/podnet-vnet
podnet: shim     /tmp/podnet-vnet/libpodnet-vnet.so
podnet: origin   Compiled
  export LD_PRELOAD=/tmp/podnet-vnet/libpodnet-vnet.so
  ...                                                           exit 0
LD_PRELOAD=/tmp/podnet-vnet/libpodnet-vnet.so /bin/true
ERROR: ld.so: object '/tmp/podnet-vnet/libpodnet-vnet.so' from LD_PRELOAD cannot be preloaded (failed to map segment from shared object): ignored.
```

`mount` says `tmpfs on /tmp type tmpfs (rw,nosuid,nodev,noexec,...)`. Without `--out`, `podnet net shim` picks
`~/.local/state/podnet/podnet-vnet/` and works. The promise: `docs/networks.md:27` (the example), and
`docs/target-environment.md:147`, "The temporary directory is `noexec` | A shim goes to the first directory that is
writable and not `noexec`". The documented example writes the shim to a `noexec` directory. podnet gives no warning and
no error, and the printed export cannot load. An explicit `--out` is not checked for `noexec`.

## Premise

- Read: `crates/podnet-core/src/state.rs:288` (`allows_exec`, `statvfs` with `ST_NOEXEC`) and `exec_dir` choose a
  directory that is not `noexec` when no `--out` is given (`src/cli/cmd/net/shim.rs:41`, `:165`, `:204`). With `--out`,
  the directory is used as it is (`src/cli/cmd/net/shim.rs:37` to `:40` and `:163` to `:162`). `net shim`, `net socksify`
  and `net ts-shims` have the same fault. `net passwd-shim` was not read.
- The code is podnet's own. Not measured: `net socksify --out` and `net ts-shims --out` in the lab (the reading says
  the same).

## Approach

1. One function takes the explicit `--out` and checks `allows_exec`. The call: a `noexec` directory that the user
   names is an error (exit 78, a configuration that cannot be used), one line: `/tmp/podnet-vnet is on a noexec file
   system: a shim cannot load from it; give another --out, or none (podnet uses DIR)`, with the directory that
   `exec_dir` would take. A fallback with a warning would hide the path that the user wrote into `LD_PRELOAD`.
2. The example of `docs/networks.md:27` uses no `--out /tmp/...`: it uses `--out "$HOME/.local/share/podnet-vnet"` or no
   `--out`. The sentence of `docs/target-environment.md:147` says that an explicit `--out` is checked.
3. One table-driven test over the commands (`net shim`, `net socksify`, `net ts-shims`, `net passwd-shim`): an
   explicit `--out` on a `noexec` directory gives exit 78 and the line; on a normal directory it works. A test cannot
   mount `noexec`: the test takes the check as a function with the `statvfs` flag as an argument, and the lab
   `noexec-tmp` row in `scripts/lab-lane.sh` runs the commands.

## Prove

```sh
cargo test -p podnet-core state
cargo test -p podnet --test shim_direct
```

## Done

2026-11-23, the commit of this entry. `podnet_core::state::shim_dir` takes the `--out` of `net shim`, `net
socksify`, `net ts-shims` and `net passwd-shim` (`src/cli/cmd/net/shim.rs`). With no `--out` it is `exec_dir`. An
`--out` whose nearest existing directory is on a `noexec` file system (`allows_exec`) gives exit 78 and one line:
`DIR is on a noexec file system: a shim cannot load from it; give another --out, or none (podnet uses DIR2)`. There
is no silent fallback. The examples of `docs/networks.md` and `docs/virtual-network.md` use no `--out /tmp/...`;
`docs/target-environment.md` and the help of `--out` say that a `noexec` file system is refused. The tests:
`state_shim_dir_refuses_an_explicit_noexec_out` (`crates/podnet-core/src/state.rs`: the check as an argument) and
`an_explicit_out_on_a_noexec_mount_exits_78` (`tests/shim_direct.rs`: a user and mount namespace mounts a `noexec`
tmpfs and an exec tmpfs; the four commands with `--out` on the `noexec` tmpfs exit 78 with the line; `net
passwd-shim` on the exec tmpfs exits 0; a host that refuses the namespace skips). `cargo test -p podnet-core state`
and `cargo test -p podnet --test shim_direct`: passed (12 tests). Control: on the code before the fix the test fails
with each `noexec` row at exit 0 (`noexec shim 0 2 podnet: shim     .../noexec/shim/libpodnet-vnet.so`).


# T-526: docs/host-runtime.md: the examples `podnet fetch` and `podnet init generate --node` do not run (GitHub #701)

**Source:** GitHub #701 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Plain host. Example 1, `docs/host-runtime.md:142`:

```
PODNET_POLICY=rules.json podnet fetch https://example.net/file
error: unrecognized subcommand 'fetch'
  tip: a similar subcommand exists: 'bench'                      exit 64
```

`podnet fetch` is in no `podnet --help` list. Example 2, `docs/host-runtime.md`, section "The initial configuration":

```
podnet init generate --node node-a --relay http://relay.example
error: unexpected argument '--node' found
Usage: podnet init generate [OPTIONS] --relay <URL> <NODE>      exit 64
```

The command takes NODE as an argument: `podnet init generate node-a --relay http://relay.example` works. The text of the
document says "`podnet init generate --node NAME --relay URL` writes a configuration"
(`docs/host-runtime.md:102`; the example is at `:114`). The example `podnet init enroll state --node node-a --relay
http://relay.example --offline` runs as written. The promise: the pass prompt, check 1, "Each example of the help and of
the documents runs as written."

## Premise

- Read: `src/fetch.rs` is a library module of the client with proxy and policy; no subcommand `fetch` exists in
  `src/cli/mod.rs`. `podnet bench URL` is the command that makes requests. The example names a command that podnet
  does not have.
- Read: `src/cli/help_tests.rs:220` (`each_example_of_the_manual_parses`) parses the examples of the manual model
  (`podnet::manual::facts::EXAMPLES`) with clap. Nothing parses the fenced blocks of `docs/*.md` and `README.md`
  (293 lines of the documents start with `podnet `). `tests/claims_docs.rs` checks the claims of three documents, not
  the commands.
- The conflict is only in the documents: the code of `init generate` takes a positional NODE (as `init enroll` takes a
  positional `state`), and no `fetch` command exists. The call: change the documents, not the code.

## Approach

1. `docs/host-runtime.md:142` uses a command that exists: `PODNET_POLICY=rules.json podnet bench https://example.net/file`,
   or a `podnet connect` line. The sentence at `:102` and the example at `:114` give `podnet init generate NODE --relay
   URL`.
2. One table-driven test, `each_command_of_a_document_parses` in `src/cli/help_tests.rs` (or a new test file): it reads each fenced `sh`
   block of `README.md` and `docs/*.md`, joins the lines that end with `\`, drops a leading `VAR=value`, and parses
   each line that starts with `podnet ` with clap (as the test of the manual does; `podnet pod` goes to its own parser
   in `crates/podnet-netns`). A short list of lines with a placeholder (`PATH`, `<NAME>`) is allowed, each with the
   reason. The test lists each document line that does not parse. It fails before the fix for the two lines of the issue,
   and for each other line that it finds: the entry lists them in its Done section.
3. Each fault that the test finds in other documents is fixed in the same commit (the documents change; a command that a
   document needs and the code does not have becomes its own entry).

## Prove

```sh
cargo test -p podnet each_command_of_a_document_parses
cargo test -p podnet --test claims_docs
```

## Done

2026-11-23, the commit of this entry.

- Cause: the documents named a command that does not exist (`podnet fetch`) and a flag that does not exist
  (`init generate --node`; NODE is positional). No test parsed the command lines of the documents.
- Change: `docs/host-runtime.md` gives `podnet init generate NODE --relay URL` (the sentence and the example) and
  `PODNET_POLICY=rules.json podnet bench --duration 1 https://example.net/file` (`bench` sends through
  `podnet::fetch`, which asks the policy). The test found four more faults, fixed in the same commit:
  `docs/harness.md` (a usage line with `[--json]`: now two command lines), `docs/link.md:446` (`link listen` with no
  `--allow`), `docs/link.md:447` (`link dial` has no `--to`: now `--peer FINGERPRINT`) and `docs/serve.md:61`
  (`link connect` with `--peer` and `--allow`, which clap refuses together: now `--allow hub.allow`).
  `scripts/doc-commands.txt` has the new hashes of the five blocks (same classes).
- Test: `each_command_of_a_document_parses` (`src/cli/help_tests.rs`). The extraction is the one of
  `scripts/doc-commands.py` (new mode `--lines`: each `podnet` command of each fenced `sh` block of `README.md`
  and `docs/*.md`; continued lines are joined, leading `VAR=value` words are dropped, a `<NAME>` placeholder is a
  value; a pipe, a list operator, a redirection and `$(` end a command). The test parses each line with the clap
  model; `podnet pod` has its own parser. Seven lines with a placeholder (`...`, `PORT`, `TAG`) are in a list,
  each with its reason. The documents have 311 lines.
- Control: on the documents before the fix the test fails with 6 lines: `docs/harness.md:8` (unexpected argument
  `[--json]`), `docs/host-runtime.md:114` (unexpected argument `--node`), `docs/host-runtime.md:142`
  (unrecognized subcommand `fetch`), `docs/link.md:446` (required `--allow`), `docs/link.md:447` (unexpected
  argument `--to`), `docs/serve.md:61` (`--peer` cannot be used with `--allow`).
- `cargo test -p podnet each_command_of_a_document_parses`: 1 passed. `cargo test -p podnet --test claims_docs`:
  3 passed. `python3 -I scripts/doc-commands.py --check`: 135 blocks, each with a class.

# T-527: pod --serve: a refused target gives SOCKS5 reply 0x01; docs/proxies.md says 0x04 for "refused" (GitHub #702)

**Source:** GitHub #702 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Plain host:

```
podnet pod --serve unix:/var/tmp/qa/work/p.sock -x direct &
python3 s5.py /var/tmp/qa/work/p.sock 127.0.0.1 1         # SOCKS5 CONNECT to a closed port (Connection refused)
b'\x05\x01\x00\x01\x00\x00\x00\x00\x00\x00' rep= 1
python3 s5.py /var/tmp/qa/work/p.sock nowhere.invalid 80
rep= 1
python3 s5.py /var/tmp/qa/work/p.sock 192.0.2.1 80
rep= 1
```

(`s5.py` sends the SOCKS5 greeting `05 01 00`, then a CONNECT request, and prints the reply.) With HTTP CONNECT the door
answers 502 for each case. The promise: `docs/proxies.md:103` to `:105`, "any other failure is a general failure (or a
host unreachable, when the text says refused or denied) and HTTP 502." A refused target is "Connection refused (os error
111)": the text says "refused", so the reply should be SOCKS5 `0x04` by this sentence. The door gives `0x01` (general
failure) for a refused port, for a name that does not resolve and for a network that is unreachable. The client
(`podnet pod`, curl: `Can't complete SOCKS5 connection to 127.0.0.1. (1)`) cannot tell these classes apart. Either the
document or the door is wrong.

## Premise

- Read: `socks_code` (`crates/podnet-netns/src/door.rs:453`) gives `0x02` for `Class::Policy`, `0x04` for
  `Class::Timeout`, and for the rest `socks5_code_for_error(&e.to_string())`
  (`crates/podnet-netns/src/socks.rs:156`): `0x04` when the text holds "denied", "acl" or "refused", else `0x01`.
- Read: `e.to_string()` of a `pod_core::Failure` is its own text, with no cause (`Failure` has `text()` and
  `chain()`). A dial failure that wraps its cause (`Failure::caused`) holds "Connection refused" in the cause (the
  `io::Error`), not in its text. Then the words are not found and the reply is `0x01`, as the tester saw. Not measured:
  the text that the door gets for each of the three cases. The document is right about the rule as written; the code
  reads the wrong text.
- Read: the rule of the document is odd. RFC 1928 has a code for each case: `0x03` network unreachable, `0x04` host
  unreachable, `0x05` connection refused, `0x06` TTL expired. `0x04` for "denied" is wrong by the RFC too (a denial is
  `0x02`). The failure has the typed cause: `Failure::io_cause()` gives the kind of the `io::Error`.
- The code is podnet's own. Not measured: the reply for the `Class::Policy` row and the early-reply path
  (`crates/podnet-netns/src/frontdoor.rs`).

## Approach

1. The call (the document and the door change together): the door maps the typed cause, not words. `ConnectionRefused`
   gives `0x05`, a network that is unreachable `0x03`, a host that is unreachable or a name that does not resolve
   `0x04`, `Class::Timeout` `0x06` or `0x04` (keep `0x04`, the documented value), `Class::Policy` `0x02`, any other
   failure `0x01`. The HTTP side keeps 403, 504 and 502. The word search of `socks5_code_for_error` goes away.
   The other choice, which the lead may take: keep the rule of the document (refused gives `0x04`) and read
   `failure.chain()`. Curl and other clients show a different text for `0x05`, so the first choice helps the user more.
2. `docs/proxies.md:103` to `:105` states the table of the replies. The manual (`podnet man`) gets the same table if it
   has the text.
3. One table-driven test: for each cause (a closed port, a name that does not resolve, an unreachable network, a
   policy refusal, a timeout, an other failure) and each client form (SOCKS5 CONNECT, HTTP CONNECT): the reply code or
   the status. The rows with a refused port, a name that does not resolve and an unreachable network fail before the fix
   (each is `0x01`).

## Prove

```sh
cargo test -p podnet-netns door
cargo test -p podnet --test pod_early_reply
```

## Done

2026-11-23, the commit of this entry. The call of the lead: the door maps the typed cause to the codes of RFC 1928
(`socks_code` of `crates/podnet-netns/src/door.rs`): `0x02` a policy refusal; the code of an upstream that refused
(`UpstreamRefused`); `0x05` `ConnectionRefused`; `0x03` `NetworkUnreachable`; `0x04` `HostUnreachable`, a name with
no address (`NoAddress`, `connect_direct` of `crates/podnet-netns/src/upstream.rs` now resolves first) or a time
limit; `0x01` any other failure. The door no longer reads words (`socks5_code_for_error` is not called; it stays in
`crates/podnet-netns/src/socks.rs`, a file that podcrates imported). The front door of `pod --serve` keeps the cause
(`Failure::caused`, `crates/podnet-netns/src/frontdoor.rs`). HTTP keeps 403, 504 and 502. `docs/proxies.md` has the
table. The tests: `door_socks_reply_code_by_cause` (`crates/podnet-netns/src/door/tests.rs`: 9 causes, also a failure
whose words say "denied" and "refused": `0x01`) and `serve_gives_the_socks5_reply_code_of_the_cause`
(`tests/pod_early_reply.rs`: `pod --serve -x direct`, a refused port gives `0x05` and a name with no address `0x04`;
HTTP CONNECT gives 502 for each). `cargo test -p podnet-netns door` and `cargo test -p podnet --test
pod_early_reply`: passed (5 tests). Control: on the code before the fix the test fails with `a refused port: SOCKS5
0x01 (want 0x05)` and `a name with no address: SOCKS5 0x01 (want 0x04)`. Not measured: an unreachable network on a
real host (the unit row holds it).


# T-528: doctor: exit 1 when no leg works; `podnet man exit-status` says 69 for "no edge leg worked" (GitHub #703)

**Source:** GitHub #703 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Lab `no-proxy-no-dns` (also `no-loopback`, and `proxy-only` without `--way-out`):

```
podnet doctor
...
podnet doctor: 0 of 4 legs can work on this host
next: no leg works: read the proxy and path lines above, then podnet bug-report --doctor for an issue
podnet: no leg can work on this host; the doctor gives the reason for each
```

Exit code 1; `podnet doctor --json` also exits 1. The promise, in two forms that disagree:

- `podnet man exit-status`: "69 (unavailable) A service or a network path is not available: no edge leg worked, the
  quick-tunnel service, the WARP API or the MASQUE endpoint did not answer." `docs/cli.md:139`: "69 when a service or a
  network path is not available (no edge leg worked)".
- `docs/doctor.md:46` and `podnet doctor --help`: "Exit 0: at least one leg can work. Exit 1: none can." The table says
  "1 (failure): A failure that has no other code."

`bench` has the same case ("no request worked": exit 1). Also, `podnet doctor --help` lists the legs "quic, http2,
quic+warp" for exit 0, while the output counts 4 legs (it adds `http2+relay`). It is not clear whether a host where only
the relay leg works gives exit 0.

## Premise

- Read: `src/cli/cmd/host.rs:406` bails with a plain error when `report.ok()` is false: exit 1.
  `Report::ok` (`src/doctor.rs:251`) is true when any of the legs has the verdict `ok`, the relay leg too. Thus a host
  where only `http2+relay` works gives exit 0, and the help (`src/cli/mod.rs:291`) lists three legs only.
- Read: the exit 1 of `doctor` is in the help, in `docs/doctor.md`, in the research list
  (`docs/research/podnet/parity-checklist.md`), and in the lane (`scripts/lab-lane.sh`, the case
  `proxy-doctor` runs `doctor` through a proxy that refuses). The manual row of 69 (`src/manual/facts.rs:203`) and `docs/cli.md:139` name "no edge
  leg worked" for the commands that start a tunnel (`tunnel`, `warp`); `podnet tunnel` with no quick-tunnel service gives 69 (#695).
- The call (the owner may rule otherwise; `TODO/PROGRESS.md`, Calls made without a ruling): keep the exit 1 of `doctor`
  and `bench`. A command whose job is to report that something is not available has failed in its check; 69 is for the
  command that needed the service. Change the two sentences of the manual and `docs/cli.md`. The other choice is exit
  69 for `doctor` and `bench` (a breaking change; each script that reads 1 changes): it makes the table true as written.

## Approach

1. `src/manual/facts.rs:203` and `docs/cli.md:139` say that "no edge leg worked" is 69 for `tunnel`, `warp` and the
   commands that start a session, and that `doctor` and `bench` give 1 when no leg or no request worked. The row of 1
   names them. `docs/exit-codes.md` gets the same sentence.
2. `podnet doctor --help` (`src/cli/mod.rs:291`) and `docs/doctor.md:46` name the four legs (`quic`, `http2`,
   `quic+warp`, `http2+relay`): exit 0 when at least one works, the relay leg too.
3. One table-driven test: for each command that reports "no leg" or "no request" (`doctor`, `doctor --json`, `bench`,
   `tunnel` with no edge), the exit code is a code that the row of the table of `podnet man exit-status --json` names for
   that command. The rows for `doctor` and `bench` fail before the fix (no row names them with exit 1).

## Prove

```sh
cargo test -p podnet --test exit_contract
cargo test -p podnet --test doctor
```

## Done

2026-11-23, the commit of this entry.

- Cause: the manual row of 69 (`src/manual/facts.rs`) and `docs/cli.md` named "no edge leg worked" for each
  command; `doctor` and `bench` give 1 (the help, `docs/doctor.md` and the lane say 1). The help of `doctor` named
  three legs; `Report::ok` counts four (the relay leg too).
- Change (the call of `TODO/PROGRESS.md`: keep exit 1): the row of 1 names `podnet doctor` (no leg can work) and
  `podnet bench` (no request worked). The row of 69 names `tunnel`, `warp` and the commands that start a session,
  and says that `doctor` and `bench` give 1. `docs/cli.md` and `docs/exit-codes.md` say the same.
  `podnet doctor --help` and `docs/doctor.md` name the four legs; the relay leg alone gives exit 0. No exit code
  changes.
- Test: `a_failed_check_exits_with_a_code_whose_row_names_the_command` (`tests/exit_contract.rs`): one table
  (`doctor` and `doctor --json` through a proxy at 127.0.0.1:1 with the edge 192.0.2.1:7844, and `bench` of
  `http://127.0.0.1:1/`). Each run exits with a code that is not 0 and whose row of `podnet man exit-status --json`
  names the command.
- Control: on the manual before the fix the test fails on the three rows:
  `exit Some(1), and its row does not name podnet doctor` (two times) and `... does not name podnet bench`.
- `cargo test -p podnet --test exit_contract`: 9 passed. `cargo test -p podnet --test doctor`: 9 passed.

# T-529: state directory: the warning names a path that does not exist, and `warp register` prints it three times (GitHub #704)

**Source:** GitHub #704 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Plain host. HOME is a directory owned by root (mode 1777), as for a service user with a shared HOME. User nobody (uid
65534), `HOME=/var/tmp/qa/home`:

```
ls -ld /var/tmp/qa/home /var/tmp/qa/home/.local
drwxrwxrwt root root /var/tmp/qa/home
ls: cannot access '/var/tmp/qa/home/.local': No such file or directory

podnet warp register 2>&1 | grep -c 'belongs to the user'
3
podnet doctor 2>&1 >/dev/null | head -1
WARN podnet_core::state: /var/tmp/qa/home/.local/state/podnet belongs to the user 0, not to this user (65534): podnet writes no file of another user (a file of root in the home of a user breaks it). Run as that user (sudo -u USER, or sudo -H), give another path, or set PODNET_FOREIGN_HOME=allow; podnet uses the next state directory
```

`podnet warp register` prints the same WARN line 3 times. `podnet doctor`, `link status --create` and `net shim` print it
once. Then podnet writes in `/tmp/podnet-65534/` (for example `saved       /tmp/podnet-65534/warp.json`). The
promise: `podnet man environment` (`PODNET_FOREIGN_HOME`), "podnet writes no file in a directory that belongs to another
user (root with the HOME of a user): the state directory skips that HOME"; the pass prompt, check 3, "names each cause
once". What happened: the path in the warning does not exist (the directory that belongs to root is HOME itself, so
"belongs to the user 0" is false for the named path); one command prints the same warning 3 times; the warning does not
say where the files go (`/tmp/podnet-65534`), only "the next state directory".

## Premise

- Read: `check_own_with` (`crates/podnet-core/src/state.rs:99`) takes the owner of the nearest directory that exists
  (`owner_of`, `crates/podnet-core/src/state.rs:73`: it walks up from `~/.local/state/podnet` to HOME) and names `path`
  (the path that does not exist) in the message. The owner and the path do not match.
- Read: `choose` (`crates/podnet-core/src/state.rs:136`) writes the warning (`:138`) on each call. `state_dir()` has no
  cache, so each caller of the state directory in one process (the three calls of `warp register`) repeats it.
- Read: the line says "podnet uses the next state directory" and not which one; `choose` knows the next candidate
  (the one it returns).
- The code is podnet's own. Not measured: the three call sites of `warp register`.

## Approach

1. The message names the directory that has the foreign owner (the one `owner_of` found) and the path that podnet
   skips: `/var/tmp/qa/home belongs to the user 0, not to this user (65534): podnet skips /var/tmp/qa/home/.local/state/podnet`.
2. The warning comes once for each process (a `OnceLock` or the cache of the chosen directory), and a second line
   (or the end of the first) names the directory that podnet uses: `podnet uses /tmp/podnet-65534`. The fixed form of the
   steps stays (`sudo -u USER`, `PODNET_FOREIGN_HOME=allow`).
3. One table-driven test of `choose` (the function takes the owner check as an argument): for each layout (HOME owned by
   another user and `.local` missing; `.local` owned by another user; HOME owned by this user; two calls of the function
   in one process) the warning names the real owner path, comes once, and names the directory that is used. The rows
   with a missing `.local` and with two calls fail before the fix.

## Prove

```sh
cargo test -p podnet-core state
cargo test -p podnet --test warp_register
```

## Done

2026-11-23, the commit of this entry. `check_own_with` (`crates/podnet-core/src/state.rs`) names the directory that
has the other owner and the path in it: `/var/tmp/qa/home (in it: /var/tmp/qa/home/.local/state/podnet) belongs to
the user 0, ...`. `choose` gives each warning one time in each process (`warn_first`) and ends it with the directory
that it uses: `; podnet uses /tmp/podnet-65534`. `choose_with` takes the owner check as an argument. The tests:
`state_a_foreign_home_is_named_once_with_the_used_directory` (`crates/podnet-core/src/state.rs`: HOME of another
user with no `.local`; `.local` of another user; HOME of this user; two warnings of one line) and
`a_foreign_home_gives_one_warning_with_the_used_directory` (`tests/warp_register.rs`: `podnet warp register` with
HOME of uid 65534 as root, or HOME `/` as another user: one warning, it names HOME and the used directory).
`cargo test -p podnet-core state` and `cargo test -p podnet --test warp_register`: passed. Control: with the code
before the fix the binary test fails with 3 lines `WARN podnet_core::state: /tmp/.tmpZsudpd/home/.local/state/podnet
belongs to the user 65534, not to this user (0): ...`, and the unit test fails at the row "no .local" (the line
names the path that does not exist).


# T-530: bench: the text output shows `null` for the latency of a run with no answer (GitHub #698)

**Source:** GitHub #698 (`[qa 0.4.0-beta.18]`, black-box QA pass of 2026-11-23): the text of the output of `bench` with
no answer. The error line of the same issue is in T-521 and T-522.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Lab `no-proxy-no-dns`:

```
podnet bench --duration 1 --timeout 3 http://nowhere.invalid/
podnet bench: http://nowhere.invalid/
  requests 50 (ok 0, errors 50) in 2.8 s on 4 connections
  rate 0.0 requests/s
  latency ms: min null p50 null p95 null p99 null max null
```

The text output shows `null` (the JSON word) for the latency of a run where no request worked.

## Premise

- Read: `text` (`src/cli/cmd/bench.rs:139`) prints the `serde_json::Value` of each latency with `{}`. A `null` of the
  JSON document (`document`, `latency_ms`) comes out as the word `null`. The JSON document is right: `null` is its
  value for "no sample".
- The code is podnet's own.

## Approach

1. `text` prints `-` for a latency with no sample, and `0.0 requests/s` stays. The JSON document keeps `null`.
2. One table-driven test of `text` for a document with samples, and one with none (and with a rate of `null`):
   no `null` in the text. The row with no samples fails before the fix.

## Prove

```sh
cargo test -p podnet bench
```

## Done

2026-11-23, the commit of this entry.

- Cause: `text` (`src/cli/cmd/bench.rs`) printed each `serde_json::Value` with `{}`; a `null` (no sample) came out
  as the word `null`.
- Change: `shown` gives `-` for `null` (and a string with no quotes). The text of a run with no answer is
  `latency ms: min - p50 - p95 - p99 - max -`, and a run of no time has `rate - requests/s`. The JSON document
  keeps `null`.
- Test: `the_text_of_bench_has_no_null` (`src/cli/cmd/bench.rs`): one table (samples, no sample, no time, an
  empty document). No row has `null` in its text, and the JSON document keeps `null`.
- Control: with `null` printed as before, the row `no sample` fails: `no sample: podnet bench: http://a/ ...
  latency ms: min null p50 null ...`.
- `cargo test -p podnet bench`: 2 passed.

# T-531: pod: a bad PODNET_RESOLVE is ignored; the manual says exit 78 for each command (GitHub #687, end of the pass)

**Source:** GitHub #687, the comment "End of the pass: [qa 0.4.0-beta.18]", the list "Observed and not filed": "`pod` ignores
a bad `PODNET_RESOLVE` while other commands exit 78".
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet man environment`: "PODNET_RESOLVE ... A bad value is a configuration error (exit 78) for each command."
Measured on 2026-11-23 with the debug binary of beta.17:

```
PODNET_RESOLVE=bad podnet bench --duration 1 http://127.0.0.1:1/
podnet: PODNET_RESOLVE: "bad" is not HOST=IP                                       exit 78
PODNET_RESOLVE=bad podnet pod -x direct -- true
podnet pod: the program does not get no_proxy, https_proxy, NO_PROXY, HTTPS_PROXY (...)   exit 0
PODNET_RESOLVE=bad podnet pod --backend seccomp -x direct -- true                   exit 0
```

## Premise

- Read: `src/cli/opts.rs:315` checks the variable (`parse_pins`, `Fault::Config`) in the code that runs after the
  command line is parsed. `podnet pod` goes to `podnet_netns::main_with_args` before that parse (`src/main.rs:104`),
  so each check of `opts.rs` is skipped: also the other variables that `opts.rs` checks (`PODNET_PROXY` with no scheme,
  `--egress`).
- The code is podnet's own. Not measured: whether the pod uses the pins at all (the manual says "all commands").

## Approach

1. The `pod` branch of `src/main.rs` runs the same checks of the variables (a shared function that `opts.rs` and the
   `pod` branch call) before `main_with_args`, and exits 78 for a bad value. If the pod does not use the pins, the
   manual says "each command but pod" and the check stays for the variable that the pod uses.
2. One table-driven test: for each variable that the manual calls a configuration error with a bad value
   (`PODNET_RESOLVE`, `PODNET_PROXY` with no scheme, `PODNET_POLICY` with a bad file) and each command (`bench`, `pod`,
   `pod --backend seccomp`): exit 78 and one line that names the variable. The `pod` rows fail before the fix (exit 0).

## Prove

```sh
cargo test -p podnet --test config_input
cargo test -p podnet --test pod_usage
cargo test -p podnet --test exit_contract
```

## Done

2026-11-23, the commit of this entry. `cli::opts::variable_checks` holds the check of `PODNET_RESOLVE`;
`network_options` and both branches of `podnet pod` in `src/main.rs` (`pod_variables`) call it, so a bad value exits 78
with one line (`podnet pod: PODNET_RESOLVE: "bad" is not HOST=IP`) before a backend starts. `pod --help` and `pod
--version` work with a bad value, as for each other command. `PODNET_POLICY` was checked before the branch of the pod
already; `PODNET_PROXY` is not a variable of the pod (it takes `-x`). The rows `PODNET_RESOLVE` x (`bench`, `pod`, `pod
--backend seccomp`) of tests/config_input.rs. Before the fix: "Some(("PODNET_RESOLVE", "bad")) podnet pod -x
socks5://127.0.0.1:9 -- true: exit Some(0), not 78", the same for `--backend seccomp`.


# T-532: pod: the seccomp backend counts each refused connect twice; autostart remove of an unknown name exits 1 (GitHub #687, end of the pass)

**Source:** GitHub #687, the comment "End of the pass: [qa 0.4.0-beta.18]", the list "Observed and not filed": "the seccomp
backend counts each refused connect twice in "(N refused)""; "`autostart remove NAME` for an unknown name exits 1 while
`stop` and `logs` exit 69".
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The tester saw two small faults and did not file them. The other items of that list (`podnet help --help` exits 64; each
unknown-flag error ends with "a newer release can: podnet update --check"; `-v` before `pod` does not log each flow, while
`pod -v` does) are not faults: they follow the help and the design, and have no entry.

1. Rule rows of a pod: `podnet pod --backend seccomp` with a `reject` rule prints `connect HOST:PORT refused by the rule R
   (N refused)`; for one refused `connect` of the program, N grows by two.
2. Measured on 2026-11-23 with the debug binary of beta.17:

```
podnet autostart remove nosuchname     podnet: no autostart entry nosuchname              exit 1
podnet stop nosuchname                 podnet: no instance nosuchname runs (podnet ps)    exit 69
podnet logs nosuchname                 podnet: no log of nosuchname at ...: No such file or directory   exit 69
```

## Premise

- Read: `note_refusal` (`crates/podnet-netns/src/rules.rs:156`) adds one to a static counter for each call. The calls are
  in `connect.rs:219` (the seccomp path), `kernel_serve.rs`, `frontdoor.rs`, `udp.rs`, `netns/backend.rs` and
  `netns/udpflow.rs`. Not found: why the seccomp path calls it twice for one connect (a retry of the notification, or
  two routes for one flow). Not measured with a debugger.
- Read: `src/cli/cmd/autostart.rs:138` calls `autostart::remove(&name, ...)?` and gives its error no class: exit 1.
  `stop` and `logs` give an unknown name the class 69.
- The code is podnet's own.

## Approach

1. Find the second call of `note_refusal` in the seccomp path; one refusal counts once. One test of the counter: a
   pod with a `reject` rule and a program that connects once: the line says `(1 refused)`, on each backend that the host
   has (the seccomp row fails before the fix).
2. `autostart remove` of an unknown name gives the same class as `stop` and `logs` (69). One table-driven test over
   `stop`, `logs` and `autostart remove` with an unknown name: the same exit code and one line.

## Prove

```sh
cargo test -p podnet --test pod_seccomp_rules
cargo test -p podnet --test exit_contract
```

## Done

2026-11-23, the commit of this entry. The cause of the double count, found with the test: the fake DNS of the seccomp
backend gives a name an IPv4 and an IPv6 address (`198.18.0.0` and `fd63:6672:7300::c612:0` for `lab.test`); a
program that takes each address of `getaddrinfo` in order calls `connect` two times for one flow, and each refused
`connect` counted (the netns backend gives an IPv4 address only, so it counts 1). `note_refusal_of`
(`crates/podnet-netns/src/rules.rs`) counts the second family of the same target within 2 s as the same flow; `-v`
names it as "the other address of the same flow". `autostart remove` of an unknown name gives exit 69 and `no
autostart entry NAME (podnet autostart list)` (`crates/podnet-core/src/autostart.rs`, a file that podcrates imported
into pod-unit: a fix only). The tests: `one_refused_connect_counts_once_on_each_backend`
(`tests/pod_seccomp_rules.rs`: a C client that tries each address of `lab.test` with a reject rule and `-v`, on
seccomp and netns: one count line, `(1 refused)`) and `exit_69_is_an_unknown_name` (`tests/exit_contract.rs`:
`stop`, `logs` and `autostart remove` with an unknown name: exit 69 and one line). `cargo test -p podnet --test
pod_seccomp_rules`: 11 passed; `cargo test -p podnet --test exit_contract`: 9 passed. Control: on the code before the
fix the first test fails with `seccomp: TRIES:2 OK:0` and the lines `(1 refused)` and `(2 refused)`, and the second
with `["autostart", "remove", "nosuchname"]: exit Some(1)`. The COORD note (to podcrates): the fix of the frozen file
`crates/podnet-core/src/autostart.rs` (`remove`: exit 69 for an unknown name) for pod-unit, if pod-unit takes
`remove`.


# T-533: Publish `v0.4.0-beta.18`: the fixes of the QA pass of 2026-11-23, `pod-release` and podcrates `v0.82.0`

**Source:** `docs/maintenance.md`, Release; `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease when a batch of
fixes is green. The black-box QA pass of 2026-11-23 (GitHub #687, #689 to #706) and the proof of T-487 (COORD-109,
PodUtils/podnet#656).
**Category:** release
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.17` has none of the fixes of the QA pass of 2026-11-23 (T-515 to T-532), the shared verifier of
`pod-release` (T-494), the takes of podcrates `v0.71.0` to `v0.82.0`, and the deferred features of `4df28952`. The
steps of the image of T-487 run only on a tag, so they have no proof yet.

## Premise

- The steps are `docs/maintenance.md`, "Release". Step 0 (the black-box QA pass) found 19 issues. Each is an entry,
  and each entry is done except T-516 and T-522. These two are partial: the part of podnet is done, and the rest waits
  for a tag of podcrates (COORD-115, PodUtils/podnet#707). The release does not wait for that rest
  (`docs/decisions.md`, 2026-11-23, A partial QA entry whose rest is a crate of podcrates). The notes name the limit.
- The notes have `## Breaking`: the exit codes and the JSON keys that changed (T-494, T-513, T-515 to T-532).
- `ci` 45289544540 and `live` 45289544910 passed on `cca0b3d`, the last commit of the fixes before the version.

## Approach

0. The QA pass of 2026-11-23: #687, #689 to #706. Each entry is done, except T-516 and T-522 (partial; their rest
   waits for podcrates).
1. Set the version `0.4.0-beta.18` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`), write the llms files again.
2. Write `docs/releases/v0.4.0-beta.18.md`, with `## Breaking`.
3. Push; wait until `ci` and `live` pass on that commit. Dispatch `gate.yml` on that commit and wait until it passes.
4. Dispatch the dry run of `release.yml` (`publish` empty).
5. Dispatch `release.yml` with `publish=v0.4.0-beta.18`.
6. Check the files of the release, `release-proofs.yml`, and the job `image` (the attestation of the manifest list,
   the pull-back check, the section "The image, by digest" of the notes). Write the digests in COORD-109 (#656) and
   close it through the record.

## Prove

```sh
gh release view v0.4.0-beta.18 -R PodUtils/podnet
gh run list -R PodUtils/podnet --workflow release-proofs.yml --limit 1
```

The job `image` of the release run passes, and the notes of the release have the section "The image, by digest".

## Correction

2026-11-24: the dry runs of `release.yml` found two faults of the release path, not of the code:

- Dry run 45347322318 on `cfbc215`: the job `linux loongarch64-unknown-linux-musl` failed at the link: "unsupported
  linker arg: --no-rosegment" (zig cc). The build image of Dependabot #680 (`rust:1-alpine@sha256:8d0e82ab`) gives it.
  `27d2e3a` takes back the digest `0cce0a5e` of `v0.4.0-beta.17`; dry run 45353622174 (attempt 2) links each target.
- Dry run 45353622174 (attempt 2): the job `publish` failed: `promises.py --notes` ran
  `dist/podnet-x86_64-unknown-linux-musl`, and a downloaded artifact has no execute bit ("Permission denied"). The step
  runs a copy with mode 0755.
- `live` on `cfbc215` failed three times at outside services (WARP's API: HTTP 429, error 1015; the MASQUE session: no
  DNS answer, T-414; a quick tunnel over HTTP/2 not ready in 90 s, with no log in the script). `live` 45353877401
  passed on `27d2e3a`.
- The job `image` of the release run 45366987263 failed at the step "the provenance of the image, in the registry":
  the step before it logged out of ghcr.io (T-538). The images and the manifest list are on ghcr.io, and the pull-back
  check of each image passed.

## Done

2026-11-24, `de6027b` (the tag `v0.4.0-beta.18`). `release.yml` 45366921104 (`publish=v0.4.0-beta.18`) made the
tag; the release run 45366987263 built each target, passed the job `gate / lane`, and published the prerelease with
17 files (the 8 binaries, `SHA256SUMS`, `SHA256SUMS.sigstore.json`, `provenance.sigstore.json`, the installer, the
man page, the llms files, the schema and the licenses). `release-proofs.yml` 45374896321 passed (the rebuild of
`x86_64` and `aarch64`, and the proofs). The job `image` failed after the push (T-538): the manifest list is
`sha256:3bdf929f0f0bd87cfb299faa51fdbac89dd2fd4ad8d15067c13f1ec77be9079b`; it has no attestation, and the notes have
no section "The image, by digest". The next release proves T-487; COORD-109 (#656) stays open until then.


# T-534: The release commit 9f3c8a2 fails the lane and the restricted host: the row warp-proxy-dead-proxy and a dead unix socket that a forked child holds

**Source:** `gate.yml` run 45309579073 on `9f3c8a2` (job `lane`, case `errors` in the labs `bind-denied`,
`listen-denied`, `bind-netns-vm-denied` and `testers`); `ci` run 45309573045 on `9f3c8a2` (job `restricted-host`, the
step "Test with bind(AF_INET) refused").
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

1. The case `errors` (`scripts/lab-errors.sh`) exits 1 in four labs. The log of the job shows `tail -n 40` of each
   `LAB.errors.txt`, and each row in these lines is `pass` or `n.a.`: the failed row is not in the log.
2. The unit test `warp::proxy::tests::a_unix_socket_that_answers_is_not_taken` (`src/warp/proxy.rs`) fails with "a
   dead socket is taken" (488 passed, 1 failed). The same test passed in `restricted-host` on `cca0b3d` (ci
   45289544540).

## Premise

- Read: `gate` passed on `9d34eb7`. The commits since then are the fixes T-515 to T-532.
- Read: T-523 (`cca0b3d`) binds the listeners of `warp proxy` before the registration of the device
  (`front::bind_early`). The row `warp-proxy-dead-proxy` gives `--listen 127.0.0.1:0`.
- Read: the test drops its listener, then `bind` probes the socket with one connect. A note in
  `crates/podnet-netns/src/listen.rs` (test of `claim_unix_path`) says that a bound and closed listener can stay open
  in a child that a parallel test forks.
- Measured: podlab v0.81.0 (`get-podlab.sh v0.82.0` falls back to it), the release binary of `9f3c8a2`.

## Approach

1. Run the case `errors` in the labs; find the failed row and its fault line.
2. A stale check of the script: change the row, not podnet, when podnet gives the true line and the documented exit
   code.
3. `gate.yml`: for a failed case with rows, print each line `row NAME: fail` with the lines of that row.
4. The probe of a unix socket: a connect that works is checked again. A socket answers only when each of 3 connects,
   50 ms apart, works. One test pins the class: a child that holds the listener until its exec.

## Prove

```sh
python3 -I scripts/check-workflows.py
sh -n scripts/lab-errors.sh
cargo test -p podnet-core --lib net::
cargo test -p podnet-netns --lib listen::
sh scripts/restricted-host.sh --lib warp::proxy
```

The case `errors` passes in the labs `bind-denied`, `listen-denied`, `bind-netns-vm-denied` and `testers`.

## Done

2026-11-24, the commit of this entry.

- The failed row, in each of the four labs: `warp-proxy-dead-proxy`. Its lines (bind-denied):
  `podnet: warp proxy: a listener did not start: binding tcp://127.0.0.1:0: Permission denied (os error 13); this
  host refuses the bind (...)`, exit 69, and `fault: 0 lines match 'each host failed: .*could not reach the proxy
  127\.0\.0\.1:1'`. listen-denied gives the same line. The cause: T-523 binds the listener before
  the registration, so a lab that refuses each TCP bind stops the command at the bind. That is the true line and the
  documented exit code (69; the rows `listen-*` check it). The check of the row was stale.
- The fix of the row: the listener is `unix:DIR/warp-proxy-dead-proxy.sock` (bind-denied refuses each TCP bind, not
  a bind of AF_UNIX), so the row checks the dead proxy in these labs. A lab that refuses the listener too (exit 69
  and `a listener did not start`) gives n.a. with the line.
- `gate.yml`: for a failed case, each line `row NAME: fail` with the lines of its row (awk); a case with no row
  line gives its last 40 lines. `check-workflows.py`: 11 files, 0 faults.
- The cause of the failed test: a child that another test forks keeps a copy of the listener until its exec. Thus
  the connect of the probe to the closed socket works, and `bind` says "served by another process". The fix:
  `podnet_core::net::socket_answers` (3 connects, 50 ms apart; each must work), in `claim_socket_path` and
  `warp::proxy::bind`; the same rule in `podnet_netns::listen::claim_unix_path`. The test of `warp::proxy` keeps
  its assertion and prints the error.
- The test: `a_socket_that_a_forked_child_holds_until_its_exec_is_stale` (`crates/podnet-core/src/net.rs`: a child
  sleeps 40 ms between its fork and its exec; the claim takes the closed socket, and a live socket answers).
  Control: with one probe (the code before the fix) it fails 5 of 5 runs with "is served by another process"; with
  the fix it passes 20 of 20 runs.
- Local lab runs (podlab v0.81.0, uid 65534): before the fix, `bind-denied` and `listen-denied` exit 1 with the row
  above. After the fix, the case `errors` exits 0 in `bind-denied`, `listen-denied` (the row is n.a.: the lab
  refuses the unix listener), `bind-netns-vm-denied` and `testers` (the row passes in the other three).
- The lib tests of podnet with bind(AF_INET) refused (the preload of `scripts/restricted-host.sh`): 40 runs, 0
  failures of `warp::proxy` (the tests `router::kernel` and `router::guest::veth_tests` skipped: they need the
  kernel of a runner). Before the fix, 80 runs here gave no failure: the race is rare on this host.
- An ask for podcrates: the same one-connect probe is in `pod-control` (`unix.rs`), `pod-link` (`unix.rs`) and
  `pod-warp` (`find.rs`); `socket_answers` belongs in `pod-core`.

# T-535: live: the check of a WebSocket through a --url origin gives no line of podnet when the tunnel is not ready

**Source:** `live` 45322152685 on `cfbc215`, attempt 3 (2026-11-24): the step "WebSocket through a --url origin" failed
with "the tunnel is not ready after 90 s" for `http2`, and printed no line of podnet.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`scripts/live-ws-url.sh` writes the lines of `podnet tunnel` to `tunnel.err` and stops after 90 s with one line of its
own. The cause (a refusal of the edge, a slow registration, or a fault of podnet) is not in the log of the job.

## Premise

- `scripts/live-share.sh` prints the masked tail of its `share.err` when the share is not ready (the pattern to copy).
- The rule "Quality first" (2026-11-17): each failure gives one true line; a check must keep its evidence.

## Approach

1. When the tunnel is not ready in time, print the masked tail (25 lines) of `tunnel.err`, as `scripts/live-share.sh`
   does.
2. Check each other `scripts/live-*.sh` for a wait that ends with no line of podnet, and make it print the same tail.

## Prove

```sh
cargo test -p podnet --test live_scripts
# the next failed wait of live shows the lines of podnet
```

## Correction

2026-11-24: `live` 45396168492 on `1d446af` failed at the step "The commands of the second batch", "a nested session
(issue 156)": `curl: (97) Recv failure: Connection reset by peer`, and no line of podnet. The runtime code of
`1d446af` is the code of `4a4361a`, whose `live` 45388481255 passed: the cause is outside (WARP; the same run shows the
door `162.159.198.1` with a certificate of Let's Encrypt, T-406). `scripts/live-batch2.sh` now prints the masked tail
of the logs of the nested and the outer proxy when the request through the nested session fails (`redact`, the mask of
`wait_port`).

## Done

2026-11-24, the commit of this entry.

- `scripts/live-ws-url.sh` prints the masked tail (25 lines) of `tunnel.err` at each failure after the start of the
  tunnel: the wait of 90 s, the wait for the name, the WebSocket client. `scripts/live-ws-url.sh`,
  `scripts/live-throughput.sh` and `scripts/live-share.sh` stop at once when podnet ends before its ready line, with
  the tail and the exit code of podnet. `scripts/live-throughput.sh` defines `log` before its first use, and prints it
  when the public URL does not answer.
- The test: `tests/live_scripts.rs`, `a_live_script_prints_the_lines_of_podnet_when_podnet_ends_before_ready` (a
  table of the three scripts; a stand-in `podnet` writes one line with a tunnel host and exits 69; each script must
  print the masked line and "ended before it was ready (exit 69)" in less than 30 s). The row of `live-ws-url.sh`
  needs the Python module `websockets` and skips with its name when the host has none. Control: on the script before
  the fix, `live-ws-url.sh` prints only "the tunnel is not ready after 90 s", after 90 s, and no line of podnet.


# T-536: a refused unix bind says that the directory is not writable when the host refuses the socket itself

**Source:** T-534 (2026-11-24): in the lab `listen-denied` of podlab, a unix listener gives "Permission denied (os error
13); the directory of the socket is not writable", but the directory is writable: the lab refuses the socket.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The next step of a refused unix bind (T-523, `bind_fault_unix`) reads EACCES as a directory that is not writable. On a
host whose policy refuses AF_UNIX listeners (a seccomp or LSM rule), the line names a false cause.

## Premise

- podlab `listen-denied` refuses the bind of an AF_UNIX listener in a writable directory (T-534, local runs).

## Approach

1. On EACCES, test the directory (`access(dir, W_OK | X_OK)`). Name the directory only when the test fails; else say
   that the host refuses the unix listener, and name `podnet net doctor`.
2. One table-driven test: a writable directory with a refused socket (a seccomp filter in the test, or the lab row),
   and a directory with no write bit.

## Prove

```sh
cargo test -p podnet-core --lib net::
# the lab listen-denied: the row of a unix listener names the refusal of the host
```

## Done

2026-11-24, the commit of this entry.

- `crates/podnet-core/src/net.rs`: `unix_listener(path)` tests the directory of the socket with `access(2)` (`W_OK |
  X_OK`) and gives `Listener::UnixWritableDir` or `Listener::Unix`. `bind_fault_unix(path)` takes the path; at
  `PermissionDenied` in a writable directory, the step is "the directory of the socket is writable: this host refuses
  the unix listener (a seccomp or LSM rule); give a TCP address where the command takes one (podnet net doctor)". The
  other kinds of a refused bind keep the steps of `Listener::Unix`. Each of the 8 callers gives its path; `net
  tailscale` takes the kind from `unix_listener`.
- The tests: three rows of `UnixWritableDir` in `a_refused_bind_is_69_with_one_line_and_a_next_step`, and
  `a_refused_unix_bind_names_the_directory_only_when_it_is_not_writable` (a writable directory, a directory that does
  not exist, and a directory of mode 0500, which root can write). `cargo test -p podnet-core --lib net::`: 5 passed.
  Control: the code before the fix had one step for each unix `PermissionDenied`, "the directory of the socket is not
  writable", also in a writable directory (the row of T-534 in the lab `listen-denied`). The lab row is not run here.


# T-537: the build image of Dependabot #680 cannot link loongarch64: --no-rosegment

**Source:** release dry run 45347322318 on `cfbc215` (2026-11-24), job `linux loongarch64-unknown-linux-musl`:
"unsupported linker arg: --no-rosegment" (zig cc).
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`images/build/Dockerfile` pins `rust:1-alpine` by digest. The digest of Dependabot #680 (`8d0e82ab`) gives a link of
`loongarch64-unknown-linux-musl` that passes `--no-rosegment`, and zig cc refuses it. `27d2e3a` took back the digest
`0cce0a5e`; dry run 45353622174 (attempt 2) then linked each target. Dependabot proposes the new digest again.

## Premise

- The digest `0cce0a5e` built `v0.4.0-beta.17` and `v0.4.0-beta.18` for each target.
- `ci` does not read `images/build/Dockerfile`: only `release.yml` builds in the image. A fault of the image shows in a
  release dry run only.

## Approach

1. Find which part of the new image gives `--no-rosegment` (the version of rustc in the image, or the target spec).
2. Take the new digest only with a fix (a linker flag of the target in `release.yml`, or a newer cargo-zigbuild or
   zig), and run a release dry run before the commit.
3. Until then, close each Dependabot pull request of this image with a link to this entry.

## Prove

```sh
# a release dry run with the new digest passes the job linux loongarch64-unknown-linux-musl
```

## Correction

2026-11-25: the cause is in cargo-zigbuild: newer rustc gives `-Wl,--no-rosegment` for loongarch64
(rust-lang/rust#162546), and cargo-zigbuild `0.24.0` and later drop it (rust-cross/cargo-zigbuild#505, commit
`d36754d1`, the file `linker_args.rs`). `release.yml` takes cargo-zigbuild `0.24.1` and `images/build/Dockerfile` the
digest `8d0e82ab` of Dependabot #680. A release dry run proves it.

## Done

2026-11-25, `ec69b3b`. The release dry run 45533314862 on `ec69b3b` (cargo-zigbuild `0.24.1`, the image digest
`8d0e82ab`) passed each job: the job `linux loongarch64-unknown-linux-musl` linked, its binary is static and ran under
QEMU, and the jobs of the other eight targets and `gate / lane` passed.


# T-538: The job image of the release of v0.4.0-beta.18 fails: a docker logout before the push of the attestation

**Source:** `release.yml` run 45366987263 on `de6027b` (tag `v0.4.0-beta.18`), job `image` 138371539304, the step
"the provenance of the image, in the registry": `Error: No credentials found for registry ghcr.io`.
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The step "the images" pushes the images and the manifest list, and then runs `docker logout ghcr.io`. The next step
(`actions/attest` with `push-to-registry: true`) reads the credentials of the registry from the docker config, and
there are none. The attestation of the manifest list is not made, and the step that checks it and writes the section
"The image, by digest" into the notes does not run (T-487).

## Premise

- Read: the job log of 138371539304. The images and the manifest list are on ghcr.io (the digest of the manifest
  list is `sha256:3bdf929f0f0bd87cfb299faa51fdbac89dd2fd4ad8d15067c13f1ec77be9079b`). The pull-back check of each
  image passed.
- Read: these steps run only on a tag, so the dry runs did not find the fault.

## Approach

1. `release.yml`: the step "the images" keeps the login. The last step logs out after `gh attestation verify`.
2. `scripts/check-workflows.py`: a rule for the class. A step with `push-to-registry: true` comes after a
   `docker login` (or `docker/login-action`) of its job, with no `docker logout` between them.
3. `ci.yml`, job `plant`: a plant that puts a `docker logout` before the step must fail the check.

## Prove

```sh
python3 -I scripts/check-workflows.py
```

The job `image` of the next release passes.

## Done

2026-11-24, the commit of this entry.

- The test: the rule of `scripts/check-workflows.py`, and its plant in `ci.yml` (job `plant`, "a logout before
  push-to-registry"). Control: on `release.yml` before the fix the check fails with `.github/workflows/release.yml:
  job image, the provenance of the image, in the registry: push-to-registry with no docker login before it` (exit 1).
  After the fix: `check-workflows: 11 files, 0 faults`. The plant on the fixed copy gives the same fault line.
- The images of `v0.4.0-beta.18` have no attestation, and its notes have no section "The image, by digest". The next
  release proves T-487.


# T-539: Take podcrates v0.82.1

**Source:** `PodUtils/podcrates` tag `v0.82.1` (`0d7832b9`, 2026-11-24); PodUtils/podcrates#257 (COORD-152); the answer
to COORD-113 (PodUtils/podnet#685).
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.82.0`. The tag `v0.82.1` answers COORD-113: in `pod-cloudflare`, a refusal of the
registration with retry: yes gets new tries for 30 s with the same tunnel, and "no edge leg worked" has the class
TempFail (exit 75) when each leg asked for a retry, with the typed `fail::Refused` of the last leg in its chain. The
stand-in of the edge has `Reply::RefuseRetry` and `Edge::script`, and `Reply` is `non_exhaustive`. `pod-testkit` has
`need_route!`.

## Premise

- Read: the diff `v0.82.0..v0.82.1` of the crates `pod-cloudflare` and `pod-testkit`. The tag has no
  `adoption/podnet/` patch. `core.sh`, `core-net.sh` and `pins.py` of `scripts/install/` did not change.
- Read: `crates/podnet-core/src/hop.rs` `refusals` returned the typed refusals only, when the chain had one. With
  v0.82.1 the chain has the typed refusal of the last leg only: a start whose first leg said retry: no and whose last
  leg said retry: yes read as "each refusal asks for a retry" (exit 75 in place of 69).
- COORD-115 (PodUtils/podnet#707): `v0.82.1` answers none of its asks. T-516 and T-522 stay partial.

## Approach

1. Each `pod-*` pin names `v0.82.1`: the manifests, `Cargo.lock`, `fuzz/Cargo.lock`, `scripts/install/SOURCES` (tag
   and commit), `.github/workflows/issue-sync.yml` (`uses:` at `0d7832b9` and `podcrates_ref`).
2. `refusals` reads the typed refusals, then each refusal of the text of the chain, each one time. `again` of
   `src/tunnel.rs` stays as the second path.
3. `tests/quick_tunnel_standin.rs`: a row with 4 refusals with retry: yes, then an accept.
4. `docs/tunnels.md` (A refused registration) and `docs/protocol.md` (the attempt limits: 110 s and 210 s).
5. `tests/pod_seccomp_diff.rs` takes `pod_testkit::need_route!`; `tests/test_host/mod.rs` drops its copy.

## Prove

```sh
cargo test -p podnet --lib a_refused_registration_gives_the_hint_and_the_code_of_its_kind
cargo test -p podnet --test quick_tunnel_standin
cargo test -p podnet --test pod_seccomp_diff
sh scripts/agent/check.sh
```

## Done

2026-11-24, the commit of this entry.

- The test of the class: two rows of `a_refused_registration_gives_the_hint_and_the_code_of_its_kind`
  (`src/report.rs`) in the form of v0.82.1 (the text of each leg, the typed refusal of the last leg, the class of
  pod-cloudflare). Control: on `refusals` before the fix the row "Invalid tunnel secret (retry: no); Tunnel not found
  (retry: yes)" of a named tunnel fails: the second line is "the edge asks for a retry; ... (exit 75)". After the fix:
  1 passed.
- `tests/quick_tunnel_standin.rs`: 4 passed (the new row: ready with 1 provision and 4 refused registrations).
  `tests/pod_seccomp_diff.rs`: 1 passed.
- The take missed one line: `docs/tunnels.md` (`--leg-failover serial`) gave 130 s and 280 s. The next commit gives
  110 s and 210 s (PodUtils/podcrates@T-1117).


# T-540: release.yml writes `release-index.json` before `SHA256SUMS`

**Source:** PodUtils/podcrates@T-1118 (the answer to COORD-114, PodUtils/podnet#686): each project writes its own
`release-index.json` with the script `release-index.py` of podcrates in its release workflow, before `SHA256SUMS`.
**Category:** feature
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The releases of podnet have no `release-index.json`. `pod-release verify --all` and the other readers of the index
cannot check a release of podnet by its index. podssh writes it since `v0.77.0`.

## Premise

- Read: PodUtils/podcrates@T-1118 at `v0.82.1`. Its start condition: the coordinator names the step in a COORD issue
  `(to podnet)`.
- Read: `podnet update` does not need the index (T-494).

## Approach

1. `release.yml`, job `publish`: get the script `release-index.py` of podcrates at the tag of `Cargo.toml`, check its
   SHA-256, and run it before `SHA256SUMS`. `SHA256SUMS` has the line of the index.
2. The dry run writes the index too. The job compares the index with `SHA256SUMS`; `release-proofs.yml` does the
   same for a published release that has an index.

## Prove

```sh
python3 -I scripts/check-workflows.py
```

The dry run of `release.yml` has `release-index.json`, and `SHA256SUMS` names it.

## Done

2026-11-24, `c6b5bdf`. The job `publish` checks out `scripts` of PodUtils/podcrates at `0d7832b9` (the tag
`v0.82.1`), checks the SHA-256 of `release-index.py` (`32726ae3...`), writes `release-index.json` after the installer
and before `SHA256SUMS`, and compares the two. `release-proofs.yml` checks the line of the index, its version and each
SHA-256 when a release has one. `docs/install.md` names the file. The release dry run 45389095107 on `c6b5bdf`
passed: "release-index.py: OK", "release-index.json: 13 files, each as SHA256SUMS", and the line of the index in
`SHA256SUMS`. The first release with the index is the release after `v0.4.0-beta.18`.


# T-541: tests of a tool or of IPv6 of the host fail on a host without it, in place of a skip

**Source:** the local gate of 2026-11-24 (T-539): `tests/link_p2p_namespaces.rs`
(`three_namespaces_show_direct_first_and_relay_fallback`) and `tests/network_lab.rs`
(`the_network_surface_needs_the_network_lab`), as root in a cloud container with no iproute2 and no ping;
`tests/pod_seccomp/kernel_listen.rs` (`seccomp_kernel_socket_refuses_listen_v6`) on a kernel with no IPv6.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The tests check CAP_NET_ADMIN only. On a host with the capability and no `ip`, the test of three namespaces panics
with "the command runs: Os { code: 2, kind: NotFound, ... }"; with `ip` and no `ping`, both tests panic with "exec of
\"ping\" failed". `cargo test --workspace` stops at the first such test binary, and the test binaries after it do
not run. On a kernel with no IPv6 (no `/proc/net/if_inet6`), `socket(AF_INET6)` of the program is not a kernel socket,
so `listen` works, and the v6 test of the seccomp backend fails with "listen on a kernel socket must get EPERM".

## Premise

- Read: the module comment says that the test skips with a named reason. `pod_testkit::need_tool!` gives the skip
  line of a missing tool (`pod-testkit` v0.82.1).
- Measured: the panic in a container as root with no iproute2.

## Approach

1. `pod_testkit::need_tool!` for each tool of the lab: `ip`, `iptables`, `sysctl` and `ping` in
   `tests/link_p2p_namespaces.rs` (after the check of the capability); `ip`, `mount` and `ping` in
   `tests/network_lab.rs`. A lab that starts and fails stays a failure.
2. The v6 test of `tests/pod_seccomp/kernel_listen.rs` skips when the host cannot bind `[::1]:0`.
3. The module comments name the tools.

## Prove

```sh
PATH=/usr/bin:/bin cargo test -p podnet --test link_p2p_namespaces --test network_lab
cargo test -p podnet --test pod_seccomp kernel_listen
```

## Done

2026-11-24, the commit of this entry.

- Control: in this container as root, with no iproute2 and no ping, `three_namespaces_show_direct_first_and_relay_fallback`
  panicked with "the command runs: ... NotFound", then (with iproute2) with "exec of \"ping\" failed";
  `the_network_surface_needs_the_network_lab` panicked with "exec of \"ping\" failed"; on this kernel with no IPv6,
  `seccomp_kernel_socket_refuses_listen_v6` failed (3 runs of 3) with "listen 0 0 ok".
- After the fix: with no ping, both labs skip with "skipping, this host has no ping that can execute"; with
  `/usr/sbin` off the PATH, the test of three namespaces skips on `iptables`; with each tool, both labs pass (1 passed
  each). `cargo test -p podnet --test pod_seccomp kernel_listen`: 3 passed (the v6 test skips with "this host has no
  IPv6 socket").


# T-542: Take podcrates v0.83.0

**Source:** `PodUtils/podcrates` tag `v0.83.0` (`5fd6fb7e`, 2026-11-24); PodUtils/podcrates#259 (COORD-154); the answers
to COORD-115 (PodUtils/podnet#707) and COORD-117 (PodUtils/podnet#709).
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.82.1`. The tag `v0.83.0` answers COORD-115 (`pod-hold`, `pod-wormhole`, `pod-ws`,
`pod-proxy`, `pod-dns`, `pod-tls`, `pod-warp`, `pod-cloudflare`: PodUtils/podcrates@T-1125 to PodUtils/podcrates@T-1134) and COORD-117
(`pod_core::files::socket_answers`, PodUtils/podcrates@T-1209), and adds `pod-natprobe` (PodUtils/podcrates@T-802) and
the shared script `runs.py` of podcrates (PodUtils/podcrates@T-1213).

## Premise

- Read: COORD-154 and the diff `v0.82.1..v0.83.0`. The tag has no `adoption/podnet/` patch. `core.sh`, `core-net.sh`,
  `pins.py` and `release-index.py` did not change.
- Read: the breaking changes (`Failed::Unreachable` holds `Missed`; `EnrolError::NoAnswer`; `Class::CantCreate` and
  `Class::Internal`) need no change of podnet: podnet matches with a `_` arm or binds no `String`. podnet maps a class
  of a crate onto its own contract (`exit::code_of_class`): 70 and 73 exit 1.
- Read: podnet does not take `pod-unit`, `pod-acme` or `pod-opentunnel`; their items of COORD-154 do not apply.

## Approach

1. Each `pod-*` pin names `v0.83.0` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`, `scripts/install/SOURCES`,
   `issue-sync.yml`, and the checkout of `release-index.py` in `release.yml`).
2. `podnet_core::net::socket_answers` and `podnet_netns::listen::socket_answers` re-export
   `pod_core::files::socket_answers`.
3. `src/transfer/mod.rs`: the words of an expired code are those of `pod-wormhole` (T-520); `exact_time` is gone.
4. The rest of T-516 and of T-522 (their Done sections).
5. `pod-natprobe` and `runs.py` are entries of their own (T-543, T-544).

## Prove

```sh
cargo test --workspace
cargo test -p podnet --test pod_error_lines
cargo test -p podnet --test error_words
```

## Done

2026-11-24, the commit of this entry. `cargo build --workspace --all-targets --features full` and `cargo clippy
--workspace --all-targets --features full -- -D warnings` pass with no change for the breaking items.
`cargo test -p podnet-core --lib net::`: 5 passed; `cargo test -p podnet-netns`: 173 passed; `cargo test -p podnet
--lib transfer::`: 22 passed; `tests/pod_error_lines.rs`: 4 passed; `tests/error_words.rs`: 2 passed. The full suite:
see the commit.


# T-543: Take `pod-natprobe`: the class of the NAT of the host for `link p2p` and `doctor`

**Source:** PodUtils/podcrates#259 (COORD-154): "Take `pod-natprobe` (PodUtils/podcrates@T-802)". PodUtils/podcrates@T-802 (GitHub #212 of
podcrates).
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`podnet link p2p` picks the direct path or the relay by a try. It cannot say before the try which class of NAT the
host is behind (the mapping and the filtering of RFC 5780, the port allocation of RFC 4787, NAT64, PCP and UPnP).
`podnet doctor` has no NAT row.

## Premise

- Read: the README of the crate `pod-natprobe` at `v0.83.0`: a state machine with no I/O; the caller gives the sockets, the
  clock, the resolver and the list of STUN servers. Each property is `Observed`, `Unknown` (with its reason) or
  `NotApplicable`.
- Read: `src/networks/turn_*.rs` and `tests/link_p2p_*.rs` (the TURN relay and the direct path).

## Approach

1. Add `pod-natprobe` by the tag of the other crates. Drive `Prober` with the dialers of podnet (the egress policy;
   no UDP where the host refuses it).
2. `podnet doctor`: a row `nat` with the mapping and the filtering class, and `Unknown` with its reason on a host
   with no UDP. `--json` has the same fields.
3. `podnet link p2p`: when the class says that the direct path cannot work (both ends dependent mappings), go to
   the relay first.
4. One table-driven test over the stand-in NATs of `tests/link_p2p_namespaces.rs` (cone and symmetric).

## Prove

```sh
cargo test -p podnet --test link_p2p_namespaces
podnet doctor --json
```

## Correction

2026-11-25: step 3 needs the class of the peer too ("both ends dependent mappings"). `link p2p` has no channel to the
peer before its dial, so the class of this host alone does not decide it: step 3 is T-586. Step 4 uses the simulator
of `pod-natprobe` (each class of RFC 5780 in virtual time): the namespaces of `tests/link_p2p_namespaces.rs` have no
STUN server.

## Done

2026-11-25 (steps 1 and 2), the commit of this entry. podnet takes `pod-natprobe` by the tag `v0.84.0`.
`src/doctor/nat_check.rs`: the row `nat` of `podnet doctor` (text and `--json`: `verdict`, `connectivity`, `mapping`,
`filtering`, `detail`) runs `Prober` with the profile `Fast` over the STUN servers of the catalog of `pod-services`
that the egress policy permits, resolved by the resolver of podnet. Where the UDP probe of the doctor is refused, no
STUN request goes out and the row is `unknown` with the reason; a probe that does not end in 4 s is `unknown`. The row
does not change `ok`. `docs/doctor.md` names it. Test: `doctor::nat_check::tests::a_host_with_no_udp_gets_no_probe`
(`cargo test -p podnet --lib doctor::`: 33 passed). On this host (UDP refused): `nat unknown: UDP does not leave this
host (...): no NAT probe`. Step 4, the same day: `doctor::nat_check::tests::each_class_of_nat_gives_its_row` drives the
simulator of `pod-natprobe` (a dev-dependency with the feature `sim`) for a table of five rows: an endpoint-independent
and an address-and-port-dependent NAT (`ok`, the mapping named), a blocked UDP path in the 600 ms of `Fast` (`unknown`),
and with `Standard` (`refused`, `udp-blocked`; `unknown` with no corroboration). The simulator takes the place of the
stand-in NATs of `tests/link_p2p_namespaces.rs` (Correction). Step 3 is T-586.
# T-544: Take the script `runs.py` of podcrates in place of podnet's `watch-runs.sh` and `runs.py`

**Source:** PodUtils/podcrates#259 (COORD-154): "Take the script `runs.py` of podcrates in place of `watch-runs.sh` and its own
`runs.py` (PodUtils/podcrates@T-1213)".
**Category:** chore
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

podnet kept its own run watcher (the shell script `watch-runs.sh` and its helper `runs.py` in `scripts/agent/`).
podcrates has one watcher of the runs for each session (PodUtils/podcrates@T-1213). The rule "Shared code first" says
to take it.

## Premise

- Read: the script `runs.py` of podcrates at `v0.83.0`. It imports `watch.py` from beside it and needs the `watch.py`
  of the same tag (`watch.Checks`): with the `watch.py` of `f76099d`, its self-test fails with "module 'watch' has no
  attribute 'Checks'".

## Approach

1. `CLAUDE.md`: the watcher, its test and `runs.py` come from podcrates at the commit of `v0.83.0` (`5fd6fb7e`).
2. Remove podnet's `watch-runs.sh` and `runs.py`.
3. `docs/maintenance.md`: the commands of the shared `runs.py`.

## Prove

```sh
python3 -I .work/podcrates-agent/watch.py --self-test
python3 -I .work/podcrates-agent/runs.py --self-test
```

## Done

2026-11-24, the commit of this entry. With `watch.py`, `watch_test.py` and `runs.py` of `5fd6fb7e`: "watch
self-test: passed" and "runs self-test: passed". `CLAUDE.md` gets the three files at that commit; podnet's two
scripts are gone; `docs/maintenance.md` gives `runs.py --repo PodUtils/podnet --until SHA --min-runs 5`.


# T-545: temp files and sockets named by the pid alone: two processes in two PID namespaces meet on one path

**Source:** PodUtils/podvm#567 (COORD-180 of podvm, to all), lesson 5: "A pid is not unique where your process can run
in a PID namespace. Name temp files with `mkdir` or `O_EXCL` and a nonce."
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

podnet names temp paths with `std::process::id()` alone. A pod, a container and a PID namespace give pid 1 or the same
small pid to two processes that share `/tmp`. The second process then takes or removes the file of the first. Paths
of the code (not of tests): `src/cli/cmd/warp/run.rs:47` (the socket of `warp run`), `src/warp/export.rs:343`,
`src/cli/cmd/link/mod.rs:635`, `src/stat.rs:158`, `src/initcfg.rs:470`, `src/cli/cmd/serve_at.rs:57`,
`src/router/guest/wire.rs:196`, `crates/podnet-netns/src/netns/mod.rs:348` and `crates/podnet-netns/src/raw.rs` (the
resolv.conf of a pod), `crates/podnet-vnet/src/doctor.rs:504`, `:618` and `:635`,
`crates/podnet-vnet/src/tailscale_ssh.rs:210`, `crates/podnet-core/src/state.rs:257` and `:451`.

## Premise

- Read: `pod_core::files::create_new_private` creates a file with `O_EXCL` and mode 0600. pod-core has no public
  function for a temp directory or a temp file with a random name (`random_name` of `files/socket.rs` is private).
- Read: some paths add a time (`nanos`, `stamp`); a time is not a nonce either.

## Approach

1. COORD ask (to podcrates): `pod_core::files::new_temp_dir(parent, prefix)` and `new_temp_file(parent, prefix)`: a
   random name, `mkdir` with mode 0700 or `O_EXCL` with mode 0600, and a new name on `EEXIST`.
2. podnet, until the tag: one function in `podnet-core` with the same contract, and each path above uses it. A path
   that another process must find (a socket that a client opens by name) keeps a fixed name in a private directory
   of the user (`state_dir`), not in `/tmp`.
3. A check pins the class: `scripts/check-layers.py` (or a new check) refuses `process::id()` in a path of the code
   outside tests.

## Prove

```sh
cargo test -p podnet-core --lib temp
python3 -I scripts/check-layers.py
```

## Done

2026-11-25, the commit of this entry. `podnet-core` has `state::temp` (`nonce`, `random_path`, `new_temp_dir` with mode
0700, `new_temp_file` with `O_EXCL` and mode 0600, `beside`): a nonce of 64 bits from `getrandom`, the hasher seed of
the standard library and the time as the second path, and a new name on `EEXIST`. `state::temp_name` and
`state::writable` use it. The code paths of the Problem use it: the enrol directory of `initcfg`, the bind directory
of `router::guest::wire`, the scratch of `serve-at`, the capture file of `stat` (`O_EXCL`), the socket of `warp run`,
the default control socket of `link` (now `link-PID-NONCE.ctl`; the help, the manual and `docs/link.md` say so), the
probe directory, socket and abstract name of `vnet doctor`, the CA file of the TURN stand. `podnet-netns` (no
`podnet-core` dependency) writes the resolv.conf of a pod with `listen::write_new_file` (`O_EXCL`, mode 0644, a
random name); its `random_hex` no longer falls back to the pid. Tests: `cargo test -p podnet-core --lib temp` (5
passed): `temp_paths_are_new_private_and_distinct` (a planted path of the pid is not taken) and the class test
`no_temp_path_is_named_by_the_pid`, which scans each source file of the workspace and failed on the code before the
fix (`src/stat.rs:158`, shown once with the old `src/stat.rs`). The libraries of `podnet-core`, `podnet-netns` and
`podnet-vnet` pass. When podcrates tags the function of COORD-119 (#711), the take of that tag (T-137) replaces
`state::temp` with it.


# T-546: warp proxy: a client with no WARP session gets a TCP reset, and the live step hides the line of pod_warp

**Source:** `live` 45518282050 (`1bf4161`, 2026-11-25), step "WARP proxy".
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Three of four runs of `podnet warp proxy` in the step gave `curl: (97) Recv failure: Connection reset by peer` in the
SOCKS5 handshake; the fourth run carried 20 MB. The step printed no line of the cause: it sets
`RUST_LOG=podnet=debug`, and the proxy is in `pod_warp`. `pod_warp::proxy::Proxy::client` (podcrates `v0.83.0`) logs
`WARP proxy client refused: no session` and drops the stream with the greeting unread; Linux then sends a reset, and
the client gets no reply of its protocol. The run before (`live` 45517951377) showed the class of the cause: both
MASQUE doors gave a certificate of Let's Encrypt in place of the pinned key (T-406).

## Premise

- Read: the file `proxy.rs` of `pod-warp` in podcrates `v0.83.0`, the function `client`.
- Read: the job log of `live` 45518282050 (job 138893564385) and of `live` 45517951377 (job 138882163681).
- Not measured: that the resets of 45518282050 came from the path "no session". The next run with `pod_warp=debug`
  shows it.

## Approach

1. podnet (`.github/workflows/live.yml`, step "WARP proxy"): the burst runs log `pod_warp=debug`; a download that ends
   with no HTTP status and a line `refused: no session` is tried again after a wait, as a 429 is. When no default run
   had a session, the step gives a warning and does not measure the rate.
2. pod-warp (COORD-120, #712): a failure reply in the protocol of the client (SOCKS5 reply 1, HTTP 503) before a close
   with no reset, with a test. podnet takes it with the next tag.

## Prove

```sh
python3 -I scripts/check-workflows.py
# the next live run prints the line of pod_warp at a reset; the take of the tag with the fix of COORD-120
```

## Done

2026-11-25, the commit of T-583. podcrates `v0.84.0` has the fix of COORD-120 (PodUtils/podcrates@T-1365): a client of
the proxy with no WARP session gets the SOCKS5 reply 1 (also for `UDP ASSOCIATE`) or HTTP 503 `no WARP session:
CAUSE`, then a close with no reset; `Forward` with no session closes with no reset. Its tests are in the crate
(`proxy::tests::a_socks5_client_with_no_session_gets_the_reply_1`, `an_http_client_with_no_session_gets_the_status_503`,
`a_forward_with_no_session_closes_with_no_reset`). Step 1 (the `live` step) was done before.

# T-547: pod doctor: the netns backend is ok on a host where its TCP connect is refused (bind-denied)

**Source:** GitHub #713 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podlab run bind-denied -- podnet pod doctor` gives `backend netns: ok` (and `--json` `"netns": "ok"`). On the same host `podnet pod --backend netns ...` exits 3: "the netns backend cannot carry a TCP connect here (a TCP connect to 127.0.0.1 is refused: Permission denied (os error 13); the refusal holds in each network namespace); use --backend seccomp". `docs/pods.md` says that `pod doctor` "measures what the host permits and says which backend can run". The `--name-backend` line of `auto` also says "use --backend netns when a program fails" on this host.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19, lab `bind-denied`).
- Not measured: the code of `pod doctor`; the probe of the backend `netns` at its start.

## Approach

1. `pod doctor` runs the probe of a TCP connect that the backend `netns` runs at its start, and gives `fail` with that reason.
2. The `--name-backend` line of `auto` names `--backend netns` only when the doctor gives it `ok`.
3. A test in the lab `bind-denied` (the lane) pins the agreement of the doctor and of the start.

## Prove

`podnet pod doctor --json` and `podnet pod --backend netns -- /bin/true` in the lab `bind-denied` (`sh scripts/lab-lane.sh`)

## Done

2026-11-25, the commit of T-578 (one change for both). `pod doctor` has the row "TCP connect (127.0.0.1)", and its
verdict of netns and netns-kernel is the rule of `canrun.rs` that the start uses, with the cause in the text line.
Where `auto` takes seccomp because netns cannot run, the line of `--name-backend` does not say "use --backend netns".
Test: `tests/pod_backend_line.rs` `the_doctor_and_the_start_agree_where_netns_cannot_run` (TCP connect refused by
Landlock, `unshare` refused by a seccomp filter) failed on the code before the fix and passes now.

# T-548: logs: the log of a detached instance does not hold the lines that the start shows before the ready

**Source:** GitHub #714 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet net proxy --listen unix:... --detach --name np2`, then `podnet logs np2`: the log has only the line `podnet: ---- start ... ----`. The start showed `podnet: proxy    unix:...` (the same with `serve --local`). `docs/background.md` says that each start writes "its log lines from the start (also the lines before the ready, which the start shows too)".

## Premise

- Read: `crates/podnet-core/src/detach.rs`. The tee of the log (`report::set_log_tee`) copies the lines of `tracing` only. A banner is a `println!` to stdout, which is the pipe to the parent before the ready.

## Approach

`detach/copy.rs`: in a detached child, a pipe at stdout and at stderr until the ready; a thread copies each line to the descriptor of before and to the log (masked, with no colour codes). The ready puts the descriptors back and waits for the threads (1 s at most). An exit before the ready drains the pipes (`atexit`), so the error line of a failed start reaches the caller.

## Prove

`cargo test -p podnet --test detach`

## Correction

2026-11-25: `ci` 45537758563 (`ef14ef2`) failed in the jobs `restricted-host` and `no-loopback`: the case `serve --local 127.0.0.1:0` of the test needs a TCP listener of the loopback, which these hosts refuse. The case runs only where `test_host::loopback` passes.

## Done

2026-11-25, the commit of this entry. `tests/detach/logs.rs`, `the_log_holds_each_line_that_the_start_shows` (a table of `net proxy`, `net switch` and `serve --local`; each line that the start shows is in the log; a start that fails before its ready shows its error line): it failed on the code before the fix (`"podnet: proxy    unix:..." is not in the log`), and passes now. `cargo test --test detach`: 26 passed.

# T-549: warp proxy: a MASQUE door behind a TLS middlebox gives the step --ca-file

**Source:** GitHub #715 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet warp proxy --no-proxy --no-relay --ephemeral` on a host whose TCP 443 goes through a middlebox that terminates TLS (issuer `O=Anthropic, CN=Egress Gateway SDS Issuing CA (production)`) ends with `podnet: the tls hop failed; behind a proxy that inspects TLS, give its root with --ca-file; check the clock of the host`. `docs/warp.md` says that a proxy that terminates TLS cannot carry the MASQUE connection: no root helps, the key is pinned.

## Premise

- Read: `src/report.rs` (the second line of an error, by the hop) and the job log of `live` 45518282050: WARP's door shows `C=US, O=Cloudflare, Inc., CN=2024-02-27 Self-Signed Root`, and at times `C=US, O=Let's Encrypt, CN=YE1`.

## Approach

`podnet_core::hop::door_cert` gives the class of the issuer that the door showed (WARP, an outage of WARP's side, a middlebox) and its step; `report::error_line` gives that step for a failure that names the certificate of a door, before the hint of the hop. The text of `pod-warp` is T-550.

## Prove

`cargo test -p podnet --lib a_masque_door_failure_names_the_class_of_its_certificate` and `cargo test -p podnet-core --lib the_door_certificate_has_its_class_and_its_hint`

## Done

2026-11-25, the commit of this entry. Tests: `report::tests::a_masque_door_failure_names_the_class_of_its_certificate` (a table of the three issuers; no line gives `--ca-file`) and `hop::tests::the_door_certificate_has_its_class_and_its_hint`. Before the fix the line gave the hint of the hop `tls` (the issue shows it).

# T-550: pod-warp: a pin mismatch blames WARP for the certificate of a middlebox and says the cause two times

**Source:** GitHub #715 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The first line of the failure of #715 (from `pod_warp::open`) says "WARP's side shows such a certificate at times ... a later try passes then" for the certificate of a middlebox, and gives the TLS failure of the door two times.

## Premise

- Read: `open.rs` of `pod-warp` at podcrates `v0.83.0` (`why_shown`, the text after "the device record was read again").

## Approach

COORD-121 (#726, to podcrates): the class by the issuer, and each cause once. podnet takes the text with the next tag.

## Prove

`cargo test -p podnet --lib a_masque_door_failure_names_the_class_of_its_certificate` (with the tag of the fix)

## Done

2026-11-25, the commit of T-583. podcrates `v0.84.0` has the fix of COORD-121 (PodUtils/podcrates@T-1366): the line of a
door that shows another key names the class by the issuer of its certificate, and says each cause once
(`open::tests::the_cause_of_another_key_is_named_by_the_issuer_of_the_certificate` in the crate).
`report::tests::a_masque_door_failure_names_the_class_of_its_certificate` passes with the text of the tag.

# T-551: tunnel, send, receive, relay, offline: a JSON event with no schema and no podnet key

**Source:** GitHub #716, GitHub #720, GitHub #733 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet tunnel --json`: the event `verified` has no `schema` and no `podnet` (#716). `podnet send --json` and `podnet receive --json`: the events `code`, `sent` and `received` have neither (#720). `docs/cli.md`, JSON documents, rule 2: "each object has `schema` (a positive integer) and `podnet`".

## Premise

- Read: each `println!` of a JSON document in `src/`. `status::stamped` stamps a document; six sites printed without it: `run.rs` (verified), `transfer.rs` (two), `relay.rs` (the pair), `offline/replica.rs`, `offline/discover.rs`.

## Approach

Each site prints `status::stamped(doc)`. A test scans each source file of `src/` for a `println!` of a JSON document with no `stamped`.

## Prove

`cargo test -p podnet --lib each_printed_json_document_is_stamped`

## Done

2026-11-25, the commit of this entry. `status::tests::each_printed_json_document_is_stamped` (with a plant): it failed on the code before the fix with the six sites, and passes now.

# T-552: pod --serve: a listener that cannot bind gives the bare error of the system and exit 3

**Source:** GitHub #717 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet pod --serve tcp://127.0.0.1:18081 -x ...` with the port in use gives `podnet pod: Address in use (os error 98)` and exit 3; a unix path in a directory that does not exist and a refused TCP bind (lab `bind-denied`) are the same. `docs/cli.md`: "A listener that cannot bind ... exits 69, in each command with a listener. The error is one line: the cause, the address one time, and the next step".

## Premise

- Read: `crates/podnet-netns/src/frontdoor.rs` (`bind_front_door`) and `lib.rs` (each error of `--serve` exits 3).

## Approach

`bind_front_door` gives a `BindRefused` for each kind of a refused bind (in use, refused, a missing directory, an address of no interface): the address one time and the next step. `--serve` exits 69 for it.

## Prove

`cargo test -p podnet --test pod_usage a_front_door_that_cannot_bind_exits_69_with_the_address`

## Done

2026-11-25, the commit of this entry. `tests/pod_usage.rs`, `a_front_door_that_cannot_bind_exits_69_with_the_address` (a port in use, a unix path in a missing directory: exit 69, one line, the address one time, the step). Before the fix the command gave exit 3 and the bare error (the issue).

# T-553: doctor: the leg quic+warp is ok where a TLS middlebox answers at the MASQUE door

**Source:** GitHub #718 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet doctor` on a host whose TCP 443 goes through a TLS middlebox gives `tls ok: issuer O=Anthropic, CN=Egress Gateway SDS Issuing CA (production) (pinned per device; not judged here)` for the MASQUE door, `quic+warp ok`, `roads warp ok`, and a `next:` line for the default legs. `warp proxy` then fails with exit 69.

## Premise

- Read: `src/doctor/tls_check.rs`: a MASQUE door was never judged. The issuer of WARP's door is of Cloudflare (`live` 45518282050); at times WARP's side shows Let's Encrypt.

## Approach

`tls_check::masque_door`: the issuer of Cloudflare is ok; Let's Encrypt is unknown (an outage of WARP's side, try later); another issuer fails (a middlebox; MASQUE cannot pass; go around it). The leg `quic+warp` takes the verdict.

## Prove

`cargo test -p podnet --lib a_masque_door_is_judged_by_its_issuer`

## Done

2026-11-25, the commit of this entry. `doctor::tests::a_masque_door_is_judged_by_its_issuer` (a table of the three issuers and no certificate). Before the fix each door that answered TLS was `ok` (the issue).

# T-554: exit: a name with no address and a dial time limit get SOCKS5 0x01, not 0x04

**Source:** GitHub #719 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet exit --no-proxy`: a SOCKS5 CONNECT to `nothing.invalid:80` gets `0x01`, and to `192.0.2.1:443` gets `0x01` after 30 s. `docs/proxies.md`: "The host is unreachable, or the name has no address: `0x04`"; "A time limit: `0x04`".

## Premise

- Read: `socks_code` of `crates/podnet-netns/src/door.rs` (by `NoAddress` and the kind of an error of I/O), and the dialers of podnet (`src/dialers/mod.rs`, `src/egress.rs`): the class of a failure was `Unavailable` unless an error of I/O was in the chain, and the `DialError` of `pod-proxy` (with its class `Timeout`) was not read.

## Approach

`egress::class_of` takes the class of the `DialError` in the chain (a time limit, a name), else of the error of I/O. `socks_code` gives 0x04 for the classes `Timeout` and `NotFound` and the stage `Dns`.

## Prove

`cargo test -p podnet --lib the_reply_of_a_failed_dial_follows_its_class`

## Done

2026-11-25, the commit of this entry. `exit_stdio::tests::the_reply_of_a_failed_dial_follows_its_class` (a refused port 0x05, a name with no address 0x04, a time limit 0x04): it failed on the code before the fix (`a name with no address: [5, 1, ...]`, then `a time limit: [5, 1, ...]`), and passes now. `cargo test -p podnet --lib`: 493 passed.

# T-555: net proxy: a closed virtual port gets SOCKS5 0x01

**Source:** GitHub #721 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet net proxy --forward 10.66.0.2:8080=...`: a SOCKS5 CONNECT to `10.66.0.2:9999` (a closed port) gets `0x01`. `docs/networks.md` said `0x04`; `docs/proxies.md` gives `0x05` for a refused connection.

## Premise

- Read: `crates/podnet-vnet/src/proxy.rs` (`wait_established`: a refusal had the class `Unavailable`) and `socks_code`.

## Approach

A refusal of the stack has the class `Refused`; `socks_code` gives 0x05 for it (RFC 1928). `docs/networks.md` gives 0x05 for a closed port and 0x04 for an address that nothing owns.

## Prove

`cargo test -p podnet-vnet --lib a_closed_virtual_port_gets_the_socks5_reply_of_a_refusal`

## Done

2026-11-25, the commit of this entry. `proxy::tests::a_closed_virtual_port_gets_the_socks5_reply_of_a_refusal` (podnet-vnet): the reply is 0x05 at once. Before the fix the reply was 0x01 (the issue).

# T-556: init enroll: a failed dial of the relay drops the cause and gives no next step

**Source:** GitHub #722 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet init enroll DIR --node node-a --relay http://127.0.0.1:1` gives `the enrolment did not complete (INIT-3): dialling the relay 127.0.0.1:1: connecting to 127.0.0.1:1`, with no cause (refused, a name, a proxy) and no next step; other commands give both.

## Premise

- Read: `src/initcfg.rs`, `Registrar::register`: the error was formatted with `{e}` (the outer context only).

## Approach

`dial_failed`: the chain of the error (each cause one time) and the step of its hop, else a step for `--relay` and `--proxy`.

## Prove

`cargo test -p podnet --lib a_failed_enrolment_gives_one_line_with_the_cause_and_a_step`

## Done

2026-11-25, the commit of this entry. `initcfg::tests::a_failed_enrolment_gives_one_line_with_the_cause_and_a_step` (a refused relay: the cause and a step).

# T-557: init enroll: a relay URL that is not a relay prints the raw chunked HTML body

**Source:** GitHub #723 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet init enroll ... --relay https://example.com` gives `the relay answered 405: 22e`, then the HTML page and `0` on four lines.

## Premise

- Read: `parse_reply` of `src/initcfg.rs`: a status that is not 2xx gave the whole body.

## Approach

`refused_by_relay`: `HTTP STATUS`, one short line of a text body (no HTML, no chunked body), and the step "this URL may not be a relay of podnet: check --relay".

## Prove

`cargo test -p podnet --lib a_failed_enrolment_gives_one_line_with_the_cause_and_a_step`

## Done

2026-11-25, the commit of this entry. `initcfg::tests::a_failed_enrolment_gives_one_line_with_the_cause_and_a_step` (a chunked HTML 405 and a plain 403: one line, no body of HTML, no chunk size).

# T-558: host probe, stat --json: schema is a string and the podnet key is missing

**Source:** GitHub #724, GitHub #729 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet host probe DIR --json` gives `"schema": "podnet.host-probe/1"` and no `podnet` (#724); `podnet stat --json` the same with `podnet.stat/1` (#729). `docs/cli.md`: `schema` is a positive integer, `podnet` the version line, and a named form goes in `kind`.

## Premise

- Read: `src/hostprobe.rs` and `src/stat.rs`, the two documents with a string in `schema`.

## Approach

Both give `schema` 1, `kind` (the name of the form) and `podnet`. The scan of `each_printed_json_document_is_stamped` refuses a string in `schema` in the code of `src/`.

## Prove

`cargo test -p podnet --lib each_printed_json_document_is_stamped`

## Done

2026-11-25, the commit of this entry. `status::tests::each_printed_json_document_is_stamped` refuses a `"schema": "` in the code of `src/`; it found the two sites before the fix, and passes now.

# T-559: rules check: the documents do not give the fields of a policy rule

**Source:** GitHub #728 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A rule written from the words of the manual ("A rule names a host, a family, a port and an action") fails `podnet rules check`: `missing field name`, and `"port": 443` gives `invalid type: integer 443, expected a string`. No document gives the keys of a rule, that `name` is necessary, the form of `port` or of `family`, or the meaning of `*`.

## Premise

- Read: `RawRule` of `src/policy.rs` (`name`, `host`, `port` as a string, `family`, `action`) and `docs/host-runtime.md`, "The classified egress" (no example file).

## Approach

`docs/host-runtime.md` gives an example policy file and a table of each field. `port` takes a number too. The help of `rules check` gives one rule in full.

## Prove

`cargo test --test docs the_policy_example_of_host_runtime_md_parses`

## Done

2026-11-25, the commit of this entry. `tests/docs.rs`, `the_policy_example_of_host_runtime_md_parses` (the example of the document parses, with a port as a number and as a string).

# T-560: ci check: a declaration with schema 1 is refused, and each string is taken

**Source:** GitHub #730 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet ci check ci.json` with `{"schema":1,"hosts":[...]}` gives `invalid type: integer 1, expected a string` and exit 78; any string is taken. The help gives `{"schema": ..., ...}`, and `docs/cli.md` says that `schema` is a positive integer.

## Premise

- Read: `Declaration` of `src/ci.rs` (`schema: String`, not checked).

## Approach

The declaration takes `"schema": 1` with the `kind` `podnet.ci-egress/1` (or no `kind`), and the older `"schema": "podnet.ci-egress/1"`; another value exits 78 with the form to give. The help and `docs/host-runtime.md` give an example.

## Prove

`cargo test --test ci the_declaration_takes_the_stamp_and_refuses_another_form`

## Done

2026-11-25, the commit of this entry. `tests/ci.rs`, `the_declaration_takes_the_stamp_and_refuses_another_form` (five forms: three are read, two exit 78 with the form). Before the fix `"schema": 1` exited 78 (the issue).

# T-561: man --json SECTION: docs/cli.md names the keys variables, exits and table as sections

**Source:** GitHub #732 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet man --json exits`, `variables` and `table` exit 64. `docs/cli.md` says that `podnet man --json SECTION` takes `variables`, `files`, `exits`, `examples` or `table` for a topic. The binary takes `environment`, `files`, `exit-status`, `examples` and `inventory`, and these are the keys of the documents.

## Premise

- Measured: the key of each topic section of `podnet man --json`: environment (`variables`), files (`files`), exit-status (`exits`), examples (`examples`), inventory (`table`). The names of the sections come from `pod-manual`.

## Approach

`docs/cli.md` gives each section with its key.

## Prove

`cargo test --test docs each_topic_of_man_json_in_cli_md_runs_with_its_key`

## Done

2026-11-25, the commit of this entry. `tests/docs.rs`, `each_topic_of_man_json_in_cli_md_runs_with_its_key` (each section of the line runs, and its document has the key).

# T-562: autostart add: refuses a binary under /var/tmp as temporary, with exit 64

**Source:** GitHub #734 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`~/.local/bin/podnet autostart add qa1 -- net proxy ...` with HOME under `/var/tmp` gives `... is in a temporary directory: copy podnet to a lasting place (for example ~/.local/bin) first` and exit 64; `--manual` too. The step of the line cannot be followed.

## Premise

- Read: `autostart::add` of `crates/podnet-core/src/autostart.rs`: `/var/tmp` was in the list. FHS 5.15: `/var/tmp` is kept across a reboot.

## Approach

Refuse only a directory that the host clears at a reboot or a logout: the temporary directory, `/tmp`, `/dev/shm`, `$XDG_RUNTIME_DIR`. The exit is 78 (the place of the binary is a fault of the host, not of the command line). `docs/background.md` says so.

## Prove

`cargo test -p podnet-core --lib a_binary_is_refused_only_in_a_directory_that_the_host_clears`

## Done

2026-11-25, the commit of this entry. `autostart::tests::a_binary_is_refused_only_in_a_directory_that_the_host_clears` (a table of six paths; `/var/tmp` and a home are lasting).

# T-563: offline bundle build: the example of docs/offline.md (--file ./site:/site) does not run

**Source:** GitHub #735 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet offline bundle build --out ./bundle --name field --file ./site:/site` (the example of `docs/offline.md`) gives `./site: Is a directory (os error 21)` and exit 66; an absolute DEST exits 65.

## Premise

- Read: `files_of` of `src/cli/cmd/offline/bundle.rs`: a SRC was read as one file; the manifest refuses an absolute DEST (as the help says).

## Approach

A directory SRC gives each regular file under it at DEST/PATH, in the order of the names; a link or a special file in it exits 65. The example of the document has a relative DEST (`--file ./site:site`), and the help says that a directory is taken.

## Prove

`cargo test --test offline_bundle a_directory_source_gives_each_file_under_it`

## Done

2026-11-25, the commit of this entry. `tests/offline_bundle.rs`, `a_directory_source_gives_each_file_under_it` (the example form: 2 files; a link in the directory exits 65).

# T-564: serve --local: on a host with no loopback address the listener is ready, and no client can connect

**Source:** GitHub #736 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

In a lab with no address on `lo`, `podnet serve --local 127.0.0.1:18140` prints its banner and `--detach` is ready; `serve --check --local` passes (`can be bound (no listen)`); a client gets `Couldn't connect to server`. `forward -L` and `net proxy --listen tcp://127.0.0.1:...` start with no warning. `podnet net doctor` knows the fault (`TCP on the loopback does not work here: use unix sockets`).

## Premise

- Read: the GitHub issue (lab with no loopback).
- Not measured: the bind helpers of each command.

## Approach

1. One shared probe after a TCP bind on a loopback address: a connect to the bound address; a refusal gives one warning line with the next step (a unix socket), or exit 69 where the command has a unix form.
2. `serve --check --local` runs the same probe.
3. A test with a stand-in that refuses the connect (the seam of `net doctor`).

## Prove

`cargo test -p podnet-core --lib loopback` and the lab `no-loopback` of the lane

## Done

2026-11-25, the commit of this entry. `podnet_core::net::loopback::check` binds a second socket of a loopback
address on port 0 and connects to it (1 s): a connect that fails (not a refusal of this process, EACCES or EPERM)
exits 69 with the cause and the step (a unix socket). `net::bind_tcp` (`warp proxy`, `relay connect`, `net
switch`, `net proxy --listen tcp://`), `serve --local`, `git serve` and `share --local`, `forward -L` and `connect
-L` run it after the bind; `serve --check` fails the row of the listener. The listener of the command gets no
connection of the check. `docs/serve.md` says it. Test: `net::loopback::tests::a_loopback_that_no_connect_reaches_exits_69`
(a table: ENETUNREACH on IPv4 and IPv6, a time limit, an open loopback, an address that is not a loopback, EACCES).
The lab `no-loopback` of podlab `v0.84.0` (uid 65534): `serve --local 127.0.0.1:18140`, `forward -L` and `net
proxy --listen tcp://` exit 69 with the line (before: ready, and no client connects); `serve --check --local` fails
the row; on this host each starts as before.

# T-565: docs/virtual-network.md says that curl does not take a unix-socket SOCKS proxy

**Source:** GitHub #742 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** docs
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`docs/virtual-network.md`, 5.2: "A client must speak SOCKS5 over `AF_UNIX` itself, because `curl` does not accept a unix-socket SOCKS proxy." curl 8.5 takes `--proxy socks5h://localhost/PATH` and reaches the virtual network through `net proxy`.

## Premise

- Measured by the tester: `curl --proxy socks5h://localhost/PATH http://10.66.0.2:8080/` through `net proxy --listen unix:PATH` gives the page, also in the lab `bind-denied`.

## Approach

The document gives the curl form (curl 7.84 and later) and keeps the TCP listener for a client that takes no socket path.

## Prove

`python3 -I scripts/doc-commands.py --check`

## Done

2026-11-25, the commit of this entry. The paragraph gives the form; `doc-commands.py --check` passes.

# T-566: --log-format json: the last ERROR object has no timestamp

**Source:** GitHub #743 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

With `--log-format json`, each log object has `timestamp`, `level`, `fields` and `target`; the last object (the error that ends the command) has no `timestamp`.

## Premise

- Read: `error_json` of `src/cli/help.rs`: it wrote `level`, `fields` and `target` only.

## Approach

`error_json` writes `timestamp` in the form of the log of `tracing` (RFC 3339 UTC, microseconds).

## Prove

`cargo test --test cli log_format_json_gives_json_lines_only`

## Done

2026-11-25, the commit of this entry. `tests/cli/log.rs`, `log_format_json_gives_json_lines_only`: each JSON line of stderr has a `timestamp` of 27 characters (the last error line failed this before the fix).

# T-567: route --config: a missing or bad file exits 64; tunnel --config gives 78

**Source:** GitHub #747 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet route --config MISSING --check` and a file with an unknown key exit 64; `tunnel --config` exits 78 for the same faults, as `podnet man exit-status` says (78: a configuration file given with --config).

## Premise

- Read: `src/cli/cmd/route.rs` (`router::load(...).fault(Fault::Usage)`) and `docs/router.md` ("An unknown key is a usage error (exit 64)").

## Approach

A file that cannot be read or used exits 78 (also `--export mwan-json` with no kernel section). `docs/router.md` says so.

## Prove

`cargo test --test detach route_detaches_and_lists_its_members`

## Done

2026-11-25, the commit of this entry. `tests/detach/route.rs`: a bad file, a missing file and an unknown key exit 78 (the test asserted 64 before).

# T-568: wg serve: on a host that refuses UDP, the step says to give a port above 1023 or a unix socket

**Source:** GitHub #748 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podlab run udp-denied -- podnet wg serve --config wg.conf` (`ListenPort = 51900`) gives `this host refuses the bind (a port below 1024 needs privilege, and some hosts refuse each TCP listener); give a port above 1023, or a unix socket`. The port is above 1023, the host refuses UDP, and `wg serve` takes no unix socket.

## Premise

- Read: `bind_step` of `crates/podnet-core/src/net.rs`: one step for each TCP and UDP address.

## Approach

`Listener::Udp` and `Listener::UdpPrivileged` with their steps (a host that gives no UDP; a port below 1024); `net::bind_fault_udp(port)` for each UDP bind: `wg serve`, `forward -L udp:`, the DNS door of `forward` and of `warp`.

## Prove

`cargo test -p podnet-core --lib net::tests`

## Done

2026-11-25, the commit of this entry. `net::tests::a_refused_bind_is_69_with_one_line_and_a_next_step` (the table of the kinds of a refused bind) has the UDP rows: a high port names a host with no UDP and no `1023` and no `unix:`.

# T-569: serve --local, net proxy --listen: where getsockname is refused, a bare Not supported and exit 1

**Source:** GitHub #750 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podlab run getsockname-denied -- podnet serve --dir D --local 127.0.0.1:18220` and `net proxy --listen tcp://127.0.0.1:0` give `podnet: Not supported (os error 95)` and exit 1; with a fixed port the address is known, and the listener still fails.

## Premise

- Read: the binds of `serve --local` (`src/cli/cmd/local.rs`, `crates/podnet-serve/src/road.rs`) and of `net proxy` and `net switch` (podnet-vnet): `local_addr()?`. `forward` has a second path already (T-502).

## Approach

`net::bound_addr`: getsockname, else the asked address when its port is fixed; with port 0, exit 69 with the flag, the cause and the step (a fixed port or a unix socket). The four binds use it.

## Prove

`cargo test -p podnet-core --lib a_refused_getsockname_has_a_second_path`

## Done

2026-11-25, the commit of this entry. `net::tests::a_refused_getsockname_has_a_second_path`. In the lab `getsockname-denied`: `serve --local 127.0.0.1:18220` serves, and `--local 127.0.0.1:0` gives one line with the step.

# T-570: --egress, forward --network: an unknown spec is read as a proxy HOST:PORT

**Source:** GitHub #745 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet --egress bogus:thing bench ...` gives `proxy port: invalid digit found in string`; `--egress wireguard` gives `proxy "wireguard" needs host:port`; `forward --network bogus:thing` does not name its flag. The line does not say that the spec is not known and does not give the known specs.

## Premise

- Read: `kind` of `src/networks.rs`: a spec with no scheme went to the parser of a hop of the pod (a proxy HOST:PORT).

## Approach

A spec with no scheme that is not `HOST:PORT` (a port of 0 to 65535) and not `unix:` is not known: the line gives the first form of each row of the registry. A proxy scheme that is not known gives the same list. `forward --network` names its flag.

## Prove

`cargo test -p podnet --lib network_spec_an_unknown_spec_gives_the_known_forms`

## Done

2026-11-25, the commit of this entry. `networks::tests::network_spec_an_unknown_spec_gives_the_known_forms` (`bogus:thing`, `wireguard`, `foo://x`: no "proxy port", the forms are named; `127.0.0.1:1080` is still a proxy).

# T-571: forward: a UDP forward and the DNS door over the way of the process carry no datagram

**Source:** GitHub #746 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`podnet forward -L udp:15401=127.0.0.1:15400` with no proxy logs `a UDP flow ended: direct carries no UDP`, and the client gets no answer; the DNS door answers SERVFAIL. `docs/forward.md`: "`direct` and `wg:FILE` carry UDP."

## Premise

- Read: `Egress` of `src/egress.rs` (the way of the process, label `direct`): it had no `datagrams`, so the default of `pod_core::Dial` refused. `--network direct` (`dialers::Direct`) carried UDP.

## Approach

`Egress::datagrams`: with no proxy for UDP, the socket of the system (as `dialers::Direct`); with a proxy, a refusal that names the proxy.

## Prove

`cargo test --test forward_udp forward_carries_udp_over_the_way_of_the_process`

## Done

2026-11-25, the commit of this entry. `tests/forward_udp.rs`, `forward_carries_udp_over_the_way_of_the_process` (a UDP forward and the DNS door, both over `--no-proxy`, to a UDP echo): it failed on the code before the fix (`no answer`), and passes now.

# T-572: pod: PODNET_POLICY does not refuse a destination

**Source:** GitHub #725 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

With `PODNET_POLICY` that refuses `example.com` (or `"default":"refuse"`), `podnet pod -x PROXY -- curl https://example.com` gives 200 on the backends netns and seccomp and through `--serve`; `bench` and `forward` refuse with the same file. `exit` ignores it too.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- A fix agent wrote the change on `ef14ef2` (the directory docs/reports/wip-2026-11-25 of `405d47c`); the lead reviewed it.

## Approach

A permit hook in podnet-netns that `src/main.rs` sets from `podnet::policy::permit`; each dial of the pod asks it; a refusal has the class Policy.

## Prove

`cargo test --test pod_policy`

## Done

2026-11-25, the commit of this entry (a fix agent wrote it on `ef14ef2`; the lead reviewed it and took it on
`main`). `podnet_netns::rules::set_permit` holds the check of the egress policy of the process; `src/main.rs` sets it
to `podnet::policy::permit`. `select_chain` and `select_chain_udp` ask it before the rules of `-r`, so the netns,
netns-kernel and seccomp backends and `--serve` refuse a destination that the policy refuses: `ECONNREFUSED` (netns:
a reset), SOCKS5 `0x02`, HTTP 403. The line names the rule of the policy (`refused by PODNET_POLICY: ...`); the flow
log names the rule `policy`; `--learn` does not send such a flow through `-x`. Tests: `tests/pod_policy.rs` (2: the
program on each backend that runs here, and the door) failed on the code before the fix (`5809e66` with the tests of
the fix: 0 passed, 2 failed) and pass now; `rules::tests::the_policy_refuses_before_the_rules` passes.

# T-573: bench, connect: a refusal of PODNET_POLICY gives the next step of a connect failure

**Source:** GitHub #727 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A refusal of the policy ends with `the connect hop failed; check the host and the port of the target ...`, not the step of the policy.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- A fix agent wrote the change on `ef14ef2` (the directory docs/reports/wip-2026-11-25 of `405d47c`); the lead reviewed it.

## Approach

`report::error_line` gives the step of the policy (the rule, `PODNET_POLICY`, `podnet rules check FILE`) for a failure of the class Policy.

## Prove

`cargo test -p podnet --lib report`

## Done

2026-11-25, the commit of this entry (a fix agent wrote it; the lead reviewed it). `src/policy/said.rs` reads a
refusal of the policy in the chain (the typed `Refused` first, then the words of its reason for a layer that keeps
only the text), and `report::error_line` gives its step before each other hint: the rule (or the default), the
variable and the file of the policy, and `podnet rules check FILE`. `connect` says "the egress policy refuses the
remote", not "does not answer". `egress::dial`, `dial_direct`, `dial_addrs`, `permit_fault` and `permit_failure` keep
the typed `Refused` in the chain. Tests: `tests/egress_refusal.rs`
`a_refusal_gives_the_step_of_the_policy_in_each_command` (bench direct, bench with `--proxy`, connect) failed on the
code before the fix and passes now; the unit tests of `policy::said` pass.

# T-574: warp status, rotate, delete, license: a missing identity file exits 78, not 66

**Source:** GitHub #731 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A missing identity file exits 78; `podnet man exit-status` gives 66 for a file that must exist.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- A fix agent wrote the change on `ef14ef2` (the directory docs/reports/wip-2026-11-25 of `405d47c`); the lead reviewed it.

## Approach

Exit 66 (no-input) for an absent identity file.

## Prove

`cargo test --test cli warp`

## Done

2026-11-25, the commit of this entry (a fix agent wrote it; the lead reviewed it). An absent identity file of `warp
status`, `rotate`, `delete` and `license` gives `Fault::NoInput` (66); a stat error and `PODNET_STATE_DIR=none` stay 78.
The row 66 of `podnet man exit-status` names the WARP identity. Test: `tests/exit_contract.rs`
`exit_66_is_a_missing_warp_identity` (the four commands, at the default path and at `--identity`) failed on the code
before the fix (exit 78) and passes now.

# T-575: bench, connect, tunnel, relay connect, forward: the line of a name that does not resolve is cut with ... in a cause

**Source:** GitHub #737 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

The error line of a name that does not resolve is cut with `...` in the middle of a cause:

```
podnet: bench: no request worked: GET http://nothing.invalid/: connecting to nothing.invalid:80: could not resolve nothing.invalid: system resolver: failed to lookup address information: Name does not resolve; DNS over HTTPS: 8 resolvers failed, 4 with: TLS: invalid peer certificate: cert...
```

The line says "4 with:" and does not give the cause of the other 4 resolvers. The same cut is in `connect`, `relay
connect`, the WARN line of `forward` and `net proxy --forward`, and in the lab `no-proxy-no-dns` (`Address family not
supported...`).

## Premise

- Read: `order.rs` and `lookup.rs` of `pod-dns` at podcrates `v0.84.0`. `order::fit` cuts the joined failures of the
  sources at 160 bytes with `pod_core::text::clip`; `lookup::fold` names only the cause of the largest group of
  resolvers. The whole text goes only to the log of `pod-dns` at debug.
- Read: the change of a fix agent (the directory docs/reports/wip-2026-11-25 of `405d47c`): a layer of
  `tracing` in podnet that keeps the two debug events of `pod-dns` and puts their text back into the error line.

## Approach

The cut is made in `pod-dns`: the fix goes there (COORD-123, #753, to podcrates). podnet takes it with the next tag.

## Decision

2026-11-25: the change of the fix agent is not taken. It reads the words and the targets of the debug log of another
crate, so a change of a log line of `pod-dns` breaks the error line of podnet with no failed build. The rule "Shared
code first" (`docs/decisions.md`, 2026-11-17) puts the fix into the crate that makes the cut.

## Prove

```sh
cargo test -p podnet --test error_words
```

## Blocker

The fix of `pod-dns` (COORD-123, #753, to podcrates): no cut inside a cause, and each group of resolvers with its cause.

# T-576: warp up: the failure prints two lines of about 1500 characters that repeat each cause

**Source:** GitHub #738 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`warp up` fails with two lines of about 1500 characters; each cause comes several times.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- A fix agent wrote the change on `ef14ef2` (the directory docs/reports/wip-2026-11-25 of `405d47c`); the lead reviewed it.

## Approach

One line that names each cause one time (the tidy of `exit::text`); a repetition in `pod-warp` goes to COORD-121.

## Prove

`cargo test -p podnet --lib warp`

## Done

2026-11-25, the commit of this entry (a fix agent wrote it; the lead reviewed it). `warp up` keeps the error lines of
the child until a log line comes after them, so the failure says the cause and the steps of the child once. The
relay leg of WARP (`src/warp/relay_dial.rs`) gives the cause alone at the stage relay (pod-warp names the door and
the relay); an IPv6 door is "the relay has no IPv6 egress". `ws_relay::connect` names the failed hosts of the relay
with the same words together. `exit::tidy` names a way out ("through the relay URL", "through the proxy URL") once
in a line. The text of `pod-warp` says each cause once since podcrates `v0.84.0` (COORD-121, T-583). Tests:
`warp::relay_dial::tests::a_failure_through_the_relay_names_each_cause_once`,
`warp::up::tests::the_error_of_the_child_is_said_once` and a row of `exit::tests::tidy_says_each_part_once_and_no_debug_form`
(`cargo test -p podnet --lib warp::`: 74 passed). These are unit tests of new seams: the code before the fix has no
such seam, so the failure before the fix is the output of the issue.

# T-577: pod --report: the backend netns-kernel is not a value of pod-report.schema.json

**Source:** GitHub #739 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`pod --report` writes `"backend": "netns-kernel"`, which `docs/schema/pod-report.schema.json` does not allow.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- A fix agent wrote the change on `ef14ef2` (the directory docs/reports/wip-2026-11-25 of `405d47c`); the lead reviewed it.

## Approach

The schema takes the value of the documented backend.

## Prove

`cargo test --test pod_report`

## Done

2026-11-25, the commit of this entry (a fix agent wrote it; the lead reviewed it). `pod-report.schema.json` takes the
backend `netns-kernel`; `docs/pods.md` names the value, and `udp_limit` null for `netns-kernel`. Test:
`tests/pod_report.rs` `exit_status_of_the_program_is_in_the_report` (a table of the three backends; each report agrees
with the schema) failed on the code before the fix and passes now.

# T-578: pod --backend netns (userns-off): the start failure gives no next step, and the report has no true reason

**Source:** GitHub #740 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

In the lab userns-off, `pod --backend netns` fails with no next step, and the report gives no true reason.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- A fix agent wrote the change on `ef14ef2` (the directory docs/reports/wip-2026-11-25 of `405d47c`); the lead reviewed it.

## Approach

One line with the cause and the step (`--backend seccomp`); the true reason in the report. One probe for the doctor and the start (with T-547).

## Prove

`cargo test -p podnet-netns --lib doctor`

## Done

2026-11-25, the commit of this entry (a fix agent wrote it; the lead reviewed it). `crates/podnet-netns/src/canrun.rs` has
the one rule of the netns backend (the test hook, the TCP connect of T-382, `unshare` with or without a user
namespace). `--backend netns` and `--backend netns-kernel` run it before the start: a refused namespace gives one line
with the cause and the step ("use --backend seccomp, or run the pod where an unprivileged user namespace works"),
exit 3, and a report with `backend` null, `hold` none and that line as the reason. Tests: `tests/pod_backend_line.rs`
`the_doctor_and_the_start_agree_where_netns_cannot_run` and `tests/pod_error_lines.rs`
`each_refusal_of_the_netns_backend_is_one_line` failed on the code before the fix and pass now;
`canrun::tests::the_netns_rule_and_its_line` passes.

# T-579: tunnel: behind a proxy the start waits about 35 s in DNS over HTTPS with no line

**Source:** GitHub #741 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

Behind a proxy, `tunnel` waits about 35 s in DNS over HTTPS with no line at the default level and no cause at `-v`.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- Read: the code path is in `pod-cloudflare` of podcrates `v0.84.0` (podnet only shows its text and its wait).

## Approach

A line at the default level when a step of the resolver chain is slow, with its cause; a shorter limit for a resolver that does not answer.

## Prove

`cargo test -p podnet --lib resolve`

## Blocker

The fix of `pod-cloudflare` (COORD-125 (#755), to podcrates). podnet takes it with the next tag; the release notes of `v0.4.0-beta.19` name the limit (`docs/decisions.md`, 2026-11-25).

# T-580: --proxy http+exec: a command that does not exist floods stderr and ends with a wrong cause

**Source:** GitHub #744 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

`--proxy http+exec:/nonexist/prog` prints one `sh: ... not found` line for each try (2430 in 1 s), hides the whole command, and ends with "the connection closed before the response ended".

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- Read: the code path is in `pod-proxy` of podcrates `v0.84.0` (podnet only shows its text and its wait).

## Approach

Capture the stderr of the command; a command that ends before its first byte gives its exit status and its stderr once; show the command when it has no secret; stop the tries of a command that cannot start.

## Prove

`cargo test --test proxy_exec`

## Blocker

The fix of `pod-proxy` (COORD-124 (#754), to podcrates). podnet takes it with the next tag; the release notes of `v0.4.0-beta.19` name the limit (`docs/decisions.md`, 2026-11-25).

# T-581: pod doctor: seccomp and netns are ok in no-proc and proc-masked, where the pod cannot start

**Source:** GitHub #749 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

In the labs no-proc and proc-masked, `pod doctor` says ok and exits 0; the pod cannot start.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- A fix agent wrote the change on `ef14ef2` (the directory docs/reports/wip-2026-11-25 of `405d47c`); the lead reviewed it.

## Approach

The doctor runs the probes of the start of each backend (with T-547, T-578).

## Prove

`cargo test -p podnet-netns --lib doctor`

## Done

2026-11-25, the commit of this entry. The unfinished change of a fix agent, reviewed and finished by the lead. The
rule of the netns backend (`canrun::netns_probe_with`) asks the probe of the id maps where only a user namespace
works: the parent writes `setgroups`, `uid_map` and `gid_map` of a child, as the start does (`canrun::probe_id_maps`);
a masked `/proc` refuses them. `pod doctor` has that row, and its verdict of seccomp is the rule of the start
(`canrun::seccomp_probe_with`: the filter and the read of the memory of a program). Where netns cannot run, the line
of a seccomp backend that cannot read the memory does not send the user to `--backend netns`. A fault of the change
was fixed: the child of the probe held the write end of its own pipe and waited without end (the lib tests hung);
the child closes the ends of the parent, and the parent kills the child after the maps. Tests:
`canrun::tests::the_netns_rule_and_its_line` (a row for each probe), `the_seccomp_rule_names_the_probe`,
`the_id_maps_probe_gives_one_row`, `doctor::tests` (`cargo test -p podnet-netns --lib`: 177 passed).

# T-582: update --check: a proxy that does not answer costs 60 s and the error gives no next step

**Source:** GitHub #751 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`update --check --proxy http://192.0.2.1:3128` tries the API and then releases.atom through the same proxy (2 x 29 s), says the cause two times, and gives no next step.

## Premise

- Read: the GitHub issue (black-box QA pass of 0.4.0-beta.19).
- Not measured: the code path.

## Approach

A time limit of the proxy ends both tries (the second path goes another way, or is skipped); the cause one time; the step of the proxy hop.

## Prove

`cargo test -p podnet --lib update`

## Done

2026-11-25, the commit of this entry. `update::newest` does not read `releases.atom` after a failure of the proxy
hop of the API: the feed goes through the same proxy, so a proxy that does not answer cost a second time limit. The
error is the typed error of the API, so the line names the cause once and the second line gives the step of the proxy
hop (the joined text before lost the type, and the hint with it). Test: `tests/egress.rs`
`a_failed_proxy_ends_update_check_after_one_try` failed on the code before the fix (the line held `releases.atom` and
the cause two times) and passes now.

# T-583: Take podcrates v0.84.0

**Source:** `PodUtils/podcrates` tag `v0.84.0` (`ca1d3e3b`, 2026-11-25); PodUtils/podcrates#260 (COORD-155); the answers
to COORD-119 (PodUtils/podnet#711), COORD-120 (PodUtils/podnet#712) and COORD-121 (PodUtils/podnet#726).
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.83.0`. The tag `v0.84.0` answers COORD-119 (`pod_core::files::new_temp_dir` and
`new_temp_file`, PodUtils/podcrates@T-1343), COORD-120 (a client of the proxy of `pod-warp` with no session gets the
SOCKS5 reply 1 or the HTTP status 503, PodUtils/podcrates@T-1365) and COORD-121 (a pin mismatch names the class by the
issuer of the door certificate, PodUtils/podcrates@T-1366). Its breaking items for podnet: `Class::Data` (65), the
workspace dependency `h2` is the fork `pod-h2`, the `Display` of a failure of `pod-cloudflare` gives its step alone,
and `standin::edge::listen` is removed.

## Premise

- Read: COORD-155 and its section "podnet". The tag has no `adoption/podnet/` patch. `core.sh`, `core-net.sh`,
  `pins.py` and `release-index.py` did not change (their SHA-256 at `ca1d3e3b` is the one of `scripts/install/SOURCES`
  and of `release.yml`).
- Read: podnet names no item of the crate `h2` in its code; the root dependency `h2` becomes `pod-h2`, and the lock has
  one `h2`.
- Read: `exit::text` and the rows of `--check` (`report::chain_text`) walk the `source()` chain and say each cause
  once, so the step-only `Display` of `pod-cloudflare` keeps its causes in a podnet line.
- Read: podnet does not call `standin::edge::listen`.

## Approach

1. Each `pod-*` pin names `v0.84.0` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`, `scripts/install/SOURCES`,
   `issue-sync.yml`, and the checkout of `release-index.py` in `release.yml`); `h2` is `pod-h2` of the tag.
2. `podnet_core::state::temp::{new_temp_dir, new_temp_file}` call the functions of `pod-core` (the name is
   `PREFIX-RANDOM`), and add the parent to the text of an error. `nonce`, `random_path` and `beside` stay: they give a
   name only, for a socket that `bind` makes and a file that a rename moves.
3. `exit::code_of_class` gives `DATA_ERROR` (65) for `Class::Data`.
4. T-546 and T-550 take the fixes of the tag.

## Prove

```sh
cargo test --workspace
cargo test -p podnet-core --lib temp
cargo test -p podnet --lib a_masque_door_failure_names_the_class_of_its_certificate
```

## Done

2026-11-25, the commit of this entry. Each `pod-*` pin names `v0.84.0` (`ca1d3e3b`): the manifests, `Cargo.lock`,
`fuzz/Cargo.lock`, `scripts/install/SOURCES`, `issue-sync.yml` and the checkout of `release-index.py`; `h2` is
`pod-h2` 0.4.20 of the tag, and the lock has no `h2` of crates.io. `state::temp::{new_temp_dir, new_temp_file}` call
`pod-core` (podnet's loop is gone); `exit::code_of_class` gives 65 for `Class::Data`. The text of `pod-extract` for a
component over 255 bytes changed: `tests/transfer_land.rs` reads "more than 255 bytes" (exit 65 as before).
`cargo clippy --workspace --all-targets --features full -- -D warnings` passes. Tests: `cargo test -p podnet-core
--lib` (`state::temp` and `exit::tests::each_class_of_a_crate_has_a_code_of_podnet`), and `cargo test --workspace
--no-fail-fast`: each binary passes but two SIGHUP cases (`tests/pod_signals.rs`, `tests/signals.rs`), which failed only because the run
was under `nohup` (SIGHUP ignored for each child) and pass alone (`cargo test --test pod_signals`: 1 passed, two times; `cargo test --test signals`: 3 passed).

# T-584: send --json: the sent event of a text gives "bytes": 0

**Source:** GitHub #720 (`[qa 0.4.0-beta.19]`, black-box QA pass of 2026-11-25); the rest of T-551.
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`podnet send --json --text` of an 8-byte text prints `{"event": "sent", "bytes": 0, ...}`. T-551 fixed the stamp of
the event only.

## Premise

- Read: `src/transfer/send.rs`, `send_text`: a text gives `bytes: 0` (the field held the bytes of the transit only).

## Approach

`bytes` of a text is its length in UTF-8; `sha256` stays null. `docs/transfer.md` says it.

## Prove

`cargo test --test cli transfer::send_and_receive_on_the_loopback`

## Done

2026-11-25, the commit of this entry. `tests/cli/transfer.rs` `send_and_receive_on_the_loopback` reads the `sent`
event of a text: it failed on the code before the fix (`bytes` 0, want 11) and passes now.

# T-585: Publish `v0.4.0-beta.19`: the fixes of the QA pass of 2026-11-25 and podcrates `v0.84.0`

**Source:** `docs/maintenance.md`, Release; `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease when a batch of
fixes is green. The black-box QA pass of 2026-11-25 (GitHub #713 to #751) and the proof of T-487 (COORD-109,
PodUtils/podnet#656).
**Category:** release
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.18` has none of the fixes of the QA pass of 2026-11-25 (T-545 to T-584), the take of podcrates
`v0.83.0` and `v0.84.0`, and the row `nat` of `doctor`. The steps of the image of T-487 have no proof yet (the job
`image` of beta.18 failed after the push, T-538).

## Premise

- The steps are `docs/maintenance.md`, "Release". Step 0 (the black-box QA pass) found 38 issues (#713 to #751, not
  #726). Each is an entry. Each P1 and P2 entry is done, but T-575, T-579 and T-580: their cause and fix are wholly in
  a crate of podcrates (COORD-123 #753, COORD-125 #755, COORD-124 #754). The release does not wait for them
  (`docs/decisions.md`, 2026-11-25, A QA finding whose whole fix is a crate of podcrates). The notes name the limits.
- The notes have `## Breaking`: the exit codes, the verdicts of `pod doctor` and the policy of `pod`.

## Approach

1. Set the version `0.4.0-beta.19` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`), write the llms files again.
2. Write `docs/releases/v0.4.0-beta.19.md`, with `## Breaking`.
3. Push; wait until `ci` and `live` pass on that commit. Dispatch `gate.yml` on that commit and wait until it passes.
4. Dispatch the dry run of `release.yml` (`publish` empty).
5. Dispatch `release.yml` with `publish=v0.4.0-beta.19`.
6. Check the files of the release, `release-proofs.yml`, and the job `image` (the attestation of the manifest list,
   the pull-back check, the section "The image, by digest" of the notes). Write the digests in a hand-off COORD issue
   `(to all)` and in COORD-109 (#656), and close #656 through the record.

## Prove

```sh
gh release view v0.4.0-beta.19 -R PodUtils/podnet
gh run list -R PodUtils/podnet --workflow release-proofs.yml --limit 1
```

The job `image` of the release run passes, and the notes of the release have the section "The image, by digest".

## Correction

2026-11-25: `ci` 45632652111 and `gate` 45632656832 on `66b29ce` failed on the faults of new tests (the key `nat` of
`doctor --json`, the relay test of T-576 on a host with no loopback, the rows of `forward` of the lane in the lab
`no-loopback` after T-564, and the backend line of `warp exec` after T-547): fixed in `2fdb97c`, `b4aa4a9` and
`2ef8e3a`. `live` failed three times at outside services (WARP's door, the quota of the quick-tunnel service): T-587,
T-589. The take of podcrates `v0.85.0` (T-588) went in before the tag.

## Done

2026-11-25, the tag `v0.4.0-beta.19` at `2ef8e3a`. `ci` 45646491033 and `live` 45646491036 passed on `2ef8e3a`;
`gate` 45640209602 on `b4aa4a9` (the same code but one test file). The dry run 45652900346 passed. `release.yml`
45659243262 (`publish=v0.4.0-beta.19`) made the tag; the release run 45660048233 built each of the eight targets,
passed `gate / lane`, published the prerelease with 18 files, and passed `proofs` and `image` (the attestation of the
manifest list, the pull-back check of each image, the section "The image, by digest" of the notes).
`release-proofs.yml` 45668331159 passed. The x86-64 binary of the release passes `sha256sum -c` and prints `podnet
0.4.0-beta.19 (2ef8e3a...)`. COORD-129 (#759) hands off the files and the digests; COORD-109 (#656) is closed.

# T-586: link p2p: take the relay first when the NAT classes of both ends rule out the direct path

**Source:** T-543 step 3 (PodUtils/podcrates#259, COORD-154: "Take `pod-natprobe`").
**Category:** feature
**Milestone:** M8
**Priority:** P3
**Effort:** M
**Status:** blocked

## Problem

`podnet link p2p` tries the direct path first and the TURN relay after a refusal or a time limit. Where both ends are
behind a dependent mapping (RFC 4787), the direct path cannot work, and the try costs its time limit.

## Premise

- Read: `src/cli/cmd/link/p2p.rs` and `src/link_p2p.rs`: the dial has no channel to the peer before the direct try.
- Read: `pod_natprobe::Report::compact_profile` (20 bytes for a peer) and `strategy::predict` (the strategy of two
  peers from their classes).

## Approach

1. A channel that carries the compact profile of each end before the dial (the pairing of `link`, or the relay).
2. `strategy::predict` over both profiles: the relay first where it says that the direct path cannot work.
3. A test over the simulator of `pod-natprobe` for each pair of classes.

## Prove

`cargo test -p podnet --test link_p2p`

## Blocker

A channel between the two ends before the dial: `link p2p` has none (step 1).

# T-587: live: a second run of doctor sees the leg quic+warp as not ok, and the step prints no reason

**Source:** `live` 45625694770 (`405d47c`, the step "podnet doctor") and `live` 45628685352 (`581b852`) and
45624138074 (`5809e66`, the step "Quick tunnel with UDP and TCP 7844 dropped (staggered legs)"), 2026-11-25.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

Each step runs `podnet doctor` two times: the text, then `--json`. In three runs of one day the text run gave the leg
`quic+warp` ok, and the `--json` run a second later gave it not ok; `jq -e` printed `false`, and the step printed no
reason of the leg.

## Premise

- Read: the job logs of the three runs. The text runs list "4 of 4 legs" (or `quic+warp ok` with 7844 dropped).
- Read: T-406 and `live` 45589043028: WARP's door shows a certificate of Let's Encrypt at times in place of its key; the
  leg is then `unknown` with that reason. Not measured: the reason of the three `--json` runs (the steps did not print
  it).

## Approach

`scripts/live-doctor.sh` runs `doctor --json` until the leg `quic+warp` is ok, at most three times with 15 s between
the runs, and prints the legs with their reasons after each run that is not ok. Both doctor steps of `live.yml` use
it; their time limits grow to 5 and 7 minutes.

## Prove

```sh
python3 -I scripts/check-workflows.py
sh -n scripts/live-doctor.sh
# the next live runs: a run that is not ok prints its legs
```

## Correction

2026-11-26: `live` 45731580672 (`49baa57`) proves the print: three runs 15 s apart each gave the leg `quic+warp`
`unknown` with "issuer C=US, O=Let's Encrypt, CN=YE1: WARP's door shows a certificate of Let's Encrypt at times in
place of its key" (T-406). The retry does not help a run of that outage. `scripts/live-step.sh` (T-589) now turns a
step that fails with that line into a warning, as it does with the quota of the quick-tunnel service.

## Done

2026-11-25, the commit of this entry. `python3 -I scripts/check-workflows.py`: 11 files, 0 faults. The proof is the
next `live` run on `main`.

# T-588: Take podcrates v0.85.0

**Source:** `PodUtils/podcrates` tag `v0.85.0` (`1d1c9fc3`, 2026-11-25); PodUtils/podcrates#262 (COORD-156) and
PodUtils/podcrates#263 (COORD-157, the owner's ruling "Hosted sessions and worker roles").
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

podnet named podcrates `v0.84.0`. The tag `v0.85.0` makes the text of a failure its step (PodUtils/podcrates@T-1351): `Failure::io(stage,
step, e)` takes a step, and the texts of `pod-cloudflare`, `pod-quic` and `pod-warp` changed. It gives random temp names
to `pod-link`, `pod-warp` and five other crates (PodUtils/podcrates@T-1343). COORD-157 asks each repository to keep the role table of the
lead and its workers in `docs/hosted-sessions.md`.

## Premise

- Read: the section "podnet" of COORD-156 and COORD-157. The tag has no `adoption/podnet/` patch. `core.sh`,
  `core-net.sh`, `pins.py` and `release-index.py` did not change.
- Read: podnet finds a refused registration with `downcast_ref::<pod_cloudflare::fail::Refused>` over each link of the
  chain (`hop.rs`), which is what `fail::find` does.

## Approach

1. Each `pod-*` pin names `v0.85.0` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`, `scripts/install/SOURCES`,
   `issue-sync.yml`, the checkout of `release-index.py`).
2. Each call of `Failure::io` gives its step (17 calls).
3. The tests that pin a changed text read the new text.
4. `docs/hosted-sessions.md` has the role table and the brief of a worker of podcrates `v0.85.0`; `docs/decisions.md`
   has the ruling.

## Prove

```sh
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets --features full -- -D warnings
```

## Done

2026-11-25, the commit of this entry. Each `pod-*` pin names `v0.85.0` (`1d1c9fc3`); the installer core and
`release-index.py` did not change. Each of the 17 calls of `Failure::io` gives its step (for example "a UDP socket on
0.0.0.0:0", "sending the WireGuard handshake", "TCP to ADDR through LABEL"). `dialer::tests::
dialer_tailnet_gives_the_refusal_of_the_daemon` reads `Failure::chain()` (the text of the failure is its step now).
`cargo clippy --workspace --all-targets --features full -- -D warnings` passes. `cargo test --workspace
--no-fail-fast`: each binary passes but `doctor_json_keys_are_fixed` (the key `nat` of T-543, fixed in the commit
before) and the test of the tailnet above (fixed here); both pass now. `docs/hosted-sessions.md` has the role table and
the brief of a worker of podcrates `v0.85.0` (COORD-157); `docs/decisions.md` has the ruling.


# T-589: live: the quick-tunnel service refuses the runner for an hour (HTTP 429, error 1015), and each later step fails

**Source:** `live` 45616517699 (`5809e66`), 45632652103 (`66b29ce`) and 45589043028's neighbours, 2026-11-25: the steps
"Quick tunnel through the WebSocket relay" and "Quick tunnel with UDP and TCP 7844 dropped".
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

A run of `live` provisions about twelve quick tunnels. Three runs of one day met the quota of the quick-tunnel
service: `quick-tunnel service returned HTTP 429 (Retry-After: 3600 s): error code: 1015`. podnet exits 75 as
documented; the step fails, and each later step is skipped. The run is red for a quota of another party.

## Premise

- Read: the job logs of the three runs. The 429 comes after 10 to 13 provisions in about 15 minutes.
- Not measured: the quota of the service for one address.

## Approach

1. `scripts/live-step.sh` is the shell of each step of the job `live` (`defaults.run.shell`). It runs the script of the
   step as `bash -e` does. A step that fails with `quick-tunnel service returned HTTP 429` in its output gives a
   `::warning::`, writes `LIVE_QUICK_REFUSED=1` to `$GITHUB_ENV`, and passes. Each other failure keeps its exit code.
2. The output goes to a file that `tail --pid` streams: a process that a step leaves in the background (a stand-in
   proxy) does not hold the step.
3. A green run has no `report` job: the quota does not open the issue "live: two runs in a row failed".

## Prove

```sh
python3 -I scripts/check-workflows.py
# the next live run with a 429: a warning, and the run is green
```

## Correction

2026-11-26: `live` 45733368792 (`585b722`) failed in the step "Autostart with a systemd user unit": the URL of the
unit answered, then `podnet autostart run` exited 75 and systemd restarted it, and `systemctl --user is-active` failed
with no cause in the step (the line of podnet is in the journal of the unit). The step now prints the last 30 lines of
the journal when the unit is not active, so the cause is in the output of the step, and a quota of the quick-tunnel
service there gives the warning of `scripts/live-step.sh`.

## Done

2026-11-25 (steps 1 to 3), the commit of this entry. A local run of `scripts/live-step.sh` over four scripts: a 429
line and exit 75 gives the warning, the variable and exit 0; another exit 3 stays 3; exit 0 stays 0; a script that
leaves `sleep 30` in the background ends in 1 s. `python3 -I scripts/check-workflows.py`: 0 faults. The proof on the
runner is the next `live` run with a 429.
2026-11-26, the proof on the runner: `live` 45738194994 on `ac0aa576` met the quota at 02:05:33 (`HTTP 429
(Retry-After: 3600 s): error code: 1015`). The steps "Quick tunnel, QUIC through WARP (TCP 443)", "Quick tunnel with
UDP and TCP 7844 dropped" and "Quick tunnel through the WebSocket relay" each gave the warning `the quick-tunnel service
refuses this runner for now (HTTP 429); the step is not measured (T-589)` and set `LIVE_QUICK_REFUSED: 1`; each later
step ran, and the run is green. The step "Autostart with a systemd user unit" passed in 11 s (before the quota); its
journal line stays for the next stop of the unit.

# T-590: The crash log and the harness name podcrates 0.79.0, and podnet takes v0.85.0

**Source:** a run of `podnet harness --data tests/data/harness` for T-398, 2026-11-26: the line of the conditions says
`podcrates 0.79.0`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`src/crashlog.rs` and `src/harness.rs` each held `PODCRATES = "0.79.0"`. The comment said that a test reads the tag of
`Cargo.toml`; no test did. Six takes later, the crash log and the conditions of each harness run named a wrong version
of the shared crates. A crash report or a measurement then points at the wrong code.

## Premise

- Read: `src/crashlog.rs`, `src/harness.rs`, `tests/harness_cli.rs` (a literal `0.79.0`), and `Cargo.toml` (`v0.85.0`).

## Approach

1. `build.rs` reads the tag of `pod-core` in `Cargo.toml` and sets `PODNET_PODCRATES` (`unknown` when there is none).
   `crashlog::PODCRATES` is `env!("PODNET_PODCRATES")`; `harness::PODCRATES` is the same constant.
2. The test `the_podcrates_version_is_the_tag_of_the_manifest` reads each `pod-*` dependency of the manifest: one tag,
   and it is `PODCRATES`. `tests/harness_cli.rs` reads the tag of the manifest in place of a literal.

## Prove

```sh
cargo test --lib -p podnet the_podcrates_version_is_the_tag_of_the_manifest
cargo test --test harness_cli
```

## Done

2026-11-26, the commit of this entry. The tests `the_podcrates_version_is_the_tag_of_the_manifest`,
`the_table_has_the_date_the_profiles_and_the_skips` and `the_text_names_the_conditions_and_the_benign_rate` pass; `podnet harness` prints
`podcrates 0.85.0`.

# T-591: podnet cover with no --name answers each request with the 404 page, also /

**Source:** a run of `podnet cover` for T-398, 2026-11-26: `curl -k https://localhost:PORT/` got `404 Not Found`.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`Cover::load` put the site into `pod_cover::Sites` only under `--name`. `Sites::respond` gives an unknown name, and no
name, the 404 page of the default site. With no `--name`, each name was unknown: the cover had no page, and a probe saw
a site with no `/`.

## Premise

- Read: `src/cover.rs` (`Cover::load`, `Cover::respond`) and `Sites::respond` of `pod-cover` at `v0.85.0`.
- Measured: `curl -k` of `/` with no `--name` got 404; with `--name localhost` it got the index.

## Approach

1. `Cover` keeps `every_name` (no `--name`). `Cover::respond` then asks the default site; else it asks `Sites`.
2. A table test loads the cover with and with no `--name`, and asks `/` for three names of the handshake.

## Decision

With no `--name`, the site answers each name and a handshake with no name, as the default server of a web server does.
With `--name`, only that name gets the site, as before. The help of `--name` says so.

## Prove

```sh
cargo test --lib -p podnet the_site_answers_the_names_of_its_flag
```

## Done

2026-11-26, the commit of this entry. The table test `the_site_answers_the_names_of_its_flag` (three names with no
`--name`, three with it) passes, and `tests/cover.rs` passes; `curl -k` of `/` with no `--name` gets the index.

# T-592: link p2p drops the answer of the peer when stdin ends first

**Source:** the lane case `local-tcp` of T-398, 2026-11-26: `echo ping | podnet link p2p --peer ECHO` printed the
path and no `echo:ping`, in each of three tries.
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet link p2p` copied stdin up and the stream down in one `select!`, and ended at the first copy that ended. At the end
of stdin the process ended before the peer answered: a request through `link p2p` lost its answer.

## Premise

- Read: `src/cli/cmd/link/p2p.rs`, and `pod_warp::nc::pipe_with` of podcrates `v0.85.0` (the pipe of `warp nc`).
- Measured: an echo peer on the loopback got `ping`, and stdout got no answer.

## Approach

1. Shared code first: the pipe is `pod_warp::nc::pipe_with` with `shutdown: true`. At the end of stdin the FIN goes to
   the peer, and the stream is read until the peer closes. A peer that closes first ends the command, also with stdin
   open.
2. A table test of `tests/link_p2p_cli.rs` pins both ends.

## Prove

```sh
cargo test --test link_p2p_cli link_p2p_carries_the_answer_of_the_peer_to_the_end
```

## Correction

2026-11-26: the test failed each time in its second case (the peer closes while stdin is open): the command printed
the answer and then waited for the end of stdin. A read of stdin blocks a thread of the runtime, and the drop of the
runtime at the end of `main` waits for each such thread (a hand test with `< <(sleep 12)` ended after 12 s). The first
pass of the test was a race. `main` now ends the runtime with `shutdown_timeout` of 1 s, for each command that reads
stdin. The test passes twice in a row (1.45 s); `--test signals`, `pod_signals`, `e2e` and `exit_stdio` pass.

## Done

2026-11-26, the commit of this entry. The test `link_p2p_carries_the_answer_of_the_peer_to_the_end` passes; with the old
`select!` it fails (no answer in 20 s).

# T-593: connect: the timeout line says "timed out after 0s" after a wait of 20 s

**Source:** GitHub #761 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A dial to an address that does not answer (192.0.2.1) ends after 20 s with `the connection to 192.0.2.1:443 timed out after 0s` in `connect`, `relay connect` and `send`. The line names a time limit that is not the real wait.

## Premise

- Read: `pod_ws::way::dial_error` (podcrates `v0.85.0`) makes a `DialError::Timeout` with `after: Duration::ZERO` for a failure that has no `DialError` as its cause; `egress::dial_failure` kept the `DialError` only at the hop of a proxy.

## Approach

1. `egress::dial_failure` keeps a `DialError::Timeout` as the cause at each hop, so `pod-ws` takes it as it is, with its limit.

## Prove

```sh
cargo test --lib -p podnet egress::
```

## Done

2026-11-26, the commit of this entry. The test `a_refusal_of_the_far_side_is_the_target_and_a_timeout_keeps_its_limit` (a timeout of 29 s stays the cause of the failure, with 29 s) passes; on the old code the cause was no `DialError`.

# T-594: connect: a target that fails behind a working proxy is reported as a proxy failure

**Source:** GitHub #762 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

A proxy that answers CONNECT with HTTP 502 (or a SOCKS5 reply of the far side: 4, 5) for a target that does not resolve or does not answer gives `could not reach the proxy` and `the proxy hop failed; check the proxy URL ...; --no-proxy goes around it`. The proxy answered; the target failed.

## Premise

- Read: `egress::failed` put each `DialError::ProxyRefused` at the hop `proxy`. `pod-proxy` gives a SOCKS5 refusal of the far side the status 502.

## Approach

1. A refusal with the status 502 or 504 is at the hop `connect` (the target); 403 and 407 stay at the hop `proxy`.

## Prove

```sh
cargo test --lib -p podnet egress::
```

## Done

2026-11-26, the commit of this entry. The test `a_refusal_of_the_far_side_is_the_target_and_a_timeout_keeps_its_limit` (502 and 504 give the hop `connect`, 403 and 407 the hop `proxy`) passes. By hand: `connect --proxy` to a stand-in that answers 502 prints `the connect hop failed` and exits 69.

# T-595: connect: a name that does not resolve exits 69, not 68 (no-host)

**Source:** GitHub #763 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`connect`, `relay connect`, `send` and `tunnel --transport relay` with a name under `.invalid` print `the dns hop failed` and exit 69. `podnet man exit-status` says that 68 (no-host) is for an address of a peer that does not resolve.

## Premise

- Read: `exit::code_of` takes the first `Faulted` of the chain (the `Unavailable` of the command); the hop is known (`hop::of`). `Faulted::source` skips its inner error, so the hop is read from the inner error.

## Approach

1. `exit::code_of`: a code 69 whose hop is `dns` is 68. A proxy host that does not resolve stays 69 (the hop `proxy`).
2. `tests/error_words.rs` and `scripts/lab-errors.sh` (the rows `connect-no-name` and `relay-no-name`) take 68.

## Prove

```sh
cargo test -p podnet-core --lib exit::
cargo test --test error_words
```

## Done

2026-11-26, the commit of this entry. The test `an_unavailable_service_whose_name_does_not_resolve_is_no_host` (dns 68; proxy, connect 69; a usage fault stays 64) passes; `connect --no-proxy wss://x.invalid/...` exits 68. A breaking change of the notes.

# T-596: net proxy: --switch tcp://NAME:PORT is refused as a "bad TCP listen address"

**Source:** GitHub #764 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`net proxy --switch tcp://localhost:9000` exits 64 with `bad TCP listen address`; the example of `docs/networks.md` (`--switch tcp://switch-host:7000`) cannot run. `--switch` dials; it does not listen.

## Premise

- Read: `src/cli/cmd/net/proxy.rs` parsed `--switch` with `ProxyListen::parse`, which takes an IP only.

## Approach

1. `switch_of` takes `tcp://HOST:PORT` with a name, resolves it with the resolver of podnet (a name that does not resolve exits 68), and keeps `unix:` and an IP as before.

## Prove

```sh
cargo test --bin podnet switch_tests
```

## Done

2026-11-26, the commit of this entry. The test `a_switch_takes_a_name_an_ip_or_a_path` passes (a name, an IP, a path; three bad forms exit 64).

# T-597: wait: the help says exit 69 at --timeout, the command exits 124

**Source:** GitHub #765 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`podnet wait --help` says `--timeout ... (else exit 69)`; the command exits 124, as `docs/background.md` and `podnet man exit-status` say.

## Premise

- Read: `src/cli/mod.rs`, the help of `--timeout` of `wait`.

## Approach

1. The help says `else exit 124, as timeout(1) exits`.

## Prove

```sh
cargo test --bin podnet the_llms_files_equal_the_model
```

## Done

2026-11-26, the commit of this entry. The help and `llms-full.txt` say 124; the test `the_llms_files_equal_the_model` passes.

# T-598: doctor: on bind-denied the TCP carrier rows give "Address family not supported", not the refused bind

**Source:** GitHub #766 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** blocked

## Problem

In the lab `bind-denied` with no IPv6, each TCP carrier row of `doctor` and `components` says `TCP sockets on the loopback: Address family not supported by protocol (os error 97)`. The cause is the refused bind of 127.0.0.1 (EACCES).

## Premise

- Read: `pin::available` and `mail::tcp_available` of `pod-link` (podcrates `v0.85.0`) give the error of the IPv6 try when the IPv4 bind fails.

## Approach

1. The fix is in `pod-link`: COORD-131 (PodUtils/podnet#776). podnet takes it with the next tag (T-137).

## Prove

```sh
podnet doctor   # in the lab bind-denied: the row names the refused bind
```

## Blocker

COORD-131 (PodUtils/podnet#776): a tag of podcrates with the fix of `pod-link`.

# T-599: link dial: a bad --peer value exits 78 and names "line 1"

**Source:** GitHub #767 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`link dial --peer AAAA` and a fingerprint with a bad checksum exit 78 with `--peer: line 1: ...`. A bad value of a flag is a usage error (64), and a flag has no line.

## Premise

- Read: `link dial` and `relay` parsed `--peer` with `AllowList::parse` of the joined values, the parser of a file.

## Approach

1. `AllowList::of_peers` parses each value alone; a bad value exits 64 with `--peer VALUE: WHY`. A file keeps its line and 78.

## Prove

```sh
cargo test --lib -p podnet identity::
```

## Done

2026-11-26, the commit of this entry. The test `a_bad_peer_flag_is_a_usage_error_with_no_line` passes; `link dial --peer AAAA` exits 64.

# T-600: net proxy: --switch to an address that does not answer waits 135 s, past --run-for, with no line

**Source:** GitHub #768 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`net proxy --switch tcp://192.0.2.1:9000 --run-for 10` waits 135 s (the limit of the kernel) with no line, then exits 69.

## Premise

- Read: `switch::attach` of `podnet-vnet` called `TcpStream::connect` with no time limit.

## Approach

1. The connect to a TCP switch has a limit of 20 s (`SWITCH_CONNECT`), as the other dials; the line names the switch and the limit. `attach_within` takes the limit, for a test. The attach moves to `switch/attach.rs` (the size limit).

## Prove

```sh
cargo test -p podnet-vnet --lib a_tcp_switch_that_does_not_answer_ends_at_the_limit
```

## Correction

2026-11-26: the job `restricted-host` of `ci` 45756419163 (`58a1c617`) refuses `bind(AF_INET)`, and the test failed at
the bind of its listener (EACCES). The test now starts with `pod_testkit::need_loopback!()`, as the other tests of the
file that bind TCP.

## Done

2026-11-26, the commit of this entry. The test `a_tcp_switch_that_does_not_answer_ends_at_the_limit` (a listener with a full queue, a limit of 0.8 s: one line in 1.7 s) passes.

# T-601: tunnel: the warning "DNS discovery of the edge failed" comes two times at one start

**Source:** GitHub #769 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

One start of a quick tunnel logs the same warning of about 1400 characters two times, 10 ms apart.

## Premise

- Read: `pod-cloudflare` runs the edge discovery for the plan (`tunnel/plan.rs`) and for the WARP targets (`tunnel/warp_targets.rs`); each run warns.

## Approach

1. `decisions::report` says an equal warning (one kind, one text) once in 60 s. Each other warning is said as before.

## Prove

```sh
cargo test -p podnet-core --lib decisions
```

## Done

2026-11-26, the commit of this entry. The test `an_equal_warning_is_said_once_in_its_quiet_time` passes.

# T-602: pod --serve: SIGINT or SIGTERM kills it with no clean stop, and the socket file stays

**Source:** GitHub #770 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`pod --serve unix:PATH` ends by SIGINT or SIGTERM with the status of the signal (130, 143), no stop line, and its socket file stays. `docs/cli.md` says that a server command stops cleanly with exit 0.

## Premise

- Read: `podnet-netns` `serve_front_door` had no handler of a stop signal.

## Approach

1. `serve_front_door` waits for SIGINT, SIGTERM and SIGHUP beside the door: one line `SIGNAL: stopping`, the unix socket removed, exit 0.

## Prove

```sh
cargo test --test pod_signals a_stop_signal_ends_pod_serve_cleanly_and_removes_its_socket
```

## Done

2026-11-26, the commit of this entry. The test `a_stop_signal_ends_pod_serve_cleanly_and_removes_its_socket` (SIGINT, SIGTERM and SIGHUP: exit 0, the line, no socket) passes.

# T-603: offline: the help says "a four-word pairing code", the code has eight words

**Source:** GitHub #771 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The help of `podnet offline` says a four-word single-use pairing code; `pair offer` prints eight words, as `docs/offline.md` says.

## Premise

- Read: `src/cli/mod.rs`, the help of `offline`.

## Approach

1. The help says an eight-word code.

## Prove

```sh
cargo test --bin podnet the_llms_files_equal_the_model
```

## Done

2026-11-26, the commit of this entry. The help and `llms-full.txt` say eight words; the test `the_llms_files_equal_the_model` passes.

# T-604: offline pair accept: it waits for the code on stdin with no prompt, and the help does not say so

**Source:** GitHub #772 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`pair accept --offer offer.txt --used ./used` (the example of the summary of `docs/offline.md`) waits for stdin with no prompt; its help does not say that the code comes from stdin.

## Premise

- Read: `src/cli/cmd/offline/pair.rs` read stdin to its end.

## Approach

1. On a terminal, the command asks `the eight words of the code:` and reads one line; else it reads stdin to its end.
2. The help says that the code comes from stdin. The summary of `docs/offline.md` writes the words to `code.txt` and reads them from it.

## Prove

```sh
cargo test --bin podnet the_llms_files_equal_the_model
```

## Done

2026-11-26, the commit of this entry. The help and `llms-full.txt` name stdin; the test `the_llms_files_equal_the_model` passes; the lane case `local-files` (`pair accept` with the code from a file) passes.

# T-605: expose --via switch: the example of docs/forward.md exits 64 "--switch needs --address"

**Source:** GitHub #773 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`expose unix:/run/app.sock --via switch --as 10.66.0.2:8080 -- --switch unix:...` (the example of `docs/forward.md`) exits 64: `--switch needs --address`.

## Premise

- Read: `src/cli/cmd/expose.rs` made `net proxy --forward VIRT=REAL` and gave the stack no address.

## Approach

1. With a `--switch` after `--` and no `--address`, `expose` gives `--address` the IP of `--as`. `docs/forward.md` says so.

## Prove

```sh
cargo test --bin podnet expose
```

## Done

2026-11-26, the commit of this entry. The test `expose_via_a_switch_gives_the_stack_the_address_of_as` passes.

# T-606: forward: a refused bind of port 18000 tells the user to "give a port above 1023"

**Source:** GitHub #774 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

In the lab `bind-denied`, `forward -L 18000=...` and the other TCP listeners say `this host refuses the bind (a port below 1024 needs privilege, ...); give a port above 1023, or a unix socket`. The port is above 1023.

## Premise

- Read: `net::bind_step` of `podnet-core` gave one text for each refused TCP bind.

## Approach

1. `bind_fault_of` reads the port of the address that the line names: above 1023, the host refuses the TCP listener (a unix socket); below 1024, a port above 1023; no port, the text as before.

## Prove

```sh
cargo test -p podnet-core --lib net::
```

## Done

2026-11-26, the commit of this entry. The test `a_refused_tcp_bind_names_the_step_of_its_port` passes.

# T-607: init check: a missing file is reported as "the configuration is not schema 1"

**Source:** GitHub #775 (`[qa 0.4.0-beta.20]`, black-box QA pass of 2026-11-26).
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`init check /nonexistent.yml` says `the configuration is not schema 1: /nonexistent.yml: No such file or directory` and exits 78.

## Premise

- Read: `NodeConfig::load` of `src/initcfg.rs` gave a read error as `InitError::Schema`.

## Approach

1. `InitError::Read`: the path and the error; a missing file exits 66 (no-input), another read error 78.

## Prove

```sh
cargo test --lib -p podnet initcfg
```

## Done

2026-11-26, the commit of this entry. The test `a_missing_file_is_no_input_and_names_its_path` passes; `init check /nonexistent.yml` exits 66.

# T-608: Publish `v0.4.0-beta.20`: the carriers of the feature `release` and the fixes of the QA pass of 2026-11-26

**Source:** `docs/maintenance.md`, Release; `CLAUDE.md`, Releases (2026-11-09): cut the next prerelease when a batch of
fixes is green. T-428 (#620: the release binaries had no network link carrier) and the black-box QA pass of 2026-11-26
(GitHub #761 to #775).
**Category:** release
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

`v0.4.0-beta.19` has no network link carrier in its release binaries (#620), and none of the fixes of T-589 to T-607.

## Premise

- The steps are `docs/maintenance.md`, "Release". Step 0 (the black-box QA pass on the static release build of
  `4a51e702`) found 15 issues (#761 to #775). Each is an entry (T-593 to T-607). Each P1 and P2 entry is done; T-598
  (P3) waits for `pod-link` (COORD-131, #776). The notes name it.
- The notes have `## Breaking`: exit 68 for a name that does not resolve, 64 for a bad `--peer`, 66 for a missing
  node configuration, the clean stop of `pod --serve`, `cover` with no `--name`, `link p2p` to the end of the answer.

## Approach

1. Set the version `0.4.0-beta.20` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`), write the llms files again.
2. Write `docs/releases/v0.4.0-beta.20.md`, with `## Breaking`.
3. Push; wait until `ci` and `live` pass on that commit. Dispatch `gate.yml` on that commit and wait until it passes.
4. Dispatch the dry run of `release.yml` (`publish` empty), then `publish=v0.4.0-beta.20`; check `release-proofs.yml`
   and the job `image`; hand off the files and the digests in a COORD issue `(to all)`.

## Prove

```sh
# release-proofs.yml on the tag: each file OK, Verified OK
```

## Done

2026-11-26, the tag `v0.4.0-beta.20` at `be2c0df`. ci 45764673829 and live 45764673855 passed on `be2c0df`; gate
45756432875 passed on `58a1c617` (the release commit: a test and the allow list of the secret scan changed after it);
the dry run 45768240934 passed. The release run 45772282526: each of the eight targets, `gate / lane`, `publish`,
`proofs` and `image` passed; `release-proofs.yml` 45777283954 passed. The x86-64 file checks against SHA256SUMS, prints
`podnet 0.4.0-beta.20 (be2c0df...)`, lists the carriers of the feature `release`, and exits 68 for a name under
`.invalid`. COORD-132 (PodUtils/podnet#777) hands off the files and the digests.
