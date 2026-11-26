# shellcheck shell=sh
# docs/serve.md: `serve --list --local` in the first shell, then the JSON
# listing of a directory.
# shellcheck disable=SC2034
BACKGROUND="1"

setup() {
    doc_need_port 8080
    mkdir -p public/docs
    echo 'a page' > public/docs/a.txt
}

ready() {
    doc_wait_tcp 8080
}
