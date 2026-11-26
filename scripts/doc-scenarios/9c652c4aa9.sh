# shellcheck shell=sh
# docs/virtual-network.md: a program under the shim. The set-up writes
# ./server: it binds 10.66.0.2:8080, an address that the kernel does not
# have, so the bind works only through the shim.
# The block is n.a. on a host that refuses a unix listener (the shim).

setup() {
    doc_need_unix_listen
    cat > server <<'EOF'
#!/bin/sh
exec python3 -I -c 'import socket
s = socket.socket()
s.bind(("10.66.0.2", 8080))
s.listen()
print("bound", s.getsockname())'
EOF
    chmod +x server
}
