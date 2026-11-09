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
  the supervisor (`crates/podnet-netns/src/seccomp.rs`). Then `setsockopt` goes to the kernel
  AF_UNIX socket and gives EOPNOTSUPP for `TCP_NODELAY` and `TCP_KEEPIDLE`. `getsockname` and
  `getpeername` go to the kernel and give the AF_UNIX address.
- The evidence log shows `ss_family: 1` for the local and the peer address
  (`docs/reports/field-report-2026-11-05-evidence.log`).
- Read: `getpeername` of a tracked stream gives the local bound address, not the peer.
- The only test of the addresses uses a UDP socket (`tests/pod_seccomp.rs`).
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
`tests/pod_seccomp_tcp.rs` (`tests/pod_seccomp.rs` is in the size allowlist): it passes. Plant: no
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

- Read: the descriptor table and the `close` branch in `crates/podnet-netns/src/seccomp.rs`.
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
The tests are in a new file, `tests/pod_seccomp_fds.rs` (`tests/pod_seccomp.rs` is in the size
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
  already (`crates/podnet-netns/src/seccomp.rs`).
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

- The test is in `tests/pod_seccomp_handoff.rs`, not in `tests/pod_seccomp.rs`.
  `tests/pod_seccomp.rs` is in `scripts/size-allow.txt` and cannot get larger. The Prove names
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

- Read: `handle_socket` does not read the protocol argument (`crates/podnet-netns/src/seccomp.rs`).
- `recvmmsg` is not in the list of intercepted calls. The ruling of 2026-11-04 names `recvfrom`
  and `recvmsg` (`docs/decisions.md`, pod seccomp UDP).
- `wrap_udp_name` cuts a name over 255 bytes (`crates/podnet-netns/src/seccomp.rs`).

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

2026-11-06. The three tests are in a new file, `tests/pod_seccomp_dgram.rs`: `tests/pod_seccomp.rs`
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
  (`supervise_seccomp` and `handle_connect` in `crates/podnet-netns/src/seccomp.rs`).
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

- 2026-11-06: `tests/pod_seccomp.rs` is at its size limit (`scripts/size-allow.txt`). The test is in
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
`tests/commands.rs` (`tests/cli.rs` cannot grow). Control: before, `warp tun --session-max-age 60`
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
- Read: `run_dir()` depends on the state directory (`src/instances.rs`).
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
`src/instances/text.rs`. The test of `--list` is `warp_forward_list_prints_the_forwards_of_ps`.

## Done

2026-11-06, commit bbfe527 (`warp: the pins of --resolve come first, ps prints each forward, warp
forward --list (T-073)`). `pinned(host)` in `src/resolve.rs` gives the pins of `--resolve` and
`PODNET_RESOLVE`. `resolve_all` and `resolve` (`src/warp/proxy/names.rs`) return the pins before
a DNS query through the tunnel: `warp forward`, `warp proxy` and `warp nc`. `ps` prints each
forward of an array `forward` as `LISTEN -> TO` (`src/instances/text.rs`). `warp forward --list`
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
- The switch drops a datagram of another user with `SO_PASSCRED` (`src/vnet/switch.rs`).

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

- `src/ingress.rs` matches the rule with the `regex` crate.
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
- `src/doctor.rs` and `src/report.rs` write the vocabulary of the other doctors.

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
The test is in `tests/commands.rs` (`tests/cli.rs` cannot grow) and passes. The `## Breaking` list
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

- `src/yaml.rs` keeps no positions.
- `src/config.rs` writes the errors with the rule and the key.

## Approach

1. Keep the line and the column of each node in `src/yaml.rs`.
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
names `ingress rule N (line L)`. A full map of positions in `src/yaml.rs` is not made: the errors
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
**Status:** open

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
**Status:** open

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
- Read: the test `seccomp_closed_sockets_free_their_entries` (`tests/pod_seccomp_fds.rs:260`) makes
  5000 sockets that never send. It cannot see the leak.
- Not measured: the count of 3 per socket. No build ran in this triage.
- Existing tests have a SOCKS5 UDP-in-stream stand-in (`tests/pod_seccomp.rs:435`).

## Approach

1. Move `ensure_udp_relay`, `relay_datagram` and `UdpRelay` out of `seccomp.rs` into a new file
   (the file is at its cap and cannot grow).
2. Give each `Sock::Datagram` a stop signal (a `tokio::sync::watch` sender or a `CancellationToken`).
   Select on it in both reader loops. Drop of the entry sends it.
3. Send the stop from `handle_close`, `sweep` and the shutdown near `drain_relays` (`seccomp.rs:851`).
4. Correct `crates/podnet-netns/README.md:81-83` so that it separates `read` and `write` on TCP from
   the UDP path (the write part belongs to T-143).

## Prove

```sh
cargo test --test pod_seccomp_dgram -- seccomp_udp_close_frees_the_relay_descriptors seccomp_udp_exit_frees_the_relay_descriptors seccomp_udp_stream_control_ends_on_close
```

The first test opens 100 UDP sockets, sends one datagram on each, closes each, and counts
`/proc/PID/fd` of the supervisor (not `/proc/self/fd`). It expects the count before the test. The
second test uses a child that exits (the `sweep` path) or `close_range`. The third test checks that the
control connection of the stream form ends.

# T-143: seccomp: a datagram written with `write(2)` leaves the pod (GitHub #347)

**Source:** GitHub #347 (https://github.com/PodUtils/podnet/issues/347) (defect 1)
**Category:** defect
**Milestone:** M5
**Priority:** P1
**Effort:** M
**Status:** open

## Problem

`write(2)` on an emulated datagram socket returns success, and the datagram never leaves the pod. The
reply never comes. `sendto`, `send`, `sendmsg` and `sendmmsg` work. Programs write datagrams with
`write` after `connect` (C code, Python `os.write`, some Go paths). The README says that `write` works
on the pair (`crates/podnet-netns/README.md:81-83`). That is true for TCP only.

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

# T-144: seccomp: `bind` on a datagram socket is recorded, and `getsockname` reports it (GitHub #348)

**Source:** GitHub #348 (https://github.com/PodUtils/podnet/issues/348)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

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

# T-145: fake DNS: the pool refuses at its end, and names match without case (GitHub #349)

**Source:** GitHub #349 (https://github.com/PodUtils/podnet/issues/349)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

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
3. Update `README.md:195-201`: the pool bound, case-insensitivity, SERVFAIL at the end.

## Prove

```sh
cargo test -p podnet-netns --lib -- fake_pool_refuses_past_its_end fake_names_ignore_case fake_pin_does_not_evict_a_live_name servfail_answers_a_name_that_does_not_fit
```

# T-146: seccomp: `connect(AF_UNSPEC)` dissolves the association of a datagram socket (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 1)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

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

# T-147: seccomp: a connected datagram socket takes datagrams from its peer only (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 2)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

A connected UDP socket reads datagrams from any source that the relay returns. The kernel delivers only
the datagrams of the peer.

## Premise

- Read: `Inbox::deliver` (`inbox.rs:79`) sends into the pair with no check of the source. Both relay
  reader tasks call it (`seccomp.rs:1226`, `:1246`). `Inbox::new` has no `dest`.
- Read: the ruling of 2026-11-04 (`docs/decisions.md:36`): a read of a connected socket stays direct,
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

# T-148: seccomp: datagram and ICMP sockets keep their IP-level options (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 3)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

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

# T-149: seccomp: decide and document the ICMP error and PMTU limit of the UDP relay (GitHub #350)

**Source:** GitHub #350 (https://github.com/PodUtils/podnet/issues/350) (item 4)
**Category:** research
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

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

# T-154: seccomp: fuzz targets for the notification handlers (GitHub #353)

**Source:** GitHub #353 (https://github.com/PodUtils/podnet/issues/353) (part 1)
**Category:** chore
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-155: seccomp: a differential check of the socket calls against the host kernel (GitHub #353)

**Source:** GitHub #353 (https://github.com/PodUtils/podnet/issues/353) (part 2)
**Category:** chore
**Milestone:** M5
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

No test runs one scripted syscall sequence natively and in the pod and compares the errno, the returned
addresses and the cmsg. A rewrite of a handler can pass the hand-written C tests and still differ.

## Premise

- Read: the end-to-end tests are in `tests/pod_seccomp*.rs` (for example `:1588`, `:1657`, `:1717`,
  `:1810`). `tests/pod_seccomp.rs` is at its cap (2021).
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
  `tests/docs.rs:74-85` reads eight documents, not these.
- Read: the live plan is the record. T-040 (`TODO/split.md:751`) still says that `seccomp.rs` has 1,900
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

2026-11-09, commit 9738eb9. Each research document and `docs/reports/aerrand-agent.md` has the banner `Reading of the tree at COMMIT`. T-040 has a Correction (the counts, T-063 to T-067 done). No checker (Decision). `scripts/agent/check.sh` exits 0. CI: at the merge.

# T-160: tunnel verify: a name that this host cannot resolve yet keeps the tunnel up (GitHub #357)

**Source:** GitHub #357 (https://github.com/PodUtils/podnet/issues/357)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

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
- Read: the three front ends each shut down and return the error (`src/cli/cmd/run.rs:17-176`).
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

- `src/cli/opts.rs:58-60` declares it (`global = true`); `src/cli/opts.rs:173` sets `config.verify`.
- `src/cli/cmd/run.rs:23,69` read `config.verify` and gate `verify_accepting`; `src/tunnel.rs:1230`
  runs the fetch. `src/net.rs` takes `no_tls_verify` from the ingress rule. The two flags share no state.
- `grep -n -e '--no-verify' README.md docs/*.md` finds nothing. `wants_verify` (`src/tunnel.rs:1208`) has no caller.
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

2026-11-09, commit be7b624. The README has a `--no-verify` bullet beside `--no-tls-verify`. The help line says that origin TLS checks stay on. The name stays. `scripts/agent/check.sh` exits 0. CI: at the merge.

# T-169: `tunnel`: try the MASQUE door of the host's address family first, and take `--masque-endpoint` (GitHub #366)

**Source:** GitHub #366 (https://github.com/PodUtils/podnet/issues/366)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

On a host with no IPv4 route, `tunnel --transport warp` tries the IPv4 door first (device record order),
fails with `Network unreachable`, then the IPv6 door, and can reach the WebSocket relay first. `warp`
has `--masque-endpoint`; `tunnel` has none.

## Premise

- `warp_targets` builds the client with no `with_masque` (`src/tunnel.rs:823-834`). Neither
  `TunnelOpts` (`src/cli/opts.rs:20-105`) nor `TunnelArgs` has the flag. `with_masque` exists
  (`src/warp/mod.rs:286`); `warp` uses it (`src/cli/cmd/warp/client.rs:67-74`).
- `open_on` (`src/warp/mod.rs:823-848`) uses `endpoint_order` (`:101-122`): the record order, IPv4 first.
  `has_default_route(true)` (v6) only decides whether the IPv6 door is included.
- The log lines of the issue are from beta.6 on one host. The silent-drop timing is read, not measured.

## Approach

1. Fix: in the `endpoint == None` branch, with no IPv4 default route (`has_default_route(false)`) and no
   won door, put the IPv6 door first. Dual-stack keeps the order. Log the family of a failed door.
2. Flag: `--masque-endpoint IP:PORT` in `TunnelOpts`, through `Config`, to `warp_targets`
   (as `--warp-identity`, `src/cli/opts.rs:172`, `src/tunnel.rs:157,262`).

## Decision

The lead, 2026-11-09: item 1 (the family order) is a fix and lands now. Item 2 (the flag `--masque-endpoint`) is a feature in code that moves to `pod-warp` and `pod-cloudflare`: send it as a COORD issue (to podcrates) and take it with the crates (T-050, T-052). The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet endpoint_order_puts_v6_first_without_a_v4_route
cargo test --test cli -- tunnel_takes_masque_endpoint
```

# T-170: Quick-tunnel provisioning does not retry a DNS failure that cannot recover (GitHub #367)

**Source:** GitHub #367 (https://github.com/PodUtils/podnet/issues/367)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

`provision` retries each transport error up to `PROVISION_ATTEMPTS = 5` (2, 4, 8, 16 s). Only `Hop::Proxy`,
`Hop::Tls` and `Hop::Network` end the loop; `Hop::Dns` falls into the retry arm. With no resolver, five
attempts repeat the same failure for about 30 s.

## Premise

- `src/quicktunnel.rs:308-310` (budget and attempts), `:341-353` (the arms; `Hop::Dns` goes to
  `ProvisionError::Http(_)` at `:353`).
- `crate::resolve::answering_step(host)` (`src/resolve.rs:150`) names the step that answers now.
- `src/quicktunnel.rs` is Cloudflare adapter code (pod-cloudflare, T-029/T-052): a fix is allowed.
- Not measured: the 30 s run of the issue.

## Approach

1. Retry a `Hop::Dns` failure once. Retry again only when `answering_step` of the API host changed.
2. End with the `Hop::Dns` hint (`src/hop.rs:45`).

## Prove

```sh
cargo test -p podnet quicktunnel::tests::a_dns_failure_is_retried_once
```

# T-171: The verify loop names the remedy when the public name does not resolve (GitHub #367)

**Source:** GitHub #367 (https://github.com/PodUtils/podnet/issues/367)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

`verify_fetch` retries 180 s (`VERIFY_BUDGET`, `src/tunnel.rs:1315`). On a host with no resolver the
error text repeats `system resolver: no answer ... Network unreachable`. The final error
(`src/tunnel.rs:1307-1310`) names no remedy: `--resolve HOST=IP`, or `--no-verify`.

## Premise

- The loop keeps `last = "waiting for DNS: ..."` (`src/tunnel.rs:1276-1279`). `Verified.dns` records the
  step on success only. `Hop::Dns` has a hint (`src/hop.rs:45`).
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

- `src/warp/mod.rs:942-968` (owner and mode checks, bare `bail!`); readers: `load` (`:381`) and
  `src/warp/export.rs:68`. `register_or_load` (`src/warp/mod.rs:459-478`) refuses and does not write over the file.
- `src/state.rs:67-118` has the model. `src/manual/facts.rs:126` lists `PODNET_FOREIGN_HOME`.
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
**Status:** blocked

## Problem

`podnet doctor --agent` checks one TLS handshake for each row. Three rows name the host of the index, and the tool gets its files from another host: `pip` (`pypi.org`, files from `files.pythonhosted.org`), `cargo` (`index.crates.io`, files from `static.crates.io`) and `docker pull` (`registry-1.docker.io`, blobs by a 307 to another host, `production.cloudfront.docker.com` at the time of the issue). A host that reaches the index and not the files shows `ok`. `npm`, `go modules` and `github.com` use one host.

## Premise

- `Task` has one `host` and one `port`, and `Check::Tls` is one handshake (`src/agent.rs:25-39`). The three rows are at `src/agent.rs:55-59`, `:67-71` and `:79-83`. The table prints one target (`src/agent.rs:491-506`).
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

## Blocker

The session of `PodUtils/podcrates` (the crate `pod-services`): the file host of pip is a file of that crate. Send a COORD issue (to podcrates). The entry is unblocked when the file exists.

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

- `--relay` and `--transport` are `global = true` in `TunnelOpts` (`src/cli/opts.rs:38-48`). `TunnelOpts` is flattened only into `tunnel` and `share` (`src/cli/cmd/tunnel.rs:17`, `src/cli/cmd/share.rs:22,62,114`). `DoctorArgs` has no `--relay` (`src/cli/cmd/host.rs:14-33`). A global flag does not reach a sibling command, so `doctor --relay X` most likely fails in clap. Not run (no build).
- `targets()` has no argument (`src/doctor.rs:111`). `doctor()` calls `Options::default()` with three fields set (`src/cli/cmd/host.rs:248-253`).
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

A client that connects and sends nothing, or one byte of a SOCKS5 greeting, holds its task and socket until the process ends. Each connection is unauthenticated. The accept loops have no cap. The WARP door has the fix (`REQUEST_LIMIT`, 30 s, `src/warp/proxy.rs:588`). `CLIENT_TIMEOUT` (300 s) wraps only the copy.

## Premise

- `src/vnet/proxy.rs:84-96`: `fill_buf`, `socks5_read_request` and `http_read_connect` have no timeout. The copy is wrapped at `:118-125`. Accept loops: `:269-294`.
- `crates/podnet-netns/src/tailscale.rs:377,383,387`: same reads. The dial and the copy have a timeout (`:386`, `:406`).
- `crates/podnet-netns/src/frontdoor.rs:79,121`: same reads in the door of `pod --serve` (not in the issue).
- `src/warp/proxy.rs:628-630,657-661,703-706`: the precedent.
- Not run.

## Approach

1. One constant `REQUEST_LIMIT` (30 s) in `podnet-netns` (`socks.rs`). Wrap the first byte and the request read in each of the three doors. The warp door uses the same constant.
2. On expiry send a SOCKS5 reply `0x04` or an HTTP `504`, then close (`fail_with`, `src/vnet/proxy.rs:161-180`).
3. Fix the comment of `CLIENT_TIMEOUT`: it bounds the copy only.
4. Count live client tasks (as `SocketCount::live`, `src/vnet/stack.rs:642-648`). A cap is a later choice.

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

`bind_unix` of the tailnet door removes the path and binds (`crates/podnet-netns/src/tailscale.rs:354-357`). It deletes a regular file and takes over a live socket. The ruling "Sockets" (`docs/decisions.md:22`): a live socket is 69, a file that is not a socket is 64, a stale socket is removed. The default path is `unix:/tmp/podnet-socks.sock` (`src/cli/cmd/net/mod.rs:68`).

## Premise

- Five callers use `claim_socket_path` (`src/warp/proxy.rs:87`, `src/vnet/switch.rs:435`, `src/vnet/proxy.rs:315`, `src/cli/cmd/connect.rs:226`, `src/cli/cmd/relay.rs:192`). The function is in the root crate (`src/net.rs:32-57`): `podnet-netns` cannot call it.
- Two more listeners do `remove_file` then `bind_private`: `crates/podnet-netns/src/frontdoor.rs:55` and `crates/podnet-netns/src/seccomp.rs:1368`.
- `src/net.rs` has no test of `claim_socket_path` (tests start at `src/net.rs:347`).
- `net_tailscale` opens the LocalAPI socket before the bind (`src/cli/cmd/net/shim.rs:57-63`).

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

`RealTarget::parse` takes only a `SocketAddr` (`src/cli/cmd/net/proxy.rs`, before this entry) and `localhost:3000` gives `bad real address`. The help and `docs/virtual-network.md:119-120` say `host:port`. The dial is a bare `TcpStream::connect` (same file): the proxy of the process, `NO_PROXY` and the resolver chain do not apply, and the warn has a bare `io::Error` (`src/vnet/proxy.rs:210`). Measured by the issue: a dead `--proxy` changed nothing.

## Premise

- `connect_origin` (`src/net.rs:89-114`) resolves names and `localhost`. `egress::dial` (`src/egress.rs:178-196`) uses the proxy and `NO_PROXY`. `split_host_port` is private (`src/net.rs:80-84`).
- `rules()` sends the loopback through the proxy (`loopback_direct: false`). A forward to `127.0.0.1` would then use the proxy. The Decision rules it: the loopback stays direct.
- The `F: Fn() -> Fut` shape of `forward` stays (`src/vnet/proxy.rs:185-189`).

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

`forward` binds inside a spawned task and returns a `JoinHandle` (`src/vnet/proxy.rs:185-203`). `net_proxy` prints `forward VIRT -> REAL` at once (`src/cli/cmd/net/proxy.rs:116`). An address outside the subnet, a duplicate `--forward`, an ACL refusal or a collision with a shim bind give the line, a log warn and exit 0. `--import` binds before it prints (`src/vnet/switch.rs:755-759`).

## Premise

- Refusals: `src/vnet/stack.rs:815-832` (`AddrNotAvailable`, `PermissionDenied`, `AddrInUse`).
- `spawn_forwards` cannot return a result (`src/vnet/proxy.rs:237-244`).
- The `--map` entry outside the subnet fails only at the first request (note of the issue: not verified).

## Approach

1. Bind first (async `forward`, or a `oneshot`). Return the result to `net_proxy`.
2. Map `AddrNotAvailable` to `Fault::Usage`, `AddrInUse` and `PermissionDenied` to `Fault::Unavailable`. Print the line after the bind.
3. Reject a repeated `VIRT` at parse time with its name.
4. Keep the per-connection warn at `src/vnet/proxy.rs:210-212`.

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

`Table::forward` drops a packet when the queue of the peer is full and ignores the error (`let _ = peer.try_send(...)`, `src/vnet/switch.rs:164,171`). The queue is 1024 packets (`:30`, `:465`). No counter, no log, no accessor. The `spoofed` counter (`:153`) has no accessor. The rule of the project: a drop is counted (`src/vnet/stack.rs:171-175`). The fault injector counts are not readable either (`:54-111`).

## Premise

- Read only. No run. `src/vnet/switch.rs` has 711 lines.

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

2026-11-09, commit da010b5. Table::deliver counts sent and dropped for each peer; Switch::counts (src/vnet/switch/counts.rs) gives them with the spoofed count; the stop logs the totals; FaultInjector logs its counts on drop. Test: vnet::switch::tests::a_full_queue_counts_its_drops. check.sh: exit 0. CI: at the merge.

# T-190: A stack over a switch link has bounded queues (GitHub #388, part 2)

**Source:** GitHub #388 (https://github.com/PodUtils/podnet/issues/388)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

`NetStack::over_link` builds `LinkDevice::tunnel(config.mtu)`: both queues are unbounded and the stack does not hold when full (`src/vnet/stack.rs:1436-1441`, `src/vnet/device.rs:304-313`). A slow pump grows the heap. The WARP stack uses `tunnel_bounded` and `with_inbound_limit` (`src/warp/ip.rs:454-455`, 512 and 4096 packets).

## Premise

- `over_link` has one production caller (`src/cli/cmd/net/proxy.rs:42`) and the switch tests (`src/vnet/switch/tests.rs`).
- `src/vnet/stack.rs` has 2103 lines in `scripts/size-allow.txt` and can only shrink. Two new `StackConfig` fields grow it.

## Approach

1. Add `LinkDevice::link(mtu)` in `src/vnet/device.rs` (639 lines): a bounded tunnel with the limits as named constants. In `stack.rs` replace the name `tunnel` by `link` (same line count). No `StackConfig` field now; fields wait for T-034.
2. Check that `device_stats()` shows the drops.
3. Existing switch tests, including the 64 KiB transfer with faults (`docs/virtual-network.md`), pass.

## Prove

```sh
cargo test -p podnet --lib vnet::
python3 scripts/check-size.py
```

# T-191: The listener table of `net doctor` names each command that listens (GitHub #389, part 1)

**Source:** GitHub #389 (https://github.com/PodUtils/podnet/issues/389)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

`NEEDS` (`src/vnet/doctor.rs:91-96`) lists `warp proxy`, `connect -L` and `net proxy` for `listen(AF_INET)`, and `warp proxy` and `connect -L` for `listen(AF_UNIX)`. Missing: `net switch` (`src/vnet/switch.rs:413-415`), `net tailscale` (`tailscale.rs:321-322`) and `relay connect --listen` (`src/cli/cmd/relay.rs:177,190-192`). `net proxy`, `net switch` and `net tailscale` default to a unix path (`src/cli/cmd/net/mod.rs:65,138,196`). `docs/target-environment.md:114` repeats the short list.

## Premise

- Printed at `src/cli/cmd/net/mod.rs:257-258`. `tests/cli.rs` has `doctor_tells_listen_from_bind`.
- `README.md:903-905` uses `net switch --listen tcp://` as its example.

## Approach

1. Rewrite `NEEDS` with each command and the default form. Fix `docs/target-environment.md:114`.
2. A test reads the `--listen` flags of the command tree and fails when a command is not in `NEEDS` (a generated table is a later step).

## Prove

```sh
cargo test -p podnet --lib vnet::doctor::
cargo test -p podnet --test cli doctor_tells_listen_from_bind
```

# T-192: A refused TCP bind exits 69 in each listener command (GitHub #389, part 2; GitHub #385 step 5)

**Source:** GitHub #389 (https://github.com/PodUtils/podnet/issues/389)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

`src/exit.rs:89-91` promises 69 for "a service or a network path is not available". A TCP bind that fails gives exit 1: `src/vnet/switch.rs:413-415`, `crates/podnet-netns/src/tailscale.rs:320-321`, `src/cli/cmd/relay.rs:190`, and also `src/vnet/proxy.rs:262-264` and `src/warp/proxy.rs:71-73` (not named in the issue). `exit.rs:152` reads only `Faulted`.

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

# T-193: `net proxy --map` fills the gateway resolver and the shim table (GitHub #390)

**Source:** GitHub #390 (https://github.com/PodUtils/podnet/issues/390)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

`--map NAME=ADDR` builds one `HashMap` for the door only (`src/cli/cmd/net/proxy.rs:91-103`). The stack gets `dns: None` (`src/vnet/stack.rs:119`), so nothing listens on `10.66.0.1:53` (`:732-742`, `:1050-1053`). `net shim` has an empty `hosts` (`src/cli/cmd/net/shim.rs:21-25`, `src/vnet/shim.rs:53,82-90`). `docs/virtual-network.md:208-210` and `net addresses` advertise the resolver.

## Premise

- `vnet::dns::Resolver` exists and is tested: `from_lines` (`src/vnet/dns.rs:57`), `hosts_pairs` (`:138`), fields `upstream` and `catch_all` (`:26,29`) that nothing sets.
- `src/vnet/stack.rs` allowlist (2103 lines): no new test in that file before T-034. A test can use the public `StackConfig.dns` from `src/vnet/dns.rs` or `tests/`.

## Approach

1. Build one `Resolver` from `--map`. Pass it as `StackConfig.dns` (switch path) and to the door through `hosts_pairs()`.
2. `net shim --map NAME=ADDR` (repeatable) fills `ShimOptions::hosts`.
3. `--map-file PATH` with `Resolver::from_lines`. Validate each address against the subnet at parse time.
4. Update the help and `docs/virtual-network.md:208-210`. The use of `upstream` and `catch_all` is a separate ruling: do not wire them here.

## Prove

```sh
cargo test -p podnet --lib vnet::dns::
cargo test -p podnet --test vnet
```

A test answers a DNS query for a mapped name at the gateway address.

# T-194: A relay `close` aborts the session task of the node (GitHub #391)

**Source:** GitHub #391 (https://github.com/PodUtils/podnet/issues/391)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The `close` control message removes the entry of the session and drops its `AbortHandle` (`src/relay_reverse.rs:448-452`). A dropped handle does not stop the task. If the task waits in `write_all` to a target that does not read (`:645-653`), it stays with its socket and its queued frames. The binary-frame arm aborts (`:473-477`).

## Premise

- The comment at `:523-524` says that dropping the sender ends the task. It does not, for a blocked write.
- The same removal at `:573-575` (`OutKind::End`) is safe: the task has returned.
- The test `a_local_side_that_does_not_read_stops_no_other_session` (`tests/relay_reverse.rs:483`) reaches only the binary-frame arm. The fake relay sends `close` at `:179`.
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

2026-11-09, commit 699d002 (`T-194: a relay close aborts the session task`). The `close` arm calls `end_session` (`src/relay_reverse/session.rs`): it removes the entry and aborts the task. The binary-frame arm and the `End` arm use the same helper. The session code of the node moved to `src/relay_reverse/session.rs`; `src/relay_reverse.rs` has 973 lines (was 1139). Test: `a_relay_close_ends_a_session_whose_target_does_not_read` (`tests/relay_reverse.rs`, a unix target with a fixed buffer): it fails with no abort (the target reads 3145728 of 3145728 bytes), and passes with it. `scripts/agent/check.sh` exits 0. CI: at the merge.

# T-195: `relay stop` checks the pair before it stops the local node (GitHub #392)

**Source:** GitHub #392 (https://github.com/PodUtils/podnet/issues/392)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`relay stop NAME` stops each local instance of the pair, then calls the relay (`src/cli/cmd/relay.rs:231-255`). `reverse::stop` refuses an empty stop token (`src/relay_reverse.rs:143-145`). The command then prints one line per stopped instance and exits 69. The pair file stays and the relay keeps the pair for up to 72 h. A pair with no stop token is a valid state (`src/relay_reverse.rs:55,111`, fixture at `tests/relay_reverse.rs:185-194`).

## Premise

- The local-first order has a good reason (`src/cli/cmd/relay.rs:240-241`): a stopped node does not connect again.
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

`session` ends when the local read returns 0 (`src/relay_reverse.rs:632`, select at `:654-657`) and sends `close` at once (`:659`). The operator side keeps the session open after its input ends (`EOF_IDLE`, `:535`, `:623`). A target that shuts down its write side and still reads loses the rest of the input of the operator. The operator exits 0 (a close with code 1000 is success, `:655`). The forward leg has the right shape (`src/relay.rs:263-297`).

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

2026-11-09, commit 0682f98 (`T-196: the relay node waits after a local end of stream`). When the local read returns 0, the session sends no `close` and logs `the target closed its write side`. The write side goes on until the relay closes the id (abort, T-194), a write to the target fails, or no frame comes for 2 s (`HALF_CLOSE_IDLE`, the Decision; `src/relay_reverse/session.rs`). The queued frames go to the target first. Test: `a_half_closing_target_gets_the_whole_input` (`tests/relay_reverse.rs`): it fails before the fix (the target reads 0 of 300000 bytes), and passes after it. `cargo test --workspace` passes; the other checks of `scripts/agent/check.sh` exit 0. CI: at the merge.

# T-197: The pair file is always mode 0600, and `load` checks the file (GitHub #394)

**Source:** GitHub #394 (https://github.com/PodUtils/podnet/issues/394)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`save` opens the pair file with `.mode(0o600)` (`src/relay_reverse.rs:149-154`). The kernel applies the mode only to a new file. An old file of mode 0644 keeps it, and `relay pair` prints `mode 0600` (`src/cli/cmd/relay.rs:105`). `save` follows a symlink. `load` uses `read_to_string` (`:172`) with no check of type, owner, mode or size. The file holds three tokens. The token cache has all four checks and a rename write (`src/relay_pool.rs:163-195`).

## Premise

- `state::replace_file(path, bytes, mode)` writes through a rename, sets the mode and replaces a symlink (`src/state.rs:344-363`).
- `read_private` returns `Option` and logs the reason: `load` needs an error with a `Fault::Config` class (`src/relay_reverse.rs:151-188`).
- `DirBuilder::mode(0o700)` applies only to a new directory (`:143-146`).
- The test `a_pair_file_is_private_and_the_newest_is_chosen` is at `src/relay_reverse.rs:879`.

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

`docs/protocol.md:209` says the alternates are the first 3 hosts of the pool. The text after it says the default relay uses 6 hosts until a pool is cached. The code takes 3 (`MAX_ALTERNATES`, `src/relay_pool.rs:39`, `:119`). The seed has 6 endpoints (`ajam-relay.toml`). A connect with no pool tries the primary and 3 hosts. The stagger is 2 s each (`src/ws_relay.rs:200`).

## Premise

- The test pins the count of alternates only (`src/relay_pool.rs:347`).
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

2026-11-09, commit be7b624. `docs/protocol.md` says that the seed holds 6 hosts and the client takes 3 (`MAX_ALTERNATES`). `docs/decisions.md` has the ruling. The test `the_alternates_come_from_the_cache_else_the_seed` checks `seed().len() >= MAX_ALTERNATES`. `scripts/agent/check.sh` exits 0. CI: at the merge.

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
- `detach_of` has no arm for it: `src/cli/opts.rs:253-262`. `connect` has one: `:246`.
- Both accept loops use `accepted?`: `src/cli/cmd/relay.rs:186`, `:198`. `connect -L` logs and returns:
  `src/cli/cmd/connect.rs:185-188`.
- The unix branch calls `claim_socket_path(&path, false)` with no guard: `src/cli/cmd/relay.rs:192`.
  `SocketFile` is in `src/cli/cmd/connect.rs:196-216` (`pub(crate)`).
- Message: `src/net.rs:38-44`.
- `src/main.rs:139` allows `--replace` without `--detach` only for `Command::Connect`.

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

2026-11-09, commit 456c7d9 (`T-199: relay connect --listen has the instance surface of connect -L`). `relay connect` flattens `DetachArgs` (the positional is the field `pair`: the id `name` clashed with `--name`); `detach_of` has an arm that applies only with `--listen`; `--replace` works as for `connect -L` (`replaces_a_socket` in `src/cli/opts.rs`). An accept error ends the command with exit 69 (a supervisor starts it again). The unix listener uses `connect::unix_listener` (`SocketFile` removes the file at the end; the real `replace` reaches `claim_socket_path`). The ready document has `pair` and `listen`. A detached start moves `--token` of `relay connect` to `PODNET_RELAY_CONNECT_TOKEN` in the secret file (`split_secrets`, `SECRET_VARIABLES`); before, it went to `TUNNEL_TOKEN` and the child had no token. The message of `claim_socket_path` stays: `podnet ps`, `podnet stop NAME` and `--replace` now exist for this caller. The tests are in the new file `tests/relay_listen.rs` (`tests/cli.rs` is in the size allowlist): `relay_connect_listen_detach` (start with `--detach --name fwd`, `ps --json` lists `fwd`, no token in the record, `stop fwd` ends it) and `relay_connect_unix_socket_removed` (the socket file is gone after SIGTERM); both fail before the change. `relay connect x --listen tcp://127.0.0.1:0 --detach --help` exits 0. `cargo test --workspace --no-fail-fast` passes; the other checks of `scripts/agent/check.sh` exit 0. CI: at the merge.

# T-201: `WebSocketTunnel` dials through the egress layer and parses IPv6 (GitHub #398)

**Source:** GitHub #398 (https://github.com/PodUtils/podnet/issues/398)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

`WebSocketTunnel::connect` opens TCP with `Direct::new().connect`. `Direct` uses the system resolver only: it ignores
`--proxy`, `NO_PROXY`, `--resolve` and DoH. `parse_ws_url` has no bracket handling, so `wss://[::1]:8443/ws` fails. The
TLS config is `util::tls::client_config(..)` with no ALPN pin. The trait arguments `host` and `port` are dropped. No
command builds the type (latent).

## Premise

- `src/transport/websocket.rs:181-185` (direct dial, `host` and `port` dropped), `:142-150` (`rsplit_once(':')`),
  `:193-197` (TLS).
- The precedent: `src/ws.rs:120-123`, `:155` (`crate::egress::dial`), `:161` (ALPN `http/1.1`);
  `src/ws_relay.rs:605-619`; `Host::parse`, `src/ws_relay.rs:137-149`.
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

# T-202: `serve` of a directory with no `index.html` logs no WARN for the 404 at `/` (GitHub #399)

**Source:** GitHub #399 (https://github.com/PodUtils/podnet/issues/399)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

`Show::root_404` makes the verify accept a 404 at `/` as the normal answer (issue 119, commit 0775407). No code sets it
to true. Each `serve --dir DIR` of a directory with no `index.html` logs `WARN ... the origin answered HTTP 404`.

## Premise

- `grep -rn root_404 src` gives 3 lines: `src/cli/opts.rs:114` (field), `:124` (`root_404: false`),
  `src/cli/cmd/run.rs:31` (read).
- `serve` passes `args.common.show()`: `src/cli/cmd/share.rs:149-151`.
- The warn branch: `src/tunnel.rs:1266-1270`. `static_router` gives 404 at `/` with no index: `src/service.rs:110-150`.
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

# T-203: The ready line of `share --json` has `schema` and `podnet` (GitHub #400)

**Source:** GitHub #400 (https://github.com/PodUtils/podnet/issues/400)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

Each JSON document of podnet has the keys `schema` (1) and `podnet` (`status::stamped`, `src/status.rs:119-127`).
`run_share` prints its ready document with a bare `serde_json::json!`.

## Premise

- `src/cli/cmd/run.rs:79-90` (no `stamped`). The only `stamped` call of `run.rs` is `:248` (`announce`).
- `grep -rn "status::stamped" src/cli` gives 8 hits, none in `run_share`.
- The `verified` line is unstamped in every command (`run.rs:119`, `:155`). It stays as it is.

## Approach

1. Move the document to a helper `share_ready_doc(..) -> Value` and wrap it with `podnet::status::stamped`.
2. Add a unit test of the helper. Keep the `verified` line as it is.

## Prove

`cargo test -p podnet --lib share_ready_doc_is_stamped` (new test: `schema == 1`, non-empty `podnet`).
`cargo test -p podnet` (the other documents keep both keys).

# T-204: `tunnel --config` refuses the origin flags (GitHub #401)

**Source:** GitHub #401 (https://github.com/PodUtils/podnet/issues/401)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

With `--config FILE`, the code skips the flag-built ingress. `--forward`, `--url`, `--unix`, `--port`, `--tcp`,
`--hello-world`, `--no-tls-verify`, `--http-host-header` and `--strip-accept-encoding` have no effect, and nothing says
so. The README (`:307-362`) documents the precedence of the token and protocol flags only.

## Premise

- Guard: `src/cli/cmd/tunnel.rs:98` (`if args.config.is_none()`). The flags are read at `:100-124`, `:207`.
  The three per-origin flags are declared at `:56-64`.
- The token case already has a usage error: `src/cli/cmd/tunnel.rs:73-89` (issue 172).
- Existing tests with `--config`: `tests/cli.rs:71`, `:610`, `:1237`, `:1298` (none gives an origin flag).

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

# T-205: A nested detached start does not inherit `PODNET_DETACH_SUPERVISED` (GitHub #402)

**Source:** GitHub #402 (https://github.com/PodUtils/podnet/issues/402)
**Category:** defect
**Milestone:** M5
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

`start_inner` copies each `PODNET_*` variable of its environment into the child. The copy includes
`PODNET_DETACH_SUPERVISED`. A detached child that starts a detached instance with `--no-restart` makes a grandchild that
takes the child lock and writes a record with `supervisor: None`. `targets()` then returns an empty list. `podnet stop`
sends no SIGTERM, waits the full timeout and sends SIGKILL.

## Premise

- `src/detach.rs:164-190` (`child_begin` reads the variable). `:513-520` sets 4 detach variables and not this one.
  `:892` is the only writer (the supervisor).
- `src/autostart.rs:178-182` copies each `PODNET_` variable except `PODNET_ERROR_PROGRAM`.
- `src/detach.rs:709-716` (`targets()` gives `Vec::new()` for this record).
- The default start (restart on) runs `--detach-supervise`: the supervisor sets the variable for its own child, so that
  path is right. The defect needs `--no-restart`. I did not run it.
- `src/detach.rs` is in the size allowlist (1210 lines): add no net line.

## Approach

1. Add `.env_remove(SUPERVISED_VARIABLE)` in `start_inner` (offset by removing a line).
2. Exclude `PODNET_DETACH_SUPERVISED` in `split_secrets` (as `PODNET_ERROR_PROGRAM`).
3. Add a test of the argv and environment of the child, and a test of `targets()` for "no supervisor, instance lock free,
   child lock held".

## Prove

`cargo test -p podnet --lib detach::` (new tests: `child_env_has_no_supervised_variable`,
`split_secrets_skips_supervised_variable`).
`cargo test -p podnet --test detach` (existing tests pass).

# T-206: `agent status --json` (GitHub #403)

**Source:** GitHub #403 (https://github.com/PodUtils/podnet/issues/403)
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

`agent status` prints text only. `agent up` and `doctor --agent --json` print stamped JSON. `Status` has no fields, so
`podnet agent status --json` exits 64. The report type already derives `Serialize`.

## Premise

- `src/cli/cmd/agent.rs:14-35` (`Status` with no fields), `:205-211` (`instances_ps(false)`, `agent::text`).
- `src/cli/cmd/instances.rs:10-41` (`instances_ps(json)` prints its own document).
- `src/cli/cmd/host.rs:234-244` (stamped report). `src/agent.rs:143-145` (`Serialize`). `README.md:997-1003`.
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

# T-207: The library static path decodes `+` as a literal plus (GitHub #404)

**Source:** GitHub #404 (https://github.com/PodUtils/podnet/issues/404)
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

`origin::static_files::percent_decode` turns `+` into a space. `service::percent_decode_path` (the CLI), and
`ingress::decode_path`, keep `+`. A `+` in a URL path is a literal plus. The README says both paths follow the same rules.

## Premise

- `src/origin/static_files.rs:256-258` (the `b'+'` arm), `:84-85` (`resolve`), test `:603-607` asserts `"a b"`.
- `src/service.rs:217-232`, `:183`. `src/ingress.rs:184-187`, test `:616-629` (`"a + stays a +"`).
- `README.md:482-483`.
- One more divergence (not in the issue): a bad escape (`%zz`) passes through in the library decoder (`:249-251`, test
  `:606`). The CLI decoder returns `None` (`src/service.rs:222-225`), so the request is refused. A short escape at the
  end (`i + 2 < len`) also differs.

## Approach

1. Remove the `+` arm. Change the test to assert `"a+b"` and `"a b"` for `%20`.
2. Add a test that sends the same path to `StaticDir::respond` and `static_router` over one file set (a file `a b.txt`
   and a file `a+b.txt`).
3. Decide the bad-escape case in the same test. Recommendation: the library refuses it as the CLI does.

## Prove

`cargo test -p podnet --lib origin::static_files::tests::percent_decoding` (changed test).
`cargo test -p podnet --lib static_paths_agree_on_plus` (new test).

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

- `src/cli/cmd/warp/up.rs:19-29` (the `tun: true` arm matches `..`), `:403-415` (`warp_up_tun_args`), `:207-211` (the
  proxy path pushes `--replace`).
- `src/cli/cmd/warp/args.rs:391-435` (the flags), `:404` (`conflicts_with = "persist"` on `tun`), `:513-528` (`Tun` has
  `DetachArgs`, so `--replace` exists there, and has no `--dns`).
- `src/detach.rs:486-493` (the refusal text).

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
then restarts a working tunnel. `agent status` has the same text (`src/agent.rs:488`).

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
**Status:** open

## Problem

`warp_plus_label` gives `yes` for each non-empty `account_type` that is not exactly `free`, or when `premium_data > 0`.
`warp status`, `warp status --json` (`warp_plus_active`) and `warp license` use it. The issue says a free account with
another type is shown as WARP+.

## Premise

- `src/warp/identity.rs:157-171`. Users: `src/cli/cmd/warp/account.rs:35`, `:53-56`, `:88-91`. Test: `src/main.rs:892-895`.
- The issue says the fixture type `limited` is a free type. This is not shown: `src/warp/register.rs:687` is a test of
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

## Correction

2026-11-09: the measurement of Approach 1 is not possible in this session. No record of the `account_type` values of the live service exists in the tree: `grep -rn account_type docs tests src` finds the report line `docs/reports/aerrand-agent.md:639` (type `free` with `warp_plus` true), the fixtures `src/warp/register.rs:687` and `src/main.rs:894` (`limited`, made up), and no list of the paid types. A registration spends a device and needs the live service (`docs/decisions.md`, WARP devices). The code stays as it is. Start condition: one `podnet warp status --json` of a free device and one of a device with a licence, recorded in `docs/reports/`.

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

`README.md:700-701` says: "UDP (SOCKS5 UDP ASSOCIATE) is not served, and the rules do not apply." The code applies
`-r`, `--rules-file` and `--rule-default` to `--l4`. The `--help` text says so.

## Premise

- `README.md:697-701`. `src/warp/proxy.rs:509-529` (the dial closure: `rules::current()`, `decide_static`, three actions).
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

2026-11-09, commit be7b624. The README says that the rules apply to `warp proxy --l4`. `grep -n "rules do not apply" README.md` prints nothing. Test: `warp::rules` lib tests. `scripts/agent/check.sh` exits 0. CI: at the merge.

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

- `src/warp/register.rs:181-190` (`Endpoint`; frozen file: no edit). `src/warp/export.rs:242-260`, `:30` (`WG_PORT`).
- `src/warp/wg.rs:482-489` (v4 only; the message `:485-487`).
- MASQUE reads `v4` and `v6`: `src/warp/register.rs:618-657`.
- `grep -rn "\.ports" src/warp` finds no reader.
- Test to extend: `src/warp/export.rs:399-434`.

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

- `src/warp/nc.rs:294-303` (raw `{exe}` at `:285`). `src/warp/ssh_config.rs:34-36`, `:65-73`.
- The only test of the printed block: `src/warp/nc.rs:356-360` (plain path).
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

- `src/warp/proxy.rs:195-217` (`serve_dns`), `:139-150` (`admit`), `:160-187` (`health_log`), `:110-125` (`Stats`),
  `:383-385` (length check in `forward_dns_to`).
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

# T-216: Freeze `doctor --json` as schema 1, with `--quick` (PodUtils/podcrates#35, A-N-04)

**Source:** PodUtils/podcrates#35 (cfb), ask A-N-04, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** open

## Problem

Ask A-N-04 (P1): "Freeze `podnet doctor --json` as schema 1 for the legs and hops, with a quick mode."
cfb reads the document to qualify a host. The keys must not change within schema 1, and a run must end soon.

## Premise

- Read: `src/doctor.rs:243` (`to_json`) writes `ok`, `roads`, `legs` (`leg`, `verdict`, `status`, `reason`) and `names`, and `status::stamped` adds `schema` 1 and `podnet`. No test pins the key set.
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

# T-217: `doctor --json`: a fixed block for the egress lanes (PodUtils/podcrates#35, A-C-10)

**Source:** PodUtils/podcrates#35 (cfb), ask A-C-10, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M5
**Priority:** P1
**Effort:** S
**Status:** open

## Problem

Ask A-C-10 (P1): "A fixed JSON schema of the egress lanes for the node probe of cfb: `tunnel_quic`, `tunnel_http2`, `tunnel_quic_warp`, `masque_tcp443`, `masque_udp443`, `ws_upgrade` and `proxy`, each `ok`, `fail` or `unknown`, with the time."

## Premise

- Read: `src/doctor.rs:221` (`roads`) groups the legs by road. The legs have their own names, not the seven lane names. A leg has a time (`ms`) in the paths, not in the legs.
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

# T-222: `llms.txt`, `llms-full.txt` and `--help --json` (PodUtils/podcrates#35, A-X-19)

**Source:** PodUtils/podcrates#35 (cfb), ask A-X-19, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

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
