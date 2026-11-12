# Build from source

The repository is a Cargo workspace:

| Crate | Contents |
| --- | --- |
| `podnet` (root) | The CLI and the library: the edge client (provisioning, discovery, QUIC, HTTP/2, Cap'n Proto and its RPC), the service layer, WARP. |
| `crates/podnet-core` | A library: the core layer of podnet (the process, the files, the state, the reports, the supervisor). The other crates use it. |
| `crates/podnet-serve` | A library: the services of podnet (the file server, the file transfer, the gate, the ingress rules, the origins, the client of the SSH relay). It uses `podnet-core`. |
| `crates/podnet-netns` | A library: run a program with all its traffic sent through a proxy (`podnet pod`). |
| `crates/podnet-vnet` | A library: the userspace virtual network (`podnet net`) and the C sources of its `LD_PRELOAD` shims. |

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
| `quic` | The QUIC carrier. | T-273 |
| `s3` | The object-storage carrier. | T-279 |
| `icmp` | The ICMP echo carrier. | T-282 |
| `ss` | The Shadowsocks service. | T-294 |
| `trojan` | The Trojan service. | T-295 |
| `c-abi` | The C ABI. | T-304 |
| `portable` | The portable subset. It takes code away. | T-305 |
| `full` | `quic`, `s3`, `icmp`, `ss`, `trojan` and `c-abi`. It does not hold `portable`. | T-311 |

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
  names its entry (T-273, T-279 or T-282).
* The static musl build of the release set uses the default features. Build
  a feature on every target of the release set before it joins that set.
* `podnet::build_features` is the list (`OPTIONAL`, `FULL`), `built(NAME)`,
  `enabled()` and `require(NAME)` (the gate of a command: exit 69 with the
  feature name).

## Fuzz

`fuzz/` has a libFuzzer target for each parser of bytes from the network
(Cap'n Proto, the RPC of the edge, capsules, proxy URLs and answers, SRV
and DoH, ACL rules, ingress rules, YAML, HTTP/2 headers, vnet frames, the
request head to the origin). It
needs a nightly toolchain and glibc. `.github/workflows/fuzz.yml` runs each
target each week. A crash input becomes a unit test before the fix.

```sh
cargo +stable install cargo-fuzz --locked --version 0.13.1
cd fuzz
cargo +nightly-2026-10-15 run --bin seed -- corpus
cargo +nightly-2026-10-15 fuzz run capnp_message corpus/capnp_message -- -max_total_time=60 -malloc_limit_mb=64
```

`cargo test` runs each target on its seeds and on 2000 fixed mutations
(`fuzzing::tests::each_target_survives_mutations`).
