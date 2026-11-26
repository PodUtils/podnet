# shellcheck shell=sh
# docs/quic.md: the TUIC server of the block before. Each block has its own
# TMPDIR: the set-up runs the block before (the download), and the server
# runs only when its SHA-256 is the pin of the document.

# shellcheck disable=SC2034
LIMIT=60

setup() {
    curl -sSL -o "${TMPDIR:-/tmp}/tuic-server-v2" \
        "https://github.com/Itsusinn/tuic/releases/download/v2.0.0-dev10/tuic-server-x86_64-linux-musl"
    echo "d2c76a3bd4fbd32b209d4f29bbf76dcb80b4c7d25ae8ef1a8b5130e0364c498a  ${TMPDIR:-/tmp}/tuic-server-v2" |
        sha256sum -c -
    chmod +x "${TMPDIR:-/tmp}/tuic-server-v2"
}
