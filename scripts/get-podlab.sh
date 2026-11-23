#!/bin/sh
# Get the release binary podlab of PodUtils/podcrates (T-358, T-514).
#
# Usage: sh scripts/get-podlab.sh VERSION [DIR]
#
# VERSION is a tag of podcrates, vX.Y.Z (v0.58.0 or later: earlier tags
# have no podlab asset). DIR must not exist (default: .work/podlab-VERSION
# in the current directory). The script downloads SHA256SUMS and
# podlab-ARCH-unknown-linux-musl of this machine into DIR, checks the
# asset against SHA256SUMS, and makes it executable. Give DIR to
# scripts/lab-lane.sh as PODLAB. The release of podcrates has no signature
# of its assets: the check is the SHA-256 of SHA256SUMS only.
#
# Not each tag of podcrates has a GitHub release. When the release of
# VERSION does not exist or does not have the asset (a download gives
# 404), the script reads the list
# of releases of podcrates (the GitHub REST API) and takes the newest
# release at or below VERSION that has SHA256SUMS and the asset. It writes
# one line that names VERSION and the release that it took.
#
# The environment:
#   GET_PODLAB_API  the base of the REST API (default https://api.github.com).
#   GH_TOKEN        a token for the REST API; sent to api.github.com only.
#
# Exit codes: 0 done; 1 a download, the list of releases or the check
# fails, or no release at or below VERSION has the asset; 64 usage.
# It needs curl, sha256sum and python3.

set -eu

usage() {
    echo "usage: sh scripts/get-podlab.sh VERSION [DIR]" >&2
    exit 64
}
test $# -ge 1 && test $# -le 2 || usage
version=$1
case $version in
    v[0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "get-podlab: $version is not a tag vX.Y.Z" >&2; exit 64 ;;
esac
dir=${2:-$PWD/.work/podlab-$version}
if test -e "$dir"; then
    echo "get-podlab: $dir exists; give a new directory" >&2
    exit 64
fi
asset="podlab-$(uname -m)-unknown-linux-musl"
api=${GET_PODLAB_API:-https://api.github.com}
ok=0
mkdir -p "$dir"
trap 'test "$ok" = 1 || rm -rf "$dir"' EXIT

die() {
    echo "get-podlab: $*" >&2
    exit 1
}

# fetch URL FILE: the HTTP status in $code. Retry on each status but 200
# and 404 (a 404 is final).
fetch() {
    n=0
    while :; do
        code=$(curl -qsSL --proto '=https' -o "$2" -w '%{http_code}' "$1") || code=000
        case $code in 200 | 404) return 0 ;; esac
        n=$((n + 1))
        test $n -ge 4 && return 0
        sleep 2
    done
}

# The newest release at or below VERSION with SHA256SUMS and the asset,
# from the list of releases on stdin.
pick() {
    python3 -I -c '
import json, re, sys
want, asset = sys.argv[1], sys.argv[2]
def ver(tag):
    m = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", tag or "")
    return tuple(map(int, m.groups())) if m else None
top = ver(want)
best = None
for r in json.load(sys.stdin):
    v = ver(r.get("tag_name"))
    if v is None or v > top or r.get("draft") or r.get("prerelease"):
        continue
    names = {a.get("name") for a in r.get("assets", [])}
    if "SHA256SUMS" in names and asset in names and (best is None or v > best[0]):
        best = (v, r["tag_name"])
if best:
    print(best[1])
' "$version" "$asset"
}

# get TAG: SHA256SUMS and the asset of the release TAG into DIR. The
# status of the last download in $code.
get() {
    base="https://github.com/PodUtils/podcrates/releases/download/$1"
    fetch "$base/SHA256SUMS" "$dir/SHA256SUMS"
    test "$code" = 200 || return 0
    fetch "$base/$asset" "$dir/$asset"
}

get "$version"
if test "$code" = 404; then
    list="$dir/releases.json"
    auth=
    case $api in https://api.github.com) auth=${GH_TOKEN:-} ;; esac
    n=0
    while :; do
        lcode=$(curl -qsL --proto '=https' -o "$list" -w '%{http_code}' \
            -H 'Accept: application/vnd.github+json' \
            ${auth:+-H "Authorization: Bearer $auth"} \
            "$api/repos/PodUtils/podcrates/releases?per_page=100") || lcode=000
        test "$lcode" = 200 && break
        n=$((n + 1))
        test $n -ge 4 && die "podcrates $version has no release with $asset, and the list of releases at $api does not download (HTTP $lcode)"
        sleep 2
    done
    taken=$(pick < "$list") || die "cannot read the list of releases of podcrates"
    rm -f "$list" "$dir/SHA256SUMS" "$dir/$asset"
    test -n "$taken" || die "podcrates $version has no release with $asset, and no release at or below $version has it"
    echo "get-podlab: podcrates $version has no release with $asset; took release $taken"
    get "$taken"
fi
test "$code" = 200 || die "cannot download from $base (HTTP $code)"
(cd "$dir" && grep " $asset\$" SHA256SUMS | sha256sum -c -) || die "$asset does not match SHA256SUMS of $base"
chmod 755 "$dir/$asset"
"$dir/$asset" --version
ok=1
echo "PODLAB: $dir"
