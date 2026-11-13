#!/usr/bin/env python3
"""The ctypes test of libpodnet (T-304): the same checks as tests/c_abi.c.

  python3 -I tests/c_abi.py PATH_TO_libpodnet.so WORKDIR

Two installs, a memory link, a stream to the service "echo", 1 MiB through
the peer, and the SHA-256 of the bytes that were sent against the bytes
that came back. Exit 0 when each check passes.
"""

import ctypes
import hashlib
import os
import sys
import threading

TOTAL = 1 << 20
CHUNK = 8192
E_LIMIT, E_STATE, E_PERMISSION = -6, -5, -3

lib = ctypes.CDLL(sys.argv[1])
work = sys.argv[2]
vp = ctypes.c_void_p
lib.podnet_last_error.restype = ctypes.c_char_p
lib.podnet_version.restype = ctypes.c_char_p
lib.podnet_stream_service.restype = ctypes.c_char_p
lib.podnet_stream_service.argtypes = [vp]
lib.podnet_init.argtypes = [ctypes.c_char_p, vp]
lib.podnet_identity_fingerprint.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_size_t]
lib.podnet_link_new.argtypes = [ctypes.c_char_p, ctypes.c_char_p, vp]
lib.podnet_link_pair.argtypes = [vp, vp]
lib.podnet_link_open.argtypes = [vp, ctypes.c_char_p, vp]
lib.podnet_link_accept.argtypes = [vp, vp]
lib.podnet_link_close.argtypes = [vp]
lib.podnet_stream_read.argtypes = [vp, vp, ctypes.c_size_t]
lib.podnet_stream_write.argtypes = [vp, vp, ctypes.c_size_t]
lib.podnet_stream_close.argtypes = [vp]


def ok(what, code):
    if code < 0:
        sys.exit(f"c_abi.py: {what}: {code}: {lib.podnet_last_error().decode()}")
    return code


def check(cond, what):
    if not cond:
        sys.exit(f"c_abi.py: {what}")


def new_link(dir_, fp):
    out = vp()
    ok("link_new", lib.podnet_link_new(dir_.encode(), fp, ctypes.byref(out)))
    return out


check(lib.podnet_version().decode().count(".") >= 2, "version")
err = ctypes.c_char_p()
ok("init", lib.podnet_init(None, ctypes.byref(err)))
ok("init again", lib.podnet_init(b'{"threads": 2}', None))
check(lib.podnet_init(b"{", ctypes.byref(err)) == -1 and err.value, "a bad config is refused")

dirs, fps = [], []
for i in range(2):
    d = os.path.join(work, f"py{i}")
    os.makedirs(d, mode=0o700, exist_ok=True)
    buf = ctypes.create_string_buffer(128)
    ok("fingerprint", lib.podnet_identity_fingerprint(d.encode(), buf, 128))
    dirs.append(d)
    fps.append(buf.value)
small = ctypes.create_string_buffer(4)
check(lib.podnet_identity_fingerprint(dirs[0].encode(), small, 4) == E_LIMIT, "a small buffer")

wrong_a, wrong_b = new_link(dirs[0], fps[0]), new_link(dirs[1], fps[0])
check(lib.podnet_link_pair(wrong_a, wrong_b) == E_PERMISSION, "a refused key")
ok("close", lib.podnet_link_close(wrong_a))
ok("close", lib.podnet_link_close(wrong_b))

a, b = new_link(dirs[0], fps[1]), new_link(dirs[1], fps[0])
ok("pair", lib.podnet_link_pair(a, b))
check(lib.podnet_link_pair(a, b) == E_STATE, "pair twice")

data = hashlib.sha256(b"podnet").digest() * (TOTAL // 32)
check(len(data) == TOTAL, "data size")
problems = []


def echo():
    s = vp()
    try:
        ok("accept", lib.podnet_link_accept(b, ctypes.byref(s)))
        check(lib.podnet_stream_service(s) == b"echo", "service name")
        buf = ctypes.create_string_buffer(CHUNK * 4)
        seen = 0
        while seen < TOTAL:
            n = ok("echo read", lib.podnet_stream_read(s, buf, len(buf)))
            if n == 0:
                break
            ok("echo write", lib.podnet_stream_write(s, buf, n))
            seen += n
        ok("echo close", lib.podnet_stream_close(s))
    except SystemExit as e:
        problems.append(str(e))


server = threading.Thread(target=echo)
server.start()
s = vp()
ok("open", lib.podnet_link_open(a, b"echo", ctypes.byref(s)))
back = hashlib.sha256()
got = [0]


def collect():
    buf = ctypes.create_string_buffer(CHUNK * 3)
    while got[0] < TOTAL:
        n = ok("read", lib.podnet_stream_read(s, buf, len(buf)))
        if n == 0:
            break
        back.update(buf.raw[:n])
        got[0] += n


reader = threading.Thread(target=collect)
reader.start()
for off in range(0, TOTAL, CHUNK):
    chunk = data[off : off + CHUNK]
    ok("write", lib.podnet_stream_write(s, chunk, len(chunk)))
server.join()
reader.join()
check(not problems, "; ".join(problems))
check(got[0] == TOTAL, f"read {got[0]} bytes")
check(back.digest() == hashlib.sha256(data).digest(), "the digest differs")
one = ctypes.create_string_buffer(1)
check(lib.podnet_stream_read(s, one, 1) == 0, "the end of the stream")
ok("close stream", lib.podnet_stream_close(s))
ok("close a", lib.podnet_link_close(a))
ok("close b", lib.podnet_link_close(b))
ok("free", lib.podnet_free())
print(f"c_abi.py: ok, 1 MiB echoed, sha256 {back.hexdigest()[:16]}")
