#!/bin/sh
# Generic hermetic client capture recipe (runs any future client command inside the capture rig).
# Enables all capture planes simultaneously:
#   1. Passive tcpdump PCAP across all interfaces
#   2. SSLKEYLOGFILE for TLS 1.3 / QUIC / HTTP/3 decryption
#   3. Auto-fallback MITM/passthrough HTTP CONNECT proxy (mitm_proxy.py)
#   4. Syscall network trace fallback via strace (optional CAPTURE_STRACE=1)
set -eu

RAW_DIR="${CAPTURE_RAW_DIR:-/work/raw}"
MODE="${CAPTURE_MODE:-dual}"
PROXY_PORT="${CAPTURE_PROXY_PORT:-18080}"
CLIENT_CMD="${CAPTURE_CMD:-}"

if [ -z "$CLIENT_CMD" ]; then
    echo "generic capture recipe requires CAPTURE_CMD" >&2
    exit 64
fi

mkdir -p "$RAW_DIR"
TCPDUMP_PID=""
PROXY_PID=""

cleanup() {
    set +e
    if [ -n "$PROXY_PID" ]; then
        kill "$PROXY_PID" 2>/dev/null || true
    fi
    if [ -n "$TCPDUMP_PID" ]; then
        kill -INT "$TCPDUMP_PID" 2>/dev/null || true
        wait "$TCPDUMP_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

tcpdump -U --immediate-mode -i any -s 0 -w "$RAW_DIR/capture.pcap" > "$RAW_DIR/tcpdump.log" 2>&1 &
TCPDUMP_PID=$!
sleep 0.3

PROXY_ENV=""
if [ "$MODE" != "passive" ]; then
    python3 /rig/mitm_proxy.py --host 127.0.0.1 --port "$PROXY_PORT" --out "$RAW_DIR" --mode "$MODE" \
        > "$RAW_DIR/proxy.log" 2>&1 &
    PROXY_PID=$!
    i=0
    while [ "$i" -lt 40 ] && [ ! -f "$RAW_DIR/proxy.ready" ]; do
        i=$((i + 1))
        sleep 0.2
    done
    PROXY_ENV="http://127.0.0.1:${PROXY_PORT}"
fi

KEYLOG="$RAW_DIR/tls_keylog.txt"
: > "$KEYLOG"
chmod 600 "$KEYLOG"
mkdir -p "$RAW_DIR/qlog"

CA_BUNDLE="/etc/ssl/certs/ca-certificates.crt"
if [ -f "$RAW_DIR/ca/ca.crt" ] && [ "$MODE" != "passive" ]; then
    cat /etc/ssl/certs/ca-certificates.crt "$RAW_DIR/ca/ca.crt" > "$RAW_DIR/combined_ca.pem"
    CA_BUNDLE="$RAW_DIR/combined_ca.pem"
fi

set +e
if [ "${CAPTURE_STRACE:-0}" = "1" ]; then
    SSLKEYLOGFILE="$KEYLOG" QLOGDIR="$RAW_DIR/qlog" \
    HTTP_PROXY="$PROXY_ENV" HTTPS_PROXY="$PROXY_ENV" http_proxy="$PROXY_ENV" https_proxy="$PROXY_ENV" \
    SSL_CERT_FILE="$CA_BUNDLE" CURL_CA_BUNDLE="$CA_BUNDLE" \
    strace -f -e trace=connect,sendto,recvfrom -o "$RAW_DIR/strace.log" \
        /bin/sh -c "$CLIENT_CMD" > "$RAW_DIR/client_stdout.txt" 2> "$RAW_DIR/client_stderr.txt"
    RC=$?
else
    SSLKEYLOGFILE="$KEYLOG" QLOGDIR="$RAW_DIR/qlog" \
    HTTP_PROXY="$PROXY_ENV" HTTPS_PROXY="$PROXY_ENV" http_proxy="$PROXY_ENV" https_proxy="$PROXY_ENV" \
    SSL_CERT_FILE="$CA_BUNDLE" CURL_CA_BUNDLE="$CA_BUNDLE" \
        /bin/sh -c "$CLIENT_CMD" > "$RAW_DIR/client_stdout.txt" 2> "$RAW_DIR/client_stderr.txt"
    RC=$?
fi
set -e
printf '%s\n' "$RC" > "$RAW_DIR/client_exit_code.txt"

sleep 0.5
if [ -n "$PROXY_PID" ]; then
    kill "$PROXY_PID" 2>/dev/null || true
    PROXY_PID=""
fi
kill -INT "$TCPDUMP_PID" 2>/dev/null || true
wait "$TCPDUMP_PID" 2>/dev/null || true
TCPDUMP_PID=""

python3 /rig/quic_dissect.py \
    --pcap "$RAW_DIR/capture.pcap" \
    --keylog "$KEYLOG" \
    --qlog-dir "$RAW_DIR/qlog" \
    --out "$RAW_DIR/wire_summary.json"
