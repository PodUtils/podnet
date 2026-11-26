# shellcheck shell=sh
# docs/quic.md: the download of the TUIC server. The check compares its
# SHA-256 with the pin of the document.

# shellcheck disable=SC2034
LIMIT=60

setup() {
    :
}

check() {
    echo "d2c76a3bd4fbd32b209d4f29bbf76dcb80b4c7d25ae8ef1a8b5130e0364c498a  ${TMPDIR:-/tmp}/tuic-server-v2" |
        sha256sum -c -
    test -x "${TMPDIR:-/tmp}/tuic-server-v2"
}
