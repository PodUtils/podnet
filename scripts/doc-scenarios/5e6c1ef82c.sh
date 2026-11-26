# shellcheck shell=sh
# docs/serve.md: `serve --tar --local` in the first shell, then curl and tar
# take a directory as one archive.
# shellcheck disable=SC2034
BACKGROUND="1"

setup() {
    doc_need_port 8080
    mkdir -p public/docs/guide
    echo 'the first page' > public/docs/a.txt
    echo 'the second page' > public/docs/guide/b.txt
}

ready() {
    doc_wait_tcp 8080
}

check() {
    grep -q 'the first page' docs/a.txt
    grep -q 'the second page' docs/guide/b.txt
}
