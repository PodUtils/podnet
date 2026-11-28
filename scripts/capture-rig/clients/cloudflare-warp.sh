#!/bin/sh
# Hermetic official cloudflare-warp capture recipe (runs inside the capture rig container).
# Captures registration, account fields, and proxy-mode L4/MASQUE wire traffic using
# redundant capture planes (SSLKEYLOGFILE + tcpdump PCAP + optional proxy + tshark/fallback).
set -eu

RAW_DIR="${CAPTURE_RAW_DIR:-/work/raw}"
WARP_DEB_URL="${WARP_DEB_URL:-https://pkg.cloudflareclient.com/pool/noble/main/c/cloudflare-warp/cloudflare-warp_2026.8.2115.0_amd64.deb}"
WARP_DEB_SHA256="${WARP_DEB_SHA256:-0209107860c3c6851a9ab8fd03690121a3dea3199b084f8197620072c671cd68}"
WARP_CACHE="${WARP_CACHE_DEB:-/cache/cloudflare-warp.deb}"
WARP_PORT="${WARP_PROXY_PORT:-40000}"

mkdir -p "$RAW_DIR" /run/dbus /run/cloudflare-warp /var/lib/cloudflare-warp

TCPDUMP_PID=""
WARP_PID=""
DBUS_PID=""
REGISTERED=0

cleanup() {
    set +e
    if [ "$REGISTERED" -eq 1 ] && command -v warp-cli >/dev/null 2>&1; then
        warp-cli --accept-tos disconnect >/dev/null 2>&1 || true
        warp-cli --accept-tos registration delete >/dev/null 2>&1 || true
    fi
    if [ -n "$WARP_PID" ]; then
        kill "$WARP_PID" 2>/dev/null || true
        wait "$WARP_PID" 2>/dev/null || true
    fi
    if [ -n "$TCPDUMP_PID" ]; then
        kill -INT "$TCPDUMP_PID" 2>/dev/null || true
        wait "$TCPDUMP_PID" 2>/dev/null || true
    fi
    if [ -n "$DBUS_PID" ]; then
        kill "$DBUS_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# 1. Obtain and verify the official cloudflare-warp package, then unpack cleanly (no systemd postinst).
DEB_PATH="/tmp/cloudflare-warp.deb"
if [ -f "$WARP_CACHE" ]; then
    cp "$WARP_CACHE" "$DEB_PATH"
else
    curl -fsSL -o "$DEB_PATH" "$WARP_DEB_URL"
fi
printf '%s  %s\n' "$WARP_DEB_SHA256" "$DEB_PATH" | sha256sum -c - > "$RAW_DIR/package_verify.txt"

dpkg-deb -x "$DEB_PATH" / > "$RAW_DIR/dpkg_install.log" 2>&1
warp-cli --version > "$RAW_DIR/client_version.txt" 2>&1 || true

# 2. Start dbus (required by warp-svc on Linux).
dbus-daemon --system --nofork --nopidfile > "$RAW_DIR/dbus.log" 2>&1 &
DBUS_PID=$!
sleep 0.5

# 3. Start passive full-packet capture across all interfaces.
tcpdump -U --immediate-mode -i any -s 0 -w "$RAW_DIR/capture.pcap" \
    'udp port 443 or tcp port 443 or udp port 2408 or udp port 500 or udp port 1701 or udp port 4500' \
    > "$RAW_DIR/tcpdump.log" 2>&1 &
TCPDUMP_PID=$!
sleep 0.5

# 4. Start official warp-svc with SSLKEYLOGFILE + QLOGDIR + debug logging so every
#    HTTP/3 CONNECT frame, QPACK header, QUIC transport parameter, and TLS extension is captured.
mkdir -p "$RAW_DIR/qlog" /var/log/cloudflare-warp
SSLKEYLOGFILE="$RAW_DIR/tls_keylog.txt" \
QLOGDIR="$RAW_DIR/qlog" \
RUST_LOG="info,warp_connection::forward_proxy=trace,warp_edge=debug,tokio_quiche=debug,h3=trace" \
warp-svc > "$RAW_DIR/warp-svc.log" 2>&1 &
WARP_PID=$!

# Wait for warp-svc socket.
i=0
while [ "$i" -lt 40 ]; do
    if warp-cli --accept-tos status >/dev/null 2>&1; then
        break
    fi
    i=$((i + 1))
    sleep 0.5
done

# 5. Register a fresh anonymous device, enable QLOG, configure proxy mode + MASQUE, connect, and send traffic.
warp-cli --accept-tos registration new > "$RAW_DIR/registration_new.txt" 2>&1 || true
REGISTERED=1
warp-cli --accept-tos debug qlog enable > "$RAW_DIR/qlog_enable.txt" 2>&1 || true
warp-cli --accept-tos tunnel protocol set MASQUE > "$RAW_DIR/protocol_set.txt" 2>&1 || true
warp-cli --accept-tos mode proxy > "$RAW_DIR/mode_proxy.txt" 2>&1 || true
warp-cli --accept-tos proxy port "$WARP_PORT" > "$RAW_DIR/proxy_port.txt" 2>&1 || true
warp-cli --accept-tos settings > "$RAW_DIR/settings.txt" 2>&1 || true
warp-cli --accept-tos registration show > "$RAW_DIR/registration_show.txt" 2>&1 || true
warp-cli --accept-tos connect > "$RAW_DIR/connect.txt" 2>&1 || true

# Wait for Connected status and listening proxy port.
i=0
while [ "$i" -lt 40 ]; do
    ST=$(warp-cli --accept-tos status 2>&1 || true)
    printf '%s\n' "$ST" > "$RAW_DIR/status.txt"
    if printf '%s' "$ST" | grep -qi "Connected"; then
        sleep 1
        break
    fi
    i=$((i + 1))
    sleep 0.5
done

warp-cli --accept-tos tunnel stats > "$RAW_DIR/tunnel_stats.txt" 2>&1 || true

# 6. Trigger real proxied TCP/HTTPS flows (both IP literal and hostname) through official proxy mode.
curl -sS --max-time 15 -x "socks5h://127.0.0.1:${WARP_PORT}" https://1.1.1.1/cdn-cgi/trace \
    > "$RAW_DIR/trace_ip.txt" 2>&1 || true
curl -sS --max-time 15 -x "socks5h://127.0.0.1:${WARP_PORT}" https://www.cloudflare.com/cdn-cgi/trace \
    > "$RAW_DIR/trace_name.txt" 2>&1 || true
curl -sS --max-time 15 -x "http://127.0.0.1:${WARP_PORT}" https://example.com/ \
    -o /dev/null -w 'http_code=%{http_code} time_total=%{time_total}\n' \
    > "$RAW_DIR/http_proxy_check.txt" 2>&1 || true

# Flush qlogs and collect all qlog/sqlog files written by warp-svc.
warp-cli --accept-tos debug qlog flush > "$RAW_DIR/qlog_flush.txt" 2>&1 || true
sleep 0.5
find /var/log/cloudflare-warp /var/lib/cloudflare-warp /tmp -maxdepth 3 \
    \( -name '*.qlog' -o -name '*.sqlog' \) -exec cp {} "$RAW_DIR/qlog/" \; 2>/dev/null || true

# Save redacted-safe copy of registration state metadata if present (redact.py scrubs secrets).
if [ -f /var/lib/cloudflare-warp/reg.json ]; then
    cp /var/lib/cloudflare-warp/reg.json "$RAW_DIR/reg.json"
fi
if [ -f /var/lib/cloudflare-warp/conf.json ]; then
    cp /var/lib/cloudflare-warp/conf.json "$RAW_DIR/conf.json"
fi

sleep 1
# Stop warp-svc and tcpdump cleanly before dissection.
if [ "$REGISTERED" -eq 1 ]; then
    warp-cli --accept-tos disconnect > "$RAW_DIR/disconnect.txt" 2>&1 || true
    warp-cli --accept-tos registration delete > "$RAW_DIR/registration_delete.txt" 2>&1 || true
    REGISTERED=0
fi
kill "$WARP_PID" 2>/dev/null || true
wait "$WARP_PID" 2>/dev/null || true
WARP_PID=""
kill -INT "$TCPDUMP_PID" 2>/dev/null || true
wait "$TCPDUMP_PID" 2>/dev/null || true
TCPDUMP_PID=""

# 7. Dissect PCAP + SSLKEYLOGFILE + QLOG across all four layers.
python3 /rig/quic_dissect.py \
    --pcap "$RAW_DIR/capture.pcap" \
    --keylog "$RAW_DIR/tls_keylog.txt" \
    --qlog-dir "$RAW_DIR/qlog" \
    --out "$RAW_DIR/wire_summary.json"
