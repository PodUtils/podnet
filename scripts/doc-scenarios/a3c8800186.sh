# shellcheck shell=sh
# docs/target-environment.md: a nested WARP session. The second proxy
# (the lines 2 and 3) runs in the foreground until Ctrl-C: it runs in the
# background here. The check sends a request through the second proxy.
# The teardown removes the two WARP devices.

# shellcheck disable=SC2034
BACKGROUND="2"
LIMIT=90

setup() {
    doc_need_port 40000
    doc_need_port 40001
}

ready() {
    doc_wait_tcp 40001 60
}

check() {
    curl -fsS --max-time 30 -x socks5h://127.0.0.1:40001 https://one.one.one.one/cdn-cgi/trace | grep '^warp=on'
}

teardown() {
    doc_warp_delete a.json b.json
}
