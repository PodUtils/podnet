#!/bin/sh
# The checks of CLAUDE.md before each push: fmt, clippy and the tests, the
# check of the work record (cargo todo check), the size of the Rust files
# (scripts/check-size.py), the layers of the modules
# (scripts/check-layers.py), the docs (cargo doc, as CI) and the class of each
# sh block of the documents (scripts/doc-commands.py --check), and the man page
# with groff and mandoc when both exist, the installer core
# (scripts/check-install.py), the rules of the workflows, and the
# shell scripts with shellcheck when it exists (as CI). Print each
# exit code, and the first faults; exit 1 on a fault. With the argument notest, the tests are skipped.
#
#   sh scripts/agent/check.sh [notest]
set -u
cd "$(dirname "$0")/../.." || exit 1
log=${TMPDIR:-/tmp}/podnet-check
mkdir -p "$log"
rc=0
cargo fmt --all --check > "$log/fmt.log" 2>&1; f=$?
echo "fmt $f"; [ "$f" -eq 0 ] || { rc=1; head -20 "$log/fmt.log"; }
cargo clippy --workspace --all-targets -- -D warnings > "$log/clippy.log" 2>&1; c=$?
echo "clippy $c"; [ "$c" -eq 0 ] || { rc=1; grep -E "^(error|warning)" -A6 "$log/clippy.log" | head -40; }
cargo run -q -p podnet-todo -- check > "$log/record.log" 2>&1; r=$?
echo "record $r"; [ "$r" -eq 0 ] || { rc=1; tail -20 "$log/record.log"; }
python3 -I scripts/check-size.py > "$log/size.log" 2>&1; s=$?
echo "size $s"; [ "$s" -eq 0 ] || { rc=1; grep -v "^check-size:" "$log/size.log" | head -20; }
RUSTDOCFLAGS="-D warnings" cargo doc -q --locked --no-deps --workspace > "$log/doc.log" 2>&1; d=$?
echo "doc $d"; [ "$d" -eq 0 ] || { rc=1; grep -E "^(error|warning)" -A6 "$log/doc.log" | head -20; }
python3 -I scripts/check-layers.py > "$log/layers.log" 2>&1; l=$?
echo "layers $l"; [ "$l" -eq 0 ] || { rc=1; grep -v "^check-layers:" "$log/layers.log" | head -20; }
python3 -I scripts/check-claims.py > "$log/claims.log" 2>&1; q=$?
echo "claims $q"; [ "$q" -eq 0 ] || { rc=1; head -20 "$log/claims.log"; }
python3 -I scripts/doc-commands.py --check > "$log/doccmd.log" 2>&1; k=$?
echo "doc-commands $k"; [ "$k" -eq 0 ] || { rc=1; head -20 "$log/doccmd.log"; }
python3 -I scripts/gate-scenarios.py --check > "$log/scenarios.log" 2>&1; g=$?
echo "scenarios $g"; [ "$g" -eq 0 ] || { rc=1; head -20 "$log/scenarios.log"; }
# The man page, as the job "test" of CI renders it: groff with each warning
# (-ww) and mandoc lint. When groff or mandoc is not on this host, the step
# says "skipped": CI still runs it.
if command -v groff > /dev/null 2>&1 && command -v mandoc > /dev/null 2>&1; then
    m=0
    : > "$log/man.log"
    if ! cargo run --locked -q --bin podnet -- man --roff > "$log/podnet.1" 2> "$log/man-cargo.log"; then
        m=1
        cat "$log/man-cargo.log" >> "$log/man.log"
    fi
    groff -man -Tutf8 -ww -z "$log/podnet.1" 2>> "$log/man.log" || true
    mandoc -T lint -W error "$log/podnet.1" >> "$log/man.log" 2>&1 || m=1
    [ -s "$log/man.log" ] && m=1
    echo "man $m"; [ "$m" -eq 0 ] || { rc=1; head -20 "$log/man.log"; }
else
    echo "man skipped (no groff or mandoc on this host)"
fi
python3 -I scripts/check-install.py > "$log/install.log" 2>&1; i=$?
echo "install $i"; [ "$i" -eq 0 ] || { rc=1; head -20 "$log/install.log"; }
python3 scripts/check-workflows.py > "$log/workflows.log" 2>&1; w=$?
echo "workflows $w"; [ "$w" -eq 0 ] || { rc=1; head -20 "$log/workflows.log"; }
# The shell scripts, as the job "workflows" of CI checks them.
if command -v shellcheck > /dev/null 2>&1; then
    shellcheck -S warning scripts/*.sh scripts/*/*.sh > "$log/shellcheck.log" 2>&1; h=$?
    echo "shellcheck $h"; [ "$h" -eq 0 ] || { rc=1; head -20 "$log/shellcheck.log"; }
else
    echo "shellcheck skipped (not on this host)"
fi
if [ "${1:-}" != notest ]; then
    timeout 1500 cargo test --workspace > "$log/test.log" 2>&1; t=$?
    echo "test $t"; [ "$t" -eq 0 ] || { rc=1; grep -E "FAILED|panicked|failed" "$log/test.log" | head -30; }
fi
exit "$rc"
