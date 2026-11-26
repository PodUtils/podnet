# shellcheck shell=sh
# docs/serve.md: hash a password, add the user alice, serve with the users
# in the first shell, then curl asks for the password of alice (stdin: the
# block has no terminal).
# shellcheck disable=SC2034
BACKGROUND="4"
STDIN="$DOC_DIR/typed"

setup() {
    doc_need_port 8080
    mkdir -p public
    echo 'a page for alice' > public/a.txt
    PASSWORD='a long password'
    export PASSWORD
    printf '%s\n' "$PASSWORD" > "$STDIN"
}

ready() {
    doc_wait_tcp 8080
}
