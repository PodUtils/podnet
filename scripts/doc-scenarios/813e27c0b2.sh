# shellcheck shell=sh
# docs/link.md: the icmp carrier on one host, as root. The block is n.a.
# when the binary has no build feature icmp (the release set has it).

setup() {
    if podnet link carriers | grep '^icmp:' | grep -q 'not built'; then
        doc_na "the binary has no build feature icmp"
    fi
}
