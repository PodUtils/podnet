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
