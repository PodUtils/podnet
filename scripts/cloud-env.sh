#!/bin/sh
# Source this file from the repository root.
# Keep the tools and Cargo cache in the hosted checkout by default.
export CARGO_HOME="${CARGO_HOME:-$PWD/.work/cloud/cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$PWD/.work/cloud/rustup}"
# Use the same pin as the main CI job.
podnet_rust_toolchain=$(sed -n 's/^  RUST_TOOLCHAIN: "\([^"]*\)"$/\1/p' .github/workflows/ci.yml)
if [ -z "$podnet_rust_toolchain" ]; then
    echo "cloud-env: open the repository root and check the Rust pin in ci.yml" >&2
    return 1
fi
export RUSTUP_TOOLCHAIN=$podnet_rust_toolchain
unset podnet_rust_toolchain
case ":$PATH:" in
    *":$CARGO_HOME/bin:"*) ;;
    *) export PATH="$CARGO_HOME/bin:$PATH" ;;
esac
export CARGO_NET_GIT_FETCH_WITH_CLI="${CARGO_NET_GIT_FETCH_WITH_CLI:-true}"
if [ -z "${CARGO_BUILD_JOBS:-}" ]; then
    podnet_cpus=$(nproc 2>/dev/null || printf 4)
    case "$podnet_cpus" in
        [1-3]) CARGO_BUILD_JOBS=$podnet_cpus ;;
        *) CARGO_BUILD_JOBS=4 ;;
    esac
    unset podnet_cpus
fi
export CARGO_BUILD_JOBS
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
