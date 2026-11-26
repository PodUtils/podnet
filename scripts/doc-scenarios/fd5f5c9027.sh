# shellcheck shell=sh
# docs/harness.md: the usage lines. The set-up copies the fixtures of the
# repository to the directory ./DIR; the archive goes to the file ./FILE.

setup() {
    cp -R "$DOC_ROOT/tests/data/harness" DIR
}

check() {
    python3 -I -m zipfile -l FILE | grep -q 'run.sh'
}
