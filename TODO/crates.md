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
**Status:** open

## Problem

Several facts of the root crate assume one library. A second library crate cannot build, or its
checks do not run, until they change.

## Premise

- The write-safe print macros and `update_atomic!` are defined in `src/lib.rs`.
- `build.rs` sets the build commit, target and date for the root crate only. `src/fetch.rs`,
  `src/report.rs`, `src/config.rs`, `src/quicktunnel.rs` and `src/tunnel.rs` read them.
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

# T-047: Extract the crate `podnet-core`

**Source:** `docs/research/podnet/architecture.md`, sections 3.2, 4.1 and 4.6, step E.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** L
**Status:** open

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
`src/trust.rs` is glue over `pod-tls`. `src/egress.rs`, `PodUtils/podnet:src/proxy_auth.rs` and `src/fetch.rs` move to
`pod-proxy`, `src/resolve.rs` to `pod-dns`, and the WebSocket bridge of `src/ws.rs` to `pod-ws`
(T-137). Extract `podnet-core` from what stays in podnet after these takes. Recount the module list
when the entry starts. Step 4 names the MSRV job: it stays, and its plant step is gone.

# T-048: Extract the crate `podnet-vnet`, with the C sources of the shims

**Source:** `docs/research/podnet/architecture.md`, sections 4.1 and 4.2 (the row cargo package).
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The userspace network (`src/vnet/`, about 8,400 lines) is in the root crate. Its shims are C
files under `shim/`, which `src/vnet/shim.rs` includes with `include_str!`.

## Premise

- `include_str!` must name a file inside the crate, or `cargo package` fails.
- `.github/workflows/ci.yml` checks the C file of the shim in the package list of the root
  crate.
- `podnet-vnet` depends on the core and on `podnet-netns`
  (`docs/research/podnet/architecture.md`, section 4.1).

## Approach

1. Make crates/podnet-vnet with `src/vnet/`. Move the files of `shim/` into the crate.
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

2026-11-05, `src/vnet/` uses smoltcp 0.12 (`Cargo.toml`). The pin has no reason in the toolchain now
(Rust 1.99). The userspace stack of `pod-netstack` (wave 2 of PodUtils/podcrates) uses smoltcp 0.14.
Before the extraction, read whether `src/vnet/` takes `pod-netstack` (T-137); if it does, do not extract the
stack code into `podnet-vnet`.

# T-049: The relay client of podnet becomes the shared crate `pod-relay`

**Source:** `docs/research/podnet/architecture.md`, section 4.1.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

The relay client and the reverse road (about 2,400 lines) are in the root crate.

## Premise

- The crate takes the client of `src/ws_relay.rs` (`Relay`, `Host`, the mint, the token cache,
  `RelayBudget`, `websocket_at`), `src/relay_pool.rs` and `src/relay_reverse.rs`.
- The HTTP/2 leg through the relay stays for the Cloudflare crate (T-052).
- `tests/relay_reverse.rs` calls the library of the reverse road.

## Approach

1. Take `pod-relay` of PodUtils/podcrates (a git tag) in place of the relay modules. Remove
   `src/ws_relay.rs`, `src/relay_pool.rs` and `src/relay_reverse.rs` from podnet when the crate has
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

# T-050: Move the WARP adapter to the shared crate `pod-warp` in `PodUtils/podcrates`

**Source:** `docs/research/podnet/architecture.md`, section 4.1.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-051: Extract the crate `podnet-serve`, with the library-only modules

**Source:** `docs/research/podnet/architecture.md`, sections 3.2 and 4.1.
**Category:** chore
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The services (about 9,300 lines) are in the root crate. About 5,600 lines of them have no caller
in the binary.

## Premise

- The crate takes `src/share.rs`, `src/gate.rs`, `src/service.rs`, `src/ingress.rs`,
  `src/origin/`, `src/feature/`, `src/transport/`, `src/relay.rs` and the client of the
  WebSocket forward.
- `src/feature/`, `src/origin/`, `src/transport/` and `src/relay.rs` are a capability of the
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
   code of podnet (`src/trust.rs`, `src/resolve.rs`, `src/egress.rs`) with `pod-tls` (done), `pod-dns`
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
  `src/trust.rs` is the glue (one context of the process on ring, the edge roots of Cloudflare as
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
  is a read of `pod-services` through `src/services.rs`; `report::log_filter` gives `-v` the debug
  lines of the pod crates. Each `pod-*` dependency names `tag = "v0.4.0"`. The COORD issue is
  PodUtils/podnet#320.
- v0.5.0 (dfd8bd8): each `pod-*` dependency names `tag = "v0.5.0"`; no API change for podnet.
  The COORD issue is PodUtils/podnet#321.
- v0.6.0 (872a276): `rules()` sets `http_proxy_vars`; `live.yml` checks the DoH server again. The
  COORD issue is PodUtils/podnet#333. CI run 42263403147 at 872a276 and live run 42265938003 at 4094fee:
  success.
- Next: `pod-ws` (`src/ws.rs`).
- The owner, 2026-11-05 (`docs/decisions.md`, Services crate): the lists of services of other
  parties, their measurements and the watch workflows move to `pod-services`. podnet takes it
  like each other crate.

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

The session of `PodUtils/podcrates` (the crate `pod-relay`) and the owner of the relay server. The relay publishes no renew call, and the relay client is code of `pod-relay`. Send a COORD issue (to podcrates). The entry is unblocked when the relay has the call and podnet takes the client with T-049.

## Start condition

The relay publishes a renew call, and T-049 is done.

# T-162: relay: a host that stalls is tried last on the next connect (GitHub #359)

**Source:** GitHub #359 (https://github.com/PodUtils/podnet/issues/359)
**Category:** feature
**Milestone:** M3
**Priority:** P3
**Effort:** M
**Status:** blocked

## Problem

The relay host order is fixed at start. A host that accepts TCP and stalls the upgrade stays first, and
each reconnect waits the 2 s stagger before the second host starts. The pool is read again only after a
mint, and only the cache file changes, not the running list.

## Premise

- Read: `Relay::new` builds the list once (`ws_relay.rs:231-271`); `connect` never changes it
  (`:428-497`); `refresh_pool` writes the cache only (`:389-421`) and does not run for a listed list
  (`:363`). `alternates_of` takes pool order (`relay_pool.rs:108-121`); `STALE_AFTER` is 7 days.
- Correction of the issue: a stalled host costs the stagger (2 s), not the full timeout. The race starts
  the next host after the stagger and the first success wins (`src/race.rs:1-9`). The loopback test of
  the issue must assert a connect time near 2 s before and near 0 s after, not a full timeout.
- Precedent: the front door orders the address that worked first (`src/warp/mod.rs:975`, issue 124).
- `ws_relay.rs` and `relay_pool.rs` go to `pod-relay` (T-049): a new feature waits for the crate.
  T-056 and T-058 plan the dialer combinators; T-097 plans router health.

## Approach

1. Ask podcrates for the health memory in `pod-relay` (COORD `(to podcrates)`): count failures and
   losses of a race for each host; demote after 2; restore on success; the order stays stable.
2. After the take, adopt the pool at an exhausted connect.
3. Add the loopback test in podnet's tests for the taken crate.

## Prove

```sh
cargo test -p podnet --lib -- a_stalled_relay_host_is_demoted_after_two_losses a_demoted_relay_host_recovers_on_success
```

## Blocker

The session of `PodUtils/podcrates` (the crate `pod-relay`): the health memory of the relay hosts is code of that crate. Send a COORD issue (to podcrates). The entry is unblocked when podnet takes `pod-relay` with T-049.

## Start condition

T-049 is done.

# T-163: relay: the session budget comes from the pool document (GitHub #360)

**Source:** GitHub #360 (https://github.com/PodUtils/podnet/issues/360)
**Category:** feature
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** blocked

## Problem

`SESSION_BYTES` (60 MiB) and `SESSION_AGE` (11.5 h) are constants for the default deployment
(`ws_relay.rs:49-52`). A relay that the user selects (`PODNET_RELAY`) has other limits. A smaller cap
cuts streams with no replacement. A larger cap wastes sessions.

## Premise

- Read: constants at `ws_relay.rs:49-52`. `RelayBudget::meter` applies them (`:66`) and `meter_with`
  takes values (`:70`). Other users: `src/warp/session.rs:413` (through `meter`), `src/edge_h2.rs:409`
  and `:644` (direct, outside `RelayBudget`). `parse_pool` reads only `pool[].host`
  (`relay_pool.rs:83-100`). `CachedPool` has no limits (`:121-129`).
- Measured on 2026-11-09: `https://tcp.ssh.relay.ajam.dev/relays.json` already has a `limits` object
  with `max_session_ms` 43200000, `max_session_bytes` 67108864, `idle_timeout_ms` 180000 and
  `connect_timeout_ms` 10000. The key names differ from the issue (`session_bytes`). No relay-server
  change is needed for the default deployment.
- `docs/decisions.md:75` (2026-11-05): the limits of `/relays.json` are data of `pod-services`; the
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
cargo test -p podnet --lib -- a_pool_with_limits_sets_the_budget a_pool_with_no_limits_keeps_the_constants
```

## Blocker

The session of `PodUtils/podcrates` (the crates `pod-relay`, `pod-core` and `pod-services`): the limits of `/relays.json` are data of `pod-services`, the budget is code of `pod-relay`, and `src/edge_h2.rs` is frozen code of `pod-core`. Send a COORD issue (to podcrates). The entry is unblocked when podcrates answers and podnet takes `pod-relay` with T-049.

## Start condition

T-049 is done; podcrates answers the COORD issue for `edge_h2.rs`.

# T-175: DoH: keep one connection for each resolver server (an ask of pod-dns and pod-proxy) (GitHub #370)

**Source:** GitHub #370 (https://github.com/PodUtils/podnet/issues/370)
**Category:** feature
**Milestone:** M3
**Priority:** P2
**Effort:** M
**Status:** blocked

## Problem

Each DoH query opens a TCP and TLS connection through the egress and sends one request: `get.rs`
(`get_now`), `h2.rs:67` ("of its own"), `ask.rs:92`. The HTTP/1.1 client sends `Connection: close`
(`pod-proxy` `http.rs:116`). A lookup costs a CONNECT and a TLS handshake each time. The router health
checks (T-097) resolve names on this chain.

## Premise

- Read in the cargo checkout of podcrates; podnet only consumes it (`src/resolve.rs:80-104`,
  `names: Names::Last` at `:96`). The fix belongs to `pod-dns` and `pod-proxy`.
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

## Blocker

The session of `PodUtils/podcrates` (the crates `pod-dns` and `pod-proxy`): the DoH connection is code of these crates. Send a COORD issue (to podcrates). The entry is unblocked when a release of these crates reuses one connection for each resolver server.

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

- `src/ws_relay.rs:561-568` (`websocket`, `hosts[0]`), `:572-592` (`post_json`, `self.url()`), `:326-328` (`url()`),
  `:428-497` (`connect`), `:254-264` (`new` adds up to 3 alternates), `:275-284` (`single`).
- `src/relay_reverse.rs:415` (`run_node` builds `Relay::single`), `:289-331` (reconnect loop).
- `src/cli/cmd/relay.rs:76`, `:123` (validation only), `:222`.
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

The session of `PodUtils/podcrates` (the crate `pod-relay`): the reverse road is code of that crate. Send a COORD issue (to podcrates). The entry is unblocked when podnet takes `pod-relay` with T-049.
