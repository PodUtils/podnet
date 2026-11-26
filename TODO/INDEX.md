# Index

Each entry of the work record, one row each, sorted by id. The entry is in
the area file that its row links. It closes there, with its Prove run and
the result written.

`TODO/PROGRESS.md` has the only work order. `TODO/RULES.md` gives the rules
of the record. `TODO/issues.md` maps each GitHub issue to its entries.

`cargo todo set` and `cargo todo counts` derive the counts from the rows.
`cargo todo check` makes sure that the counts, the rows and the entries
agree.

## Values

**Priority.**

- **P0**: correctness fails, data is lost, or the process stops.
- **P1**: a documented capability does not work, or a flag does nothing.
- **P2**: worth doing; nothing is wrong without it.
- **P3**: worth recording, so that nobody must find it again.

**Effort.** S is less than one day. M is some days. L is one week. There
is no XL: make two entries.

**Status.** `open`, `partial`, `blocked` or `done` (`TODO/RULES.md`).

**Milestone.** `M0` to `M9`, as in `TODO/ROADMAP.md`: M0 Record, M1
Rename, M2 Split, M3 Crates, M4 Adapters, M5 Fixes, M6 Services, M7 Pods,
M8 Overlays, M9 Later.

**Category.** `defect`, `feature`, `measurement`, `release`, `chore`,
`research` or `docs`.

## Entries

**610 entries: 8 open, 9 partial, 32 blocked, 561 done.**

| Priority | open | partial | blocked | done | total |
| --- | --- | --- | --- | --- | --- |
| P0 | 0 | 0 | 0 | 1 | 1 |
| P1 | 0 | 2 | 3 | 92 | 97 |
| P2 | 0 | 4 | 15 | 296 | 315 |
| P3 | 8 | 3 | 14 | 172 | 197 |
| **All** | 8 | 9 | 32 | 561 | 610 |

| ID | Priority | Effort | Milestone | Category | Status | Item |
| --- | --- | --- | --- | --- | --- | --- |
| [T-001](deferred.md) | P3 | S | M9 | chore | blocked | Tracking: dev work in a sandbox with basic networking, and launch once and forget (GitHub #89) |
| [T-002](deferred.md) | P2 | M | M9 | feature | blocked | `warp proxy --l4`: the L4 door answers 403 to each form of the CONNECT (GitHub #140) |
| [T-003](deferred.md) | P2 | S | M9 | feature | blocked | `warp register`: enrol through a Zero Trust team token (GitHub #149) |
| [T-004](deferred.md) | P3 | M | M9 | feature | blocked | TLS: send an encrypted ClientHello on the hops that publish a config (GitHub #151) |
| [T-005](deferred.md) | P2 | S | M9 | release | blocked | crates.io: publish the library and the CLI (GitHub #161) |
| [T-006](deferred.md) | P2 | S | M9 | release | blocked | crates.io: the workspace version and the `podnet-netns` requirement are prereleases (GitHub #222) |
| [T-007](deferred.md) | P2 | S | M9 | release | blocked | crates.io: the name `cfrs` belongs to another project; choose a package name (GitHub #224) |
| [T-008](record.md) | P1 | S | M0 | chore | done | A parity snapshot: the command tree, the public items and the test list, with a diff of two runs |
| [T-009](record.md) | P1 | S | M0 | chore | done | A size check: at most 800 lines in each source file, with an allowlist that can only shrink |
| [T-010](record.md) | P1 | M | M0 | chore | done | A layer check: no module refers to a module of a higher layer, with an allowlist of 18 references that can only shrink |
| [T-011](record.md) | P2 | M | M0 | chore | done | Loopback tests for the commands that have no behaviour test, before the splits |
| [T-012](record.md) | P2 | L | M0 | chore | done | WARP stand-ins on the loopback: the registration API and a MASQUE edge, with tests of the WARP commands |
| [T-013](record.md) | P2 | S | M0 | chore | done | A daily check that the record's list of GitHub numbers agrees with GitHub |
| [T-014](record.md) | P2 | S | M0 | docs | done | The milestones M0 to M9 in a roadmap, and the open work of the maintenance page in the record |
| [T-015](record.md) | P3 | S | M2 | chore | done | CI checks that each pushed commit moved the citations of the files that it changed |
| [T-016](rename.md) | P1 | M | M1 | chore | done | Rename `cfrs` to `podnet` in the code, the tests, the workflows, the scripts and the documents |
| [T-017](rename.md) | P2 | S | M1 | docs | done | The documents and the scripts state the facts of the old name |
| [T-018](rename.md) | P2 | S | M1 | docs | done | `CLAUDE.md` names `podnet`, its crates and the record |
| [T-019](rename.md) | P1 | M | M1 | chore | done | Drop the `pod-netns` binary, and move its integration tests to the root crate |
| [T-020](rename.md) | P1 | S | M1 | chore | done | Rename the crate `pod-netns` to `podnet-netns` |
| [T-021](rename.md) | P1 | S | M1 | chore | done | Verify the GitHub redirects after the transfer of the repository to PodUtils |
| [T-022](rename.md) | P1 | S | M1 | release | done | The version `0.4.0-beta.1`, and its release note with each breaking change |
| [T-023](rename.md) | P1 | S | M1 | release | done | Publish `v0.4.0-beta.1`: the dry run, the publish and the release proofs |
| [T-024](rename.md) | P2 | S | M1 | chore | done | The update checks pass against a podnet release again |
| [T-025](rename.md) | P2 | S | M1 | chore | done | The image `ghcr.io/podutils/podnet` is public |
| [T-026](split.md) | P2 | M | M2 | chore | done | Move the unit tests of nine files into their own files |
| [T-027](split.md) | P2 | M | M2 | chore | done | `src/main.rs` to a CLI module, part 1: the signals, the help, the argument rewrites and the shared options |
| [T-028](split.md) | P2 | M | M2 | chore | done | `src/main.rs` to a CLI module, part 2: the host commands and the `net` commands |
| [T-029](split.md) | P2 | M | M2 | chore | done | `src/main.rs` to a CLI module, part 3: the server commands |
| [T-030](split.md) | P2 | M | M2 | chore | done | `src/main.rs` to a CLI module, part 4: the WARP arguments, the client and `warp up` |
| [T-031](split.md) | P2 | M | M2 | chore | done | `src/main.rs` to a CLI module, part 5: the other WARP subcommands |
| [T-032](split.md) | P2 | M | M2 | chore | done | `src/main.rs` to a CLI module, part 6: the unit tests go to the modules that they test |
| [T-033](split.md) | P2 | M | M2 | chore | done | Split `src/egress.rs`, `crates/podnet-core/src/detach.rs`, `src/update.rs` and `src/doctor.rs` |
| [T-034](split.md) | P2 | M | M2 | chore | done | Split `crates/podnet-vnet/src/stack.rs` |
| [T-035](split.md) | P2 | M | M2 | chore | done | Split `src/warp/ip.rs`, `src/warp/session.rs` and `src/warp/wg.rs` |
| [T-036](split.md) | P2 | M | M2 | chore | done | Split `src/warp/mod.rs` and `src/warp/proxy.rs` |
| [T-037](split.md) | P2 | M | M2 | chore | done | Split `src/relay_reverse.rs` and `crates/podnet-serve/src/share/mod.rs` |
| [T-038](split.md) | P2 | M | M2 | chore | done | Split `crates/podnet-serve/src/feature/mod.rs` and `src/proxy.rs` |
| [T-039](split.md) | P2 | M | M2 | chore | done | Split `src/edge_h2.rs` and `src/tunnel.rs` |
| [T-040](split.md) | P2 | L | M2 | chore | done | Split the seccomp backend and the netns backend of the pod crate |
| [T-041](split.md) | P2 | M | M2 | chore | done | Split the four large test files |
| [T-042](split.md) | P2 | M | M2 | chore | done | Cut the upward references 1 to 7: `net`, `util`, `fetch`, `hop`, `report`, `detach` and `relay_reverse` |
| [T-043](split.md) | P2 | M | M2 | chore | done | Cut the upward references 8, 9 and 18: a WebSocket bridge and `Metered` in the core |
| [T-044](split.md) | P2 | L | M2 | chore | blocked | Cut the upward references 10 to 17: the WARP QUIC configuration, `ShareLink`, `RULE_KEYS`, `connect_tunnel` and the edge side of `ws` |
| [T-045](split.md) | P2 | S | M2 | chore | done | Each file is under 800 lines, and the allowlists of the size check and the layer check are empty |
| [T-046](crates.md) | P2 | M | M3 | chore | done | Prepare the workspace for more crates: the print macros, the build facts, the scans, the package list and the test helpers |
| [T-047](crates.md) | P2 | L | M3 | chore | done | Extract the crate `podnet-core` |
| [T-048](crates.md) | P2 | M | M3 | chore | done | Extract the crate `podnet-vnet`, with the C sources of the shims |
| [T-049](crates.md) | P2 | S | M3 | chore | done | The relay client of podnet becomes the shared crate `pod-relay` |
| [T-050](crates.md) | P2 | M | M3 | chore | done | Move the WARP adapter to the shared crate `pod-warp` in `PodUtils/podcrates` |
| [T-051](crates.md) | P2 | M | M3 | chore | done | Extract the crate `podnet-serve`, with the library-only modules |
| [T-052](crates.md) | P2 | M | M3 | chore | done | Move the Cloudflare adapter to the shared crate `pod-cloudflare` in `PodUtils/podcrates` |
| [T-053](crates.md) | P2 | M | M3 | chore | done | The root package is the CLI and a facade that keeps each library path |
| [T-054](crates.md) | P2 | S | M3 | release | done | A prerelease after the crate split, with the static builds of each target |
| [T-055](adapters.md) | P2 | M | M4 | feature | done | The network types in `podnet-core`: a target, a dialer, datagrams, a network and its link |
| [T-056](adapters.md) | P2 | M | M4 | feature | done | Dialers for direct, the egress proxy, the relay, the pod upstream chains, the tailnet front door and the virtual network |
| [T-057](adapters.md) | P2 | M | M4 | feature | done | A dialer and datagrams for WARP |
| [T-058](adapters.md) | P2 | M | M4 | feature | done | The combinators `Chain`, `Rules`, `Race` and `Failover` |
| [T-059](adapters.md) | P2 | M | M4 | feature | done | The registry of networks, the parser of network specs, and the rows of `doctor` |
| [T-060](adapters.md) | P2 | M | M4 | feature | done | `--via SPEC` on each command, with each old network flag as an alias |
| [T-061](adapters.md) | P2 | M | M4 | feature | done | One front door over a dialer for `warp proxy`, `pod --serve`, `net proxy` and `net tailscale` |
| [T-062](adapters.md) | P2 | M | M4 | feature | done | The service and ingress traits, with wrappers of the Cloudflare tunnel, the local listener, the relay node and the virtual network |
| [T-063](fixes.md) | P1 | M | M5 | defect | done | seccomp: a TCP socket stays tracked after its connect, and its options and addresses come from its entry |
| [T-064](fixes.md) | P1 | M | M5 | defect | done | seccomp: one entry for each socket of each process, and copies of a descriptor are tracked |
| [T-065](fixes.md) | P3 | S | M5 | feature | done | seccomp: hand a plain upstream descriptor to the program after its handshake |
| [T-066](fixes.md) | P2 | M | M5 | defect | done | seccomp: ICMP datagram sockets, `recvmmsg`, and UDP names over 255 bytes |
| [T-067](fixes.md) | P2 | M | M5 | defect | done | seccomp: a slow `connect` dial does not stop the supervisor for the other calls |
| [T-068](fixes.md) | P1 | M | M5 | defect | done | WARP: a probe of the TCP data path renews a session that carries no TCP |
| [T-069](fixes.md) | P1 | L | M5 | defect | done | `warp tun`: a SYN probe of the session, and `--session-max-age` |
| [T-070](fixes.md) | P1 | S | M5 | defect | done | `warp exec`, `warp shell` and `warp nc` find a running `warp up` with another state directory |
| [T-071](fixes.md) | P2 | M | M5 | feature | done | `warp exec` and `warp shell` take `--backend`, name the backend in one line, and log the seccomp emulation |
| [T-072](fixes.md) | P2 | S | M5 | defect | done | `warp tun` sets `rp_filter` to 0 on its interface |
| [T-073](fixes.md) | P2 | M | M5 | defect | done | `warp forward`: the pins of `--resolve`, and the list of forwards |
| [T-074](fixes.md) | P2 | L | M5 | defect | done | `warp export`: the reserved bytes in the file, `--check`, and a TCP probe on the WireGuard leg |
| [T-075](fixes.md) | P2 | S | M5 | defect | done | The datagram socket of the vnet shim takes the same user only |
| [T-076](fixes.md) | P3 | S | M5 | chore | done | A CI job runs the tests with the loopback down |
| [T-077](fixes.md) | P2 | S | M5 | defect | done | An ingress `path:` rule matches the decoded path, as cloudflared does |
| [T-078](fixes.md) | P3 | S | M5 | defect | done | `pod doctor` uses the verdict words of `doctor` and has a `schema` key |
| [T-079](fixes.md) | P3 | S | M5 | measurement | done | Read the numbers of #206 and #184 in a live run |
| [T-080](fixes.md) | P3 | M | M5 | feature | done | The errors of a config file name the line |
| [T-081](fixes.md) | P2 | S | M5 | feature | done | The tailnet front door warns when `tailscaled` is not logged in |
| [T-082](services.md) | P2 | S | M6 | feature | done | `connect --stdio` for an SSH `ProxyCommand` through `tunnel --forward` |
| [T-083](services.md) | P2 | S | M6 | feature | done | A relay node serves a handler in the process, with no socket file |
| [T-084](services.md) | P2 | M | M6 | feature | done | One HTTP authentication layer for the local listener, the tunnel and the relay |
| [T-085](services.md) | P2 | M | M6 | feature | done | A ustar writer and reader, and a directory as one tar download of the file server |
| [T-086](services.md) | P2 | L | M6 | feature | done | Take the resumable layer, the end-to-end channel, the key identity and UDP in a stream from the shared crates |
| [T-087](services.md) | P2 | M | M6 | feature | done | `relay node` and `relay connect` resume a session past the limits of the relay |
| [T-088](services.md) | P2 | S | M6 | feature | done | File server: a directory listing in HTML and JSON |
| [T-089](services.md) | P2 | M | M6 | feature | done | File server: users with read and write roles, and a PIN, on each listener |
| [T-090](services.md) | P2 | M | M6 | feature | done | File server: uploads into the served tree with PUT and a multipart form |
| [T-091](services.md) | P2 | M | M6 | feature | done | `podnet serve DIR --via`, a local `share`, a directory in `share`, and the live step of the file server |
| [T-092](services.md) | P2 | L | M6 | feature | done | Git: a CGI runner for `git http-backend`, and the smart HTTP router |
| [T-093](services.md) | P2 | M | M6 | feature | done | Git: an SSH server for upload-pack and receive-pack |
| [T-094](services.md) | P2 | M | M6 | feature | done | `podnet git serve`: the repository root, `git init`, `git urls`, the tunnel, the relay and the live step |
| [T-095](services.md) | P2 | M | M6 | feature | done | Router: several WARP identities in one process, each with its own counters |
| [T-096](services.md) | P2 | M | M6 | feature | done | Router: groups of networks with weights and a kill switch |
| [T-097](services.md) | P2 | M | M6 | feature | done | Router: a TCP health check for each member, and `podnet route status` |
| [T-098](services.md) | P2 | M | M6 | feature | done | Router: doors with a default network, and rules to a network, a group or a refusal |
| [T-099](services.md) | P2 | M | M6 | feature | done | Router: `podnet forward` for each network, UDP forwards, a DNS door, and `podnet expose` |
| [T-100](services.md) | P2 | M | M6 | feature | done | Router: a config file, and `podnet route` with `--detach` and autostart |
| [T-101](services.md) | P2 | M | M6 | measurement | done | Router: the live step with WARP, the relay and direct, and a failover |
| [T-102](services.md) | P2 | M | M6 | feature | done | Take the mailbox client, the code words and the SPAKE2 phases from `pod-wormhole` |
| [T-103](services.md) | P2 | L | M6 | feature | done | The file-transfer protocol and the transit protocol of magic-wormhole |
| [T-104](services.md) | P2 | M | M6 | feature | done | The podnet relay as a carrier of a transfer |
| [T-105](services.md) | P2 | M | M6 | feature | done | `podnet send` and `podnet receive` |
| [T-106](services.md) | P2 | M | M6 | measurement | done | An interop test with the Python magic-wormhole, and the live step of a transfer of 100 MiB |
| [T-107](pods.md) | P2 | M | M7 | feature | done | seccomp: the pod lives past its first program |
| [T-108](pods.md) | P2 | S | M7 | feature | done | netns: the pod lives past its first program |
| [T-109](pods.md) | P2 | M | M7 | feature | done | `warp exec --detach --name N` keeps a pod under the supervisor |
| [T-110](pods.md) | P2 | M | M7 | feature | done | `warp exec --join N` runs a command in a detached pod |
| [T-111](pods.md) | P2 | M | M7 | feature | done | netns: generic UDP |
| [T-112](pods.md) | P3 | S | M7 | feature | done | netns: the gateway address reaches the loopback of the host, when the user asks |
| [T-113](pods.md) | P2 | S | M7 | feature | done | `warp exec` and `warp shell` try the netns backend first |
| [T-114](overlays.md) | P2 | M | M8 | chore | blocked | A headscale harness in CI for the `tailscaled` sidecar |
| [T-115](overlays.md) | P3 | S | M8 | feature | partial | The tailnet as a network of the registry and of the router |
| [T-116](overlays.md) | P3 | S | M8 | feature | done | WireGuard: read a wg-quick file into a peer, with optional reserved bytes |
| [T-117](overlays.md) | P3 | M | M8 | feature | done | WireGuard: `wg:FILE` as a network over UDP or over a carrier |
| [T-118](overlays.md) | P3 | M | M8 | research | done | Research: NetBird, Firezone and ZeroTier as networks |
| [T-119](later.md) | P3 | M | M9 | feature | done | Router kernel mode: tables, rules and multipath routes through rtnetlink |
| [T-120](later.md) | P3 | M | M9 | feature | done | Router kernel mode: masquerade, an MSS clamp, an unreachable kill switch and `rp_filter` |
| [T-121](later.md) | P3 | S | M9 | feature | done | Router: export the groups as a JSON file of the mwan form |
| [T-122](later.md) | P3 | M | M9 | research | done | Research: a native git server with no installed `git` |
| [T-123](later.md) | P3 | M | M9 | feature | open | Tailscale in the process: take the fork of tailscale-rs through `pod-ts` |
| [T-124](later.md) | P3 | L | M9 | feature | open | Tailscale in the process: an adapter crate behind a feature that is off by default, with the static build proven |
| [T-125](later.md) | P3 | L | M9 | feature | done | A userspace WireGuard responder: `podnet wg serve` |
| [T-126](later.md) | P3 | M | M9 | feature | open | Egress through an `https://` proxy and through wstunnel |
| [T-127](later.md) | P3 | L | M9 | feature | open | Egress through VLESS, Shadowsocks and Wisp |
| [T-128](later.md) | P3 | L | M9 | feature | open | The DERP carrier through `pod-derp` |
| [T-129](later.md) | P3 | M | M9 | feature | open | An ACME client for a public listener |
| [T-130](later.md) | P3 | M | M9 | research | done | Research: the iroh road, and a relay and mailbox of podnet's own |
| [T-131](later.md) | P3 | L | M9 | chore | done | A crate that podnet and podssh share for the layer, the channel and the identity |
| [T-132](later.md) | P3 | S | M9 | chore | done | `try_update` in place of the macro `update_atomic!` |
| [T-133](later.md) | P3 | S | M9 | chore | done | Zig from the community mirrors in the release workflow |
| [T-134](later.md) | P3 | M | M9 | chore | done | A weekly watch of the relay contract |
| [T-135](fixes.md) | P3 | S | M5 | defect | done | `podnet pod` prints its own program name in each message and in `--version` |
| [T-136](rename.md) | P1 | S | M1 | defect | done | live: two runs in a row failed before the transfer of the repository (GitHub #311) |
| [T-137](crates.md) | P1 | M | M3 | chore | partial | podnet uses the shared crates of `PodUtils/podcrates` |
| [T-138](fixes.md) | P2 | S | M5 | defect | done | live: two runs in a row failed on the edge and WARP services (GitHub #316) |
| [T-139](fixes.md) | P2 | S | M5 | defect | done | The start of a session changes the global git identity and can drop commits of main |
| [T-140](fixes.md) | P2 | S | M5 | feature | done | `data_path` and the time of the last TCP probe in `status`, the ready document and `warp status` |
| [T-141](fixes.md) | P1 | S | M5 | defect | done | live: two runs in a row failed after the take of pod-dns and pod-services (GitHub #325) |
| [T-142](fixes.md) | P2 | M | M5 | defect | done | seccomp: closing a UDP socket stops its relay and frees three descriptors (GitHub #347) |
| [T-143](fixes.md) | P1 | M | M5 | defect | done | seccomp: a datagram written with `write(2)` leaves the pod (GitHub #347) |
| [T-144](fixes.md) | P2 | S | M5 | defect | done | seccomp: `bind` on a datagram socket is recorded, and `getsockname` reports it (GitHub #348) |
| [T-145](fixes.md) | P2 | S | M5 | defect | done | fake DNS: the pool refuses at its end, and names match without case (GitHub #349) |
| [T-146](fixes.md) | P2 | S | M5 | defect | done | seccomp: `connect(AF_UNSPEC)` dissolves the association of a datagram socket (GitHub #350) |
| [T-147](fixes.md) | P2 | S | M5 | defect | done | seccomp: a connected datagram socket takes datagrams from its peer only (GitHub #350) |
| [T-148](fixes.md) | P3 | S | M5 | defect | done | seccomp: datagram and ICMP sockets keep their IP-level options (GitHub #350) |
| [T-149](fixes.md) | P3 | S | M5 | research | done | seccomp: decide and document the ICMP error and PMTU limit of the UDP relay (GitHub #350) |
| [T-150](pods.md) | P3 | S | M7 | feature | done | seccomp: an accepted inbound connection goes to the program with no relay (GitHub #351) |
| [T-151](pods.md) | P3 | M | M7 | measurement | done | seccomp: measure the relay throughput, then replace the copy with splice (GitHub #351) |
| [T-152](pods.md) | P3 | S | M7 | research | done | netns: measure in-namespace netfilter on the target hosts (GitHub #352) |
| [T-153](later.md) | P3 | L | M9 | feature | done | netns: a backend with an in-namespace netfilter data plane (GitHub #352) |
| [T-154](fixes.md) | P2 | M | M5 | chore | done | seccomp: fuzz targets for the notification handlers (GitHub #353) |
| [T-155](fixes.md) | P3 | M | M5 | chore | done | seccomp: a differential check of the socket calls against the host kernel (GitHub #353) |
| [T-156](pods.md) | P2 | M | M7 | feature | done | pod: a `reject` action in `-r`, for TCP and for datagrams (GitHub #354) |
| [T-157](pods.md) | P2 | M | M7 | feature | done | pod: `--default-action` and a JSON flow record (GitHub #355) |
| [T-158](pods.md) | P3 | M | M7 | feature | done | pod: `--learn` and `--fail-on-violation` (GitHub #355) |
| [T-159](fixes.md) | P3 | S | M5 | docs | done | research documents are readings; fix the stale counts of the record (GitHub #356) |
| [T-160](fixes.md) | P2 | S | M5 | defect | done | tunnel verify: a name that this host cannot resolve yet keeps the tunnel up (GitHub #357) |
| [T-161](crates.md) | P2 | M | M3 | feature | blocked | relay node: renew the pair before it expires (GitHub #358) |
| [T-162](crates.md) | P3 | M | M3 | feature | done | relay: a host that stalls is tried last on the next connect (GitHub #359) |
| [T-163](crates.md) | P2 | M | M3 | feature | done | relay: the session budget comes from the pool document (GitHub #360) |
| [T-164](fixes.md) | P3 | S | M5 | docs | done | Document `--no-verify` in the README, and say what it does not do (GitHub #361) |
| [T-165](later.md) | P3 | M | M9 | feature | done | The SSH forward-relay leg: supervise the session after `run()` returns (GitHub #362) |
| [T-166](later.md) | P3 | M | M9 | feature | done | The SSH relay leg: the relay, the user and the URL rule are data (GitHub #363) |
| [T-167](services.md) | P2 | M | M6 | feature | done | `net socksify`: route chosen destinations, not only the tailnet or all (GitHub #364) |
| [T-168](services.md) | P2 | S | M6 | feature | done | A passwd shim for any client on a host with no `/etc/passwd` (GitHub #365) |
| [T-169](fixes.md) | P2 | S | M5 | defect | done | `tunnel`: try the MASQUE door of the host's address family first, and take `--masque-endpoint` (GitHub #366) |
| [T-170](fixes.md) | P2 | S | M5 | defect | done | Quick-tunnel provisioning does not retry a DNS failure that cannot recover (GitHub #367) |
| [T-171](fixes.md) | P2 | S | M5 | defect | done | The verify loop names the remedy when the public name does not resolve (GitHub #367) |
| [T-172](later.md) | P3 | M | M9 | feature | done | A pin of the new public name while the process runs (GitHub #367) |
| [T-173](fixes.md) | P2 | S | M5 | feature | done | A WARP identity of another uid, with a consent variable (GitHub #368) |
| [T-174](pods.md) | P3 | M | M7 | feature | done | netns: a veth wire when `/dev/net/tun` is not there (GitHub #369) |
| [T-175](crates.md) | P2 | M | M3 | feature | done | DoH: keep one connection for each resolver server (an ask of pod-dns and pod-proxy) (GitHub #370) |
| [T-176](later.md) | P3 | S | M9 | research | done | Guest-wire router on smoltcp: the constraints of the TCP terminator and the UDP socket (GitHub #371 and #372) |
| [T-177](services.md) | P3 | S | M6 | feature | done | Router: `podnet route check`, a dry run of the rules (GitHub #375) |
| [T-178](later.md) | P3 | M | M9 | feature | done | Router: named group policies (`rr`, `minrtt`, `leastload`, `protect`) (GitHub #375) |
| [T-179](later.md) | P3 | S | M9 | research | done | Research: a direction split for asymmetric uplinks (GitHub #375) |
| [T-180](services.md) | P3 | M | M6 | feature | done | Router: write-side control of the members (select, rotate, reconnect, interval) (GitHub #376) |
| [T-181](pods.md) | P3 | M | M7 | feature | done | seccomp: a budget of tracked flows with counters, a documented ceiling and a doctor row (GitHub #376) |
| [T-182](fixes.md) | P2 | M | M5 | defect | done | `doctor --agent`: a row is `ok` only when the index host and the file host answer (GitHub #378) |
| [T-183](fixes.md) | P2 | S | M5 | defect | done | One port for an `http://` proxy URL with no port in podnet and podssh (GitHub #379) |
| [T-184](fixes.md) | P2 | S | M5 | defect | done | `doctor`: the relay row follows `--relay` and keeps a loopback relay (GitHub #380) |
| [T-185](fixes.md) | P1 | M | M5 | defect | done | A deadline for the client handshake of `net proxy`, `net tailscale` and `pod --serve` (GitHub #384) |
| [T-186](fixes.md) | P1 | S | M5 | defect | done | `net tailscale` and the other unix listeners claim the socket path first (GitHub #385) |
| [T-187](fixes.md) | P2 | M | M5 | defect | done | `net proxy --forward`: a real target may be a name, and the dial reports its hop (GitHub #386) |
| [T-188](fixes.md) | P2 | S | M5 | defect | done | `net proxy --forward` reports a failed bind before it prints the forward (GitHub #387) |
| [T-189](fixes.md) | P2 | S | M5 | defect | done | The switch counts each packet that it drops (GitHub #388, part 1) |
| [T-190](fixes.md) | P2 | M | M5 | defect | done | A stack over a switch link has bounded queues (GitHub #388, part 2) |
| [T-191](fixes.md) | P3 | S | M5 | defect | done | The listener table of `net doctor` names each command that listens (GitHub #389, part 1) |
| [T-192](fixes.md) | P3 | S | M5 | defect | done | A refused TCP bind exits 69 in each listener command (GitHub #389, part 2; GitHub #385 step 5) |
| [T-193](fixes.md) | P2 | M | M5 | defect | done | `net proxy --map` fills the gateway resolver and the shim table (GitHub #390) |
| [T-194](fixes.md) | P2 | S | M5 | defect | done | A relay `close` aborts the session task of the node (GitHub #391) |
| [T-195](fixes.md) | P2 | S | M5 | defect | done | `relay stop` checks the pair before it stops the local node (GitHub #392) |
| [T-196](fixes.md) | P2 | M | M5 | defect | done | The relay node drains and waits after a local end of stream (GitHub #393) |
| [T-197](fixes.md) | P2 | S | M5 | defect | done | The pair file is always mode 0600, and `load` checks the file (GitHub #394) |
| [T-198](fixes.md) | P3 | S | M5 | docs | done | `docs/protocol.md` and the test agree on the number of seed alternates (GitHub #395) |
| [T-199](fixes.md) | P2 | M | M5 | defect | done | `relay connect --listen` has the instance surface of `connect -L` (GitHub #396) |
| [T-200](crates.md) | P2 | M | M3 | feature | blocked | The reverse road of the relay uses the alternate hosts (GitHub #397) |
| [T-201](fixes.md) | P3 | S | M5 | defect | done | `WebSocketTunnel` dials through the egress layer and parses IPv6 (GitHub #398) |
| [T-202](fixes.md) | P3 | S | M5 | defect | done | `serve` of a directory with no `index.html` logs no WARN for the 404 at `/` (GitHub #399) |
| [T-203](fixes.md) | P2 | S | M5 | defect | done | The ready line of `share --json` has `schema` and `podnet` (GitHub #400) |
| [T-204](fixes.md) | P2 | S | M5 | defect | done | `tunnel --config` refuses the origin flags (GitHub #401) |
| [T-205](fixes.md) | P3 | S | M5 | defect | done | A nested detached start does not inherit `PODNET_DETACH_SUPERVISED` (GitHub #402) |
| [T-206](fixes.md) | P2 | S | M5 | feature | done | `agent status --json` (GitHub #403) |
| [T-207](fixes.md) | P2 | S | M5 | defect | done | The library static path decodes `+` as a literal plus (GitHub #404) |
| [T-208](fixes.md) | P2 | S | M5 | defect | done | `warp up --tun` forwards `--replace` and refuses the flags it cannot forward (GitHub #405) |
| [T-209](fixes.md) | P2 | S | M5 | defect | done | `warp status` names the `warp-tun` instance (GitHub #406) |
| [T-210](fixes.md) | P3 | S | M5 | defect | blocked | The WARP+ label names the paid account types (GitHub #407) |
| [T-211](fixes.md) | P2 | S | M5 | defect | done | `warp down` and `warp up --persist` own the fish file only when podnet wrote it (GitHub #408) |
| [T-212](fixes.md) | P3 | S | M5 | docs | done | The README states that the rules apply to `warp proxy --l4` (GitHub #409) |
| [T-213](fixes.md) | P2 | M | M5 | defect | done | The WireGuard endpoint of a device record has one derivation (GitHub #410) |
| [T-214](fixes.md) | P3 | S | M5 | defect | done | `warp ssh-config` prints a `ProxyCommand` with the same quote as `--write` (GitHub #411) |
| [T-215](fixes.md) | P2 | S | M5 | defect | done | `warp proxy --dns` has a bound and a counter (GitHub #412) |
| [T-216](fixes.md) | P1 | S | M5 | feature | done | Freeze `doctor --json` as schema 1, with `--quick` (PodUtils/podcrates#35, A-N-04) |
| [T-217](fixes.md) | P1 | S | M5 | feature | done | `doctor --json`: a fixed block for the egress lanes (PodUtils/podcrates#35, A-C-10) |
| [T-218](rename.md) | P0 | M | M1 | release | done | A connector profile that holds within a minor version (PodUtils/podcrates#35, A-N-05) |
| [T-219](services.md) | P1 | M | M6 | feature | blocked | `podnet service install`: a hardened system unit (PodUtils/podcrates#35, A-N-11) |
| [T-220](services.md) | P1 | S | M6 | feature | blocked | `relay node` as a system service, with no operator act (PodUtils/podcrates#35, A-N-12) |
| [T-221](services.md) | P2 | M | M6 | feature | blocked | `podnet bench URL --json` (PodUtils/podcrates#35, A-N-15) |
| [T-222](fixes.md) | P2 | S | M5 | feature | done | `llms.txt`, `llms-full.txt` and `--help --json` (PodUtils/podcrates#35, A-X-19) |
| [T-223](fixes.md) | P3 | M | M5 | defect | done | seccomp: the TCP gaps that the differential check finds |
| [T-224](crates.md) | P1 | S | M3 | chore | done | podnet-netns calls `pod_proxy::udp` for the SOCKS5 datagram path |
| [T-225](fixes.md) | P1 | S | M5 | defect | done | The send poller of the WARP QUIC socket keeps its wake-up |
| [T-226](fixes.md) | P1 | S | M5 | defect | done | The netns backend moves a large TCP download at full speed with a bounded buffer |
| [T-227](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.2`: the dry run, the publish and the release proofs |
| [T-228](fixes.md) | P1 | S | M5 | defect | done | Each sender that waits on the WARP QUIC socket gets its wake-up |
| [T-229](pods.md) | P2 | M | M7 | research | done | Measure `podnet pod` with the official runner and three CI payloads |
| [T-230](pods.md) | P3 | S | M7 | feature | done | `pod doctor --json` states the hold level and what the seccomp backend does not hold |
| [T-231](pods.md) | P3 | S | M7 | feature | done | `podnet pod` refuses a run with no hold, and can apply the rules to loopback |
| [T-232](crates.md) | P3 | S | M3 | chore | done | The hand-off of the generic hold of the pod backends to `pod-hold` |
| [T-233](pods.md) | P2 | S | M5 | defect | done | seccomp: a dual-stack connect to the IPv4-mapped form of a fake address goes to the proxy as an address |
| [T-234](pods.md) | P2 | S | M5 | defect | done | netns: `parse_tcp_syn` reads a SYN in a header under 20 bytes and in a later fragment |
| [T-235](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.3`: the dry run, the publish and the release proofs |
| [T-236](services.md) | P2 | S | M5 | defect | done | `serve --ingress relay --detach` and `git serve --ingress relay --detach` report ready before the node is connected |
| [T-237](adapters.md) | P3 | M | M4 | feature | done | Front door: an opt-in `--early-reply` answers the client before the upstream connect and pipelines the SOCKS5 upstream handshake (GitHub #428) |
| [T-238](fixes.md) | P2 | S | M5 | defect | done | `listen::bind_private`: bind in a private 0700 directory, chmod, then rename into place, so the socket never has a wider mode under its name (GitHub #429) |
| [T-239](fixes.md) | P2 | M | M5 | defect | done | `install.sh` checks the release tag and reads SHA256SUMS strictly, `podnet update` does the same, and the install tests assert the downloader flags (GitHub #430) |
| [T-240](fixes.md) | P2 | M | M5 | defect | done | `warp tun --default-route`: keep the resolver backup on disk, restore it after a kill, set the mode of `resolv.conf`, and test the route table with `ip route get` (GitHub #431) |
| [T-241](fixes.md) | P2 | M | M5 | defect | done | `warp`: register under a lock, save the device before the enrolment step, and fail over the registration POST only when no response came (GitHub #432) |
| [T-242](services.md) | P3 | M | M6 | feature | done | Rules: warn when an earlier domain or cidr rule shadows a later one, and fail when the shadowed rule is a `reject` (GitHub #433) |
| [T-243](overlays.md) | P3 | M | M8 | feature | done | `net tailscale`: `--tailnet NAME` refuses to start or to dial when the daemon is in another tailnet, a doctor row shows the status, and the docs name `TS_FORCE_NOISE_443` (GitHub #434) |
| [T-244](later.md) | P3 | L | M9 | feature | blocked | `tunnel --inspect`: capture each exchange beside the copy helpers of `forward_http`, keep a ring, redact by default, replay over a 0600 socket (GitHub #435) |
| [T-245](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.4`: the dry run, the publish and the release proofs |
| [T-246](overlays.md) | P3 | S | M8 | feature | done | NetBird: the network spec `netbird:[PORT]` over the SOCKS5 listener of netstack mode |
| [T-247](overlays.md) | P3 | M | M8 | measurement | blocked | NetBird: measure netstack mode on a proxy-only host |
| [T-248](later.md) | P3 | L | M9 | feature | done | Router: a guest-wire router that ends guest TCP on smoltcp |
| [T-249](pods.md) | P2 | M | M7 | chore | done | netns: the router of the namespace runs on the interface of `pod-netstack` |
| [T-250](pods.md) | P2 | S | M7 | chore | done | CI: a root step that runs the netns tests that skip on a runner |
| [T-251](services.md) | P2 | S | M6 | feature | done | Transfer: the hosted magic-wormhole mailboxes on 443 as the default, with a choice that the code carries |
| [T-252](services.md) | P3 | M | M6 | feature | done | Transfer: a piping-server relay as a transit carrier of last resort |
| [T-253](services.md) | P3 | S | M6 | measurement | done | Live: probe the hosted mailboxes and relays from a proxy-only runner |
| [T-254](fixes.md) | P2 | M | M5 | chore | done | Housekeeping: one footnote for cfrs, no narrative history |
| [T-255](pods.md) | P2 | S | M7 | defect | done | seccomp: an IPv4-mapped loopback address is loopback |
| [T-256](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.5`: the dry run, the publish and the release proofs |
| [T-257](pods.md) | P2 | S | M7 | defect | done | doctor: the probes of the namespace allocate in a fork child |
| [T-258](fixes.md) | P3 | S | M5 | docs | done | Docs: stale or misleading comments and records (GitHub #459) |
| [T-259](fixes.md) | P2 | M | M5 | docs | done | The user documentation by topic: a short README and one document for each area |
| [T-260](fixes.md) | P2 | M | M5 | feature | done | doctor: probes for the lanes `masque_udp443` and `ws_upgrade` (GitHub #461, A-C-10) |
| [T-261](link.md) | P3 | M | M9 | feature | done | link: an Ed25519 key for each install, its fingerprint and an allow list (GitHub #466) |
| [T-262](link.md) | P3 | L | M9 | feature | done | link: the session of streams with credit flow control and keepalive (GitHub #466) |
| [T-263](link.md) | P3 | M | M9 | feature | done | link: the 10-minute resume of a session through pod-session (GitHub #466) |
| [T-264](link.md) | P3 | M | M9 | feature | done | link: a registry of carriers with an available() reason (GitHub #466) |
| [T-265](link.md) | P3 | M | M9 | feature | done | link: the commands listen, dial, status, close and carriers (GitHub #466) |
| [T-266](link.md) | P3 | M | M9 | feature | done | link: `peer:FINGERPRINT/SERVICE` as a network target and `forward -L LISTEN=peer:...` (GitHub #466 and #471) |
| [T-267](link.md) | P3 | M | M9 | feature | done | link: the control socket, its discipline and the status verb (GitHub #466) |
| [T-268](link.md) | P3 | L | M9 | feature | done | link: the TCP carrier with TLS 1.3, the accept side and the peer handshake (GitHub #467) |
| [T-269](link.md) | P3 | M | M9 | feature | done | link: the WebSocket carrier, its server half and its wrapper (GitHub #467) |
| [T-270](link.md) | P3 | M | M9 | feature | done | link: the HTTP/2 carrier with one bidirectional POST (GitHub #467) |
| [T-271](link.md) | P3 | M | M9 | feature | done | link: the HTTP/1.1 long-poll carrier (GitHub #467) |
| [T-272](link.md) | P3 | M | M9 | feature | done | link: the SSE and chunked-stream carrier (GitHub #467) |
| [T-273](link.md) | P3 | M | M9 | feature | done | link: the QUIC carrier (GitHub #467) |
| [T-274](link.md) | P3 | S | M9 | feature | done | link: the unix-socket carrier and its socketpair half (GitHub #467) |
| [T-275](link.md) | P3 | S | M9 | feature | done | link: the in-process memory carrier (GitHub #467) |
| [T-276](link.md) | P3 | L | M9 | feature | done | link: the UDP carrier with KCP (GitHub #468) |
| [T-277](link.md) | P3 | L | M9 | feature | done | link: a selective-repeat ARQ for any lossy carrier (GitHub #468) |
| [T-278](link.md) | P3 | L | M9 | feature | done | link: the DNS simplex carrier (GitHub #468) |
| [T-279](link.md) | P3 | L | M9 | feature | done | link: the object-storage simplex carrier (GitHub #468) |
| [T-280](link.md) | P3 | M | M9 | feature | done | link: the HTTP poll simplex carrier (GitHub #468) |
| [T-281](link.md) | P3 | L | M9 | feature | done | link: the DNS stream carrier with a pipelined query window (GitHub #469) |
| [T-282](link.md) | P3 | L | M9 | feature | done | link: the ICMP echo carrier (GitHub #469) |
| [T-283](link.md) | P3 | M | M9 | feature | blocked | link: a WireGuard peer as a carrier (GitHub #469) |
| [T-284](link.md) | P3 | L | M9 | feature | blocked | link: NAT traversal with a relay fallback (GitHub #469) |
| [T-285](link.md) | P3 | L | M9 | feature | done | link: signed route announcements (GitHub #470) |
| [T-286](link.md) | P3 | M | M9 | feature | done | link: the path-vector route table (GitHub #470) |
| [T-287](link.md) | P3 | L | M9 | feature | done | link: multi-hop forwarding of a session stream (GitHub #470) |
| [T-288](link.md) | P3 | M | M9 | feature | done | link: load balancing across link groups (GitHub #470) |
| [T-289](link.md) | P3 | M | M9 | feature | done | link: the local topology and the status view of a mesh (GitHub #470) |
| [T-290](link.md) | P3 | M | M9 | feature | done | link: per-hop access control for services and forwarding (GitHub #470) |
| [T-291](link.md) | P3 | M | M9 | feature | done | link: a SOCKS5 service at a peer (GitHub #471) |
| [T-292](link.md) | P3 | M | M9 | feature | done | link: an HTTP forward proxy service at a peer (GitHub #471) |
| [T-293](link.md) | P3 | S | M9 | feature | done | link: a raw passthrough service at a peer (GitHub #471) |
| [T-294](link.md) | P3 | M | M9 | feature | blocked | link: a Shadowsocks AEAD service at a peer (GitHub #471) |
| [T-295](link.md) | P3 | M | M9 | feature | blocked | link: a Trojan service at a peer (GitHub #471) |
| [T-296](link.md) | P3 | M | M9 | feature | done | link: an exec adapter that runs a program for each stream (GitHub #471) |
| [T-297](link.md) | P3 | M | M9 | feature | done | link: serve a file tree at a peer (GitHub #471) |
| [T-298](link.md) | P3 | L | M9 | feature | done | link: the AEAD record layer with negotiated suite and rekey (GitHub #472) |
| [T-299](link.md) | P3 | S | M9 | feature | done | link: snappy compression of records (GitHub #472) |
| [T-300](link.md) | P3 | S | M9 | feature | done | link: padding of records to buckets (GitHub #472) |
| [T-301](link.md) | P3 | M | M9 | feature | done | link: the replay window and the RESUME record (GitHub #472) |
| [T-302](link.md) | P3 | L | M9 | feature | done | link: split one session across an up carrier and a down carrier (GitHub #473) |
| [T-303](link.md) | P3 | M | M9 | feature | done | link: chain composition with a peer element and --check (GitHub #473) |
| [T-304](link.md) | P3 | L | M9 | feature | done | link: a C ABI with a header, a shared library and a static library (GitHub #474) |
| [T-306](link.md) | P3 | M | M9 | feature | done | link: the query keys of a carrier URL (GitHub #476) |
| [T-307](link.md) | P3 | M | M9 | feature | done | link: the write verbs of the control plane (GitHub #476) |
| [T-308](link.md) | P3 | S | M9 | feature | done | link: names for live links (GitHub #476) |
| [T-309](link.md) | P3 | M | M9 | feature | done | link: the redacted state dump and bug-report --include-link (GitHub #476) |
| [T-310](link.md) | P3 | M | M9 | feature | done | link: the component inventory command (GitHub #476) |
| [T-311](link.md) | P3 | M | M9 | feature | done | link: Cargo features for the link carriers and services (GitHub #476) |
| [T-312](fixes.md) | P2 | S | M5 | defect | done | tests/signals.rs: the test sends a stop signal before the ready of the command (flaky under load) |
| [T-313](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.6`: the dry run, the publish and the release proofs |
| [T-314](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.7`: the dry run, the publish and the release proofs |
| [T-315](fixes.md) | P1 | M | M5 | defect | done | live: `relay connect` hangs on the runner after the take of pod-relay (GitHub #483) |
| [T-316](fixes.md) | P2 | L | M5 | feature | partial | A local stand-in of the quick-tunnel service and the edge, so that CI runs the quick tunnel with no call to Cloudflare |
| [T-317](pods.md) | P1 | S | M7 | defect | done | seccomp: a mapped loopback address is dialed direct, not through the chain (PodUtils/podrunner#69) |
| [T-318](fixes.md) | P2 | M | M5 | defect | done | ssh-relay: try the authentication methods the relay offers, and verify an RSA host key (GitHub #505) |
| [T-319](fixes.md) | P2 | S | M5 | feature | done | ssh-relay: a remote port from a flag, and from the service data (GitHub #506) |
| [T-320](later.md) | P3 | L | M9 | feature | done | net: a Dev Tunnels network for TCP streams (GitHub #503) |
| [T-321](later.md) | P3 | L | M9 | feature | partial | net: a TURN network for UDP flows (GitHub #507) |
| [T-322](fixes.md) | P2 | S | M5 | defect | done | doctor: resolve the ssh-relay probe through the configured proxy (GitHub #504) |
| [T-323](fixes.md) | P2 | S | M5 | defect | done | live: the WebSocket step leaves its echo server, and waits 30 s for the DNS of a new name (GitHub #511) |
| [T-324](fixes.md) | P3 | S | M5 | defect | done | doctor: the test of a refused direct dial holds its port |
| [T-325](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.8`: the dry run, the publish and the release proofs |
| [T-326](fixes.md) | P2 | S | M5 | chore | done | Remove the ignore of RUSTSEC-2023-0071 when russh takes `rsa` 0.10.0 |
| [T-327](services.md) | P3 | M | M6 | feature | done | Router: add and remove a door and a network while the router runs (N-1) |
| [T-328](services.md) | P3 | S | M6 | feature | done | Router: rules by port, also on the loopback (N-2) |
| [T-329](services.md) | P3 | M | M6 | feature | done | `podnet exit --stdio`: a SOCKS5 exit on stdin and stdout (N-3) |
| [T-330](pods.md) | P3 | S | M7 | feature | done | `podnet pod --report FILE`: a JSON report of the backend and the hold (N-5) |
| [T-331](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.9`: the dry run, the publish and the release proofs |
| [T-332](fixes.md) | P1 | S | M5 | defect | done | QUIC over WARP after the take of pod-warp: the socket binds one address family (GitHub #523) |
| [T-333](fixes.md) | P1 | S | M5 | defect | done | link door: --resolve-at local tries only the first address of a name |
| [T-334](link.md) | P3 | S | M9 | feature | done | pod: the seccomp rule of a raw ICMP socket (GitHub #469) |
| [T-335](link.md) | P3 | S | M9 | feature | done | link: keep the routes of two next hops for one announcement (GitHub #470) |
| [T-336](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.10`: the dry run, the publish and the release proofs |
| [T-337](overlays.md) | P3 | S | M8 | feature | done | WireGuard: take the preshared key of `pod-wg` |
| [T-338](services.md) | P3 | S | M6 | feature | done | `podnet send --words N`: the count of words of a code |
| [T-339](fixes.md) | P2 | S | M5 | chore | done | Fuzz targets for the parsers of peer and file input that came after 2026-11-12 |
| [T-340](fixes.md) | P2 | S | M5 | chore | done | The parity checklist after the take of pod-warp, the router, the transfer and the carriers |
| [T-341](fixes.md) | P1 | S | M5 | defect | done | podnet receive: a directory can land more bytes and entries than the offer that the person accepted |
| [T-342](fixes.md) | P1 | S | M5 | defect | done | wg serve: a peer can send with the address of another peer when their AllowedIPs overlap |
| [T-343](fixes.md) | P2 | S | M5 | defect | done | link record: a middle node makes an end take 16 MiB for each record header that it sends |
| [T-344](fixes.md) | P2 | S | M5 | defect | done | podnet receive and send: the other end of a transfer can make this end dial thousands of hosts |
| [T-345](fixes.md) | P2 | M | M5 | defect | done | wg serve: one peer can take each flow of the server from the other peers |
| [T-346](fixes.md) | P2 | M | M8 | defect | done | link mesh: a neighbour can claim a direct link to each origin that it hears, and attract its routes |
| [T-347](fixes.md) | P3 | M | M5 | defect | done | Hardening: the low findings of the security review of 2026-11-14 |
| [T-348](fixes.md) | P1 | S | M5 | defect | done | capnp: a list in a segment that does not exist panics the reader of a message of the edge |
| [T-349](fixes.md) | P1 | S | M5 | defect | done | proxy: a password in the URL form of a Unix proxy shows in each line that names the proxy |
| [T-350](fixes.md) | P2 | S | M5 | defect | done | pod: a packet socket opens in a seccomp pod, and the raw rule of T-334 is in the supervisor |
| [T-351](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.11`: the dry run, the publish and the release proofs |
| [T-352](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.12`: the dry run, the publish and the release proofs |
| [T-353](link.md) | P3 | S | M9 | defect | done | link: a closed half can end the listener of a split session with "the carrier of the link ended" |
| [T-354](later.md) | P3 | S | M9 | feature | done | net: `devtunnel://` with a tunnel access token, or with no token |
| [T-355](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.13`: the dry run, the publish and the release proofs |
| [T-356](fixes.md) | P2 | S | M5 | defect | done | vnet shim: an IPv4-mapped IPv6 address gets the IPv4 abstract name |
| [T-357](fixes.md) | P2 | S | M5 | feature | done | vnet shim: `PODNET_VNET_FORWARD` sends a TCP destination to a unix socket path |
| [T-358](fixes.md) | P1 | L | M5 | chore | partial | The release gate: each beta passes a scenario run of its own binary on restricted hosts |
| [T-359](fixes.md) | P2 | M | M3 | chore | done | One shared crate for the help and the manual of each PodUtils command line |
| [T-360](fixes.md) | P2 | S | M5 | defect | done | install.sh: a second run with the same version changes nothing and says so |
| [T-361](fixes.md) | P2 | S | M5 | defect | done | git init: make the root when it does not exist |
| [T-362](fixes.md) | P3 | S | M5 | defect | done | docs/cli.md: the completion command writes to a directory that can be absent |
| [T-363](fixes.md) | P2 | S | M5 | defect | done | net passwd-shim: the home of the entry is $HOME, not /state/home |
| [T-364](fixes.md) | P3 | S | M5 | measurement | done | live: two runs in a row failed at the quick tunnel; the rerun passed (GitHub #516) |
| [T-365](deferred.md) | P3 | L | M9 | feature | done | Deferred: carry a session in NTPv4 extension fields, SMTP and IMAP (GitHub #484) |
| [T-366](deferred.md) | P3 | L | M9 | feature | done | Deferred: carry a session through a TURN allocation, with RFC 6062 (GitHub #485) |
| [T-367](deferred.md) | P3 | L | M9 | feature | partial | Deferred: close four QUIC gaps: MASQUE enrolment, HTTP/3 shape, version, connection ID (GitHub #486) |
| [T-368](deferred.md) | P3 | L | M9 | feature | blocked | Deferred: register third-party services as roads, with a watch, a date and a count (GitHub #489) |
| [T-369](deferred.md) | P3 | L | M9 | feature | done | Deferred: switch after a real handshake, with a cover script and multiplexed cover (GitHub #491) |
| [T-370](deferred.md) | P3 | L | M9 | feature | done | Deferred: shape the record layer: padding profiles, entropy, first flight, pacing (GitHub #493) |
| [T-371](deferred.md) | P3 | L | M9 | feature | done | Deferred: shape the application: deferred responses, a generator, carriers in the payload (GitHub #494) |
| [T-372](deferred.md) | P3 | L | M9 | feature | done | Deferred: add a packet sidecar: desync search, disagreement probe, fake TCP, bonding (GitHub #495) |
| [T-373](deferred.md) | P3 | L | M9 | feature | done | Deferred: keep the session end to end: opaque destination and keys from the ends (GitHub #496) |
| [T-374](deferred.md) | P3 | L | M9 | feature | blocked | Deferred: make the pool live, the budget per deployment, tokens expiring, hosts movable (GitHub #497) |
| [T-375](deferred.md) | P3 | L | M9 | feature | blocked | Deferred: resume a session across hours, and serve the full service surface at a peer (GitHub #498) |
| [T-376](deferred.md) | P3 | L | M9 | feature | done | Deferred: carry a session with no network: serial, HF, spool, discovery, pairing (GitHub #499) |
| [T-377](deferred.md) | P3 | L | M9 | feature | done | Deferred: state the host runtime: no open port, classified egress, stated limits (GitHub #500) |
| [T-378](deferred.md) | P3 | L | M9 | feature | done | Deferred: build the verification harness: classifier, exit codes, conformance matrix (GitHub #501) |
| [T-379](fixes.md) | P1 | S | M5 | defect | done | A unix listener of podnet fails with EXDEV in a Landlock domain |
| [T-380](fixes.md) | P1 | S | M5 | defect | done | link listen over the unix carrier fails with EXDEV in a Landlock domain (pod-link) |
| [T-381](fixes.md) | P2 | S | M5 | defect | done | The netns backend of podnet pod needs an AF_INET datagram socket (pod-hold) |
| [T-382](fixes.md) | P1 | S | M5 | defect | done | podnet pod takes the netns backend where each TCP connect of the program is refused (Landlock) |
| [T-383](fixes.md) | P2 | M | M5 | defect | done | podnet pod: no name resolves when the program cannot make a UDP socket (udp-denied) |
| [T-384](fixes.md) | P3 | S | M5 | defect | done | podnet pod --backend netns-kernel reads the ports of its listeners with getsockname (getsockname-denied) |
| [T-385](crates.md) | P2 | S | M3 | chore | done | Take podcrates v0.58.0 and remove the glue of the gaps of pod-manual |
| [T-386](crates.md) | P2 | S | M3 | chore | done | Take podcrates v0.59.0 (the fixes of pod-link and pod-hold for T-380 and T-381) |
| [T-387](crates.md) | P2 | S | M3 | chore | done | Take podcrates v0.60.0 (the network labs of pod-lab, clap's requires in pod-manual) |
| [T-388](crates.md) | P2 | S | M3 | chore | done | Take podcrates v0.61.0 (a way out of a network lab, the rows that send a packet out) |
| [T-389](crates.md) | P2 | S | M5 | chore | done | The installer core of podcrates v0.61.0: the cosign check of the core |
| [T-390](fixes.md) | P1 | S | M5 | defect | done | vnet shim: fcntl reads the struct flock pointer of the lock commands as an int |
| [T-391](fixes.md) | P1 | S | M5 | defect | done | vnet shim: a bind to the wildcard address gets a name that no peer dials |
| [T-392](fixes.md) | P2 | M | M5 | defect | done | vnet shim: answer netlink route dumps with the virtual interfaces |
| [T-393](crates.md) | P2 | S | M3 | chore | done | Take podcrates v0.62.0 (the servers of the command in a network lab, the resume line of pod-session) |
| [T-394](crates.md) | P2 | S | M3 | chore | done | Take podcrates v0.63.0 and its installer core (one signature check for the copy of the tree) |
| [T-395](fixes.md) | P2 | S | M5 | defect | done | live: the check of agent up fails when the resolver of the runner caches NXDOMAIN for the new name |
| [T-396](link.md) | P3 | S | M9 | chore | done | link: remove the entry 305 and the empty feature `portable` (GitHub #475) |
| [T-397](fixes.md) | P2 | S | M5 | defect | done | live: two quick tunnels over WARP within a minute register two devices, and WARP answers HTTP 429 |
| [T-398](fixes.md) | P2 | M | M5 | chore | done | The release gate: a scenario for each experimental command |
| [T-399](fixes.md) | P2 | S | M5 | defect | done | live: the WebSocket client of a --url origin dials a name that has no IPv4 answer yet |
| [T-400](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.14`: the gate, the dry run, the publish and the release proofs |
| [T-401](fixes.md) | P2 | S | M5 | defect | done | --detach and autostart stop on a host with no /proc: podnet cannot find its own file |
| [T-402](fixes.md) | P2 | M | M5 | defect | done | a quick tunnel over quic+warp does not recover when its MASQUE session closes |
| [T-403](later.md) | P3 | M | M9 | feature | done | link: an exec service with the exit status and stderr of its program (`exec-status:`, `link dial --status`) (GitHub #600) |
| [T-404](later.md) | P3 | M | M9 | feature | done | forward: a name table (`--name NAME=HOST:PORT`, `--alias-port`) with an address of 127.0.0.0/8 for each name and the DNS door (GitHub #601) |
| [T-405](later.md) | P3 | L | M9 | feature | done | route: a rate, a quota, a period and `on_exhaust` for each door, with the usage kept across a restart (GitHub #602) |
| [T-406](fixes.md) | P2 | S | M5 | defect | blocked | live: the relay keeps a lost socket of a node for minutes after a reset (409), and SSH through it hangs |
| [T-407](fixes.md) | P2 | S | M5 | defect | done | podnet connect resets a local connection when one WebSocket dial fails |
| [T-408](fixes.md) | P2 | S | M5 | defect | done | a detached tunnel stops at 60 s while the check of its new public name may take 180 s |
| [T-409](crates.md) | P1 | S | M3 | chore | done | Take podcrates v0.64.0 |
| [T-410](crates.md) | P1 | S | M3 | chore | done | Take podcrates v0.65.0; a node that the relay holds by its lost socket takes the next pair |
| [T-411](fixes.md) | P2 | S | M5 | defect | done | live: speed.cloudflare.com answers HTTP 429 to the shared address of WARP, and the rate check fails |
| [T-412](crates.md) | P1 | S | M3 | chore | done | Take podcrates v0.66.0 |
| [T-413](fixes.md) | P2 | S | M5 | defect | done | live: warp scan gets no answer from a front door once, and the step fails |
| [T-414](fixes.md) | P2 | S | M5 | defect | partial | live: after a kill of its MASQUE connection, the WARP proxy opens no new session in 60 s |
| [T-415](fixes.md) | P2 | S | M5 | defect | partial | live: the first request through the WARP proxy gets a reset 5 s after the start |
| [T-416](crates.md) | P1 | S | M3 | chore | done | Take podcrates v0.67.0 |
| [T-417](crates.md) | P2 | S | M3 | chore | done | One durable replace of a private file: podnet's copies call `pod_core::files::replace_private` |
| [T-418](fixes.md) | P2 | S | M5 | defect | done | ssh-relay asks for a remote forward on 0.0.0.0, and localhost.run reads it as a custom domain |
| [T-419](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.15`: the gate, the dry run, the publish and the release proofs |
| [T-420](crates.md) | P1 | S | M3 | chore | done | Take podcrates v0.68.0 |
| [T-421](fixes.md) | P1 | S | M5 | defect | done | The WebSocket and relay connections ignore `--ca-file`, and a proxy's refusal of the upgrade reads as a bad token |
| [T-422](fixes.md) | P1 | S | M5 | release | done | Publish `v0.4.0-beta.16`: the gate, the dry run, the publish and the release proofs |
| [T-423](crates.md) | P1 | M | M3 | chore | done | Take podcrates v0.69.0 with `pod-cloudflare` |
| [T-424](fixes.md) | P2 | S | M6 | defect | done | `git init` of an existing bare repository fails (GitHub #614) |
| [T-425](fixes.md) | P3 | S | M6 | chore | done | `warp ssh-config` prints a `GIT_SSH_COMMAND` line with three layers of quotes (GitHub #615) |
| [T-426](fixes.md) | P3 | S | M6 | docs | done | The unlock flow of a PIN is not documented (GitHub #618) |
| [T-427](fixes.md) | P2 | S | M6 | defect | done | `route` writes `router-usage.json` to the global state directory, not to `state_dir` (GitHub #619) |
| [T-428](fixes.md) | P2 | S | M6 | docs | done | The release binaries have no optional link carrier, and the notes do not say it (GitHub #620) |
| [T-429](fixes.md) | P3 | S | M6 | defect | done | Small output faults of the QA round (GitHub #622) |
| [T-430](fixes.md) | P1 | S | M7 | defect | done | `pod -x` with a dead upstream: a non-blocking connect() looks connected, then hangs (GitHub #610) |
| [T-431](fixes.md) | P2 | S | M7 | docs | done | `pod -r 'cidr:...=reject'` does not refuse a name (GitHub #611) |
| [T-432](fixes.md) | P3 | S | M7 | defect | done | `pod -x direct` prints a false warning (GitHub #621) |
| [T-433](fixes.md) | P3 | S | M7 | defect | done | `net proxy` and a target outside the virtual network (GitHub #609) |
| [T-434](fixes.md) | P2 | S | M6 | defect | done | `serve` drops a client that half-closes before the answer (GitHub #613) |
| [T-435](fixes.md) | P2 | S | M6 | defect | done | `expose ... -- --detach` stays in the foreground (GitHub #612) |
| [T-436](fixes.md) | P3 | S | M6 | defect | done | A syntax error of the command line names `podnet update --check` (GitHub #616) |
| [T-437](fixes.md) | P2 | S | M6 | defect | done | A command that ends at once prints the line "runs in the foreground" (GitHub #617) |
| [T-438](fixes.md) | P3 | S | M6 | defect | done | `exit` needs `--stdio`, its only mode (GitHub #623) |
| [T-439](fixes.md) | P3 | S | M6 | defect | done | `autostart run` needs `--dir`, whose default is known (GitHub #624) |
| [T-440](fixes.md) | P3 | S | M6 | defect | done | `forward --name` needs `--alias-port` (GitHub #625) |
| [T-441](fixes.md) | P3 | S | M6 | feature | done | `warp env` gives nothing for a unix socket of `warp up` (GitHub #626) |
| [T-442](fixes.md) | P3 | S | M6 | defect | done | `warp scan` names a command that does not exist (GitHub #627) |
| [T-443](fixes.md) | P3 | S | M6 | feature | done | The output of `serve --pin` does not say how to unlock (GitHub #628) |
| [T-444](fixes.md) | P3 | S | M6 | feature | done | `serve --list --tar`: the listing has no tar link (GitHub #629) |
| [T-445](fixes.md) | P3 | S | M6 | feature | done | `autostart`: no `--json` for `list`, no `--manual` for a host with no manager (GitHub #630) |
| [T-446](fixes.md) | P3 | S | M6 | feature | done | `relay node` has no `--json` (GitHub #631) |
| [T-447](fixes.md) | P3 | S | M6 | feature | done | `ssh-relay` has no `--run-for`, and `-q` prints the banner (GitHub #632) |
| [T-448](fixes.md) | P3 | S | M6 | feature | done | No way to add a user to the users file of `serve` (GitHub #633) |
| [T-449](fixes.md) | P1 | S | M5 | defect | done | live: four checks read the log lines of the tunnel before the take of `pod-cloudflare` |
| [T-450](fixes.md) | P1 | S | M5 | chore | done | Rule: an issue closes only with a test that reproduces it |
| [T-451](fixes.md) | P1 | M | M5 | chore | done | Contract tests from the manual: each flag of the help exists, each example parses, each JSON document has one schema |
| [T-452](fixes.md) | P1 | M | M5 | chore | done | Error-path rows of the lane: dead, refused, rejected, unreachable, no DNS |
| [T-453](fixes.md) | P1 | S | M5 | chore | done | A lab of the host class of the testers: no AF_INET bind, no netns, no write of the memory of a program |
| [T-454](fixes.md) | P1 | M | M5 | chore | done | A black-box QA pass before each beta |
| [T-455](fixes.md) | P2 | M | M5 | chore | done | Readiness: the promises with a test, in place of the commands with a scenario |
| [T-456](fixes.md) | P1 | S | M7 | defect | done | `pod --backend seccomp`: a connect to a loopback port gives EOPNOTSUPP (PodUtils/podrunner#213) |
| [T-457](fixes.md) | P1 | S | M7 | defect | done | `pod --backend seccomp`: the runner times out to github.com under the hold (PodUtils/podrunner#142) |
| [T-458](fixes.md) | P2 | S | M5 | defect | done | `link`: the address scheme does not select the carrier; a `unix:` prefix is taken as a file name (GitHub #634) |
| [T-459](fixes.md) | P2 | S | M5 | defect | done | Error messages repeat the cause twice (GitHub #635) |
| [T-460](fixes.md) | P2 | S | M5 | defect | done | `serve --local --pin`: a plain-HTTP listener sets a Secure cookie, so the PIN unlock does not persist (GitHub #636) |
| [T-461](fixes.md) | P2 | S | M5 | defect | done | `route`: a door quota can be overshot by a full 64 KiB buffer per flow (GitHub #637) |
| [T-462](fixes.md) | P2 | S | M5 | defect | done | `forward --dns`: the DNS door is printed as up before it binds; a bind failure exits 1 after the success line (GitHub #638) |
| [T-463](fixes.md) | P2 | S | M5 | defect | done | `bench` exits 0 when each request fails (GitHub #639) |
| [T-464](fixes.md) | P2 | S | M5 | defect | done | `autostart add` exits 69 but still creates the entry (GitHub #640) |
| [T-465](fixes.md) | P2 | S | M5 | defect | done | Detached `--json` documents do not share a schema (`url` and `http`, `doors`, `forward`) (GitHub #641) |
| [T-466](fixes.md) | P2 | S | M5 | defect | done | `--json` is accepted by some commands and refused by others (GitHub #642) |
| [T-467](fixes.md) | P2 | S | M5 | defect | done | `serve --check` refuses `--local`, so a plan of a local file server cannot be checked (GitHub #643) |
| [T-468](fixes.md) | P2 | S | M5 | defect | done | The default log level prints an INFO line on stderr for `status` (GitHub #644) |
| [T-469](fixes.md) | P3 | S | M5 | defect | done | `podnet-install.sh --dry-run` prints the proxy line twice; the default directory can be noexec (GitHub #645) |
| [T-470](fixes.md) | P2 | S | M5 | defect | done | `expose --via switch` refuses the `http://` target that `expose` documents (GitHub #646) |
| [T-471](fixes.md) | P2 | S | M5 | defect | done | `ssh-relay --no-restart`: the help names `--stop-after`, `--idle-secs` and `--run-for`, which the command refuses (GitHub #647) |
| [T-472](fixes.md) | P3 | S | M5 | defect | done | `serve --list`: the HTML listing omits the modified time and the upload limit (GitHub #648) |
| [T-473](fixes.md) | P2 | M | M3 | chore | done | A survey of podnet's copies of shared code against podcrates and the siblings (PodUtils/podcrates#221) |
| [T-474](fixes.md) | P1 | S | M5 | defect | done | `ssh-relay` ignores `--proxy`, `--egress`, `--resolve` and the proxy variables |
| [T-475](fixes.md) | P2 | S | M5 | defect | done | `warp up` reads its own list of 4 system CA bundles; the manual and `pod_tls` have 6 |
| [T-476](fixes.md) | P2 | S | M6 | defect | blocked | `transfer_over_the_relay_crosses_the_cap` fails under load: a session meets the cap before its move |
| [T-477](fixes.md) | P2 | S | M5 | defect | done | A refusal for now (HTTP 429, exit 75) gets the hint "does not change with a retry" |
| [T-478](fixes.md) | P1 | M | M6 | defect | blocked | Two parallel SSH sessions through one relay node: one hung with no line |
| [T-479](fixes.md) | P2 | S | M5 | defect | done | Tests free a port to get a refused address; another test can take the port |
| [T-480](crates.md) | P1 | S | M3 | chore | done | Take podcrates v0.70.0 |
| [T-481](fixes.md) | P1 | S | M5 | release | done | Publish `v0.4.0-beta.17` for podrunner: the fixes of T-456 and T-457, and the QA round |
| [T-482](fixes.md) | P3 | S | M5 | defect | done | `pod --backend netns` prints a second, false cause after a refused network namespace |
| [T-483](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.71.0 |
| [T-484](fixes.md) | P1 | S | M5 | docs | done | The version line is a contract that podrunner reads |
| [T-485](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.72.0 |
| [T-486](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.73.0 |
| [T-487](fixes.md) | P1 | S | M5 | release | done | The image on ghcr.io: an attestation of its provenance, and its digests in the release notes |
| [T-488](fixes.md) | P3 | S | M5 | research | open | A quick tunnel with two edge connections gets HTTP 530 for some requests |
| [T-489](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.74.0 |
| [T-490](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.75.0 |
| [T-491](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.76.0 |
| [T-492](fixes.md) | P2 | S | M5 | defect | done | Tests free a port and then bind it; another test can take the port first |
| [T-493](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.77.0 |
| [T-494](fixes.md) | P2 | M | M3 | chore | done | Take `pod-release` in place of podnet's own update |
| [T-495](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.78.1 |
| [T-496](fixes.md) | P1 | S | M7 | defect | done | `pod`: a failed connect of the program gives no line at the default level |
| [T-497](fixes.md) | P2 | S | M5 | defect | done | The error lines of `tunnel`, `warp proxy` and `connect` name one cause two times (code of podcrates) |
| [T-498](fixes.md) | P2 | S | M5 | defect | done | A direct dial that timed out names its step two times |
| [T-499](fixes.md) | P2 | M | M7 | defect | done | `pod --backend netns`: no name resolves when `/etc/resolv.conf` is a mount of a deleted file |
| [T-500](fixes.md) | P2 | S | M5 | defect | done | `podnet pod --help --json` prints text, and the manual of `pod` has no flag of `pod` |
| [T-501](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.79.0 |
| [T-502](fixes.md) | P2 | S | M5 | defect | done | `forward` ends with a bare `Operation not supported` where the host refuses getsockname |
| [T-503](fixes.md) | P1 | S | M7 | defect | done | `pod --backend netns`: the second path of the resolver does not answer a loopback nameserver of the host |
| [T-504](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.80.0 |
| [T-505](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.81.0 |
| [T-506](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.82.0 |
| [T-507](fixes.md) | P1 | S | M7 | defect | done | `pod --backend seccomp`: refuse `listen(2)` on a kernel socket with `EPERM` (H-02) |
| [T-508](fixes.md) | P2 | M | M7 | defect | done | `pod --backend netns`: reassemble IPv4 UDP fragments up to 65,507-byte payloads (H-04) |
| [T-509](fixes.md) | P2 | S | M5 | defect | done | `net shim`: per-run `PODNET_VNET_PREFIX`, 59-byte cap, pre-build validation, and stderr warning (H-05) |
| [T-510](fixes.md) | P2 | S | M5 | defect | done | live: the check of agent up cannot connect to the address that the resolver of the runner gives |
| [T-511](fixes.md) | P2 | S | M5 | defect | done | ci: the test of the HTTP/3 qlog reads the QUIC trace of noq before noq flushes it |
| [T-512](fixes.md) | P1 | S | M5 | defect | done | ci: the new loopback tests of 4df28952 fail on a host with no bind(AF_INET) and with the loopback down |
| [T-513](fixes.md) | P2 | S | M5 | defect | done | live: the edge did not know a new quick tunnel, and the second line said that a retry does not help |
| [T-514](fixes.md) | P1 | S | M5 | defect | done | gate: get-podlab.sh fails when the podcrates tag of Cargo.toml has no GitHub release |
| [T-515](fixes.md) | P1 | S | M5 | defect | done | connect: a TCP or unix listener that cannot bind exits 1; net proxy, forward and warp proxy exit 69 (GitHub #689) |
| [T-516](fixes.md) | P1 | S | M5 | defect | done | pod: with the seccomp backend, a program that cannot run exits 127 with no line (GitHub #690) |
| [T-517](fixes.md) | P1 | M | M5 | defect | done | pod: SIGTERM, SIGINT or SIGHUP to podnet ends podnet and leaves the program running (GitHub #691) |
| [T-518](fixes.md) | P1 | S | M5 | defect | done | rules check: a policy file that cannot be read or parsed exits 1; PODNET_POLICY gives 78 (GitHub #700) |
| [T-519](fixes.md) | P2 | S | M5 | defect | done | JSON documents without the stamp: warp status, pod doctor and rules check (GitHub #693, #694) |
| [T-520](fixes.md) | P2 | S | M5 | defect | done | send: the timeout line gives the wrong time, "within its 1 minutes" for --timeout 5 (GitHub #687) |
| [T-521](fixes.md) | P2 | M | M5 | defect | done | error lines: a hint of the wrong hop (quick tunnel), and no hint for a dead proxy or a name that does not resolve (GitHub #695, #697, #698) |
| [T-522](fixes.md) | P2 | M | M5 | defect | done | error lines and rows name one cause two to six times, with the Debug text of a Rust struct, and one line holds three resolvers (GitHub #696, #697, #698) |
| [T-523](fixes.md) | P2 | S | M5 | defect | done | a listener that cannot bind: two lines for one cause, no next step, and the bind comes after the WARP session (GitHub #705, #706) |
| [T-524](fixes.md) | P2 | S | M5 | defect | done | net doctor: a host with no loopback gets "not needed", and a TCP loopback listener starts and cannot be reached (GitHub #692) |
| [T-525](fixes.md) | P2 | S | M5 | defect | done | net shim: the documented `--out /tmp/podnet-vnet` example prints a working LD_PRELOAD line on a noexec /tmp (GitHub #699) |
| [T-526](fixes.md) | P2 | S | M5 | defect | done | docs/host-runtime.md: the examples `podnet fetch` and `podnet init generate --node` do not run (GitHub #701) |
| [T-527](fixes.md) | P2 | S | M5 | defect | done | pod --serve: a refused target gives SOCKS5 reply 0x01; docs/proxies.md says 0x04 for "refused" (GitHub #702) |
| [T-528](fixes.md) | P2 | S | M5 | defect | done | doctor: exit 1 when no leg works; `podnet man exit-status` says 69 for "no edge leg worked" (GitHub #703) |
| [T-529](fixes.md) | P2 | S | M5 | defect | done | state directory: the warning names a path that does not exist, and `warp register` prints it three times (GitHub #704) |
| [T-530](fixes.md) | P3 | S | M5 | defect | done | bench: the text output shows `null` for the latency of a run with no answer (GitHub #698) |
| [T-531](fixes.md) | P2 | S | M5 | defect | done | pod: a bad PODNET_RESOLVE is ignored; the manual says exit 78 for each command (GitHub #687, end of the pass) |
| [T-532](fixes.md) | P3 | S | M5 | defect | done | pod: the seccomp backend counts each refused connect twice; autostart remove of an unknown name exits 1 (GitHub #687, end of the pass) |
| [T-533](fixes.md) | P1 | S | M5 | release | done | Publish `v0.4.0-beta.18`: the fixes of the QA pass of 2026-11-23, `pod-release` and podcrates `v0.82.0` |
| [T-534](fixes.md) | P1 | S | M5 | defect | done | The release commit 9f3c8a2 fails the lane and the restricted host: the row warp-proxy-dead-proxy and a dead unix socket that a forked child holds |
| [T-535](fixes.md) | P2 | S | M5 | defect | done | live: the check of a WebSocket through a --url origin gives no line of podnet when the tunnel is not ready |
| [T-536](fixes.md) | P3 | S | M5 | defect | done | a refused unix bind says that the directory is not writable when the host refuses the socket itself |
| [T-537](fixes.md) | P2 | S | M5 | chore | done | the build image of Dependabot #680 cannot link loongarch64: --no-rosegment |
| [T-538](fixes.md) | P1 | S | M5 | defect | done | The job image of the release of v0.4.0-beta.18 fails: a docker logout before the push of the attestation |
| [T-539](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.82.1 |
| [T-540](fixes.md) | P3 | S | M5 | feature | done | release.yml writes `release-index.json` before `SHA256SUMS` |
| [T-541](fixes.md) | P3 | S | M5 | defect | done | tests of a tool or of IPv6 of the host fail on a host without it, in place of a skip |
| [T-542](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.83.0 |
| [T-543](fixes.md) | P2 | M | M5 | feature | done | Take `pod-natprobe`: the class of the NAT of the host for `link p2p` and `doctor` |
| [T-544](fixes.md) | P3 | S | M5 | chore | done | Take the script `runs.py` of podcrates in place of podnet's `watch-runs.sh` and `runs.py` |
| [T-545](fixes.md) | P2 | M | M5 | defect | done | temp files and sockets named by the pid alone: two processes in two PID namespaces meet on one path |
| [T-546](fixes.md) | P2 | S | M5 | defect | done | warp proxy: a client with no WARP session gets a TCP reset, and the live step hides the line of pod_warp |
| [T-547](fixes.md) | P2 | S | M5 | defect | done | pod doctor: the netns backend is ok on a host where its TCP connect is refused (bind-denied) |
| [T-548](fixes.md) | P2 | S | M5 | defect | done | logs: the log of a detached instance does not hold the lines that the start shows before the ready |
| [T-549](fixes.md) | P2 | S | M5 | defect | done | warp proxy: a MASQUE door behind a TLS middlebox gives the step --ca-file |
| [T-550](fixes.md) | P2 | S | M5 | defect | done | pod-warp: a pin mismatch blames WARP for the certificate of a middlebox and says the cause two times |
| [T-551](fixes.md) | P2 | S | M5 | defect | done | tunnel, send, receive, relay, offline: a JSON event with no schema and no podnet key |
| [T-552](fixes.md) | P2 | S | M5 | defect | done | pod --serve: a listener that cannot bind gives the bare error of the system and exit 3 |
| [T-553](fixes.md) | P2 | S | M5 | defect | done | doctor: the leg quic+warp is ok where a TLS middlebox answers at the MASQUE door |
| [T-554](fixes.md) | P2 | S | M5 | defect | done | exit: a name with no address and a dial time limit get SOCKS5 0x01, not 0x04 |
| [T-555](fixes.md) | P2 | S | M5 | defect | done | net proxy: a closed virtual port gets SOCKS5 0x01 |
| [T-556](fixes.md) | P2 | S | M5 | defect | done | init enroll: a failed dial of the relay drops the cause and gives no next step |
| [T-557](fixes.md) | P2 | S | M5 | defect | done | init enroll: a relay URL that is not a relay prints the raw chunked HTML body |
| [T-558](fixes.md) | P2 | S | M5 | defect | done | host probe, stat --json: schema is a string and the podnet key is missing |
| [T-559](fixes.md) | P2 | S | M5 | defect | done | rules check: the documents do not give the fields of a policy rule |
| [T-560](fixes.md) | P2 | S | M5 | defect | done | ci check: a declaration with schema 1 is refused, and each string is taken |
| [T-561](fixes.md) | P2 | S | M5 | defect | done | man --json SECTION: docs/cli.md names the keys variables, exits and table as sections |
| [T-562](fixes.md) | P2 | S | M5 | defect | done | autostart add: refuses a binary under /var/tmp as temporary, with exit 64 |
| [T-563](fixes.md) | P2 | S | M5 | defect | done | offline bundle build: the example of docs/offline.md (--file ./site:/site) does not run |
| [T-564](fixes.md) | P2 | S | M5 | defect | done | serve --local: on a host with no loopback address the listener is ready, and no client can connect |
| [T-565](fixes.md) | P2 | S | M5 | docs | done | docs/virtual-network.md says that curl does not take a unix-socket SOCKS proxy |
| [T-566](fixes.md) | P2 | S | M5 | defect | done | --log-format json: the last ERROR object has no timestamp |
| [T-567](fixes.md) | P2 | S | M5 | defect | done | route --config: a missing or bad file exits 64; tunnel --config gives 78 |
| [T-568](fixes.md) | P2 | S | M5 | defect | done | wg serve: on a host that refuses UDP, the step says to give a port above 1023 or a unix socket |
| [T-569](fixes.md) | P2 | S | M5 | defect | done | serve --local, net proxy --listen: where getsockname is refused, a bare Not supported and exit 1 |
| [T-570](fixes.md) | P2 | S | M5 | defect | done | --egress, forward --network: an unknown spec is read as a proxy HOST:PORT |
| [T-571](fixes.md) | P1 | S | M5 | defect | done | forward: a UDP forward and the DNS door over the way of the process carry no datagram |
| [T-572](fixes.md) | P1 | S | M5 | defect | done | pod: PODNET_POLICY does not refuse a destination |
| [T-573](fixes.md) | P2 | S | M5 | defect | done | bench, connect: a refusal of PODNET_POLICY gives the next step of a connect failure |
| [T-574](fixes.md) | P2 | S | M5 | defect | done | warp status, rotate, delete, license: a missing identity file exits 78, not 66 |
| [T-575](fixes.md) | P2 | S | M5 | defect | blocked | bench, connect, tunnel, relay connect, forward: the line of a name that does not resolve is cut with ... in a cause |
| [T-576](fixes.md) | P2 | S | M5 | defect | done | warp up: the failure prints two lines of about 1500 characters that repeat each cause |
| [T-577](fixes.md) | P2 | S | M5 | defect | done | pod --report: the backend netns-kernel is not a value of pod-report.schema.json |
| [T-578](fixes.md) | P2 | S | M5 | defect | done | pod --backend netns (userns-off): the start failure gives no next step, and the report has no true reason |
| [T-579](fixes.md) | P2 | S | M5 | defect | blocked | tunnel: behind a proxy the start waits about 35 s in DNS over HTTPS with no line |
| [T-580](fixes.md) | P2 | S | M5 | defect | blocked | --proxy http+exec: a command that does not exist floods stderr and ends with a wrong cause |
| [T-581](fixes.md) | P2 | S | M5 | defect | done | pod doctor: seccomp and netns are ok in no-proc and proc-masked, where the pod cannot start |
| [T-582](fixes.md) | P2 | S | M5 | defect | done | update --check: a proxy that does not answer costs 60 s and the error gives no next step |
| [T-583](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.84.0 |
| [T-584](fixes.md) | P3 | S | M5 | defect | done | send --json: the sent event of a text gives "bytes": 0 |
| [T-585](fixes.md) | P1 | S | M5 | release | done | Publish `v0.4.0-beta.19`: the fixes of the QA pass of 2026-11-25 and podcrates `v0.84.0` |
| [T-586](fixes.md) | P3 | M | M8 | feature | blocked | link p2p: take the relay first when the NAT classes of both ends rule out the direct path |
| [T-587](fixes.md) | P2 | S | M5 | defect | done | live: a second run of doctor sees the leg quic+warp as not ok, and the step prints no reason |
| [T-588](fixes.md) | P1 | S | M3 | chore | done | Take podcrates v0.85.0 |
| [T-589](fixes.md) | P3 | S | M5 | defect | done | live: the quick-tunnel service refuses the runner for an hour (HTTP 429, error 1015), and each later step fails |
| [T-590](fixes.md) | P2 | S | M5 | defect | done | The crash log and the harness name podcrates 0.79.0, and podnet takes v0.85.0 |
| [T-591](fixes.md) | P2 | S | M5 | defect | done | podnet cover with no --name answers each request with the 404 page, also / |
| [T-592](fixes.md) | P2 | S | M5 | defect | done | link p2p drops the answer of the peer when stdin ends first |
| [T-593](fixes.md) | P2 | S | M5 | defect | done | connect: the timeout line says "timed out after 0s" after a wait of 20 s |
| [T-594](fixes.md) | P2 | S | M5 | defect | done | connect: a target that fails behind a working proxy is reported as a proxy failure |
| [T-595](fixes.md) | P2 | S | M5 | defect | done | connect: a name that does not resolve exits 69, not 68 (no-host) |
| [T-596](fixes.md) | P2 | S | M5 | defect | done | net proxy: --switch tcp://NAME:PORT is refused as a "bad TCP listen address" |
| [T-597](fixes.md) | P3 | S | M5 | defect | done | wait: the help says exit 69 at --timeout, the command exits 124 |
| [T-598](fixes.md) | P3 | S | M5 | defect | blocked | doctor: on bind-denied the TCP carrier rows give "Address family not supported", not the refused bind |
| [T-599](fixes.md) | P2 | S | M5 | defect | done | link dial: a bad --peer value exits 78 and names "line 1" |
| [T-600](fixes.md) | P2 | S | M5 | defect | done | net proxy: --switch to an address that does not answer waits 135 s, past --run-for, with no line |
| [T-601](fixes.md) | P3 | S | M5 | defect | done | tunnel: the warning "DNS discovery of the edge failed" comes two times at one start |
| [T-602](fixes.md) | P2 | S | M5 | defect | done | pod --serve: SIGINT or SIGTERM kills it with no clean stop, and the socket file stays |
| [T-603](fixes.md) | P3 | S | M5 | defect | done | offline: the help says "a four-word pairing code", the code has eight words |
| [T-604](fixes.md) | P3 | S | M5 | defect | done | offline pair accept: it waits for the code on stdin with no prompt, and the help does not say so |
| [T-605](fixes.md) | P2 | S | M5 | defect | done | expose --via switch: the example of docs/forward.md exits 64 "--switch needs --address" |
| [T-606](fixes.md) | P2 | S | M5 | defect | done | forward: a refused bind of port 18000 tells the user to "give a port above 1023" |
| [T-607](fixes.md) | P3 | S | M5 | defect | done | init check: a missing file is reported as "the configuration is not schema 1" |
| [T-608](fixes.md) | P1 | S | M5 | release | done | Publish `v0.4.0-beta.20`: the carriers of the feature `release` and the fixes of the QA pass of 2026-11-26 |
| [T-609](fixes.md) | P2 | S | M5 | feature | partial | The release set holds each carrier of the link: s3, icmp, ntp, smtp and imap join the feature release |
| [T-610](fixes.md) | P3 | S | M5 | defect | done | link carriers names the cost class of a carrier two times |
| [T-611](later.md) | P3 | L | M9 | feature | open | The direction split of a TCP flow of a router group, over a split session of the link |
