# shellcheck shell=sh
# docs/serve.md: the unlock of a PIN. The set-up starts `serve --pin
# --upload` on a free port, and gives the block PIN, ORIGIN and URL.

setup() {
    doc_need_loopback
    mkdir -p site
    echo 'a file behind the PIN' > site/a.txt
    echo 'an upload' > file.txt
    port=$(doc_free_port)
    doc_bg podnet serve --dir ./site --list --upload --pin --local "127.0.0.1:$port" --json
    doc_wait_line "$DOC_DIR/bg-1.log" '"pin":"[0-9]{6}"' 20
    PIN=$(sed -n 's/.*"pin":"\([0-9]*\)".*/\1/p' "$DOC_DIR/bg-1.log")
    ORIGIN="http://127.0.0.1:$port"
    URL="$ORIGIN"
    export PIN ORIGIN URL
}

check() {
    cmp file.txt site/file.txt
}
