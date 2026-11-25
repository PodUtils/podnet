#!/bin/bash
# T-589: the shell of each step of the job live (defaults.run.shell). It
# runs the script of the step as `bash -e` does, with the output to the log
# of the step. The quick-tunnel service refuses a busy address for an hour
# (HTTP 429, error code 1015), and podnet then exits 75 as documented: a
# step that fails with that line in its output gives a warning, sets
# LIVE_QUICK_REFUSED=1 for the later steps, and passes. Each other failure
# keeps its exit code.
#
#   shell: bash scripts/live-step.sh {0}
set -u
script=$1
log=$(mktemp)
# The output goes to a file, and tail streams it until the step ends: a
# process that the step leaves in the background (a stand-in proxy) keeps
# no pipe open, and does not hold the step.
bash --noprofile --norc -eo pipefail "$script" > "$log" 2>&1 &
step=$!
tail -n +1 -f --pid="$step" "$log"
wait "$step"
rc=$?
if [ "$rc" -ne 0 ] && grep -q 'quick-tunnel service returned HTTP 429' "$log"; then
    echo "::warning::the quick-tunnel service refuses this runner for now (HTTP 429); the step is not measured (T-589)"
    if [ -n "${GITHUB_ENV:-}" ]; then
        echo "LIVE_QUICK_REFUSED=1" >> "$GITHUB_ENV"
    fi
    rc=0
fi
rm -f "$log"
exit "$rc"
