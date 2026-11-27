# shellcheck shell=sh
# docs/serve.md: create share/inbox, add the write user bob, start the
# upload server in the first shell, then curl puts report.pdf (the password
# of bob on stdin: the block has no terminal).
# shellcheck disable=SC2034
BACKGROUND="3"
STDIN="$DOC_DIR/typed"

setup() {
    doc_need_port 8080
    printf '%%PDF-1.4 a report\n' > report.pdf
    PASSWORD='the password of bob'
    export PASSWORD
    printf '%s\n' "$PASSWORD" > "$STDIN"
}

ready() {
    doc_wait_tcp 8080
}

check() {
    cmp report.pdf share/inbox/report.pdf
}
