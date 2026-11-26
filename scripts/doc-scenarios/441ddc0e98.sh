# shellcheck shell=sh
# docs/pods.md: the rules of a pod. The set-up starts a SOCKS5 stand-in at
# 127.0.0.1:1080 that connects web.test:80 to an HTTP server of the
# loopback, and writes ./program: it gets http://web.test/ with curl.

setup() {
    doc_need_port 1080
    podnet pod -x direct -- true > "$DOC_DIR/probe" 2>&1 || {
        [ $? -ne 3 ] || doc_na "no pod backend on this host: $(tail -n 1 "$DOC_DIR/probe")"
        cat "$DOC_DIR/probe"
        return 1
    }
    port=$(doc_free_port)
    doc_bg python3 -I "$DOC_ROOT/tests/consumers/servers.py" http-tcp "$port" 'the page of web.test'
    doc_bg python3 -I "$DOC_ROOT/tests/consumers/servers.py" socks-tcp 1080 "$DOC_DIR/socks.log" \
        "web.test:80=$port"
    doc_wait_tcp "$port"
    doc_wait_tcp 1080
    printf '#!/bin/sh\nexec curl -fsS -o page.html http://web.test/\n' > program
    chmod +x program
}

check() {
    grep -q 'the page of web.test' page.html
    grep -q 'connect web.test:80' "$DOC_DIR/socks.log"
}
