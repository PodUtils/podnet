# Build from source

The repository is a Cargo workspace:

| Crate | Contents |
| --- | --- |
| `podnet` (root) | The CLI and the library: the edge client (provisioning, discovery, QUIC, HTTP/2, Cap'n Proto and its RPC), the service layer, WARP. |
| `crates/podnet-core` | A library: the core layer of podnet (the process, the files, the state, the reports, the supervisor). The other crates use it. |
| `crates/podnet-serve` | A library: the services of podnet (the file server, the file transfer, the gate, the ingress rules, the origins, the client of the SSH relay). It uses `podnet-core`. |
| `crates/podnet-netns` | A library: run a program with all its traffic sent through a proxy (`podnet pod`). |
| `crates/podnet-vnet` | A library: the userspace virtual network (`podnet net`) and the C sources of its `LD_PRELOAD` shims. |
| `crates/podnet-capi` | The C ABI: `include/podnet.h`, a shared library and a static library over the link. It is not a default member (see The C library). |

## Build

```sh
cargo build --release
```

The binary is `target/release/podnet`. Rust
1.99 or newer is necessary. The build takes the `pod-*` crates from the
git repository `PodUtils/podcrates` (by tag), so it needs access to
GitHub.

A static binary has no run-time dependency on the C library of the host:

```sh
rustup target add x86_64-unknown-linux-musl
CC_x86_64_unknown_linux_musl=clang cargo build --release --target x86_64-unknown-linux-musl
```

The `LD_PRELOAD` shims need a C compiler at run time (`cc`, `gcc` or
`clang`). The other commands do not.

## Features

The default build has no optional feature. An optional carrier or service of
the link sits behind a Cargo feature of the `podnet` crate:

| Feature | Holds | Entry |
| --- | --- | --- |
| `tcp` | The TCP carrier with TLS 1.3 and a pinned Ed25519 key. | T-268 |
| `ws` | The WebSocket carrier with TLS 1.3 and a pinned Ed25519 key. It holds `tcp`. | T-269 |
| `h2` | The HTTP/2 carrier (one POST) with TLS 1.3 and a pinned Ed25519 key. It holds `tcp`. | T-270 |
| `quic` | The QUIC carrier (one stream of a QUIC connection) with TLS 1.3 and a pinned Ed25519 key. It holds `tcp`. | T-273 |
| `udp` | The UDP carrier (KCP) with TLS 1.3 and a pinned Ed25519 key. It holds `tcp`. | T-276 |
| `poll` | The poll carriers `poll`, `sse`, `chunked` and `httppoll` (requests of HTTP/1.1) with TLS 1.3 and a pinned Ed25519 key. It holds `tcp`. | T-271, T-272, T-280 |
| `dns` | The DNS simplex carrier (query names and TXT answers) with TLS 1.3 and a pinned Ed25519 key. It holds `tcp`. | T-278 |
| `s3` | The object-storage carrier. | T-279 |
| `icmp` | The ICMP echo carrier (the data of echo messages on a raw socket, `CAP_NET_RAW`) with an ARQ, TLS 1.3 and a pinned Ed25519 key. It holds `tcp`. | T-282 |
| `ss` | The Shadowsocks service. | T-294 |
| `trojan` | The Trojan service. | T-295 |
| `c-abi` | The C ABI. The code is the crate `crates/podnet-capi` (see The C library), which does not turn the feature on: the feature gates no code. | T-304 |
| `portable` | The portable subset. It takes code away. | T-305 |
| `full` | `tcp`, `ws`, `h2`, `quic`, `udp`, `poll`, `dns`, `s3`, `icmp`, `ss`, `trojan` and `c-abi`. It does not hold `portable`. | T-311 |

```sh
cargo build --release --features quic,icmp   # two optional carriers
cargo test --workspace --features full       # what the CI job features runs
```

* `full` is for CI. No flag at run time turns a feature on: a binary cannot
  run code that it was not built with.
* A carrier whose feature is off stays in the registry. `podnet link carriers`
  lists it with the reason `not built: the feature NAME is off (build with
  --features NAME)`, `podnet doctor` shows it as `unknown`, and a command
  that names it (`--carrier NAME`) exits 69 with the same reason. In a
  `--features` build, a carrier whose code is not in the tree says so and
  names its entry (T-279).
* The static musl build of the release set uses the default features. Build
  a feature on every target of the release set before it joins that set.
* `podnet::build_features` is the list (`OPTIONAL`, `FULL`), `built(NAME)`,
  `enabled()` and `require(NAME)` (the gate of a command: exit 69 with the
  feature name).

## The C library

`crates/podnet-capi` builds the C ABI of the link (T-304). The header is
`include/podnet.h` at the root of the repository. The crate has the crate
types `cdylib`, `staticlib` and `rlib`. The root crate has no `crate-type`:
no other build makes a shared library. The crate is a member of the workspace
and not a default member, so `cargo build --release` does not build it.

```sh
# The shared library (glibc target) and the static library:
cargo build --release -p podnet-capi --target x86_64-unknown-linux-gnu
#   target/x86_64-unknown-linux-gnu/release/libpodnet_capi.so  (soname libpodnet.so)
#   target/x86_64-unknown-linux-gnu/release/libpodnet_capi.a
# The static library for a static musl target:
CC_x86_64_unknown_linux_musl=clang cargo rustc --release -p podnet-capi \
  --target x86_64-unknown-linux-musl --crate-type staticlib
```

* The library crate is `podnet_capi`, so the files are `libpodnet_capi.so`
  and `libpodnet_capi.a`. The soname of the shared library is
  `libpodnet.so`. Copy the file to `libpodnet.so` to link with `-lpodnet`.
* A `cdylib` does not build for a static musl target: `cargo build` drops the
  crate type with the warning `dropping unsupported crate type cdylib`, and
  builds the static library. The shared library is a glibc build. The static
  library is the build for the musl target.
* The shared library exports the 14 functions of `include/podnet.h` and no
  other symbol: rustc hides each symbol that is not a `#[no_mangle]` item
  of the crate. A second linker version script conflicts with the script of
  rustc (`rust-lld: attempt to reassign symbol`), so the crate has none. A
  static library keeps the other symbols of Rust; its interface is the
  names `podnet_*`.
* Link a program with the static library: `cc prog.c libpodnet_capi.a
  -lpthread -lm -ldl`. Use `-I include`.
* Tests: `cargo test -p podnet-capi` runs the tests of the crate and
  `crates/podnet-capi/tests/c_abi.rs`. That test checks the symbols of the shared library and of
  the static library against the header, builds `crates/podnet-capi/tests/c_abi.c` against both
  libraries, and runs `crates/podnet-capi/tests/c_abi.py` (ctypes). A host without `nm`, `cc` or
  `python3` prints `SKIP NAME: TOOL not found` for that test. The CI job
  `c-abi` also runs the C test under AddressSanitizer and under valgrind (no
  leak), and builds the static library for musl.

## Fuzz

`fuzz/` has a libFuzzer target for each parser of bytes from the network
or from a file (Cap'n Proto, the RPC of the edge, capsules, proxy URLs and
answers, SRV and DoH, ACL rules, ingress rules, YAML, HTTP/2 headers, vnet
frames, the request head to the origin, the seccomp handlers, the wg-quick
file, the network specs and the router file, the messages, the zip and the
records of the transfer, the route announcements and the route table of the
link mesh). It
needs a nightly toolchain and glibc. `.github/workflows/fuzz.yml` runs each
target each week. A crash input becomes a unit test before the fix.

```sh
cargo +stable install cargo-fuzz --locked --version 0.13.1
cd fuzz
cargo +nightly-2026-10-15 run --bin seed -- corpus
cargo +nightly-2026-10-15 fuzz run capnp_message corpus/capnp_message -- -max_total_time=60 -malloc_limit_mb=64
```

`cargo test` runs each target on its seeds and on 2000 fixed mutations
(100 for `mesh_announce`, which signs; `fuzzing::tests::each_target_survives_mutations`).
