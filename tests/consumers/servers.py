# T-358 step 3: the servers of the consumer scenarios.
#
# Usage:
#   python3 -I servers.py http-unix PATH BODY LOG
#   python3 -I servers.py http-tcp PORT BODY
#   python3 -I servers.py socks-unix PATH LOG NAME:PORT=TCPPORT
#   python3 -I servers.py socks-tcp PORT LOG NAME:PORT=TCPPORT
#   python3 -I servers.py socks-tcp PORT LOG '*'
#
# http-unix serves HTTP on the unix socket PATH. Each request gets BODY.
# For each connection it appends the peer (SO_PEERCRED: pid, uid and the
# program) to LOG, and for each request the request line. A request line
# with an absolute URI is a request to a proxy: this server then is the
# proxy, and it answers the request itself.
# http-tcp serves HTTP on 127.0.0.1:PORT (the decoy of a scenario).
# socks-unix is a SOCKS5 server on the unix socket PATH. It appends each
# CONNECT target to LOG. It connects only NAME:PORT, to 127.0.0.1:TCPPORT;
# it refuses each other target (reply 0x02).
# socks-tcp is the same SOCKS5 server on 127.0.0.1:PORT (T-358 step 2: the
# scenarios of the documented commands). With the target `*`, it connects
# each target, by its own name and port (a proxy to the network of the
# host, for the `live` blocks of the documents).
import http.server
import os
import socket
import socketserver
import struct
import sys
import threading


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def do_GET(self):
        log(self.server, "request %s" % self.requestline)
        body = self.server.body
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def address_string(self):
        return "peer"

    def log_message(self, *args):
        pass


def log(server, line):
    with server.lock:
        with open(server.log, "a") as f:
            f.write(line + "\n")


def peer(server, request):
    cred = request.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
    pid, uid, _ = struct.unpack("3i", cred)
    try:
        with open("/proc/%d/comm" % pid) as f:
            comm = f.read().strip()
    except OSError:
        comm = "?"
    log(server, "peer pid=%d uid=%d comm=%s" % (pid, uid, comm))


class HttpUnix(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True

    def verify_request(self, request, client_address):
        peer(self, request)
        return True


class HttpTcp(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


def read_exact(conn, n):
    data = b""
    while len(data) < n:
        chunk = conn.recv(n - len(data))
        if not chunk:
            raise EOFError("short read")
        data += chunk
    return data


def pipe(src, dst):
    try:
        while True:
            chunk = src.recv(65536)
            if not chunk:
                break
            dst.sendall(chunk)
    except OSError:
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass


class Socks(socketserver.BaseRequestHandler):
    def handle(self):
        conn = self.request
        try:
            _, n = read_exact(conn, 2)
            read_exact(conn, n)
            conn.sendall(b"\x05\x00")
            _, cmd, _, atyp = read_exact(conn, 4)
            if atyp == 1:
                host = socket.inet_ntop(socket.AF_INET, read_exact(conn, 4))
            elif atyp == 4:
                host = "[%s]" % socket.inet_ntop(socket.AF_INET6, read_exact(conn, 16))
            else:
                host = read_exact(conn, read_exact(conn, 1)[0]).decode("ascii", "replace")
            (port,) = struct.unpack("!H", read_exact(conn, 2))
        except (EOFError, OSError):
            return
        target = "%s:%d" % (host, port)
        log(self.server, "connect %s cmd=%d" % (target, cmd))
        each = self.server.name == "*"
        if cmd != 1 or not each and target != self.server.name:
            conn.sendall(b"\x05\x02\x00\x01" + b"\0" * 6)
            return
        if not each:
            up = socket.create_connection(("127.0.0.1", self.server.tcp_port))
        else:
            try:
                up = socket.create_connection((host.strip("[]"), port), timeout=30)
                up.settimeout(None)
            except OSError as e:
                log(self.server, "connect %s failed: %s" % (target, e))
                conn.sendall(b"\x05\x05\x00\x01" + b"\0" * 6)
                return
        conn.sendall(b"\x05\x00\x00\x01" + b"\0" * 6)
        t = threading.Thread(target=pipe, args=(up, conn), daemon=True)
        t.start()
        pipe(conn, up)
        t.join()
        up.close()


class SocksUnix(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


class SocksTcp(socketserver.ThreadingMixIn, socketserver.TCPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    mode = sys.argv[1]
    os.umask(0)
    if mode == "http-unix":
        srv = HttpUnix(sys.argv[2], Handler)
        srv.body, srv.log = sys.argv[3].encode(), sys.argv[4]
    elif mode == "http-tcp":
        srv = HttpTcp(("127.0.0.1", int(sys.argv[2])), Handler)
        srv.body, srv.log = sys.argv[3].encode(), os.devnull
    elif mode in ("socks-unix", "socks-tcp"):
        if mode == "socks-unix":
            srv = SocksUnix(sys.argv[2], Socks)
        else:
            srv = SocksTcp(("127.0.0.1", int(sys.argv[2])), Socks)
        srv.log = sys.argv[3]
        srv.name, _, tcp_port = sys.argv[4].partition("=")
        srv.tcp_port = int(tcp_port or 0)
    else:
        print("unknown mode %s" % mode, file=sys.stderr)
        return 64
    srv.lock = threading.Lock()
    srv.serve_forever()
    return 0


sys.exit(main())
