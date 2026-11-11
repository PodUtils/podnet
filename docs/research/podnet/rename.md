# The names of podnet

> Reading of the tree at 3bed023 (2026-11-05), written with the names of the tree now. Paths and counts are history. The plan is the record (`TODO/`).

Each kind of place that holds the name of the project, and the rule for each. The release of a new name has
no compatibility layer: an older binary does not update to it, and no alias of an older name exists (beta;
`docs/decisions.md`).

## 1. Names by kind

| Kind | Name |
| --- | --- |
| Package, library, binary | `podnet` (`Cargo.toml`: `[package] name`, `[lib] name`, `[[bin]] name`); the CLI name is `podnet` (`src/cli/mod.rs`) |
| Crates | `podnet-core`, `podnet-netns`, `podnet-vnet`, `podnet-serve`, `podnet-todo` (`crates/`) |
| Environment variables | `PODNET_*`; the shims read `PODNET_VNET_*` and `PODNET_SOCKS_*`; the build sets `PODNET_BUILD_COMMIT`, `PODNET_BUILD_TARGET`, `PODNET_BUILD_DATE`; `PODNET_REPO` is read by `scripts/install.sh` |
| Shim files | `crates/podnet-vnet/shim/podnet-vnet.c`, `crates/podnet-vnet/shim/podnet-socks.c`, `libpodnet-vnet.so`, `libpodnet-socks.so` |
| Sockets in `/tmp` | `/tmp/podnet-vnet.socks`, `/tmp/podnet-vnet.switch`, `/tmp/podnet-socks.sock` |
| State directory | The first writable of `$XDG_STATE_HOME/podnet`, `$HOME/.local/state/podnet`, `$XDG_RUNTIME_DIR/podnet`, `podnet-<uid>` in the temporary directory |
| TLS roots file | `podnet-ca.pem` beside the binary |
| TUN device | `podnet0` (default) |
| Autostart | Unit `podnet-<name>.service`; cron marker `# podnet autostart <name>` |
| Shell marker | `# >>> podnet warp up >>>` in `~/.profile` and `~/.bashrc` |
| Process title | `podnet` |
| JSON documents | `schema` (integer 1) and `podnet` (the version and the commit), set by `status::stamped` |
| Log filter | The tracing target is the crate name: `RUST_LOG=podnet=debug` |
| Metrics | `podnet_connections_total`, `podnet_streams_total`, `podnet_active_streams`, `podnet_bytes_in_total`, `podnet_bytes_out_total`, `podnet_errors_total`, `podnet_reconnects_total`, `podnet_decisions_total` |
| Wire-visible strings | User-Agent `podnet/<version>`; `Basic realm="podnet"`; paths `/__podnet/ws` and `/__podnet/unlock`; cookie `podnet_pin` |
| Release assets | `podnet-<target>`, `podnet.1`, `SHA256SUMS` and its cosign bundle |
| Image | `ghcr.io/podutils/podnet` |

## 2. The self-update chain

`podnet update` (`src/update.rs`, `src/update/cosign.rs`) has these values compiled in:

| Item | Value |
| --- | --- |
| Repository | `PodUtils/podnet` (`REPOSITORY`) |
| Release list | `https://api.github.com/repos/{REPOSITORY}/releases?per_page=30` |
| Fallback list | `https://github.com/{REPOSITORY}/releases.atom` |
| Pinned download | `https://github.com/{REPOSITORY}/releases/download/{tag}/{name}` |
| Asset | `podnet-{asset_target(TARGET)}` |
| Certificate identity | `https://github.com/{REPOSITORY}/.github/workflows/release.yml@refs/tags/{tag}`, issuer `https://token.actions.githubusercontent.com` |
| Trial of the new binary | The first line of `--version` starts with `podnet {version} ` |

Three checks must agree for a release: the asset name, the certificate identity and the trial prefix.
`release.yml` and `release-proofs.yml` take the identity from `$GITHUB_REPOSITORY`, so a change of the
repository name needs no change in them. A release that another repository or another workflow signed fails
the identity check. `scripts/install.sh` reads `PODNET_REPO` for the repository and checks the same identity.
`podnet update` ignores `PODNET_REPO` and warns.

## 3. Rules for a change of a name

1. Change the name in one commit with no file move; `git log --follow` then stays readable.
2. Run `cargo fmt --all`: a longer name reflows the lines.
3. Change the pairs together: the CLI name and the unit tests that assert it (`CARGO_BIN_EXE_podnet`), the
   manual (`src/manual/facts.rs`), the workflows, the scripts and the documents.
4. Write each breaking change of a name in the `## Breaking` section of the release note
   (`tests/docs.rs::each_new_release_note_has_a_breaking_section`).
5. A test that runs the binary from `/tmp` fails in `crates/podnet-core/src/autostart.rs` (it refuses a binary in `/tmp`): build
   outside `/tmp`.
