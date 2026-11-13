#!/usr/bin/env python3
"""Check that no module of src/ refers to a module of a crate that it may not use.

  python3 -I scripts/check-layers.py [--list] [--plant-empty] [--self-test]

Each top module of src/ belongs to one target crate of the split
(docs/research/podnet/architecture.md, section 4.1; LAYER below). A crate
may use only the crates of USES. Each `crate::MODULE` of a file of src/
(also in a `use crate::{...}` group, also in a doc link) outside the
`#[cfg(test)]` items and outside the files of a test module is one
reference. A reference from a module to a module of a crate that its
crate may not use is a fault, except the references of the allowlist
(scripts/layers-allow.txt: `FROM -> TO` on each line). The allowlist
can only shrink: a row whose reference no longer exists is a
fault too. src/main.rs is the binary: it may use each crate.

Each crate of CRATES (crates/NAME/src) holds the modules of one layer:
a top module there that LAYER puts in another layer is a fault. A
`crate::MODULE` of such a file names a module of that crate, and a
`podnet_core::MODULE` or `podnet_serve::MODULE` of each file is a
reference to MODULE (T-046, T-051).
A crate of ONE_MODULE is one module of LAYER (crates/podnet-vnet is the
module vnet, T-048): each of its files is that module, a `crate::` of it
is a reference to itself, and a `podnet_vnet::` of each file is a
reference to that module.

A top module that LAYER does not name is a fault: give it a crate. A scan
that reads fewer than MIN_FILES files is a fault.

--list prints each reference between crates (allowed or not) as
`FROM -> TO`. --plant-empty scans an empty tree: it must exit 1.
--self-test plants faults in temporary trees.

Exit 0: no fault. Exit 1: a fault, one line for each.
"""

import argparse
import pathlib
import re
import sys
import tempfile

MIN_FILES = 100

LAYER = {
    # core: process, files, network, supervisor
    "exit": "core", "hop": "core", "state": "core", "report": "core",
    "decisions": "core", "metrics": "core", "rlimit": "core", "yaml": "core",
    "race": "core", "util": "core", "qr": "core", "ports": "core",
    "net": "core", "trust": "core", "egress": "core", "resolve": "core",
    "fetch": "core", "udp": "core", "connect": "core", "detach": "core",
    "instances": "core", "autostart": "core",
    "vnet": "vnet",
    "ws_relay": "relay", "relay_reverse": "relay",
    "dialers": "relay",
    "warp": "warp",
    "share": "serve", "gate": "serve", "service": "serve", "ingress": "serve",
    "origin": "serve", "feature": "serve", "transport": "serve",
    "relay": "serve", "ws": "serve", "tar": "serve", "tar_http": "serve",
    "git": "serve", "files": "serve", "road": "serve",
    "cloudflare": "cloudflare", "tunnel": "cloudflare", "edge_h2": "cloudflare",
    "edge_discovery": "cloudflare", "quicktunnel": "cloudflare",
    "config": "cloudflare", "proxy": "cloudflare",
    "doctor": "podnet", "preflight": "podnet", "agent": "podnet", "agent_files": "podnet",
    "status": "podnet", "bug_report": "podnet", "manual": "podnet",
    "update": "podnet", "fuzzing": "podnet", "lib": "podnet", "networks": "podnet", "roads": "podnet", "forwards": "podnet", "router": "podnet",
    "test_host": "podnet", "link": "podnet", "link_ctl": "podnet", "link_service": "podnet", "link_group": "podnet", "link_tcp": "podnet", "link_chain": "podnet", "link_alias": "podnet", "link_socks": "podnet", "link_http": "podnet", "link_exec": "podnet", "exit_stdio": "podnet", "exit_stdio_tests": "podnet", "link_query": "podnet", "carriers": "podnet", "components": "podnet", "build_features": "podnet", "identity": "podnet", "hello": "podnet", "session": "podnet", "services": "core", "main": "podnet",
    "cli": "podnet",
    "mesh_table": "podnet", "link_announce": "podnet", "link_forward": "podnet", "mesh_announce": "podnet",
    # The modules of crates/podnet-core that are new there.
    "build": "core", "network": "core",
    # The tests of the core that read a higher layer (src/core_tests.rs).
    "core_tests": "podnet",
    # The tests of the services that read a higher layer (src/serve_tests.rs).
    "serve_tests": "podnet",
    # The tests of the wg-quick reader with the WARP adapter (src/wg_quick_tests.rs).
    "wg_quick_tests": "podnet",
}

# The crates of the workspace (crates/NAME) and the layer of each. The
# lib.rs of a crate is the module NAME (with `-` as `_`) of that layer.
CRATES = {
    "podnet-core": "core",
    "podnet-serve": "serve",
    "podnet-vnet": "vnet",
}

# The crates of CRATES that are one module of LAYER, and that module.
ONE_MODULE = {
    "podnet-vnet": "vnet",
}

USES = {
    "core": {"core"},
    "vnet": {"core", "vnet"},
    "relay": {"core", "relay"},
    "warp": {"core", "vnet", "relay", "warp"},
    "serve": {"core", "serve"},
    "cloudflare": {"core", "relay", "warp", "serve", "cloudflare"},
    "podnet": {"core", "vnet", "relay", "warp", "serve", "cloudflare", "podnet"},
}

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def strip_tests(text):
    """Return the text without the items under #[cfg(test)], and the names
    of the test modules that live in their own file (`mod x;`)."""
    out = []
    files = set()
    i = 0
    pat = re.compile(r"#\[cfg\(test\)\]")
    while True:
        m = pat.search(text, i)
        if not m:
            out.append(text[i:])
            break
        out.append(text[i:m.start()])
        j = m.end()
        # Skip the other attributes and the blanks of the item.
        while True:
            while j < len(text) and text[j].isspace():
                j += 1
            if text.startswith("#[", j):
                depth = 0
                while j < len(text):
                    if text[j] == "[":
                        depth += 1
                    elif text[j] == "]":
                        depth -= 1
                        if depth == 0:
                            j += 1
                            break
                    j += 1
                continue
            break
        fm = re.match(r"(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;", text[j:])
        if fm:
            files.add(fm.group(1))
            i = j + fm.end()
            continue
        # The item ends at its first `;` before a `{`, or at the `}` that
        # closes its first `{`.
        k = j
        depth = 0
        while k < len(text):
            c = text[k]
            if c == ";" and depth == 0:
                k += 1
                break
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    k += 1
                    break
            k += 1
        i = k
    return "".join(out), files


def refs_of(text, own=True):
    """Yield each top module named after `crate::` (when own is true),
    `podnet_core::` or `podnet_serve::`, and the module of each
    `podnet_vnet::`."""
    for m in re.finditer(r"\b(crate|podnet_core|podnet_serve|podnet_vnet)\s*::\s*", text):
        if m.group(1) == "podnet_vnet":
            yield ONE_MODULE["podnet-vnet"]
            continue
        if m.group(1) == "crate" and not own:
            continue
        j = m.end()
        if text.startswith("{", j):
            depth = 0
            k = j
            while k < len(text):
                if text[k] == "{":
                    depth += 1
                elif text[k] == "}":
                    depth -= 1
                    if depth == 0:
                        break
                k += 1
            body = text[j + 1:k]
            # The top names of the group: the idents at depth 0 that start
            # a path segment.
            depth = 0
            start = True
            for t in re.finditer(r"[{}]|,|::|[A-Za-z_][A-Za-z0-9_]*", body):
                s = t.group(0)
                if s == "{":
                    depth += 1
                elif s == "}":
                    depth -= 1
                elif s == ",":
                    if depth == 0:
                        start = True
                elif s == "::":
                    pass
                else:
                    if depth == 0 and start:
                        yield s
                    start = False
        else:
            im = IDENT.match(text, j)
            if im:
                yield im.group(0)


def top_module(src, path):
    rel = path.relative_to(src)
    return rel.parts[0][:-3] if len(rel.parts) == 1 else rel.parts[0]


def scan(root):
    src = root / "src"
    files = sorted(src.rglob("*.rs")) if src.is_dir() else []
    # The source directory of each file of a crate of CRATES, and its layer.
    home = {}
    for name, layer in CRATES.items():
        csrc = root / "crates" / name / "src"
        lib = name.replace("-", "_")
        LAYER.setdefault(lib, layer)
        if name in ONE_MODULE:
            lib = ONE_MODULE[name]
        for f in sorted(csrc.rglob("*.rs")) if csrc.is_dir() else []:
            home[f] = (csrc, layer, lib)
            files.append(f)
    faults = []
    edges = {}
    skip = set()
    skip_dirs = set()
    stripped = {}
    for f in files:
        text, tests = strip_tests(f.read_text(encoding="utf-8"))
        base = f.parent if f.name in ("mod.rs", "lib.rs", "main.rs") else f.with_suffix("")
        for t in tests:
            skip.add(base / (t + ".rs"))
            # The files of a test module are test code too.
            skip_dirs.add(base / t)
        stripped[f] = text
    for f in files:
        if f in skip or any(d in f.parents for d in skip_dirs):
            continue
        if f in home:
            csrc, layer, lib = home[f]
            frm = top_module(csrc, f)
            if lib in ONE_MODULE.values():
                frm = lib
            elif frm == "lib":
                frm = lib
            elif frm in LAYER and LAYER[frm] != layer:
                faults.append(f"{f.relative_to(root)}: the module {frm} ({LAYER[frm]}) is in a crate of {layer}")
                continue
        else:
            frm = top_module(src, f)
        if frm not in LAYER:
            faults.append(f"{f.relative_to(root)}: the module {frm} has no crate in LAYER")
            continue
        own = not (f in home and home[f][2] in ONE_MODULE.values())
        for to in refs_of(stripped[f], own):
            if to in ("self", "super") or to == frm or to[0].isupper():
                continue
            if to not in LAYER:
                continue
            edges.setdefault((frm, to), f.relative_to(root))
    return files, edges, faults


def read_allow(path):
    rows = {}
    if not path.is_file():
        return rows
    for n, line in enumerate(path.read_text().splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        m = re.fullmatch(r"(\w+)\s*->\s*(\w+)", line)
        if not m:
            rows[("?", str(n))] = n
            continue
        rows[(m.group(1), m.group(2))] = n
    return rows


def check(root, allow_path, listing=False):
    files, edges, faults = scan(root)
    if len(files) < MIN_FILES:
        faults.append(f"check-layers: {len(files)} files read, fewer than {MIN_FILES}")
    allow = read_allow(allow_path)
    for (a, b), n in allow.items():
        if a == "?":
            faults.append(f"{allow_path.name}:{n}: not a `FROM -> TO` line")
    for (a, b), where in sorted(edges.items()):
        la, lb = LAYER[a], LAYER[b]
        if la == lb:
            continue
        ok = lb in USES[la]
        if listing:
            print(f"{a} -> {b}{'' if ok else '  (up)'}  {where}")
        if not ok and (a, b) not in allow:
            faults.append(f"{where}: {a} ({la}) -> {b} ({lb}): {la} may not use {lb}")
    for (a, b), n in sorted(allow.items(), key=lambda x: x[1]):
        if a == "?":
            continue
        if (a, b) not in edges:
            faults.append(f"{allow_path.name}:{n}: {a} -> {b} no longer exists: remove the row")
        elif a in LAYER and b in LAYER and LAYER[b] in USES[LAYER[a]]:
            faults.append(f"{allow_path.name}:{n}: {a} -> {b} is allowed: remove the row")
    return faults


def self_test():
    ok = True

    def tree(files):
        d = pathlib.Path(tempfile.mkdtemp())
        (d / "src").mkdir()
        for i in range(MIN_FILES):
            (d / "src" / f"util{i}.rs").write_text("")
        LAYER.update({f"util{i}": "core" for i in range(MIN_FILES)})
        for name, text in files.items():
            p = d / name if name.startswith("crates/") else d / "src" / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)
        return d

    cases = [
        ("control", {"egress.rs": "use crate::util::x;\n"}, "", False),
        ("upward", {"egress.rs": "use crate::tunnel;\n", "tunnel.rs": ""}, "", True),
        ("group", {"egress.rs": "use crate::{util, tunnel::A};\n"}, "", True),
        ("test item", {"egress.rs": "#[cfg(test)]\nmod tests { use crate::tunnel; }\n"}, "", False),
        ("test file", {"egress.rs": "#[cfg(test)]\nmod t;\n", "egress/t.rs": "use crate::tunnel;\n"}, "", False),
        ("test subtree", {"egress.rs": "#[cfg(test)]\nmod t;\n", "egress/t/mod.rs": "mod u;\n",
                          "egress/t/u.rs": "use crate::tunnel;\n"}, "", False),
        ("not a test subtree", {"egress.rs": "mod t;\n", "egress/t/mod.rs": "mod u;\n",
                                "egress/t/u.rs": "use crate::tunnel;\n"}, "", True),
        ("allowed", {"egress.rs": "use crate::tunnel;\n"}, "egress -> tunnel\n", False),
        ("stale row", {"egress.rs": ""}, "egress -> tunnel\n", True),
        ("unknown", {"zzz.rs": ""}, "", True),
        ("core crate", {"crates/podnet-core/src/lib.rs": "pub mod exit;\n",
                        "crates/podnet-core/src/exit.rs": "use crate::util::x;\n"}, "", False),
        ("core crate upward", {"crates/podnet-core/src/exit.rs": "use crate::tunnel;\n"}, "", True),
        ("wrong crate", {"crates/podnet-core/src/tunnel.rs": ""}, "", True),
        ("core path", {"egress.rs": "use podnet_core::tunnel;\n"}, "", True),
        ("serve crate", {"crates/podnet-serve/src/gate.rs": "use podnet_core::util;\n"}, "", False),
        ("serve crate upward", {"crates/podnet-serve/src/gate.rs": "use crate::tunnel;\n"}, "", True),
        ("serve path", {"crates/podnet-core/src/exit.rs": "use podnet_serve::gate;\n"}, "", True),
        ("vnet crate", {"crates/podnet-vnet/src/lib.rs": "pub mod doctor;\n",
                        "crates/podnet-vnet/src/doctor.rs": "use crate::proxy;\nuse podnet_core::util;\n"},
         "", False),
        ("vnet crate upward", {"crates/podnet-vnet/src/dns.rs": "use podnet_core::util;\nuse podnet_core::tunnel;\n"},
         "", True),
        ("vnet path", {"egress.rs": "use podnet_vnet::stack;\n"}, "", True),
    ]
    for name, files, allow, want in cases:
        d = tree(files)
        a = d / "allow.txt"
        a.write_text(allow)
        got = bool(check(d, a))
        if got != want:
            print(f"self-test: {name}: fault={got}, want {want}")
            ok = False
    d = pathlib.Path(tempfile.mkdtemp())
    if not check(d, d / "allow.txt"):
        print("self-test: an empty tree passes")
        ok = False
    print("self-test: " + ("ok" if ok else "FAILED"))
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--plant-empty", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    root = pathlib.Path(__file__).resolve().parent.parent
    allow = root / "scripts" / "layers-allow.txt"
    if a.plant_empty:
        root = pathlib.Path(tempfile.mkdtemp())
    faults = check(root, allow, a.list)
    for f in faults:
        print(f)
    print(f"check-layers: {len(faults)} faults")
    return 1 if faults else 0


if __name__ == "__main__":
    sys.exit(main())
