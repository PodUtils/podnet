# shellcheck shell=sh
# docs/target-environment.md: WARP through a SOCKS5 proxy at
# 127.0.0.1:9050 (a stand-in for Tor, to the network of the host). The
# proxy of the line 2 runs in the foreground until Ctrl-C: it runs in the
# background here. The check sends a request through it, and the teardown
# removes the WARP device.

# shellcheck disable=SC2034
BACKGROUND="2"
LIMIT=90

setup() {
    doc_need_port 40000
    doc_socks_out 9050
}

ready() {
    doc_wait_tcp 40000 60
}

check() {
    curl -fsS --max-time 30 -x socks5h://127.0.0.1:40000 https://one.one.one.one/cdn-cgi/trace | grep '^warp=on'
    grep -q '^connect ' "$DOC_DIR/socks-9050.log"
}

teardown() {
    doc_warp_delete
}
