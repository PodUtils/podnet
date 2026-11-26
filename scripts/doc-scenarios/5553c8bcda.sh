# shellcheck shell=sh
# docs/background.md: a detached quick tunnel of port 3000 and a detached
# WARP proxy. The set-up serves HTTP on 127.0.0.1:3000: a detached tunnel
# is ready when its public URL answers. `podnet logs web -f` prints until
# Ctrl-C: a user types the next line in a second shell, so the line 4
# runs in the background. The teardown removes the WARP device that the
# proxy registers.

# shellcheck disable=SC2034
BACKGROUND="4"
LIMIT=90

setup() {
    doc_need_port 3000
    doc_free_socket /tmp/warp.sock
    doc_bg python3 -I "$DOC_ROOT/tests/consumers/servers.py" http-tcp 3000 'the app of port 3000'
    doc_wait_tcp 3000
}

check() {
    grep -q 'podnet: public' "$DOC_DIR/bg-2.log"
}

teardown() {
    doc_warp_delete
}
