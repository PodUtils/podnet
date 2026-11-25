#!/bin/sh
# Call the same startup as Codex and keep the session available for recovery.
root=${CLAUDE_PROJECT_DIR:-"$(dirname -- "$0")/.."}
root=$(CDPATH='' cd -- "$root" && pwd -P) || exit 0
if ! sh "$root/scripts/session-start.sh"; then
    echo "session-start: run sh scripts/session-start.sh in the checkout. See TODO/RESUME.md." >&2
fi
if [ "${CLAUDE_CODE_REMOTE:-}" = true ] && [ -n "${CLAUDE_ENV_FILE:-}" ]; then
    quoted_root=$(printf '%s' "$root" | sed "s/'/'\\\\''/g")
    {
        printf "podnet_project_directory='%s'\n" "$quoted_root"
        cat <<'ENV'
podnet_previous_directory=$PWD
if CDPATH='' cd -- "$podnet_project_directory"; then
    . ./scripts/cloud-env.sh
    cd "$podnet_previous_directory"
fi
unset podnet_project_directory podnet_previous_directory
ENV
    } >>"$CLAUDE_ENV_FILE"
fi
