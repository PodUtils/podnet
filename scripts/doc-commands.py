#!/usr/bin/env python3
"""The documented commands as tests (T-358, step 2).

Extract each fenced `sh` or `bash` block of README.md and docs/*.md (not
docs/releases/, docs/reports/ or docs/research/). Give each block its
class from scripts/doc-commands.txt.

The list has one line for each block:

    FILE HASH CLASS REASON

FILE is the path from the root of the repository. HASH is the first 10
hex digits of the SHA-256 of the block text (the indent removed). CLASS
is one of:

    run   runs offline as written, with `podnet` on PATH
    live  needs a live service or a second host (for live.yml later)
    root  needs root or a kernel feature
    skip  not a command that a user runs (output, a template)

REASON is one line. A line that starts with `#` is a comment. A change of
the text of a block changes its hash: the block then needs a new decision.

    python3 -I scripts/doc-commands.py --list    print the table
    python3 -I scripts/doc-commands.py --check   fail on an unclassified
                                                 block or a stale entry
    python3 -I scripts/doc-commands.py --run [--bin PATH] [--timeout S]
                                                 [--only FILE[:LINE]]
        run each `run` block with `sh -e`, in a new temporary HOME and
        working directory, with `podnet` (default target/debug/podnet)
        first on PATH; print pass or fail, and the output of a failure.
        A reason that starts with `[needs TOOL,TOOL]` names the other
        programs of the block: with one of them absent, the block is not
        run (MISS), and the exit code is 1.
    python3 -I scripts/doc-commands.py --lines
        print each `podnet` command line of each block (all classes) as
        one JSON object on each line: {"at": "FILE:LINE", "args": [...]}.
        A line that ends with a backslash continues on the next line. The words
        `VAR=value` before the command are not in `args`; a pipe, a list
        operator, a redirection and `$(` end the command. The test
        `each_command_of_a_document_parses` (src/cli/help_tests.rs)
        parses each line with the command model of podnet (T-526).
"""

import argparse
import hashlib
import json
import os
import pathlib
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
LIST = ROOT / "scripts" / "doc-commands.txt"
CLASSES = ("run", "live", "root", "skip")
EXCLUDE = ("docs/releases/", "docs/reports/", "docs/research/")
OPEN = re.compile(r"^(\s*)```(sh|bash)\s*$")
CLOSE = re.compile(r"^\s*```\s*$")


def documents():
    files = [ROOT / "README.md"] + sorted((ROOT / "docs").glob("*.md"))
    return [f for f in files if f.is_file()]


def blocks():
    """Return a list of (file, line, hash, text) for each block."""
    out = []
    for path in documents():
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith(EXCLUDE):
            continue
        lines = path.read_text(encoding="utf-8").split("\n")
        i = 0
        while i < len(lines):
            m = OPEN.match(lines[i])
            if not m:
                i += 1
                continue
            start, indent, body = i + 1, len(m.group(1)), []
            i += 1
            while i < len(lines) and not CLOSE.match(lines[i]):
                prefix = lines[i][:indent]
                body.append(lines[i][indent:] if prefix.strip() == "" else lines[i].lstrip())
                i += 1
            if i >= len(lines):
                sys.exit(f"{rel}:{start}: the block has no closing fence")
            text = "\n".join(body).strip("\n") + "\n"
            digest = hashlib.sha256(text.encode()).hexdigest()[:10]
            out.append((rel, start, digest, text))
            i += 1
    return out


ASSIGN = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
# A placeholder `<NAME>` of a document is a value, not a redirection.
PLACEHOLDER = re.compile(r"<([A-Za-z][-A-Za-z0-9_]*)>")
BREAK = set("|&;()<>$")


def command_lines(text, start):
    """Give (line, args) for each `podnet` command of the block `text`
    that starts at the line `start` of its file."""
    out, joined, first = [], "", None
    for n, raw in enumerate(text.split("\n"), start + 1):
        if first is None:
            first = n
        if raw.rstrip().endswith("\\"):
            joined += raw.rstrip()[:-1] + " "
            continue
        joined += raw
        line, joined, at = PLACEHOLDER.sub(r"\1", joined), "", first
        first = None
        try:
            lex = shlex.shlex(line, posix=True, punctuation_chars=True)
            lex.whitespace_split = True
            lex.commenters = "#"
            words = list(lex)
        except ValueError:
            continue
        segment = []
        for i, word in enumerate(words + [";"]):
            if word and set(word) <= BREAK or word.startswith("$("):
                while segment and ASSIGN.match(segment[0]):
                    segment.pop(0)
                if segment and segment[0] == "podnet":
                    out.append((at, segment))
                # A redirection: the next word is a file, not a command.
                segment = [] if not set(word) & set("<>") else ["<file>"]
                continue
            nxt = words[i + 1] if i + 1 < len(words) else ""
            if word.isdigit() and nxt[:1] in ("<", ">"):
                continue  # the number of a file descriptor
            if segment == ["<file>"]:
                segment = ["<after-file>"]
                continue
            segment.append(word)
    return out


def read_list():
    """Return {(file, hash): (class, reason, line of the list)}."""
    entries, errors = {}, []
    for n, raw in enumerate(LIST.read_text(encoding="utf-8").split("\n"), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 3)
        if len(parts) < 4 or parts[2] not in CLASSES or not re.fullmatch(r"[0-9a-f]{10}", parts[1]):
            errors.append(f"{LIST.name}:{n}: expected FILE HASH CLASS REASON, class in {CLASSES}")
            continue
        key = (parts[0], parts[1])
        if key in entries:
            errors.append(f"{LIST.name}:{n}: a second entry for {parts[0]} {parts[1]}")
        entries[key] = (parts[2], parts[3], n)
    return entries, errors


def check(found, entries, errors):
    keys = {(f, h) for f, _, h, _ in found}
    for f, line, h, text in found:
        if (f, h) not in entries:
            first = text.split("\n", 1)[0]
            errors.append(f"{f}:{line}: block {h} has no class in {LIST.name} ({first})")
    for key, (_, _, n) in sorted(entries.items(), key=lambda e: e[1][2]):
        if key not in keys:
            errors.append(f"{LIST.name}:{n}: stale entry {key[0]} {key[1]}: no such block")
    return errors


def run_block(text, binary, timeout):
    with tempfile.TemporaryDirectory(prefix="podnet-doc-") as tmp:
        tmp = pathlib.Path(tmp)
        for d in ("home", "work", "bin", "tmp"):
            (tmp / d).mkdir()
        os.symlink(binary, tmp / "bin" / "podnet")
        env = {
            "HOME": str(tmp / "home"),
            "TMPDIR": str(tmp / "tmp"),
            "PATH": f"{tmp / 'bin'}:/usr/local/bin:/usr/bin:/bin",
            "LANG": "C.UTF-8",
        }
        try:
            p = subprocess.run(
                ["sh", "-e", "-c", text], cwd=tmp / "work", env=env, timeout=timeout,
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            return p.returncode, p.stdout.decode(errors="replace")
        except subprocess.TimeoutExpired as e:
            return "timeout", (e.stdout or b"").decode(errors="replace")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--list", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--run", action="store_true")
    mode.add_argument("--lines", action="store_true")
    ap.add_argument("--bin", default=str(ROOT / "target" / "debug" / "podnet"))
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument("--only", help="FILE or FILE:LINE")
    a = ap.parse_args()
    sys.stdout.reconfigure(line_buffering=True)

    found = blocks()
    if a.lines:
        for f, line, _, text in found:
            for at, args in command_lines(text, line):
                print(json.dumps({"at": f"{f}:{at}", "args": args}))
        return 0
    entries, errors = read_list()
    errors = check(found, entries, errors)

    if a.list:
        counts = dict.fromkeys(CLASSES, 0)
        for f, line, h, text in found:
            cls, reason, _ = entries.get((f, h), ("NONE", "", 0))
            counts[cls] = counts.get(cls, 0) + 1
            print(f"{f}:{line}\t{h}\t{cls}\t{text.split(chr(10), 1)[0][:60]}\t{reason}")
        print(" ".join(f"{k}={v}" for k, v in counts.items()), f"total={len(found)}", file=sys.stderr)
        return 0
    if errors:
        for e in errors:
            print(f"doc-commands: {e}", file=sys.stderr)
        return 1
    if a.check:
        print(f"doc-commands: {len(found)} blocks, each with a class", file=sys.stderr)
        return 0

    binary = pathlib.Path(a.bin).resolve()
    if not os.access(binary, os.X_OK):
        print(f"doc-commands: no binary at {binary}; build it or give --bin", file=sys.stderr)
        return 1
    if shutil.which("sh") is None:
        print("doc-commands: no sh", file=sys.stderr)
        return 1
    failed = passed = missing = 0
    for f, line, h, text in found:
        cls, reason, _ = entries[(f, h)]
        if cls != "run":
            continue
        if a.only and a.only not in (f, f"{f}:{line}"):
            continue
        need = re.match(r"\[needs ([^\]]+)\]", reason)
        absent = [t for t in (need.group(1).split(",") if need else []) if shutil.which(t) is None]
        if absent:
            missing += 1
            print(f"MISS {f}:{line} {h}: not run, no {', '.join(absent)} on this host")
            continue
        rc, output = run_block(text, binary, a.timeout)
        if rc == 0:
            passed += 1
            print(f"pass {f}:{line} {h}")
        else:
            failed += 1
            print(f"FAIL {f}:{line} {h} (exit {rc})")
            for cmd in text.rstrip("\n").split("\n"):
                print(f"  $ {cmd}")
            for out in output.rstrip("\n").split("\n")[-40:]:
                print(f"  | {out}")
    print(f"doc-commands: {passed} passed, {failed} failed, {missing} not run", file=sys.stderr)
    return 1 if failed or missing else 0


if __name__ == "__main__":
    sys.exit(main())
