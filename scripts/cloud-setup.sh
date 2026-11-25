#!/bin/sh
# Prepare the native development tools and fetch the locked dependencies.
set -eu

cd "$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)"
# shellcheck source=scripts/cloud-env.sh
. ./scripts/cloud-env.sh

if [ "$(uname -s)" != Linux ]; then
    echo "cloud-setup: Linux is required" >&2
    exit 2
fi
for tool in cc curl git python3 sha256sum timeout; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'cloud-setup: install %s on this host\n' "$tool" >&2
        exit 1
    fi
done

install_rustup() (
    version=1.29.3
    case "$(uname -m)" in
        x86_64)
            arch=x86_64
            checksum=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
            ;;
        aarch64|arm64)
            arch=aarch64
            checksum=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282
            ;;
        *)
            echo "cloud-setup: install Rustup for $(uname -m) on this host" >&2
            exit 1
            ;;
    esac
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/podnet-rustup.XXXXXXXX")
    trap 'rm -f -- "$tmp/rustup-init"; rmdir -- "$tmp"' EXIT
    url="https://static.rust-lang.org/rustup/archive/$version/$arch-unknown-linux-gnu/rustup-init"
    curl -qfsSL --proto '=https' --proto-redir '=https' --max-time 120 \
        --max-filesize 33554432 -o "$tmp/rustup-init" "$url" </dev/null
    if ! printf '%s  %s\n' "$checksum" "$tmp/rustup-init" | sha256sum --check --status; then
        echo "cloud-setup: Rustup installer failed the SHA-256 check" >&2
        exit 1
    fi
    chmod u+x "$tmp/rustup-init"
    timeout 120 "$tmp/rustup-init" -y --no-modify-path --default-toolchain none </dev/null
)

if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
    install_rustup
fi
if ! RUSTUP_AUTO_INSTALL=0 rustup which cargo >/dev/null 2>&1 ||
   ! RUSTUP_AUTO_INSTALL=0 cargo fmt --version >/dev/null 2>&1 ||
   ! RUSTUP_AUTO_INSTALL=0 cargo clippy --version >/dev/null 2>&1; then
    timeout 600 rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal \
        --component clippy,rustfmt --no-self-update </dev/null
fi
rustc --version
GIT_TERMINAL_PROMPT=0 timeout 600 cargo fetch --locked </dev/null
printf 'cloud-setup: %s on %s\n' "$(uname -m)" "$(uname -r)"
printf 'cloud-setup: uid %s\n' "$(id -u)"
if command -v unshare >/dev/null 2>&1; then
    if unshare -U true 2>/dev/null; then
        echo "cloud-setup: user namespaces available"
    else
        echo "cloud-setup: user namespaces unavailable"
    fi
fi
df -h .
