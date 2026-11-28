# shellcheck shell=sh
# docs/quic.md: the download of the TUIC server. The check compares its
# SHA-256 with the pin of the document.

# shellcheck disable=SC2034
LIMIT=60

setup() {
    :
}

check() {
    echo "b608276df5ae2c662b8d264184563b252d50684389833607b2bd02249ffa3b04  ${TMPDIR:-/tmp}/tuic-server-v2" |
        sha256sum -c -
    test -x "${TMPDIR:-/tmp}/tuic-server-v2"
}
