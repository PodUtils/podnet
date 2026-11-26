# shellcheck shell=sh
# docs/cli.md: `--check` of serve and tunnel. The set-up writes ./site and a
# config.yml whose origin is a file server on a free port of the loopback.

setup() {
    doc_need_loopback
    mkdir -p site origin
    echo '<p>a site</p>' > site/index.html
    echo 'the origin' > origin/index.html
    port=$(doc_free_port)
    printf 'ingress:\n  - service: http://127.0.0.1:%s\n' "$port" > config.yml
    doc_bg podnet serve --dir ./origin --local "127.0.0.1:$port"
    doc_wait_tcp "$port" 20
}
