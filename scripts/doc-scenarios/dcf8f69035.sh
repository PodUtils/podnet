# shellcheck shell=sh
# docs/quic.md: the TUIC server of the block before. Each block has its own
# TMPDIR: the set-up runs the block before (the download), and the server
# runs only when its SHA-256 is the pin of the document.

# shellcheck disable=SC2034
LIMIT=60

setup() {
    curl -fsSL -o "${TMPDIR:-/tmp}/tuic-server-v2" \
        "https://github.com/Itsusinn/tuic/releases/download/v1.5.2/tuic-server-x86_64-linux-musl"
    echo "b608276df5ae2c662b8d264184563b252d50684389833607b2bd02249ffa3b04  ${TMPDIR:-/tmp}/tuic-server-v2" |
        sha256sum -c -
    chmod +x "${TMPDIR:-/tmp}/tuic-server-v2"
}
