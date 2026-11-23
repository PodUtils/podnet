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
# The rows of connect, tunnel and warp proxy came with podcrates v0.79.0
# (T-497: their lines named a cause two times before).

set -u

podnet=$1
dir=$2
mkdir -p "$dir" || exit 1
unset HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy NO_PROXY no_proxy PODNET_PROXY
export XDG_STATE_HOME="$dir/state" XDG_RUNTIME_DIR="$dir/run" XDG_CONFIG_HOME="$dir/config"
mkdir -p "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME"
chmod 700 "$XDG_RUNTIME_DIR"

refused='Connection refused|Permission denied|Network (is )?unreachable|Operation not permitted'
failed=0

# check NAME CODE CAUSE: the row's exit code is CODE, and the lines of
# podnet on stderr hold one line that matches CAUSE (an ERE), with no part
# of a line said two times.
check() {
    python3 -I - "$dir/$1.err" "$2" "$3" "$code" "$took" <<'EOF'
import re, sys
err, want, cause, code, took = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4]), float(sys.argv[5])
# The lines of podnet: its own error lines, and the WARN and ERROR lines of
# its log (a server names the fault of each connection there).
lines = [l for l in open(err, errors="replace").read().splitlines()
         if (l.startswith("podnet") or " WARN " in l or " ERROR " in l)
         and " INFO " not in l and "runs in the foreground" not in l]
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

# connect checks its remote at the start, tunnel asks the quick-tunnel
# service, warp proxy asks the WARP API: each fault ends with exit 69 (a
# service or a network path is not available).
run connect-refused 60 connect --stdio --no-proxy wss://127.0.0.1:1/__podnet/ws
check connect-refused 69 "does not answer.*($refused)"
row connect-refused $?

run connect-no-name 60 connect --stdio --no-proxy wss://nowhere.invalid/__podnet/ws
check connect-no-name 69 "could not resolve nowhere\.invalid"
row connect-no-name $?

run connect-dead-proxy 60 connect --stdio --proxy http://127.0.0.1:1 wss://lab.test/__podnet/ws
check connect-dead-proxy 69 "could not reach the proxy 127\.0\.0\.1:1"
row connect-dead-proxy $?

run tunnel-dead-proxy 60 tunnel --hello-world --proxy http://127.0.0.1:1
check tunnel-dead-proxy 69 "could not reach the proxy 127\.0\.0\.1:1"
row tunnel-dead-proxy $?

run warp-proxy-dead-proxy 60 warp proxy --proxy http://127.0.0.1:1 --listen 127.0.0.1:0
check warp-proxy-dead-proxy 69 "each host failed: .*could not reach the proxy 127\.0\.0\.1:1"
row warp-proxy-dead-proxy $?

# relay connect: a session to a node through the relay; a fault of the
# relay ends it with exit 69. The token is a stand-in (no dial gets so far).
run relay-refused 60 relay connect x --token t0k --no-proxy --relay http://127.0.0.1:1
check relay-refused 69 "the session to x: .*($refused)"
row relay-refused $?

run relay-no-name 60 relay connect x --token t0k --no-proxy --relay https://nowhere.invalid
check relay-no-name 69 "could not resolve nowhere\.invalid"
row relay-no-name $?

run relay-dead-proxy 60 relay connect x --token t0k --relay https://lab.test --proxy http://127.0.0.1:1
check relay-dead-proxy 69 "could not reach the proxy 127\.0\.0\.1:1"
row relay-dead-proxy $?

# forward: a server. The fault shows for each connection: the client is
# closed within 40 s (no data), and the log has one line with the cause. A stop
# signal ends the command with exit 0.
forward_row() {
    name=$1
    port=$2
    cause=$3
    shift 3
    start=$(date +%s)
    timeout 60 "$podnet" forward -L "127.0.0.1:$port=$target" "$@" --run-for 30 </dev/null >"$dir/$name.out" 2>"$dir/$name.err" &
    pid=$!
    i=0
    while ! grep -q ' via ' "$dir/$name.out" "$dir/$name.err" 2>/dev/null && kill -0 "$pid" 2>/dev/null && test $i -lt 100; do
        i=$((i + 1))
        sleep 0.1
    done
    if ! grep -q ' via ' "$dir/$name.out" "$dir/$name.err"; then
        kill "$pid" 2>/dev/null
        wait "$pid"
        if grep -Eq '(Permission denied|Operation not permitted|Address family)' "$dir/$name.err"; then
            why="the lab refuses the listener: $(grep -m 1 '^podnet' "$dir/$name.err")"
            row "$name" 77
            return
        fi
        echo "forward did not start: $(cat "$dir/$name.err")"
        row "$name" 1
        return
    fi
    python3 -I - "$port" >"$dir/$name.client" 2>&1 <<'PY'
import socket, sys, time
t = time.time()
try:
    s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), 5)
except OSError as e:
    print("noconnect", e)
    sys.exit(0)
s.settimeout(40)  # a name with no answer takes up to 20 s on an IPv6-only host
try:
    s.sendall(b"GET / HTTP/1.0\r\n\r\n")
    data = s.recv(100)
    print("data" if data else "closed", round(time.time() - t, 1))
except OSError as e:
    print(type(e).__name__, round(time.time() - t, 1))
PY
    sleep 0.5
    kill -TERM "$pid" 2>/dev/null
    wait "$pid"
    code=$?
    took=$(($(date +%s) - start))
    echo "client: $(cat "$dir/$name.client")"
    if grep -q '^noconnect' "$dir/$name.client"; then
        why="the client cannot reach the listener on the loopback of the lab: $(cat "$dir/$name.client")"
        row "$name" 77
        return
    fi
    if grep -qE '^(data|TimeoutError|timeout)' "$dir/$name.client"; then
        row "$name" 1
        return
    fi
    check "$name" 0 "$cause"
    row "$name" $?
}
target=127.0.0.1:1
forward_row forward-refused 17401 "did not reach its target: .*($refused)" --network direct
target=nowhere.invalid:80
forward_row forward-no-name 17402 "could not resolve nowhere\.invalid" --network direct
target=lab.test:80
forward_row forward-dead-proxy 17403 "could not reach the proxy 127\.0\.0\.1:1" --network http://127.0.0.1:1

# net proxy --forward VIRT=REAL: the proxy answers the CONNECT to VIRT (the
# virtual endpoint accepts, as the near side of ssh -L does), and the real
# dial fails after it: the client is closed within 40 s, with no data, and the
# log has one line with the cause.
net_proxy_row() {
    name=$1
    real=$2
    cause=$3
    shift 3
    start=$(date +%s)
    sock="$dir/$name.sock"
    rm -f "$sock"
    timeout 60 "$podnet" net proxy --listen "unix:$sock" --forward "10.66.0.2:8080=$real" "$@" --run-for 30 \
        </dev/null >"$dir/$name.out" 2>"$dir/$name.err" &
    pid=$!
    i=0
    # The line `podnet: proxy unix:PATH` says that the proxy listens.
    while ! grep -q '^podnet: proxy ' "$dir/$name.out" 2>/dev/null && kill -0 "$pid" 2>/dev/null && test $i -lt 100; do
        i=$((i + 1))
        sleep 0.1
    done
    if ! grep -q '^podnet: proxy ' "$dir/$name.out"; then
        kill "$pid" 2>/dev/null
        wait "$pid"
        if grep -Eq '(Permission denied|Operation not permitted)' "$dir/$name.err"; then
            why="the lab refuses the listener: $(grep -m 1 '^podnet' "$dir/$name.err")"
            row "$name" 77
            return
        fi
        echo "net proxy did not start: $(cat "$dir/$name.err")"
        row "$name" 1
        return
    fi
    python3 -I - "$sock" >"$dir/$name.client" 2>&1 <<'PY'
import socket, sys, time
t = time.time()
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.settimeout(40)  # a name with no answer takes up to 20 s on an IPv6-only host
try:
    s.sendall(b"CONNECT 10.66.0.2:8080 HTTP/1.1\r\n\r\n")
    head = s.recv(200)
    s.sendall(b"GET / HTTP/1.0\r\n\r\n")
    data = s.recv(100)
    print("data" if data else "closed", round(time.time() - t, 1), head[:12])
except OSError as e:
    print(type(e).__name__, round(time.time() - t, 1))
PY
    sleep 0.5
    kill -TERM "$pid" 2>/dev/null
    wait "$pid"
    code=$?
    took=$(($(date +%s) - start))
    echo "client: $(cat "$dir/$name.client")"
    if grep -qE '^(data|TimeoutError|timeout)' "$dir/$name.client"; then
        row "$name" 1
        return
    fi
    check "$name" 0 "$cause"
    row "$name" $?
}
net_proxy_row net-proxy-refused 127.0.0.1:1 "could not reach the real service .*($refused)" --no-proxy
net_proxy_row net-proxy-no-name nowhere.invalid:80 "could not resolve nowhere\.invalid" --no-proxy
net_proxy_row net-proxy-dead-proxy lab.test:80 "could not reach the proxy 127\.0\.0\.1:1" --proxy http://127.0.0.1:1

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

# A listener that cannot bind (T-515, T-523): exit 69 and one line with the
# address and a next step. The address is a port that a holder keeps; in a
# lab that refuses each TCP bind (bind-denied), the bind fails with EACCES.
python3 -I -c 'import socket, time
s = socket.socket()
s.bind(("127.0.0.1", 17410))
s.listen()
time.sleep(90)' 2>/dev/null &
holder=$!
sleep 0.5
listen_row() {
    name=$1
    shift
    run "$name" 30 "$@" --run-for 2
    check "$name" 69 "127\.0\.0\.1:17410.*; give "
    row "$name" $?
}
listen_row listen-connect connect wss://nowhere.invalid/__podnet/ws -L tcp://127.0.0.1:17410 --no-check
listen_row listen-net-proxy net proxy --listen tcp://127.0.0.1:17410
listen_row listen-forward forward -L 17410=192.0.2.1:80
listen_row listen-serve serve "$dir" --local 127.0.0.1:17410
listen_row listen-warp-proxy warp proxy --ephemeral --listen tcp://127.0.0.1:17410
kill "$holder" 2>/dev/null

# pod: a SIGTERM to podnet goes to the program (T-517). podnet exits 143,
# and the program does not stay. A backend that the lab refuses: n.a.
pod_signal_row() {
    name=$1
    shift
    pidfile="$dir/$name.pid"
    start=$(date +%s)
    "$podnet" pod "$@" -- sh -c "echo \$\$ > $pidfile; exec sleep 37" </dev/null >"$dir/$name.out" 2>"$dir/$name.err" &
    pid=$!
    i=0
    while ! test -s "$pidfile" && kill -0 "$pid" 2>/dev/null && test $i -lt 100; do
        i=$((i + 1))
        sleep 0.1
    done
    if ! test -s "$pidfile"; then
        wait "$pid"
        if test $? -eq 3; then
            why="the lab refuses the pod: $(grep -m 1 '^podnet' "$dir/$name.err")"
            row "$name" 77
            return
        fi
        echo "the program did not start: $(cat "$dir/$name.err")"
        row "$name" 1
        return
    fi
    kill -TERM "$pid"
    wait "$pid"
    code=$?
    took=$(($(date +%s) - start))
    sleep 0.2
    program=$(cat "$pidfile")
    if kill -0 "$program" 2>/dev/null && ! grep -q '^State:.*Z' "/proc/$program/status" 2>/dev/null; then
        echo "the program $program stays after podnet exits $code"
        kill -KILL "$program" 2>/dev/null
        row "$name" 1
        return
    fi
    check "$name" 143 "SIGTERM: stopping the program"
    row "$name" $?
}
pod_signal_row pod-sigterm-auto -x direct
pod_signal_row pod-sigterm-seccomp --backend seccomp -x direct

exit $failed
