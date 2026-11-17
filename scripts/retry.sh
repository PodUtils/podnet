#!/bin/sh
# Run COMMAND up to 5 times, 10, 20, 40 and 80 s apart. A fault of the
# network (DNS, a mirror) that lasts less than 150 s does not stop a job.
#
#   sh scripts/retry.sh COMMAND [ARGUMENT...]
set -u
[ "$#" -gt 0 ] || { echo "usage: retry.sh COMMAND [ARGUMENT...]" >&2; exit 2; }
delay=10
for try in 1 2 3 4 5; do
    "$@" && exit 0
    rc=$?
    [ "$try" -eq 5 ] && { echo "retry.sh: $1 failed 5 times (exit $rc)" >&2; exit "$rc"; }
    echo "retry.sh: $1 failed (exit $rc); try $((try + 1)) in $delay s" >&2
    sleep "$delay"
    delay=$((delay * 2))
done
