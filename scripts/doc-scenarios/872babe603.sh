# shellcheck shell=sh
# docs/serve.md: an upload server with the write user bob in the first
# shell, then curl puts report.pdf (the password of bob on stdin: the block
# has no terminal).
# shellcheck disable=SC2034
BACKGROUND="1"
STDIN="$DOC_DIR/typed"

setup() {
    doc_need_port 8080
    mkdir -p share/inbox
    printf '%%PDF-1.4 a report\n' > report.pdf
    printf '%s\n' 'the password of bob' > "$STDIN"
    podnet serve --add-user bob:write --users users < "$STDIN"
}

ready() {
    doc_wait_tcp 8080
}

check() {
    cmp report.pdf share/inbox/report.pdf
}
