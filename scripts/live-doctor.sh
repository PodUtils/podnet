#!/bin/sh
# T-587: run `podnet doctor --json` into OUT until the leg quic+warp is ok,
# at most three times with 15 s between the runs. WARP's door shows another
# certificate at times (T-406), so one run can see the leg as not ok where
# the run before saw it ok. Each run that is not ok prints the legs with
# their reasons. Exit 0 when a run has the leg ok, else 1.
#
#   sh scripts/live-doctor.sh BINARY OUT [ARGS of doctor]
set -u
bin=$1
out=$2
shift 2
n=1
while :; do
    "$bin" doctor "$@" --json > "$out" || true
    if jq -e '.legs[] | select(.leg == "quic+warp") | .status == "ok"' "$out" > /dev/null; then
        exit 0
    fi
    echo "live-doctor: run $n: the leg quic+warp is not ok:"
    jq -c '.legs[] | {leg, status, reason}' "$out" || cat "$out"
    if [ "$n" -ge 3 ]; then
        exit 1
    fi
    n=$((n + 1))
    sleep 15
done
