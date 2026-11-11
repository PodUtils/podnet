#!/usr/bin/env python3
"""Probe the hosted mailboxes and relays through an HTTP CONNECT proxy.

  python3 -I scripts/ci/probe-hosted.py [PROXY_URL]

PROXY_URL is http://HOST:PORT. It defaults to $HTTPS_PROXY. With no proxy,
the script connects direct. The script uses the Python 3 standard library
only.

The hosts below are test targets of a study
(docs/research/podnet/hosted-relays.md). They belong to other parties.
podnet names none of them in its Rust code.

Each service prints one line: NAME ok|fail MS ms REASON. A failed service
prints a GitHub warning line too (::warning::). The exit code is 0 when each
service ran, also when a service failed: the services are not podnet's.
"""

import base64
import http.client
import json
import os
import secrets
import socket
import ssl
import sys
import threading
import time
import urllib.parse

TIMEOUT = 20.0
MAILBOXES = [
    ("wormhole-mailbox", "relay.magic-wormhole.io", "/v1"),
    ("winden-mailbox", "mailbox.winden.app", "/v1"),
]
PIPING = [("piping-ppng", "ppng.io"), ("piping-onrender", "piping.onrender.com")]
IROH = [("iroh-relay", "use1-1.relay.n0.iroh.link")]
CTX = ssl.create_default_context()


def proxy_addr(url: str):
    if not url:
        return None
    u = urllib.parse.urlparse(url)
    return u.hostname, u.port or 80


def open_tls(host: str, proxy) -> ssl.SSLSocket:
    """A TLS socket to HOST:443, through the proxy when one is given."""
    if proxy is None:
        raw = socket.create_connection((host, 443), timeout=TIMEOUT)
    else:
        raw = socket.create_connection(proxy, timeout=TIMEOUT)
        raw.sendall(f"CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n".encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = raw.recv(4096)
            if not chunk:
                raise RuntimeError("proxy closed the connection")
            head += chunk
        status = head.split(b"\r\n", 1)[0].decode("latin-1")
        if " 200" not in status:
            raise RuntimeError(f"proxy: {status}")
    return CTX.wrap_socket(raw, server_hostname=host)


def read_more(sock: ssl.SSLSocket, buf: bytes, n: int) -> bytes:
    """Read until BUF holds N bytes."""
    while len(buf) < n:
        chunk = sock.recv(4096)
        if not chunk:
            raise RuntimeError("connection closed")
        buf += chunk
    return buf


def mailbox(host: str, path: str, proxy) -> str:
    """A WebSocket upgrade, then the first message: the server's welcome."""
    sock = open_tls(host, proxy)
    try:
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        sock.sendall(
            (
                f"GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\n"
                f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                "Sec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(4096)
            if not chunk:
                raise RuntimeError("closed before the upgrade answer")
            buf += chunk
        head, rest = buf.split(b"\r\n\r\n", 1)
        status = head.split(b"\r\n", 1)[0].decode("latin-1")
        if " 101" not in status:
            raise RuntimeError(f"upgrade: {status}")
        # One frame. A server frame has no mask.
        rest = read_more(sock, rest, 2)
        opcode, length = rest[0] & 0x0F, rest[1] & 0x7F
        rest = rest[2:]
        need = {126: 2, 127: 8}.get(length, 0)
        if need:
            rest = read_more(sock, rest, need)
            length = int.from_bytes(rest[:need], "big")
            rest = rest[need:]
        rest = read_more(sock, rest, length)
        if opcode != 1:
            raise RuntimeError(f"first frame has opcode {opcode}, not text")
        msg = json.loads(rest[:length].decode())
        if msg.get("type") != "welcome":
            raise RuntimeError(f"first message is {msg.get('type')!r}, not welcome")
        return "welcome received"
    finally:
        sock.close()


class TunnelConn(http.client.HTTPSConnection):
    """HTTPS to HOST:443 through a plain-HTTP CONNECT proxy."""

    def __init__(self, host, proxy):
        super().__init__(host, 443, timeout=TIMEOUT, context=CTX)
        self._proxy = proxy
        self._target = host

    def connect(self):
        self.sock = open_tls(self._target, self._proxy)


def piping(host: str, proxy) -> str:
    """A PUT and a GET of random bytes on one random path."""
    path = "/podnet-probe-" + secrets.token_hex(8)
    data = secrets.token_bytes(32)
    box = {}

    def put():
        try:
            c = TunnelConn(host, proxy)
            c.request("PUT", path, body=data, headers={"Content-Type": "application/octet-stream"})
            box["put"] = c.getresponse().status
            c.close()
        except Exception as e:  # noqa: BLE001
            box["put_err"] = f"{type(e).__name__}: {e}"

    t = threading.Thread(target=put, daemon=True)
    t.start()
    time.sleep(0.5)
    c = TunnelConn(host, proxy)
    c.request("GET", path)
    r = c.getresponse()
    if r.status != 200:
        raise RuntimeError(f"GET status {r.status}")
    got = r.read()
    c.close()
    t.join(TIMEOUT)
    if "put_err" in box:
        raise RuntimeError(f"PUT {box['put_err']}")
    if got != data:
        raise RuntimeError(f"bytes differ ({len(got)} read, {len(data)} sent)")
    return f"{len(data)} bytes equal"


def iroh(host: str, proxy) -> str:
    """HTTPS 200 on the root of a relay."""
    c = TunnelConn(host, proxy)
    c.request("GET", "/")
    r = c.getresponse()
    r.read(4096)
    c.close()
    if r.status != 200:
        raise RuntimeError(f"HTTP {r.status}")
    return "HTTP 200"


def run(name: str, fn, *args) -> bool:
    start = time.monotonic()
    try:
        reason = fn(*args)
        ok = True
    except Exception as e:  # noqa: BLE001
        reason = f"{type(e).__name__}: {e}".replace("\n", " ")
        ok = False
    ms = int((time.monotonic() - start) * 1000)
    print(f"{name} {'ok' if ok else 'fail'} {ms} ms {reason}", flush=True)
    if not ok:
        print(f"::warning::hosted service {name} failed: {reason}", flush=True)
    return ok


def main() -> int:
    arg = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("HTTPS_PROXY", "")
    proxy = proxy_addr(arg)
    for name, host, path in MAILBOXES:
        run(name, mailbox, host, path, proxy)
    for name, host in PIPING:
        run(name, piping, host, proxy)
    for name, host in IROH:
        run(name, iroh, host, proxy)
    return 0


if __name__ == "__main__":
    sys.exit(main())
