# shellcheck shell=sh
# docs/warp.md: a WARP proxy with rules. The set-up writes the rules file
# of the user. The proxy runs in the foreground until Ctrl-C: it runs in
# the background here. The check sends a request through it, and the
# teardown removes the WARP device.

# shellcheck disable=SC2034
BACKGROUND="1"
LIMIT=90

setup() {
    doc_need_port 40000
    mkdir -p "$HOME/.config/podnet"
    printf '# the rules of the user\ndomain:.tracker.example=reject\n' > "$HOME/.config/podnet/rules"
}

ready() {
    doc_wait_tcp 40000 60
}

check() {
    curl -fsS --max-time 30 -x socks5h://127.0.0.1:40000 https://one.one.one.one/cdn-cgi/trace | grep '^warp=on'
}

teardown() {
    doc_warp_delete
}
