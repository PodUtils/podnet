#!/bin/sh
# Prepare the native development tools and fetch the locked dependencies.
#
# Usage: sh scripts/cloud-setup.sh [--dev-tools]
#
# With --dev-tools, also add the host musl Rust target and install missing
# groff, mandoc, shellcheck and musl-tools packages when root or passwordless
# sudo apt-get works.
set -eu

dev_tools=0
while [ $# -gt 0 ]; do
    case "$1" in
        --dev-tools)
            dev_tools=1
            shift
            ;;
        -h | --help)
            sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "usage: sh scripts/cloud-setup.sh [--dev-tools]" >&2
            exit 64
            ;;
    esac
done

cd "$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)"
# shellcheck source=scripts/cloud-env.sh
. ./scripts/cloud-env.sh

if [ "$(uname -s)" != Linux ]; then
    echo "cloud-setup: Linux is required" >&2
    exit 2
fi
for tool in cc curl flock git python3 sha256sum timeout; do
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

install_dev_tools() {
    case "$(uname -m)" in
        x86_64) musl_target=x86_64-unknown-linux-musl ;;
        aarch64|arm64) musl_target=aarch64-unknown-linux-musl ;;
        *) musl_target= ;;
    esac
    if [ -n "$musl_target" ] && ! rustup target list --installed | grep -Fxq "$musl_target"; then
        timeout 300 rustup target add "$musl_target" </dev/null
    fi
    missing=
    for pkg in groff mandoc shellcheck; do
        if ! command -v "$pkg" >/dev/null 2>&1; then
            missing="$missing $pkg"
        fi
    done
    if ! command -v musl-gcc >/dev/null 2>&1; then
        missing="$missing musl-tools"
    fi
    if [ -n "$missing" ] && command -v apt-get >/dev/null 2>&1; then
        apt_sudo=none
        if [ "$(id -u)" = 0 ]; then
            apt_sudo=
        elif command -v sudo >/dev/null 2>&1 && sudo -n -E true 2>/dev/null; then
            apt_sudo="sudo -n -E"
        elif command -v sudo >/dev/null 2>&1 &&
             (sudo -n true 2>/dev/null || sudo -n apt-get --version >/dev/null 2>&1); then
            apt_sudo="sudo -n"
        fi
        if [ "$apt_sudo" != none ]; then
            # shellcheck disable=SC2086
            DEBIAN_FRONTEND=noninteractive timeout 300 $apt_sudo apt-get update -qq </dev/null &&
                DEBIAN_FRONTEND=noninteractive timeout 300 $apt_sudo apt-get install -y -qq \
                    --no-install-recommends $missing </dev/null
        fi
    fi
}

mkdir -p "$CARGO_HOME"
exec 9>"$CARGO_HOME/podnet-setup.lock"
if ! flock -w 120 9; then
    echo "cloud-setup: another setup still holds the Cargo cache lock" >&2
    exit 1
fi
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
    install_rustup
fi
export RUSTUP_AUTO_INSTALL=0
if ! rustup which cargo >/dev/null 2>&1 ||
   ! cargo fmt --version >/dev/null 2>&1 ||
   ! cargo clippy --version >/dev/null 2>&1; then
    timeout 600 rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal \
        --component clippy,rustfmt --no-self-update </dev/null
fi
if [ "$dev_tools" = 1 ]; then
    install_dev_tools
fi
rustc --version
host=$(rustc -vV | sed -n 's/^host: //p')
test -n "$host"
if ! timeout 30 cargo fetch --locked --offline --target "$host" </dev/null >/dev/null 2>&1; then
    GIT_TERMINAL_PROMPT=0 timeout 600 cargo fetch --locked --target "$host" </dev/null
fi
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
