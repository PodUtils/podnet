# podnet hard problems: research results

This page summarizes the answers to all 32 items (`H-01` to `H-32`) in `docs/research/podnet/hard-problems.md`. The study evaluated `podnet` at `177336b` (`v0.4.0-beta.17`) against `podcrates` `a44b185` (`v0.78.1`) on Linux x86_64 (Debian 13, kernel `6.12.116`, uid 1000, `CapEff 0`, Landlock ABI 6). Five rounds produced working proofs of concept, live measurements, and patches.

## Summary of all 32 items

| Item | Old grade | New grade | Answer |
| --- | --- | --- | --- |
| H-01 | very hard | likely impossible (full fidelity), partial route | `SECCOMP_IOCTL_NOTIF_ADDFD` installs a kernel socket with true `AF_INET` names. Exact peer names still require either binding the destination port or writing program memory (`CLONE_NEWUSER` plus `process_vm_writev`); neither holds on hosts that block both. |
| H-02 | likely impossible | very hard | Landlock ABI 6 enforces an outbound TCP port allow-list and closes inherited sockets, but ignores destination addresses, UDP below ABI 10, unix, netlink, vsock, and unbound `listen(2)`. Refusing `listen` on `Sock::Kernel` with `EPERM` closes the wildcard inbound door in `podnet-netns`. The hold stays advisory. |
| H-03 | likely impossible | unchanged | A static musl binary cannot resolve when UDP sockets are refused. Bind-mounting a merged `/etc/hosts` in the mount namespace works for known names; a seccomp supervisor can also interpose `/etc/hosts` from memory. `AI_ADDRCONFIG` still fails when UDP is denied. |
| H-04 | hard | hard (fixed for IPv4 UDP) | Oversize UDP datagrams above 1472 bytes received false ICMP port unreachable from smoltcp. Reassembling IPv4 fragments in `crates/podnet-netns/src/netns/udpflow.rs` raises `NETNS_UDP_LIMIT` to `65507` bytes with bounded sets (64), pieces (1024), 30 s idle reap, and overlap drops. ICMP through SOCKS5 remains unsupported by SOCKS5. |
| H-05 | likely impossible (shim) | confirmed in shim, met outside it | Static binaries ignore `LD_PRELOAD`; `io_uring` and `/proc/net` require `netns`. Per-run unique `PODNET_VNET_PREFIX` values (capped at 59 bytes, validated before build with exit 64, unconditional stderr warning in C shim) prevent abstract-name collisions between two shims in one network namespace. |
| H-06 | hard | hard | `pod-netstack` already provides counted loss (`udp_rx_dropped`, `udp_tx_dropped`, `udp_oversize`) with a 128-datagram capacity (64 backlog + 64 app channel). The guarantee is counted loss, not zero loss. |
| H-07 | very hard | very hard | Unprivileged `ip mptcp endpoint add` returns `EPERM`, and MPTCP cannot cross a CONNECT proxy. A userspace multi-leg bond over TCP demonstrates failover and direction split with order intact; integration belongs as a `pod-link` bond service at a podnet exit. |
| H-08 | very hard | hard (policy-bound) | A 3-line `ring` patch builds all 8 static musl targets, but adds 134 lockfile packages and trips `cargo deny` on `chrono`, `paste`, and `dyn-eq`. Remaining blockers are dependency policy. |
| H-09 | hard | hard | NetBird ignores `HTTPS_PROXY`, `HTTP_PROXY`, and `ALL_PROXY` for management, signal, and relay. Use `podnet pod` with `NB_RELAY_TRANSPORT=ws`. |
| H-10 | likely impossible (direct) | works (direct MASQUE and edge via WARP) | Direct MASQUE over TCP 443 through an HTTP CONNECT proxy works with an anonymous WARP device (DNS, TCP, HTTPS carried). A paired `quic+warp` edge run verified `HTTP 200` and served a visitor `HTTP/2 200`. Direct TCP 7844 remains blocked by allowlist-only proxies, and TLS-terminating proxies still break the MASQUE pin. |
| H-11 | hard | hard | `cloudflared` uses 1 HA connection for quick tunnels as a client policy, not a documented edge rule. 20 rounds per arm is underpowered; a paired early-vs-steady McNemar design (20 rounds of 10 pairs) has power 0.996 for 10% vs 1%. Capturing the HTTP 530 status, `cf-ray`, and body (1XXX code) comes first. |
| H-12 | very hard | hard (stand-in), very hard (faithful edge) | The tunnel client already reaches a stand-in and `pod-cloudflare` is unfrozen. One server module with ALPN `argotunnel` closes the stand-in gap. |
| H-13 | very hard | hard | Public clients send HTTP/3 `CONNECT` with an IP-literal `:authority` and no `:scheme`, `:path`, or `:protocol`. `podnet` sends a hostname; changing to the IP-literal form is the leading candidate fix for 403. |
| H-14 | likely impossible now | hard (code ready) | ECH is implemented and tested in `pod-tls`, but no Cloudflare WARP or tunnel hop publishes `ech=` in HTTPS DNS records. |
| H-15 | hard | hard | A failed health check ends only the first WARP session. Because the listener binds before the session is ready, queued clients receive `ECONNRESET`. |
| H-16 | very hard | hard | WARP registration returns HTTP 429 after about 10 calls in 90 minutes and clears after about 30 minutes. Idle MASQUE sessions close near 5 minutes. |
| H-17 | hard | hard | The relay contract states no node-silence bound. A half-open TCP write fails at 938.5 s at default `tcp_retries2` and at 3.5 s when `tcp_retries2=2`. |
| H-18 | hard | hard | No pair-renewal endpoint exists and pool hosts answer 503. Design: `POST /v1/pair/NAME/renew`, renew at 48 h, one shared pair store, and a systemd timer. |
| H-19 | hard | hard | Worst-case bytes before a reverse-relay move are `move_bytes + PAIR_WINDOW + 2 * CHUNK + 64`. The default 48 MiB move under a 64 MiB cap has 15.2 MiB of room, while the 1 MiB test with a 256 KiB move overshoots by 65,600 bytes. Moving at `<= 191 KiB` (160 KiB in tests) fits safely under 1 MiB. |
| H-20 | hard | hard | Two public transit relays answer on port 443 and `podnet` already dials them. No podnet-owned mailbox exists. |
| H-21 | very hard | very hard | A rootless two-NAT namespace lab reproduces cone and symmetric-like failures. The relay carries candidates inside a session, not before it. |
| H-22 | hard | hard | The concurrency semaphore exists. A 32-request window helps at high RTT (3.35 s down to 0.91 s at 200 ms), after fixing ID matching and backoff first. |
| H-23 | hard | hard | `pod-wg` interoperates with Linux kernel WireGuard in both roles. Only the `pod-link` carrier glue is missing. |
| H-24 | hard | hard | A standalone RFC 6062 client completed Allocate, Connect, and ConnectionBind against `coturn` 4.6.1 over TLS and plain TCP. `pod-turn` carries the TCP allocation API from `v0.82.0`. |
| H-25 | hard | hard | The Shadowsocks decoder, HKDF-SHA1 schedule, and 3 test vectors reproduce. A listener, salt replay cache, target policy, and Trojan fallback remain. |
| H-26 | hard | hard | Release binaries build default features; `full` adds only pure-Rust crates (no C dependency risk). A build matrix and binary-size decision remain. |
| H-27 | very hard (as a set) | very hard (split corrected) | The deferred issues `#484` to `#501` split into 10 feasible items, 4 items needing outside resources (`#489`, `#497`, `#498`, plus MASQUE enrolment in `#486`), and 0 impossible items. |
| H-28 | hard | hard | One unprivileged uid 1000 process can combine Landlock, hide-`/proc`, and seccomp holds. `bind-netns-vm-denied` does not produce the read-only memory state. |
| H-29 | hard | hard | Seven of eight named flaky tests exist; `quick_tunnel_standin_serves_a_visitor_over_http2` is stale and removed from `docs/HANDOFF.md`. `T-479` is done. |
| H-30 | hard | hard | Bare `cargo install` skips prereleases. `cargo publish --dry-run` catches missing registry versions; `podcrates` publishes before `podnet`. |
| H-31 | very hard | hard | The Zero Trust team header exists in `pod-warp` without a `podnet` CLI flag. Third-party captures show `warp_plus=true` on a free account, so that boolean alone is unreliable. |
| H-32 | hard | hard | The size allowlist is empty and the three early crates are already taken. Two crate-adoption ordering constraints remain. |

## Changes landed directly from this study

1. **H-02**: `crates/podnet-netns/src/seccomp/mod.rs` and `crates/podnet-netns/src/seccomp/accept.rs` refuse `listen(2)` on `Sock::Kernel` with `EPERM`, verified by 3 cases (bare IPv4, bound IPv4, bare IPv6) plus `/proc/<pid>/fd` inode checks in `tests/pod_seccomp/kernel_listen.rs`.
2. **H-04**: `crates/podnet-netns/src/netns/udpflow.rs` reassembles IPv4 UDP fragments up to 65,507-byte payloads (`NETNS_UDP_LIMIT = 65_507`), verified by 17 unit tests (including oldest-set eviction) and `tests/pod_seccomp/netns_udp.rs`.
3. **H-05**: `crates/podnet-vnet/src/{addr,shim}.rs`, `crates/podnet-vnet/shim/podnet-vnet.c`, and `src/cli/cmd/net/shim.rs` generate per-run `PODNET_VNET_PREFIX` values, cap prefixes at 59 bytes, reject invalid or non-UTF-8 prefixes before build with exit 64, copy full 107-byte `sun_path` names, and print ignored-prefix warnings unconditionally to stderr.
4. **H-10**: `docs/warp.md` corrects the TCP probe description to reflect live TCP carriage over MASQUE.
5. **H-11 & H-19**: `scripts/live-throughput.sh` captures non-2xx HTTP status, `cf-ray`, and error bodies instead of discarding them with `curl -f`; `tests/transfer_relay.rs` sets the 1 MiB cap test move threshold to 160 KiB (`<= 191 KiB`).
