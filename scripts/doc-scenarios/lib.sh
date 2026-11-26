# shellcheck shell=sh
# The helpers of the scenarios of the documented commands (T-358, step 2).
#
# scripts/doc-commands.py --run sources this file, then the scenario
# scripts/doc-scenarios/HASH.sh of a block of the class `scenario`. The
# scenario defines the function `setup`, and it can define `ready`, `check`
# and `teardown` (docs/development.md, Documented commands). The working
# directory is the work directory of the block. DOC_DIR is a private
# directory for the logs and the state of the run.

# Stop the run, and report the block as not run on this host: the host
# cannot give what the scenario needs (for example no loopback).
doc_na() {
    printf '%s\n' "$*" > "$DOC_DIR/na"
    exit 77
}

# Stop the run if this host has no loopback where a TCP listener binds.
doc_need_loopback() {
    python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen()' \
        > /dev/null 2>&1 || doc_na "no TCP listener on the loopback"
}

# Stop the run if the port $1 of 127.0.0.1 is in use: the block names that
# port, and another program holds it. $2 is tcp (the default) or udp.
doc_need_port() {
    doc_need_loopback
    python3 -I -c 'import socket, sys
s = socket.socket(type=socket.SOCK_DGRAM if sys.argv[2] == "udp" else socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1])))' "$1" "${2:-tcp}" > /dev/null 2>&1 \
        || doc_na "the ${2:-tcp} port 127.0.0.1:$1 of the block is in use"
}

# Make the fixed unix socket path $1 of the block free: stop the run if
# another process serves it, and remove a stale socket file.
doc_free_socket() {
    if [ -S "$1" ] && python3 -I -c 'import socket, sys
socket.socket(socket.AF_UNIX).connect(sys.argv[1])' "$1" 2> /dev/null; then
        doc_na "another process serves $1"
    fi
    if [ -S "$1" ]; then
        rm -f "$1"
    fi
}

# Print a free TCP port of 127.0.0.1.
doc_free_port() {
    python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

# Start a command in the background. Its output goes to the log
# $DOC_DIR/bg-N.log, and the stop at the end of the run ends it.
doc_bg() {
    doc_n=$(($(wc -l < "$DOC_DIR/pids") + 1))
    "$@" < /dev/null > "$DOC_DIR/bg-$doc_n.log" 2>&1 &
    echo "$!" >> "$DOC_DIR/pids"
}

# Wait until the TCP port $1 of 127.0.0.1 accepts a connection ($2 seconds,
# default 10).
doc_wait_tcp() {
    python3 -I -c 'import socket, sys, time
end = time.monotonic() + float(sys.argv[2])
while True:
    try:
        socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=1).close()
        break
    except OSError:
        if time.monotonic() > end:
            sys.exit("doc-scenario: no listener on 127.0.0.1:%s" % sys.argv[1])
        time.sleep(0.05)' "$1" "${2:-10}"
}

# Wait until the path $1 exists ($2 seconds, default 10).
doc_wait_path() {
    doc_t=0
    while [ ! -e "$1" ]; do
        doc_t=$((doc_t + 1))
        if [ "$doc_t" -gt $((${2:-10} * 20)) ]; then
            echo "doc-scenario: no $1" >&2
            return 1
        fi
        sleep 0.05
    done
}

# Wait until the file $1 has a line that matches the extended regular
# expression $2 ($3 seconds, default 10).
doc_wait_line() {
    doc_t=0
    while ! grep -Eq "$2" "$1" 2> /dev/null; do
        doc_t=$((doc_t + 1))
        if [ "$doc_t" -gt $((${3:-10} * 20)) ]; then
            echo "doc-scenario: no line /$2/ in $1" >&2
            return 1
        fi
        sleep 0.05
    done
}

# Stop each background command and each detached instance of podnet.
doc_stop() {
    while read -r doc_pid; do
        kill "$doc_pid" 2> /dev/null || true
    done < "$DOC_DIR/pids"
    podnet stop --all > /dev/null 2>&1 || true
    doc_t=0
    while read -r doc_pid; do
        while kill -0 "$doc_pid" 2> /dev/null && [ "$doc_t" -lt 40 ]; do
            doc_t=$((doc_t + 1))
            sleep 0.05
        done
        kill -9 "$doc_pid" 2> /dev/null || true
    done < "$DOC_DIR/pids"
}

# Wait until a UDP socket is bound to the port $1 ($2 seconds, default 10).
doc_wait_udp() {
    python3 -I -c 'import sys, time
port = "%04X" % int(sys.argv[1])
end = time.monotonic() + float(sys.argv[2])
def bound():
    for name in ("/proc/net/udp", "/proc/net/udp6"):
        try:
            rows = open(name).read().split("\n")[1:]
        except OSError:
            continue
        if any(r.split()[1].endswith(":" + port) for r in rows if r.split()):
            return True
    return False
while not bound():
    if time.monotonic() > end:
        sys.exit("doc-scenario: no UDP socket on the port %s" % sys.argv[1])
    time.sleep(0.05)' "$1" "${2:-10}"
}
