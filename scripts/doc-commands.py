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

    run       runs offline as written, with `podnet` on PATH
    scenario  runs offline as written after a set-up: the scenario
              scripts/doc-scenarios/HASH.sh (see below)
    live      needs a live service or a second host (for live.yml later)
    root      needs root or a kernel feature
    skip      not a command that a user runs (output, a template)

REASON is one line. A line that starts with `#` is a comment. A change of
the text of a block changes its hash: the block then needs a new decision.

    python3 -I scripts/doc-commands.py --list    print the table
    python3 -I scripts/doc-commands.py --check   fail on an unclassified
                                                 block or a stale entry
    python3 -I scripts/doc-commands.py --run [--bin PATH] [--timeout S]
                                                 [--only FILE[:LINE]]
        run each `run` and `scenario` block with `sh -e`, in a new
        temporary HOME and working directory, with `podnet` (default
        target/debug/podnet) first on PATH; print pass or fail, and the
        output of a failure. A reason that starts with `[needs TOOL,TOOL]`
        names the other programs of the block: with one of them absent,
        the block is not run (MISS), and the exit code is 1.
    python3 -I scripts/doc-commands.py --lines
        print each `podnet` command line of each block (all classes) as
        one JSON object on each line: {"at": "FILE:LINE", "args": [...]}.
        A line that ends with a backslash continues on the next line. The words
        `VAR=value` before the command are not in `args`; a pipe, a list
        operator, a redirection and `$(` end the command. The test
        `each_command_of_a_document_parses` (src/cli/help_tests.rs)
        parses each line with the command model of podnet (T-526).

A scenario is a shell file that the block shares with
scripts/doc-scenarios/lib.sh (the helpers). DOC_ROOT is the root of the
repository, and DOC_DIR a private directory of the run. It defines the
function `setup` (files, variables with `export`, servers and stand-ins
with `doc_bg`), and it can define `ready`, `check` (after the block) and
`teardown`. `BACKGROUND="N M"` names the lines of the block (from 1) that
run in the background, as the first shell of a block of two shells: each
starts at its place in the block, then `ready N` runs (N is 1 for the
first background line). A line that ends with a backslash takes the next
line with it. `STDIN=FILE` gives the block its stdin (a typed password).
`LIMIT=SECONDS` gives the block a longer time limit than --timeout. At the
end, a stop ends each background command, each detached instance and the
process group of the block. A set-up that calls `doc_na REASON` reports
the block as not run on this host (n.a.); n.a. does not change the exit
code. `--check` also fails on a `scenario` line with no scenario file, on
a scenario file with no `scenario` line or no `setup()`, and on a
BACKGROUND line that names no line of the block.
"""

import argparse
import hashlib
import json
import os
import pathlib
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
LIST = ROOT / "scripts" / "doc-commands.txt"
CLASSES = ("run", "scenario", "live", "root", "skip")
SCENARIOS = ROOT / "scripts" / "doc-scenarios"
LIB = SCENARIOS / "lib.sh"
BACKGROUND = re.compile(r'^BACKGROUND="?([0-9 ]*)"?\s*$', re.M)
SETUP = re.compile(r"^setup\(\)", re.M)
LIMIT = re.compile(r'^LIMIT="?([0-9]+)"?\s*$', re.M)
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
    texts = {(f, h): text for f, _, h, text in found}
    for (f, h), (cls, _, n) in sorted(entries.items(), key=lambda e: e[1][2]):
        if cls != "scenario":
            continue
        path = SCENARIOS / f"{h}.sh"
        if not path.is_file():
            errors.append(f"{LIST.name}:{n}: the scenario {f} {h} has no set-up {path.relative_to(ROOT)}")
            continue
        body = path.read_text(encoding="utf-8")
        if not SETUP.search(body):
            errors.append(f"{path.relative_to(ROOT)}: no function setup()")
        if (f, h) in texts:
            try:
                background(body, texts[(f, h)])
            except ValueError as e:
                errors.append(f"{path.relative_to(ROOT)}: {e}")
    used = {h for (_, h), (cls, _, _) in entries.items() if cls == "scenario"}
    for path in sorted(SCENARIOS.glob("*.sh")):
        if path != LIB and path.stem not in used:
            errors.append(f"{path.relative_to(ROOT)}: a set-up with no `scenario` line in {LIST.name}")
    return errors


def background(scenario, text):
    """Split the block `text` by the BACKGROUND line of `scenario`. Give
    (the foreground text, [the text of each background command])."""
    m = BACKGROUND.search(scenario)
    numbers = [int(x) for x in m.group(1).split()] if m else []
    lines = text.rstrip("\n").split("\n")
    fg, bg = list(lines), []
    for n in numbers:
        if not 1 <= n <= len(lines) or fg[n - 1] != lines[n - 1]:
            raise ValueError(f"BACKGROUND names the line {n}, and the block has no such line")
        command, i = [], n - 1
        while True:
            command.append(lines[i])
            fg[i] = ":"
            if not lines[i].rstrip().endswith("\\") or i + 1 >= len(lines):
                break
            i += 1
        line = "\n".join(command) + "\n"
        # One command takes the place of its shell, so a stop reaches it.
        bg.append(line if set(line) & set("|;&") else "exec " + line)
        fg[n - 1] = f"doc_start {len(bg)}"
    return FG_HEAD + "\n".join(fg) + "\n", bg


# The driver of a scenario: the helpers, the scenario, the set-up, the
# block, the check, and the stop at each exit.
DRIVER = """set -e
. "$DOC_LIB"
ready() { :; }
check() { :; }
teardown() { :; }
. "$DOC_SCENARIO"
doc_exit() { doc_rc=$?; set +e; teardown; doc_stop; exit "$doc_rc"; }
trap doc_exit EXIT
trap 'exit 143' HUP INT TERM
setup
sh -e "$DOC_DIR/fg.sh" < "${STDIN:-/dev/null}"
check
"""
# The head of the block of a scenario: a background line of the block is
# `doc_start N`, which starts the command N and calls `ready N`.
FG_HEAD = """. "$DOC_LIB"; ready() { :; }; . "$DOC_SCENARIO"
doc_start() { doc_bg sh -e "$DOC_DIR/bg-$1.sh"; ready "$1"; }
"""


def kill_group(p):
    """End the process group of `p` (the block and each child of it): a
    SIGTERM, 3 s for a clean stop (a listener removes its socket), then a
    SIGKILL."""
    try:
        os.killpg(p.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    end = time.monotonic() + 3
    while time.monotonic() < end:
        p.poll()
        try:
            os.killpg(p.pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.05)
    try:
        os.killpg(p.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    p.wait()


def run_block(text, binary, timeout, scenario=None):
    """Run one block. Give (exit code or "timeout" or "n.a.", output)."""
    with tempfile.TemporaryDirectory(prefix="podnet-doc-") as tmp:
        tmp = pathlib.Path(tmp)
        for d in ("home", "work", "bin", "tmp", "doc"):
            (tmp / d).mkdir()
        os.symlink(binary, tmp / "bin" / "podnet")
        env = {
            "HOME": str(tmp / "home"),
            "TMPDIR": str(tmp / "tmp"),
            "PATH": f"{tmp / 'bin'}:/usr/local/bin:/usr/bin:/bin",
            "LANG": "C.UTF-8",
        }
        argv = ["sh", "-e", "-c", text]
        if scenario is not None:
            doc = tmp / "doc"
            fg, bg = background(scenario.read_text(encoding="utf-8"), text)
            (doc / "fg.sh").write_text(fg)
            for i, command in enumerate(bg, 1):
                (doc / f"bg-{i}.sh").write_text(command)
            (doc / "pids").write_text("")
            env.update(DOC_DIR=str(doc), DOC_LIB=str(LIB), DOC_SCENARIO=str(scenario), DOC_ROOT=str(ROOT))
            argv = ["sh", "-c", DRIVER]
        # The output goes to a file, not a pipe: a command that the block
        # starts with `&` keeps its output open after the block ends.
        with open(tmp / "output", "wb") as out:
            p = subprocess.Popen(
                argv, cwd=tmp / "work", env=env, stdin=subprocess.DEVNULL,
                stdout=out, stderr=subprocess.STDOUT, start_new_session=True,
            )
        try:
            rc = p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            rc = "timeout"
        kill_group(p)
        output = (tmp / "output").read_bytes().decode(errors="replace")
        if scenario is not None:
            na = tmp / "doc" / "na"
            if na.is_file():
                return "n.a.", na.read_text().strip()
            if rc != 0:
                for log in sorted((tmp / "doc").glob("bg-*.log")):
                    tail = log.read_text(errors="replace").rstrip("\n").split("\n")[-20:]
                    output += f"\n-- {log.name} (a background command):\n" + "\n".join(tail)
        return rc, output


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
    failed = passed = missing = absent_host = 0
    start = time.monotonic()
    for f, line, h, text in found:
        cls, reason, _ = entries[(f, h)]
        if cls not in ("run", "scenario"):
            continue
        if a.only and a.only not in (f, f"{f}:{line}"):
            continue
        need = re.match(r"\[needs ([^\]]+)\]", reason)
        absent = [t for t in (need.group(1).split(",") if need else []) if shutil.which(t) is None]
        if absent:
            missing += 1
            print(f"MISS {f}:{line} {h}: not run, no {', '.join(absent)} on this host")
            continue
        scenario, limit = None, a.timeout
        if cls == "scenario":
            scenario = SCENARIOS / f"{h}.sh"
            m = LIMIT.search(scenario.read_text(encoding="utf-8"))
            limit = max(limit, float(m.group(1))) if m else limit
        rc, output = run_block(text, binary, limit, scenario)
        if rc == "n.a.":
            absent_host += 1
            print(f"n.a. {f}:{line} {h}: not run on this host: {output}")
        elif rc == 0:
            passed += 1
            print(f"pass {f}:{line} {h}")
        else:
            failed += 1
            print(f"FAIL {f}:{line} {h} (exit {rc})")
            for cmd in text.rstrip("\n").split("\n"):
                print(f"  $ {cmd}")
            for out in output.rstrip("\n").split("\n")[-40:]:
                print(f"  | {out}")
    print(
        f"doc-commands: {passed} passed, {failed} failed, {missing} not run (MISS), "
        f"{absent_host} not run on this host (n.a.), {time.monotonic() - start:.1f} s",
        file=sys.stderr,
    )
    return 1 if failed or missing else 0


if __name__ == "__main__":
    sys.exit(main())
