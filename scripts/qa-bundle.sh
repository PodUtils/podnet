#!/bin/sh
# Give a tester the candidate binary, its manual, and their checksums.
set -eu
umask 022

if [ "$#" -ne 2 ]; then
    echo "usage: qa-bundle.sh BINARY OUTPUT_DIR" >&2
    exit 2
fi
if [ ! -f "$1" ] || [ ! -x "$1" ]; then
    printf 'qa-bundle: %s is not an executable file\n' "$1" >&2
    exit 2
fi

parent=$(CDPATH='' cd -- "$(dirname -- "$2")" && pwd -P)
output=$parent/$(basename -- "$2")
if [ -e "$output" ] || [ -L "$output" ]; then
    printf 'qa-bundle: %s already exists\n' "$output" >&2
    exit 2
fi

stage=$(mktemp -d -- "$parent/.podnet-qa.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
trap 'exit 1' HUP INT TERM
install -m 0755 -- "$1" "$stage/podnet"
if ! TERM=dumb timeout --kill-after=5 30 "$stage/podnet" man --no-pager </dev/null >"$stage/podnet.man" ||
   [ ! -s "$stage/podnet.man" ]; then
    echo "qa-bundle: the candidate did not produce a manual" >&2
    exit 1
fi
(
    cd "$stage"
    sha256sum podnet podnet.man >SHA256SUMS
)
chmod 0755 -- "$stage"
mv -nT -- "$stage" "$output"
if [ -d "$stage" ]; then
    printf 'qa-bundle: %s already exists\n' "$output" >&2
    exit 2
fi
printf 'qa-bundle: %s\n' "$output"
