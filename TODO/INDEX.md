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

**313 entries: 73 open, 15 partial, 24 blocked, 201 done.**

| Priority | open | partial | blocked | done | total |
| --- | --- | --- | --- | --- | --- |
| P0 | 0 | 0 | 0 | 1 | 1 |
| P1 | 0 | 1 | 2 | 25 | 28 |
| P2 | 19 | 7 | 10 | 123 | 159 |
| P3 | 54 | 7 | 12 | 52 | 125 |
| **All** | 73 | 15 | 24 | 201 | 313 |

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
| [T-012](record.md) | P2 | L | M0 | chore | blocked | WARP stand-ins on the loopback: the registration API and a MASQUE edge, with tests of the WARP commands |
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
| [T-035](split.md) | P2 | M | M2 | chore | open | Split `src/warp/ip.rs`, `src/warp/session.rs` and `src/warp/wg.rs` |
| [T-036](split.md) | P2 | M | M2 | chore | open | Split `src/warp/mod.rs` and `src/warp/proxy.rs` |
| [T-037](split.md) | P2 | M | M2 | chore | done | Split `src/relay_reverse.rs` and `crates/podnet-serve/src/share/mod.rs` |
| [T-038](split.md) | P2 | M | M2 | chore | done | Split `crates/podnet-serve/src/feature/mod.rs` and `src/proxy.rs` |
| [T-039](split.md) | P2 | M | M2 | chore | open | Split `src/edge_h2.rs` and `src/tunnel.rs` |
| [T-040](split.md) | P2 | L | M2 | chore | done | Split the seccomp backend and the netns backend of the pod crate |
| [T-041](split.md) | P2 | M | M2 | chore | done | Split the four large test files |
| [T-042](split.md) | P2 | M | M2 | chore | done | Cut the upward references 1 to 7: `net`, `util`, `fetch`, `hop`, `report`, `detach` and `relay_reverse` |
| [T-043](split.md) | P2 | M | M2 | chore | done | Cut the upward references 8, 9 and 18: a WebSocket bridge and `Metered` in the core |
| [T-044](split.md) | P2 | L | M2 | chore | blocked | Cut the upward references 10 to 17: the WARP QUIC configuration, `ShareLink`, `RULE_KEYS`, `connect_tunnel` and the edge side of `ws` |
| [T-045](split.md) | P2 | S | M2 | chore | partial | Each file is under 800 lines, and the allowlists of the size check and the layer check are empty |
| [T-046](crates.md) | P2 | M | M3 | chore | done | Prepare the workspace for more crates: the print macros, the build facts, the scans, the package list and the test helpers |
| [T-047](crates.md) | P2 | L | M3 | chore | done | Extract the crate `podnet-core` |
| [T-048](crates.md) | P2 | M | M3 | chore | done | Extract the crate `podnet-vnet`, with the C sources of the shims |
| [T-049](crates.md) | P2 | S | M3 | chore | done | The relay client of podnet becomes the shared crate `pod-relay` |
| [T-050](crates.md) | P2 | M | M3 | chore | open | Move the WARP adapter to the shared crate `pod-warp` in `PodUtils/podcrates` |
| [T-051](crates.md) | P2 | M | M3 | chore | done | Extract the crate `podnet-serve`, with the library-only modules |
| [T-052](crates.md) | P2 | M | M3 | chore | open | Move the Cloudflare adapter to the shared crate `pod-cloudflare` in `PodUtils/podcrates` |
| [T-053](crates.md) | P2 | M | M3 | chore | open | The root package is the CLI and a facade that keeps each library path |
| [T-054](crates.md) | P2 | S | M3 | release | open | A prerelease after the crate split, with the static builds of each target |
| [T-055](adapters.md) | P2 | M | M4 | feature | done | The network types in `podnet-core`: a target, a dialer, datagrams, a network and its link |
| [T-056](adapters.md) | P2 | M | M4 | feature | done | Dialers for direct, the egress proxy, the relay, the pod upstream chains, the tailnet front door and the virtual network |
| [T-057](adapters.md) | P2 | M | M4 | feature | open | A dialer and datagrams for WARP |
| [T-058](adapters.md) | P2 | M | M4 | feature | done | The combinators `Chain`, `Rules`, `Race` and `Failover` |
| [T-059](adapters.md) | P2 | M | M4 | feature | done | The registry of networks, the parser of network specs, and the rows of `doctor` |
| [T-060](adapters.md) | P2 | M | M4 | feature | done | `--via SPEC` on each command, with each old network flag as an alias |
| [T-061](adapters.md) | P2 | M | M4 | feature | partial | One front door over a dialer for `warp proxy`, `pod --serve`, `net proxy` and `net tailscale` |
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
| [T-086](services.md) | P2 | L | M6 | feature | open | Take the resumable layer, the end-to-end channel, the key identity and UDP in a stream from the shared crates |
| [T-087](services.md) | P2 | M | M6 | feature | open | `relay node` and `relay connect` resume a session past the limits of the relay |
| [T-088](services.md) | P2 | S | M6 | feature | done | File server: a directory listing in HTML and JSON |
| [T-089](services.md) | P2 | M | M6 | feature | done | File server: users with read and write roles, and a PIN, on each listener |
| [T-090](services.md) | P2 | M | M6 | feature | done | File server: uploads into the served tree with PUT and a multipart form |
| [T-091](services.md) | P2 | M | M6 | feature | done | `podnet serve DIR --via`, a local `share`, a directory in `share`, and the live step of the file server |
| [T-092](services.md) | P2 | L | M6 | feature | done | Git: a CGI runner for `git http-backend`, and the smart HTTP router |
| [T-093](services.md) | P2 | M | M6 | feature | done | Git: an SSH server for upload-pack and receive-pack |
| [T-094](services.md) | P2 | M | M6 | feature | done | `podnet git serve`: the repository root, `git init`, `git urls`, the tunnel, the relay and the live step |
| [T-095](services.md) | P2 | M | M6 | feature | open | Router: several WARP identities in one process, each with its own counters |
| [T-096](services.md) | P2 | M | M6 | feature | done | Router: groups of networks with weights and a kill switch |
| [T-097](services.md) | P2 | M | M6 | feature | partial | Router: a TCP health check for each member, and `podnet route status` |
| [T-098](services.md) | P2 | M | M6 | feature | done | Router: doors with a default network, and rules to a network, a group or a refusal |
| [T-099](services.md) | P2 | M | M6 | feature | partial | Router: `podnet forward` for each network, UDP forwards, a DNS door, and `podnet expose` |
| [T-100](services.md) | P2 | M | M6 | feature | done | Router: a config file, and `podnet route` with `--detach` and autostart |
| [T-101](services.md) | P2 | M | M6 | measurement | open | Router: the live step with WARP, the relay and direct, and a failover |
| [T-102](services.md) | P2 | M | M6 | feature | open | Take the mailbox client, the code words and the SPAKE2 phases from `pod-wormhole` |
| [T-103](services.md) | P2 | L | M6 | feature | open | The file-transfer protocol and the transit protocol of magic-wormhole |
| [T-104](services.md) | P2 | M | M6 | feature | open | The podnet relay as a carrier of a transfer |
| [T-105](services.md) | P2 | M | M6 | feature | open | `podnet send` and `podnet receive` |
| [T-106](services.md) | P2 | M | M6 | measurement | open | An interop test with the Python magic-wormhole, and the live step of a transfer of 100 MiB |
| [T-107](pods.md) | P2 | M | M7 | feature | done | seccomp: the pod lives past its first program |
| [T-108](pods.md) | P2 | S | M7 | feature | done | netns: the pod lives past its first program |
| [T-109](pods.md) | P2 | M | M7 | feature | done | `warp exec --detach --name N` keeps a pod under the supervisor |
| [T-110](pods.md) | P2 | M | M7 | feature | done | `warp exec --join N` runs a command in a detached pod |
| [T-111](pods.md) | P2 | M | M7 | feature | done | netns: generic UDP |
| [T-112](pods.md) | P3 | S | M7 | feature | done | netns: the gateway address reaches the loopback of the host, when the user asks |
| [T-113](pods.md) | P2 | S | M7 | feature | done | `warp exec` and `warp shell` try the netns backend first |
| [T-114](overlays.md) | P2 | M | M8 | chore | open | A headscale harness in CI for the `tailscaled` sidecar |
| [T-115](overlays.md) | P3 | S | M8 | feature | partial | The tailnet as a network of the registry and of the router |
| [T-116](overlays.md) | P3 | S | M8 | feature | done | WireGuard: read a wg-quick file into a peer, with optional reserved bytes |
| [T-117](overlays.md) | P3 | M | M8 | feature | partial | WireGuard: `wg:FILE` as a network over UDP or over a carrier |
| [T-118](overlays.md) | P3 | M | M8 | research | done | Research: NetBird, Firezone and ZeroTier as networks |
| [T-119](later.md) | P3 | M | M9 | feature | done | Router kernel mode: tables, rules and multipath routes through rtnetlink |
| [T-120](later.md) | P3 | M | M9 | feature | partial | Router kernel mode: masquerade, an MSS clamp, an unreachable kill switch and `rp_filter` |
| [T-121](later.md) | P3 | S | M9 | feature | done | Router: export the groups as a JSON file of the mwan form |
| [T-122](later.md) | P3 | M | M9 | research | done | Research: a native git server with no installed `git` |
| [T-123](later.md) | P3 | M | M9 | feature | open | Tailscale in the process: take the fork of tailscale-rs through `pod-ts` |
| [T-124](later.md) | P3 | L | M9 | feature | open | Tailscale in the process: an adapter crate behind a feature that is off by default, with the static build proven |
| [T-125](later.md) | P3 | L | M9 | feature | open | A userspace WireGuard responder: `podnet wg serve` |
| [T-126](later.md) | P3 | M | M9 | feature | open | Egress through an `https://` proxy and through wstunnel |
| [T-127](later.md) | P3 | L | M9 | feature | open | Egress through VLESS, Shadowsocks and Wisp |
| [T-128](later.md) | P3 | L | M9 | feature | open | The DERP carrier through `pod-derp` |
| [T-129](later.md) | P3 | M | M9 | feature | open | An ACME client for a public listener |
| [T-130](later.md) | P3 | M | M9 | research | done | Research: the iroh road, and a relay and mailbox of podnet's own |
| [T-131](later.md) | P3 | L | M9 | chore | open | A crate that podnet and podssh share for the layer, the channel and the identity |
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
| [T-153](later.md) | P3 | L | M9 | feature | open | netns: a backend with an in-namespace netfilter data plane (GitHub #352) |
| [T-154](fixes.md) | P2 | M | M5 | chore | done | seccomp: fuzz targets for the notification handlers (GitHub #353) |
| [T-155](fixes.md) | P3 | M | M5 | chore | done | seccomp: a differential check of the socket calls against the host kernel (GitHub #353) |
| [T-156](pods.md) | P2 | M | M7 | feature | done | pod: a `reject` action in `-r`, for TCP and for datagrams (GitHub #354) |
| [T-157](pods.md) | P2 | M | M7 | feature | done | pod: `--default-action` and a JSON flow record (GitHub #355) |
| [T-158](pods.md) | P3 | M | M7 | feature | done | pod: `--learn` and `--fail-on-violation` (GitHub #355) |
| [T-159](fixes.md) | P3 | S | M5 | docs | done | research documents are readings; fix the stale counts of the record (GitHub #356) |
| [T-160](fixes.md) | P2 | S | M5 | defect | done | tunnel verify: a name that this host cannot resolve yet keeps the tunnel up (GitHub #357) |
| [T-161](crates.md) | P2 | M | M3 | feature | blocked | relay node: renew the pair before it expires (GitHub #358) |
| [T-162](crates.md) | P3 | M | M3 | feature | done | relay: a host that stalls is tried last on the next connect (GitHub #359) |
| [T-163](crates.md) | P2 | M | M3 | feature | partial | relay: the session budget comes from the pool document (GitHub #360) |
| [T-164](fixes.md) | P3 | S | M5 | docs | done | Document `--no-verify` in the README, and say what it does not do (GitHub #361) |
| [T-165](later.md) | P3 | M | M9 | feature | done | The SSH forward-relay leg: supervise the session after `run()` returns (GitHub #362) |
| [T-166](later.md) | P3 | M | M9 | feature | done | The SSH relay leg: the relay, the user and the URL rule are data (GitHub #363) |
| [T-167](services.md) | P2 | M | M6 | feature | done | `net socksify`: route chosen destinations, not only the tailnet or all (GitHub #364) |
| [T-168](services.md) | P2 | S | M6 | feature | done | A passwd shim for any client on a host with no `/etc/passwd` (GitHub #365) |
| [T-169](fixes.md) | P2 | S | M5 | defect | partial | `tunnel`: try the MASQUE door of the host's address family first, and take `--masque-endpoint` (GitHub #366) |
| [T-170](fixes.md) | P2 | S | M5 | defect | done | Quick-tunnel provisioning does not retry a DNS failure that cannot recover (GitHub #367) |
| [T-171](fixes.md) | P2 | S | M5 | defect | done | The verify loop names the remedy when the public name does not resolve (GitHub #367) |
| [T-172](later.md) | P3 | M | M9 | feature | done | A pin of the new public name while the process runs (GitHub #367) |
| [T-173](fixes.md) | P2 | S | M5 | feature | partial | A WARP identity of another uid, with a consent variable (GitHub #368) |
| [T-174](pods.md) | P3 | M | M7 | feature | done | netns: a veth wire when `/dev/net/tun` is not there (GitHub #369) |
| [T-175](crates.md) | P2 | M | M3 | feature | done | DoH: keep one connection for each resolver server (an ask of pod-dns and pod-proxy) (GitHub #370) |
| [T-176](later.md) | P3 | S | M9 | research | done | Guest-wire router on smoltcp: the constraints of the TCP terminator and the UDP socket (GitHub #371 and #372) |
| [T-177](services.md) | P3 | S | M6 | feature | done | Router: `podnet route check`, a dry run of the rules (GitHub #375) |
| [T-178](later.md) | P3 | M | M9 | feature | done | Router: named group policies (`rr`, `minrtt`, `leastload`, `protect`) (GitHub #375) |
| [T-179](later.md) | P3 | S | M9 | research | partial | Research: a direction split for asymmetric uplinks (GitHub #375) |
| [T-180](services.md) | P3 | M | M6 | feature | partial | Router: write-side control of the members (select, rotate, reconnect, interval) (GitHub #376) |
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
| [T-242](services.md) | P3 | M | M6 | feature | partial | Rules: warn when an earlier domain or cidr rule shadows a later one, and fail when the shadowed rule is a `reject` (GitHub #433) |
| [T-243](overlays.md) | P3 | M | M8 | feature | done | `net tailscale`: `--tailnet NAME` refuses to start or to dial when the daemon is in another tailnet, a doctor row shows the status, and the docs name `TS_FORCE_NOISE_443` (GitHub #434) |
| [T-244](later.md) | P3 | L | M9 | feature | blocked | `tunnel --inspect`: capture each exchange beside the copy helpers of `forward_http`, keep a ring, redact by default, replay over a 0600 socket (GitHub #435) |
| [T-245](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.4`: the dry run, the publish and the release proofs |
| [T-246](overlays.md) | P3 | S | M8 | feature | done | NetBird: the network spec `netbird:[PORT]` over the SOCKS5 listener of netstack mode |
| [T-247](overlays.md) | P3 | M | M8 | measurement | blocked | NetBird: measure netstack mode on a proxy-only host |
| [T-248](later.md) | P3 | L | M9 | feature | partial | Router: a guest-wire router that ends guest TCP on smoltcp |
| [T-249](pods.md) | P2 | M | M7 | chore | done | netns: the router of the namespace runs on the interface of `pod-netstack` |
| [T-250](pods.md) | P2 | S | M7 | chore | done | CI: a root step that runs the netns tests that skip on a runner |
| [T-251](services.md) | P2 | S | M6 | feature | open | Transfer: the hosted magic-wormhole mailboxes on 443 as the default, with a choice that the code carries |
| [T-252](services.md) | P3 | M | M6 | feature | open | Transfer: a piping-server relay as a transit carrier of last resort |
| [T-253](services.md) | P3 | S | M6 | measurement | done | Live: probe the hosted mailboxes and relays from a proxy-only runner |
| [T-254](fixes.md) | P2 | M | M5 | chore | done | Housekeeping: one footnote for cfrs, no narrative history |
| [T-255](pods.md) | P2 | S | M7 | defect | done | seccomp: an IPv4-mapped loopback address is loopback |
| [T-256](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.5`: the dry run, the publish and the release proofs |
| [T-257](pods.md) | P2 | S | M7 | defect | done | doctor: the probes of the namespace allocate in a fork child |
| [T-258](fixes.md) | P3 | S | M5 | docs | done | Docs: stale or misleading comments and records (GitHub #459) |
| [T-259](fixes.md) | P2 | M | M5 | docs | done | The user documentation by topic: a short README and one document for each area |
| [T-260](fixes.md) | P2 | M | M5 | feature | done | doctor: probes for the lanes `masque_udp443` and `ws_upgrade` (GitHub #461, A-C-10) |
| [T-261](link.md) | P3 | M | M9 | feature | blocked | link: an Ed25519 key for each install, its fingerprint and an allow list (GitHub #466) |
| [T-262](link.md) | P3 | L | M9 | feature | blocked | link: the session of streams with credit flow control and keepalive (GitHub #466) |
| [T-263](link.md) | P3 | M | M9 | feature | blocked | link: the 10-minute resume of a session through pod-session (GitHub #466) |
| [T-264](link.md) | P3 | M | M9 | feature | open | link: a registry of carriers with an available() reason (GitHub #466) |
| [T-265](link.md) | P3 | M | M9 | feature | open | link: the commands listen, dial, status, close and carriers (GitHub #466) |
| [T-266](link.md) | P3 | M | M9 | feature | open | link: `peer:FINGERPRINT/SERVICE` as a network target and `forward -L LISTEN=peer:...` (GitHub #466 and #471) |
| [T-267](link.md) | P3 | M | M9 | feature | open | link: the control socket, its discipline and the status verb (GitHub #466) |
| [T-268](link.md) | P3 | L | M9 | feature | open | link: the TCP carrier with TLS 1.3, the accept side and the peer handshake (GitHub #467) |
| [T-269](link.md) | P3 | M | M9 | feature | open | link: the WebSocket carrier, its server half and its wrapper (GitHub #467) |
| [T-270](link.md) | P3 | M | M9 | feature | open | link: the HTTP/2 carrier with one bidirectional POST (GitHub #467) |
| [T-271](link.md) | P3 | M | M9 | feature | open | link: the HTTP/1.1 long-poll carrier (GitHub #467) |
| [T-272](link.md) | P3 | M | M9 | feature | open | link: the SSE and chunked-stream carrier (GitHub #467) |
| [T-273](link.md) | P3 | M | M9 | feature | open | link: the QUIC carrier (GitHub #467) |
| [T-274](link.md) | P3 | S | M9 | feature | open | link: the unix-socket carrier and its socketpair half (GitHub #467) |
| [T-275](link.md) | P3 | S | M9 | feature | open | link: the in-process memory carrier (GitHub #467) |
| [T-276](link.md) | P3 | L | M9 | feature | open | link: the UDP carrier with KCP (GitHub #468) |
| [T-277](link.md) | P3 | L | M9 | feature | open | link: a selective-repeat ARQ for any lossy carrier (GitHub #468) |
| [T-278](link.md) | P3 | L | M9 | feature | open | link: the DNS simplex carrier (GitHub #468) |
| [T-279](link.md) | P3 | L | M9 | feature | open | link: the object-storage simplex carrier (GitHub #468) |
| [T-280](link.md) | P3 | M | M9 | feature | open | link: the HTTP poll simplex carrier (GitHub #468) |
| [T-281](link.md) | P3 | L | M9 | feature | open | link: the DNS stream carrier with a pipelined query window (GitHub #469) |
| [T-282](link.md) | P3 | L | M9 | feature | open | link: the ICMP echo carrier (GitHub #469) |
| [T-283](link.md) | P3 | M | M9 | feature | open | link: a WireGuard peer as a carrier (GitHub #469) |
| [T-284](link.md) | P3 | L | M9 | feature | open | link: NAT traversal with a relay fallback (GitHub #469) |
| [T-285](link.md) | P3 | L | M9 | feature | open | link: signed route announcements (GitHub #470) |
| [T-286](link.md) | P3 | M | M9 | feature | open | link: the path-vector route table (GitHub #470) |
| [T-287](link.md) | P3 | L | M9 | feature | open | link: multi-hop forwarding of a session stream (GitHub #470) |
| [T-288](link.md) | P3 | M | M9 | feature | open | link: load balancing across link groups (GitHub #470) |
| [T-289](link.md) | P3 | M | M9 | feature | open | link: the local topology and the status view of a mesh (GitHub #470) |
| [T-290](link.md) | P3 | M | M9 | feature | open | link: per-hop access control for services and forwarding (GitHub #470) |
| [T-291](link.md) | P3 | M | M9 | feature | open | link: a SOCKS5 service at a peer (GitHub #471) |
| [T-292](link.md) | P3 | M | M9 | feature | open | link: an HTTP forward proxy service at a peer (GitHub #471) |
| [T-293](link.md) | P3 | S | M9 | feature | open | link: a raw passthrough service at a peer (GitHub #471) |
| [T-294](link.md) | P3 | M | M9 | feature | blocked | link: a Shadowsocks AEAD service at a peer (GitHub #471) |
| [T-295](link.md) | P3 | M | M9 | feature | blocked | link: a Trojan service at a peer (GitHub #471) |
| [T-296](link.md) | P3 | M | M9 | feature | open | link: an exec adapter that runs a program for each stream (GitHub #471) |
| [T-297](link.md) | P3 | M | M9 | feature | open | link: serve a file tree at a peer (GitHub #471) |
| [T-298](link.md) | P3 | L | M9 | feature | blocked | link: the AEAD record layer with negotiated suite and rekey (GitHub #472) |
| [T-299](link.md) | P3 | S | M9 | feature | open | link: snappy compression of records (GitHub #472) |
| [T-300](link.md) | P3 | S | M9 | feature | open | link: padding of records to buckets (GitHub #472) |
| [T-301](link.md) | P3 | M | M9 | feature | open | link: the replay window and the RESUME record (GitHub #472) |
| [T-302](link.md) | P3 | L | M9 | feature | open | link: split one session across an up carrier and a down carrier (GitHub #473) |
| [T-303](link.md) | P3 | M | M9 | feature | open | link: chain composition with a peer element and --check (GitHub #473) |
| [T-304](link.md) | P3 | L | M9 | feature | open | link: a C ABI with a header, a shared library and a static library (GitHub #474) |
| [T-305](link.md) | P3 | L | M9 | feature | blocked | link: a portable build subset for Windows, macOS and FreeBSD (GitHub #475) |
| [T-306](link.md) | P3 | M | M9 | feature | open | link: the query keys of a carrier URL (GitHub #476) |
| [T-307](link.md) | P3 | M | M9 | feature | open | link: the write verbs of the control plane (GitHub #476) |
| [T-308](link.md) | P3 | S | M9 | feature | open | link: names for live links (GitHub #476) |
| [T-309](link.md) | P3 | M | M9 | feature | open | link: the redacted state dump and bug-report --include-link (GitHub #476) |
| [T-310](link.md) | P3 | M | M9 | feature | open | link: the component inventory command (GitHub #476) |
| [T-311](link.md) | P3 | M | M9 | feature | open | link: Cargo features for the link carriers and services (GitHub #476) |
| [T-312](fixes.md) | P2 | S | M5 | defect | done | tests/signals.rs: the test sends a stop signal before the ready of the command (flaky under load) |
| [T-313](fixes.md) | P2 | S | M5 | release | done | Publish `v0.4.0-beta.6`: the dry run, the publish and the release proofs |
