#!/usr/bin/env python3
"""Zero-dependency HTTP/HTTPS/CONNECT MITM and auto-fallback passthrough proxy.

Part of the podnet hermetic client capture rig (scripts/capture-rig/).

Modes:
  - dual (default): MITM CONNECT targets using an ephemeral per-run CA, recording
    decrypted HTTP/1.1 requests/responses to http_transcript.jsonl. If a client
    rejects our ephemeral CA (TLS alert / certificate pinning), the host is
    recorded in pinned_hosts and automatically switched to passthrough on retry
    so the client succeeds without manual intervention.
  - mitm: always MITM CONNECT targets.
  - passthrough: never terminate TLS; record CONNECT authority, ClientHello SNI,
    and ALPN while forwarding raw bytes transparently.
"""

from __future__ import annotations

import argparse
import json
import os
import select
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Optional


def utc_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def parse_client_hello(data: bytes) -> dict:
    """Extract TLS ClientHello SNI and ALPN without external libraries."""
    info: dict = {}
    if len(data) < 5 or data[0] != 0x16:
        return info
    rec_len = int.from_bytes(data[3:5], "big")
    if len(data) < 5 + rec_len or rec_len < 42:
        return info
    body = data[5 : 5 + rec_len]
    if body[0] != 0x01:  # ClientHello
        return info
    pos = 4  # handshake type + 3-byte len
    if pos + 34 > len(body):
        return info
    pos += 2 + 32  # version + random
    if pos >= len(body):
        return info
    sid_len = body[pos]
    pos += 1 + sid_len
    if pos + 2 > len(body):
        return info
    cs_len = int.from_bytes(body[pos : pos + 2], "big")
    pos += 2 + cs_len
    if pos >= len(body):
        return info
    comp_len = body[pos]
    pos += 1 + comp_len
    if pos + 2 > len(body):
        return info
    ext_total = int.from_bytes(body[pos : pos + 2], "big")
    pos += 2
    end = min(len(body), pos + ext_total)
    while pos + 4 <= end:
        ext_type = int.from_bytes(body[pos : pos + 2], "big")
        ext_len = int.from_bytes(body[pos + 2 : pos + 4], "big")
        pos += 4
        ext = body[pos : pos + ext_len]
        pos += ext_len
        if ext_type == 0x0000 and len(ext) >= 5:  # SNI
            name_len = int.from_bytes(ext[3:5], "big")
            if 5 + name_len <= len(ext):
                info["sni"] = ext[5 : 5 + name_len].decode("ascii", "replace")
        elif ext_type == 0x0010 and len(ext) >= 2:  # ALPN
            alpn_list = []
            ap = 2
            while ap < len(ext):
                n = ext[ap]
                ap += 1
                if ap + n <= len(ext):
                    alpn_list.append(ext[ap : ap + n].decode("ascii", "replace"))
                ap += n
            info["alpn"] = alpn_list
        elif ext_type == 0xFE0D:  # ECH
            info["ech_present"] = True
    return info


class EphemeralCA:
    """Generates a fresh per-run root CA and signs leaf certs via openssl CLI."""

    def __init__(self, work_dir: Path) -> None:
        self.dir = work_dir / "ca"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.ca_key = self.dir / "ca.key"
        self.ca_crt = self.dir / "ca.crt"
        self._lock = threading.Lock()
        self._cache: dict[str, tuple[Path, Path]] = {}
        subprocess.run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:prime256v1",
                "-nodes",
                "-keyout",
                str(self.ca_key),
                "-out",
                str(self.ca_crt),
                "-days",
                "2",
                "-subj",
                "/CN=podnet-capture-rig-ephemeral-ca",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        os.chmod(self.ca_key, 0o600)

    def cert_for(self, host: str) -> tuple[Path, Path]:
        safe = "".join(c if c.isalnum() or c in ".-_" else "_" for c in host)[:120]
        with self._lock:
            if safe in self._cache:
                return self._cache[safe]
            key = self.dir / f"{safe}.key"
            csr = self.dir / f"{safe}.csr"
            crt = self.dir / f"{safe}.crt"
            ext = self.dir / f"{safe}.ext"
            is_ip = False
            try:
                socket.inet_pton(socket.AF_INET, host)
                is_ip = True
            except OSError:
                try:
                    socket.inet_pton(socket.AF_INET6, host.strip("[]"))
                    is_ip = True
                except OSError:
                    pass
            san = f"IP:{host.strip('[]')}" if is_ip else f"DNS:{host}"
            ext.write_text(f"subjectAltName={san}\nextendedKeyUsage=serverAuth\n")
            subprocess.run(
                [
                    "openssl",
                    "req",
                    "-newkey",
                    "ec",
                    "-pkeyopt",
                    "ec_paramgen_curve:prime256v1",
                    "-nodes",
                    "-keyout",
                    str(key),
                    "-out",
                    str(csr),
                    "-subj",
                    f"/CN={host[:60]}",
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            subprocess.run(
                [
                    "openssl",
                    "x509",
                    "-req",
                    "-in",
                    str(csr),
                    "-CA",
                    str(self.ca_crt),
                    "-CAkey",
                    str(self.ca_key),
                    "-CAcreateserial",
                    "-out",
                    str(crt),
                    "-days",
                    "2",
                    "-extfile",
                    str(ext),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            os.chmod(key, 0o600)
            self._cache[safe] = (crt, key)
            return crt, key


class CaptureProxy:
    def __init__(
        self,
        listen: tuple[str, int],
        out_dir: Path,
        mode: str,
        ca: Optional[EphemeralCA],
        upstream_ca: Optional[str],
    ) -> None:
        self.listen = listen
        self.out_dir = out_dir
        self.mode = mode
        self.ca = ca
        self.upstream_ca = upstream_ca
        self._log_lock = threading.Lock()
        self._pinned_lock = threading.Lock()
        self.pinned_hosts: set[str] = set()
        self.connect_log = (out_dir / "connect_log.jsonl").open("a", encoding="utf-8")
        self.http_log = (out_dir / "http_transcript.jsonl").open("a", encoding="utf-8")

    def _write_jsonl(self, fp, obj: dict) -> None:
        with self._log_lock:
            fp.write(json.dumps(obj, sort_keys=True) + "\n")
            fp.flush()

    def _pipe(self, a: socket.socket, b: socket.socket) -> None:
        try:
            while True:
                r, _, _ = select.select([a, b], [], [], 60.0)
                if not r:
                    break
                for src in r:
                    dst = b if src is a else a
                    data = src.recv(65536)
                    if not data:
                        return
                    dst.sendall(data)
        except OSError:
            pass
        finally:
            for s in (a, b):
                try:
                    s.close()
                except OSError:
                    pass

    def _read_http_message(self, sock: socket.socket, first_chunk: bytes = b""):
        buf = first_chunk
        while b"\r\n\r\n" not in buf and len(buf) < 65536:
            chunk = sock.recv(4096)
            if not chunk:
                break
            buf += chunk
        if b"\r\n\r\n" not in buf:
            return None, {}, b"", b""
        head_raw, rest = buf.split(b"\r\n\r\n", 1)
        lines = head_raw.decode("latin1").split("\r\n")
        start_line = lines[0]
        headers: dict[str, str] = {}
        for line in lines[1:]:
            if ":" in line:
                k, v = line.split(":", 1)
                headers[k.strip()] = v.strip()
        cl = headers.get("Content-Length") or headers.get("content-length")
        body = b""
        if cl and cl.isdigit():
            need = int(cl)
            while len(rest) < need:
                chunk = sock.recv(min(65536, need - len(rest)))
                if not chunk:
                    break
                rest += chunk
            body, rest = rest[:need], rest[need:]
        return start_line, headers, body, rest

    def _handle_connect(self, client: socket.socket, target: str) -> None:
        host, port_s = target.rsplit(":", 1)
        host_clean = host.strip("[]")
        port = int(port_s)
        with self._pinned_lock:
            is_pinned = host_clean in self.pinned_hosts
        use_mitm = self.mode == "mitm" or (self.mode == "dual" and not is_pinned and self.ca is not None)

        try:
            upstream = socket.create_connection((host_clean, port), timeout=15)
        except OSError as exc:
            self._write_jsonl(
                self.connect_log,
                {"ts": utc_now(), "target": target, "mode": "error", "error": str(exc)},
            )
            client.sendall(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
            client.close()
            return

        client.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")

        if not use_mitm:
            first = b""
            try:
                client.settimeout(5.0)
                first = client.recv(16384)
                client.settimeout(None)
            except OSError:
                pass
            hello = parse_client_hello(first)
            self._write_jsonl(
                self.connect_log,
                {
                    "ts": utc_now(),
                    "target": target,
                    "mode": "passthrough",
                    "pinned_fallback": is_pinned,
                    "tls_hello": hello,
                },
            )
            if first:
                upstream.sendall(first)
            self._pipe(client, upstream)
            return

        assert self.ca is not None
        crt, key = self.ca.cert_for(host_clean)
        srv_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        srv_ctx.load_cert_chain(str(crt), str(key))
        srv_ctx.set_alpn_protocols(["http/1.1"])
        try:
            tls_client = srv_ctx.wrap_socket(client, server_side=True)
        except ssl.SSLError as exc:
            with self._pinned_lock:
                self.pinned_hosts.add(host_clean)
            self._write_jsonl(
                self.connect_log,
                {
                    "ts": utc_now(),
                    "target": target,
                    "mode": "mitm_rejected_pinned",
                    "error": str(exc),
                },
            )
            upstream.close()
            client.close()
            return

        cli_ctx = ssl.create_default_context()
        if self.upstream_ca:
            cli_ctx.load_verify_locations(self.upstream_ca)
        cli_ctx.set_alpn_protocols(["http/1.1"])
        try:
            tls_up = cli_ctx.wrap_socket(upstream, server_hostname=host_clean)
        except ssl.SSLError as exc:
            self._write_jsonl(
                self.connect_log,
                {"ts": utc_now(), "target": target, "mode": "upstream_tls_error", "error": str(exc)},
            )
            tls_client.close()
            upstream.close()
            return

        self._write_jsonl(
            self.connect_log,
            {
                "ts": utc_now(),
                "target": target,
                "mode": "mitm",
                "alpn": tls_up.selected_alpn_protocol(),
                "version": tls_up.version(),
            },
        )
        try:
            # Capture first HTTP exchange cleanly, then stream any remainder.
            req_line, req_hdrs, req_body, req_rest = self._read_http_message(tls_client)
            if req_line is None:
                return
            raw_req = (
                req_line.encode("latin1")
                + b"\r\n"
                + b"".join(f"{k}: {v}\r\n".encode("latin1") for k, v in req_hdrs.items())
                + b"\r\n"
                + req_body
                + req_rest
            )
            tls_up.sendall(raw_req)
            resp_line, resp_hdrs, resp_body, resp_rest = self._read_http_message(tls_up)
            if resp_line is not None:
                raw_resp = (
                    resp_line.encode("latin1")
                    + b"\r\n"
                    + b"".join(f"{k}: {v}\r\n".encode("latin1") for k, v in resp_hdrs.items())
                    + b"\r\n"
                    + resp_body
                    + resp_rest
                )
                tls_client.sendall(raw_resp)
                self._write_jsonl(
                    self.http_log,
                    {
                        "ts": utc_now(),
                        "scheme": "https",
                        "target": target,
                        "request_line": req_line,
                        "request_headers": req_hdrs,
                        "request_body_preview": req_body[:4096].decode("utf-8", "replace"),
                        "response_line": resp_line,
                        "response_headers": resp_hdrs,
                        "response_body_preview": resp_body[:4096].decode("utf-8", "replace"),
                    },
                )
            self._pipe(tls_client, tls_up)
        except OSError:
            pass
        finally:
            for s in (tls_client, tls_up):
                try:
                    s.close()
                except OSError:
                    pass

    def _handle_client(self, client: socket.socket) -> None:
        try:
            req_line, req_hdrs, req_body, req_rest = self._read_http_message(client)
            if req_line is None:
                client.close()
                return
            parts = req_line.split(" ")
            if len(parts) < 3:
                client.close()
                return
            method, target, version = parts[0], parts[1], parts[2]
            if method == "CONNECT":
                self._handle_connect(client, target)
                return
            # Plain HTTP proxy request
            from urllib.parse import urlsplit

            sp = urlsplit(target)
            host = sp.hostname or ""
            port = sp.port or 80
            path = sp.path or "/"
            if sp.query:
                path += "?" + sp.query
            upstream = socket.create_connection((host, port), timeout=15)
            fwd_line = f"{method} {path} {version}"
            raw_req = (
                fwd_line.encode("latin1")
                + b"\r\n"
                + b"".join(f"{k}: {v}\r\n".encode("latin1") for k, v in req_hdrs.items())
                + b"\r\n"
                + req_body
                + req_rest
            )
            upstream.sendall(raw_req)
            resp_line, resp_hdrs, resp_body, resp_rest = self._read_http_message(upstream)
            if resp_line is not None:
                raw_resp = (
                    resp_line.encode("latin1")
                    + b"\r\n"
                    + b"".join(f"{k}: {v}\r\n".encode("latin1") for k, v in resp_hdrs.items())
                    + b"\r\n"
                    + resp_body
                    + resp_rest
                )
                client.sendall(raw_resp)
                self._write_jsonl(
                    self.http_log,
                    {
                        "ts": utc_now(),
                        "scheme": "http",
                        "target": f"{host}:{port}",
                        "request_line": fwd_line,
                        "request_headers": req_hdrs,
                        "request_body_preview": req_body[:4096].decode("utf-8", "replace"),
                        "response_line": resp_line,
                        "response_headers": resp_hdrs,
                        "response_body_preview": resp_body[:4096].decode("utf-8", "replace"),
                    },
                )
            self._pipe(client, upstream)
        except OSError:
            try:
                client.close()
            except OSError:
                pass

    def serve(self) -> None:
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(self.listen)
        srv.listen(128)
        actual_port = srv.getsockname()[1]
        (self.out_dir / "proxy.ready").write_text(
            json.dumps(
                {
                    "host": self.listen[0],
                    "port": actual_port,
                    "mode": self.mode,
                    "ca_cert": str(self.ca.ca_crt) if self.ca else None,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        while True:
            conn, _ = srv.accept()
            threading.Thread(target=self._handle_client, args=(conn,), daemon=True).start()


def main() -> int:
    ap = argparse.ArgumentParser(description="podnet capture rig MITM/passthrough proxy")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=18080)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--mode", choices=["dual", "mitm", "passthrough"], default="dual")
    ap.add_argument("--upstream-ca", default=None)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    ca = EphemeralCA(args.out) if args.mode in ("dual", "mitm") else None
    proxy = CaptureProxy((args.host, args.port), args.out, args.mode, ca, args.upstream_ca)
    proxy.serve()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
