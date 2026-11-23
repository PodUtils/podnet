#!/bin/sh
# Run the reference-server gates of PodUtils/podnet#486, with the raw output
# of each in $PODNET_EVIDENCE_DIR (default .work/evidence). The gate is one
# command:
#
#   JUICITY_SERVER=/path/juicity-server \
#   TUIC_SERVER=/path/tuic-server sh scripts/ci/reference-tests.sh
#
# It exits nonzero when a required binary is absent, so a missing server
# cannot turn into a silent pass. The stock quic-go client needs `go` and
# quic-go v0.59.0 in the module cache.
set -eu
cd "$(dirname "$0")/../.."
evidence=${PODNET_EVIDENCE_DIR:-.work/evidence}
mkdir -p "$evidence"
rc=0
for var in JUICITY_SERVER TUIC_SERVER; do
    eval "value=\${$var:-}"
    if [ -z "$value" ]; then
        echo "reference-tests: $var is not set; it must name the reference binary" >&2
        rc=1
    elif [ ! -x "$value" ]; then
        echo "reference-tests: $var=$value is no executable" >&2
        rc=1
    fi
done
if ! command -v go >/dev/null 2>&1; then
    echo "reference-tests: go is not on PATH; the stock quic-go client cannot run" >&2
    rc=1
fi
[ "$rc" -eq 0 ] || exit 1

if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    export CARGO_TARGET_DIR
fi
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_INCREMENTAL=0
export TMPDIR=${TMPDIR:-/tmp}

run() {
    name=$1
    shift
    out="$evidence/$name.out"
    echo "reference-tests: $name -> $out"
    if ! "$@" >"$out" 2>&1; then
        echo "reference-tests: $name failed:" >&2
        tail -80 "$out" >&2
        rc=1
        return
    fi
    cat "$out"
    if grep -iE "^skip(ping)?:" "$out" >/dev/null; then
        echo "reference-tests: $name skipped; the gate did not run" >&2
        rc=1
    fi
}

run juicity cargo test -p podnet --test quic_juicity_reference -- --nocapture
run tuic cargo test -p podnet --test quic_tuic_reference -- --nocapture
run h3-stock cargo test -p podnet --test quic_h3_stock -- --nocapture
exit "$rc"
