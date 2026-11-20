#!/bin/sh
# The error rows of the lane (T-452): each command that dials, with a
# fault, ends within a limit, with its documented exit code and one error
# line that names each cause once.
#
# Usage (in a lab, from scripts/lab-lane.sh, case errors):
#   sh scripts/lab-errors.sh PODNET DIR
#
# Each row prints one line: pass, fail or n.a. with the reason. The script
# exits 0 when each row passes or is n.a., else 1. The logs of a row are
# DIR/ROW.out and DIR/ROW.err.
#
# The faults: a refused port (127.0.0.1:1), a dead proxy (a proxy on
# 127.0.0.1:1), a name with no answer (under .invalid), an unreachable
# target (192.0.2.1, TEST-NET-1) and a reject rule of the pod. A lab can
# refuse a connect in its own words (Permission denied, Network is
# unreachable): a cause is a pattern with these words.
#
# The rows of connect, tunnel and warp proxy wait for T-497 (their lines
# name a cause two times in the code of podcrates).

set -u

podnet=$1
dir=$2
mkdir -p "$dir" || exit 1
unset HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy NO_PROXY no_proxy PODNET_PROXY
export XDG_STATE_HOME="$dir/state" XDG_RUNTIME_DIR="$dir/run" XDG_CONFIG_HOME="$dir/config"
mkdir -p "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME"
chmod 700 "$XDG_RUNTIME_DIR"

refused='Connection refused|Permission denied|Network is unreachable|Operation not permitted'
failed=0

# check NAME CODE CAUSE: the row's exit code is CODE, and the lines of
# podnet on stderr hold one line that matches CAUSE (an ERE), with no part
# of a line said two times.
check() {
    python3 -I - "$dir/$1.err" "$2" "$3" "$code" "$took" <<'EOF'
import re, sys
err, want, cause, code, took = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4]), float(sys.argv[5])
lines = [l for l in open(err, errors="replace").read().splitlines()
         if l.startswith("podnet") and " INFO " not in l and "runs in the foreground" not in l]
faults = []
if code != want:
    faults.append(f"exit {code}, want {want}")
hits = [l for l in lines if re.search(cause, l)]
if len(hits) != 1:
    faults.append(f"{len(hits)} lines match {cause!r}")
for line in lines:
    # The parts of a line: the text between ": ". A part that comes again,
    # or that a later part starts with, names one step two times.
    parts = [p.strip() for p in line.split(": ") if len(p.strip()) >= 12]
    for i, p in enumerate(parts):
        if any(q == p or q.startswith(p + " ") for q in parts[i + 1:]):
            faults.append(f"the part {p!r} comes two times")
            break
print(f"exit {code} in {took:.1f}s; {len(lines)} lines of podnet")
for l in lines:
    print("  " + l[:300])
for f in faults:
    print("fault: " + f)
sys.exit(1 if faults else 0)
EOF
}

# run NAME LIMIT ARGS...: run podnet with ARGS, at most LIMIT seconds.
run() {
    name=$1
    limit=$2
    shift 2
    start=$(date +%s)
    timeout "$limit" "$podnet" "$@" </dev/null >"$dir/$name.out" 2>"$dir/$name.err"
    code=$?
    took=$(($(date +%s) - start))
}

# row NAME RESULT: print the row; RESULT is 0, 77 (with the reason in
# $why) or other.
row() {
    case $2 in
    0) echo "row $1: pass" ;;
    77) echo "row $1: n.a.: $why" ;;
    *)
        echo "row $1: fail"
        failed=1
        ;;
    esac
}

# bench: exit 1 when no request worked (its help), within --duration and
# the time limit of the dial.
run bench-refused 30 bench --duration 2 --timeout 5 --no-proxy http://127.0.0.1:1/
check bench-refused 1 "no request worked: .*($refused)"
row bench-refused $?

run bench-dead-proxy 30 bench --duration 2 --timeout 5 --proxy http://127.0.0.1:1 http://lab.test/
check bench-dead-proxy 1 "could not reach the proxy 127\.0\.0\.1:1"
row bench-dead-proxy $?

run bench-no-name 60 bench --duration 2 --timeout 5 --no-proxy http://nowhere.invalid/
check bench-no-name 1 "could not resolve nowhere\.invalid"
row bench-no-name $?

run bench-unreachable 60 bench --duration 2 --timeout 5 --no-proxy http://192.0.2.1/
check bench-unreachable 1 "no request worked: GET http://192\.0\.2\.1/"
row bench-unreachable $?

# pod: the program sees ECONNREFUSED or a reset; podnet names the cause
# in one line (T-496). A backend that the lab refuses gives exit 3 and its
# reason: n.a.
client='import socket, sys
s = socket.socket()
s.settimeout(10)
try:
    s.connect((sys.argv[1], 80))
    s.sendall(b"GET / HTTP/1.0\r\n\r\n")
    print("ok" if s.recv(1) else "reset")
except OSError as e:
    print(type(e).__name__)'
pod_row() {
    name=$1
    cause=$2
    shift 2
    run "$name" 60 pod "$@" -- python3 -I -c "$client" "$target"
    if test "$code" -eq 3; then
        why="the lab refuses the pod: $(grep -m 1 '^podnet' "$dir/$name.err")"
        row "$name" 77
        return
    fi
    check "$name" 0 "$cause"
    row "$name" $?
}
target=nowhere.invalid
pod_row pod-no-name "upstream for nowhere\.invalid:80 failed" -x direct
target=lab.test
pod_row pod-reject "refused by the rule domain:lab\.test=reject" -x direct -r domain:lab.test=reject

exit $failed
