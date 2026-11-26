# shellcheck shell=sh
# README.md: podnet update. The update replaces the file of the binary:
# the set-up puts a copy of the binary first on PATH, so that the binary
# of the run does not change.

setup() {
    mkdir -p bin
    cp "$(command -v podnet)" bin/podnet
    PATH="$PWD/bin:$PATH"
    export PATH
}

check() {
    bin/podnet --version
}
