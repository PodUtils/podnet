#!/bin/sh
# The lane of podnet in the profiles of pod-lab (T-358 step 1).
#
# Usage: sh scripts/lab-lane.sh PODLAB PODNET [PROFILE...]
#
# PODLAB is the binary podlab of PodUtils/podcrates (crate pod-lab, feature
# cli), or a directory from scripts/get-podlab.sh: the lane then checks the
# asset podlab-ARCH-unknown-linux-musl against the SHA256SUMS of the
# directory and runs it. PODNET is the podnet binary to test. With no
# PROFILE, the lane runs in each profile of `podlab list`, and in the rows
# allow-connect and way-out. Each command
# runs in a lab of the profile: `podlab run PROFILE -- sh lab-lane.sh
# --case NAME ...`.
#
# The row allow-connect is not a profile of podlab. It runs the rows of
# PROXY_CASES in the lab
# `podlab run --allow-connect PORT bind-denied` (podlab 0.58.0 or later):
# each TCP connect is refused, except to PORT. A stand-in HTTP CONNECT
# proxy (tests/consumers/connect-proxy.py) listens on 127.0.0.1:PORT
# outside the lab. It connects only proxy.test:PORT+1, to a server on
# 127.0.0.1:PORT+1. LANE_PROXY_PORT gives PORT (default 7120). LANE_PLANT
# plants a defect: refuse (the proxy refuses each CONNECT) or port (podnet
# gets the port PORT+2, which the lab refuses). Each pod row must then fail.
#
# The row way-out is the lab `podlab run --way-out proxy-only` (podlab
# 0.61.0 or later): the proxy of the lab sends each CONNECT to the real
# network. It runs the rows of WAYOUT_CASES: a quick tunnel of podnet from
# a host whose only way out is a proxy, and relay connect to the public
# relay. The row way-ws is the lab `podlab run --way-out ws-upgrade-refused`:
# its proxy refuses the WebSocket upgrade (WAYWS_CASES). Both need the
# network of the host.
#
# The network labs of NET_LABS (podlab 0.60.0 or later) run the rows of
# NET_CASES. A lab gives its own proxy (HTTPS_PROXY, maybe with a
# credential), a TLS stand-in server for each name, and SSL_CERT_FILE.
# The plant LANE_PLANT=net gives podnet a proxy that does not listen; each
# net-pod row must then fail.
#
# The row testers (T-453) is the host class of the external testers: the
# lab bind-denied (no bind of AF_INET, no TCP connect but to the
# loopback, so no netns backend), and PODNET_NETNS_TEST_MEM=none (the pod
# cannot write the memory of the program; a lab cannot take that away,
# so the test hook of the seccomp backend gives it). It runs the rows of
# TESTER_CASES.
#
# The case errors (in CASES and NET_CASES) runs scripts/lab-errors.sh: the
# error rows of T-452 (a refused port, a dead proxy, a name with no answer,
# an unreachable target, a reject rule).
#
# The script prints one row for each profile and command: pass, fail or
# n.a. with the reason. It exits 1 when a command fails, else 0. The logs
# are in LANE_DIR (default: .work/lab-lane in the current directory).
# LANE_DIR must not be in /tmp (a lab mounts a tmpfs on /tmp) or in HOME
# (a lab mounts HOME noexec). python3 runs the stand-in servers and the
# dynamic client.
#
# Run it as a user that is not root: the profile nonroot-nocaps does not
# hold for root, and the other labs then run as root.

set -u

CASES="version man local errors doctor shim-forward link-unix link-control net-switch wg-serve pod-seccomp pod-netns net-proxy-unix"
NET_LABS="proxy-only proxy-tls-reterminate proxy-per-name ws-upgrade-refused sni-allowlist no-proxy-no-dns ipv6-only"
NET_CASES="version errors doctor net-curl net-bench net-exit net-pod-auto net-pod-seccomp net-pod-netns"
WAYOUT_CASES="way-tunnel way-relay"
WAYWS_CASES="way-relay-refused"
TESTER_CASES="pod-seccomp pod-netns errors"
PROXY_CASES="proxy-direct proxy-port proxy-pod-auto proxy-pod-seccomp proxy-pod-env proxy-doctor"

# The cases. Each one runs inside the lab and exits 0 (pass), 1 (fail) or
# 77 (not applicable, with the reason on the last line of its output).
case_main() {
    name=$1
    podnet=$2
    dir=$3
    mkdir -p "$dir" || exit 1
    export XDG_STATE_HOME="$dir/state" XDG_RUNTIME_DIR="$dir/run" XDG_CONFIG_HOME="$dir/config"
    mkdir -p "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME"
    chmod 700 "$XDG_RUNTIME_DIR"
    case $name in
    version)
        out=$("$podnet" --version) || exit 1
        echo "$out"
        case $out in podnet\ [0-9]*) exit 0 ;; *) exit 1 ;; esac
        ;;
    man)
        "$podnet" man --no-pager >"$dir/man.txt" || exit 1
        grep -q '^EXIT STATUS' "$dir/man.txt" || { echo "no EXIT STATUS section"; exit 1; }
        wc -l <"$dir/man.txt"
        ;;
    local)
        # The commands that need no network and no other process: each
        # exits 0 with its output (T-358 step 4).
        # The output of the command is in local.txt; it exited CODE. An
        # empty pattern checks the exit code only.
        local_check() {
            echo "exit $code"
            test $code -eq 0 || { cat "$dir/local.err"; return 1; }
            test -z "$1" || grep -q -- "$1" "$dir/local.txt" || { echo "no line with: $1"; head -5 "$dir/local.txt"; return 1; }
        }
        timeout 60 "$podnet" completions bash >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '^_podnet()' || exit 1
        timeout 60 "$podnet" components --json >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '"components":' || exit 1
        timeout 60 "$podnet" status --json >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '"instances":' || exit 1
        timeout 60 "$podnet" ports >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '' || exit 1
        timeout 60 "$podnet" qr https://lab.test/ >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '█' || exit 1
        timeout 60 "$podnet" bug-report >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '^podnet bug-report' || exit 1
        timeout 60 "$podnet" net addresses >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check 'gateway' || exit 1
        timeout 60 "$podnet" link carriers >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '^unix: ' || exit 1
        timeout 60 "$podnet" autostart list >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '' || exit 1
        timeout 60 "$podnet" agent status >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check 'proxy:' || exit 1
        timeout 60 "$podnet" net demo >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '^PROOF: ' || exit 1
        # The passwd shim: a program under the preload finds the user.
        timeout 60 "$podnet" net passwd-shim --out "$dir/pw" --user labuser >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check 'fakepwd' || exit 1
        LD_PRELOAD="$dir/pw/fakepwd.so" SANDHOME_PASSWD="$dir/pw/passwd" whoami >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '^labuser$' || exit 1
        timeout 60 "$podnet" net ts-shims --out "$dir/ts" --user labuser >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '' || exit 1
        "$dir/ts/getent" passwd labuser >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '^labuser:' || exit 1
        # The socks shim is built, with its environment.
        timeout 60 "$podnet" net socksify --out "$dir/socks" >"$dir/local.txt" 2>"$dir/local.err"
        code=$?
        local_check '^export LD_PRELOAD=' || exit 1
        test -s "$dir/socks/libpodnet-socks.so" || { echo "no libpodnet-socks.so"; exit 1; }
        ;;
    doctor)
        timeout 120 "$podnet" doctor >"$dir/doctor.txt" 2>&1
        code=$?
        cat "$dir/doctor.txt"
        echo "exit $code"
        # 0: a leg can work; 1: no leg can work. Both are answers.
        case $code in 0 | 1) ;; *) exit 1 ;; esac
        if grep -q 'panicked' "$dir/doctor.txt"; then exit 1; fi
        test -s "$dir/doctor.txt" || exit 1
        # T-524: where a connect to the loopback fails (the lab
        # no-loopback), net doctor does not say "not needed".
        timeout 60 "$podnet" net doctor >"$dir/net-doctor.txt" 2>&1
        cat "$dir/net-doctor.txt"
        if grep -q '^connect(AF_INET, 127.0.0.1) *fail' "$dir/net-doctor.txt" &&
            grep -q 'podnet-vnet: not needed' "$dir/net-doctor.txt"; then
            echo "net doctor: not needed with no loopback connect"
            exit 1
        fi
        # In a network lab, the doctor names the proxy of the lab, and its
        # credential where the lab gives one.
        if in_list "$LANE_PROFILE" "$NET_LABS"; then
            want="proxy  none: each connection is direct"
            if test -n "${HTTPS_PROXY:-}"; then
                want="proxy  http://127.0.0.1:3128 (credentials: none)"
                case $HTTPS_PROXY in *@*) want="proxy  http://127.0.0.1:3128 (credentials: present)" ;; esac
            fi
            grep -qxF "$want" "$dir/doctor.txt" || { echo "no line: $want"; exit 1; }
        fi
        ;;
    shim-forward)
        # The echo server runs outside the lab (the broker of a consumer).
        sock="$dir/../echo.sock"
        "$podnet" net shim --out "$dir/shim" --forward "127.0.0.1:7001=unix:$sock" --json >"$dir/shim.json" ||
            exit 1
        cat "$dir/shim.json"
        # The negative control: with no shim, nothing answers at 127.0.0.1:7001.
        bare=$(python3 -I "$dir/../client.py" 127.0.0.1 7001 2>&1)
        echo "with no shim: $bare"
        got=$(python3 -I "$dir/../client.py" --env "$dir/shim.json" 127.0.0.1 7001)
        code=$?
        echo "client exit $code: $got"
        test "$bare" != "echo:ping" && test $code -eq 0 && test "$got" = "echo:ping"
        exit $?
        ;;
    link-unix)
        fp=$("$podnet" link status --create --json | python3 -I -c \
            'import json,sys; print(json.load(sys.stdin)["identity"]["fingerprint"])') || exit 1
        echo "fingerprint $fp"
        printf '%s\n' "$fp" >"$dir/allow"
        chmod 600 "$dir/allow"
        "$podnet" link listen "$dir/link.sock" --allow "$dir/allow" --to "unix:$dir/../echo.sock" \
            >"$dir/listen.txt" 2>&1 &
        listener=$!
        if ! wait_path "$dir/link.sock"; then
            kill $listener 2>/dev/null
            cat "$dir/listen.txt"
            listen_refused "$dir"
            exit $?
        fi
        got=$(printf 'ping' | timeout 30 "$podnet" link dial "$dir/link.sock" --peer "$fp" 2>"$dir/dial.txt")
        code=$?
        kill $listener 2>/dev/null
        cat "$dir/listen.txt" "$dir/dial.txt"
        echo "dial exit $code: $got"
        test $code -eq 0 && test "$got" = "echo:ping"
        exit $?
        ;;
    pod-seccomp | pod-netns)
        backend=${name#pod-}
        timeout 60 "$podnet" pod --backend "$backend" -x "socks5://unix:$dir/none.sock" -- /bin/true \
            >"$dir/pod.txt" 2>&1
        code=$?
        cat "$dir/pod.txt"
        echo "exit $code"
        test $code -eq 0 && exit 0
        # Exit 3: the backend cannot run here, with the reason. That is
        # not applicable only where the profile takes what the backend
        # needs by design (refusal_expected). Else it is a failure.
        if test $code -eq 3 && test -s "$dir/pod.txt" && refusal_expected "$name" "$LANE_PROFILE"; then
            echo "podnet refused with exit 3 and a reason: $why"
            exit 77
        fi
        exit 1
        ;;
    link-control)
        # A listener and a dialer in the background, and the control
        # commands of the dialer (T-398).
        fp=$("$podnet" link status --create --json | python3 -I -c \
            'import json,sys; print(json.load(sys.stdin)["identity"]["fingerprint"])') || exit 1
        printf '%s\n' "$fp" >"$dir/allow"
        chmod 600 "$dir/allow"
        if ! "$podnet" link listen "$dir/link.sock" --allow "$dir/allow" --to "unix:$dir/../echo.sock" \
            --announce --detach --name lane-l >"$dir/listen.txt" 2>&1; then
            cat "$dir/listen.txt"
            listen_refused "$dir"
            exit $?
        fi
        bad=0
        timeout 30 "$podnet" wait lane-l >"$dir/wait.txt" 2>&1 || bad=1
        grep -q '"command":"link listen"' "$dir/wait.txt" || bad=1
        if test $bad -eq 0; then
            "$podnet" link connect "$dir/link.sock" --peer "$fp" --announce --detach --name lane-c \
                >"$dir/connect.txt" 2>&1 || bad=1
        fi
        if test $bad -eq 0; then
            "$podnet" link status lane-c >"$dir/status.txt" 2>&1 || bad=1
            grep -q ' up ' "$dir/status.txt" || bad=1
            # A unix listener of forward to the service raw of the peer.
            "$podnet" forward -L "unix:$dir/f.sock=peer:$fp/raw" >"$dir/forward.txt" 2>&1 &
            fwd=$!
            if wait_path "$dir/f.sock"; then
                got=$(timeout 30 python3 -I "$dir/../unix-client.py" "$dir/f.sock")
                echo "forward: $got"
                test "$got" = "echo:ping" || bad=1
            else
                bad=1
            fi
            kill $fwd 2>/dev/null
            # The mesh view of the dialer: itself, and the edge to the peer.
            ctl=$(sed -n 's/.*"control":"\([^"]*\)".*/\1/p' "$dir/connect.txt" | head -n 1)
            timeout 30 "$podnet" link mesh --control "unix:$ctl" >"$dir/mesh.txt" 2>&1 || bad=1
            grep -q '^  edge ' "$dir/mesh.txt" || bad=1
            timeout 30 "$podnet" link reconnect lane-c >"$dir/reconnect.txt" 2>&1 || bad=1
            grep -q '^reconnect link 1 -> 2 ' "$dir/reconnect.txt" || bad=1
            timeout 30 "$podnet" link rotate lane-c >"$dir/rotate.txt" 2>&1 || bad=1
            grep -q '^rotate link 2 -> 3 ' "$dir/rotate.txt" || bad=1
            timeout 30 "$podnet" link close lane-c >"$dir/close.txt" 2>&1 || bad=1
            grep -q '^closed link 3 ' "$dir/close.txt" || bad=1
        fi
        "$podnet" link close lane-l >"$dir/closel.txt" 2>&1 || bad=1
        cat "$dir/listen.txt" "$dir"/wait.txt "$dir"/connect.txt "$dir"/status.txt "$dir"/forward.txt "$dir"/mesh.txt \
            "$dir"/reconnect.txt \
            "$dir"/rotate.txt "$dir"/close.txt "$dir"/closel.txt 2>/dev/null
        echo "bad $bad"
        exit $bad
        ;;
    net-switch)
        # Two stacks on a unix switch (T-398): stack A (podnet expose --via
        # switch) forwards 10.66.0.2:8080 to the echo server; a SOCKS5
        # client of stack B connects 10.66.0.2:8080 through the switch.
        "$podnet" net switch --listen "unix:$dir/sw" >"$dir/sw.txt" 2>&1 &
        sw=$!
        if ! wait_path "$dir/sw"; then
            kill $sw 2>/dev/null
            cat "$dir/sw.txt"
            listen_refused "$dir"
            exit $?
        fi
        "$podnet" expose "unix:$dir/../echo.sock" --via switch --as 10.66.0.2:8080 -- \
            --switch "unix:$dir/sw" --address 10.66.0.2 --listen "unix:$dir/a.sock" >"$dir/a.txt" 2>&1 &
        sa=$!
        "$podnet" net proxy --switch "unix:$dir/sw" --address 10.66.0.3 --listen "unix:$dir/b.sock" >"$dir/b.txt" 2>&1 &
        sb=$!
        got=fail
        if wait_path "$dir/a.sock" && wait_path "$dir/b.sock"; then
            got=$(timeout 30 python3 -I "$dir/../socks-echo.py" "$dir/b.sock" 10.66.0.2 8080 2>&1)
        fi
        kill $sa $sb $sw 2>/dev/null
        cat "$dir/sw.txt" "$dir/a.txt" "$dir/b.txt"
        echo "client: $got"
        test "$got" = "echo:ping"
        exit $?
        ;;
    wg-serve)
        # podnet wg serve on UDP, with keys made for this run; a client of
        # podnet (forward --network wg:FILE) sends a stream through it; the
        # server dials through a SOCKS5 stub that echoes (T-398).
        if ! python3 -I -c 'import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", 0)); s.sendto(b"x", s.getsockname())' 2>/dev/null; then
            echo "the lab refuses UDP on the loopback"
            exit 77
        fi
        python3 -I "$dir/../wgkeys.py" >"$dir/keys" || exit 1
        sk=$(awk '$1 == "server" { print $2 }' "$dir/keys")
        sp=$(awk '$1 == "server" { print $3 }' "$dir/keys")
        ck=$(awk '$1 == "client" { print $2 }' "$dir/keys")
        cp=$(awk '$1 == "client" { print $3 }' "$dir/keys")
        python3 -I "$dir/../socks-stub.py" "$dir/stub.sock" >"$dir/stub.txt" 2>&1 &
        stub=$!
        (
            umask 077
            printf '[Interface]\nPrivateKey = %s\nListenPort = 0\nAddress = 10.9.0.1/24\n\n[Peer]\nPublicKey = %s\nAllowedIPs = 10.9.0.2/32\n' \
                "$sk" "$cp" >"$dir/server.conf"
        )
        "$podnet" wg serve --config "$dir/server.conf" --egress "socks5h+unix:$dir/stub.sock" >"$dir/wg.txt" 2>&1 &
        wg=$!
        i=0
        port=
        while test -z "$port" && test $i -lt 100; do
            port=$(sed -n 's/.*udp:0\.0\.0\.0:\([0-9]*\).*/\1/p' "$dir/wg.txt" | head -n 1)
            i=$((i + 1))
            sleep 0.1
        done
        (
            umask 077
            printf '[Interface]\nPrivateKey = %s\nAddress = 10.9.0.2/32\n\n[Peer]\nPublicKey = %s\nEndpoint = 127.0.0.1:%s\nAllowedIPs = 0.0.0.0/0\n' \
                "$ck" "$sp" "$port" >"$dir/client.conf"
        )
        "$podnet" forward --network "wg:$dir/client.conf" -L "unix:$dir/f.sock=192.0.2.10:7" >"$dir/forward.txt" 2>&1 &
        fwd=$!
        got=fail
        if test -n "$port" && wait_path "$dir/f.sock"; then
            got=$(timeout 30 python3 -I "$dir/../unix-client.py" "$dir/f.sock" 2>&1)
        fi
        kill $fwd $wg $stub 2>/dev/null
        cat "$dir/wg.txt" "$dir/forward.txt"
        if grep -q 'f.sock: Permission denied' "$dir/forward.txt"; then
            listen_refused "$dir"
            exit $?
        fi
        echo "client: $got"
        test "$got" = "echo:ping"
        exit $?
        ;;
    net-proxy-unix)
        "$podnet" net proxy --listen "unix:$dir/proxy.sock" >"$dir/proxy.txt" 2>&1 &
        proxy=$!
        if ! wait_path "$dir/proxy.sock"; then
            kill $proxy 2>/dev/null
            cat "$dir/proxy.txt"
            listen_refused "$dir"
            exit $?
        fi
        got=$(python3 -I "$dir/../socks.py" "$dir/proxy.sock")
        code=$?
        kill $proxy 2>/dev/null
        cat "$dir/proxy.txt"
        echo "greeting exit $code: $got"
        test $code -eq 0 && test "$got" = "0500"
        exit $?
        ;;
    way-tunnel)
        # A quick tunnel from a host whose only way out is a proxy: the lab
        # proxy-only with --way-out sends each CONNECT to the real network
        # (T-358). The lab sets SSL_CERT_FILE to the roots of its stand-in
        # servers, which replace the roots of podnet: the real edge needs
        # the roots of the system. LANE_CA_FILE names the roots of a proxy
        # of the host that re-signs TLS (not on a GitHub runner).
        unset SSL_CERT_FILE
        if test -n "${LANE_CA_FILE:-}"; then
            timeout 300 "$podnet" demo --exit-on-verify --ca-file "$LANE_CA_FILE" >"$dir/out.txt" 2>&1
        else
            timeout 300 "$podnet" demo --exit-on-verify >"$dir/out.txt" 2>&1
        fi
        code=$?
        grep -v '^ ' "$dir/out.txt" | grep -v trycloudflare.com | tail -20
        echo "demo exit $code"
        test $code -eq 0 && grep -q -E 'verified (public URL|https://[^ ]+ HTTP 200)' "$dir/out.txt"
        exit $?
        ;;
    way-relay | way-relay-refused)
        # relay connect to the public relay through the proxy of the lab,
        # with a token that the relay refuses (T-421). way-relay (proxy-only):
        # the WebSocket passes, and the relay itself refuses the token (exit
        # 78). way-relay-refused (ws-upgrade-refused): the proxy terminates
        # TLS with the CA of SSL_CERT_FILE and refuses the upgrade; podnet
        # trusts that CA on the WebSocket path and names the hop, not the
        # token (exit 69).
        timeout 120 "$podnet" relay connect lanetest --token not-a-token </dev/null >"$dir/out.txt" 2>&1
        code=$?
        grep -v '^ ' "$dir/out.txt" | tail -5
        echo "relay connect exit $code"
        if test "$name" = way-relay; then
            test $code -eq 78 && grep -q 'refused the connect token' "$dir/out.txt"
        else
            test $code -eq 69 && grep -q 'on the way refused the WebSocket upgrade' "$dir/out.txt"
        fi
        exit $?
        ;;
    errors)
        # The error rows (T-452): scripts/lab-errors.sh, beside this file.
        sh "$(dirname "$0")/lab-errors.sh" "$podnet" "$dir"
        exit $?
        ;;
    proxy-*)
        proxy_case "$name" "$podnet" "$dir"
        exit $?
        ;;
    net-*)
        net_case "$name" "$podnet" "$dir"
        exit $?
        ;;
    *)
        echo "unknown case $name"
        exit 2
        ;;
    esac
}

# The rows of allow-connect, in the lab with --allow-connect of the proxy
# port. The proxy log is $dir/../proxy.txt; the outer script empties it
# before each row.
proxy_case() {
    name=$1
    podnet=$2
    dir=$3
    port=$LANE_PROXY_PORT
    tport=$((port + 1))
    upstream="http://127.0.0.1:$LANE_UPSTREAM_PORT"
    url="http://proxy.test:$tport/"
    body='podnet-proxy-only'
    log="$dir/../proxy.txt"
    # The variables of the host of this script are not the proxy of the lab.
    unset HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy NO_PROXY no_proxy
    case $name in
    proxy-direct)
        # The negative control: with no podnet and no proxy, the lab
        # refuses the connect to the server.
        python3 -I "$dir/../http-client.py" urllib "http://127.0.0.1:$tport/" >"$dir/out.txt" 2>&1
        code=$?
        cat "$dir/out.txt"
        echo "client exit $code"
        test $code -ne 0 && grep -q 'Permission denied' "$dir/out.txt"
        return $?
        ;;
    proxy-port)
        # The control of the lab: the proxy port is open, with no podnet.
        python3 -I "$dir/../connect.py" "$port" "proxy.test:$tport" >"$dir/out.txt" 2>&1
        code=$?
        cat "$dir/out.txt" "$log"
        echo "client exit $code"
        test $code -eq 0 && grep -qx "$body" "$dir/out.txt"
        return $?
        ;;
    proxy-pod-auto | proxy-pod-seccomp)
        backend=${name#proxy-pod-}
        timeout 60 "$podnet" pod --backend "$backend" -x "$upstream" -- \
            python3 -I "$dir/../http-client.py" urllib "$url" >"$dir/out.txt" 2>&1
        code=$?
        ;;
    proxy-pod-env)
        # The upstream from the environment, as a proxy-only host sets it.
        HTTPS_PROXY=$upstream HTTP_PROXY=$upstream NO_PROXY=localhost,127.0.0.1 \
            timeout 60 "$podnet" pod -- python3 -I "$dir/../http-client.py" urllib "$url" >"$dir/out.txt" 2>&1
        code=$?
        ;;
    proxy-doctor)
        # The doctor sends its probes through the proxy, and names the
        # refusals of the proxy (the stand-in refuses each other target).
        HTTPS_PROXY=$upstream timeout 120 "$podnet" doctor >"$dir/out.txt" 2>&1
        code=$?
        cat "$dir/out.txt" "$log"
        echo "doctor exit $code"
        case $code in 0 | 1) ;; *) return 1 ;; esac
        grep -q 'panicked' "$dir/out.txt" && return 1
        grep -q '^request CONNECT ' "$log" && grep -q 'proxy 403' "$dir/out.txt"
        return $?
        ;;
    esac
    cat "$dir/out.txt" "$log"
    echo "pod exit $code"
    test $code -eq 0 && grep -qx "$body" "$dir/out.txt" && grep -q "^request CONNECT proxy.test:$tport " "$log"
}

# The rows of the network labs. The lab sets HTTPS_PROXY (or no proxy)
# and SSL_CERT_FILE; its TLS stand-in answers each name.
net_case() {
    name=$1
    podnet=$2
    dir=$3
    url=https://lab.test/
    if test -z "${HTTPS_PROXY:-}"; then
        # No proxy: podnet pod has no egress, and must say so.
        case $name in net-curl | net-bench | net-exit)
            echo "the lab has no proxy"
            return 77
            ;;
        esac
        timeout 60 "$podnet" pod -- /bin/true >"$dir/out.txt" 2>&1
        code=$?
        cat "$dir/out.txt"
        echo "pod exit $code"
        test $code -ne 0 && grep -q 'no proxy: give -x PROXY' "$dir/out.txt" || return 1
        echo "the lab has no proxy; podnet pod refused with the reason"
        return 77
    fi
    case $name in
    net-bench)
        # podnet bench through the proxy of the lab, with its roots
        # (SSL_CERT_FILE): each request gets an answer (T-398).
        timeout 60 "$podnet" bench "$url" --duration 2 --connections 2 --json >"$dir/out.txt" 2>"$dir/err.txt"
        code=$?
        cat "$dir/out.txt" "$dir/err.txt"
        echo "bench exit $code"
        test $code -eq 0 || return 1
        python3 -I -c 'import json, sys
d = json.load(open(sys.argv[1]))
print("bench:", {k: d.get(k) for k in ("requests", "errors", "status")})
sys.exit(0 if d.get("requests", 0) > 0 and not d.get("errors") else 1)' "$dir/out.txt"
        return $?
        ;;
    net-exit)
        # podnet exit --stdio: a SOCKS5 session on a pipe, out through the
        # proxy of the lab; the client runs TLS and GET over it (T-398).
        timeout 60 python3 -I "$dir/../exit-client.py" lab.test "$podnet" exit --stdio >"$dir/out.txt" 2>&1
        code=$?
        cat "$dir/out.txt"
        test $code -eq 0 && grep -q '^HTTP/1.1 200' "$dir/out.txt"
        return $?
        ;;
    net-curl)
        # The control of the lab: curl reaches the stand-in through the proxy.
        curl -sS -m 20 -o /dev/null -w '%{http_code}\n' "$url" >"$dir/out.txt" 2>&1
        code=$?
        cat "$dir/out.txt"
        test $code -eq 0 && grep -qx 200 "$dir/out.txt"
        return $?
        ;;
    esac
    backend=${name#net-pod-}
    if test "${LANE_PLANT:-}" = net; then
        # The plant: a proxy that does not listen.
        HTTPS_PROXY=http://127.0.0.1:1 https_proxy=http://127.0.0.1:1
        HTTP_PROXY=http://127.0.0.1:1 http_proxy=http://127.0.0.1:1
        export HTTPS_PROXY https_proxy HTTP_PROXY http_proxy
    fi
    timeout 60 "$podnet" pod --backend "$backend" -- \
        curl -sS -m 20 -o /dev/null -w '%{http_code}\n' "$url" >"$dir/out.txt" 2>&1
    code=$?
    cat "$dir/out.txt"
    echo "pod exit $code"
    test $code -eq 0 && grep -qx 200 "$dir/out.txt" && return 0
    if test $code -eq 3 && refusal_expected "pod-$backend" "$LANE_PROFILE"; then
        echo "podnet refused with exit 3 and a reason: $why"
        return 77
    fi
    return 1
}

in_list() {
    for i in $2; do
        test "$i" = "$1" && return 0
    done
    return 1
}

# The profiles that take what a pod backend needs, by design. It sets why.
refusal_expected() {
    case $1:$2 in
    pod-netns:userns-off | pod-netns:nonroot-nocaps | pod-netns:seccomp-nonewprivs)
        why="no user namespace"
        ;;
    pod-netns:bind-denied | pod-netns:testers)
        # Since T-382 the backend probes a TCP connect first.
        why="the lab refuses each TCP connect of the program (Landlock), in each namespace"
        ;;
    pod-netns:bind-netns-vm-denied)
        # pod-lab v0.70.0 (T-866): clone(CLONE_NEWUSER|CLONE_NEWNET) is EPERM.
        why="the lab refuses each new network namespace"
        ;;
    pod-netns:no-proc | pod-netns:proc-masked)
        why="no /proc for the id maps"
        ;;
    pod-seccomp:no-proc | pod-seccomp:proc-masked)
        why="no /proc/PID/mem of the program"
        ;;
    pod-netns:no-net-admin)
        # Since Linux 5.12, a map of the uid 0 of the parent needs
        # CAP_SETFCAP: root with no capability cannot map itself.
        test "$(id -u)" = 0 || return 1
        why="uid 0 with no capability cannot write its id map"
        ;;
    pod-netns:raw-denied)
        # The veth wire needs AF_PACKET; with /dev/net/tun the TUN works.
        test -r /dev/net/tun && test -w /dev/net/tun && return 1
        why="no /dev/net/tun for this user, and no AF_PACKET for the veth wire"
        ;;
    *) return 1 ;;
    esac
}

# A listener of podnet did not start. Where the profile refuses listen(2)
# of AF_UNIX (listen-denied), podnet cannot serve: not applicable, when
# podnet said so. Else a failure.
listen_refused() {
    if python3 -I "$1/../listen.py" "$1/probe.sock"; then
        return 1
    fi
    echo "the profile refuses listen(2) of AF_UNIX; podnet gave an error and stopped"
    return 77
}

wait_path() {
    i=0
    while ! test -S "$1"; do
        i=$((i + 1))
        test $i -gt 100 && return 1
        sleep 0.1
    done
}

write_helpers() {
    cat >"$1/exit-client.py" <<'EOF'
import os, ssl, subprocess, sys
# COMMAND (podnet exit --stdio): one SOCKS5 session on a pipe. CONNECT HOST:443, then
# TLS over the pipe with the roots of SSL_CERT_FILE, then GET /.
# Usage: exit-client.py HOST COMMAND...
host, command = sys.argv[1], sys.argv[2:]
p = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
w, r = p.stdin, p.stdout
def read(n):
    data = b""
    while len(data) < n:
        b = r.read(n - len(data))
        if not b:
            raise SystemExit("exit: the stream ended")
        data += b
    return data
w.write(b"\x05\x01\x00"); w.flush()
if read(2) != b"\x05\x00":
    raise SystemExit("exit: no method")
name = host.encode()
w.write(b"\x05\x01\x00\x03" + bytes([len(name)]) + name + (443).to_bytes(2, "big")); w.flush()
head = read(4)
if head[1] != 0:
    raise SystemExit("exit: CONNECT reply %d" % head[1])
if head[3] == 1:
    read(4)
elif head[3] == 4:
    read(16)
else:
    read(read(1)[0])
read(2)
ctx = ssl.create_default_context(cafile=os.environ.get("SSL_CERT_FILE"))
# The stand-in certificate of the lab has no Authority Key Identifier:
# check the chain and the name, not the strict profile of Python 3.13.
ctx.verify_flags &= ~getattr(ssl, "VERIFY_X509_STRICT", 0)
inc, out = ssl.MemoryBIO(), ssl.MemoryBIO()
t = ctx.wrap_bio(inc, out, server_hostname=host)
def pump():
    data = out.read()
    if data:
        w.write(data); w.flush()
while True:
    try:
        t.do_handshake()
        break
    except ssl.SSLWantReadError:
        pump()
        inc.write(r.read1(65536))
pump()
t.write(b"GET / HTTP/1.1\r\nHost: " + name + b"\r\nConnection: close\r\n\r\n")
pump()
reply = b""
while b"\r\n" not in reply:
    try:
        reply += t.read(65536)
    except ssl.SSLWantReadError:
        chunk = r.read1(65536)
        if not chunk:
            break
        inc.write(chunk)
print(reply.split(b"\r\n")[0].decode())
w.close(); p.wait(timeout=10)
EOF
    cat >"$1/socks-echo.py" <<'EOF'
import socket, sys
# SOCKS5 CONNECT IPV4:PORT through the unix door PATH, send ping, print the answer.
path, addr, port = sys.argv[1], sys.argv[2], int(sys.argv[3])
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(path)
s.sendall(b"\x05\x01\x00")
if s.recv(2) != b"\x05\x00":
    raise SystemExit("no method")
s.sendall(b"\x05\x01\x00\x01" + socket.inet_aton(addr) + port.to_bytes(2, "big"))
reply = s.recv(10)
if len(reply) < 2 or reply[1] != 0:
    raise SystemExit("CONNECT reply %r" % reply)
s.sendall(b"ping")
s.shutdown(socket.SHUT_WR)
data = b""
while True:
    b = s.recv(4096)
    if not b:
        break
    data += b
print(data.decode())
EOF
    cat >"$1/wgkeys.py" <<'EOF'
import base64, os
# X25519 (RFC 7748) for test keys made at run time: NAME PRIVATE PUBLIC.
P = 2**255 - 19
def x25519(k, u):
    k = bytearray(k); k[0] &= 248; k[31] &= 127; k[31] |= 64
    k = int.from_bytes(k, "little"); u = int.from_bytes(u, "little")
    x1, x2, z2, x3, z3, swap = u, 1, 0, u, 1, 0
    for t in reversed(range(255)):
        b = (k >> t) & 1
        swap ^= b
        if swap: x2, x3, z2, z3 = x3, x2, z3, z2
        swap = b
        A = x2 + z2; AA = A * A; B = x2 - z2; BB = B * B; E = AA - BB
        C = x3 + z3; D = x3 - z3; DA = D * A; CB = C * B
        x3 = (DA + CB) ** 2 % P; z3 = x1 * (DA - CB) ** 2 % P
        x2 = AA * BB % P; z2 = E * (AA + 121665 * E) % P
    if swap: x2, x3, z2, z3 = x3, x2, z3, z2
    return (x2 * pow(z2, P - 2, P) % P).to_bytes(32, "little")
for name in ("server", "client"):
    k = os.urandom(32)
    print(name, base64.b64encode(k).decode(), base64.b64encode(x25519(k, (9).to_bytes(32, "little"))).decode())
EOF
    cat >"$1/socks-stub.py" <<'EOF'
import os, socket, sys, threading
# A SOCKS5 server on the unix socket PATH: each CONNECT, to any target,
# gets success, and the stream gets the echo of the echo server.
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(8)
def read(c, n):
    data = b""
    while len(data) < n:
        b = c.recv(n - len(data))
        if not b:
            raise EOFError
        data += b
    return data
def serve(c):
    try:
        n = read(c, 2)[1]
        read(c, n)
        c.sendall(b"\x05\x00")
        head = read(c, 4)
        if head[3] == 1:
            read(c, 4)
        elif head[3] == 4:
            read(c, 16)
        else:
            read(c, read(c, 1)[0])
        read(c, 2)
        c.sendall(b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00")
        data = b""
        while True:
            b = c.recv(4096)
            if not b:
                break
            data += b
        c.sendall(b"echo:" + data)
    except EOFError:
        pass
    c.close()
while True:
    c, _ = s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
EOF
    cat >"$1/unix-client.py" <<'EOF'
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
s.sendall(b"ping")
s.shutdown(socket.SHUT_WR)
data = b""
while True:
    b = s.recv(4096)
    if not b:
        break
    data += b
print(data.decode())
EOF
    cat >"$1/echo.py" <<'EOF'
import os, socket, sys
path = sys.argv[1]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(path)
s.listen(8)
while True:
    c, _ = s.accept()
    data = b""
    while True:
        b = c.recv(4096)
        if not b:
            break
        data += b
    c.sendall(b"echo:" + data)
    c.close()
EOF
    cat >"$1/listen.py" <<'EOF'
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
try:
    s.listen(1)
except PermissionError:
    sys.exit(1)
EOF
    cat >"$1/client.py" <<'EOF'
import json, os, socket, sys
if sys.argv[1] == "--env":
    # Start this client again, dynamic, with the environment of the shim.
    env = dict(os.environ)
    with open(sys.argv[2]) as f:
        env.update(json.load(f))
    os.execve(sys.executable, [sys.executable, "-I", sys.argv[0]] + sys.argv[3:], env)
c = socket.create_connection((sys.argv[1], int(sys.argv[2])), timeout=10)
c.sendall(b"ping")
c.shutdown(socket.SHUT_WR)
data = b""
while True:
    b = c.recv(4096)
    if not b:
        break
    data += b
sys.stdout.write(data.decode())
EOF
    cat >"$1/connect.py" <<'EOF'
import socket, sys
c = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=10)
c.sendall(b"CONNECT %s HTTP/1.1\r\nHost: %s\r\n\r\n" % ((sys.argv[2].encode(),) * 2))
head = b""
while b"\r\n\r\n" not in head:
    b = c.recv(1)
    if not b:
        sys.exit("no reply of the proxy")
    head += b
if head.split()[1] != b"200":
    sys.exit("proxy: " + head.decode().splitlines()[0])
c.sendall(b"GET / HTTP/1.0\r\n\r\n")
data = b""
while True:
    b = c.recv(4096)
    if not b:
        break
    data += b
sys.stdout.write(data.split(b"\r\n\r\n", 1)[1].decode() + "\n")
EOF
    cat >"$1/socks.py" <<'EOF'
import socket, sys
c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
c.settimeout(10)
c.connect(sys.argv[1])
c.sendall(b"\x05\x01\x00")
sys.stdout.write(c.recv(2).hex())
EOF
}

if test "${1:-}" = "--case"; then
    case_main "$2" "$3" "$4"
    exit $?
fi

if test $# -lt 2; then
    echo "usage: sh scripts/lab-lane.sh PODLAB PODNET [PROFILE...]" >&2
    exit 64
fi

abs() { (cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd)" "$(basename "$1")"); }
podlab=$(abs "$1")
if test -d "$podlab"; then
    # A directory of scripts/get-podlab.sh: the asset of this machine,
    # checked against the SHA256SUMS of the release.
    asset="podlab-$(uname -m)-unknown-linux-musl"
    if ! (cd "$podlab" && grep " $asset\$" SHA256SUMS | sha256sum -c --status -); then
        echo "lab-lane: $podlab/$asset does not match $podlab/SHA256SUMS" >&2
        exit 1
    fi
    podlab="$podlab/$asset"
    test -x "$podlab" || { echo "lab-lane: $podlab is not executable" >&2; exit 1; }
fi
podnet=$(abs "$2")
self=$(abs "$0")
root=$(cd "$(dirname "$self")/.." && pwd)
shift 2
work=${LANE_DIR:-$PWD/.work/lab-lane}
rm -rf "$work"
mkdir -p "$work" || exit 1
work=$(cd "$work" && pwd)
case $work in
/tmp | /tmp/*) echo "lab-lane: LANE_DIR $work is in /tmp; a lab hides it" >&2; exit 64 ;;
esac
write_helpers "$work"
cp "$root/tests/consumers/python-http/client.py" "$work/http-client.py" || exit 1
cp "$root/tests/consumers/connect-proxy.py" "$root/tests/consumers/servers.py" "$work/" || exit 1
python3 -I "$work/echo.py" "$work/echo.sock" &
pids=$!
# $pids is a list of numbers: split it.
# shellcheck disable=SC2086
trap 'kill $pids 2>/dev/null' EXIT
i=0
while ! test -S "$work/echo.sock"; do
    i=$((i + 1))
    test $i -gt 100 && { echo "lab-lane: the echo server did not start" >&2; exit 1; }
    sleep 0.1
done

LANE_PROXY_PORT=${LANE_PROXY_PORT:-7120}
LANE_UPSTREAM_PORT=$LANE_PROXY_PORT
proxy_mode=
case ${LANE_PLANT:-} in
'') ;;
refuse) proxy_mode=refuse ;;
port) LANE_UPSTREAM_PORT=$((LANE_PROXY_PORT + 2)) ;;
net) ;;
*) echo "lab-lane: LANE_PLANT is refuse, port or net" >&2; exit 64 ;;
esac
export LANE_PROXY_PORT LANE_UPSTREAM_PORT

# The stand-in proxy and its server, outside the lab.
start_proxy() {
    tport=$((LANE_PROXY_PORT + 1))
    python3 -I "$work/servers.py" http-tcp "$tport" podnet-proxy-only >"$work/target.out" 2>&1 &
    pids="$pids $!"
    : >"$work/proxy.txt"
    python3 -I "$work/connect-proxy.py" "$LANE_PROXY_PORT" "$work/proxy.txt" \
        "proxy.test:$tport=$tport" $proxy_mode >"$work/proxy.out" 2>&1 &
    pids="$pids $!"
    i=0
    while ! grep -qx ready "$work/proxy.out" 2>/dev/null; do
        i=$((i + 1))
        test $i -gt 100 && { cat "$work/proxy.out"; return 1; }
        sleep 0.1
    done
}

if test $# -eq 0; then
    # Each profile name is one word with no space or glob character: the
    # split of the list into the arguments is the intent.
    # shellcheck disable=SC2046
    set -- $("$podlab" list | awk '/^[^ ]/ { print $1 }') allow-connect testers way-out way-ws
fi

failed=0
printf '%-22s %-17s %s\n' PROFILE COMMAND RESULT
for profile in "$@"; do
    # allow-connect: the lab bind-denied with a connect to the proxy port
    # only. A network lab runs the rows of NET_CASES.
    lab=$profile
    cases=$CASES
    mem_hook=
    if test "$profile" = testers; then
        lab=bind-denied
        cases=$TESTER_CASES
        mem_hook=none
    elif test "$profile" = allow-connect; then
        lab="--allow-connect $LANE_PROXY_PORT bind-denied"
        cases=$PROXY_CASES
    elif test "$profile" = way-out; then
        lab="--way-out proxy-only"
        cases=$WAYOUT_CASES
    elif test "$profile" = way-ws; then
        lab="--way-out ws-upgrade-refused"
        cases=$WAYWS_CASES
    elif in_list "$profile" "$NET_LABS"; then
        cases=$NET_CASES
    fi
    # $lab is the profile, or options and the profile: split it.
    # shellcheck disable=SC2086
    "$podlab" run $lab -- /bin/true >"$work/$profile.lab.txt" 2>&1
    code=$?
    if test $code -ne 0; then
        why=$(grep -v '^ ' "$work/$profile.lab.txt" | grep -v '^holds' | head -n 1)
        printf '%-22s %-17s n.a.: the lab does not hold (exit %s): %s\n' "$profile" '*' "$code" "$why"
        continue
    fi
    if test "$profile" = allow-connect && ! start_proxy; then
        printf '%-22s %-17s fail: the stand-in proxy did not start\n' "$profile" '*'
        failed=1
        continue
    fi
    for c in $cases; do
        log="$work/$profile.$c.txt"
        : >"$work/proxy.txt"
        # shellcheck disable=SC2086
        PODNET_NETNS_TEST_MEM=$mem_hook LANE_PROFILE=$profile "$podlab" run $lab -- /bin/sh "$self" --case "$c" "$podnet" "$work/$profile-$c" >"$log" 2>&1
        code=$?
        case $code in
        0) result=pass ;;
        77) result="n.a.: $(tail -n 1 "$log")" ;;
        *)
            result="fail (exit $code, $log)"
            failed=1
            ;;
        esac
        printf '%-22s %-17s %s\n' "$profile" "$c" "$result"
    done
done
exit $failed
