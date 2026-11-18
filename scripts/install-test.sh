#!/bin/sh
# SPDX-License-Identifier: 0BSD
# The test of scripts/install.sh in one shell, with no network: a fake
# release on the loopback (an HTTP server of python3) and in a file://
# directory, a fake cosign, and a copy of the installer with the pins of the
# fake release (scripts/install/pins.py stamp).
#
#   sh scripts/install-test.sh [SHELL]
#
# SHELL is the shell under test, for example "dash", "bash --posix" or
# "busybox sh" (default: sh). PODNET_TEST_BIN is the binary of the fake
# release, with PODNET_TEST_VERSION its version (default: a program of sh
# that prints "podnet 9.9.9 (test)"). Exit 0 when each case passes, 1 at
# the first fault.
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
shell=${1:-sh}
work=$(mktemp -d) || exit 1
pid=''
cleanup() {
    [ -z "$pid" ] || kill "$pid" 2> /dev/null
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
n=0
ok() { n=$((n + 1)); echo "ok $n $shell: $*"; }
fail() {
    echo "FAIL $shell: $*" >&2
    [ ! -f "$work/out" ] || sed 's/^/  | /' "$work/out" >&2
    exit 1
}
expect() { grep -q -e "$1" "$work/out" || fail "$2: no line has '$1'"; }
sum() { sha256sum < "$1" | cut -d ' ' -f 1; }
stat_of() { ls -li "$1" | awk '{ print $1 }'; cksum < "$1"; find "$1" -newer "$work/mark" | wc -l; }
leftover() { for f in "$1"/.podnet-install.*; do [ ! -e "$f" ] || fail "$2: $f is left"; done; }

case $(uname -m) in
    x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
    aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
    *) echo "skip: the test has no target for $(uname -m)"; exit 0 ;;
esac
asset=podnet-$target
version=${PODNET_TEST_VERSION:-9.9.9}
rel=$work/rel
mkdir -p "$rel" "$work/fake" "$work/home"
if [ -n "${PODNET_TEST_BIN:-}" ]; then
    cp "$PODNET_TEST_BIN" "$rel/$asset"
else
    printf '#!/bin/sh\necho "podnet %s (test)"\n' "$version" > "$rel/$asset"
fi
chmod 0755 "$rel/$asset"
(cd "$rel" && sha256sum "$asset" > SHA256SUMS)
echo '{}' > "$rel/SHA256SUMS.sigstore.json"
# A fake cosign that logs its call and exits COSIGN_RC.
printf '#!/bin/sh\necho "$*" >> %s/cosign.log\nexit ${COSIGN_RC:-0}\n' "$work" > "$work/fake/cosign"
chmod 0755 "$work/fake/cosign"

# The fake release on the loopback: each request is a line of the log.
cat > "$work/serve.py" << 'EOF'
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
root, portfile, log = sys.argv[1:4]
class H(SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=root, **k)
    def log_message(self, fmt, *args):
        with open(log, "a") as f:
            f.write(self.path + "\n")
s = ThreadingHTTPServer(("127.0.0.1", 0), H)
Path(portfile).write_text(str(s.server_address[1]))
s.serve_forever()
EOF
python3 -I "$work/serve.py" "$rel" "$work/port" "$work/log" &
pid=$!
i=0
while [ ! -s "$work/port" ]; do
    i=$((i + 1))
    [ "$i" -lt 100 ] || fail "the server did not start"
    sleep 0.1
done
mirror=http://127.0.0.1:$(cat "$work/port")

# Run the installer FILE with the shell under test; the output is in out.
run() {
    f=$1
    shift
    : > "$work/log"
    # shellcheck disable=SC2086 # the shell is a list of words.
    env -i PATH="$work/fake:$PATH" HOME="$work/home" PODNET_VERSION="v$version" PODNET_MIRROR="$mirror" \
        "$@" $shell "$f" > "$work/out" 2>&1
}
runargs() {
    f=$1
    shift
    : > "$work/log"
    # shellcheck disable=SC2086 # the shell is a list of words.
    env -i PATH="$work/fake:$PATH" HOME="$work/home" PODNET_MIRROR="$mirror" $shell "$f" "$@" > "$work/out" 2>&1
}
tree=$root/scripts/install.sh
d=$work/bin

# 1. The copy of the tree: the first run installs the binary of the pin.
run "$tree" PODNET_INSTALL_DIR="$d" || fail "first run: exit $?"
expect "installed $d/podnet" "first run"
expect "podnet $version" "first run"
expect "release: v$version" "first run"
# The core checks the signature for the copy of the tree too, and does not
# compare this file with podnet-install.sh of the release.
expect "the signature of SHA256SUMS: ok (cosign, https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/v$version)" "first run"
expect "this installer holds no pins of its own" "first run"
[ "$(sum "$d/podnet")" = "$(sum "$rel/$asset")" ] || fail "first run: the binary is not the file of the release"
leftover "$d" "first run"
ok "the first run installs the binary"

# 2. A second run changes nothing and says so: it gets SHA256SUMS only.
touch "$work/mark"
sleep 1
before=$(stat_of "$d/podnet")
run "$tree" PODNET_INSTALL_DIR="$d" || fail "second run: exit $?"
expect "is already installed" "second run"
[ "$(stat_of "$d/podnet")" = "$before" ] || fail "second run: the binary changed (inode, bytes or mtime)"
grep -q "/$asset\$" "$work/log" && fail "second run: the binary was downloaded"
grep -q "/SHA256SUMS\$" "$work/log" || fail "second run: no request of SHA256SUMS"
leftover "$d" "second run"
ok "a second run changes nothing and says so"

# 3. --dry-run makes no file and downloads no binary.
runargs "$tree" --dry-run --prefix "$work/dry" --version "v$version" || fail "dry run: exit $?"
expect "dry run: no file was changed" "dry run"
[ ! -e "$work/dry" ] || fail "dry run: the directory was made"
grep -q "/$asset\$" "$work/log" && fail "dry run: the binary was downloaded"
ok "a dry run changes no file"

# 3b. One proxy in two variables gives one proxy line (GitHub #645, T-469).
: > "$work/log"
# shellcheck disable=SC2086 # the shell is a list of words.
env -i PATH="$work/fake:$PATH" HOME="$work/home" PODNET_MIRROR="$mirror" \
    https_proxy=http://proxy.invalid:3128 http_proxy=http://proxy.invalid:3128 no_proxy=127.0.0.1 \
    $shell "$tree" --dry-run --prefix "$work/dry" --version "v$version" > "$work/out" 2>&1 \
    || fail "one proxy: exit $?"
[ "$(grep -c 'proxy: http://proxy.invalid:3128' "$work/out")" = 1 ] || fail "one proxy: not one proxy line"
ok "one proxy in two variables gives one proxy line"

# 4. A planted defect: a wrong SHA-256 in SHA256SUMS fails, and the old
# binary stays.
cp "$rel/SHA256SUMS" "$work/sums.good"
printf '%s  %s\n' "$(printf '0%.0s' $(seq 64))" "$asset" > "$rel/SHA256SUMS"
printf 'old\n' > "$work/old"
mkdir -p "$work/d4"
cp "$work/old" "$work/d4/podnet"
run "$tree" PODNET_INSTALL_DIR="$work/d4" && fail "wrong SHA-256: exit 0"
expect "the SHA-256 is" "wrong SHA-256"
cmp -s "$work/old" "$work/d4/podnet" || fail "wrong SHA-256: the old binary changed"
leftover "$work/d4" "wrong SHA-256"
cp "$work/sums.good" "$rel/SHA256SUMS"
ok "a wrong SHA-256 fails and keeps the old binary"

# 5. A failed cosign check stops the run.
run "$tree" PODNET_INSTALL_DIR="$work/d5" COSIGN_RC=1 && fail "bad signature: exit 0"
expect "signature of SHA256SUMS is not valid" "bad signature"
[ ! -e "$work/d5/podnet" ] || fail "bad signature: a binary was installed"
ok "a bad signature fails"

# 6. A file:// mirror and a binary in place that is old: the binary is
# replaced.
run "$tree" PODNET_INSTALL_DIR="$work/d4" PODNET_MIRROR="file://$rel" || fail "file mirror: exit $?"
expect "installed $work/d4/podnet" "file mirror"
[ "$(sum "$work/d4/podnet")" = "$(sum "$rel/$asset")" ] || fail "file mirror: not the new binary"
ok "a file:// mirror replaces an old binary"

# 7. The copy of a release, with the pins (pins.py stamp).
stamped=$work/podnet-install.sh
python3 -I "$root/scripts/install/pins.py" stamp "$tree" --version "$version" --assets "$rel" --out "$stamped" > /dev/null ||
    fail "pins.py stamp failed"
: > "$work/cosign.log"
runargs "$stamped" --prefix "$work/d7" || fail "stamped: exit $?"
expect "installed $work/d7/podnet" "stamped"
# The core checks the signature of SHA256SUMS for the copy of a release.
expect "the signature of SHA256SUMS: ok (cosign, https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/v$version)" "stamped"
grep -q -e "--certificate-identity https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/v$version" "$work/cosign.log" ||
    fail "stamped: cosign did not get the identity of release.yml"
runargs "$stamped" --prefix "$work/d7" || fail "stamped second run: exit $?"
expect "is already installed" "stamped second run"
[ -s "$work/log" ] && fail "stamped second run: a request: $(cat "$work/log")"
runargs "$stamped" --prefix "$work/d7" --version 1.2.3 && fail "stamped: another version: exit 0"
expect "holds the pins of $version" "stamped: another version"
ok "the copy of a release installs, and a second run downloads nothing"

# 8. The copy of a release, offline, from the directory of the release.
env -i PATH="$PATH" HOME="$work/home" $shell "$stamped" --offline "$rel" --prefix "$work/d8" > "$work/out" 2>&1 ||
    fail "offline: exit $?"
expect "installed $work/d8/podnet" "offline"
ok "the copy of a release installs offline"

# 9. A planted defect in the mirror: a changed binary fails against the pin
# of the copy of a release.
cp "$rel/$asset" "$work/asset.good"
printf '#!/bin/sh\necho "podnet %s (changed)"\n' "$version" > "$rel/$asset"
runargs "$stamped" --prefix "$work/d9" && fail "changed binary: exit 0"
expect "the SHA-256 is" "changed binary"
[ ! -e "$work/d9/podnet" ] || fail "changed binary: a binary was installed"
cp "$work/asset.good" "$rel/$asset"
ok "a changed binary fails against the pin"

# 10. A binary that does not run keeps the old binary.
printf '#!/bin/sh\nexit 1\n' > "$rel/$asset"
(cd "$rel" && sha256sum "$asset" > SHA256SUMS)
mkdir -p "$work/d10"
cp "$work/old" "$work/d10/podnet"
run "$tree" PODNET_INSTALL_DIR="$work/d10" && fail "no run: exit 0"
expect "does not run on this host" "no run"
cmp -s "$work/old" "$work/d10/podnet" || fail "no run: the old binary changed"
leftover "$work/d10" "no run"
cp "$work/asset.good" "$rel/$asset"
cp "$work/sums.good" "$rel/SHA256SUMS"
ok "a binary that does not run keeps the old binary"

# 11. A hostile version stops the run before a request.
run "$tree" PODNET_INSTALL_DIR="$work/d11" PODNET_VERSION='v1/../x' && fail "hostile version: exit 0"
expect "not a release tag" "hostile version"
[ -s "$work/log" ] && fail "hostile version: a request"
ok "a hostile version stops before a request"

# 12. PODNET_COSIGN=none: no signature check, and the run says so. A bad
# signature stops the copy of a release too.
: > "$work/cosign.log"
run "$tree" PODNET_INSTALL_DIR="$work/d12" PODNET_COSIGN=none COSIGN_RC=1 || fail "cosign none: exit $?"
expect "PODNET_COSIGN=none: the signature of SHA256SUMS is not checked" "cosign none"
[ -s "$work/cosign.log" ] && fail "cosign none: cosign ran"
# shellcheck disable=SC2086 # the shell is a list of words.
env -i PATH="$work/fake:$PATH" HOME="$work/home" PODNET_MIRROR="$mirror" COSIGN_RC=1 \
    $shell "$stamped" --prefix "$work/d12b" > "$work/out" 2>&1 && fail "stamped, bad signature: exit 0"
expect "signature of SHA256SUMS is not valid" "stamped, bad signature"
[ ! -e "$work/d12b/podnet" ] || fail "stamped, bad signature: a binary was installed"
ok "PODNET_COSIGN=none skips the signature; a bad signature stops the copy of a release"
echo "$n cases passed in $shell"
