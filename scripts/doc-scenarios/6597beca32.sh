# shellcheck shell=sh
# docs/host-runtime.md: the status page of a tree. The set-up makes a tree
# with one git repository (./app, a Cargo package with one commit).

setup() {
    export GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=doc GIT_AUTHOR_EMAIL=doc@example.com
    export GIT_COMMITTER_NAME=doc GIT_COMMITTER_EMAIL=doc@example.com
    git init -q app
    printf '[package]\nname = "app"\nversion = "0.1.0"\n' > app/Cargo.toml
    git -C app add Cargo.toml
    git -C app commit -q -m 'the first commit'
}

check() {
    grep -q app status.md
}
