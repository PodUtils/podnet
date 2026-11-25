#!/bin/sh
# Call the same startup as Codex and keep the session available for recovery.
root=${CLAUDE_PROJECT_DIR:-"$(dirname -- "$0")/.."}
if ! sh "$root/scripts/session-start.sh"; then
    echo "session-start: run sh scripts/session-start.sh in the checkout. See TODO/RESUME.md." >&2
fi
