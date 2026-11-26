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

CASES="version man local local-files local-udp local-tcp errors doctor shim-forward link-unix link-control net-switch wg-serve pod-seccomp pod-netns net-proxy-unix consumer-podrunner"
NET_LABS="proxy-only proxy-tls-reterminate proxy-per-name ws-upgrade-refused sni-allowlist no-proxy-no-dns ipv6-only"
NET_CASES="version errors doctor net-curl net-bench net-exit net-pod-auto net-pod-seccomp net-pod-netns"
WAYOUT_CASES="way-tunnel way-relay"
WAYWS_CASES="way-relay-refused"
TESTER_CASES="pod-seccomp pod-netns errors"
# The labs of the case docs (the documented commands): the restricted
# hosts of docs/target-environment.md.
DOC_LABS="proxy-only bind-denied nonroot-nocaps no-net-admin no-loopback udp-denied listen-denied"
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
    local-files)
        # T-398: the commands that read and write local files only: the
        # host probe, the policy and the node configuration, the CI
        # declaration, the spool, the bundle, the pairing and the clone of a
        # seed. Each exits 0 with its words.
        cd "$dir" || exit 1
        step() {
            want=$1
            shift
            timeout 60 "$@" >"$dir/files.txt" 2>"$dir/files.err"
            code=$?
            shift
            echo "$* -> exit $code"
            test $code -eq 0 || { cat "$dir/files.err"; return 1; }
            test -z "$want" || grep -q -- "$want" "$dir/files.txt" || { echo "no line with: $want"; head -5 "$dir/files.txt"; return 1; }
        }
        step '"kind": "podnet.host-probe/1"' "$podnet" host probe "$dir" --json || exit 1
        printf '{"default":"direct","rules":[{"name":"no-x","action":"refuse","host":"x.example"}]}' >pol.json
        step '1 rules, default direct' "$podnet" rules check pol.json || exit 1
        step '' "$podnet" init generate --relay https://relay.example lab-node --out node.json || exit 1
        step 'node lab-node' "$podnet" init check node.json || exit 1
        printf '{"schema":1,"kind":"podnet.ci-egress/1","hosts":[{"host":"lab.test","port":443}]}' >decl.json
        export PODNET_POLICY="$dir/pol.json"
        step '1 need(s) covered, 0 refused' "$podnet" ci check decl.json || exit 1
        unset PODNET_POLICY
        echo "a report" >body.txt
        step '' "$podnet" offline spool add ./spool --file body.txt || exit 1
        step '^0 ' "$podnet" offline spool list ./spool || exit 1
        step 'no fault' "$podnet" offline spool check ./spool || exit 1
        step '' "$podnet" offline spool take ./spool --out taken.txt || exit 1
        cmp -s body.txt taken.txt || { echo "the taken body differs"; exit 1; }
        mkdir -p site && echo one >site/a.txt
        step '^bundle field ' "$podnet" offline bundle build --out old --name field --file ./site:site || exit 1
        echo two >site/b.txt
        step '^delta field ' "$podnet" offline bundle build --out delta --name field --base ./old --file ./site:site || exit 1
        step '^bundle field ' "$podnet" offline bundle verify old || exit 1
        step '^applied field ' "$podnet" offline bundle apply --base ./old --delta ./delta || exit 1
        timeout 60 "$podnet" offline pair offer --out offer.txt >code.txt 2>"$dir/files.err" || { cat "$dir/files.err"; exit 1; }
        timeout 60 "$podnet" offline pair accept --offer offer.txt --used ./used <code.txt >"$dir/files.txt" 2>"$dir/files.err"
        code=$?
        echo "offline pair accept -> exit $code"
        test $code -eq 0 && cat "$dir/files.txt" "$dir/files.err" | grep -q '^paired with ' || { cat "$dir/files.err"; exit 1; }
        # The replica logs of two actors (the public keys of RFC 8032, test
        # 1 and 2) write one key at the same time: one conflict, and the key
        # of test 1 wins (the greater hex breaks an equal counter).
        hx() { printf %s "$1" | od -An -tx1 | tr -d ' \n'; }
        a=d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a
        b=3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c
        echo "$a 1 $(hx session:mode) $a:1 $(hx one)" >a.log
        echo "$b 2 $(hx session:mode) $b:1 $(hx two)" >b.log
        mkdir -p key
        step "conflict session:mode .* winner_key=$a" "$podnet" offline replica merge a.log b.log --out m1.log --key ./key || exit 1
        step '"records":2' "$podnet" offline replica merge b.log a.log --out m2.log --key ./key --json || exit 1
        cmp -s m1.log m2.log || { echo "the two orders of the merge differ"; exit 1; }
        # Two captures, and a secret that the merge must redact.
        printf 'pod-offline capture 1\n200 in 0 %s\n' "$(hx hi)" >a.cap
        printf 'pod-offline capture 1\n100 out 0 %s\n' "$(hx 'tok lab-secret-1')" >b.cap
        echo lab-secret-1 >secrets.txt
        step '^merged 2 record(s) into all.cap, 1 marked redacted' "$podnet" offline capture merge a.cap b.cap --out all.cap --secret-file secrets.txt || exit 1
        if grep -q "$(hx lab-secret-1)" all.cap; then echo "the secret is in the merged capture"; exit 1; fi
        # T-358 step 1: the icmp carrier (the release set) on a host that
        # gives no raw socket: exit 69 with the cause. Root has the socket.
        if test "$(id -u)" != 0; then
            key=d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a
            timeout 60 "$podnet" link dial "$key@127.0.0.1" --carrier icmp --allow-raw icmp --peer "$key" --check >"$dir/files.txt" 2>"$dir/files.err"
            code=$?
            echo "link dial --carrier icmp -> exit $code"
            if grep -q 'feature icmp is off' "$dir/files.err"; then
                echo "this binary has no icmp carrier"
            else
                test $code -eq 69 && grep -q 'CAP_NET_RAW' "$dir/files.err" || { cat "$dir/files.err"; exit 1; }
            fi
        fi
        step '"registered": false' "$podnet" init enroll --node lab-node --relay https://relay.example --offline enrolled --json || exit 1
        test -s enrolled/link-identity.key || { echo "init enroll wrote no key"; exit 1; }
        # The verification matrix over the fixtures of tests/data/harness.
        step '^conditions: host ' "$podnet" harness --data "$dir/../harness-data" || exit 1
        if command -v git >/dev/null 2>&1; then
            git init -q seed && git -C seed -c user.email=lab@lab.test -c user.name=lab commit -q --allow-empty -m seed || exit 1
            step '^cloned seed ' "$podnet" forge clone --seed seed --expect "$(git -C seed rev-parse HEAD)" clone || exit 1
        else
            echo "no git on this host: forge clone is not run"
        fi
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
    local-tcp)
        # T-398: the commands that listen and dial TCP on the loopback only:
        # link p2p on its direct path to an echo peer, and the cover site of
        # podnet cover to a TLS client with no switch key.
        if ! python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(1); c = socket.create_connection(s.getsockname(), 2)' 2>/dev/null; then
            echo "the lab refuses TCP on the loopback"
            exit 77
        fi
        cd "$dir" || exit 1
        free() { python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'; }
        port=$(free)
        python3 -I -c '
import socket, sys
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(4); s.settimeout(60)
for _ in range(2):
    c, _ = s.accept(); data = c.recv(100); c.sendall(b"echo:" + data); c.close()
' "$port" &
        peer=$!
        sleep 1
        timeout 60 "$podnet" link p2p --peer "127.0.0.1:$port" --check --json >p2p.json 2>p2p.err
        code=$?
        echo "link p2p --check -> exit $code"
        test $code -eq 0 && grep -q '"path":"direct"' p2p.json || { cat p2p.json p2p.err; kill $peer 2>/dev/null; exit 1; }
        echo ping | timeout 60 "$podnet" link p2p --peer "127.0.0.1:$port" >p2p.txt 2>p2p.err
        code=$?
        echo "link p2p -> exit $code"
        test $code -eq 0 && grep -q 'echo:ping' p2p.txt || { cat p2p.txt p2p.err; kill $peer 2>/dev/null; exit 1; }
        wait $peer
        if ! command -v openssl >/dev/null 2>&1; then
            echo "no openssl on this host: cover is not run"
            exit 0
        fi
        openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout cover.key -out cover.pem -days 2 \
            -subj /CN=site.lab -addext subjectAltName=DNS:site.lab >/dev/null 2>&1 || { echo "openssl made no certificate"; exit 1; }
        mkdir -p site && echo '<p>the lab cover</p>' >site/index.html
        python3 -I -c 'import os; print(os.urandom(32).hex())' >switch.key
        port=$(free)
        "$podnet" cover --listen "127.0.0.1:$port" --site site --script "$dir/../cover-script.toml" --cert cover.pem \
            --key cover.key --switch-key switch.key --to 127.0.0.1:9 >cover.txt 2>cover.err &
        cover=$!
        python3 -I -c '
import socket, ssl, sys, time
port = int(sys.argv[1])
for _ in range(50):
    try:
        raw = socket.create_connection(("127.0.0.1", port), 2); break
    except OSError:
        time.sleep(0.2)
else:
    raise SystemExit("the cover does not listen")
ctx = ssl.create_default_context(); ctx.check_hostname = False; ctx.verify_mode = ssl.CERT_NONE
t = ctx.wrap_socket(raw, server_hostname="site.lab")
t.sendall(b"GET / HTTP/1.1\r\nHost: site.lab\r\n\r\n")
data = b""
while b"the lab cover" not in data:
    b = t.recv(4096)
    if not b: break
    data += b
print(data.decode(errors="replace"))
' "$port" >cover.out 2>&1
        code=$?
        kill $cover 2>/dev/null
        echo "cover -> exit $code"
        test $code -eq 0 && grep -q '^HTTP/1.1 200' cover.out && grep -q 'the lab cover' cover.out || { cat cover.out cover.err; exit 1; }
        ;;
    local-udp)
        # T-398: the commands that send UDP on the loopback only: a signed
        # announcement to a unicast listener, and the timestamping peer of
        # podnet stat.
        if ! python3 -I -c 'import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", 0)); s.sendto(b"x", s.getsockname())' 2>/dev/null; then
            echo "the lab refuses UDP on the loopback"
            exit 77
        fi
        cd "$dir" || exit 1
        free() { python3 -I -c 'import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'; }
        port=$(free)
        mkdir -p key
        timeout 60 "$podnet" offline discover listen --bind "127.0.0.1:$port" --timeout 8 --json >disc.json 2>disc.err &
        listen=$!
        # The listener binds after its start: send until it ends.
        i=0
        while kill -0 "$listen" 2>/dev/null && test $i -lt 14; do
            "$podnet" offline discover announce --to "127.0.0.1:$port" --name lab-a --address lab.test --port 22 --key ./key >announce.txt 2>announce.err ||
                { cat announce.err; exit 1; }
            i=$((i + 1))
            sleep 0.5
        done
        wait "$listen"
        code=$?
        echo "offline discover listen -> exit $code"
        test $code -eq 0 || { cat disc.err; exit 1; }
        grep -q '"name":"lab-a","address":"lab.test","port":22' disc.json || { echo "the listener heard no peer"; cat disc.json; exit 1; }
        grep -q '^announced lab-a lab.test:22 ' announce.txt || { cat announce.txt; exit 1; }
        port=$(free)
        "$podnet" stat --serve "127.0.0.1:$port" >serve.txt 2>serve.err &
        serve=$!
        sleep 2
        timeout 60 "$podnet" stat --peer "127.0.0.1:$port" --count 20 --out stat.txt >stat.out 2>stat.err
        code=$?
        kill "$serve" 2>/dev/null
        echo "stat -> exit $code"
        test $code -eq 0 || { cat stat.err; exit 1; }
        grep -q '^received 20$' stat.txt || { echo "the peer did not stamp each packet"; tail -8 stat.txt; exit 1; }
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
    docs)
        # T-358 step 2 (c): the documented commands of the classes run and
        # scenario (scripts/doc-commands.py) in the lab, with this binary.
        # The outer script copies the documents and the set-ups to
        # doc-tree. A set-up that the lab cannot give makes its block n.a.
        # A block that needs a program that the host does not have is MISS:
        # the row does not fail for it.
        python3 -I "$dir/../doc-tree/scripts/doc-commands.py" --run --miss-ok --bin "$podnet" >"$dir/docs.txt" 2>&1
        code=$?
        cat "$dir/docs.txt"
        exit $code
        ;;
    errors)
        # The error rows (T-452): scripts/lab-errors.sh, beside this file.
        sh "$(dirname "$0")/lab-errors.sh" "$podnet" "$dir"
        exit $?
        ;;
    consumer-podrunner)
        consumer_case "$name" "$podnet" "$dir"
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

# The case consumer-podrunner (T-358 step 3): the calls of podnet that
# PodUtils/podrunner makes, as it makes them, with a stand-in of its egress
# broker: an HTTP CONNECT proxy on a unix socket (podrunner-standin.py).
# Each row prints "row NAME: pass", "row NAME: n.a.: REASON" or "row NAME:
# fail: REASON". The case fails when a row fails.
consumer_case() {
    podnet=$2
    dir=$3
    bad=0
    row() {
        echo "row $1: $2"
        case $2 in fail*) bad=1 ;; esac
    }
    unset HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy NO_PROXY no_proxy
    # The version line (crates/podrunner/src/serve/host.rs).
    first=$("$podnet" --version | head -n 1)
    echo "version: $first"
    if printf '%s\n' "$first" | grep -Eq '^podnet [0-9][^ ]*( |$)'; then
        row version pass
    else
        row version "fail: the first line is not 'podnet VERSION'"
    fi
    consumer_doctor "$podnet" "$dir"
    # The stand-in broker: in the lab, else the one outside (listen-denied).
    sock="$dir/egress.sock"
    log="$dir/egress.log"
    : >"$log"
    python3 -I "$dir/../podrunner-standin.py" "$sock" "$log" >"$dir/standin.txt" 2>&1 &
    standin=$!
    i=0
    while ! grep -qx ready "$dir/standin.txt" && kill -0 $standin 2>/dev/null && test $i -lt 100; do
        i=$((i + 1))
        sleep 0.1
    done
    if ! grep -qx ready "$dir/standin.txt"; then
        cat "$dir/standin.txt"
        sock="$dir/../podrunner-egress.sock"
        log="$dir/../podrunner-egress.log"
        test -S "$sock" && echo "the stand-in in the lab did not start: the stand-in outside the lab serves"
    fi
    if ! test -S "$sock"; then
        for r in pod-netns detach-netns pod-seccomp detach-seccomp under-shim; do
            row $r "n.a.: no stand-in broker (the lab refuses listen(2) of AF_UNIX, and none runs outside)"
        done
        sock=
    fi
    # The loopback listeners of the seccomp rules: the port of the broker
    # (direct) and one other port (the rules refuse it).
    loop=
    python3 -I "$dir/../podrunner-loop.py" "$dir/loop.ports" >"$dir/loop.txt" 2>&1 &
    lpid=$!
    i=0
    while ! test -s "$dir/loop.ports" && kill -0 $lpid 2>/dev/null && test $i -lt 50; do
        i=$((i + 1))
        sleep 0.1
    done
    if test -s "$dir/loop.ports"; then
        loop=$(cat "$dir/loop.ports")
    else
        cat "$dir/loop.txt"
    fi
    q=$(python3 -I -c 'import random; print(random.randint(20000, 60000))')
    for b in netns seccomp; do
        test -n "$sock" || break
        consumer_pod "$podnet" "$dir" $b
    done
    consumer_shim "$podnet" "$dir"
    echo "the log of the stand-in:"
    cat "$log"
    kill $standin $lpid 2>/dev/null
    return $bad
}

# pod doctor --json (crates/podrunner-podvm/src/unit/hold.rs): exit 0 or
# 3, JSON on stdout in both cases, a known level of each backend. It sets
# netns and seccomp to the levels.
consumer_doctor() {
    netns=none
    seccomp=none
    "$1" pod doctor --json </dev/null >"$2/doctor.json" 2>"$2/doctor.err"
    code=$?
    echo "pod doctor --json -> exit $code"
    levels=$(python3 -I -c '
import json, sys
v = json.load(open(sys.argv[1]))
def level(name):
    h = v.get("hold") if isinstance(v.get("hold"), dict) else {}
    return (h.get(name) or {}).get("level") or (v.get(name) or {}).get("level")
print(level("netns"), level("seccomp"))' "$2/doctor.json" 2>&1)
    pcode=$?
    echo "levels: $levels"
    if test $code -ne 0 && test $code -ne 3; then
        cat "$2/doctor.err"
        row pod-doctor "fail: exit $code (podrunner takes 0 or 3)"
        return
    fi
    if test $pcode -ne 0; then
        head -c 300 "$2/doctor.json"
        row pod-doctor "fail: the output is not JSON"
        return
    fi
    # Two words: the split is the intent.
    # shellcheck disable=SC2086
    set -- $levels
    for l in "$1" "$2"; do
        case $l in
        enforced | advisory | none) ;;
        *)
            row pod-doctor "fail: a level is '$l', not enforced, advisory or none"
            return
            ;;
        esac
    done
    netns=$1
    seccomp=$2
    row pod-doctor pass
}

# The rows pod-B and detach-B: podnet pod --backend B as podrunner starts
# podvm (crates/podrunner-podvm/src/unit/hold.rs), and a detached payload
# (podvm run -d) that connects after podnet pod returned.
consumer_pod() {
    podnet=$1
    dir=$2
    b=$3
    lvl=$seccomp
    test $b = netns && lvl=$netns
    if test "$lvl" = none; then
        row pod-$b "n.a.: pod doctor gives the $b level none"
        row detach-$b "n.a.: pod doctor gives the $b level none"
        return
    fi
    rules=
    extra=
    note=
    if test $b = seccomp; then
        if test -n "$loop"; then
            # Two port numbers: the split is the intent.
            # shellcheck disable=SC2086
            set -- $loop
            rules="--loopback-rules -r tcp:127.0.0.1:$1=direct -r cidr:127.0.0.0/8=reject -r cidr:::1/128=reject"
            extra="$1 $2"
        else
            note=" (no loopback check: no TCP listener on the loopback in this lab)"
        fi
    fi
    q=$((q + 1))
    # podrunner gives each word: $rules and $extra split on purpose.
    # shellcheck disable=SC2086
    PODVM_SPARE_TREE=0 timeout 60 "$podnet" pod --backend $b -x "http://unix:$sock" $rules -- \
        python3 -I "$dir/../podrunner-payload.py" 10.255.0.1 $q 7 $extra >"$dir/pod-$b.txt" 2>&1
    code=$?
    cat "$dir/pod-$b.txt"
    echo "pod --backend $b -> exit $code"
    if test $code -ne 7; then
        row pod-$b "fail: exit $code, not the exit 7 of the program$note"
    elif ! grep -qx "CONNECT 10.255.0.1:$q" "$log"; then
        row pod-$b "fail: the stand-in logged no CONNECT 10.255.0.1:$q"
    else
        row pod-$b "pass$note"
    fi
    q=$((q + 1))
    late="python3 -I $dir/../podrunner-payload.py 10.255.0.1 $q 0 $extra >$dir/late-$b.txt 2>&1"
    t0=$(date +%s)
    # shellcheck disable=SC2086
    PODVM_SPARE_TREE=0 timeout 60 "$podnet" pod --backend $b -x "http://unix:$sock" $rules -- \
        sh -c "(sleep 3; $late) & exit 0" >"$dir/detach-$b.txt" 2>&1
    code=$?
    t1=$(date +%s)
    cat "$dir/detach-$b.txt"
    echo "pod --backend $b (detached) -> exit $code after $((t1 - t0)) s"
    i=0
    while ! grep -qx "CONNECT 10.255.0.1:$q" "$log" && test $i -lt 150; do
        i=$((i + 1))
        sleep 0.1
    done
    sleep 1
    cat "$dir/late-$b.txt" 2>/dev/null
    if test $code -ne 0; then
        row detach-$b "fail: exit $code, not the exit 0 of sh"
    elif test $((t1 - t0)) -gt 2; then
        row detach-$b "fail: podnet pod returned after $((t1 - t0)) s, not at once"
    elif ! grep -qx "CONNECT 10.255.0.1:$q" "$log"; then
        row detach-$b "fail: the late connect did not reach the stand-in in 15 s"
    else
        row detach-$b pass
    fi
}

# The rows net-shim (crates/podrunner/src/run_local/vnet.rs) and under-shim:
# a runner under the shim (crates/podrunner-local/src/exec/vnet.rs).
consumer_shim() {
    podnet=$1
    dir=$2
    mkdir -p "$dir/shim"
    shim=
    "$podnet" net shim --json --out "$dir/shim" </dev/null >"$dir/shim.json" 2>"$dir/shim.err"
    code=$?
    cat "$dir/shim.json" "$dir/shim.err"
    echo "net shim -> exit $code"
    if test $code -ne 0; then
        if test -z "${CC:-}" && ! command -v cc >/dev/null && ! command -v gcc >/dev/null &&
            ! command -v clang >/dev/null && test "$(wc -l <"$dir/shim.err")" -eq 1; then
            row net-shim "n.a.: no C compiler, and podnet says so in one line"
        else
            row net-shim "fail: exit $code"
        fi
    else
        # The split of podrunner: the first absolute path that ends in the
        # name of the shim, and the magic of ELF.
        shim=$(python3 -I -c '
import re, sys
for t in re.split(r"[\":= \s]", open(sys.argv[1]).read()):
    if t.startswith("/") and t.endswith("/libpodnet-vnet.so"):
        with open(t, "rb") as f:
            if f.read(4) == b"\x7fELF":
                print(t)
        break' "$dir/shim.json")
        if test -n "$shim"; then
            row net-shim pass
        else
            row net-shim "fail: no absolute path of an ELF libpodnet-vnet.so on stdout"
        fi
    fi
    test -n "$sock" || return
    if test -z "$shim"; then
        row under-shim "n.a.: no shim"
        return
    fi
    # The control: with no shim, the stand-in gets no CONNECT.
    env HTTPS_PROXY=http://127.0.0.1:3128 NO_PROXY= NODE_USE_ENV_PROXY=1 \
        timeout 30 python3 -I "$dir/../podrunner-https.py" "https://control-$q.test/" >"$dir/control.txt" 2>&1
    env LD_PRELOAD="$shim" PODNET_VNET_FORWARD="127.0.0.1:3128=unix:$sock" HTTPS_PROXY=http://127.0.0.1:3128 \
        NO_PROXY= NODE_USE_ENV_PROXY=1 \
        timeout 30 python3 -I "$dir/../podrunner-https.py" "https://shim-$q.test/" >"$dir/under.txt" 2>&1
    code=$?
    cat "$dir/control.txt" "$dir/under.txt"
    echo "the client under the shim -> exit $code"
    if grep -q "^CONNECT control-$q.test:" "$log"; then
        row under-shim "fail: the control with no shim reached the stand-in"
    elif grep -qx "CONNECT shim-$q.test:443" "$log"; then
        row under-shim pass
    else
        row under-shim "fail: the stand-in logged no CONNECT shim-$q.test:443"
    fi
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
    cat >"$1/podrunner-standin.py" <<'EOF'
import os, socket, sys, threading
# The stand-in of the egress broker of podrunner: an HTTP CONNECT proxy on
# the unix socket PATH. It appends "CONNECT TARGET" to LOG for each request
# and answers 200. Then it reads the first bytes of the client, answers
# "standin:" and these bytes, and closes. It prints "ready" when it listens.
# Usage: podrunner-standin.py PATH LOG
path, log = sys.argv[1], sys.argv[2]
try:
    os.unlink(path)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(path)
try:
    s.listen(16)
except PermissionError as e:
    sys.exit("listen: %s" % e)
print("ready", flush=True)
lock = threading.Lock()
def serve(c):
    c.settimeout(30)
    try:
        head = b""
        while b"\r\n\r\n" not in head:
            b = c.recv(4096)
            if not b:
                return
            head += b
        words = head.split(b"\r\n", 1)[0].decode("ascii", "replace").split()
        with lock, open(log, "a") as f:
            f.write(" ".join(words[:2]) + "\n")
        if len(words) != 3 or words[0] != "CONNECT":
            c.sendall(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
            return
        c.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
        rest = head.split(b"\r\n\r\n", 1)[1]
        c.sendall(b"standin:" + (rest or c.recv(4096)))
    except OSError:
        pass
    finally:
        c.close()
while True:
    c, _ = s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
EOF
    cat >"$1/podrunner-loop.py" <<'EOF'
import socket, sys, threading
# Two TCP listeners on 127.0.0.1: each answers "direct:" and the first bytes
# of the client. It writes "PORT1 PORT2" to FILE when both answer.
socks = []
for _ in range(2):
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    s.listen(8)
    socks.append(s)
def serve(s):
    while True:
        c, _ = s.accept()
        try:
            c.sendall(b"direct:" + c.recv(4096))
        except OSError:
            pass
        c.close()
for s in socks:
    threading.Thread(target=serve, args=(s,), daemon=True).start()
# The control: each port answers a connect with no pod.
for s in socks:
    c = socket.create_connection(s.getsockname(), timeout=5)
    c.sendall(b"x")
    if c.recv(100) != b"direct:x":
        sys.exit("a listener does not answer")
with open(sys.argv[1] + ".tmp", "w") as f:
    f.write("%d %d\n" % (socks[0].getsockname()[1], socks[1].getsockname()[1]))
import os
os.rename(sys.argv[1] + ".tmp", sys.argv[1])
threading.Event().wait()
EOF
    cat >"$1/podrunner-payload.py" <<'EOF'
import socket, sys
# The payload of a held unit: connect HOST:PORT (the stand-in answers
# "standin:ping"). With DIRECT and REFUSED: 127.0.0.1:DIRECT answers
# "direct:ping", and the rules refuse 127.0.0.1:REFUSED. Then exit CODE.
# Usage: podrunner-payload.py HOST PORT CODE [DIRECT REFUSED]
def ask(host, port):
    c = socket.create_connection((host, port), timeout=10)
    c.sendall(b"ping")
    data = b""
    while True:
        b = c.recv(4096)
        if not b:
            break
        data += b
    return data.decode(errors="replace")
host, port, code = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
got = ask(host, port)
print("%s:%d -> %s" % (host, port, got))
if got != "standin:ping":
    sys.exit(1)
if len(sys.argv) > 5:
    got = ask("127.0.0.1", int(sys.argv[4]))
    print("127.0.0.1:%s -> %s" % (sys.argv[4], got))
    if got != "direct:ping":
        sys.exit(1)
    try:
        got = ask("127.0.0.1", int(sys.argv[5]))
        print("127.0.0.1:%s -> %s: not refused" % (sys.argv[5], got))
        sys.exit(1)
    except ConnectionRefusedError:
        print("127.0.0.1:%s -> refused" % sys.argv[5])
sys.exit(code)
EOF
    cat >"$1/podrunner-https.py" <<'EOF'
import sys, urllib.request
# An HTTPS client that takes HTTPS_PROXY from the environment. The stand-in
# answers no TLS: the CONNECT is the proof, not the answer.
try:
    urllib.request.urlopen(sys.argv[1], timeout=10)
except Exception as e:
    print("client: %s" % e)
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
cp -r "$root/tests/data/harness" "$work/harness-data" || exit 1
cp "$root/tests/data/cover/script.toml" "$work/cover-script.toml" || exit 1
cp "$root/tests/consumers/python-http/client.py" "$work/http-client.py" || exit 1
cp "$root/tests/consumers/connect-proxy.py" "$root/tests/consumers/servers.py" "$work/" || exit 1
# The case docs: the documents, the classes and the set-ups of
# scripts/doc-commands.py, and the files of the set-ups, at their paths of
# the repository.
mkdir -p "$work/doc-tree/docs" "$work/doc-tree/scripts" "$work/doc-tree/tests/consumers" "$work/doc-tree/tests/data" &&
    cp "$root/README.md" "$work/doc-tree/" &&
    cp "$root"/docs/*.md "$work/doc-tree/docs/" &&
    cp "$root/scripts/doc-commands.py" "$root/scripts/doc-commands.txt" "$work/doc-tree/scripts/" &&
    cp -r "$root/scripts/doc-scenarios" "$work/doc-tree/scripts/" &&
    cp "$root/tests/consumers/servers.py" "$work/doc-tree/tests/consumers/" &&
    cp -r "$root/tests/data/harness" "$work/doc-tree/tests/data/" || exit 1
python3 -I "$work/echo.py" "$work/echo.sock" &
pids=$!
# The stand-in broker of consumer-podrunner for a lab that refuses
# listen(2) of AF_UNIX.
python3 -I "$work/podrunner-standin.py" "$work/podrunner-egress.sock" "$work/podrunner-egress.log" \
    >"$work/podrunner-standin.txt" 2>&1 &
pids="$pids $!"
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
    if in_list "$profile" "$DOC_LABS"; then
        cases="$cases docs"
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
