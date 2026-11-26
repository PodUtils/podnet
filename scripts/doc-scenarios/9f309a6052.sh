# shellcheck shell=sh
# docs/doctor.md: podnet bench, direct and through a SOCKS5 proxy at
# 127.0.0.1:1080 (a stand-in to the network of the host).

# shellcheck disable=SC2034
LIMIT=60

setup() {
    doc_socks_out 1080
}

check() {
    grep -q 'connect example.com:443' "$DOC_DIR/socks-1080.log"
}
