This file holds the entries of milestone M3: the extraction of the crates from the bottom up, one
crate in each commit. podnet has five crates (`docs/decisions.md`, 2026-11-05, Shared crates):
`podnet` (the CLI, at the root), `podnet-core`, `podnet-netns`, `podnet-vnet` and
`podnet-serve`. `podnet-netns` exists after T-020. The relay client, the WARP adapter and the
Cloudflare adapter are crates of `PodUtils/podcrates` (`pod-relay`, `pod-warp`, `pod-cloudflare`):
T-049, T-050 and T-052 are takes of these crates through T-137, not work in podnet. The shared
crates come by a git tag, not from crates.io.
`docs/research/podnet/architecture.md`, sections 4.1, 4.2, 4.5 and 4.6 (step E), gives the
contents of each crate and the edges between them. In each commit: a new manifest with a subset
of the dependencies, a `pub use` in the root library for each old path, `pub` (with
`#[doc(hidden)]`) only where a higher crate needs an item, `fuzz/Cargo.toml` and
`fuzz/Cargo.lock`, the MSRV check of `.github/workflows/ci.yml`, and the table "The code" of
`docs/maintenance.md`.

# T-046: Prepare the workspace for more crates: the print macros, the build facts, the scans, the package list and the test helpers

**Source:** `docs/research/podnet/architecture.md`, section 4.2 (the rows Macros, Build info,
Environment scan, cargo package and Versions).
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Several facts of the root crate assume one library. A second library crate cannot build, or its
checks do not run, until they change.

## Premise

- The write-safe print macros and `update_atomic!` are defined in `src/lib.rs`.
- `build.rs` sets the build commit, target and date for the root crate only. `src/fetch.rs`,
  `crates/podnet-core/src/report.rs`, `src/config.rs`, `src/quicktunnel.rs` and `src/tunnel.rs` read them.
- The variable scan of `src/manual/facts.rs` reads fixed directories.
- `.github/workflows/ci.yml` lists two packages for `cargo package --list`.
  `tests/docs.rs::podnet_netns_ships_the_license` checks the licence of one crate.
- `need_loopback!` and `need_localhost!` are in `tests/test_host/mod.rs`, included by
  `#[path]`. A crate cannot include a file outside its directory and still package.

## Approach

1. Define the print macros and `update_atomic!` in one macro of the core that each crate calls at
   its root, or copy their 25 lines into each crate.
2. Give the core a version function that the binary sets at start, from the build facts of
   `build.rs`.
3. Make the variable scan read `src`, each `crates/*/src` and the directory of the shims.
4. Make the package list of `ci.yml` read each member of the workspace. Check a `LICENSE` copy in
   each crate.
5. Put `need_loopback!` and `need_localhost!` into the core, behind a feature `testkit` that only
   the dev-dependencies of the crates turn on.
6. Measure the build before the split: `cargo build --timings`, and the rebuild after a touch of
   the WARP CLI module and of the egress module. Write the numbers in the Done section.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Crates): the test helpers go into the core behind a
feature, not into an extra crate of podnet.

## Prove

```sh
sh scripts/agent/check.sh
cargo build --timings
```

## Start condition

T-045 is done.

## Correction

2026-11-05, the ruling "Shared crates" (`docs/decisions.md`) supersedes "Crates: eight crates": podnet has
five crates. `pod-testkit` (wave 1 of PodUtils/podcrates) is a shared crate for tests. Before step
5, read `pod-testkit`. When it has `need_loopback!` and `need_localhost!`, take them from it. When it has
not, open an issue in PodUtils/podcrates. Step 1 to 4 stay.

2026-11-10: step 1, 2 and 5 changed. Step 1: one macro of the core that makes `macro_rules!
println` in the crate that calls it does not compile (E0659: the name is ambiguous with the
`println!` of the prelude; measured with Rust 1.99.1). Each crate has a copy of the 25 lines.
Step 2: the core has its own `build.rs` (as `crates/podnet-netns/build.rs`), not a value that the
binary sets at start: a library user and the tests get the commit with no call. Step 5:
`pod-testkit` at `v0.10.0` has `need_loopback!` and `need_localhost!`. The core takes them from it
as a dev-dependency. The core has no feature `testkit`, and `tests/test_host/mod.rs` (frozen, it
goes to `pod-testkit`) does not change.

## Done

2026-11-10, commit dede6df. `crates/podnet-core` is a member of the workspace with
`podnet_core::build` (`VERSION`, `COMMIT`, `version()`), its own `build.rs`, a copy of the print
macros and `update_atomic!`, and `pod-testkit` as a dev-dependency. `crates/podnet-core/src/report.rs` reads the
version line from the core. The variable scan of `src/manual/facts.rs` reads `src`, `shim`,
`build.rs`, and `src` and `build.rs` of each directory of `crates/`. `.github/workflows/ci.yml`
packs each member that cargo may publish (`cargo metadata`) and checks `LICENSE`, `README.md`
and `Cargo.toml` in each list. `tests/docs.rs::each_crate_ships_the_license` (the former
`podnet_netns_ships_the_license`) checks each crate. `scripts/check-layers.py` reads the crates
of `CRATES` (self-test: 4 new cases).

- `sh scripts/agent/check.sh`: fmt 0, clippy 0, record 0, size 0, doc 0, layers 0, test 0.
- Parity (`sh scripts/agent/parity-snapshot.sh`, one rename in the map): commands 1295 of 1295,
  items 1297 of 1294, tests 1110 of 1109.
- Step 6, the build before the split (`cargo build --timings`, debug, `CARGO_PROFILE_DEV_DEBUG=0`,
  4 jobs, the dependencies built): the crates of the workspace 30.5 s; the rebuild after a touch
  of `src/cli/cmd/warp/up.rs` 2.0 s, and of `src/egress.rs` 5.6 s.
- CI: at the merge.

# T-047: Extract the crate `podnet-core`

**Source:** `docs/research/podnet/architecture.md`, sections 3.2, 4.1 and 4.6, step E.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** L
**Status:** done

## Problem

The core modules (about 11,700 lines) are in the root crate. Each edit of the CLI or of an
adapter checks them again, and the compiler does not hold the layers.

## Premise

- The core is: `exit`, `hop`, `state`, `report`, `decisions`, `metrics` (with `Metered`),
  `rlimit`, `yaml`, `race`, `util`, `qr`, `ports`; the I/O part of `net`, `trust`, `egress`,
  `proxy_auth`, `resolve`, `fetch`, `udp`, `connect`, the WebSocket bridge of
  T-043; `detach`, `instances` and `autostart`.
- After T-045, no core module refers to a higher layer.
- A `pub use` of a moved module keeps `crate::module::Item` valid inside the root crate.

## Approach

1. Make crates/podnet-core. Move the modules with `git mv`.
2. In `src/lib.rs`, add `pub use podnet_core::{exit, state, egress, ...};` for each moved module.
3. Make `pub` (with `#[doc(hidden)]`) only the items that a higher crate needs.
4. Change the fuzz manifests, the MSRV check and the table "The code".

## Prove

```sh
sh scripts/agent/check.sh
cargo test -p podnet-core
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
cargo check --manifest-path fuzz/Cargo.toml --bins
gh workflow run live.yml
```

## Start condition

T-046 is done.

## Correction

2026-11-05, the module tls_fragment is gone (the split of the ClientHello is in `pod-tls`), and
`crates/podnet-core/src/trust.rs` is glue over `pod-tls`. `src/egress.rs`, `PodUtils/podnet:src/proxy_auth.rs` and `src/fetch.rs` move to
`pod-proxy`, `src/resolve.rs` to `pod-dns`, and the WebSocket bridge of `src/ws.rs` to `pod-ws`
(T-137). Extract `podnet-core` from what stays in podnet after these takes. Recount the module list
when the entry starts. Step 4 names the MSRV job: it stays, and its plant step is gone.

2026-11-10, the recount. The core holds 20 modules: `exit`, `hop`, `state`, `report`,
`decisions`, `metrics`, `rlimit`, `yaml`, `race`, `util`, `qr`, `ports`, `net`, `trust`, `udp`,
`connect`, `services`, `detach`, `instances`, `autostart` (about 8,000 lines). The differences from
the Premise and from `docs/research/podnet/architecture.md`, section 4.1: `src/egress.rs` and
`src/fetch.rs` (they go to `pod-proxy`), `src/resolve.rs` (it goes to `pod-dns` and it uses
`egress`) and the WebSocket bridge of `src/ws.rs` (it goes to `pod-ws`) stay in the root crate
until podnet takes their crates. `proxy_auth` and `tls_fragment` are gone. `metrics` has no
`Metered` now. `services` (glue over `pod-services`) is new in the core: `hop`, `util`, `trust`
and `udp` use it. `error_line` of `report` reads the refusal of `egress`: it stays in
`src/report.rs`, which gives the rest of the core `report` at its old path. The tests that read a
higher layer (`warp`, `config`, `origin`) or a file of the repository (`Cargo.lock`, the scan of
the verifiers) stay in the root crate. Step 4: the MSRV job reads `rust-version` of the root, and
the core has the same; it needs no change. The fuzz manifest needs no change; `fuzz/Cargo.lock`
gets the core.

## Done

2026-11-10, commit 4693062. `crates/podnet-core` holds the 20 modules of the Correction (moved
with `git mv`). `src/lib.rs` gives each at its old path with `#[doc(inline)] pub use`, so
`crate::exit::Item` and `podnet::exit::Item` stay valid. One item became public with
`#[doc(hidden)]`: `net::split_host_port` (`crates/podnet-serve/src/origin/dial.rs` uses it). Five tests moved to the
root crate: `core_tests::services::the_nameservers_of_the_service_are_1_1_1_1_and_1_0_0_1`,
`core_tests::yaml::a_config_error_names_its_line`, the two tests of `core_tests::trust`, and
`origin::dial::tests::an_unverified_origin_is_named_one_time`. The core tests take
`need_loopback!` from `pod-testkit`. The documents, the plants of `.github/workflows/ci.yml`, the
layer check (`scripts/check-layers.py`, `core_tests`) and the table "The code" of
`docs/maintenance.md` name the new paths.

- `sh scripts/agent/check.sh`: fmt 0, clippy 0, record 0, size 0, doc 0, layers 0, test 0.
- `cargo test -p podnet-core`: 102 passed.
- `unshare -n -- cargo test --offline --workspace --no-fail-fast`: exit 0, 1109 passed, 0 failed.
- `sh scripts/restricted-host.sh --no-fail-fast`: exit 0, 101 tests skipped.
- `cargo check --manifest-path fuzz/Cargo.toml --bins`: exit 0.
- Parity (`sh scripts/agent/parity-snapshot.sh`, against the tree before T-046, with a map of the
  moved tests; the map now takes `OLD<TAB>NEW`): commands 1295 of 1295, items 1566 of 1294 (the
  1202 items of `podnet` all kept), tests 1110 of 1109. No item is lost.
- The build after the split (as in T-046, step 6): the crates of the workspace 28.5 s; the rebuild
  after a touch of `src/cli/cmd/warp/up.rs` 1.8 s, of `src/egress.rs` 5.0 s, and of
  `crates/podnet-core/src/state.rs` 5.2 s.
- CI and `gh workflow run live.yml`: at the merge.

# T-048: Extract the crate `podnet-vnet`, with the C sources of the shims

**Source:** `docs/research/podnet/architecture.md`, sections 4.1 and 4.2 (the row cargo package).
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The userspace network (the module `vnet`, about 8,400 lines) is in the root crate. Its shims are C
files in the directory shim of the root, which `vnet::shim` includes with `include_str!`.

## Premise

- `include_str!` must name a file inside the crate, or `cargo package` fails.
- `.github/workflows/ci.yml` checks the C file of the shim in the package list of the root
  crate.
- `podnet-vnet` depends on the core and on `podnet-netns`
  (`docs/research/podnet/architecture.md`, section 4.1).

## Approach

1. Make crates/podnet-vnet with the module `vnet`. Move the files of the directory shim into the crate.
2. Move the package check of the shim to `podnet-vnet`.
3. Keep `tests/shim_direct.rs`, `tests/shim_no_socket.rs`, `tests/shim_socks.rs` and
   `tests/af_inet_kernel.rs` in the root crate: they run the binary.

## Prove

```sh
sh scripts/agent/check.sh
cargo test -p podnet-vnet
cargo test --test shim_direct --test shim_no_socket --test shim_socks
cargo package --list -p podnet-vnet | grep shim
```

## Start condition

T-047 is done.

## Correction

2026-11-05, the module `vnet` uses smoltcp 0.12 (`Cargo.toml`). The pin has no reason in the toolchain now
(Rust 1.99). The userspace stack of `pod-netstack` (wave 2 of PodUtils/podcrates) uses smoltcp 0.14.
Before the extraction, read whether the module `vnet` takes `pod-netstack` (T-137); if it does, do not extract the
stack code into `podnet-vnet`.

2026-11-10, the read of T-137. The module `vnet` does not take `pod-netstack`: T-137, step 2, puts the
userspace stack of `src/warp/` on `pod-netstack`, and no `crates/*/SOURCES` of PodUtils/podcrates
names a file of the module (`sh scripts/agent/podcrates.sh`). PodUtils/podcrates has no crate
`pod-netstack` on main now. The extraction goes on. The module uses `state`, `net` and `udp` of the
core and `podnet-netns`; it uses no module that goes to PodUtils/podcrates (`src/egress.rs`,
`src/resolve.rs`, `src/ws.rs`, the relay, `src/warp/`, `src/cloudflare/`), so each file moves. The crate
is the module itself: `crates/podnet-vnet/src/lib.rs` is the old `mod.rs`, and `podnet_vnet::stack` is
the old `podnet::vnet::stack`. `crates/podnet-vnet/shim/unixsockd.c` and `crates/podnet-vnet/shim/loginshell` are not in an `include_str!`:
`tools/ts-sshd.sh` compiles them, and they move with the other shims. No workflow compiles a shim:
the binary has the C sources and compiles them at run time; `release.yml` names no shim path.

## Done

2026-11-10, commit b333b53. `crates/podnet-vnet` holds the module `vnet` (moved with `git mv`, the
crate root is the old `mod.rs`) and the shims in `crates/podnet-vnet/shim/` (`podnet-vnet.c`, `podnet-socks.c`,
`fakepwd.c`, `tsgetent`, `tsid`, `unixsockd.c`, `loginshell`). `src/lib.rs` gives the crate at its
old path (`#[doc(inline)] pub use podnet_vnet as vnet`), so `crate::vnet::Item` and
`podnet::vnet::Item` stay valid. No item became public. The package check of
`.github/workflows/ci.yml` reads the five shims of `include_str!` in the package of `podnet-vnet`.
`scripts/check-layers.py` has the crate in `CRATES` and `ONE_MODULE` (each file is the module
`vnet`; a `podnet_vnet::` path is a reference to it). The test
`manual::facts::tests::each_variable_in_the_manual_is_read_by_the_source` reads the directory shim
of each crate. `tools/ts-sshd.sh`, the documents and the record name the new paths.

- `sh scripts/agent/check.sh`: fmt 0, clippy 0, record 0, size 0, doc 0, layers 0, test 0.
- `cargo test -p podnet-vnet`: 88 passed.
- `cargo test --test shim_direct --test shim_no_socket --test shim_socks`: 11, 6 and 1 passed.
- `cargo package --list -p podnet-vnet`: the directory shim has the seven files; the package of
  `podnet` has no file of it.
- `unshare -n -- cargo test --offline --workspace --no-fail-fast`: exit 0, 1109 passed, 0 failed.
- `sh scripts/restricted-host.sh --no-fail-fast`: exit 0, 100 tests skipped.
- `cargo check --manifest-path fuzz/Cargo.toml --bins`: exit 0.
- Parity (`sh scripts/agent/parity-snapshot.sh`, against the tree before the move, with the map
  `src/lib.rs podnet vnet::<TAB>src/lib.rs podnet_vnet `): commands 1295 of 1295, items 1786 of 1566,
  tests 1110 of 1110. No item is lost.
- CI: at the merge.

# T-049: The relay client of podnet becomes the shared crate `pod-relay`

**Source:** `docs/research/podnet/architecture.md`, section 4.1.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

The relay client and the reverse road (about 2,400 lines) are in the root crate.

## Premise

- The crate takes the client of `src/ws_relay.rs` (`Relay`, `Host`, the mint, the token cache,
  `RelayBudget`, `websocket_at`), src/relay_pool.rs and `src/relay_reverse.rs`.
- The HTTP/2 leg through the relay stays for the Cloudflare crate (T-052).
- `tests/relay_reverse.rs` calls the library of the reverse road.

## Approach

1. Take `pod-relay` of PodUtils/podcrates (a git tag) in place of the relay modules. Remove
   `src/ws_relay.rs`, src/relay_pool.rs and `src/relay_reverse.rs` from podnet when the crate has
   their code.
2. Leave the HTTP/2 leg of `src/ws_relay.rs` in the root crate until T-052.

## Decision

The podcrates design of 2026-11-05 (`docs/decisions.md`): `podnet-relay` merges into `pod-relay` of
`PodUtils/podcrates`, with the relay client of podssh. podnet has no crate `podnet-relay`.

## Prove

```sh
sh scripts/agent/check.sh
cargo tree -p podnet --depth 1 | grep pod-relay
cargo test --test relay_reverse
gh workflow run live.yml
```

## Start condition

T-047 is done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The work goes to `pod-relay` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. The Approach names the take. A fix of the relay
code before the take goes to the podcrates session too (`docs/decisions.md`, Who edits a shared crate).

## Done

2026-11-12, the take of podcrates v0.30.0 with `adoption/podnet/pod-relay.patch` (trial at 98d8a71: `adoption.yml`
43191076065, 22 steps). `src/ws_relay.rs` (353 lines, was 994) is glue over `pod-relay` with one `Context` of the
process; `ws_relay::{set, active}` stay. the pool and the session files of the old client are gone;
`src/relay_reverse.rs` (532 lines, was 891) runs over `pair::{create, stop}`, `reverse::run` and `operator::run`.
`pod-relay` has the features `pair` and `plain-ws`. Changes for a user: a node whose name another node holds exits 69
after 30 s; a Close 1008 with "token" does not stop a node (a 401 or 403 of its upgrade does); a pair file is
`relay/pair-NAME.json` and an old `relay/NAME.json` is not read; a bad pair name of `relay connect` exits 64. The
steps of `pod-relay.run` that ran here passed: the record, size, layers, build, `cargo deny check bans sources`, the
features in the tree, the files and the items gone, clippy, the tests, the fuzz check, rustfmt; `cargo test
--workspace --no-fail-fast`: 0 failed. Not run here: `check-consumer-toolchains.py` and the zig build (CI builds
the static binaries).


# T-050: Move the WARP adapter to the shared crate `pod-warp` in `PodUtils/podcrates`

**Source:** `docs/research/podnet/architecture.md`, section 4.1.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The WARP adapter (`src/warp/`, 21 files, about 14,500 lines) is in the root crate.

## Premise

- The WARP adapter depends on the core, on `podnet-vnet` (`device::LinkDevice`), on the relay
  client (MASQUE through the relay) and on `podnet-netns` (the backends of `warp run`).
- The stand-ins of T-012 move with it.

## Approach

1. Move `src/warp/` to `pod-warp` in `PodUtils/podcrates`, with the stand-ins of T-012 as its tests.
2. Replace the links to the crates of podnet with traits of `pod-warp`.
3. podnet depends on `pod-warp` by a git tag of PodUtils/podcrates, and keeps the WARP CLI in the root crate.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the WARP adapter moves to `PodUtils/podcrates` as `pod-warp`. A shared crate cannot depend on the crates of podnet: the links to `podnet-vnet`, the relay client and `podnet-netns` become traits of the shared crate, or move to shared crates too.

## Prove

```sh
sh scripts/agent/check.sh
cargo tree -p podnet --depth 1 | grep pod-warp
cargo test --workspace -- warp_stand_in_
gh workflow run live.yml
```

## Start condition

T-048 and T-049 are done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The work goes to `pod-warp` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. `pod-warp` is in wave 3 and uses smoltcp 0.14 and
Rust 1.99. A fix in `src/warp/` before the take stays a fix, and the podcrates session ports it.

## Done

2026-11-13: the take of podcrates `v0.37.0` with `adoption/podnet/pod-warp.patch` (T-137). `pod-warp` of
PodUtils/podcrates holds the WARP adapter; podnet depends on it by `tag = "v0.37.0"` and keeps the WARP CLI and the glue in
`src/warp/`. The stand-ins of T-012 are the tests of `pod-warp` in podcrates. Checks: the steps of
`adoption/podnet/pod-warp.run` and `sh scripts/agent/check.sh` (fmt, clippy, record, size, doc, layers and test: 0).

# T-051: Extract the crate `podnet-serve`, with the library-only modules

**Source:** `docs/research/podnet/architecture.md`, sections 3.2 and 4.1.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The services (about 9,300 lines) are in the root crate. About 5,600 lines of them have no caller
in the binary.

## Premise

- The crate takes `crates/podnet-serve/src/share/mod.rs`, `crates/podnet-serve/src/gate.rs`, `crates/podnet-serve/src/service.rs`, `crates/podnet-serve/src/ingress.rs`,
  `crates/podnet-serve/src/origin/`, `crates/podnet-serve/src/feature/`, `src/transport/`, `crates/podnet-serve/src/relay.rs` and the client of the
  WebSocket forward.
- `crates/podnet-serve/src/feature/`, `crates/podnet-serve/src/origin/`, `src/transport/` and `crates/podnet-serve/src/relay.rs` are a capability of the
  library. The owner's ruling on regressions keeps them.

## Approach

1. Make crates/podnet-serve with these modules. It depends on the core.
2. Keep each public path through the facade (`tests/api_surface.rs`).

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Regressions): no feature or capability of the code
is lost. The library-only modules stay.

## Prove

```sh
sh scripts/agent/check.sh
cargo test -p podnet-serve
cargo test --test api_surface
```

## Start condition

T-047 is done.

## Correction

2026-11-05, `podnet-serve` stays in podnet. The modules that use `PodUtils/podnet:src/proxy_auth.rs`, `src/egress.rs` or
`src/ws.rs` use `pod-proxy` and `pod-ws` after T-137. Recount the module list when the entry starts.

2026-11-10, the recount. The crate holds 7 modules: `share`, `gate`, `service`, `ingress`,
`origin`, `feature` and `relay` (about 6,300 lines). The differences from the Premise:
`src/transport/` stays in the root crate, because `src/transport/websocket.rs` dials through
`src/egress.rs` (it goes to `pod-proxy`). The client of the WebSocket forward (`src/ws.rs`) stays
in the root crate: it goes to `pod-ws`, and it uses `cloudflare`, `proxy` and `relay_reverse`.
`ingress` uses `pod-proxy` (the credentials of a served proxy) and `origin` uses `pod-tls`: these
are crates of PodUtils/podcrates, not modules of the root. The test
`feature::tests::share_link::a_quick_tunnel_link_is_built_from_the_provisioned_url` reads
`quicktunnel`: it moves to `src/serve_tests.rs` in the root crate. Three items of `ingress` that
`src/config.rs` uses become public with `#[doc(hidden)]`: `RawIngress`, `RawOriginOptions` and
`ingress_from_raw`.

## Done

2026-11-10, commit 585acd0. `crates/podnet-serve` holds the 7 modules of the Correction (moved with
`git mv`): `share`, `gate`, `service`, `ingress`, `origin`, `feature`, `relay`. It depends on
`podnet-core`. `src/lib.rs` gives each at its old path with `#[doc(inline)] pub use`, so
`crate::share::Item` and `podnet::share::Item` stay valid. A module of the crate names the core as
`podnet_core::MODULE`. Three items became public with `#[doc(hidden)]`: `ingress::RawIngress`,
`ingress::RawOriginOptions`, `ingress::ingress_from_raw` (`src/config.rs` uses them). One test moved
to the root crate: `serve_tests::feature::a_quick_tunnel_link_is_built_from_the_provisioned_url`.
`src/transport/` and `src/ws.rs` stay in the root crate. The layer check
(`scripts/check-layers.py`: `CRATES`, the paths `podnet_serve::`, `serve_tests`, three new self-test
cases), a plant of `.github/workflows/ci.yml`, the table "The code" and the release steps of
`docs/maintenance.md`, the crate table of `README.md` and the record name the new paths. The package
list of `ci.yml` reads the members of the workspace: it takes the crate with no change.

- `sh scripts/agent/check.sh`: fmt 0, clippy 0, record 0, size 0, doc 0, layers 0, test 0.
- `cargo test -p podnet-serve`: 111 passed.
- `cargo test --test api_surface`: 2 passed.
- `unshare -n -- cargo test --offline --workspace --no-fail-fast`: exit 0, 1109 passed, 0 failed.
- `sh scripts/restricted-host.sh --no-fail-fast`: exit 0, 100 tests skipped.
- `cargo check --manifest-path fuzz/Cargo.toml --bins`: exit 0.
- Parity (`sh scripts/agent/parity-snapshot.sh`, against the tree before the split, with a map of
  the moved tests): commands 1295 of 1295, items 1657 of 1566 (the 1226 items of `podnet` all
  kept), tests 1110 of 1110. No item is lost.
- CI: at the merge.

# T-052: Move the Cloudflare adapter to the shared crate `pod-cloudflare` in `PodUtils/podcrates`

**Source:** `docs/research/podnet/architecture.md`, sections 4.1 and 4.2 (the row cargo package).
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The Cloudflare tunnel (about 12,000 lines) is in the root crate. The owner made it one adapter of
podnet.

## Premise

- The crate takes `src/cloudflare/` (with `src/cloudflare/cf-edge-roots.pem`,
  `src/cloudflare/cloudflared-pin.txt` and `src/cloudflare/schemas/`), `src/tunnel.rs`,
  `src/edge_h2.rs`, `src/edge_discovery.rs`, `src/quicktunnel.rs`, `src/config.rs`,
  `src/proxy.rs`, the edge side of the WebSocket forward, the HTTP/2 leg through the relay and
  the QUIC client configuration of the quic+warp leg.
- A test of `src/cloudflare/http2.rs` includes `tools/header-oracle.txt` from outside the crate.
  A test of the trust store reads `Cargo.lock`.
- `tools/check-upstream.py` and `.github/workflows/upstream.yml` read the pin file.

## Approach

1. Move the edge client, the tunnel and the quick tunnels to `pod-cloudflare` in `PodUtils/podcrates`.
2. Replace the links to the crates of podnet with traits of `pod-cloudflare`.
3. podnet depends on `pod-cloudflare` by a git tag of PodUtils/podcrates, and keeps the tunnel CLI in the root crate.
4. Move the two tests that read files outside the crate to the root crate of podnet.
5. Move the pin file of `tools/check-upstream.py` and the watch of `.github/workflows/upstream.yml`
   to `PodUtils/podcrates` with the edge client.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Name): the Cloudflare tunnel code is one adapter of
podnet.

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the Cloudflare adapter (the edge client of `src/cloudflare/`, the tunnel, the quick tunnels) moves to `PodUtils/podcrates` as `pod-cloudflare`. podssh uses it for its quick tunnels.

## Prove

```sh
sh scripts/agent/check.sh
cargo tree -p podnet --depth 1 | grep pod-cloudflare
python3 -I tools/check-upstream.py
gh workflow run live.yml
```

## Start condition

T-049, T-050 and T-051 are done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The work goes to `pod-cloudflare` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. The rule "no dependency for the edge protocol"
stays: `pod-cloudflare` implements it.

# T-053: The root package is the CLI and a facade that keeps each library path

**Source:** `docs/research/podnet/architecture.md`, sections 4.1, 4.2 (the row Fuzz crate), 4.5
and 4.6, step E; `docs/research/podnet/podssh.md`, section 3 (the path map).
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

After T-052, the root package holds the CLI, the diagnostics and the facade. The fuzz targets
use crate-private items of several crates. No document maps each crate to its purpose.

## Premise

- The root keeps `doctor`, `preflight`, `agent`, `status`, `bug_report`, `manual`, `update` and
  `fuzzing` (`docs/research/podnet/architecture.md`, section 4.1).
- Some fuzz targets use `pub(crate)` items, for example the request head builder of the proxy
  (`src/fuzzing.rs`).
- `tests/api_surface.rs` checks the shapes of the library.

## Approach

1. Make `src/lib.rs` a facade: `pub use pod_warp as warp;`, `pub use podnet_vnet as vnet;` and
   the other modules and root re-exports.
2. Give each crate a `#[doc(hidden)] pub mod fuzzing`. Make the facade module call them.
3. Write the path map in `docs/maintenance.md`, section "The code": each crate, its purpose and
   its main modules.
4. Measure the build again, as in T-046. Write both numbers in the Done section.

## Prove

```sh
sh scripts/agent/check.sh
cargo test --test api_surface
cargo check --manifest-path fuzz/Cargo.toml --bins
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
cargo build --timings
```

## Start condition

T-052 is done.

## Correction

2026-11-05, The facade re-exports `pod-warp`, `pod-cloudflare` and `pod-relay` (shared crates), and
`podnet-core`, `podnet-vnet` and `podnet-serve` (podnet). Each fuzz target that uses a private item of
a shared crate needs a `#[doc(hidden)] pub mod fuzzing` in that crate: open an issue in
PodUtils/podcrates.

2026-11-11, T-059: the registry of networks (`src/networks.rs`) has no cargo features yet. The facade adds
the features `warp`, `cloudflare`, `relay` and `vnet`, and each row of the table depends on its feature.


# T-054: A prerelease after the crate split, with the static builds of each target

**Source:** `docs/research/podnet/architecture.md`, section 4.2 (the rows Release build and
cargo package); `docs/maintenance.md`, section "Release"; `CLAUDE.md` (Releases).
**Category:** release
**Milestone:** M3
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

The crate split changes the build of each release binary: five crates and the shared crates, each with its own
dependencies, and the C shims inside a crate. Only a release run proves the eight static targets
of cargo-zigbuild and the image.

## Premise

- `.github/workflows/release.yml` builds x86-64 and ARM64 in the build image, and the other
  targets (armv7 too) with cargo-zigbuild.
- `docs/maintenance.md`, section "Release", gives the steps.

## Approach

1. Set the next version `0.4.0-beta.N`. Write its note. Its `## Breaking` says "None." for the
   command line, when the parity snapshot is empty.
2. Run the dry run, then the publish, then `release-proofs.yml`.

## Decision

The owner (`CLAUDE.md`, Releases): cut the next prerelease unattended when a batch is green.

## Prove

```sh
gh workflow run release.yml --ref main
gh workflow run release.yml --ref main -f publish=v0.4.0-beta.N
gh workflow run release-proofs.yml -f tag=v0.4.0-beta.N
```

## Start condition

T-053 is done.

## Correction

2026-11-05, The ruling "Order" (`docs/decisions.md`): no podnet release now. Start this entry only when the
owner allows releases again. A `pod-*` dependency by a git tag stops `cargo publish` (the ruling
"Git dependencies and the publish"): the release builds with the git source and the publish stays off.

2026-11-13: the ruling "Releases" of 2026-11-09 allows betas (`docs/decisions.md`), and the betas up to v0.4.0-beta.8 are out. The line "no podnet release now" above is old. This entry stays open for its own work: a prerelease after the crate split, which waits for T-053.

# T-137: podnet uses the shared crates of `PodUtils/podcrates`

**Source:** the owner, 2026-11-05 (`docs/decisions.md`, Shared crates and Order); the proposal of
the podssh session of 2026-11-05 for `PodUtils/podcrates`.
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** M
**Status:** partial

## Problem

The ruling "Order" of 2026-11-05 held each plan of the record until the shared crates existed; the ruling "Order" of 2026-11-06 lets podnet's own work go on beside the takes.
podnet copies no code of podssh: the shared crates hold it. The WARP adapter and the Cloudflare
adapter of podnet move to the shared repository (T-050, T-052).

## Premise

- The design of `PodUtils/podcrates` (`docs/decisions.md`, podcrates design): 26 crates in four
  waves, and 8 names held only. podnet uses `pod-todo`, `pod-identity`, `pod-tls`, `pod-dns`,
  `pod-proxy`, `pod-ws`, `pod-netstack`, `pod-wg`, `pod-relay`, `pod-session`, `pod-e2e`,
  `pod-warp`, `pod-cloudflare`, `pod-wormhole`, `pod-transit` and `pod-testkit`.
- A shared crate takes a stream or dialer trait and a rustls `CryptoProvider`; the binary chooses
  the provider (podnet keeps ring). podnet takes each crate by a git tag of
  `PodUtils/podcrates` (`deny.toml`, `allow-git`), not from crates.io.
- podcrates is edition 2024 with `rust-version` 1.99 (`pod-todo` v0.1.0 needs 1.97). `pod-netstack` and `pod-warp` use smoltcp
  0.14. podnet has `rust-version` 1.99 and pins smoltcp 0.12 (`Cargo.toml`); the move to 0.14 comes with `pod-netstack`.
- podcrates v0.1.0 has `pod-todo`, `pod-core`, `pod-testkit`, `pod-term` and `pod-identity`. Its
  hand-off for podnet is `adoption/podnet/CRATE.patch` in `PodUtils/podcrates`.

## Approach

1. When wave 1 exists: replace `crates/podnet-todo` with `pod-todo` (done), and the TLS, DNS and proxy
   code of podnet (`crates/podnet-core/src/trust.rs`, `src/resolve.rs`, `src/egress.rs`) with `pod-tls` (done), `pod-dns`
   and `pod-proxy`.
2. When wave 2 exists: replace the relay client (`src/ws_relay.rs`, `src/relay_reverse.rs`) with
   `pod-relay` (no crate `podnet-relay`: T-049), and the userspace stack and WireGuard of
   `src/warp/` with `pod-netstack` and `pod-wg`; move to smoltcp 0.14 and Rust 1.99.
3. When wave 3 exists: replace `src/warp/` and `src/cloudflare/` with `pod-warp` and
   `pod-cloudflare` (T-050, T-052 hand their code to the podcrates session), and add
   `send`/`receive` on `pod-wormhole` and `pod-transit`.
4. Replace each copy entry of the record with its shared crate (T-012, T-086, T-102, T-114,
   T-123, T-128, T-131, T-134), and write the new order of the record in `TODO/PROGRESS.md`.

## Prove

```sh
cargo tree --workspace --depth 1
sh scripts/agent/check.sh
```

The tree shows the shared crates from the git tags of `PodUtils/podcrates`.

## Correction

2026-11-05, the rulings "Delivery of the shared crates", "Who edits a shared crate" and
"Toolchain" (`docs/decisions.md`): the crates come by a git tag of `PodUtils/podcrates`, not
from crates.io; the toolchain is Rust 1.99, not 1.91. The blocker is gone: `PodUtils/podcrates`
exists and its session runs.

Done so far:

- `pod-todo` v0.1.0 (6c210ed): `crates/podnet-todo` is a wrapper (`main` calls
  `pod_todo::main()`; the facts of podnet are in `TODO/todo.toml`).
- `pod-tls` and `pod-core` v0.2.0 (the patch `adoption/podnet/pod-tls.patch` of podcrates):
  `crates/podnet-core/src/trust.rs` is the glue (one context of the process on ring, the edge roots of Cloudflare as
  extra roots); the module `tls_fragment` is gone (the split is in `pod-tls`). Each `pod-*`
  dependency names `tag = "v0.2.0"`. The `rust-version` is 1.99 and each toolchain pin is 1.99.1;
  the `msrv` job has no plant step (no stable API is newer than 1.99).
- `pod-proxy` v0.3.0 (the patch `adoption/podnet/pod-proxy.patch` of podcrates): the module
  `proxy_auth` is gone; `src/egress.rs` and `src/fetch.rs` are glue over `pod-proxy` (podnet's
  rules of the variables, `--proxy`, the resolver chain, the counters, the words of a failure).
  `socks5://` resolves the name in podnet, `socks5h://` sends it. Each `pod-*` dependency names
  `tag = "v0.3.0"` with `version = "0.3.0"`.
- `pod-dns` and `pod-services` v0.4.0 (bf0c7ca; the patch `adoption/podnet/pod-dns.patch` of
  podcrates): `src/resolve.rs` is the glue of `pod-dns`; each value of a service of another party
  is a read of `pod-services` through `crates/podnet-core/src/services.rs`; `report::log_filter` gives `-v` the debug
  lines of the pod crates. Each `pod-*` dependency names `tag = "v0.4.0"`. The COORD issue is
  PodUtils/podnet#320.
- v0.5.0 (dfd8bd8): each `pod-*` dependency names `tag = "v0.5.0"`; no API change for podnet.
  The COORD issue is PodUtils/podnet#321.
- v0.6.0 (872a276): `rules()` sets `http_proxy_vars`; `live.yml` checks the DoH server again. The
  COORD issue is PodUtils/podnet#333. CI run 42263403147 at 872a276 and live run 42265938003 at 4094fee:
  success.
- v0.9.0 (191f2b4): each `pod-*` dependency names `tag = "v0.9.0"`; no API change for podnet.
  T-224 (127c095) ends the freeze of the datagram path of `crates/podnet-netns/src/upstream.rs`:
  podnet-netns calls `pod_proxy::udp`.
- `pod-ws` v0.13.0 (5eace6e; the patch `adoption/podnet/pod-ws.patch` of podcrates, PodUtils/podcrates#49):
  `src/ws.rs` takes the WebSocket client, the server session, `accept_key` and `close_hint` of `pod-ws`,
  and keeps its error words, `serve_websocket`, `forward_path`, `forward_url`, `LocalSpec` and the
  tokio-tungstenite bridge of the relay client until `pod-relay`. Each `pod-*` dependency names
  `tag = "v0.13.0"`. Each line of `adoption/podnet/pod-ws.run` passed; the static binary was built with
  clang for musl, as `cargo-zigbuild` is not on the host. The freeze of `src/ws.rs` ends.
- `pod-quic` v0.14.0 (b7b0aef; the patch `adoption/podnet/pod-quic.patch` of podcrates, PodUtils/podcrates#50):
  podnet moves from quinn to noq through `pod-quic` (`cloudflare/{mod,quic,rpc}`, `proxy`, `tunnel`,
  `warp/{h3,l4,quic,session}`, `ws`); no quinn stays in the tree. `warp::quic::endpoint` and
  `client_config` stay as glue. `src/warp/quic.rs` takes the version of the patch: `pod-quic` has the
  shared waker of T-228. Each `pod-*` dependency names `tag = "v0.14.0"`. Each line of
  `adoption/podnet/pod-quic.run` passed; the static binary was built with clang for musl.
- `pod-hold` v0.15.0 (the patch `adoption/podnet/pod-hold.patch` of podcrates, PodUtils/podcrates#54, T-232):
  the generic core of the hold of `crates/podnet-netns` (`seccomp/{filter,notif,mem}.rs`, `fdtable.rs`,
  `netns/fdpass.rs`, the setup of `netns/tun.rs`) is glue over `pod-hold` with podnet's names;
  `seccomp/mod.rs` starts the program with `pod_hold::spawn` and takes each notification with
  `pod_hold::listen`; `netns/mod.rs` enters the namespace with `pod_hold::netns`. `src/warp/tun.rs`
  takes `TUNSETIFF` of `pod-hold` (the number of the target: powerpc64le had a wrong number). Each
  `pod-*` dependency names `tag = "v0.15.0"`. Each line of `adoption/podnet/pod-hold.run` passed, except
  the static build with zig (no zig on the host; CI builds the static musl binaries).
- v0.17.0 (PodUtils/podcrates#57 and #58): each `pod-*` dependency names `tag = "v0.17.0"`, `pod-todo` too.
  v0.16.0 carries `pod_hold::connect` and `pod_hold::packet`; the patch that makes `connect.rs` and
  `netns/packet.rs` glue comes with a later release. No API change for podnet. `sh scripts/agent/check.sh`:
  each step 0.
- `pod_hold::connect` and `pod_hold::packet` (the patch `adoption/podnet/pod-hold-connect.patch` of podcrates
  8af9fb6, PodUtils/podcrates#59): `crates/podnet-netns/src/connect.rs` is glue over the dial state of
  `pod_hold::connect` (`Dials`, `Finished`, `write_error_to_task`); `netns/packet.rs` takes `parse_tcp_syn`,
  `rst_reply_for_syn` and `ipv4_checksum` of `pod_hold::packet` and keeps `dns_reply_packet`. Four tests moved
  to `pod-hold` with their names. Each line of the `.run` passed, except the static build with zig (no zig
  on the host) and the check of the toolchains of podcrates (no change of a toolchain).
- `pod-netstack` v0.20.0 (the patch `adoption/podnet/pod-netstack.patch` of podcrates, PodUtils/podcrates#64): the
  stack of `podnet-vnet` (the cut of PodUtils/podnet#427) is the crate, on smoltcp 0.14; `Acl`, the resolver and
  `Recorder` are podnet's parts behind `Admit`, `GatewayService` and `PacketTap`. Each smoltcp of podnet is 0.14;
  `.cargo/config.toml` sets `SMOLTCP_IFACE_MAX_ADDR_COUNT = "8"`. `tests/tcp_window.rs` uses
  `CongestionControl::None`, the default of podnet's stacks: on smoltcp 0.14, Reno and CUBIC grow the window by
  one segment for each ACK, so `warp --cc reno|cubic` is slower (podcrates T-188). Each pod-* dependency names
  `tag = "v0.20.0"`. Each line of the `.run` passed, except the zig build and the check of the toolchains.
- `pod-netstack` v0.27.0 (PodUtils/podcrates#85; 28c0e3d, ce28780): each `pod-*` dependency names
  `tag = "v0.27.0"`. `StackConfig` loses `accept_pool` and has `linger`; `tests/vnet_regressions.rs` no longer sets
  `accept_pool`. The close of a listener and of a stream, the RST of `abort`, `ConnectionReset` for a RST of the
  peer and no RST for a burst of SYNs close the gaps of PodUtils/podnet#457. The tests of T-176 pin the new
  behaviour (`crates/podnet-vnet/tests/guest_wire.rs`). T-249 drops its workarounds for these gaps.
- `pod-netstack` v0.28.0 (PodUtils/podcrates#86): each `pod-*` dependency names `tag = "v0.28.0"`. The UDP
  asks 3 to 5 of PodUtils/podnet#464 are closed: a backlog for each socket, the counters `udp_rx_dropped`,
  `udp_tx_dropped` and `udp_oversize`, `max_datagram` (a larger send is `InvalidInput`), and the feature
  `ipv4-fragmentation` (off). The test `guest_wire_udp_over_the_mtu_is_refused_without_fragmentation` pins the
  refusal. `cargo test --workspace --no-fail-fast`: 0 failed.
- `pod-hold` v0.29.0 (PodUtils/podcrates#87; hand-off `adoption/podnet/pod-hold-syn.patch`, trial at a7350b2): each
  `pod-*` dependency names `tag = "v0.29.0"`, and `podnet-netns` takes the feature `syn-hold`. `netns/backend.rs`
  takes `pod_hold::syn_hold::SynHold`: the hold of each SYN with its dial, the release or the RST, the listeners of
  the ports and the late dial. podnet keeps the dial (`TcpRoute` as `Dial`), the flow log, the copy of a flow, the
  DNS, the UDP flows and the device of the wire. The crate fixes four defects of the router of T-249 (a SYN past
  the budget, the late dial to the gateway, a second accept of a tuple, stale cites). `cargo test --workspace
  --no-fail-fast`: 0 failed; the netns tests as root: 0 failed.
- `pod-relay` v0.30.0 (PodUtils/podcrates#89; hand-off `adoption/podnet/pod-relay.patch`, trial at 98d8a71): each `pod-*`
  dependency names `tag = "v0.30.0"`. The relay client is glue over `pod-relay` (T-049).
- v0.31.0 (PodUtils/podcrates#90): each `pod-*` dependency names `tag = "v0.31.0"`; no hand-off for podnet.
  `pod_identity::DisplayFingerprint` (ask A1) is in the crate. `cargo test --workspace --no-fail-fast`: 0 failed.
- v0.32.0 (PodUtils/podcrates#93): each `pod-*` dependency names `tag = "v0.32.0"`; no hand-off. The operator of
  `pod-relay` flushes each frame (podnet keeps its writer of T-315). The crate `pod-link` exists
  (podcrates T-300, T-311): T-262 can take it. `cargo test --workspace --no-fail-fast`: 0 failed.
- v0.33.0 (PodUtils/podcrates#103): each `pod-*` dependency names `tag = "v0.33.0"`; no hand-off. `pod-session`
  gives a typed cause in `End::GaveUp` (A12): `src/session.rs` maps `UnknownSession` and `WindowEnded` to
  `ResumeExpired`. `pod-turn` and `pod-devtunnel` exist (T-321, T-320). `cargo test --workspace`: 0 failed (one flaky
  stand-in test fixed, T-316).
- v0.34.0 (PodUtils/podcrates#104): each `pod-*` dependency names `tag = "v0.34.0"`; no hand-off. `pod-link` has
  the TCP carrier with TLS 1.3 and a pinned Ed25519 key (feature `tcp`, for T-268). A read after the stop of a
  `pod-netstack` stack returns `ConnectionReset`. `cargo test --workspace --no-fail-fast`: 0 failed.
- v0.35.0 (PodUtils/podcrates#127): each `pod-*` dependency names `tag = "v0.35.0"` and `version = "0.35.0"`; no
  hand-off for podnet at the tag. `pod-warp` holds the rest of the WARP chain; the adoption trial of `src/warp/` passes at
  9c1fc4b (`adoption/podnet/pod-warp.patch` on `main` of podcrates after the tag); the COORD issue that holds
  `src/warp/` to fixes follows the trial of podssh. Breaking in `pod-netstack` (a socket that the TCP timeout ends reads
  `TimedOut`) and `pod-warp` (`DataPathCheck::servers`): no podnet code uses them yet. `cargo test --workspace
  --no-fail-fast`: 81 test binaries, 0 failed. `cargo deny --locked check bans sources`: ok.
- v0.36.0 (PodUtils/podcrates#129): each `pod-*` dependency names `tag = "v0.36.0"` (and `version = "0.36.0"` where a
  version is named; `pod-todo` names the tag only); no hand-off for podnet. The trial of `pod-warp` in `src/warp/` passes
  at podcrates (adoption run 43347062071); the COORD issue of the freeze follows the next tag. `cargo test --workspace
  --no-fail-fast`: 82 test binaries, 0 failed. `cargo deny --locked check bans sources`: ok.
- v0.37.0 (PodUtils/podcrates#134, #135): each `pod-*` dependency names `tag = "v0.37.0"`. The hand-off
  `adoption/podnet/pod-warp.patch` (the entry 412 of podcrates, at 9c1fc4b) is applied with `git apply -3`: `pod-warp` holds the
  WARP chain, and `src/warp/` is glue over it (34 files: 1163 lines in, 12437 lines out). `export.rs`, `up.rs`, `pod.rs`,
  `ssh_config.rs`, the record of the instance, the bind of the listeners, nc's search and relay leg and the lint of the
  rules stay in podnet. `src/warp/` holds fixes only from now on; each fix goes to podcrates as a COORD issue. The checks of
  `adoption/podnet/pod-warp.run` pass: the record, the size and the layers, `cargo deny` bans and sources, `pod-warp` in the
  tree, no moved file or item left, clippy of the workspace, `sh scripts/agent/check.sh` (each test), the fuzz targets.
- v0.38.0 (PodUtils/podcrates#136): each `pod-*` dependency names `tag = "v0.38.0"`. Breaking: `RuleMatch::matches` of
  `pod-warp` takes the port of the target (`tests/rule_words.rs` gives `None`). New in the tag: the port rules of
  `pod-warp` (T-328), `pod_dns::txt` (T-281), the record layer of `pod-e2e` (T-298, T-300, T-301), `pod_ws::serve` and
  the carriers `ws` and `h2` of `pod-link` (T-269, T-270). `cargo deny --locked check bans sources`: ok.
- v0.39.0 (PodUtils/podcrates#137): each `pod-*` dependency names `tag = "v0.39.0"`; nothing breaks. The datagrams of
  `WarpNet` send an IPv4-mapped destination as IPv4 and give the answers in the mapped form (PodUtils/podnet#524): podnet
  drops src/warp/one_family.rs at `ca53a72` (T-332) and takes `pod_core::Dial::datagrams` of the session again for QUIC and nc; the
  fix of podnet's own `nc_session` (the session lives until the pipe ends) stays. The `ws` carrier takes the subprotocol
  `podnet/1` (`ws::Ws::with_protocol`). `pod_link::arq` is the ARQ of T-277. `cargo deny --locked check bans sources`: ok.
- Next: `pod-relay` (`src/relay_reverse.rs`, the bridge of `src/ws.rs`).
- The owner, 2026-11-05 (`docs/decisions.md`, Services crate): the lists of services of other
  parties, their measurements and the watch workflows move to `pod-services`. podnet takes it
  like each other crate.

2026-11-13: the line "Next: `pod-relay`" above is old: `pod-relay` is taken (v0.30.0). The next take is `pod-warp` (`adoption/podnet/pod-warp.patch` on `main` of podcrates; the trial passes). podnet applies it at the tag that follows the COORD issue of the freeze (COORD-063, GitHub #520). `pod-cloudflare` follows. Taken at v0.36.0: `pod-todo`, `pod-core`, `pod-tls`, `pod-proxy`, `pod-dns`, `pod-services`, `pod-ws`, `pod-quic`, `pod-hold`, `pod-wg`, `pod-netstack`, `pod-relay`, `pod-identity`, `pod-session`, `pod-link`, `pod-turn` and `pod-devtunnel` (`Cargo.toml`).

# T-161: relay node: renew the pair before it expires (GitHub #358)

**Source:** GitHub #358 (https://github.com/PodUtils/podnet/issues/358); PodUtils/podcrates#35 (cfb), ask A-N-12.
**Category:** feature
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** blocked

## Problem

A pair lasts at most 72 h (`src/cli/cmd/relay.rs:29`). The node stops with "the pair NAME expired;
make a new pair" (`relay_reverse.rs:290-298`, the matching branch of the same file). A node with an explicit name cannot follow
to a next pair (`cmd/relay.rs:118-121`). A node that has no name rolls to the next stored pair, which
an operator must mint and distribute (`relay_reverse.rs:403-440`). Unattended hosts (T-001, GitHub #89)
stop after 72 h. Proposed: `POST /v1/pair/NAME/renew` with the node token; the answer has a new
`expires`; node token only (the connect token must not extend its own life).

## Premise

- Read: no renew call exists in `relay_reverse.rs`. `Relay::post_json` sends any path with a token
  (`ws_relay.rs:572`).
- Measured on 2026-11-09: the published contract `https://tcp.ssh.relay.ajam.dev/llms-full.txt` says for
  close 1001 "The 72h pair TTL elapsed. Create a new pair with `POST /v1/pair`". It lists no renew call.
  So the client change needs a relay-server change first. The server is not in this repository.
- Self-minting needs no server change for the mint, but the connector must learn the new name and
  token, so it does not remove the operator.
- `src/relay_reverse.rs` goes to `pod-relay` (T-049, not released in podcrates). T-087 covers session
  resume, not the pair lease.

## Approach

1. Ask podcrates: a COORD issue `(to podcrates)` for `pod-relay` (renew call, node thresholds at half
   the TTL and at the first warning band). The owner of the relay adds the endpoint (see Decision).
2. After `pod-relay` exists and the endpoint exists: take it (T-049, T-137). Add the podnet test.
3. Keep the clean stop when the renewal fails.
4. PodUtils/podcrates#35 (cfb), ask A-N-12, routed by PodUtils/podcrates#40: the renewal needs no
   operator act, so a node under a system unit (T-220) runs past 72 h.

## Decision

The lead, 2026-11-09: add a renew call to the relay server through the session of `PodUtils/podcrates` (`pod-relay`), not in podnet. The call is `POST /v1/pair/NAME/renew`, with the node token only. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test --test relay_reverse -- a_node_renews_its_pair_before_the_expiry a_lapsed_renewal_stops_the_node
```

## Blocker

The owner of the relay server and the session of `PodUtils/podcrates` (the crate `pod-relay`). No relay of the contract serves a renew call (`tcp.ssh.relay.ajam.dev` publishes none). The crate has a call that differs from the Decision (see the Correction), and its node does not renew. Send a COORD issue (to podcrates) to ask whether the node renews at half the TTL with the call of the Decision or with the call of the crate. The entry is unblocked when a relay serves a renew call and the node of `pod-relay` renews. The ask is GitHub #482 (COORD-051).

## Start condition

A relay serves a renew call, and the node of `pod-relay` renews a pair (T-049 is done).

## Correction

2026-11-12, the take of `pod-relay` v0.30.0 (83e0c93). The crate has `pod_relay::pair::renew::renew`: `POST /v1/renew/NAME` with the stop token (not the node token), and the answer `{expires}`. A 404 or 405 is `Renewed::NotServed`. It is the call of the relay role of podssh, beside the contract. The contract and `tcp.ssh.relay.ajam.dev` do not serve it. The path and the token differ from the Decision (`POST /v1/pair/NAME/renew`, node token). The node of the crate renews nothing: `NodeConfig::repair` is asked only after the pair has expired (a 401 or 403 after the expiry), when a renew with the stop token would be refused for a pair that ended. PodUtils/podcrates#89 does not list this entry. Not measured: any relay that serves the call.

2026-11-12: podcrates answered #482 (PodUtils/podcrates#90): the relay contract has no renew path. `pair::renew::renew` (`POST /v1/renew/NAME`, the stop token) is the call of the relay role of podssh; a relay of the contract answers it with 404 or 405. podcrates T-121 plans a setting of `reverse::run` that renews with that call where the relay serves it. The node-token call of the Decision waits for the owner of the public relay to add it to the contract.


# T-162: relay: a host that stalls is tried last on the next connect (GitHub #359)

**Source:** GitHub #359 (https://github.com/PodUtils/podnet/issues/359)
**Category:** feature
**Milestone:** M3
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

The relay host order is fixed at start. A host that accepts TCP and stalls the upgrade stays first, and
each reconnect waits the 2 s stagger before the second host starts. The pool is read again only after a
mint, and only the cache file changes, not the running list.

## Premise

- Read: `Relay::new` builds the list once (`ws_relay.rs:231-271`); `connect` never changes it
  (`:428-497`); `refresh_pool` writes the cache only (`:389-421`) and does not run for a listed list
  (`:363`). `alternates_of` takes pool order (`relay_pool.rs:108-121`); `STALE_AFTER` is 7 days.
- Correction of the issue: a stalled host costs the stagger (2 s), not the full timeout. The race starts
  the next host after the stagger and the first success wins (`crates/podnet-core/src/race.rs:1-9`). The loopback test of
  the issue must assert a connect time near 2 s before and near 0 s after, not a full timeout.
- Precedent: the front door orders the address that worked first (src/warp/mod.rs:911 at `9c1fc4b`, issue 124).
- `ws_relay.rs` and `relay_pool.rs` go to `pod-relay` (T-049): a new feature waits for the crate.
  T-056 and T-058 plan the dialer combinators; T-097 plans router health.

## Approach

1. Ask podcrates for the health memory in `pod-relay` (COORD `(to podcrates)`): count failures and
   losses of a race for each host; demote after 2; restore on success; the order stays stable.
2. After the take, adopt the pool at an exhausted connect.
3. Add the loopback test in podnet's tests for the taken crate.

## Prove

```sh
cargo test -p podnet --lib -- a_stalled_relay_host_is_tried_last_on_the_next_connect a_demoted_relay_host_recovers_on_success
```

## Start condition

T-049 is done.

## Correction

2026-11-11, the comment on GitHub #359 from the donor review (airhand.md block 3; MIT;
https://github.com/PodUtils/podnet/issues/359#issuecomment-6789136771). It adds a rank among the hosts that are not dead, to the demotion by
failure counter of this entry: the host order uses the least of the last K connect times, a switch of the head
needs a margin of `max(5 ms, 10%)` that holds for a dwell (a count of consecutive connects, not seconds), and a
head that is lost is held off the head for a time. Only the connect that wins, or that ends before the winner,
gives a time (a cancelled attempt gives none), so an alternate that never leads is never sampled and the rule
cannot promote it. A lower bound for a cancelled attempt is the time of the winner minus the stagger offset of
the loser. Whether to sample a host that is not the head, or to store the bound, is open. A host that the
operator named first moves at most one place. The values `K`, the dwell and the hold have no number yet: they
need a measurement on two hosts of one deployment. Add the text of the comment to the COORD ask (to
podcrates) of the Approach, item 1, when that ask is written. The donor values are starting values only.

2026-11-12, the take of `pod-relay` v0.30.0 (83e0c93). The health memory is in the crate (`failures.rs`; PodUtils/podcrates#89, ask 2 of GitHub #413, podcrates T-122): a host that failed with an error that another host can repair, or that said nothing while a host after it opened, is tried last for 10 minutes. A host that opens is tried first for the same time. One failure moves a host, not two, so the names of the Prove (`..._after_two_losses`) are changed: the tests are `a_stalled_relay_host_is_tried_last_on_the_next_connect` and `a_demoted_relay_host_recovers_on_success` (`src/ws_relay/relay_tests.rs`). The rank among the hosts that are not dead (the comment of the Correction above) is `rank.rs` of the crate (a rank of measured connect times, kept for one day). podnet runs no rank of its own. The values K, the dwell and the hold of the donor comment are not podnet's.

## Done

2026-11-12, commit 7e462ec over the take 83e0c93. `cargo test -p podnet --lib -- a_stalled_relay_host_is_tried_last_on_the_next_connect a_demoted_relay_host_recovers_on_success`: 2 passed. Two loopback relays (`127.0.0.1` stalls after the TCP accept, `127.0.0.2` carries): the first connect takes the stagger (2 s), and the second connect takes under 1 s. When the roles change, the third connect pays the stagger and the fourth goes to the restored host first. With the record of failures off (a cache with no directory), both tests fail on a timing assertion (the second connect takes the stagger again). `sh scripts/agent/check.sh`: exit 0. Not proven: a stall on the live relay.

# T-163: relay: the session budget comes from the pool document (GitHub #360)

**Source:** GitHub #360 (https://github.com/PodUtils/podnet/issues/360)
**Category:** feature
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** partial

## Problem

`SESSION_BYTES` (60 MiB) and `SESSION_AGE` (11.5 h) are constants for the default deployment
(`ws_relay.rs:49-52`). A relay that the user selects (`PODNET_RELAY`) has other limits. A smaller cap
cuts streams with no replacement. A larger cap wastes sessions.

## Premise

- Read: constants at `ws_relay.rs:49-52`. `RelayBudget::meter` applies them (`:66`) and `meter_with`
  takes values (`:70`). Other users: src/warp/session.rs:413 at `9c1fc4b` (through `meter`), `src/edge_h2.rs:409`
  and `:644` (direct, outside `RelayBudget`). `parse_pool` reads only `pool[].host`
  (`relay_pool.rs:83-100`). `CachedPool` has no limits (`:121-129`).
- Measured on 2026-11-09: `https://tcp.ssh.relay.ajam.dev/relays.json` already has a `limits` object
  with `max_session_ms` 43200000, `max_session_bytes` 67108864, `idle_timeout_ms` 180000 and
  `connect_timeout_ms` 10000. The key names differ from the issue (`session_bytes`). No relay-server
  change is needed for the default deployment.
- `docs/decisions.md:80` (2026-11-05): the limits of `/relays.json` are data of `pod-services`; the
  relay client goes to `pod-relay`.
- `src/edge_h2.rs` is frozen (pod-core): fixes only.
- Not measured: the limits of a third-party relay.

## Approach

1. Ask podcrates (COORD `(to podcrates)`): `pod-relay` parses `limits` with the published names, stores
   them with the cached pool, and passes them to `meter_with`. The floor: the constants. Accept values
   in a clamped range. Keep the 60/64 and 11.5/12 margins.
2. For `edge_h2.rs:408` and `:749`: send a COORD issue `(to podcrates)` with the minimal plumbing change
   (pod-core owns that code). Do it in podnet only as a fix when podcrates agrees.
3. After the take, add the tests in podnet.

## Prove

```sh
cargo test -p podnet --lib -- a_pool_with_limits_sets_the_budget a_pool_with_no_limits_keeps_the_constants the_age_of_the_budget_follows_the_limits_of_the_relay
```

## Start condition

T-049 is done; podcrates answers the COORD issue for `edge_h2.rs`.

## Correction

2026-11-12, the take of `pod-relay` v0.30.0 (83e0c93). The crate parses the `limits` of `/relays.json` with the published names, stores them with the cached pool, and gives them to the budget (`RelayBudget::meter_for`; PodUtils/podcrates#89, ask 3 of GitHub #413, podcrates T-123). `ws_relay::meter` uses it. The WARP leg through the relay (`src/warp/session.rs`) has its budget under the limits of the relay: 4 MiB under `max_session_bytes` and 30 min under `max_session_ms`, never over 60 MiB and 11.5 h. Proven by `a_pool_with_limits_sets_the_budget`, `a_pool_with_no_limits_keeps_the_constants` and `the_age_of_the_budget_follows_the_limits_of_the_relay` (`src/ws_relay/relay_tests.rs`, 3 passed, commit 7e462ec). With the constants in place of `meter_for`, the first and the third fail. What is left: the HTTP/2 edge leg. `src/edge_h2.rs:409` and `src/edge_h2.rs:644` (frozen: `pod-cloudflare`) count `ws_relay::SESSION_BYTES` and `SESSION_AGE` directly, outside `RelayBudget`. The edge leg of a tunnel through a relay with smaller caps still uses the constants. The change is `RelayBudget::meter_for` in place of the constants in `serve_h2` and `Budget::relay`. Send a COORD issue (to podcrates) with this change (Approach, item 2). Do it in podnet only as a fix when podcrates agrees. The entry is done when the edge leg uses the limits, or when podnet takes `pod-cloudflare` (T-052).

# T-175: DoH: keep one connection for each resolver server (an ask of pod-dns and pod-proxy) (GitHub #370)

**Source:** GitHub #370 (https://github.com/PodUtils/podnet/issues/370)
**Category:** feature
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Each DoH query opens a TCP and TLS connection through the egress and sends one request: `get.rs`
(`get_now`), `h2.rs:67` ("of its own"), `ask.rs:92`. The HTTP/1.1 client sends `Connection: close`
(`pod-proxy` `http.rs:116`). A lookup costs a CONNECT and a TLS handshake each time. The router health
checks (T-097) resolve names on this chain.

## Premise

- Read in the cargo checkout of podcrates; podnet only consumes it (`src/resolve.rs:111-135`,
  `names: Names::Last` at `:127`). The fix belongs to `pod-dns` and `pod-proxy`.
- A second part is in podnet: no startup note when the egress takes names only. Each resolver then pays
  one failed literal try.
- Not measured: the 3x cost of the issue.

## Approach

1. COORD issue `(to podcrates)`: a kept connection for each `(server, dial)` in `Resolver`, idle close
   after about 5 s, one reconnect, the deadline for one exchange; `keep-alive` in `exchange`.
2. Then take the tag (T-137). Add the test of the issue: N queries, one connection at a loopback DoH server.
3. In podnet: one startup line when `names_inside()` is true and literals cannot pass.

## Prove

```sh
cargo test -p podnet resolve:: -- doh_queries_share_one_connection
```

## Correction

2026-11-09: the blocker cleared. podcrates v0.8.0 has `pod_dns::Kept` and `Way::kept` (podcrates T-125); podnet takes v0.8.0 in 0df677d.

## Done

2026-11-09, commit b61a90e. The resolver of pod-dns v0.8.0 keeps one connection for each server, so the wiring needs no code. egress writes one info line when the egress resolves names. Test: resolve::kept_tests::doh_queries_share_one_connection. check.sh: exit 0. CI: at the merge.

# T-200: The reverse road of the relay uses the alternate hosts (GitHub #397)

**Source:** GitHub #397 (https://github.com/PodUtils/podnet/issues/397)
**Category:** feature
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** blocked

## Problem

`Relay::connect` races all hosts with a 2 s stagger. `Relay::websocket` and `Relay::post_json` use `hosts[0]` only.
`relay node` builds `Relay::single` (no pool). `relay pair` and `relay stop` build `Relay::new` (the alternates load and
are never used). A node whose primary host stops answering retries that host for ever (1 s to 30 s backoff).
`relay pair --json` prints the primary only.

## Premise

- `src/ws_relay.rs` lines 561-568 at `98d8a71` (`websocket`, `hosts[0]`), lines 572-592 at `98d8a71` (`post_json`,
  `self.url()`), lines 326-328 at `98d8a71` (`url()`), lines 428-497 at `98d8a71` (`connect`), lines 254-264 at
  `98d8a71` (`new` adds up to 3 alternates), lines 275-284 at `98d8a71` (`single`).
- `src/relay_reverse.rs` line 341 at `98d8a71` (`run_node` builds `Relay::single`), lines 215-257 at `98d8a71`
  (reconnect loop).
- `src/cli/cmd/relay.rs:93`, `:144` (validation only), `:275`.
- Not measured: whether the relay accepts a node socket for a pair on every host of the deployment.
  `docs/protocol.md:209-215` measures the forward road only.

## Approach

1. Ask in a COORD issue `(to podcrates)`: failover on the reverse road in `pod-relay`, with the measurement of item 2.
2. Measure first (live step): a pair minted on host A, a node socket on host B of the same pool.
3. After the take (T-049): one host loop for `connect`, `websocket` and `post_json`; `run_node` uses `Relay::new`; the
   pair output lists the hosts (additive JSON key). Keep the `Relay::single` tests.
4. If the relay refuses the alternate, keep one host and print a clear error.

## Decision

The lead, 2026-11-09: the failover of the reverse road is done in `pod-relay` of `PodUtils/podcrates`, not in podnet. podnet takes it with T-049. The owner can rule otherwise (`docs/decisions.md`).

## Prove

`cargo test -p podnet --test relay_reverse node_moves_to_alternate_host` (new test: two fake relays, stop A).
`cargo test -p podnet --test relay_reverse` (old tests still pass).

## Blocker

The relay contract and the session of `PodUtils/podcrates` (the crate `pod-relay`). The relay makes a pair on the control host only (PodUtils/podcrates#89, 2026-11-12), and the reverse road of the crate (`reverse::run`, `operator::run`, `pair::create`) uses `Relay::primary()`, the first host. A relay of the contract does not serve the node socket of that pair on another host, so the measure of the Approach, item 2, has no host to test. Send a COORD issue (to podcrates). The entry is unblocked when podcrates or the owner of the relay says that an alternate host serves the node socket of a pair. The ask is GitHub #482 (COORD-051).

## Correction

2026-11-12, the take of `pod-relay` v0.30.0 (83e0c93). The forward road of the crate (`open`, `Request::relays`) tries all the hosts of a list with the stagger, and `Relay::connect` of podnet uses it. The reverse road has no host loop. `Relay::single` for the node stays, and `relay pair --json` still prints the first host only. Not measured: a node socket on host B for a pair of host A.

2026-11-12: podcrates answered #482 (PodUtils/podcrates#90): blocked: the relay contract makes a pair on the control host only and moves none (podcrates T-124).


# T-224: podnet-netns calls `pod_proxy::udp` for the SOCKS5 datagram path

**Source:** T-137 (the take of v0.9.0 of `PodUtils/podcrates`); PodUtils/podcrates#42 and
PodUtils/podcrates#43 (the module `udp` of `pod-proxy`, entry T-083 of podcrates).
**Category:** chore
**Milestone:** M3
**Priority:** P1
**Effort:** S
**Status:** done

## Problem

The SOCKS5 datagram path of `crates/podnet-netns/src/upstream.rs` (UDP ASSOCIATE, the command
0x83, the frames and the datagram header) is frozen: `pod-proxy` v0.9.0 holds it as
`pod_proxy::udp`. The freeze ends when podnet calls the crate. Until then podnet keeps a second
copy of the same code.

## Premise

- `PodUtils/podcrates:crates/pod-proxy/SOURCES` at v0.9.0 (b7c3e6a) maps the lines 584-775 of
  `crates/podnet-netns/src/upstream.rs` at `29ba18a` to `PodUtils/podcrates:crates/pod-proxy/src/socks5/udp.rs`.
- `pod_proxy::udp` has `open_in_stream`, `associate`, `write_frame`, `read_frame`, `wrap`,
  `wrap_name` and the constants `CMD_UDP_IN_STREAM` and `REPLY_COMMAND_NOT_SUPPORTED`. A reply
  0x07 to 0x83 is `DialError::UdpRefused`. The functions take a `pod_proxy::Proxy` (the
  credentials and the name in a message) and a stream that the caller dialed.
- `wrap_name` refuses a name over 255 bytes (`DialError::InvalidTarget`); podnet's
  `wrap_udp_name` cut the name. In a pod, `crate::dgram::fits_relay` refuses such a name with
  EINVAL before the wrap (T-066), so a pod sees no change.
- `pod-proxy` sends the user and the password of RFC 1929 with no check of their length;
  podnet checks them first (`SOCKS5_CREDENTIAL_MAX`).
- The callers: `crates/podnet-netns/src/udp.rs`, `crates/podnet-netns/src/frontdoor.rs`,
  `crates/podnet-netns/src/inbox.rs` (a test), `src/warp/nc.rs` and `src/warp/proxy.rs`.
- Not measured: a SOCKS5 server of another party with the command 0x83.

## Approach

1. Add `pod-proxy` to `crates/podnet-netns/Cargo.toml` with the git tag of the root.
2. Remove the copy from `crates/podnet-netns/src/upstream.rs`. Keep two functions of glue,
   `socks5_udp_in_stream` and `socks5_udp_associate`: they check the credentials, make the
   `pod_proxy::Proxy` of podnet's `Proxy`, and call `open_in_stream` and `associate`.
3. The callers call `pod_proxy::udp` for the constants, the frames and the headers.

## Prove

```sh
cargo test -p podnet-netns
cargo test -p podnet --test pod_seccomp_dgram
cargo test --workspace
sh scripts/agent/check.sh
```

## Done

2026-11-09, commit 127c095. `crates/podnet-netns/src/upstream.rs` has no copy of the datagram
path: 111 lines out, 52 in (the glue `shared_proxy` and its percent escape). The callers in
`crates/podnet-netns/src/udp.rs`, `crates/podnet-netns/src/frontdoor.rs`, `src/warp/nc.rs` and
`src/warp/proxy.rs` call `pod_proxy::udp`. `UdpInStreamRefused` is gone: a reply 0x07 to 0x83 is
`pod_proxy::DialError::UdpRefused` in the `anyhow` error. A name over 255 bytes: in a pod,
`crate::dgram::fits_relay` refuses it with EINVAL before the wrap, as before; `podnet warp nc`
refuses it with `invalid destination: ... is longer than the 255 bytes of SOCKS5` before the
association (before, the name was cut). Other changes: `pod-proxy` refuses a reply with a version
other than 5, and the error words are those of `pod-proxy`. New tests:
`upstream::tests::udp_in_the_stream_goes_through_pod_proxy`,
`upstream::tests::udp_associate_refuses_a_long_user`,
`upstream::tests::a_udp_header_refuses_a_name_over_255_bytes`. `cargo test -p podnet-netns`: 68
passed; `pod_seccomp_dgram`: 17 passed. `sh scripts/agent/check.sh`: exit 0 (1107 passed, 0
failed); `unshare -n -- cargo test --offline --workspace --no-fail-fast`: exit 0;
`sh scripts/restricted-host.sh --no-fail-fast`: exit 0. CI: at the merge.

# T-232: The hand-off of the generic hold of the pod backends to `pod-hold`

**Source:** PodUtils/podvm#228 (COORD-022 of podvm, 2026-11-10), ask 2; PodUtils/podnet#414
(COORD-007, "The view on pod-hold"); PodUtils/podcrates T-109.
**Category:** chore
**Milestone:** M3
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

podnet#414 put two conditions on the move of the generic hold to the shared crate `pod-hold`: the time
(after T-040 and the M5 fixes of the seccomp backend), and the cut (the generic core only). podvm asks if
the time has come.

## Premise

- T-040 and T-142 to T-158 are done at `31d040f`, except T-153 (parked research, M9).
- The generic core: the seccomp user notification loop, the reading of the child memory, the descriptor
  table and the reply, and the netns hold of a `connect`. The pod CLI, the rules of the upstreams and the
  routes stay in `podnet-netns`.

## Approach

1. Tell podcrates that the time has come (a COORD issue (to podcrates)), with the cut of podnet#414.
2. When podcrates publishes `pod-hold` with `adoption/podnet/pod-hold.patch`, take it (T-137).

## Decision

2026-11-10, a call of the podnet session: the time of podnet#414 has come. From the take, the files of the
generic core take fixes only. The pod work that is left (T-174, T-229 to T-231) uses the CLI, the rules and
the doctor, which stay in `podnet-netns`.

## Prove

```sh
sh scripts/agent/podcrates.sh   # the pod-hold block of crates/pod-hold/SOURCES lists the files
```

## Done

2026-11-11, commit b759937. podnet took `pod-hold` of podcrates v0.15.0 (T-137): the generic core of the
hold (the cut of podnet#414) is glue over `pod-hold`. The adoption run passed each line except the
static build with zig (no zig on the host). The questions of podcrates T-169 (the dial state of a held
`connect`) and T-170 (the netns hold of a SYN) are answered in a COORD issue (to podcrates).
