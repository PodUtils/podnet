# shellcheck shell=sh
# docs/harness.md: `run.sh` of the archive. The set-up writes the archive of
# the fixtures of the repository, and unzips it in the work directory.

setup() {
    podnet harness --data "$DOC_ROOT/tests/data/harness" --archive harness.zip > /dev/null
    python3 -I -m zipfile -e harness.zip .
}
