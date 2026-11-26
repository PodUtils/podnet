# shellcheck shell=sh
# docs/doctor.md: podnet agent up, status and down. The set-up serves HTTP
# on 127.0.0.1:3000, the port that the block exposes. The teardown stops
# what is left and removes a WARP device that agent up registered.

# shellcheck disable=SC2034
LIMIT=90

setup() {
    doc_need_port 3000
    doc_bg python3 -I "$DOC_ROOT/tests/consumers/servers.py" http-tcp 3000 'the app of port 3000'
    doc_wait_tcp 3000
}

teardown() {
    podnet agent down > /dev/null 2>&1 || true
    doc_warp_delete
}
