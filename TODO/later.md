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
**Status:** open

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

# T-120: Router kernel mode: masquerade, an MSS clamp, an unreachable kill switch and `rp_filter`

**Source:** `docs/research/podnet/mwan-field.md`, sections 1.4 (D1, D4), 1.6 (mode B) and 2.2
(R1).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

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

# T-121: Router: export the groups as a JSON file of the mwan form

**Source:** `docs/research/podnet/mwan-field.md`, sections 2.2 (R4) and 5 (question 5).
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** open

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

# T-122: Research: a native git server with no installed `git`

**Source:** `docs/research/podnet/capabilities.md`, section 5.3 (design C); the owner's ruling of
2026-11-05 (`docs/decisions.md`, Git hosting).
**Category:** research
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

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
**Status:** open

## Problem

podnet can dial a WireGuard peer, but cannot be one. A WireGuard endpoint with no
`CAP_NET_ADMIN`, whose inner flows go to podnet networks, does not exist.

## Premise

- `src/warp/wg.rs` has the initiator, and a responder for its tests (`respond`).
- `src/vnet/device.rs` has a link device for smoltcp.

## Approach

1. Add the responder half: cookie replies, several peers and the timers.
2. Send each inner flow to a dialer of the registry.
3. Add `podnet wg serve` with a key file and a list of peers.

## Prove

```sh
cargo test --workspace -- wg_serve_accepts_a_peer_and_dials_its_flows
```

## Start condition

T-117 is done.

## Correction

2026-11-05, the responder half of the WireGuard code (`src/warp/wg.rs`) is in `pod-wg` (wave 2). The work goes to
`pod-wg` in PodUtils/podcrates; podnet takes it and adds `podnet wg serve` (T-137).

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
**Status:** open

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

# T-131: A crate that podnet and podssh share for the layer, the channel and the identity

**Source:** `docs/research/podnet/podssh-copy.md`, section 4 (option B); the owner's ruling of
2026-11-05 (`docs/decisions.md`, Code of podssh).
**Category:** chore
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

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

# T-132: `try_update` in place of the macro `update_atomic!`

**Source:** `docs/maintenance.md`, section "Open work" (`try_update`).
**Category:** chore
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

`src/lib.rs` has the macro `update_atomic!`, a loop of `compare_exchange_weak`, in place of the
deprecated `fetch_update` (#214). `src/share.rs` and `src/gate.rs` use it.

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
**Status:** open

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

# T-153: netns: a backend with an in-namespace netfilter data plane (GitHub #352)

**Source:** GitHub #352 (https://github.com/PodUtils/podnet/issues/352)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

## Problem

Same as T-152. The goal: kernel-delivered TCP and UDP for the netns backend, no smoltcp on the hot path,
no flow table of T-111.

## Premise

- Depends on T-152. T-111 (open, M7) plans the userspace flow table. This entry must not replace it
  before the owner rules (see Decision).
- The nfnetlink client is new code (the vnet ACL avoids kernel netfilter, `src/vnet/policy.rs`). T-120
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

# T-165: The SSH forward-relay leg: supervise the session after `run()` returns (GitHub #362)

**Source:** GitHub #362 (https://github.com/PodUtils/podnet/issues/362)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

`relay::run` (`src/relay.rs:432`) dials the relay, requests one remote forward, reads the URL from the
banner and returns `(TunnelHandle, Handle<Client>)`. Nothing watches the session after that. When the
relay ends the connection or the forward, the URL is dead and the caller is not told. The default
relay's free tier ends the tunnel after 60 minutes (the banner text of the test at `src/relay.rs:569`).
A reconnect gives a new port and a new URL. The reverse road (`relay node`, `src/relay_reverse.rs`)
reconnects with a 1 s to 30 s backoff; this leg does not.

## Premise

- `inactivity_timeout` is 3600 s (`src/relay.rs:444`); the forward is requested once (`:474-477`).
- `grep -rn "relay::run\|RelayConfig" src tests examples` finds no caller outside `src/relay.rs`.
  `ShareLink::from_relay` (`src/feature/mod.rs:1263`) has no caller outside a test. No CLI uses the leg.
- Not measured: the 60 minute expiry on a live relay; the `Handle::is_closed` behaviour of russh 0.64.1.

## Approach

1. A supervisor task: detect the end (`Handle::is_closed`, a failed keepalive, EOF of the banner
   channel), then reconnect with the capped backoff of `relay_reverse.rs`.
2. Give the new URL to the caller (a channel or a callback) at each reconnect.
3. Log one line that names the relay and the reason of the end.
4. `doctor`: add one line to the row at `src/doctor.rs:743-749` about the fixed session age.
5. Do not hardcode 60 minutes.

## Decision

The lead, 2026-11-09: keep the SSH forward-relay leg as a library with no caller. Park this entry. Expose the leg only after a ruling of the owner. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet relay::tests::the_supervisor_reconnects_and_announces_the_new_url
```

The test uses a local SSH server of russh that closes the session, and checks two URLs.

## Start condition

The owner decides that this leg gets a CLI. Until then, the leg stays library-only.

# T-166: The SSH relay leg: the relay, the user and the URL rule are data (GitHub #363)

**Source:** GitHub #363 (https://github.com/PodUtils/podnet/issues/363)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** blocked

## Problem

`RelayConfig::default()` has one relay (`src/relay.rs:42`), the user `podnet` (`:66`), and one URL rule
(`pick_tunnel_url`, `:399-412`): the first token that starts with `http(s)://` and does not hold the
dashboard host of that one relay. Another relay that prints a bare host, a URL split over two data
frames, or its own dashboard URL fails or gives the wrong URL.

## Premise

- The fields of `RelayConfig` are `pub`; no flag sets them (no caller, see T-165). `--relay` is the
  flag of the WebSocket relay.
- `pick_tunnel_url` runs on each `ChannelMsg::Data` chunk (`src/relay.rs:504`), so a URL that
  crosses two chunks is lost. The pinned tests are at `src/relay.rs:566-598`.
- The dashboard host is already a fact of `pod-services` (`src/relay.rs:407`).
- Not measured: the other relays of the survey (local artefacts of the issue).

## Approach

1. Collect the banner over chunks, then pick the URL.
2. Exclude every known dashboard host, not only one. Prefer the tunnel domain of the relay.
3. Read the endpoint, the user and the URL-line shape of each relay from `pod-services`. A new
   relay fact is an ask to podcrates (COORD, `(to podcrates)`). Add no host in podnet.
4. An address list tried in order (as README.md:433-435 for the WebSocket relay).
5. CLI flags only with the CLI of T-165.

## Decision

The lead, 2026-11-09: keep the SSH forward-relay leg as a library with no caller. Park this entry. The relay facts go to `pod-services` of `PodUtils/podcrates` (an ask). The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet relay::tests -- picks_the_tunnel_url url_across_two_frames
```

## Blocker

The session of `PodUtils/podcrates` (the crate `pod-services`): the relay facts (relay host, user, URL rule) are data of that crate. Send a COORD issue (to podcrates). The entry is unblocked when `pod-services` has the facts.

## Start condition

T-165.

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
**Status:** open

## Problem

No entry plans a router that ends guest TCP on smoltcp (T-095 to T-098 are doors with per-flow dials).
If one is built, a prototype found: (1) a closed flow holds its tuple up to 10 s (`TimeWait` close delay);
a new SYN for a stale tuple gets no answer; (2) dial before the handshake (`HeldSyn`) is better than a
dial at `SynReceived`; (3) a guest UDP socket needs a bounded pending queue and a stop-reading-the-wire
signal; a partial write must keep the byte offset; the socket buffer must hold 65535 plus metadata, and
an oversized datagram needs its own counter.

## Premise

- Present in podnet: `HeldSyn` (`crates/podnet-netns/src/netns/backend.rs:13,146`, `crates/podnet-netns/src/netns/packet.rs:62`); `reap_closed`
  (`crates/podnet-netns/src/netns/backend.rs:242`) reaps at `Closed`; the vnet stack treats `TimeWait` as closed (`src/vnet/stack/service.rs:270-287`)
  and defers `close` until the queues are empty (`src/vnet/stack/service.rs:254-267`). `NetStack` defaults hold the payload at
  64 KiB (`:114-115`). Async writers use `write_all` (`upstream.rs:626`, `framing.rs:43`).
- No present defect. smoltcp lines are not re-read; the prototype is not in the repo.
- Overlap: #352 (netns netfilter), T-111 (takes datagrams before smoltcp).

## Approach

Keep the list in the Problem as the acceptance rules of any such router. Do no work now.

## Prove

```sh
cargo test -p podnet-netns -- a_stale_tuple_is_released_by_a_new_syn a_partial_stream_write_keeps_the_record_stream_aligned
```

## Start condition

A guest-wire router (smoltcp) is planned.

# T-178: Router: named group policies (`rr`, `minrtt`, `leastload`, `protect`) (GitHub #375)

**Source:** GitHub #375 (https://github.com/PodUtils/podnet/issues/375)
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

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

# T-179: Research: a direction split for asymmetric uplinks (GitHub #375)

**Source:** GitHub #375 (https://github.com/PodUtils/podnet/issues/375)
**Category:** research
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

A fast-download line beside a slow-upload line fits a split: the request leg on one uplink, the
response leg on another, with the exit of each flow kept at the terminating side. It needs a relay node
or a chain member that ends a leg.

## Premise

- It contradicts T-096: "One flow uses one uplink". Adopting it changes that contract (owner; see Decision).
- `src/race.rs:54` is the per-leg primitive; `src/warp/forward.rs` ends one leg and re-dials the other.

## Approach

Write the design in `docs/research/podnet/`: the flow id, the direction bit, the exit table, the cleanup.
Do no code.

## Decision

The lead, 2026-11-09: research only. Do not adopt a direction split now: it breaks "one flow uses one uplink" of T-096. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
grep -n "direction split" docs/research/podnet/*.md
```

## Start condition

The owner changes the T-096 contract.
