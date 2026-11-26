# shellcheck shell=sh
# docs/virtual-network.md: the proxy of the stack in the background (a
# foreground command), then a SOCKS5 greeting on its unix socket.
# shellcheck disable=SC2034
BACKGROUND="1"

setup() {
    doc_free_socket /tmp/podnet-vnet.socks
}

ready() {
    doc_wait_path /tmp/podnet-vnet.socks 20
}

check() {
    python3 -I -c 'import socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(5)
s.connect("/tmp/podnet-vnet.socks")
s.sendall(b"\x05\x01\x00")
answer = s.recv(2)
sys.exit(0 if answer == b"\x05\x00" else "the greeting got %r" % answer)'
}
