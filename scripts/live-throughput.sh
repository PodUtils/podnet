#!/bin/sh
# Measure the throughput of one quick tunnel. For each size in SIZES (MiB):
# a GET and a POST on the HTTP path. For the largest size: the same through
# `podnet connect` (a --forward WebSocket) and through a WebSocket client in
# Python. Then the tunnel must still answer: a GET of the largest size again,
# and a GET of /. Each GET checks the byte count. The log of CI is public: the
# tunnel host is masked before it can be printed, and no output of podnet is
# printed without the mask.
#
#   sh scripts/live-throughput.sh PODNET PROTOCOL [SIZES] [TRANSPORT]
#
# SIZES is a list such as "1 4 16" (default "16"); TRANSPORT is direct
# (default) or warp. The script prints one line for each transfer:
# "rate PROTOCOL+TRANSPORT PATH METHOD MiB/s".
set -eu
podnet=$1
protocol=$2
sizes=${3:-16}
transport=${4:-direct}
dir=$(mktemp -d)
here=$(cd "$(dirname "$0")" && pwd)
# A free port of the loopback (a fixed port can be held by an earlier step).
port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
local=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
# Each process of the script is in pids: `jobs -p` in a command substitution
# of an EXIT trap gives no process in dash, and an origin outlived the script.
pids=""
trap 'kill $pids 2> /dev/null || true' EXIT
largest=0
for s in $sizes; do [ "$s" -gt "$largest" ] && largest=$s; done
label="$protocol+$transport"

# T-535: each failure after the start of the tunnel prints the masked tail of
# the lines of podnet.
log() {
    sed -E 's/[a-z0-9-]+\.trycloudflare\.com/<tunnel>.trycloudflare.com/g' "$dir/tunnel.err" | tail -25
    if [ -s "$dir/connect.err" ]; then
        echo "== podnet connect"
        sed -E 's/[a-z0-9-]+\.trycloudflare\.com/<tunnel>.trycloudflare.com/g' "$dir/connect.err" | tail -25
    fi
}

python3 -I "$here/origin-sink.py" "$port" &
pids="$pids $!"
sleep 1

"$podnet" tunnel --url "http://127.0.0.1:$port" --forward "tcp://127.0.0.1:$port" \
    --protocol "$protocol" --transport "$transport" --json --no-verify \
    > "$dir/tunnel.out" 2> "$dir/tunnel.err" &
tunnel=$!
pids="$pids $tunnel"
i=0
until grep -q '"event":"ready"' "$dir/tunnel.out" 2> /dev/null; do
    i=$((i + 1))
    if ! kill -0 "$tunnel" 2> /dev/null; then
        log
        wait "$tunnel" && rc=0 || rc=$?
        echo "podnet tunnel ended before it was ready (exit $rc)"
        exit 1
    fi
    if [ "$i" -gt 90 ]; then
        log
        echo "the tunnel is not ready after 90 s"
        exit 1
    fi
    sleep 1
done
url=$(jq -r 'select(.event == "ready") | .url' "$dir/tunnel.out")
host=${url#https://}
# Issue 203: the ready event gives the endpoint for podnet connect.
forward=$(jq -r 'select(.event == "ready") | .forward' "$dir/tunnel.out")
[ "$forward" = "wss://$host/__podnet/ws" ] || { echo "the ready event has no forward endpoint"; exit 1; }
echo "::add-mask::$host"
# A new name takes some seconds to resolve (issue 119).
sh "$(dirname "$0")/live-wait-name.sh" "$host" 240 || { log; exit 1; }
i=0
until curl -fsS -o /dev/null --max-time 10 "$url/"; do
    i=$((i + 1))
    [ "$i" -lt 30 ] || { echo "the public URL does not answer"; log; exit 1; }
    sleep 2
done


rate() {
    # rate NAME METHOD URL MIB [CURL ARGUMENT...]
    name=$1 method=$2 target=$3 mib=$4
    shift 4
    want=$((mib * 1048576))
    start=$(date +%s.%N)
    hdr="$dir/rate.hdr"
    body="$dir/rate.body"
    : > "$hdr"
    : > "$body"
    if [ "$method" = GET ]; then
        res=$(curl -sS --max-time 300 -D "$hdr" -o "$body" -w '%{http_code} %{size_download}' "$@" "$target") || res="000 fail"
    else
        head -c "$want" /dev/zero > "$dir/up.bin"
        res=$(curl -sS --max-time 300 -D "$hdr" -o "$body" -w '%{http_code} %{size_upload}' \
            --data-binary "@$dir/up.bin" -H 'Content-Type: application/octet-stream' "$@" "$target") || res="000 fail"
    fi
    end=$(date +%s.%N)
    code=${res%% *}
    bytes=${res#* }
    if [ "$code" != 200 ] || [ "$bytes" != "$want" ]; then
        ray=$(grep -i '^cf-ray:' "$hdr" | tr -d '\r' | head -1 || true)
        snippet=$(head -c 512 "$body" | tr '\r\n' ' ' | sed -E 's/[a-z0-9-]+\.trycloudflare\.com/<tunnel>.trycloudflare.com/g')
        echo "rate $label $name $method FAILED: status=$code bytes=$bytes/$want ${ray:+$ray }body=${snippet}"
        log
        exit 1
    fi
    echo "rate $label $name $method $(echo "$bytes $start $end" | awk '{ printf "%.2f MiB/s (%d bytes, %.2f s)", $1 / 1048576 / ($3 - $2), $1, $3 - $2 }')"
}

for s in $sizes; do
    rate http GET "$url/$s.bin" "$s"
    rate http POST "$url/sink" "$s"
done

"$podnet" connect -L "tcp://127.0.0.1:$local" "$forward" > "$dir/connect.out" 2> "$dir/connect.err" &
pids="$pids $!"
sleep 2
rate connect GET "http://127.0.0.1:$local/$largest.bin" "$largest"
rate connect POST "http://127.0.0.1:$local/sink" "$largest"

# Issue 206: the median time of 20 small requests, a new connection each,
# over the public URL and over podnet connect.
latency() {
    # latency NAME URL
    : > "$dir/lat.txt"
    : > "$dir/lat.err"
    for _ in $(seq 20); do
        hdr="$dir/lat.hdr"
        body="$dir/lat.body"
        : > "$hdr"
        : > "$body"
        res=$(curl -sS --max-time 30 -D "$hdr" -o "$body" -w '%{http_code} %{time_total}' "$2") || res="000 fail"
        code=${res%% *}
        elapsed=${res#* }
        if [ "$code" = 200 ]; then
            echo "$elapsed" >> "$dir/lat.txt"
        else
            ray=$(grep -i '^cf-ray:' "$hdr" | tr -d '\r' | head -1 || true)
            snippet=$(head -c 512 "$body" | tr '\r\n' ' ' | sed -E 's/[a-z0-9-]+\.trycloudflare\.com/<tunnel>.trycloudflare.com/g')
            echo "fail" >> "$dir/lat.txt"
            echo "status=$code ${ray:+$ray }body=${snippet}" >> "$dir/lat.err"
        fi
    done
    if grep -q fail "$dir/lat.txt"; then
        echo "latency $label $1: $(grep -c fail "$dir/lat.txt") of 20 requests FAILED"
        head -5 "$dir/lat.err"
        log
        exit 1
    fi
    sort -n "$dir/lat.txt" | awk -v n="$1" -v l="$label" '{ a[NR] = $1 } END { printf "latency %s %s: p50 %.0f ms, max %.0f ms (20 requests)\n", l, n, a[int((NR + 1) / 2)] * 1000, a[NR] * 1000 }'
}
latency http "$url/"
latency connect "http://127.0.0.1:$local/"

if python3 -c 'import websockets' 2> /dev/null; then
    python3 -I "$here/ws-client.py" "wss://$host/__podnet/ws" "$largest" | sed "s/^/rate $label /"
fi

# The tunnel still works after the large transfers.
rate "http again" GET "$url/$largest.bin" "$largest"
curl -fsS --max-time 30 "$url/" | grep -qx ok || { echo "the tunnel does not answer after the transfers"; log; exit 1; }
echo "rate $label: the tunnel answers after the transfers"
