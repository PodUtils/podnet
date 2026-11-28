#!/bin/sh
# Hermetic, idempotent, multi-layer client capture rig orchestrator.
#
# Usage:
#   sh scripts/capture-rig/rig.sh --client cloudflare-warp --out /tmp/cap
#   sh scripts/capture-rig/rig.sh --client generic --cmd 'curl -fsS https://example.com' --out /tmp/cap
#   sh scripts/capture-rig/rig.sh --self-test
#   sh scripts/capture-rig/rig.sh --clean
#
# Properties:
#   - Hermetic: runs inside an ephemeral container built from scripts/capture-rig/Dockerfile
#     with no leftover containers, networks, or host state.
#   - Idempotent: safe to invoke repeatedly with the same --out directory; stale containers
#     are removed and artifacts are rebuilt cleanly with a SHA-256 manifest.
#   - Redundant: captures passive PCAP + SSLKEYLOGFILE + auto-fallback MITM/passthrough proxy
#     and dissects across three layers (tshark decrypted, tshark passive, pure-Python PCAP).
#   - Secret-safe: runs redact.py on every artifact, audits zero leaked secrets, and scrubs
#     raw keylogs/PCAPs by default unless --keep-raw is passed.
set -eu

RIG_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
IMAGE="${CAPTURE_RIG_IMAGE:-podnet-capture-rig:local}"
CLIENT=""
OUT_DIR=""
MODE="dual"
CLIENT_CMD=""
CACHE_DEB=""
KEEP_RAW=0
SELF_TEST=0
CLEAN_ONLY=0
STRACE=0

usage() {
    cat <<'EOF'
Usage: rig.sh [OPTIONS]
  --client NAME     Client recipe (cloudflare-warp, generic)
  --cmd COMMAND     Command for --client generic
  --out DIR         Output directory for redacted bundle
  --mode MODE       dual (default), mitm, or passive
  --cache-deb PATH  Optional pre-downloaded .deb cache for idempotent re-runs
  --strace          Also record network syscalls in generic mode
  --keep-raw        Retain raw/ directory (keylog and PCAP); scrubbed by default
  --self-test       Run offline hermetic verification without Docker or network
  --clean           Remove local capture rig Docker image and exit
  -h, --help        Show this help
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --client) CLIENT="$2"; shift 2 ;;
        --cmd) CLIENT_CMD="$2"; shift 2 ;;
        --out) OUT_DIR="$2"; shift 2 ;;
        --mode) MODE="$2"; shift 2 ;;
        --cache-deb) CACHE_DEB="$2"; shift 2 ;;
        --strace) STRACE=1; shift ;;
        --keep-raw) KEEP_RAW=1; shift ;;
        --self-test) SELF_TEST=1; shift ;;
        --clean) CLEAN_ONLY=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 64 ;;
    esac
done

if [ "$CLEAN_ONLY" -eq 1 ]; then
    docker rmi -f "$IMAGE" >/dev/null 2>&1 || true
    echo "capture rig image cleaned"
    exit 0
fi

if [ "$SELF_TEST" -eq 1 ]; then
    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT
    mkdir -p "$TMP/raw" "$TMP/redacted"
    cat <<'EOF' > "$TMP/raw/sample.json"
{"account_type":"free","token":"secret-token-value-123456","authorization":"Bearer abcdef1234567890"}
EOF
    printf 'CLIENT_RANDOM deadbeef cafebabe\n' > "$TMP/raw/tls_keylog.txt"
    python3 "$RIG_DIR/redact.py" --in-dir "$TMP/raw" --out-dir "$TMP/redacted"
    if [ -f "$TMP/redacted/tls_keylog.txt" ]; then
        echo "self-test failed: keylog leaked into redacted bundle" >&2
        exit 1
    fi
    if grep -q "secret-token-value" "$TMP/redacted/sample.json"; then
        echo "self-test failed: token leaked" >&2
        exit 1
    fi
    echo "capture-rig self-test: ok"
    exit 0
fi

if [ -z "$CLIENT" ]; then
    echo "--client is required (cloudflare-warp or generic)" >&2
    exit 64
fi
if [ ! -f "$RIG_DIR/clients/${CLIENT}.sh" ]; then
    echo "unknown client recipe: $CLIENT" >&2
    exit 64
fi
if [ -z "$OUT_DIR" ]; then
    OUT_DIR="/tmp/podnet-capture-rig/${CLIENT}-$(date -u +%Y%m%dT%H%M%SZ)"
fi

if ! command -v docker >/dev/null 2>&1; then
    echo "docker is required for hermetic container captures" >&2
    exit 69
fi

mkdir -p "$OUT_DIR"
OUT_ABS=$(CDPATH='' cd -- "$OUT_DIR" && pwd)
RAW_DIR="$OUT_ABS/raw"
REDACTED_DIR="$OUT_ABS/redacted"
rm -rf "$RAW_DIR" "$REDACTED_DIR"
mkdir -p "$REDACTED_DIR"

# Build the capture rig image idempotently.
docker build -q -t "$IMAGE" "$RIG_DIR" >/dev/null

CNAME="podnet-capture-${CLIENT}-$$"
cleanup_container() {
    docker rm -f "$CNAME" >/dev/null 2>&1 || true
}
trap cleanup_container EXIT INT TERM

# Run the client recipe and redactor inside the ephemeral container so mode-0600
# daemon artifacts are readable to redact.py and raw secrets stay inside the container
# unless --keep-raw is explicitly passed.
CACHE_MOUNT=""
if [ -n "$CACHE_DEB" ] && [ -f "$CACHE_DEB" ]; then
    CACHE_ABS=$(CDPATH='' cd -- "$(dirname -- "$CACHE_DEB")" && pwd)/$(basename -- "$CACHE_DEB")
    CACHE_MOUNT="-v ${CACHE_ABS}:/cache/cloudflare-warp.deb:ro"
fi

RAW_MOUNT=""
RAW_IN_CONTAINER="/tmp/rig-raw"
if [ "$KEEP_RAW" -eq 1 ]; then
    mkdir -p "$RAW_DIR"
    RAW_MOUNT="-v ${RAW_DIR}:/work/raw"
    RAW_IN_CONTAINER="/work/raw"
fi

HOST_UID=$(id -u)
HOST_GID=$(id -g)

# shellcheck disable=SC2086
docker run --rm --name "$CNAME" \
    --cap-add NET_ADMIN --cap-add NET_RAW \
    --sysctl net.ipv6.conf.all.disable_ipv6=0 \
    -e CAPTURE_RAW_DIR="$RAW_IN_CONTAINER" \
    -e CAPTURE_MODE="$MODE" \
    -e CAPTURE_CMD="$CLIENT_CMD" \
    -e CAPTURE_STRACE="$STRACE" \
    -v "$REDACTED_DIR:/work/redacted" \
    $RAW_MOUNT \
    $CACHE_MOUNT \
    "$IMAGE" -c "
        set -eu
        sh \"/rig/clients/${CLIENT}.sh\"
        python3 /rig/redact.py --in-dir \"$RAW_IN_CONTAINER\" --out-dir /work/redacted
        chown -R ${HOST_UID}:${HOST_GID} /work/redacted
    "

echo "capture complete: $REDACTED_DIR"
echo "manifest: $REDACTED_DIR/manifest.json"
echo "audit:    $REDACTED_DIR/redaction_audit.json"
