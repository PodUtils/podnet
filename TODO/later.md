This file holds the entries of milestone M9: the work that the owner placed after the services,
the pods and the overlays, and the work that has no ruling of the owner yet. Each entry names its
start condition. An entry with no ruling asks the owner before its work starts.

# T-119: Router kernel mode: tables, rules and multipath routes through rtnetlink

**Source:** `docs/research/podnet/mwan-field.md`, sections 1.4, 1.6 (mode B) and 4 (rank 12); the
owner's ruling of 2026-11-05 (`docs/decisions.md`, Router).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

On a host with root, a user wants a kernel multi-WAN over several `warp tun` interfaces: a table
and a rule for each WAN, and a multipath default route. The route code of podnet uses ioctls only
and cannot write tables, rules or multipath routes.

## Premise

- Read: `src/warp/route.rs` adds and deletes IPv4 routes with `SIOCADDRT` and `SIOCDELRT`.
- The mwan experiment programs the kernel with the `ip` and `nft` programs, and has defects that
  podnet must not copy: a window with no rule at each interval (D3), pinned clients with no
  failover (D2), and client rules before the routes of the local subnets (D9).

## Approach

1. Write an rtnetlink writer: `RTM_NEWROUTE` with `RTA_MULTIPATH`, and `RTM_NEWRULE`. Use no
   `ip` program.
2. Make a table and a fwmark rule for each WAN, and an aggregate table with weights.
3. Replace a rule in place, with no delete before the add.
4. Put a rule `lookup main suppress_prefixlength 0` before the rules of the clients.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Router): a kernel mode for root hosts comes later.

## Prove

```sh
sudo cargo test --workspace -- route_kernel_tables_and_rules_in_a_namespace
```

The test runs in a new network namespace and compares `ip rule show` and `ip route show table N`
with the plan.

## Start condition

T-100 is done.

## Correction

2026-11-05, the route code is `src/warp/route.rs`, in the WARP adapter that moves to `pod-warp` (T-050). Read
where the code is when this entry starts.

## Done

2026-11-11, commit 2a8f121. The router file has the optional key `kernel` (`src/router/kernel.rs`).
Each WAN has a table with a default route and a rule `fwmark M lookup T`. An aggregate table has a
multipath default route with weights. The rule `lookup main suppress_prefixlength 0` has the first
priority. An old file stays valid; a file with `kernel` needs no door. `podnet route --config`
applies the plan after the doors are up and removes it at the end. The code writes rtnetlink
(`RTM_NEWROUTE` with `NLM_F_REPLACE` and `RTA_MULTIPATH`, `RTM_NEWRULE`, `RTM_DELRULE`) and uses no
`ip` program. `apply` adds the new rules first (`NLM_F_EXCL`, EEXIST is success) and then deletes
the old rules that the new plan does not hold: the kernel has no replace for a rule, so the change
is made before the break and there is no moment with no rule. Tests in `src/router/kernel/tests.rs`:
`route_kernel_tables_and_rules_in_a_namespace` (runs itself in `unshare -rn`, makes bridge links,
applies a plan, changes it, removes it, and compares the rules and routes that a netlink dump reads
with the plan; it skips with a message when the host does not allow the namespace),
`kernel_plan_puts_the_suppress_rule_before_the_rules_of_the_clients`,
`kernel_config_errors_name_the_key`, `kernel_section_keeps_each_old_file_valid`,
`kernel_requests_encode_rules_and_a_multipath_route`. check.sh: exit 0.

Not proven: the test reads the kernel with a netlink dump, not with `ip rule show` (the host has no
`ip`). The test uses bridge links because the host kernel has no `dummy` module. The path
`podnet route --config FILE` with `kernel` is run by hand only as far as the error "Network is
down" (exit 69) for a WAN link that is down; it has no automatic test. No traffic test of the
weights or of the fwmark was made. The writer is new code in `src/router/kernel.rs`; it does not
change `src/warp/route.rs` (frozen). Moving the writer to `pod-warp` or another crate of podcrates
is not part of this entry.

# T-120: Router kernel mode: masquerade, an MSS clamp, an unreachable kill switch and `rp_filter`

**Source:** `docs/research/podnet/mwan-field.md`, sections 1.4 (D1, D4), 1.6 (mode B) and 2.2
(R1).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

ECMP over tunnels needs a masquerade on each WAN, an MSS clamp and `rp_filter` 0. The kill switch
of the mwan experiment leaks: with no aggregate route, the kernel goes on to the main table.

## Premise

- The mwan experiment deletes the aggregate default as its kill switch (D1), and probes in its own
  namespace, not in the target namespace (D4).
- T-072 sets `rp_filter` 0 on a TUN of `warp tun`.

## Approach

1. Write the masquerade and the MSS clamp through nf_tables netlink. Use no `nft` program.
2. Put an `unreachable` default in the aggregate table as the kill switch.
3. Set `rp_filter` 0 on each WAN.
4. Probe each WAN through its session (T-068, T-069), not through the kernel routes.

## Prove

```sh
sudo cargo test --workspace -- route_kernel_kill_switch_does_not_leak
```

With each WAN down, a client of the aggregate gets "network unreachable", and no packet leaves
through the main table.

## Start condition

T-119 is done.

## Correction

2026-11-11: part 4 (probe each WAN through its session) is left. The session probe belongs to
`warp tun` (T-068, T-069; `src/warp/`, frozen until `pod-warp` is taken). The kernel mode has no
probe loop and no hook into the session of a TUN, so a WAN whose session is dead but whose link is up
stays in the multipath route. Continue it when the take of `pod-warp` (T-050) gives a probe API.
The kernel gives EHOSTUNREACH ("No route to host"), not ENETUNREACH, for an `unreachable` route;
ENETUNREACH is the error for no route at all. The Prove checks EHOSTUNREACH.

2026-11-14: the take of `pod-warp` (v0.41.0) gives no probe API to the router, and the router does not run the
session of a WAN. Call: part 4 is a probe of the WAN, not of the session object. Each interval, one TCP connect
to `health.target` leaves with the fwmark of the WAN, so it uses the table of that WAN and crosses its device
(for a TUN of `warp tun`, its session). The steps are those of `Tracker` (T-097). Calls made here:
- A probe that times out is a clean miss: a dead session gives no answer. The control of a round is "one WAN or
  more passed", so an outage of the host or of the target takes no WAN down.
- When no WAN takes flows, the aggregate route holds each WAN (the link state and the kill switch decide). The
  route is not removed for a fault of the probe.
- The probe is IPv4 only; a host name in `health.target` is resolved one time at the start.
- The guards keep their own nf_tables writer in `src/router/kernel/guard.rs`. `crates/podnet-netns/src/nfnl.rs`
  has a second builder with the same shape (`pub(crate)` in the pod backends crate). Joining them is a move of
  code between crates; it is not needed for this entry.
The measure on the wire found two defects of the MSS clamp of 84320e4: the key of `rt` was 2 (`nexthop6`), not 3
(`tcpmss`), and the rule had no `byteorder` (hton) before `exthdr`. The kernel accepted the rule, and the dump
showed the right names, but the MSS on the wire did not change. The rule now has the expressions of
`nft --debug=netlink` for the same rule.

The Prove of the work of 2026-11-14:

```sh
cargo test -p podnet --lib -- router::kernel
```

## Done

2026-11-11, commit 84320e4 (parts 1 to 3). `src/router/kernel/guard.rs` is new; the keys
`masquerade`, `mss_clamp`, `kill_switch` and `rp_filter` of `kernel` are `true` when absent, and an
old file stays valid.
- nf_tables netlink (no `nft` program): one batch (`NEWTABLE`, `DELTABLE`, `NEWTABLE`, chains, rules)
  replaces the table `ip podnet_router` as one transaction. Chain `nat` (postrouting, 100): `oifname
  WAN masquerade`. Chain `mss` (forward, -150): `oifname WAN tcp flags & (syn|rst) == syn tcp option
  maxseg size set rt mtu`.
- Kill switch: `unreachable default metric 4096` in the aggregate table. `read_default` skips it.
- `rp_filter` 0 on each WAN device; the first value is written back by `remove` and for a WAN that
  leaves the plan. A message names a value of `all` above 0.
Tests: `route_kernel_kill_switch_does_not_leak` (with the switch disabled in the code it fails at
"the kill switch is in place"; with it, each WAN down gives EHOSTUNREACH, the deleted switch gives
the leak through the main table, and `remove` deletes the route),
`route_kernel_nft_masquerade_and_mss_clamp_in_a_namespace` (reads the rules back with a netlink
dump; compares `rp_filter` and the write-back), `kernel_guard_keys_default_on_and_keep_old_files_valid`,
`kernel_guard_requests_encode_the_unreachable_route_and_the_batch`. The tests of T-119 still pass.
check.sh: exit 0.

Not proven: no packet crossed a WAN, so the masquerade and the MSS value on the wire are not
measured (the kernel accepts the rules and the dump shows them). The host sets `ip_forward` itself.
Part 4 is left (Correction).

2026-11-14, commit 16c7d1a (part 4, the measure on the wire, two fixes of the MSS clamp).
- `src/router/kernel/watch.rs` is new: the probe of each WAN by its fwmark, and the aggregate route of the WANs
  that take flows. `podnet route` runs it when the file has `kernel` and `health`.
- `src/router/kernel/netkit.rs` (tests only): namespaces held by a descriptor, veth pairs across two of them,
  addresses and routes by rtnetlink, and a capture of TCP SYNs with an `AF_PACKET` socket.
- `route_kernel_masquerade_and_mss_clamp_on_the_wire`: a client namespace, the router namespace and a far
  namespace behind two veth WANs (MTU 1280). Guards off: the far end sees the SYN from 10.9.0.2 with MSS 1460.
  Guards on: the source is the address of the WAN that carried it, the MSS is 1240, and the flow gets its
  answer. Without the fix it fails at "the MSS clamp" (1460, also with only one of the two fixes).
- `route_kernel_probe_takes_a_dead_wan_out_of_the_route`: the far end of WAN b loses its address (the link
  stays up); after 3 misses b is down and the aggregate route holds a only (`read_default`), and 8 flows of the
  router pass; one pass puts b back (probation), the next gives up; when each WAN fails, no WAN goes down. With
  the write of the route disabled it fails at the route of table 200.
- `cargo test -p podnet --lib -- router::kernel`: 12 passed, run here as root in `unshare -rn`. Where the host
  refuses a user and network namespace (GitHub runners, AppArmor) the namespace tests print a `skip:` line.
- `CC_i686_unknown_linux_musl=clang cargo check --workspace --target i686-unknown-linux-musl`: exit 0.
Not proven: a WAN that is a TUN of `warp tun` (the WANs of the tests are veth pairs). IPv6 is not routed by the
kernel mode.

# T-121: Router: export the groups as a JSON file of the mwan form

**Source:** `docs/research/podnet/mwan-field.md`, sections 2.2 (R4) and 5 (question 5).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The multi-WAN request asks for an export of the WANs in the JSON form of the mwan director.

## Premise

- The JSON form has `wans` (name, interface, table, weight, gateway) and `clients` (source, WAN)
  (`docs/research/podnet/mwan-field.md`, section 1.2).
- podnet routes by itself after T-100, so the export is for a host that runs the director.

## Approach

1. Add `podnet route export --format mwan-json` for the groups of `warp tun` WANs.

## Prove

```sh
cargo test --workspace -- route_export_mwan_json
```

## Start condition

T-100 is done.

## Correction

2026-11-11: the export is a flag of `podnet route`, not a subcommand: `podnet route --config FILE --export
mwan-json`, as `--check` and `--status` (`route` has no subcommands, T-100). The WANs are those of the key
`kernel` of the router file (T-119), not the WANs of `warp tun`: a WAN of the kernel mode names its
interface, its table and its gateway.

## Done

2026-11-11, 12fbc95. `Plan::mwan_json` gives `wans` (`name`, `iface`, `table`, `weight`, `gateway` when the file
has one, and `check_target` from `health.target`) and `clients` (empty: the router file has no source rules).
`route --export mwan-json` prints it, and exits 64 when the file has no `kernel` section. Test:
`route_export_mwan_json`. Measured: the binary printed the two WANs of a file, and exit 64 for a file with
no `kernel`.


# T-122: Research: a native git server with no installed `git`

**Source:** `docs/research/podnet/capabilities.md`, section 5.3 (design C); the owner's ruling of
2026-11-05 (`docs/decisions.md`, Git hosting).
**Category:** research
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

`podnet git serve` needs the `git` program on the host. A static podnet on a host with no `git`
cannot serve a repository.

## Premise

- `gix` has the client protocol and the pack code, but no complete server for upload-pack and
  receive-pack. `git2` (libgit2) is a C build with no server.

## Approach

1. Measure what a server on `gix` needs: protocol v2 negotiation, a pack writer, the receive and
   the ref update. Measure the size of the binary.
2. Write an entry for each part, or a question for the owner.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Git hosting): a native server comes later.

## Prove

```sh
cargo todo check
```

## Start condition

T-094 is done.

## Done

2026-11-11, commit 027537b. Research note: `docs/research/podnet/native-git.md`. Each fact is marked read,
measured or inferred, with URL and date. The local measures are lock files and source line counts of scratch
projects. No binary was built, so the size of the binary is not measured. No code was added.

- `gix 0.90.0` has no server for upload-pack and receive-pack. Its status page leaves "upload-pack /
  receive-pack server plumbing" and "general purpose `accept(...)` for servers" unchecked. It has the
  parts of the data layer: the pkt-line codec, pack writing, pack reading with thin-pack completion, ref
  transactions, bare init.
- `gix-pack` has no delta compression: it copies deltas from packs and sends loose objects whole. `gix` has
  no hook runner, no `fsck` checks, no bitmaps, no repack.
- Protocol: upload-pack needs v0, v1 and v2. Git itself ignores v2 in receive-pack and uses v0 (read in
  `builtin/receive-pack.c`), so push needs v0 and v1 only.
- Crates (measured, scratch lock files, musl normal graph): the data layer has 64 crates (about 39 not in the
  lock of podnet); `gix` with no default features 88; `gix` with default features 136. Each `gix` crate is
  `MIT OR Apache-2.0`. No graph links ring, aws-lc, openssl or a C build (`sha1dc` has `build = false`). Keep
  the HTTP client features of `gix` off. ring against aws-lc does not arise for the server.
- podnet writes about 3,700 lines (inferred): the advertisements, `ls-refs`, upload-pack v0, v1 and v2, want
  validation, the pack stream, receive-pack, the push ingest with connectivity check, the ref update with the
  report, the glue for HTTP and SSH, and a maintenance pass. Other native servers are fetch-only or young
  (`ggueret/git-server`, `git-internal`, `guts-git`).
- The call: a `--git-backend git|native|auto` trait. The `git` program stays the default where it exists.
  Write upload-pack first, then receive-pack.
- Not proven: the binary size, the speed on a large repository, `cargo deny`, the line counts (estimates).
- Proposed entries (the lead opens them; each needs the ruling of the owner, note section 8): G1 spike of the
  data layer in a scratch binary, size and lock (S); G2 backend trait and `--git-backend`, `doctor` reports it
  (M); G3 native upload-pack, v0, v1, v2, HTTP and SSH (L); G4 native receive-pack with the connectivity check
  and the ref update (L); G5 native hooks and `git-upload-archive` (M); G6 pack quality, a repack with deltas
  (M).


The call of the lead (2026-11-11): no entry is opened now. The `git` program stays the backend: hosts of
`docs/target-environment.md` with no `git` are not measured as a need yet, and a native server is about 3,700
lines of new code. G1 (the spike of the size) opens first when a need is measured or the owner asks.

The owner, 2026-11-11 (`docs/decisions.md`, Native git server): not now in podnet. The latest facts are in
the update of `docs/research/podnet/native-git.md` (grit-lib 0.5.33). A crate `pod-git` of
`PodUtils/podcrates` holds a native server later: PodUtils/podnet#453 gives its contract and its acceptance.


# T-123: Tailscale in the process: take the fork of tailscale-rs through `pod-ts`

**Source:** `docs/research/podnet/podssh.md`, sections 2.1 and 2.3 (step 4);
`docs/research/podnet/podssh-copy.md`, sections 2.5 and 3 (C19); the owner's ruling of 2026-11-05
(`docs/decisions.md`, Tailscale).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

The tailnet works only through a `tailscaled` sidecar. A host where no `tailscaled` can run has no
tailnet.

## Premise

- podssh carries a fork of tailscale-rs at `f4781c4` with 29 patches
  (`Azathothas/podssh:vendor/tailscale-rs`, `Azathothas/podssh:vendor/patches/`), with a policy
  page and two weekly checks of the copies.
- The fork is BSD-3-Clause with PATENTS. Upstream declines outside changes.

## Approach

1. Copy the fork and its patches into a vendor directory, with its own workspace, out of the main
   workspace and out of the size check.
2. Write the vendor policy: the upstream, the commit, the licence, why it is copied and how to copy
   it again.
3. Copy the two checks of podssh (new upstream releases, advisories) as weekly jobs.
4. Add the notice of the licence to `THIRD_PARTY_NOTICES.md` and `about.toml`.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Tailscale): an in-process Tailscale from podssh comes
later, behind a feature that is off by default.

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the shared crates of `PodUtils/podcrates` hold this code. podnet depends on the shared crate; it does not copy the code.

## Prove

```sh
cargo metadata --manifest-path vendor/tailscale-rs/Cargo.toml --format-version 1 > /dev/null
python3 -I scripts/check-vendored.py
```

## Start condition

T-114 is done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The work goes to `pod-ts` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. Do not copy code of podssh into podnet (`docs/decisions.md`, Shared crates). The code comes through PodUtils/podcrates. The fork and its patches are the code of `pod-ts` (wave 4). The
vendor policy and the weekly checks of the copies are podcrates work. The remaining work of podnet: the
feature that is off by default, and the notices of the licence (T-124).

# T-124: Tailscale in the process: an adapter crate behind a feature that is off by default, with the static build proven

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.5, 3 (C19) and 3.3; the owner's
ruling of 2026-11-05 (`docs/decisions.md`, Tailscale).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

## Problem

The fork of T-123 needs an adapter, flags and a proof that it builds for the targets of podnet.

## Premise

- The adapter of podssh is `Azathothas/podssh:crates/podssh-ts` (895 lines and 46 tests).
- The fork links aws-lc (C, cmake on some targets), needs Rust 1.92, and adds about 420 packages.
  No build for i686, arm, loongarch64 or powerpc64le is proven.
- `cargo deny` would need an ignore of RUSTSEC-2024-0436 (`paste`) and an MPL-2.0 exception. The
  owner's ruling keeps `cargo deny` green with no ignored advisory.

## Approach

1. Copy the adapter as a crate of podnet, behind a feature `ts` that is off by default and out of
   the default members.
2. Add the network `ts:` to the registry, with `--ts-auth-key-file`, `--ts-state`,
   `--ts-ephemeral` and `--ts-control-url`.
3. Add an MSRV check of 1.92 for the feature.
4. Build the feature for the eight targets with cargo-zigbuild. Try a patch that names ring in
   place of aws-lc.
5. Solve the advisory of `paste` with no ignore, or ask the owner.
6. Test it in the headscale harness of T-114.

## Prove

```sh
cargo test --features ts -p podnet --test ts_headscale
cargo deny check
gh workflow run release.yml --ref main
```

## Start condition

T-123 is done.

## Correction

2026-11-05, The adapter is `pod-ts` (wave 4 of PodUtils/podcrates), not a copy of `podssh-ts`. Step 1 becomes:
take `pod-ts` behind the feature `ts`, off by default. The MSRV check of 1.92 in step 3 is not necessary:
the toolchain is Rust 1.99 and `rust-version` is 1.99. A dependency by a git tag needs `allow-git` in
`deny.toml` (it has it for podcrates).

# T-125: A userspace WireGuard responder: `podnet wg serve`

**Source:** `docs/research/podnet/vn.md`, sections 3.2 and 5 (question 3).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

podnet can dial a WireGuard peer, but cannot be one. A WireGuard endpoint with no
`CAP_NET_ADMIN`, whose inner flows go to podnet networks, does not exist.

## Premise

- `src/warp/wg.rs` has the initiator, and a responder for its tests (`respond`).
- `crates/podnet-vnet/src/device.rs` has a link device for smoltcp.

## Approach

1. Add the responder half: cookie replies, several peers and the timers.
2. Send each inner flow to a dialer of the registry.
3. Add `podnet wg serve` with a key file and a list of peers.

## Prove

```sh
cargo test --workspace -- wg_serve_accepts_a_peer_and_dials_its_flows
```

## Blocker

The session of `PodUtils/podcrates` (the crate `pod-wg`): a responder sans I/O for production use (the ask of the
Correction of 2026-11-14). The entry is unblocked when a tag of podcrates has it; then podnet takes the tag (T-137)
and does steps 2 and 3.

## Start condition

T-117 is done.

## Correction

2026-11-05, the responder half of the WireGuard code (`src/warp/wg.rs`) is in `pod-wg` (wave 2). The work goes to
`pod-wg` in PodUtils/podcrates; podnet takes it and adds `podnet wg serve` (T-137).

2026-11-12, GitHub #469 (row E3-3). The listen half of the WireGuard carrier (T-283) needs the responder of `pod-wg` that this entry takes.

2026-11-14, blocked. T-117 is done, so the start condition holds. Read in the tree (`pod-wg` v0.41.0, the tag of
podnet): the crate is "the WireGuard initiator of WARP, sans I/O". Its only responder is the module `standin`
(feature `standin`, "for the tests of a consumer; no binary needs it"): `standin::respond` panics when an initiation
fails a check, it takes one client key and no list of peers, it keeps no last timestamp of a peer (no replay
check), it has one fixed cookie (`start_under_load`) and no secret that changes, and it has no timers of a
responder session. Step 1 (cookie replies, several peers, the timers) is protocol code of WireGuard, and the
Correction of 2026-11-05 puts it in `pod-wg`. podnet does not write it (`CLAUDE.md`, Decisions, Order). Steps 2 and
3 have nothing to run without it: they wait. The call of the session: no part of `podnet wg serve` lands before the
responder (a command with no responder is dead code).

The ask for podcrates (the lead opens the COORD issue): in `pod-wg`, a responder sans I/O, beside the initiator:

- a responder with its private key and a list of peer public keys (an optional preshared key each, as T-337 asks
  for the initiator);
- the read of an initiation: mac1 for the own key; the static key of the initiator, decrypted and found in the
  list (an unknown key gives no answer); the TAI64N timestamp of the peer must be past its last one (no replay);
  the response and a session for that peer;
- under load (a flag of the caller): an initiation with no valid mac2 gets a cookie reply, with a cookie from a
  secret that changes each 120 s and the address of the sender (the WireGuard paper, section 5.4.7);
- the transport: the session of a message by its receiver index, for several peers at once, with the window of the
  counters;
- the timers of the responder side: a session ends after `REJECT_AFTER_TIME` (180 s); a keepalive; no rekey
  initiation from the responder;
- a `Debug` that shows no key, and a fuzz target for the read of an initiation.

podnet's part after the take: `podnet wg serve FILE` with a wg-quick file (`[Interface]` `PrivateKey` and
`ListenPort`, one `[Peer]` for each peer with `PublicKey` and `AllowedIPs`; the reader
`crates/podnet-vnet/src/wg_quick.rs` keeps several peers, and it skips `ListenPort` today), a UDP socket, a
userspace stack of `pod-netstack` for the inner packets, each inner TCP flow to a dialer of the registry
(`--egress SPEC`, default the way out of the process). Tests on the loopback: a `pod-wg` initiator reaches a TCP
echo through `podnet wg serve`; a peer with an unknown key gets no answer; a plant.
# T-126: Egress through an `https://` proxy and through wstunnel

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.4 and 3 (C16).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

`src/egress.rs` refuses an `https://` proxy, so a host whose proxy needs TLS cannot use podnet.
podnet cannot use a wstunnel server.

## Premise

- podssh has the road `via` (an HTTPS proxy over TLS, with CONNECT over HTTP/1.1 and HTTP/2) and
  a wstunnel client (`Azathothas/podssh:crates/podssh-ws/src/via/`,
  `Azathothas/podssh:crates/podssh-ws/src/wstunnel/`).
- A wstunnel token is a JWT with HS256. `hmac` and `sha2` are dependencies.

## Approach

1. Copy and change the road `via` as a dialer: `egress::dial`, TLS with ring, credentials from a
   file.
2. Copy and change the wstunnel client as a dialer.
3. Add both to the registry.

## Prove

```sh
cargo test --workspace -- https_proxy_dialer_through_a_test_ca wstunnel_dialer_
```

## Start condition

T-059 is done.

## Correction

2026-11-05, The work goes to `pod-egress` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. Do not copy code of podssh into podnet (`docs/decisions.md`, Shared crates). The code comes through PodUtils/podcrates. The egress roads (`via`, wstunnel) are in the design of
`pod-egress` (wave 4). `src/egress.rs` moves to `pod-proxy`. Read both crates before the work.

2026-11-11, Checked: the crate `pod-egress` is not in `PodUtils/podcrates` (`crates/` has no such directory), and
`sh scripts/agent/podcrates.sh` lists no hand-off for it. The Approach needs podssh code, and the Correction above
forbids a copy into podnet. No part of this entry is possible in podnet now. Nothing was changed in the code. The
entry stays `open`. It starts when `pod-egress` has a release (wave 4) and podnet takes it (T-137).

# T-127: Egress through VLESS, Shadowsocks and Wisp

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.4 and 3 (C16).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

## Problem

podnet cannot use a VLESS, Shadowsocks or Wisp server that a user runs.

## Premise

- podssh has the three clients (`Azathothas/podssh:crates/podssh-ws/src/vless/`,
  `Azathothas/podssh:crates/podssh-ws/src/shadowsocks/`,
  `Azathothas/podssh:crates/podssh-ws/src/wisp/`).
- podssh uses RustCrypto AEAD crates. podnet has the AEAD of ring.

## Approach

1. Copy and change each client as a dialer, one commit for each. Use the AEAD of ring.
2. Add each to the registry. Test each against its server in a workflow.

## Prove

```sh
cargo test --workspace -- vless_dialer_ shadowsocks_dialer_ wisp_dialer_
```

## Start condition

T-059 is done.

## Correction

2026-11-05, The work goes to `pod-egress` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. Do not copy code of podssh into podnet (`docs/decisions.md`, Shared crates). The code comes through PodUtils/podcrates. The Approach says "copy": it means "take from the shared crate".

2026-11-12, GitHub #471 (rows E5-5 and E5-6). A Shadowsocks service (SIP004) and a Trojan service at a peer need the accept side of the codecs of `pod-egress`: T-294 and T-295, blocked on the take of `pod-egress`. Ask A4 for podcrates: the chunk codec of SIP004 and the Trojan request codec as parts that work on the accept side, with the published SIP004 vectors in the tests.
# T-128: The DERP carrier through `pod-derp`

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.6 and 3 (C17).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

## Problem

Two podnet hosts can meet only through the podnet relay or a quick tunnel. The public DERP servers
of Tailscale need no account and run on TCP 443.

## Premise

- podssh has a DERP carrier between two keys, with its own ARQ under the resumable layer
  (`Azathothas/podssh:crates/podssh-relay/src/derp/`, 3,543 lines). It needs `crypto_box 0.9`.

## Approach

1. Take `pod-derp` of PodUtils/podcrates. Write the glue to the relay client (`pod-relay`).
2. Use it as a carrier of the layer of T-086 and of a transfer.
3. Test it against `derper` in a workflow.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the shared crates of `PodUtils/podcrates` hold this code. podnet depends on the shared crate; it does not copy the code.

## Prove

```sh
cargo test --workspace -- derp_
cargo deny check
```

## Start condition

T-086 is done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The work goes to `pod-derp` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. There is no crate `podnet-relay`: the relay client is `pod-relay`.

# T-129: An ACME client for a public listener

**Source:** `docs/research/podnet/podssh-copy.md`, section 3 (C18).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

The file server and git on a public port need a certificate that browsers and git accept.

## Premise

- podssh has an ACME client with TLS-ALPN-01 and keys of aws-lc-rs
  (`Azathothas/podssh:crates/podssh-cli/src/acme/`, 1,372 lines).
- podnet has ring and rcgen.

## Approach

1. Take `pod-acme` of PodUtils/podcrates (wave 4) in `podnet-serve`. Keys come from ring and rcgen.
2. Add `--acme DOMAIN` to `serve` and `git serve` on a public TCP listener.

## Prove

```sh
cargo test -p podnet-serve -- acme_with_a_test_directory
```

The test runs against a local ACME test server (pebble) in a workflow.

## Start condition

T-091 is done.

## Correction

2026-11-05, The work goes to `pod-acme` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. Do not copy code of podssh into podnet (`docs/decisions.md`, Shared crates). The code comes through PodUtils/podcrates.

# T-130: Research: the iroh road, and a relay and mailbox of podnet's own

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.6 and 3 (C20, C21).
**Category:** research
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

podssh has an iroh road and a relay role with a mailbox. podnet has neither, and the owner has
not ruled on them.

## Premise

- The iroh road links aws-lc (C), needs Rust 1.91 and adds about 520 packages.
- The relay role of podssh has about 14,000 lines on its own HTTP stack.

## Approach

1. Measure the build of iroh with ring, if a feature allows it, and the size of the binary.
2. Measure the port of the relay role onto the HTTP stack of podnet.
3. Write a question for the owner with a recommendation, and an entry for each part that the
   owner accepts.

## Prove

```sh
cargo todo check
```

## Correction

2026-11-05, The iroh road is `pod-iroh` (wave 4 of PodUtils/podcrates) and the relay role is
`pod-relay-serve`. The toolchain is Rust 1.99, so the line "needs Rust 1.91" of the Premise is not a
constraint. The measures of the Approach stay: they decide whether podnet takes these crates.

## Done

2026-11-11, commit 6a62104. Research note: `docs/research/podnet/iroh-relay.md`. Each fact is marked read,
measured or inferred, with URL and date. No account was used. No binary was built, so the size of the
binary is not measured.

- The iroh road does not need aws-lc. `tls-ring` is a default feature of `iroh` and `iroh-relay`. Only
  `tls-aws-lc-rs` links `aws-lc-sys`. With `default-features = false` and `tls-ring`, the road adds 63
  crates to the lock of podnet (measured with a scratch lock file). The "about 520 packages" is the whole
  lock of podssh. Rust 1.91 is no limit.
- Through a proxy-only egress, iroh works only over its relay: `CONNECT` to the relay name, then a
  WebSocket upgrade to `/relay`. It takes an `http://` or `https://` proxy URL only. A SOCKS5, unix or
  exec network spec needs a shim. Public n0 relays are for development and hobby use, with rate limits.
- The relay role of podssh (about 14,150 lines) should not be ported whole. Its own TLS stack and its
  QUIC, HTTP/2 and cleartext roads are not needed. The mailbox server (629 lines) is the first small
  part. The call: B and C go on after the takes of T-137; the iroh road waits for `pod-iroh` and a spike;
  the self-hosted iroh relay is declined.
- Not proven: the binary size, the start on a host with no UDP `bind` or netlink, the speed through a
  proxy, the line count of each part of the relay role, `cargo deny` on the 63 crates.
- Proposed entries (the lead opens them; each needs the ruling of the owner): E1 spike of an iroh
  `Endpoint` with ring (binary size, sandbox start, path through a `CONNECT` proxy to a local
  `iroh-relay`) (M); E2 `podnet relay serve --mailbox`, the magic-wormhole mailbox server on axum with an
  interop test (M); E3 `podnet relay serve` forward road and mint on the take of `pod-relay-serve` (L);
  E4 `send --mailbox URL` and `receive --mailbox URL` (S); E5 the iroh road as a network row behind a
  feature, with a `CONNECT` shim (L, after E1 and `pod-iroh`).
- Questions for the owner (note, section 6): accept the mailbox server; accept the relay role in parts;
  defer the iroh road; decline the self-hosted iroh relay.


The call of the lead (2026-11-11): no entry is opened now. E1 and E5 wait for a release of `pod-iroh`. E2 and E4
go with the transfer (T-102 to T-105: the mailbox client, `send` and `receive`), which waits for the takes of
`pod-wormhole` and `pod-transit`. E3 waits for the take of `pod-relay-serve`. A self-hosted iroh relay is declined.
# T-131: A crate that podnet and podssh share for the layer, the channel and the identity

**Source:** `docs/research/podnet/podssh-copy.md`, section 4 (option B); the owner's ruling of
2026-11-05 (`docs/decisions.md`, Code of podssh).
**Category:** chore
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

After T-086, podnet and podssh keep two copies of the resumable layer, the end-to-end channel and
the identity. A fix in one is ported by hand to the other.

## Premise

- After the changes of T-086, the copies use no podssh-ws type (about 9,000 lines).
- `deny.toml` denies git sources. The owner ruled no publish on crates.io yet.

## Approach

1. Extract the layer, the channel and the identity into one crate that both projects use.
2. Agree the source with the owner: a git dependency (which needs an allow in `deny.toml`) or a
   published crate.
3. Make podnet use it, and remove its copy.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Code of podssh): copy podssh code now; a shared crate
comes later.

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the shared crates of `PodUtils/podcrates` hold this code. podnet depends on the shared crate; it does not copy the code.

## Prove

```sh
cargo test --workspace
cargo deny check
```

## Start condition

T-137 takes `pod-session`, `pod-e2e` and `pod-identity` (the owner chose the source: a git tag).

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The shared crates exist in PodUtils/podcrates: `pod-session` (the resumable layer), `pod-e2e`
(the end-to-end channel), `pod-identity` (the key identity). The owner chose the source: a git tag
(`docs/decisions.md`, Delivery of the shared crates). This entry is done by T-137, step 2. Close it
with the commit that takes `pod-session`, `pod-e2e` and `pod-identity`. podnet has no copy to remove.

2026-11-13: `pod-identity` and `pod-session` are taken (T-261, T-263, `Cargo.toml` at v0.36.0). `pod-e2e` is not taken. The entry closes with the take of `pod-e2e` (T-086).

2026-11-13: `pod-e2e` is a dependency of podnet since T-298 (ea9b6c0, `Cargo.toml` at v0.39.0). The start
condition holds. The entry closes with T-086.

## Done

2026-11-13, by T-137 step 2: podnet depends on `pod-identity` (b33718d), `pod-session` (75b49ba) and `pod-e2e` (ea9b6c0)
of PodUtils/podcrates by the git tag (v0.39.0). podnet had no copy to remove. ci 43459872995 (each job, with `cargo test
--workspace`) and deny 43459879363 on bf01ac6 passed.

# T-132: `try_update` in place of the macro `update_atomic!`

**Source:** `docs/maintenance.md`, section "Open work" (`try_update`).
**Category:** chore
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`src/lib.rs` has the macro `update_atomic!`, a loop of `compare_exchange_weak`, in place of the
deprecated `fetch_update` (#214). `crates/podnet-serve/src/share/mod.rs` and `crates/podnet-serve/src/gate.rs` use it.

## Premise

- The MSRV is 1.99 (`Cargo.toml`, `rust-version`).
- The standard library has `try_update` on the atomic types as a stable API from a later release.

## Approach

1. Replace each use of the macro with `try_update`.
2. Remove the macro.

## Prove

```sh
cargo check --workspace
sh scripts/agent/check.sh
```

The toolchain is Rust 1.99.1, the same as the MSRV.

## Start condition

The `rust-version` of `Cargo.toml` is at or above the release in which `try_update` is stable.

## Correction

2026-11-05, `try_update` is stable at Rust 1.99.1, the `rust-version` of `Cargo.toml` (a test program with
`AtomicUsize::try_update` compiles). The start condition holds. The work waits only for the ruling
"Order".

## Done

2026-11-11, commit 7963170. The macro `update_atomic!` is removed from `src/lib.rs`,
`crates/podnet-core/src/lib.rs` and `crates/podnet-serve/src/lib.rs`. The three uses call
`AtomicX::try_update(Ordering::SeqCst, Ordering::SeqCst, f)`: `PinGate::unlock` in
`crates/podnet-serve/src/gate.rs`, and `reserve_bytes` and the file-slot reserve in
`crates/podnet-serve/src/share/upload.rs`. The result is the same: `Ok(old)` when `f` gives `Some`,
`Err(current)` when `f` gives `None`; the orderings are the same as the macro. No frozen file used
the macro. `src/warp/live.rs` (frozen) has a test loop of `compare_exchange_weak` with a comment
about #214; it is not changed. `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers
and test are 0. No new test: the behaviour is unchanged, and the tests of the gate and the upload
limits cover the three uses. Not proven: a run under load of the limit tests.

# T-133: Zig from the community mirrors in the release workflow

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.9 and 3 (C13).
**Category:** chore
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

`.github/workflows/release.yml` downloads zig from ziglang.org only. An outage of that site stops
a release.

## Premise

- podssh gets zig from the community mirrors in a random order, then from ziglang.org, and checks
  the SHA-256 (`Azathothas/podssh:scripts/fetch-zig.sh`,
  `Azathothas/podssh:scripts/zig-mirrors.txt`). A weekly check reads the list of mirrors.

## Approach

1. Copy the script and the list of mirrors. Change the names.
2. Use the script in `release.yml`.
3. Copy the weekly check of the mirror list.

## Prove

```sh
ZIG_SOURCE=podnet sh .podcrates/scripts/fetch-zig.sh 0.16.0 SHA256 zig.tar.xz
gh workflow run release.yml --ref main
```

## Correction

2026-11-05, the owner (`docs/decisions.md`, Shared CI scripts): the zig fetch script, its list of
mirrors and the weekly check of the list live in `PodUtils/podcrates`, not as a copy in podnet.
podnet's part: when podcrates has the script, `release.yml` calls it from podcrates (pinned to a
tag or a commit). When podcrates does not have it, open an issue there. Steps 1 and 3 are void in
podnet.

## Done

2026-11-05, 7ac7681 and 5535ffb. The job `linux-zig` of `release.yml` checks out the scripts of
`PodUtils/podcrates` at 9c9c1e345112b13f340e8502a69b807501f34f42 and runs
`ZIG_SOURCE=podnet sh .podcrates/scripts/fetch-zig.sh` (the mirrors, then ziglang.org, with the
pinned SHA-256). podcrates checks the mirror list each day (its `upstream.yml`, job `mirrors`).
The release dry run 42074286318 (5535ffb) passed each job: the nine static targets (armv7 now
with cargo-zigbuild), the image and the publish steps of a dry run.

# T-134: A weekly watch of the relay contract

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.10, 3 (C14) and 4.
**Category:** chore
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

A fix of podssh in a file that podnet copied does not reach podnet. A change of the contract of
the relay (`tcp.ssh.relay.ajam.dev`) breaks the relay commands with no warning.

## Premise

- `tools/check-upstream.py` and `src/cloudflare/cloudflared-pin.txt` watch the copied parts of
  cloudflared each week (`.github/workflows/upstream.yml`).
- podssh watches the relay contract against pinned facts
  (`Azathothas/podssh:scripts/check-relay-spec.py`).
- `CLAUDE.md` says: read GitHub through the services of `https://web.pkgforge.dev/llms.txt`.

## Approach

1. Write a pin file beside each podssh copy: the source path, the podssh commit and the SHA-256.
2. Write a watch script from `tools/check-upstream.py`. It reads podssh `main` through
   `forge.web.pkgforge.dev` and names each source that changed. A changed file becomes an entry,
   not a copy. `--plant-sum` must fail.
3. Write the facts of the relay paths that podnet uses (`/v1/pair`, `/v1/node`, `/v1/connect`,
   the mint and the limits) and check them against `https://tcp.ssh.relay.ajam.dev/llms-full.txt`.
4. Run both in `.github/workflows/upstream.yml`.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the shared crates of `PodUtils/podcrates` hold this code. podnet depends on the shared crate; it does not copy the code.

## Prove

```sh
python3 -I tools/check-podssh.py
python3 -I tools/check-podssh.py --plant-sum   # must exit 1
gh workflow run upstream.yml
```

## Start condition

T-086 is done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, podnet copies no file of podssh (`docs/decisions.md`, Shared crates): steps 1 and 2 are
void. The owner (`docs/decisions.md`, Relay contract watch): the weekly watch of the relay contract
(step 3, against `https://tcp.ssh.relay.ajam.dev/llms-full.txt`) belongs to `pod-relay` in
`PodUtils/podcrates`, not to podnet. podnet's part: check that podcrates has the watch, else open
an issue there; then set this entry to done.

## Done

2026-11-12, GitHub #463 (COORD-043, to podcrates). podnet's part of the Correction: the checkout of podcrates of
2026-11-12 has no watch of the relay contract (`.github/workflows/`: no workflow reads
`https://tcp.ssh.relay.ajam.dev/llms-full.txt`; `upstream.yml` watches the sources and the zig mirrors only).
#463 asks podcrates to add the watch to `pod-relay`. Steps 1 and 2 stay void (Correction of 2026-11-05).

# T-153: netns: a backend with an in-namespace netfilter data plane (GitHub #352)

**Source:** GitHub #352 (https://github.com/PodUtils/podnet/issues/352)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

Same as T-152. The goal: kernel-delivered TCP and UDP for the netns backend, no smoltcp on the hot path,
no flow table of T-111.

## Premise

- Depends on T-152. T-111 (open, M7) plans the userspace flow table. This entry must not replace it
  before the owner rules (see Decision).
- The nfnetlink client is new code (the vnet ACL avoids kernel netfilter, `crates/podnet-vnet/src/policy.rs`). T-120
  plans an nf_tables writer for root-host router mode (`TODO/later.md`).
- Files of `podnet-netns` stay under 800 lines: new files for the client and the listeners.
- ICMP echo cannot be REDIRECTed (martian). Ping works through the ping-socket path only.

## Approach

1. Add `--backend netns-kernel` (or a mode). In the child, after unshare and before exec: install the
   rules, make the listeners, send their descriptors to the parent over the `SCM_RIGHTS` pair.
2. The parent serves them with `connect_upstream` (`upstream.rs:346`) per flow.
3. Set `ping_group_range` to the mapped gid range.
4. Add the doctor class and the `auto` order (`lib.rs:197-205`). Update the README.

## Decision

The lead, 2026-11-09: the netfilter plane is a parked option, not a replacement of the flow table of T-111. Decide after the measurement of the entry before it. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test --test pod_netns_kernel -- netns_kernel_relays_tcp netns_kernel_two_udp_flows_to_one_port_lose_nothing
```

## Start condition

T-152 gives a pass on the target hosts, and the lead decides from the result (see Decision).

## Correction

2026-11-12, the lead: the row of T-152 on the CI runner (ubuntu-24.04, ci 43183892180, job `test`): "netfilter in a
namespace (not a backend) fail  the uid and gid maps: Permission denied (os error 13)". The runner refuses the user
namespace, as it does for the netns backend. A netfilter plane thus works only where the netns backend works, and
gives speed only (no smoltcp on the hot path), not a new host. The entry stays parked, beside the flow table of T-111.
Revisit when a measure shows the netns backend too slow on a target host.

2026-11-13, the lead: the goal of the owner (close each issue except the deferred ones) takes the parked option. The
backend is an opt-in `--backend netns-kernel`, never in the `auto` order: the flow table of the netns backend stays the
default, as the Decision says.

2026-11-14: `nfnl.rs` gave an `i64` to `timeval.tv_sec`, an `i32` on the 32-bit musl targets: the release dry run
43541138625 did not compile there (T-336). Fixed: the value is clamped to `i32::MAX` and converted to the type of the
field.

## Done

2026-11-13, commit 085f5c8 (the work of the handoff, rebased on a48d3e7). `--backend netns-kernel`: in the child, the
nfnetlink client (`crates/podnet-netns/src/nfnl.rs`, `nfrules.rs`) installs `nat OUTPUT REDIRECT` rules for TCP, UDP and
DNS; the child makes the listeners and sends them to the parent; the parent serves each flow with `connect_upstream`
(`kernel.rs`, `kernel_serve.rs`). `pod doctor` and the README of `podnet-netns` name the backend. `cargo test --test
pod_netns_kernel`: 6 passed, on a host that runs the netns backend (no skip line). `cargo clippy --workspace
--all-targets -- -D warnings`: clean.

# T-165: The SSH forward-relay leg: supervise the session after `run()` returns (GitHub #362)

**Source:** GitHub #362 (https://github.com/PodUtils/podnet/issues/362)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

`relay::run` (`crates/podnet-serve/src/relay.rs:442`) dials the relay, requests one remote forward, reads the URL from the
banner and returns `(TunnelHandle, Handle<Client>)`. Nothing watches the session after that. When the
relay ends the connection or the forward, the URL is dead and the caller is not told. The default
relay's free tier ends the tunnel after 60 minutes (the banner text of the test at `crates/podnet-serve/src/relay.rs:581`).
A reconnect gives a new port and a new URL. The reverse road (`relay node`, `src/relay_reverse.rs`)
reconnects with a 1 s to 30 s backoff; this leg does not.

## Premise

- `inactivity_timeout` is 3600 s (`crates/podnet-serve/src/relay.rs:454`); the forward is requested once (`:484-487`).
- `grep -rn "relay::run\|RelayConfig" src tests examples` finds no caller outside `crates/podnet-serve/src/relay.rs`.
  `ShareLink::from_relay` (`crates/podnet-serve/src/feature/share_link.rs:118`) has no caller outside a test. No CLI uses the leg.
- Not measured: the 60 minute expiry on a live relay; the `Handle::is_closed` behaviour of russh 0.64.1.

## Approach

1. A supervisor task: detect the end (`Handle::is_closed`, a failed keepalive, EOF of the banner
   channel), then reconnect with the capped backoff of `relay_reverse.rs`.
2. Give the new URL to the caller (a channel or a callback) at each reconnect.
3. Log one line that names the relay and the reason of the end.
4. `doctor`: add one line to the row at `src/doctor.rs:750-756` about the fixed session age.
5. Do not hardcode 60 minutes.

## Decision

The owner, 2026-11-11 (`docs/decisions.md`, The SSH forward-relay leg): the leg gets a CLI. This replaces the call of
the lead of 2026-11-09.

## Prove

```sh
cargo test -p podnet relay::tests::the_supervisor_reconnects_and_announces_the_new_url
```

The test uses a local SSH server of russh that closes the session, and checks two URLs.

## Start condition

None: the owner ruled. The relay facts of T-166 come from `pod-services`; until then the CLI takes the
relay, the user and the URL rule as flags.

## Correction

2026-11-11: Approach 4 (a `doctor` line about the fixed session age) is not done. The rows of `src/doctor.rs`
that the entry cites are the legs of the edge. `doctor` has no probe of the SSH relay, so the line has no row
to join. Add the line when `doctor` gets a probe of this relay (T-166 gives the relay as data).

2026-11-11, the lead: the entry is partial, not done: Approach 4 (a row of `doctor` for the SSH relay)
is left. The Done section below holds what is built.

## Done

2026-11-11, commit 6126bf6. The command `podnet ssh-relay` runs the leg and keeps its session.
- `--server HOST:PORT`, `--user USER` and a URL rule (`--url-match TEXT`, `--url-skip TEXT`, each repeatable,
  at least one of the two) are required flags. A missing flag is a usage error (exit 64) that names the flag
  (clap). A bad `HOST:PORT` is exit 64 too. `--socket PATH` (the origin), `--connect-proxy HOST:PORT` and
  `--json` are optional. The command has `--detach`, `--name`, `--ready-timeout`, `--replace` and
  `--no-restart`. The help group is "Publish a local service".
- `relay::supervise::supervise` opens a session (`relay::open`, which keeps the banner channel), waits for the
  end (the banner channel closes, or `Handle::is_closed`, polled each second), and opens the next session
  after a backoff of 1 s doubled to 30 s. A session that lasts 10 s resets the backoff. It sends
  `Event::Up(TunnelHandle)` and `Event::Down { reason }` over a channel, and logs one line (the relay and the
  reason). It returns when the receiver is dropped.
- The command sets a keepalive of 30 s and no inactivity limit (`RelayConfig::inactivity`, `keepalive`), so no
  fixed 60 min is in the code. `relay::run` and its default (inactivity 3600 s) are as before.
- The ready document is `{url, remote_port, relay}`. After a reconnect the command prints the new URL and puts
  it into the record, so `podnet ps --json` shows it (`name` and `pid` stay).
- `relay::UrlRule` (`skip`, `hold`) replaces the fixed rule of `pick_tunnel_url`. Its default is the old rule.
- The tests of `relay.rs` moved to `relay/tests.rs`.

Tests (the three reconnect tests of `relay::tests` and the test of the binary failed with the reconnect disabled,
`return` before the sleep, then passed; 3 runs of each):
- `cargo test -p podnet-serve relay::tests` (14 passed): `the_supervisor_reconnects_and_announces_the_new_url`
  (a fake russh relay on loopback closes the banner channel of connection 1; the events are Up tunnel1, Down,
  Up tunnel2, and then none), `the_supervisor_sees_a_connection_that_the_relay_ends`,
  `the_supervisor_tries_again_when_the_relay_is_not_there`,
  `the_supervisor_ends_when_nobody_listens_to_its_events`, `a_url_rule_skips_and_holds_texts`,
  `the_backoff_doubles_to_its_limit`.
- `cargo test --test ssh_relay` (2 passed): `ssh_relay_detach_reconnects_and_ps_shows_the_new_url` (the
  binary with `--detach --no-restart` against a fake relay; the new URL is in `podnet ps --json`; with the
  supervisor disabled the instance ends and the test fails), `ssh_relay_with_a_missing_flag_is_a_usage_error`
  (exit 64, the flag in the message).
- `cargo test -p podnet ssh_relay` (unit tests of the parse and of the record amend).
- `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test are 0.

Not proven: a live relay (the 60 min expiry and the `is_closed` behaviour of russh on a real relay), the
keepalive that finds a dead session (the fake relay ends sessions with a close, not with silence), and a
connection through `--connect-proxy`.

2026-11-11, commit 7395b6a. Approach 4 is done: `podnet doctor` has a row `ssh_relay`.
- `relay::probe::probe` (`crates/podnet-serve/src/relay/probe.rs`) runs four stages with one time limit: the TCP
  connect (through `--connect-proxy` when the config has one), the SSH identification line, the SSH handshake and
  the publickey auth with the user. It opens no session channel and asks for no forward, so the relay publishes no URL.
  It gives the time of the connect and the total.
- The row (`src/doctor/ssh_relay_check.rs`) has the verdicts `ok` (each stage passed), `fail` (a stage was refused,
  or the server is not SSH, or the user was rejected) and `unknown` (a stage had no answer in three times `--timeout`,
  or no relay is given, with the reason `no SSH relay is given`). The doctor has no `warn` verdict, so the row has none.
  The detail names the failed stage.
- The relay comes from `--ssh-relay HOST:PORT` with `--ssh-user USER` (a bad address is exit 64), else from the first
  live `ssh-relay` instance of `podnet ps` (its `--server`, `--user` and `--connect-proxy`). T-166 will give the relay
  as data.
- `doctor --json` has the new key `ssh_relay` (`docs/protocol.md`, section 9): `verdict`, `status`, `detail`, `relay`,
  `user`, `source`, `connect_ms`, `banner`, `auth`, `total_ms`. No key was removed. The text output has one line.
- The row is not part of `ok`, `legs`, `roads` or `lanes`: a tunnel does not need this relay.

Tests (each failed first: `probe` disabled for the five probe tests; the key `ssh_relay` removed from the JSON for the
three doctor tests; then all passed):
- `cargo test -p podnet-serve relay::tests::probe_tests` (5 passed, fake russh relay on loopback):
  `the_probe_passes_each_stage_of_a_good_relay`, `the_probe_names_the_connect_stage_when_nobody_listens`,
  `the_probe_names_the_banner_stage_for_a_server_that_is_not_ssh`,
  `the_probe_names_the_auth_stage_when_the_relay_rejects_the_user`,
  `the_probe_times_out_on_a_relay_that_sends_nothing`.
- `cargo test --test doctor` (8 passed): `doctor_ssh_relay_row_probes_the_relay_of_the_flags` (ok against a fake relay,
  fail at `connect` for a dead port), `doctor_ssh_relay_row_is_unknown_without_a_relay`, `doctor_json_keys_are_fixed`
  (the new key set).
- `cargo test -p podnet --lib doctor` (unit): `the_ssh_relay_row_is_unknown_without_a_relay`,
  `the_ssh_relay_verdict_follows_the_probe`, `the_ssh_relay_target_comes_from_the_record_of_an_instance`.
- `sh scripts/agent/check.sh`: fmt, clippy, size, doc, layers and test are 0 (record: see the commit of the record).

Not proven: a live relay; a probe through `--connect-proxy`; the instance source in a run of the binary (only the parse
of the record is tested); a relay that sends text before its identification line.


# T-166: The SSH relay leg: the relay, the user and the URL rule are data (GitHub #363)

**Source:** GitHub #363 (https://github.com/PodUtils/podnet/issues/363)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

`RelayConfig::default()` has one relay (`crates/podnet-serve/src/relay.rs:43`), the user `podnet` (`:70`), and one URL rule
(`pick_tunnel_url`, `:409-422`): the first token that starts with `http(s)://` and does not hold the
dashboard host of that one relay. Another relay that prints a bare host, a URL split over two data
frames, or its own dashboard URL fails or gives the wrong URL.

## Premise

- The fields of `RelayConfig` are `pub`; no flag sets them (no caller, see T-165). `--relay` is the
  flag of the WebSocket relay.
- `pick_tunnel_url` runs on each `ChannelMsg::Data` chunk (`crates/podnet-serve/src/relay.rs:514`), so a URL that
  crosses two chunks is lost. The pinned tests are in `crates/podnet-serve/src/relay/tests.rs`.
- The texts to skip are a fact of `pod-services` (`url-skip` of `pinggy`, `crates/podnet-serve/src/relay.rs:95-101`).
- Not measured: the other relays of the survey (local artefacts of the issue).

## Approach

1. Collect the banner over chunks, then pick the URL.
2. Exclude every known dashboard host, not only one. Prefer the tunnel domain of the relay.
3. Read the endpoint, the user and the URL-line shape of each relay from `pod-services`. A new
   relay fact is an ask to podcrates (COORD, `(to podcrates)`). Add no host in podnet.
4. An address list tried in order (as README.md:433-435 for the WebSocket relay).
5. CLI flags only with the CLI of T-165.

## Decision

The owner, 2026-11-11 (`docs/decisions.md`, The SSH forward-relay leg): the leg gets a CLI (T-165); the relay facts
(relay host, user, URL rule) are data of `pod-services` of `PodUtils/podcrates`.

## Prove

```sh
cargo test -p podnet relay::tests -- picks_the_tunnel_url url_across_two_frames
```

## Start condition

T-165.

## Correction

2026-11-11: T-165 is partial (commit 6126bf6; the row of `doctor` is left): `podnet ssh-relay` takes the relay (`--server`), the user (`--user`)
and the URL rule (`--url-match`, `--url-skip`) as required flags, and `RelayConfig` has the field `url_rule`
(`relay::UrlRule`, with `skip` and `hold` lists). This closes Approach 5 and the line of the Premise that says
that no flag sets the fields. What stays for the data of `pod-services`:
- The relay endpoint, the user and the URL rule of each known relay as facts. The flags then get defaults from
  these facts (a name of a relay, or no flag for the default relay), and a flag still overrides them. Ask
  podcrates with a COORD issue (to podcrates); add no host in podnet.
- Approach 1: the banner is still read chunk by chunk (`UrlRule::pick` runs on each `ChannelMsg::Data`), so a URL
  that crosses two chunks is lost.
- Approach 4: an address list tried in order.
- Approach 2 is part met: `--url-skip` is repeatable, so a command line can exclude more than one dashboard
  host. The known hosts are still not data.

2026-11-12: The blocker is met. `PodUtils/podcrates` v0.26.0 (the tag that podnet takes) holds
`pod_services::forward::Relay::of`, `forward::UrlRule` and `forward::Banner` (podcrates T-224, the answer
to COORD #455). The Approach is done in the commit below.

## Done

2026-11-12, commit 6aa41b3. The facts come from `pod_services::forward` through
`crates/podnet-serve/src/relay/facts.rs` (`lookup`, `names`, `dashboards`, `default_facts`) and
`podnet_core::services::forward_relays`. podnet holds no relay host.
- Approach 3: `podnet ssh-relay --service NAME` (default `pinggy`) takes the plain SSH endpoints, the user
  (else `podnet`), the URL rule and the keepalive (else 30 s) of the relay. `--server`, `--user`,
  `--url-match` and `--url-skip` override them (each URL flag replaces its own list). `--server` with no
  `--service` is a relay that no data describes: `--user` and a URL flag are then required (exit 64). An
  unknown name is exit 64 and lists the known relays. `RelayConfig::default()` uses the facts of `pinggy`.
- Approach 1: `relay::feed` gives each `ChannelMsg` to a `forward::Banner`, which holds the word that a frame
  cut, and `finish` at the end of the channel. The own `UrlRule` (`skip`, `hold`) is gone; the type is
  `forward::UrlRule` (`skips`, `matches`).
- Approach 2: the rule of a relay skips the `url-skip` texts of every known relay (`facts::dashboards`).
- Approach 4: `RelayConfig.fallbacks`; `connect_transport` tries `address`, then each fallback, in order, on a
  transport error (a bad address stops at once). `ssh-relay` fills them with the other plain SSH endpoints
  of the relay (`serveo`: 443, then 22). The doctor probe uses the same function. The next endpoint is tried
  at the transport only: a handshake or login error does not move to the next endpoint.
- Tests (`crates/podnet-serve/src/relay/tests.rs`, `src/cli/cmd/ssh_relay.rs`): `picks_the_tunnel_url_and_not_the_dashboard`,
  `url_across_two_frames` (failed first with a per-frame reader), `a_banner_with_no_end_of_line_is_whole_when_the_channel_closes`,
  `a_banner_with_only_a_dashboard_ends_with_no_url`, `the_rule_of_each_relay_skips_every_known_dashboard`,
  `the_transport_tries_the_address_list_in_order` (failed first: one address only),
  `the_facts_of_the_default_relay_come_from_pod_services`, `ssh_relay_with_no_flag_uses_the_facts_of_the_default_relay`,
  `a_flag_overrides_the_facts_of_the_relay`, `ssh_relay_takes_the_facts_of_a_named_relay_and_the_address_list`,
  `ssh_relay_refuses_an_unknown_relay_and_lists_the_known_ones`.
- Not proven: a live relay. The URL rule of `pinggy` in the data is `https://` and `.run.pinggy-free.link`;
  the banner of 2026-10-09 also held `*.free.pinggy.net`, which the rule no longer takes. A fake relay
  that sends a URL in two SSH frames is not added (the unit test covers the reader).
# T-172: A pin of the new public name while the process runs (GitHub #367)

**Source:** GitHub #367 (https://github.com/PodUtils/podnet/issues/367)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

The name `*.trycloudflare.com` is known only after the provision. `--resolve` is one string, set once.
`PINS` (`src/resolve.rs:41`) and the resolver `SETTINGS` (`:112`) are `OnceLock`s; `set_pins` has no effect
a second time. A supervisor cannot give a pin to a running start.

## Premise

- The pins are in `pod_dns::Settings` (`src/resolve.rs:125-131`, `pins: pins()`). A refresh needs a
  settings or pins handle that can change in `pod-dns`.

## Approach

1. COORD ask to podcrates: a pin table of the resolver that can change.
2. Then re-read `PODNET_RESOLVE` at each verify attempt (or a pin file).

## Prove

```sh
cargo test -p podnet resolve::tests::a_later_pin_reaches_the_resolver
```


## Start condition

podcrates gives the changeable pin table.

## Correction

2026-11-09: the blocker cleared. podcrates v0.8.0 has `Resolver::set_pins` of pod-dns (podcrates T-126); podnet takes v0.8.0 in 0df677d.

## Done

2026-11-09, commit 8b0c546. verify_fetch calls resolve::refresh_pins at each attempt; it sets the table of the resolver (`--resolve` pins and the variable PODNET_RESOLVE as it is then) with Resolver::set_pins. A pin file is not added. Test: resolve::tests::a_later_pin_reaches_the_resolver. check.sh: exit 0. CI: at the merge.

# T-176: Guest-wire router on smoltcp: the constraints of the TCP terminator and the UDP socket (GitHub #371 and #372)

**Source:** GitHub #371 (https://github.com/PodUtils/podnet/issues/371) and https://github.com/PodUtils/podnet/issues/372
**Category:** research
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

No entry plans a router that ends guest TCP on smoltcp (T-095 to T-098 are doors with per-flow dials).
If one is built, a prototype found: (1) a closed flow holds its tuple up to 10 s (`TimeWait` close delay);
a new SYN for a stale tuple gets no answer; (2) dial before the handshake (`HeldSyn`) is better than a
dial at `SynReceived`; (3) a guest UDP socket needs a bounded pending queue and a stop-reading-the-wire
signal; a partial write must keep the byte offset; the socket buffer must hold 65535 plus metadata, and
an oversized datagram needs its own counter.

## Premise

- Present in podnet: `HeldSyn` (`crates/podnet-netns/src/netns/backend.rs:13,146`; its RST is `rst_reply_for_syn` of `pod_hold::packet`, PodUtils/podcrates); `reap_closed`
  (`crates/podnet-netns/src/netns/backend.rs:326`) reaps at `Closed`; the vnet stack treats `TimeWait` as closed (`close_and_check`, `PodUtils/podcrates:crates/pod-netstack/src/stack/service.rs`)
  and defers `close` until the queues are empty (the same function). `NetStack` defaults hold the payload at
  64 KiB (`:76-77`). Async writers use `write_all` (`upstream.rs:626`, `framing.rs:43`).
- No present defect. smoltcp lines are not re-read; the prototype is not in the repo.
- Overlap: #352 (netns netfilter), T-111 (takes datagrams before smoltcp).

## Approach

Keep the list in the Problem as the acceptance rules of any such router. Do no work now.

## Decision

The owner, 2026-11-11 (`docs/decisions.md`, Guest-wire router): plan a router that ends guest TCP on smoltcp. This entry
holds its constraints; the router itself is new work.

## Prove

```sh
cargo test -p podnet-vnet --test guest_wire
cargo test -p podnet-netns -- a_partial_stream_write_keeps_the_record_stream_aligned
```

## Start condition

None: the owner ruled. The netstack of `pod-netstack` (taken at v0.20.0, now v0.27.0) carries the guest wire.

## Correction

2026-11-12: the lead asked for measures, so the Approach ("do no work now") went further: the constraints are
measured against `pod-netstack` v0.26.0 in `crates/podnet-vnet/tests/guest_wire.rs`. The result changes three
items of the Problem. (1) The stack of `pod-netstack` reaps `TimeWait`, so the 10 s tuple hold does not exist
there; the real cost is one SYN retransmit (1 s) for a flow that the guest forgot. (2) The stack ends the
handshake before the app sees the flow, so the dial before the handshake is a hold of the SYN on the wire, not
a call of the stack. (3) A datagram over `mtu - 28` is lost with no error and no counter, because the smoltcp of
podnet has no IPv4 fragmentation. The note `docs/research/podnet/guest-wire-router.md` has the table of the
constraints and five asks for `pod-netstack`.

## Done

2026-11-12, commit 0b6b0a1. Eleven tests in `crates/podnet-vnet/tests/guest_wire.rs` pass (3 runs, 5.8 s each):
`guest_wire_closed_tuple_is_free_at_once`, `guest_wire_stale_tuple_is_released_by_a_new_syn`,
`guest_wire_handshake_ends_before_the_app_accepts`, `guest_wire_held_syn_dials_before_the_handshake`,
`guest_wire_abort_after_the_handshake_sends_no_reset`, `guest_wire_accept_queue_drops_a_flow_after_the_handshake`,
`guest_wire_stack_udp_queue_is_bounded_and_silent`, `guest_wire_udp_over_the_mtu_is_lost_without_fragmentation`,
`guest_wire_partial_write_keeps_the_byte_order`, `guest_wire_a_drop_loses_the_tail_and_a_held_shutdown_does_not`,
`guest_wire_link_queue_says_full_and_has_no_wait_for_room`. The note is
`docs/research/podnet/guest-wire-router.md`. No router code changed. Not proven: the RST answer to a held SYN
(`rst_reply_for_syn`), a guest with a kernel TCP stack, load with many flows, and the IPv4 fragmentation
features (measured once by hand, not in the repository). Five asks for `pod-netstack` are open (the note,
section 3); `abort` with no RST and the drop of a stream with queued bytes (#457) block a refusal after the
handshake.

# T-178: Router: named group policies (`rr`, `minrtt`, `leastload`, `protect`) (GitHub #375)

**Source:** GitHub #375 (https://github.com/PodUtils/podnet/issues/375)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

T-096 has one pick: weighted. Operators ask for named policies. `protect` probes degraded members at a
bounded rate and never picks them while a member is up.

## Premise

- T-096 Approach 1 and 3. The plan has no policy flag. The seeded test style is in the Prove of T-096.
- The policies read the health state of T-097 and change no transition.

## Approach

1. `--group-policy NAME=POLICY`, default `weighted`. Each policy is a pure function over member snapshots.
2. Seeded unit tests for each.

## Prove

```sh
cargo test --workspace -- group_policy_picks_by_name
```

## Start condition

T-096 and T-097 are done. The measured need is not shown: the issue gives field designs only.

## Correction

2026-11-11, two comments on GitHub #375 from the donor review (satmux.md block 2 and airhand.md block 4). They
give input to the Approach. No code changes.

- The metric of `leastload` (satmux; MIT) is bytes with a short decay plus the opens that have not seen a first
  byte (comment: https://github.com/PodUtils/podnet/issues/375#issuecomment-6789131795). `load = load * 0.35^dt + n` on each transfer. The pick sorts on the tuple
  (busy bucket under 4096 / under 65536 / more, opens without a first byte, `load / 4096`, unsent bytes, a
  rotating tie-break). The thresholds and the constant are the donor's choice for a slow link: make them named
  constants. Release the open counter on each end of a flow with a guard object. The seeded tests: 35 idle
  flows against 2 busy bulk flows (a flow-count policy fails), three opens in one tick never land on one
  member, a bucket falls back with an injected clock, a flow dropped before its first byte returns its count,
  the tie-break rotates. The snapshot of a member needs counters for decayed bytes and for opens without a
  first byte: `Tracker` in `crates/podnet-core/src/network/health.rs` holds neither.
- The input of `minrtt` (airhand; MIT) is the least of a window of passing probe times, not the last one
  (comment: https://github.com/PodUtils/podnet/issues/375#issuecomment-6789133721). `Tracker` keeps the last passing probe in `rtt`
  (`crates/podnet-core/src/network/health.rs:163-164`), so a probe that shares a loaded member reads long
  exactly when the member is busy. The input of this policy is `rtt_floor()` (see the Correction of T-097).
  A move of the head needs a margin of `max(5 ms, 10%)` for a dwell of consecutive probe rounds (the donor's
  values; re-derive them for TCP-connect probes). Existing flows never move.
- The two comments do not conflict. The measured need (Start condition) is still not shown by the donor.

2026-11-11, 184fa71, done by the call of the lead session (the measured need of the Start condition is still not shown; the code is ready for a measure). The router file takes
`policies: NAME: POLICY` (`weighted` default, `rr`, `minrtt`, `leastload`, `protect`); a file with no key keeps
the old form and the weighted pick. There is no flag `--group-policy`: the router has no group flag (T-100).
The policies are pure functions in `crates/podnet-core/src/network/policy.rs`; `Balance::pick_at` takes the
time. `Tracker` keeps a window of passing probe times (`health.window`, default 8, 0 to 64; 0 is off) and gives
`rtt_floor()`; it changes no health step. `minrtt` keeps a head that moves for a margin of `max(5 ms, 10%)`
over 3 probe rounds in a row (`Balance::end_round`, called by the probe loop). `leastload` sorts on (bucket of
the decayed bytes, open dials, `load / 4096`, a rotating tie-break); an open dial is a guard that drops at the
end of the dial. `protect` picks by weight among `Up` members, and among the degraded members only when none
is `Up`; the probe of each interval is the only rate of probe. The status document has `policy`, `head`,
`rtt_floor_ms` and `opens`; `podnet route --status` prints them. Tests: `group_policy_picks_by_name`,
`group_policy_leastload_reads_bytes_not_flow_counts`, `group_policy_leastload_spreads_opens_and_returns_them`,
`group_policy_leastload_bucket_falls_back_with_the_clock`,
`group_policy_minrtt_head_needs_a_margin_and_a_dwell`, `router_health_rtt_window_changes_no_step`,
`route_config_policy_reaches_the_group_and_the_status`. Left: nothing feeds `Member::add_bytes` (the meter of
the doors; the donor's unsent-bytes key is not there), so `leastload` reads open dials only until it does; the
first-byte release of an open (the guard drops at the end of the dial); the margin and dwell are the donor's
values and are not measured for TCP-connect probes; no field measure of any policy.

## Done

2026-11-11, 3b83bcd. The byte meter of the group members is there (`crates/podnet-core/src/network/meter.rs`).
`Balance::dial` wraps the stream of each flow in `Metered`. The wrapper counts the bytes in both
directions as the copy runs (two atomic adds for each chunk: the pending load and the total), and gives
them to the member that carried the flow. Each door, and each rules file, dials through the group, so
each flow of a door is counted; `Connected` (pod-core) is not changed. The pick folds the pending bytes
into the decayed load (`Member::add_bytes` keeps its form), so `leastload` reads the bytes. The open dial
(`OpenGuard`, now owned) ends at the first byte in either direction, on a dropped flow and on a failed
dial. The status document has the key `bytes` for each member; `podnet route --status` prints it. The keys
are only added.
Tests (each failed before the change, with the wrap disabled): `group_meter_counts_bytes_in_both_directions_as_the_flow_runs`,
`group_meter_open_ends_at_the_first_byte`, `group_meter_feeds_leastload_with_the_bytes_of_the_flows`,
`front_door_gives_the_bytes_of_a_flow_to_its_member`; the pinned key tests `router_status_json_keys` and
`router_status_json_keys_over_the_control_socket` have the key `bytes`. `sh scripts/agent/check.sh` is 0.
Not proven: the margin and the dwell of `minrtt` for TCP-connect probes; a field measure of any policy
(the owner ruled that none is needed). UDP datagrams are not metered.

# T-179: Research: a direction split for asymmetric uplinks (GitHub #375)

**Source:** GitHub #375 (https://github.com/PodUtils/podnet/issues/375)
**Category:** research
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** partial

## Problem

A fast-download line beside a slow-upload line fits a split: the request leg on one uplink, the
response leg on another, with the exit of each flow kept at the terminating side. It needs a relay node
or a chain member that ends a leg.

## Premise

- It contradicts T-096: "One flow uses one uplink". Adopting it changes that contract (owner; see Decision).
- `crates/podnet-core/src/race.rs:54` is the per-leg primitive; `src/warp/forward.rs` ends one leg and re-dials the other.

## Approach

Write the design in `docs/research/podnet/`: the flow id, the direction bit, the exit table, the cleanup.
Do no code.

## Decision

The owner, 2026-11-11 (`docs/decisions.md`, Direction split): adopt the split of the directions of a flow as an
option of a group. It is configurable and off by default; one flow on one uplink stays the default contract
of T-096.

## Prove

```sh
cargo test -p podnet-core group_split
cargo test -p podnet route_config_split
```

## Start condition

None: the owner ruled. The option needs the router of T-100 and the groups of T-096 (done).

## Correction

2026-11-11, the owner ruled (`docs/decisions.md`, Direction split): the split is an option of a group, off by
default. The Approach "do no code" is replaced: the part that podnet can do now is built (commit 56b3027); the
rest is left (below).

Design.

- Contract. With no `splits` entry, one flow uses one member (T-096). The option changes nothing then. An old
  router file stays valid.
- TCP. One byte stream has both directions. A split needs two connections (the request leg on one member, the
  response leg on another) that a far end joins into one stream. podnet has no such far end: the relay
  (`src/relay*.rs`) and the proxies end one connection. The option does not split TCP. The value `tcp` is
  refused with an error (`split::TCP_REFUSAL`). A TCP flow keeps one member, as the pick gives it.
- UDP. Datagrams have no stream. A socket of a split group sends by the member `up` and reads the member `down`
  first, then `up`, so that an answer on either path is delivered (`SplitDatagrams`). It works only when the
  far host is one host behind both members (two tunnels to one endpoint) and answers on the path of `down`.
  Plain UDP answers go to the source address of the datagram, that is, to the path of `up`. podnet cannot make
  a far host answer on `down`. The operator sets the key to say that the far host does.
- Key. `splits:` of the router file: `NAME: up=MEMBER down=MEMBER`, or `off` (default). `up` and `down` are two
  different members of the group `NAME`. The operator names them; podnet does not measure the uplink of a member.
- Fallback. A member that does not take flows (health `down`) cannot split. The socket then takes the member of
  the pick of the policy, as with no split. A `down` member with no datagrams leaves the socket on `up`. The
  kill switch stays: with no member up, the socket is refused. Each fallback is counted.
- Status. The new key `split` of each group of the status document: `mode` (`off` or `udp`), `up`, `down`,
  `sockets`, `fallbacks`. `podnet route --status` prints it on the line of the group.
- Errors. A bad value, a member that is not in the group, one member as `up` and `down`, a name that is not a
  group: each is an error that names `splits.NAME` when the file is read.

Left, with the reason.

- A split of TCP: it needs a far end that joins two connections (a flow id, a direction bit, an exit table,
  the cleanup). The relay of `PodUtils/podcrates` (`pod-relay`) is frozen (`CLAUDE.md`, Shared crates), and no
  other far end exists. Write the design in `docs/research/podnet/` when a far end exists.
- A far host that answers UDP on the path of `down`: it needs a cooperating far end or an endpoint that holds
  both sessions of one peer. podnet has none. Tunnels to one endpoint (WARP, a relay) do not steer answers.
- The byte meter (T-178) did not count datagrams. 2026-11-12: T-097 (commit f4dd392) counts the UDP bytes for the
  member in both directions, also with a split, so `leastload` sees them.
- A measure of the uplink and the downlink of a member (to pick `up` and `down` without the operator).

2026-11-12, GitHub #473 (row E7-1). The left item "a split of TCP" needs a far end that joins two connections. The link session of T-262 is such a far end: T-302 splits one session across an up carrier and a down carrier. The router split of this entry stays unchanged, and its tests must pass unchanged.
# T-244: `tunnel --inspect`: capture each exchange beside the copy helpers of `forward_http`, keep a ring, redact by default, replay over a 0600 socket (GitHub #435)

**Source:** GitHub #435 (https://github.com/PodUtils/podnet/issues/435); donor review, tunnet.md block 4
(tunnetio/Tunnet at fceb3f2; the inspector is in the MPL-2.0 crate `tunnet-core`)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

A user who publishes a local webhook receiver with `podnet tunnel` cannot see what arrived. Add
`--inspect`: in `forward_http`, which already holds the request, reads the response head and copies the
bodies, give each copy helper a capture sink that copies at most a cap of body bytes and never delays a
write. Keep the last N exchanges in memory and offer `podnet tunnel inspect` (list, show, replay) over a
unix socket with the mode 0600. A streaming response must reach the visitor as it does today. Acceptance:
after a failing delivery, `podnet tunnel inspect show ID` prints the request, and `inspect replay ID` sends
it to the origin and records a new exchange that names the first.

## Premise

- Implement from the description, not from the donor source (MPL-2.0). Describe the behaviour only.
- The code is in `src/proxy.rs`: `forward_http` (`src/proxy.rs:284`), `proxy_bidi` (`src/proxy.rs:354`),
  `forward_tcp` (`src/proxy.rs:389`), `build_request_head` (`src/proxy.rs:842`), `read_response_head`
  (`src/proxy.rs:1013`), `copy_futures_to_tokio_n` (`src/proxy.rs:1065`), `copy_tokio_to_futures_n`
  (`src/proxy.rs:1108`), `copy_tokio_to_futures_eof` (`src/proxy.rs:1135`), `copy_chunked_to_futures`
  (`src/proxy.rs:1160`). Each helper takes `metrics: &Metrics`, so a sink beside it sees each body byte.
- One stream carries one request, and the framing code already decodes the response: no request parser is
  needed. `forward_tcp` answers an HTTP request to a raw TCP origin with 501: do not capture there.
- The dial is `origin::dial::dial` (`src/proxy.rs:308`, `src/proxy.rs:410`, `src/ws.rs:112`). `Origin::connect`
  (`crates/podnet-serve/src/origin/mod.rs`) has no caller outside its own module (a grep of `src`, `crates`
  and `tests`), so a decorator on it would see no traffic.
- No capture and no request log exist in the tunnel path (a grep for `inspect`).
- The socket: `claim_socket_path` (`crates/podnet-core/src/net.rs:21`), the check of the peer uid with
  `SO_PEERCRED` (`crates/podnet-vnet/src/switch.rs:264-270`), and `instances::lookup_dir`
  (`crates/podnet-core/src/instances.rs:81`).
- `src/proxy.rs` and `src/tunnel.rs` go to `pod-cloudflare` (T-052). Podnet does not develop a new feature
  in that code (`CLAUDE.md`, Order). Pod-cloudflare is not in `PodUtils/podcrates` yet. No issue and no
  entry covers `--inspect`.
- Flaws of the donor to avoid (Source in the issue): it reads the whole message before it forwards, so a
  stream reaches the client only at the end; the chunk size is parsed with `from_str_radix` and `size + 2`
  is used; its inspector listens on `127.0.0.1:4040` with no credential and offers a replay; it stores all
  headers, `Authorization` and `Cookie` included.
- Not run: the cost of the capture on a real tunnel. Measure the added latency and CPU for 1000 requests
  per second against a loopback origin before any default other than off.

## Approach

1. Flag `podnet tunnel ... --inspect` (off by default), with `--inspect-keep N` (default 100) and
   `--inspect-body BYTES` (default 1048576). Reject 0 for both.
2. A capture sink with `head(...)` and `body(chunk)` for each direction. The helpers call it after the write
   to the peer, so a slow or full sink holds no byte back. A sink that fails is dropped for that exchange,
   and the copy goes on. A chunked response gives the decoded payload that `copy_chunked_to_futures` writes,
   never the size lines.
3. A ring of N records in memory (id, start time, method, target, heads, bodies up to the cap with the
   truncation flags, status, milliseconds, `replayed_from`, `incomplete`). The ring is lost at exit and is
   never written to a file.
4. Redaction by default: replace the value of `authorization`, `proxy-authorization`, `cookie`,
   `set-cookie` and each header whose name contains `token`, `secret` or `key` with `[redacted]` in the
   stored copy; the forward keeps the real value. `--inspect-raw` keeps the values and prints one warning
   line at the start. A request refused by the credential of the tunnel (`src/proxy.rs:134-156`) is not
   captured.
5. Access: `inspect.sock` in a private directory of the run directory, mode 0600, with the check of the peer
   uid. Commands: `podnet tunnel inspect ls [--json]`, `show ID`, `replay ID`, `clear`. `replay` sends the
   stored request to the same service with `Connection: close`, records the result with `replayed_from`, and
   does not count as a visitor request in the metrics.
6. Replay safety: only for `Service::Http`, `Unix` and `UnixTls`. Refuse a record marked `incomplete`, a
   record with a truncated request body, and a WebSocket record. `proxy_bidi` records the head and the 101
   status, then no body.
7. At the start of the work, send a COORD issue (to podcrates) so that the take of `pod-cloudflare` carries
   the change.

## Prove

```sh
cargo test -p podnet --test tunnel_inspect -- a_streaming_response_keeps_its_timing_with_inspect the_ring_keeps_the_cap_and_the_flags
cargo test -p podnet --test tunnel_inspect -- a_bad_chunk_size_ends_the_exchange_as_incomplete authorization_is_redacted_and_forwarded
cargo test -p podnet --test tunnel_inspect -- replay_records_a_new_exchange_that_names_the_first replay_of_a_truncated_request_is_refused
```

A response that streams 10 chunks with 200 ms between them reaches the client with the same timing with and
without `--inspect`; a sink that collects the whole body before the helper writes it (the order of the
donor) must fail this test. A 50 MiB upload and a 50 MiB download keep the resident set within 8 MiB above
the baseline, and the stored body is the cap with the flag set. An origin that answers with the chunk size
line `ffffffffffffffff` and closes: the exchange ends with an error as today, with no panic, and the ring
holds the partial record marked `incomplete`. Another uid cannot connect to `inspect.sock`, and its mode is
0600.

## Blocker

The session of `PodUtils/podcrates` (the crate `pod-cloudflare`, T-052): `src/proxy.rs` moves there, and
podnet does not develop new features in that code. The entry is unblocked when podnet takes `pod-cloudflare`
and the crate holds `forward_http`.

## Start condition

Podnet holds `pod-cloudflare`, and the crate holds `forward_http`.

# T-248: Router: a guest-wire router that ends guest TCP on smoltcp

**Source:** the owner, 2026-11-11 (`docs/decisions.md`, Guest-wire router); T-176; GitHub #371 and #372.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The router of T-100 has doors only: each flow is a dial of its own. A guest (a VM, a container, a host behind
one wire) cannot use the router as its gateway: no part ends the TCP and the UDP of a wire and sends each flow
to a network or a group of the router.

## Premise

- T-176 holds the constraints of such a terminator (the `TimeWait` delay, a dial at `HeldSyn`, the bounded UDP
  queue, the byte offset of a partial write, the buffer of 65535 bytes and the counter of oversized datagrams).
- `pod-netstack` (taken at v0.20.0) is the stack; T-249 moves the netns router onto it.

## Approach

1. Add a wire to the router file: a TUN, a veth or a unix socket of frames, with the address of the guest net.
2. End each guest TCP flow on the stack with the constraints of T-176, and dial it through the rules and the
   groups of the router (`Router::decide`, T-177).
3. Carry the guest UDP over `Dial::datagrams`, with the bounded queue of T-176.

## Prove

```sh
cargo test --workspace -- guest_wire_routes_tcp_by_the_rules guest_wire_udp_has_a_bounded_queue
```

## Start condition

T-249 is done.

## Correction

2026-11-12, from T-176 (`docs/research/podnet/guest-wire-router.md`, `pod-netstack` v0.26.0, tests in
`crates/podnet-vnet/tests/guest_wire.rs`). The Premise and the Approach change:

- The stack is at v0.26.0. It reaps `TimeWait`, so the router adds no timer. A flow that the guest forgot costs
  one SYN retransmit (1 s). Do not drop a SYN for a tuple that the router holds.
- Approach 2 dials at the wire, not in the stack: the stack ends the handshake before `accept`. Hold the SYN in
  the pump (key it by the tuple, a retransmit is a copy), dial, then release it into `link_in`. A failed dial
  answers the SYN with `rst_reply_for_syn` (not proven).
- The router cannot refuse a flow after the handshake: `TcpStream::abort` sends no RST, and the accept queue (32
  flows) drops a flow with no RST. These are asks for `pod-netstack` (the note, section 3, asks 1 and 2), beside
  the three gaps of PodUtils/podnet#457. Until they close, T-248 holds the hold of the SYN and uses `shutdown`.
- 2026-11-12, v0.27.0 (T-137): `pod-netstack` v0.27.0 closed asks 1 and 2 and the three gaps of
  PodUtils/podnet#457. `abort` sends its RST, so the router refuses a flow after the handshake with `abort`. The
  accept queue is gone (`accept_pool` is gone): no flow is lost after the handshake, and a SYN past
  `max_connections` is dropped with no RST. A drop of a stream sends each queued byte. A stale tuple ends the old
  flow with `ConnectionReset`. Asks 3 to 5 (the UDP counters, the error for an oversized datagram, `peek_slice`)
  stay open in PodUtils/podnet#464. The hold of the SYN stays: the stack still ends the handshake before `accept`.
- Approach 3: the UDP path loses a datagram over `mtu - 28` and a burst past 113 datagrams with no error and no
  counter (asks 3 to 5). The router needs its own bounded queue and its own counters. T-248 decides the smoltcp
  features for IPv4 fragmentation (measured: they carry a datagram of 65507 bytes).
- The Prove name `guest_wire_udp_has_a_bounded_queue` is for the router. The test of the stack in T-176 is
  `guest_wire_stack_udp_queue_is_bounded_and_silent`.

2026-11-12, T-248 work, after v0.28.0. Decision on fragmentation: on, with the feature `ipv4-fragmentation` of
`pod-netstack` (not the raw features of smoltcp), in the root crate and in `podnet-vnet`. A datagram of 65507
bytes crosses a guest wire in both directions at MTU 1500 and 1280 (the feature alone is enough for the
reassembly). Cost: a buffer of 64 KiB for each interface of the binary, WARP too. Workspace builds unify the
feature, and a build of `podnet-vnet` alone has it too. The test that main held for the refusal
(`guest_wire_udp_over_the_mtu_is_refused_without_fragmentation`) is now
`guest_wire_udp_over_the_mtu_is_carried_with_fragmentation`. The asks 3 to 5 are closed in v0.28.0: the router
uses `max_datagram` and the counters of the stack (`stack_udp_*` in its status). It keeps a queue for each flow,
because one socket of the stack serves every flow of a port.

2026-11-14, the veth wire and a real TUN. Calls made here:
- The veth wire is `veth:NAME`: an end of a veth pair in the namespace of the router, with no address. The
  router reads it with an `AF_PACKET` socket and uses the frames of `pod-hold` (`take_frame`, `frame_for`): it
  answers ARP for the gateway with `GATEWAY_MAC`, and it learns the MAC address of each guest address from its
  frames (at most 4096; at the limit the table starts again). A packet to a guest address with no MAC address
  is dropped: the guest opens each flow, so its address is known first. The router makes no veth pair: the
  operator makes it and moves one end into the guest.
- The wire takes frames up to 64 KiB. Measured: a guest kernel with the offloads of veth on sends TCP frames
  larger than the MTU; with a limit of 1500 the 1 MiB flow of the test took 39 s (losses and retransmits)
  against 2 s. The router does not change the offloads of the guest end.
- Left, not in the Approach: IPv6, ICMP and names (the Done of 2026-11-12), and a test of the idle end of a UDP
  flow (60 s). The entry closes with them as known limits in `docs/router.md`.

The Prove of the work of 2026-11-14:

```sh
cargo test -p podnet --lib -- router::guest
```

## Done

2026-11-12, partial; commits 9249037 (the code), 266fe0e (the record) and 6775594 (the take of v0.28.0). The router file has a `guests` list (`src/router/guest.rs`, `src/router/guest/`).

* Approach 1: a wire is `unix:/PATH` (SOCK_SEQPACKET, mode 0600, a new guest replaces the old) or `tun:NAME` (an
  existing TUN, opened with TUNSETIFF). `net` gives the guest net; the router is its first host.
* Approach 2: the pump holds each SYN, dials through the rules and groups of the door (`rules`, `via`), and
  releases the SYN on success. A failed dial answers with `rst_reply_for_syn` (proven now). A flow with no
  released SYN dials after the handshake and aborts on failure. `Router::decide_guest` is the dry run.
* Approach 3: UDP goes over `Dial::datagrams` of the way that the rules choose, one socket for each flow. The
  router reads each stack socket at once. Each flow has a queue of 32 datagrams and counters (`udp_queue_dropped`,
  `udp_oversize`, `udp_refused`), in the status document and the metrics (`podnet_route_guest_*`).
* Tests (`src/router/guest/tests.rs`, `udp_tests.rs`): `guest_wire_routes_tcp_by_the_rules`,
  `guest_wire_udp_has_a_bounded_queue` (the sum of the datagrams in the way, in the counter of the router and in the counter of the stack is exact), `guest_wire_dials_before_the_handshake`,
  `guest_wire_a_failed_dial_refuses_the_connect`, `guest_wire_a_flow_carries_bytes_and_ends_both_ways`,
  `guest_wire_udp_routes_by_the_rules`, `guest_wire_udp_carries_a_datagram_over_the_mtu`,
  `guest_wire_unix_socket_serves_a_guest_and_the_next`, `guest_wire_screen_drops_what_the_router_does_not_route`,
  `guest_wire_config_names_the_key_that_is_wrong`, `guest_wire_tun_opens_or_the_host_refuses_it` (skips with no TUN).
  Failure shown without the change: no hold (5 TCP tests fail), unbounded queue (`bounded_queue` fails), no
  fragmentation features (`carries_a_datagram_over_the_mtu` fails at 1473).
* Not done or not proven: the data path of a real TUN is not
  proven (the socket pair uses the same `PacketFd`). A veth wire on an existing interface is not done. IPv6, ICMP
  and names (the guest resolves names itself; `domain:` rules do not match) are not routed. The idle end of a UDP
  flow (60 s) has no test. A Linux guest kernel is not tested.

2026-11-14, commit WORK (the veth wire, the proof of a real TUN and of a Linux guest kernel).
* `src/router/guest/veth.rs` is new (`veth:NAME`, the frames, the ARP answer, the MAC addresses);
  `PacketFd::with_frames` reads and writes frames; `Opened::Frames` is the open veth wire.
* `guest_wire_veth_carries_tcp_and_udp_of_a_linux_guest`: a guest namespace on a veth pair (offloads on),
  default route to the router. A TCP flow through way `a` carries 1 MiB each way and ends both ways; UDP
  datagrams of 4 and 3000 bytes (fragmented by the guest kernel) come back; the counters give 2 up and 2 down.
* `guest_wire_tun_carries_tcp_and_udp_of_a_linux_guest`: the router opens a real TUN (`open_tun`); the kernel of
  the namespace is the guest, with its address on the TUN and its default route to the router. The same flows
  pass. This proves the data path of a real TUN.
* `cargo test -p podnet --lib -- router::guest`: 14 passed, run here as root in `unshare -rn`. Where the host
  refuses a user and network namespace (GitHub runners) the two tests print `skip:`; with no TUN device the TUN
  test prints `skip: the namespace has no TUN device`.
* The test kit of the namespaces is `src/router/kernel/netkit.rs` (T-120).



# T-320: net: a Dev Tunnels network for TCP streams (GitHub #503)

**Source:** GitHub #503 (https://github.com/PodUtils/podnet/issues/503)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

podnet dials TCP through `direct`, a proxy, the WebSocket relay, Tailscale, NetBird and userspace WireGuard. It cannot
open a TCP channel through a Microsoft Dev Tunnels relay. A host that has outbound TCP 443 only, and whose only
reachable relay is a Dev Tunnels endpoint, has no path to a port on that tunnel host.

The issue asks:
- A network spec `devtunnel://TUNNEL` in `src/networks.rs`, the code in a new a new module devtunnel, the
  dependency `pod-devtunnel` in the root `Cargo.toml`. The port of the target is the port on the tunnel host; the host
  of the target is the local address of the tunnel host (normally `127.0.0.1`). The arm `devtunnel` goes in `Kind` and
  `kind()` before the `://` fallthrough.
- Use the `pod-devtunnel` client. podnet implements neither the management API, nor the relay WebSocket, nor the SSH
  session. Through the crate the network reads the connect information, opens the relay WebSocket, runs the SSH `none`
  session, opens a `direct-tcpip` channel and hands the stream to `forward`.
- The management token comes from the settings and secret mechanism. The connect token comes from the connect
  information for each attempt. Neither appears in `argv`, a log line or `--json`.
- A relay reconnect is the behavior of the crate; the network surfaces the named error. A reconnect ends every open
  stream. A new stream after it succeeds.
- The network registers a `Row` and returns a `Link`. No second ranking engine. `ROWS` grows from six to seven; the
  scheme list in `src/networks/tests.rs` follows.
- The management host and the SSH facts come from `pod-services`; the relay URI comes from the connect information. No
  hardcoded host.
- The form goes in the `--network` help (`src/cli/mod.rs`) and the manual (`tests/cli/manual.rs`). TCP only; no host
  side.
- Acceptance: a stand-in relay test (WebSocket subprotocol, SSH `none`, `direct-tcpip`, echo); `podnet forward -L
  127.0.0.1:8080=127.0.0.1:PORT --network devtunnel://TUNNEL` reaches the origin through the stand-in; a reconnect ends
  an in-flight stream with a named error and a new stream succeeds; `podnet doctor` has a Dev Tunnels row whose
  failure names the stage; `--json` has no token; `pod-devtunnel` is the only Dev Tunnels code in the tree.
- Use case: a build runner permits TCP 443 only; `podnet forward -L 127.0.0.1:5432=127.0.0.1:5432 --network
  devtunnel://lab` carries the stream to port 5432 of the tunnel host with the token of the developer.

## Premise

- `src/networks.rs` has the registry (`ROWS: [Row; 6]`), `src/networks/tests.rs` the scheme list,
  `crates/podnet-core/src/network/` the balance, policy and health code, `src/cli/cmd/forward.rs` the consumer and
  `src/doctor.rs` the row shape.
- `PodUtils/podcrates` v0.32.0 (the checkout of Cargo.lock) has no crate `pod-devtunnel`, and `pod-services` has no
  Dev Tunnels data file (a search of the tree for "devtunnel" finds nothing). The issue names the crate issue
  "pod-devtunnel: add a Dev Tunnels management and relay client" in podcrates as its blocker.
- CLAUDE.md: a client of the service of another party is a crate of podcrates, and podnet takes it. This entry writes
  no Dev Tunnels code in podnet.
- Not measured: the Dev Tunnels service.

## Approach

1. Wait for `pod-devtunnel` (with its `pod-services` data) and its hand-off in podcrates (T-137).
2. Then: a new module devtunnel, the arm in `Kind`, the row, the doctor row, the help, the docs
   (`docs/networks.md`) and the tests of the acceptance.

## Prove

```sh
cargo test -p podnet --lib networks::tests
cargo test -p podnet --test devtunnel_network
```

## Start condition

The take of `pod-devtunnel` is done (T-137).

## Correction

2026-11-12: podcrates writes `pod-devtunnel` (PodUtils/podcrates#101): the client side first (the management read, the relay WebSocket over podnet's TLS, proxy and dialer, SSH with `none`, `direct-tcpip` streams, a bounded reconnect); the hand-off names the API for `devtunnel://TUNNEL`.

2026-11-12: podcrates v0.33.0 has `pod-devtunnel` (PodUtils/podcrates#103); the Blocker is gone.

2026-11-12: partial. `devtunnel://TUNNEL` is in `src/networks/devtunnel.rs`, over `pod-devtunnel` v0.34.0. Left: (1) the
`podnet doctor` row of Dev Tunnels whose failure names the step: only the check of `podnet net doctor` (the management host
with TLS) exists, and the step is in the error of `up`; the fixed row list of `src/doctor/targets.rs` needs a rule for a row
that depends on a spec. (2) Not proven: a live Dev Tunnels account.

2026-11-12, 7d849e1: item (1) is done in another form. `podnet net doctor --network SPEC` brings up any network of the
registry and adds the row `network SPEC`: `ok` with the status of the link, or `fail` with the error of `up`, which names
the step. The fixed rows of `podnet doctor` stay. The entry is done; a live Dev Tunnels account is not proven.

## Done

2026-11-12, commit 5f1981a (`T-320: devtunnel://TUNNEL, a Dev Tunnels network for TCP streams over pod-devtunnel`), partial
(see Correction). The spec parser, the registry row (`ROWS` is nine rows), `Kind::Devtunnel`, the status with no token, the
check of the management host, the `--egress` refusal, the `forward` help, the manual (four variables), `docs/networks.md`,
`docs/egress.md`, `docs/forward.md` and the README map. The token comes from `PODNET_DEVTUNNEL_TOKEN_FILE` or
`PODNET_DEVTUNNEL_TOKEN`, never from argv. The stand-in of `pod-devtunnel` (dev-dependency, feature `standin`) is the relay.
Tests (`cargo test -p podnet --lib networks::devtunnel`, `--test devtunnel_network`, `--test cli manual`):
`devtunnel_stream_echoes_and_status_holds_no_token` (management read, WebSocket subprotocol, SSH `none`, `direct-tcpip`,
echo), `devtunnel_reconnect_ends_a_stream_with_a_named_error_and_a_new_stream_works` (`ConnectionAborted`, then a new
stream on session 2 with a fresh token), `devtunnel_failures_name_their_step` (refused token, unknown tunnel, refused
channel), `devtunnel_target_host_is_the_local_address_of_the_tunnel_host`, the spec and account tests, and the command:
`a_tcp_forward_reaches_the_tunnel_host_through_the_stand_in` (`podnet forward -L 127.0.0.1:PORT=127.0.0.1:22 --network
devtunnel://standin-box`: two echoes, two channels, no token in the output), `a_devtunnel_network_with_no_token_is_a_usage_error_that_names_the_variables`,
`a_devtunnel_spec_with_a_token_in_it_is_refused_and_not_echoed`, `a_wrong_token_ends_the_command_at_the_management_step`,
`the_manual_names_the_devtunnel_network_and_its_variables`. Fail first (plant): with the loopback rule of the target host off,
`devtunnel_target_host_is_the_local_address_of_the_tunnel_host` failed. Not proven: a live Dev Tunnels account.

2026-11-12, commit 7d849e1 (`T-320, T-321: net doctor --network SPEC: the row of a network names the stage that failed`):
the row of the network. Test: `net_doctor_has_a_row_for_a_devtunnel_network_that_names_the_stage` (`--test
devtunnel_network`; the stand-in: the right token gives `ok`, a wrong token gives `fail` with "Dev Tunnels management
API"; no token in the document). Fail first: before 7d849e1, `net doctor` refused `--network` (a usage error).


# T-321: net: a TURN network for UDP flows (GitHub #507)

**Source:** GitHub #507 (https://github.com/PodUtils/podnet/issues/507)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** partial

## Problem

podnet carries UDP through `direct`, `wg:FILE` and the WARP commands. It cannot allocate a relayed transport address
on a TURN server and use it as the path of a UDP flow. A host whose only reachable relay is a TURN server on TCP 443
has no UDP path. `podnet forward -L udp:PORT=HOST:PORT` takes a `--network` spec; the registry has no TURN kind.

The issue asks:
- A network spec `turn://HOST:PORT` in `src/networks.rs`, the code in a new a new module turn, the dependency
  `pod-turn` in the root `Cargo.toml`. Transport, realm, user and credential source are settings, not query
  parameters. The arm `turn` goes in `Kind` and `kind()` before the `://` fallthrough.
- Use `pod-turn` for allocate, refresh, permission and channel bind. podnet implements no TURN.
- Credentials come from the settings and secret mechanism; none appears in `argv`, a log line or `--json`.
- A UDP flow uses the channel for each datagram.
- A failed stage gives a named error: allocate, refresh, permission, bind or send. The `doctor` row and `status` show
  the stage.
- The network registers a `Row` and returns a `Link`. No second ranking engine. `ROWS` grows from six to seven; the
  scheme list in `src/networks/tests.rs` follows.
- The form goes in the `--network` help (`src/cli/mod.rs`) and the manual (`tests/cli/manual.rs`).
- The host and port of a named service come from `pod-services`. UDP only; no TCP.
- Not in this issue: peer-to-peer NAT traversal (GitHub #469, E3), the deferred TURN allocation and RFC 6062 (#485,
  gate `link_p2p` in two network namespaces) and the RFC 6062 TCP relay of podssh (its entry 621). The STUN integrity
  primitives landed through `PodUtils/podssh#214`.
- Acceptance: a mock TURN server test (allocate, refresh, permission, bind, send, receive, and a 438 stale-nonce
  answer); `podnet forward -L udp:5353=resolver:53 --network turn://HOST:PORT` sends a query and gets the answer
  through the allocation; a refresh failure closes the flow with a named error and does not retry forever; `podnet
  doctor` has a TURN row whose failure names the stage; `--json` has no credential; `pod-turn` is the only TURN code
  in the tree.
- Use case: a build host reaches TCP 443 only and needs UDP DNS for a package manager. A TURN server on 443 is the
  only relay.

## Premise

- Files as in T-320: `src/networks.rs`, `src/networks/tests.rs`, `crates/podnet-core/src/network/`,
  `src/cli/cmd/forward.rs`, `src/doctor.rs`.
- `PodUtils/podcrates` v0.32.0 has no crate `pod-turn`. Its `pod-proxy` holds `stun_auth` (the STUN message
  integrity and the TURN methods and attributes, RFC 8656): this is the primitive, not a client. The issue names the
  crate issue "pod-turn: add a TURN client on the existing STUN codec" as its blocker.
- CLAUDE.md: the client is a crate of podcrates. This entry writes no TURN code in podnet.
- Not measured: a TURN server.

## Approach

1. Wait for `pod-turn` and its hand-off in podcrates (T-137).
2. Then: a new module turn, the arm in `Kind`, the row, the doctor row, the help, the docs
   (`docs/networks.md`) and the tests of the acceptance.

## Prove

```sh
cargo test -p podnet --lib networks::
cargo test -p podnet --test turn_network
```

## Start condition

The take of `pod-turn` is done (T-137).

## Correction

2026-11-12: podcrates writes `pod-turn` (PodUtils/podcrates#101): the STUN codec moves from `pod-proxy`, SHA-256 credentials, a TURN client of RFC 8656 over UDP and over TLS on TCP. A TCP relay (RFC 6062) is a later entry of podcrates.

2026-11-12: podcrates v0.33.0 has `pod-turn` (PodUtils/podcrates#103): `Settings::udp` or `Settings::tls` over podnet's dialer, resolver and TLS context; `Client::{allocate, refresh, create_permission, bind_channel, send, send_to, recv, close}`; `recv` refreshes on its own timer. RFC 6062 is a later entry of podcrates. The Blocker is gone.

2026-11-12, commits 3e4c999 and 3e4c999: the UDP half is done. The dependency `pod-turn` (tag v0.33.0) is in the root `Cargo.toml`. The
specs are `turn://HOST[:PORT]` (UDP, default port 3478) and `turns://HOST[:PORT]` (TLS on TCP, default port 5349). The
code is `src/networks/turn.rs` (spec, account, dialer, `up`) and `src/networks/turn_socket.rs` (the datagrams of one
allocation); podnet has no TURN code. The account is not in the spec (the entry says: settings, not query parameters): the
variables `PODNET_TURN_USER`, `PODNET_TURN_PASSWORD_FILE` (one line) or `PODNET_TURN_PASSWORD`, and `PODNET_TURN_REALM`. A
spec with a user, a password, a path or a query is a usage error that does not echo the secret. `ROWS` grew to eight (the
entry said seven: `peer` was added before). Each `datagrams()` makes one allocation; the first datagram to a peer binds a
channel; `recv` of the client refreshes on its timer. A failed stage ends the socket with an error that starts with the
network and the stage (`allocate`, `refresh`, `permission`, `bind`, `send`, `receive`), with no retry; `up` makes and
ends one allocation as a probe, so the stage of a wrong server or account shows at the start. The status of the link has
no user and no password. A TCP target is refused by name (RFC 6062). `--egress turn://...` and `pod -x` give one line.
`forward` exits 64 for a usage fault of a network. The stand-in server is `src/networks/turn_mock.rs` (the codec of
`pod-turn`, a UDP socket, no account). Left: (1) the TCP relay of RFC 6062 (a later entry of podcrates; the TCP half of
the entry stays partial); (2) a row of `podnet doctor` for TURN: `doctor` takes no network spec, and `doctor.rs` is at the
size limit; the stage shows at `up` and in the status; (3) one account for each process (a TURN network of the router with
its own account needs a form for it); (4) the host and the port of a named TURN service from `pod-services` (the crate
has no such fact). Not proven: a live TURN server.

2026-11-12, 7d849e1: item (2) is done: `podnet net doctor --network turn://HOST:PORT` adds the row of the network, and
a failure names the stage (`allocate` and the code). Test: `net_doctor_has_a_row_for_a_turn_network_that_names_the_stage`
(`--test turn_network`; the stand-in: the right password gives `ok`, a wrong one gives `fail` with `allocate`; no password
in the document). Items (1), (3) and (4) stay.

## Done

2026-11-12, commits 3e4c999 (`T-321: net: turn:// and turns://, a TURN network for UDP flows`) and 3e4c999, partial (see Correction). Tests (`cargo test -p podnet --lib networks::`,
`--test turn_network`): `turn_up_allocates_once_ends_it_and_shows_no_credential`,
`turn_datagrams_carry_a_datagram_to_a_peer_and_back` (allocate, bind, send, receive, one channel for two datagrams, the end
of the allocation), `turn_a_stale_nonce_does_not_lose_the_datagram` (a 438 answered), `turn_a_refresh_failure_closes_the_socket_with_a_named_stage`
(437 and 508, one failing Refresh only), `turn_a_bind_failure_closes_the_socket_with_a_named_stage`,
`turn_a_wrong_password_names_the_allocate_stage`, `turn_a_silent_server_names_the_allocate_stage`,
`turn_a_tcp_dial_is_refused_by_name`, `turn_each_socket_has_its_own_allocation`, `turn_over_the_datagrams_of_the_way_below`,
`turn_the_router_takes_the_network_in_networks`, the spec and account tests, and the command: `a_udp_forward_goes_through_a_turn_allocation`
(`podnet forward -L udp:PORT=PEER --network turn://STANDIN`: the query gets its answer; the output holds no password),
`a_turn_network_with_no_account_is_a_usage_error_that_names_the_variables`, `a_turn_spec_with_a_password_in_it_is_refused_and_not_echoed`,
`a_turn_wrong_password_ends_the_command_at_the_allocate_stage`, the manual test and the `--egress` refusal test. Fail first
(plants): with the channel not kept, the bind count was 2 (not 1) in two tests; with a refresh failure ignored, the refresh
test failed; with the `--egress` refusal off and the usage mapping off, the `way` test and the exit-code test failed (exit 1, not 64).
Not proven: a live TURN server.

