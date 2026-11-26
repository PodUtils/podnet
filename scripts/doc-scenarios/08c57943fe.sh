# shellcheck shell=sh
# docs/host-runtime.md: the peer of `podnet stat` in its own shell (the
# background), then the sender. The run takes 4 s at 1000 bit/s.
# shellcheck disable=SC2034
BACKGROUND="1"

setup() {
    doc_need_port 9000 udp
}

ready() {
    doc_wait_udp 9000
}

check() {
    grep -q '^received 10$' pod-stat.txt
}
