#!/bin/sh
# Compare podnet and cloudflared quick tunnels with `podnet bench URL --json`
# against one local HTTP origin (T-221).
#
#   sh scripts/live-bench-compare.sh PODNET [DURATION] [CONNECTIONS]
#
# DURATION is in seconds (default 10). CONNECTIONS is the worker count
# (default 4). The script masks each quick-tunnel host for public CI logs.
set -eu

podnet=${1:?usage: live-bench-compare.sh PODNET [DURATION] [CONNECTIONS]}
dur=${2:-10}
conns=${3:-4}

dir=$(mktemp -d)
here=$(cd "$(dirname "$0")" && pwd)
pids=""

cleanup() {
    for p in $pids; do
        kill "$p" 2> /dev/null || true
    done
    rm -rf "$dir"
}
trap cleanup EXIT

mask() {
    sed -E 's/[a-z0-9-]+\.trycloudflare\.com/<tunnel>.trycloudflare.com/g'
}

show_tail() {
    label=$1
    file=$2
    if [ -s "$file" ]; then
        echo "== $label" >&2
        mask < "$file" | tail -25 >&2
    fi
}

check_quota() {
    file=$1
    if [ -s "$file" ] && grep -qiE 'HTTP 429|429 Too Many Requests|status[_ ]?code[=:]? *429|error code:? 1015|error 1015' "$file"; then
        echo "::warning::the quick-tunnel service refuses this runner (HTTP 429 / 1015); the comparison is skipped (T-589)"
        if [ -n "${GITHUB_ENV:-}" ]; then
            echo "LIVE_QUICK_REFUSED=1" >> "$GITHUB_ENV"
        fi
        exit 0
    fi
}

wait_http_200() {
    target_url=$1
    err_label=$2
    err_file=$3
    w=0
    until [ "$(curl -sS -o /dev/null --max-time 10 -w '%{http_code}' "$target_url/" 2> /dev/null || echo 000)" = "200" ]; do
        w=$((w + 1))
        if [ "$w" -ge 30 ]; then
            check_quota "$err_file"
            echo "$err_label: the public URL does not return HTTP 200" >&2
            show_tail "$err_label" "$err_file"
            exit 1
        fi
        sleep 2
    done
}

# Locate or download cloudflared and verify its SHA-256 digest.
cf_version="2026.11.2"
cf_sha256="56f13335143ec0e22772963290683a84a04114a35ec64cbc794a4d39ff9bb8cf"
cf_bin_url="https://github.com/cloudflare/cloudflared/releases/download/${cf_version}/cloudflared-linux-amd64"

if [ -n "${CLOUDFLARED:-}" ] && [ -x "$CLOUDFLARED" ]; then
    cloudflared=$CLOUDFLARED
elif command -v cloudflared > /dev/null 2>&1; then
    cloudflared=$(command -v cloudflared)
else
    cloudflared="$dir/cloudflared"
    sh "$here/retry.sh" curl -fsSL --max-time 120 -o "$cloudflared" "$cf_bin_url"
    printf '%s  %s\n' "$cf_sha256" "$cloudflared" | sha256sum -c - > /dev/null
    chmod +x "$cloudflared"
fi

# Start one local HTTP origin on a free loopback port.
port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -I "$here/origin-sink.py" "$port" &
origin_pid=$!
pids="$origin_pid"

i=0
until curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$port/" 2> /dev/null; do
    i=$((i + 1))
    if ! kill -0 "$origin_pid" 2> /dev/null; then
        echo "origin-sink.py stopped before ready" >&2
        exit 1
    fi
    if [ "$i" -ge 15 ]; then
        echo "origin-sink.py does not answer on 127.0.0.1:$port" >&2
        exit 1
    fi
    sleep 1
done

# 1. Measure podnet quick tunnel.
"$podnet" tunnel --url "http://127.0.0.1:$port" --json --no-verify \
    > "$dir/podnet.out" 2> "$dir/podnet.err" &
podnet_pid=$!
pids="$origin_pid $podnet_pid"

i=0
until grep -q '"event":"ready"' "$dir/podnet.out" 2> /dev/null; do
    i=$((i + 1))
    if ! kill -0 "$podnet_pid" 2> /dev/null; then
        check_quota "$dir/podnet.err"
        show_tail "podnet tunnel" "$dir/podnet.err"
        rc=0
        wait "$podnet_pid" || rc=$?
        echo "podnet tunnel stopped before ready (exit $rc)" >&2
        exit 1
    fi
    if [ "$i" -gt 90 ]; then
        check_quota "$dir/podnet.err"
        show_tail "podnet tunnel" "$dir/podnet.err"
        echo "podnet tunnel is not ready after 90 s" >&2
        exit 1
    fi
    sleep 1
done

podnet_url=$(python3 -I -c '
import json, sys
with open(sys.argv[1], encoding="utf-8") as fh:
    for line in fh:
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except ValueError:
            continue
        if obj.get("event") == "ready" and obj.get("url"):
            print(obj["url"].rstrip("/"))
            break
' "$dir/podnet.out")
if [ -z "$podnet_url" ]; then
    show_tail "podnet tunnel" "$dir/podnet.err"
    echo "podnet tunnel printed no ready URL" >&2
    exit 1
fi
podnet_host=${podnet_url#https://}
echo "::add-mask::$podnet_host"

sh "$here/live-wait-name.sh" "$podnet_host" 240 || { show_tail "podnet tunnel" "$dir/podnet.err"; exit 1; }
wait_http_200 "$podnet_url" "podnet tunnel" "$dir/podnet.err"

bench_rc=0
"$podnet" bench "$podnet_url/" --connections "$conns" --duration "$dur" --json \
    > "$dir/podnet-bench.json" 2> "$dir/podnet-bench.err" || bench_rc=$?
if [ "$bench_rc" -ne 0 ]; then
    show_tail "podnet bench (podnet)" "$dir/podnet-bench.err"
    show_tail "podnet tunnel" "$dir/podnet.err"
    echo "podnet bench failed against podnet tunnel (exit $bench_rc)" >&2
    exit 1
fi

kill "$podnet_pid" 2> /dev/null || true
wait "$podnet_pid" 2> /dev/null || true
pids="$origin_pid"

# 2. Measure cloudflared quick tunnel. Retry transient quick-tunnel API timeouts.
cf_url=""
cf_pid=""
for attempt in 1 2 3; do
    : > "$dir/cloudflared.err"
    "$cloudflared" tunnel --no-autoupdate --url "http://127.0.0.1:$port" \
        > "$dir/cloudflared.err" 2>&1 &
    cf_pid=$!
    pids="$origin_pid $cf_pid"

    i=0
    while [ -z "$cf_url" ]; do
        cf_url=$(grep -oE 'https://[a-z0-9-]+\.trycloudflare\.com' "$dir/cloudflared.err" 2> /dev/null |
            grep -Fxv 'https://api.trycloudflare.com' | head -1 || true)
        if [ -n "$cf_url" ]; then
            break
        fi
        i=$((i + 1))
        if ! kill -0 "$cf_pid" 2> /dev/null; then
            check_quota "$dir/cloudflared.err"
            rc=0
            wait "$cf_pid" || rc=$?
            pids="$origin_pid"
            if [ "$attempt" -lt 3 ]; then
                echo "cloudflared tunnel attempt $attempt stopped before ready (exit $rc); retry in 5 s" >&2
                sleep 5
                break
            fi
            show_tail "cloudflared tunnel" "$dir/cloudflared.err"
            echo "cloudflared tunnel stopped before ready (exit $rc)" >&2
            exit 1
        fi
        if [ "$i" -gt 90 ]; then
            check_quota "$dir/cloudflared.err"
            kill "$cf_pid" 2> /dev/null || true
            wait "$cf_pid" 2> /dev/null || true
            pids="$origin_pid"
            if [ "$attempt" -lt 3 ]; then
                echo "cloudflared tunnel attempt $attempt timed out after 90 s; retry in 5 s" >&2
                sleep 5
                break
            fi
            show_tail "cloudflared tunnel" "$dir/cloudflared.err"
            echo "cloudflared tunnel is not ready after 90 s" >&2
            exit 1
        fi
        sleep 1
    done
    if [ -n "$cf_url" ]; then
        break
    fi
done

cf_host=${cf_url#https://}
echo "::add-mask::$cf_host"

sh "$here/live-wait-name.sh" "$cf_host" 240 || { show_tail "cloudflared tunnel" "$dir/cloudflared.err"; exit 1; }
wait_http_200 "$cf_url" "cloudflared tunnel" "$dir/cloudflared.err"

bench_rc=0
"$podnet" bench "$cf_url/" --connections "$conns" --duration "$dur" --json \
    > "$dir/cloudflared-bench.json" 2> "$dir/cloudflared-bench.err" || bench_rc=$?
if [ "$bench_rc" -ne 0 ]; then
    show_tail "podnet bench (cloudflared)" "$dir/cloudflared-bench.err"
    show_tail "cloudflared tunnel" "$dir/cloudflared.err"
    echo "podnet bench failed against cloudflared tunnel (exit $bench_rc)" >&2
    exit 1
fi

kill "$cf_pid" 2> /dev/null || true
wait "$cf_pid" 2> /dev/null || true
pids="$origin_pid"

# 3. Validate both JSON documents and print a masked comparison table.
python3 -I - "$dir/podnet-bench.json" "$dir/cloudflared-bench.json" "$dur" "$conns" << 'EOF'
import json
import re
import sys

MASK_RE = re.compile(r"[a-z0-9-]+\.trycloudflare\.com")


def mask(text: str) -> str:
    return MASK_RE.sub("<tunnel>.trycloudflare.com", text)


def load(path: str) -> dict:
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def fmt(val) -> str:
    if isinstance(val, (int, float)):
        return f"{val:.2f}"
    return "-"


pod_doc = load(sys.argv[1])
cf_doc = load(sys.argv[2])
dur = sys.argv[3]
conns = sys.argv[4]

rows = [("podnet", pod_doc), ("cloudflared", cf_doc)]
print(f"bench compare (duration={dur}s, connections={conns}):")
print(f"{'tunnel':<12} {'requests':>8} {'rps':>8} {'p50_ms':>8} {'p95_ms':>8} {'p99_ms':>8} {'errors':>8}")

faults = []
for name, doc in rows:
    lat = doc.get("latency_ms") or {}
    reqs = doc.get("requests")
    rps = doc.get("rate")
    errs = doc.get("errors")
    print(
        f"{name:<12} {str(reqs):>8} {fmt(rps):>8} "
        f"{fmt(lat.get('p50')):>8} {fmt(lat.get('p95')):>8} {fmt(lat.get('p99')):>8} {str(errs):>8}"
    )
    schema = doc.get("schema")
    if schema != 1 or isinstance(schema, bool):
        faults.append(f"{name}: expected schema 1, got {schema!r}")
    if not isinstance(reqs, int) or isinstance(reqs, bool) or reqs <= 0:
        faults.append(f"{name}: expected requests > 0, got {reqs!r}")
    if not isinstance(errs, int) or isinstance(errs, bool) or errs != 0:
        first_err = mask(str(doc.get("first_error") or ""))
        faults.append(f"{name}: expected errors == 0, got {errs!r} ({first_err})")

if faults:
    sys.stdout.flush()
    for fault in faults:
        print(f"bench compare fault: {fault}", file=sys.stderr)
    sys.exit(1)
EOF
