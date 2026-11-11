#!/bin/sh
# Run, as root, the netns tests that skip for a user on a GitHub runner (no
# user namespace there): the veth wire of the netns backend
# (podnet-netns --test veth) and the kernel mode of the router
# (podnet --lib router::kernel). Root needs no user namespace. The script
# fails when a test fails or prints a skip line (T-250).
#
#   sh scripts/ci/root-netns-tests.sh
set -eu
cd "$(dirname "$0")/../.."
log=${TMPDIR:-/tmp}/root-netns-tests
mkdir -p "$log"
# Build as this user; run as root. jq reads the paths of the test binaries.
bins=$(cargo test --locked --no-run --message-format=json -p podnet-netns --test veth -p podnet --lib 2> "$log/build.err" |
    jq -r 'select(.reason == "compiler-artifact" and .profile.test == true) | .executable // empty')
[ -n "$bins" ] || { cat "$log/build.err" >&2; echo "root-netns-tests: no test binary" >&2; exit 1; }
rc=0
for bin in $bins; do
    case $bin in
        */veth-*) filter=veth_wire ;;
        */podnet-*) filter=router::kernel ;;
        *) continue ;;
    esac
    out="$log/$(basename "$bin").out"
    # The log file belongs to this user, so the redirect is not under sudo.
    # shellcheck disable=SC2024
    if ! sudo -E env "PATH=$PATH" "$bin" "$filter" --nocapture --test-threads 1 > "$out" 2>&1; then
        echo "root-netns-tests: $filter failed:" >&2
        tail -40 "$out" >&2
        rc=1
        continue
    fi
    grep -E "test result" "$out"
    if grep -iE "^skip(ping)?:" "$out" > "$log/skips"; then
        echo "root-netns-tests: $filter skipped as root:" >&2
        cat "$log/skips" >&2
        rc=1
    fi
done
exit "$rc"
