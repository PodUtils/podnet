# shellcheck shell=sh
# docs/virtual-network.md: a detached switch, then a stack with faults on
# the switch in the background (a foreground command). The check: the stack
# still runs after 2 s.
# The block is n.a. on a host that refuses a unix listener.
# shellcheck disable=SC2034
BACKGROUND="2"

setup() {
    doc_need_unix_listen
    doc_free_socket /tmp/sw.sock
    doc_free_socket /tmp/podnet-vnet.socks
}

ready() {
    doc_wait_path /tmp/podnet-vnet.socks 20
}

check() {
    sleep 2
    kill -0 "$(tail -n 1 "$DOC_DIR/pids")"
}
