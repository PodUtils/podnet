#!/usr/bin/env python3
"""The promises of each command of podnet, and the promises with a test (T-455).

Usage:
    python3 scripts/promises.py --report PODNET
    python3 scripts/promises.py --notes PODNET

A promise is a flag or an argument of a command, as `PODNET man --json`
gives it. The global flags of podnet are not counted again for each
command. A flag that each subcommand of a group has (the shared options of
`warp`) is one promise of the row `GROUP (each)`; it has a test when a call
of any subcommand of the group gives it. A promise has a test when:

* bb (black box): a call of the binary in a scenario (a step of
  .github/workflows/live.yml, a step of the job static of ci.yml, or a case
  of scripts/lab-lane.sh with the scripts that they run) gives the flag to
  the command; an argument counts when the call gives a word after the
  command;
* test: a statement of a Rust test (tests/, src/**/*_tests.rs or tests.rs,
  the same in crates/, and the text after `#[cfg(test)]` of a source
  file) holds the words of the command and the flag as
  string literals, in the order of the command line. In a file where one
  statement gives only the words of the command (a helper such as
  `fn pod() -> Command`), each statement counts for the command. For a
  command of one word, each statement of a test target named after it
  (tests/pod_report.rs, tests/pod_seccomp/) counts for it. Each test of
  crates/podnet-netns counts for `pod` (it parses the line after `pod`).

The count is a measure, not a proof: a flag with a test can still break,
and the match of a Rust statement is a match of text. A command with no
scenario is experimental (scripts/gate-scenarios.py; its help says so).

--report: one line for each command: the count of promises, with a test,
in a scenario; then the promises with no test.
--notes: the section "The promises of each command" of the notes of a
release.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import importlib.util

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("scenarios", ROOT / "scripts/gate-scenarios.py")
scenarios = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(scenarios)

STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')


def models(podnet):
    """For each path (words joined by a space; "" is the root), its flags
    and its arguments, from one call of `man --json`."""
    out = subprocess.run([podnet, "man", "--json"], capture_output=True, text=True, check=True)
    table = {}
    for c in json.loads(out.stdout)["commands"]:
        flags = [f"--{f['long']}" for f in c["flags"] if f.get("long")]
        for f in c["flags"]:
            if f.get("long") and f.get("short"):
                SHORT[f"--{f['long']}"] = f"-{f['short']}"
        table[" ".join(c["path"])] = (flags, [a["name"] for a in c["args"]])
    return table


# The short spelling of each long flag that has one (`--rule`: `-r`). A
# short flag can mean another flag in another command; the measure takes
# that risk.
SHORT = {}


def blackbox_calls(paths):
    """For each path, the list of the word lists of its calls in the
    scenarios."""
    texts = [scenarios.with_scripts(t) for _, t in scenarios.steps(scenarios.LIVE)]
    texts += [scenarios.with_scripts(t) for _, t in scenarios.steps(scenarios.CI, "static")]
    lane = scenarios.LANE.read_text()
    texts.append(scenarios.with_scripts(lane))
    calls = {p: [] for p in paths}
    # The lane runs scripts/lab-errors.sh by a path beside itself, and each
    # row calls podnet through a helper: `run NAME LIMIT ARGS...` and
    # `pod_row NAME CAUSE FLAGS...` (pod).
    for line in (ROOT / "scripts/lab-errors.sh").read_text().splitlines():
        m = re.match(r"^run \S+ \d+ (.*)$", line) or re.match(r'^pod_row \S+ "[^"]*" (.*)$', line)
        if m:
            words = m.group(1).split()
            if line.startswith("pod_row"):
                words = ["pod", *words]
            path = match_path(words, paths)
            if path:
                calls[path].append(words)
    for text in texts:
        for m in scenarios.CALL.finditer(text):
            words = [w.strip("\"'") for w in m.group(1).split()]
            path = match_path(words, paths)
            if path:
                calls[path].append(words)
    return calls


def match_path(words, paths):
    """The longest path of PATHS whose words come first in WORDS, after
    the flags before the command."""
    plain = [w for w in words if not w.startswith("-")]
    best = None
    for p in paths:
        parts = p.split()
        if plain[: len(parts)] == parts and (best is None or len(parts) > len(best.split())):
            best = p
    return best


def is_test_file(f):
    return "tests" in f.parts or f.name.endswith("_tests.rs") or f.name == "tests.rs"


def test_files():
    """For each file of Rust tests, the words of the string literals of
    each statement."""
    files = list((ROOT / "tests").rglob("*.rs"))
    files += list((ROOT / "src").rglob("*.rs")) + list((ROOT / "crates").rglob("*.rs"))
    out = {}
    for f in dict.fromkeys(files):
        text = f.read_text(errors="replace")
        if not is_test_file(f):
            # A source file: only its module of tests (the text after
            # `#[cfg(test)]`), not the help texts of its code.
            at = text.find("#[cfg(test)]")
            if at < 0:
                continue
            text = text[at:]
        statements = []
        for statement in text.split(";"):
            words = []
            for lit in STRING.findall(statement):
                words += lit.split()
            if words:
                statements.append(words)
        out[f.relative_to(ROOT).as_posix()] = statements
    return out


def statements_of(files, parts):
    """The statements of the tests that count for the command PARTS."""
    out = []
    for name, statements in files.items():
        target = name.split("/")[1] if name.startswith("tests/") else ""
        if parts == ["pod"] and name.startswith("crates/podnet-netns/"):
            out += statements
        elif len(parts) == 1 and (target == f"{parts[0]}.rs" or target.startswith(f"{parts[0]}_")):
            # A test target named after the command (tests/pod_report.rs,
            # tests/pod_seccomp/): its helpers can be in another file.
            out += statements
        elif any(s == parts for s in statements):
            out += statements
        else:
            out += [s for s in statements if ordered(s, parts)]
    return out


def ordered(words, parts):
    """True when PARTS come in WORDS in their order."""
    i = 0
    for w in words:
        if i < len(parts) and w == parts[i]:
            i += 1
    return i == len(parts)


def flag_in(words, flag):
    short = SHORT.get(flag)
    return any(w == flag or w.startswith(flag + "=") or (short and w == short) for w in words)


def group_flags(paths, table, global_flags):
    """For each group of two or more subcommands, the flags that each
    subcommand has."""
    groups = {}
    for p in paths:
        if " " in p:
            groups.setdefault(p.split()[0], []).append(p)
    shared = {}
    for g, members in groups.items():
        if len(members) < 2:
            continue
        sets = [set(table[m][0]) - set(global_flags) for m in members]
        common = set.intersection(*sets)
        if common:
            shared[g] = (members, sorted(common))
    return shared


def measure(podnet):
    paths = scenarios.commands()
    table = models(podnet)
    global_flags = table[""][0]
    calls = blackbox_calls(paths)
    files = test_files()
    shared = group_flags(paths, table, global_flags)
    rows = []
    for g, (members, flags) in shared.items():
        bb, tested = set(), set()
        for m in members:
            mine = statements_of(files, m.split())
            for f in flags:
                if any(flag_in(c, f) for c in calls[m]):
                    bb.add(f)
                if any(flag_in(w, f) for w in mine):
                    tested.add(f)
        rows.append((f"{g} (each)", flags, bb, tested | bb))
    for p in paths:
        parts = p.split()
        flags, args = table[p]
        group = shared.get(parts[0], ([], []))[1]
        flags = [f for f in flags if f not in global_flags and f != "--help" and f not in group]
        mine = statements_of(files, parts)
        bb, tested = set(), set()
        for f in flags:
            if any(flag_in(c, f) for c in calls[p]):
                bb.add(f)
            if any(flag_in(w, f) for w in mine):
                tested.add(f)
        for a in args:
            # An argument: a call with a word after the command.
            if any(len([w for w in c if not w.startswith("-")]) > len(parts) for c in calls[p]):
                bb.add(a)
            if mine:
                tested.add(a)
        promises = flags + args
        rows.append((p, promises, bb, tested | bb))
    return rows


def report(podnet):
    rows = measure(podnet)
    total = sum(len(r[1]) for r in rows)
    with_test = sum(len(r[3]) for r in rows)
    print(f"{with_test} of {total} promises have a test")
    for p, promises, bb, tested in rows:
        missing = [x for x in promises if x not in tested]
        print(f"{p}: {len(promises)} promises, {len(tested)} with a test, {len(bb)} in a scenario; no test: {' '.join(missing)}")


def notes(podnet):
    rows = measure(podnet)
    total = sum(len(r[1]) for r in rows)
    with_test = sum(len(r[3]) for r in rows)
    bb = sum(len(r[2]) for r in rows)
    print("## The promises of each command\n")
    print(
        f"A promise is a flag or an argument of a command. {with_test} of {total} promises have a test (a Rust test or a "
        f"scenario of the release binary), and {bb} run in a scenario of the release binary. The count is a measure of "
        "the reach of the tests, not of correctness (`scripts/promises.py`).\n"
    )
    print("<details><summary>Each command: promises, with a test, in a scenario</summary>\n")
    print("| Command | Promises | With a test | In a scenario |\n| --- | --- | --- | --- |")
    for p, promises, b, tested in rows:
        print(f"| `podnet {p}` | {len(promises)} | {len(tested)} | {len(b)} |")
    print("\n</details>")


def main(argv):
    if len(argv) == 2 and argv[0] in ("--report", "--notes"):
        (report if argv[0] == "--report" else notes)(argv[1])
        return 0
    print(__doc__.strip().splitlines()[2], file=sys.stderr)
    return 64


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
