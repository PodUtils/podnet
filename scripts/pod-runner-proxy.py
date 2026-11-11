#!/usr/bin/env python3
"""A witness proxy for the measure of T-229: HTTP CONNECT on a Unix socket.

  python3 -I scripts/pod-runner-proxy.py SOCKET LOG [UPSTREAM]

Each CONNECT request writes one line `CONNECT HOST:PORT` to LOG, then the
proxy joins the two ends. UPSTREAM (`http://HOST:PORT`, an HTTP proxy with no
password) carries each tunnel when it is given; else the proxy connects
directly. The proxy is a second witness of the flows of a pod: a flow that
the pod sent through `-x http://unix:SOCKET` is in LOG. It stops at SIGTERM.
"""

import socket
import sys
import threading


def pump(src: socket.socket, dst: socket.socket) -> None:
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass


def head(conn: socket.socket) -> bytes:
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = conn.recv(4096)
        if not chunk:
            return b""
        data += chunk
        if len(data) > 65536:
            return b""
    return data


def open_upstream(target: str, upstream: str | None) -> socket.socket:
    if upstream is None:
        host, _, port = target.rpartition(":")
        return socket.create_connection((host.strip("[]"), int(port)), timeout=30)
    hostport = upstream.split("://", 1)[-1].rstrip("/")
    host, _, port = hostport.rpartition(":")
    up = socket.create_connection((host, int(port)), timeout=30)
    up.sendall(f"CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n".encode())
    reply = head(up)
    status = reply.split(b"\r\n", 1)[0].split(b" ")
    if len(status) < 2 or status[1] != b"200":
        up.close()
        raise OSError(f"the upstream refused {target}: {reply[:80]!r}")
    return up


def serve(conn: socket.socket, log, lock, upstream: str | None) -> None:
    try:
        tunnel(conn, log, lock, upstream)
    except OSError:
        pass


def tunnel(conn: socket.socket, log, lock, upstream: str | None) -> None:
    with conn:
        request = head(conn)
        line = request.split(b"\r\n", 1)[0].decode("latin-1")
        parts = line.split(" ")
        if len(parts) != 3 or parts[0] != "CONNECT":
            conn.sendall(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
            return
        target = parts[1]
        with lock:
            log.write(f"CONNECT {target}\n")
            log.flush()
        try:
            up = open_upstream(target, upstream)
        except OSError:
            conn.sendall(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
            return
        with up:
            up.settimeout(None)
            conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
            back = threading.Thread(target=pump, args=(up, conn), daemon=True)
            back.start()
            pump(conn, up)
            back.join()


def main() -> int:
    if len(sys.argv) not in (3, 4):
        print(__doc__.strip(), file=sys.stderr)
        return 64
    path, log_path = sys.argv[1], sys.argv[2]
    upstream = sys.argv[3] if len(sys.argv) == 4 and sys.argv[3] else None
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(path)
    server.listen(64)
    lock = threading.Lock()
    with open(log_path, "a", encoding="utf-8") as log:
        while True:
            conn, _ = server.accept()
            threading.Thread(target=serve, args=(conn, log, lock, upstream), daemon=True).start()


if __name__ == "__main__":
    sys.exit(main())
