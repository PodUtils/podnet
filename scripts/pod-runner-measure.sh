#!/bin/sh
# The measure of T-229: podnet pod with the runner of actions/runner and CI
# payloads, under the seccomp backend, an allow list of rules and
# --default-action reject.
#
#   sh scripts/pod-runner-measure.sh [PODNET] [REPORT]
#
# PODNET is the binary (default target/debug/podnet). REPORT is the report to
# write (default docs/reports/pod-runner.md).
#
# Each payload runs in its own pod. Three witnesses see its flows:
#   * the flow log of the pod (--flow-log);
#   * a witness proxy on a Unix socket (scripts/pod-runner-proxy.py): the
#     -x chain and each rule go through it, and it logs each CONNECT;
#   * strace (when it is installed): each connect, bind, sendto and sendmsg
#     with an address, and the type of each socket, also of the ways that
#     the pod does not hold.
#
# Environment:
#   UPSTREAM       the HTTP proxy that the witness proxy uses for each tunnel
#                  (default: $HTTPS_PROXY; empty: direct).
#   CA_BUNDLE      a CA file that the payloads trust (a host with a proxy that
#                  inspects TLS); empty: the store of the system.
#   GITHUB_TOKEN   the token of actions/checkout (default: a token that is
#                  not valid; a public repository may refuse it).
#   BACKEND        the pod backend (default: seccomp).
#   WORK           the work directory (default: a new temporary directory).
set -eu

podnet=${1:-target/debug/podnet}
report=${2:-docs/reports/pod-runner.md}
here=$(cd "$(dirname "$0")" && pwd)
case $podnet in /*) ;; *) podnet=$(pwd)/$podnet ;; esac
case $report in /*) ;; *) report=$(pwd)/$report ;; esac
backend=${BACKEND:-seccomp}
upstream=${UPSTREAM-${HTTPS_PROXY:-}}
work=${WORK:-$(mktemp -d)}
mkdir -p "$work"
# A Unix socket path has at most 107 bytes: the sockets go to a short directory.
socks=$(mktemp -d /tmp/prm.XXXXXX)
trap 'rm -rf "$socks"' EXIT

runner_version=2.338.2
runner_sha=baebf204307bac6193480852732f0aba2ae97b6c5597c244f4d5558390147c40
checkout_ref=v4.2.2
checkout_sha=9a320e563c4dc21e2291453f0c922906ef8d98cbf4961dfadb2eb4f1a0fcc15e
setup_node_ref=v4.4.0
setup_node_sha=1aa014a3691d373745d38a1380aaea96148f4f6c33ea3d7e5fc8b2d305e328c2

fetch() { # URL FILE SHA256
    if [ ! -s "$2" ]; then
        curl -qfsSL -o "$2.part" "$1"
        mv "$2.part" "$2"
    fi
    echo "$3  $2" | sha256sum -c --quiet -
}

echo "pod-runner-measure: the payloads go to $work"
fetch "https://github.com/actions/runner/releases/download/v$runner_version/actions-runner-linux-x64-$runner_version.tar.gz" \
    "$work/runner.tgz" "$runner_sha"
if [ ! -x "$work/runner/config.sh" ]; then
    mkdir -p "$work/runner"
    tar xzf "$work/runner.tgz" -C "$work/runner"
fi
mkdir -p "$work/actions/checkout" "$work/actions/setup-node"
fetch "https://raw.githubusercontent.com/actions/checkout/$checkout_ref/dist/index.js" \
    "$work/actions/checkout/index.js" "$checkout_sha"
fetch "https://raw.githubusercontent.com/actions/setup-node/$setup_node_ref/dist/setup/index.js" \
    "$work/actions/setup-node/index.js" "$setup_node_sha"

trace=
command -v strace > /dev/null 2>&1 && trace=strace

# run NAME DIR COMMAND...: one payload in one pod.
run() {
    name=$1 dir=$2
    shift 2
    out=$work/out/$name
    rm -rf "$out"
    mkdir -p "$out/tmp" "$out/ws" "$out/cache"
    sock=$socks/$name.sock
    python3 -I "$here/pod-runner-proxy.py" "$sock" "$out/witness.log" "$upstream" &
    proxy=$!
    i=0
    while [ ! -S "$sock" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    if [ ! -S "$sock" ]; then
        echo "pod-runner-measure: the witness proxy of $name did not start" >&2
        exit 1
    fi
    via=http://unix:$sock
    if [ -n "$trace" ]; then
        set -- strace -f -qq -e trace=socket,connect,bind,sendto,sendmsg -o "$out/strace.txt" "$@"
    fi
    set -- "$podnet" pod --backend "$backend" --default-action reject \
        -r "domain:.github.com=$via" -r "domain:.githubusercontent.com=$via" \
        -r "domain:.nodejs.org=$via" -x "$via" --flow-log "$out/flow.jsonl" -- "$@"
    rc=0
    (cd "$dir" && env \
        RUNNER_ALLOW_RUNASROOT=1 \
        ${CA_BUNDLE:+SSL_CERT_FILE=$CA_BUNDLE} \
        ${CA_BUNDLE:+NODE_EXTRA_CA_CERTS=$CA_BUNDLE} \
        ${CA_BUNDLE:+GIT_SSL_CAINFO=$CA_BUNDLE} \
        GITHUB_WORKSPACE="$out/ws" GITHUB_REPOSITORY=PodUtils/podnet \
        GITHUB_SERVER_URL=https://github.com GITHUB_API_URL=https://api.github.com \
        RUNNER_TEMP="$out/tmp" RUNNER_TOOL_CACHE="$out/cache" \
        INPUT_REPOSITORY=PodUtils/podnet INPUT_REF=main \
        INPUT_TOKEN="${GITHUB_TOKEN:-not-a-token}" INPUT_PATH=src \
        INPUT_FETCH-DEPTH=1 INPUT_PERSIST-CREDENTIALS=false INPUT_CLEAN=true \
        INPUT_SET-SAFE-DIRECTORY=true INPUT_NODE-VERSION=20 \
        INPUT_CHECK-LATEST=false INPUT_ARCHITECTURE=x64 \
        timeout 600 "$@") > "$out/output.txt" 2>&1 || rc=$?
    echo "$rc" > "$out/exit"
    kill "$proxy" 2> /dev/null || true
    wait "$proxy" 2> /dev/null || true
    echo "pod-runner-measure: $name exit $rc"
}

runner=$work/runner
run runner-help "$runner" ./config.sh --help
run runner-config "$runner" ./config.sh --unattended --url https://github.com/PodUtils/podnet \
    --token AAAAAAAAAAAAAAAAAAAAAAAAAAAAA --name pod-measure
run runner-run "$runner" ./run.sh
run node-fetch "$work" node -e \
    "fetch('https://api.github.com/repos/PodUtils/podnet').then(r => console.log('status', r.status)).catch(e => { console.log('error', String(e.cause || e)); process.exit(1) })"
run checkout "$work" node "$work/actions/checkout/index.js"
run setup-node "$work" node "$work/actions/setup-node/index.js"

python3 -I "$here/pod-runner-report.py" "$work" "$podnet" "$backend" "$runner_version" \
    "$checkout_ref" "$setup_node_ref" > "$report"
echo "pod-runner-measure: the report is $report"
