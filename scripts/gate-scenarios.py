#!/usr/bin/env python3
"""The scenarios of each command of podnet (T-358 step 4).

Usage:
    python3 scripts/gate-scenarios.py --check
    python3 scripts/gate-scenarios.py --write
    python3 scripts/gate-scenarios.py --notes LANE.TXT

A scenario runs the release binary of podnet as a user runs it:

* live:STEP, a step of .github/workflows/live.yml (each push to main),
* ci:STEP, a step of the job static of .github/workflows/ci.yml (the
  consumers on the release binary, each push),
* lane:CASE, a case of scripts/lab-lane.sh in the labs of pod-lab
  (gate.yml, the job gate of each release).

The script reads the text of each step, and of each script of scripts/
that the step runs, and finds each call of the podnet binary in it. The
file src/cli/scenarios.txt holds one line for each command of podnet with
no subcommand: the path of the command, then its scenarios. A command
with no scenario is experimental: its help says so (src/cli/help.rs), and
the notes of a release list it.

--check: the file agrees with the workflows and the lane (exit 1 if not).
--write: write the file again from the workflows and the lane.
--notes LANE.TXT: print the section "The scenarios of each command" of the
notes of a release. A lane scenario counts when a row of the case passes
in LANE.TXT (the output of scripts/lab-lane.sh).
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TABLE = ROOT / "src/cli/scenarios.txt"
LIVE = ROOT / ".github/workflows/live.yml"
CI = ROOT / ".github/workflows/ci.yml"
LANE = ROOT / "scripts/lab-lane.sh"

HEADER = """\
# The scenarios of each command of podnet with no subcommand (T-358 step 4).
# python3 scripts/gate-scenarios.py --write writes this file, and --check
# checks it. One line for each command: PATH | SCENARIO; SCENARIO ...
# A command with no scenario is experimental (its help says so).
"""

# A call of the binary: target/.../podnet, a path that ends in /podnet, or
# a variable that holds the binary.
CALL = re.compile(
    r'(?:[\w.${}/-]*/podnet"?|"\$\{?(?:podnet|PODNET|bin|exe)\}?"|\$\{?(?:podnet|PODNET)\}?)'
    r"((?:[ \t]+[^\s|;&()<>`]+)*)"
)


def commands():
    """The paths of the table, in its order."""
    paths = []
    for line in TABLE.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            paths.append(line.split("|", 1)[0].strip())
    return paths


def steps(path, job=None):
    """(name, text) of each step of a workflow, or of one job. A step has
    `- name: NAME` and the lines below it, to the next step or job."""
    out = []
    in_job = job is None
    name = None
    body = []
    for line in path.read_text().splitlines():
        m = re.match(r"^  ([\w-]+):\s*$", line)
        if m:
            if name and in_job:
                out.append((name, "\n".join(body)))
            name, body = None, []
            in_job = job is None or m.group(1) == job
            continue
        m = re.match(r"^      - name: (.+?)\s*$", line)
        if m:
            if name and in_job:
                out.append((name, "\n".join(body)))
            name, body = m.group(1).strip("'\""), []
            continue
        if name:
            body.append(line)
    if name and in_job:
        out.append((name, "\n".join(body)))
    return out


def with_scripts(text, seen=None):
    """TEXT and the text of each script of scripts/ that it runs."""
    seen = set() if seen is None else seen
    parts = [text]
    for m in re.finditer(r"scripts/[\w./-]+\.sh", text):
        rel = m.group(0)
        if rel in seen or not (ROOT / rel).is_file():
            continue
        seen.add(rel)
        parts.append(with_scripts((ROOT / rel).read_text(), seen))
    return "\n".join(parts)


def called(text, paths):
    """The paths of PATHS that TEXT calls: the first word after a call of
    the binary that names a command, then the next word that names a
    subcommand of it."""
    tops = {p.split()[0] for p in paths}
    found = set()
    for m in CALL.finditer(text):
        words = [w.strip("\"'") for w in m.group(1).split()]
        words = [w for w in words if not w.startswith("-")]
        top = next((w for w in words if w in tops), None)
        if top is None:
            continue
        rest = words[words.index(top) + 1 :]
        sub = next((w for w in rest if f"{top} {w}" in paths), None)
        if sub:
            found.add(f"{top} {sub}")
        elif top in paths:
            found.add(top)
    return found


ERRORS = ROOT / "scripts/lab-errors.sh"


def error_rows():
    """The word lists of the calls of the rows of scripts/lab-errors.sh (the
    case errors of the lane runs it by a path beside itself). A row calls
    podnet through a helper: `run NAME LIMIT ARGS...`, or
    `pod_row NAME CAUSE FLAGS...` for pod."""
    out = []
    for line in ERRORS.read_text().splitlines():
        m = re.match(r"^run \S+ \d+ (.*)$", line) or re.match(r'^pod_row \S+ "[^"]*" (.*)$', line)
        if m:
            words = [w.strip("\"'") for w in m.group(1).split()]
            out.append(["pod", *words] if line.startswith("pod_row") else words)
    return out


def error_calls(paths):
    """The paths that the rows of scripts/lab-errors.sh call."""
    tops = {p.split()[0] for p in paths}
    found = set()
    for words in error_rows():
        plain = [w for w in words if not w.startswith("-")]
        if not plain or plain[0] not in tops:
            continue
        two = " ".join(plain[:2])
        found.add(two if two in paths else plain[0])
    return found


def lane_cases():
    """The cases of the lane, and the text of each one."""
    text = LANE.read_text()
    names = []
    for var in ("CASES", "NET_CASES", "PROXY_CASES", "WAYOUT_CASES", "WAYWS_CASES"):
        m = re.search(rf'^{var}="([^"]*)"', text, re.M)
        names += m.group(1).split() if m else []
    return list(dict.fromkeys(names)), text


# The commands that each case of the lane runs. A case of the lane is a
# branch of shell code with helpers; the map says which command it runs,
# and --check checks that the lane has each case of the map, and calls
# each command of it.
LANE_MAP = {
    "version": [],
    "man": ["man"],
    # scripts/lab-errors.sh (T-452): the error rows.
    "errors": ["bench", "connect", "pod", "tunnel", "warp proxy"],
    "local": [
        "completions",
        "components",
        "status",
        "ports",
        "qr",
        "bug-report",
        "net addresses",
        "link carriers",
        "autostart list",
        "agent status",
        "net demo",
        "net passwd-shim",
        "net ts-shims",
        "net socksify",
    ],
    "doctor": ["doctor"],
    "shim-forward": ["net shim"],
    "link-unix": ["link status", "link listen", "link dial"],
    "link-control": [
        "link status",
        "link listen",
        "wait",
        "link connect",
        "forward",
        "link mesh",
        "link reconnect",
        "link rotate",
        "link close",
    ],
    "pod-seccomp": ["pod"],
    "pod-netns": ["pod"],
    "net-proxy-unix": ["net proxy"],
    "net-switch": ["net switch", "expose", "net proxy"],
    "wg-serve": ["wg serve", "forward"],
    "net-curl": [],
    "net-bench": ["bench"],
    "net-exit": ["exit"],
    "net-pod-auto": ["pod"],
    "net-pod-seccomp": ["pod"],
    "net-pod-netns": ["pod"],
    "way-tunnel": ["demo"],
    "way-relay": ["relay connect"],
    "way-relay-refused": ["relay connect"],
    "proxy-direct": [],
    "proxy-port": [],
    "proxy-pod-auto": ["pod"],
    "proxy-pod-seccomp": ["pod"],
    "proxy-pod-env": ["pod"],
    "proxy-doctor": ["doctor"],
}


def lane_calls(paths):
    """For each case of the lane, the paths that it calls (LANE_MAP). A
    fault of the map is an error."""
    names, text = lane_cases()
    if sorted(names) != sorted(LANE_MAP):
        raise SystemExit(f"gate-scenarios: the cases of the lane are {names}; LANE_MAP has {sorted(LANE_MAP)}")
    calls = called(text, paths) | error_calls(paths)
    for case, found in LANE_MAP.items():
        for p in found:
            if p not in calls:
                raise SystemExit(f"gate-scenarios: LANE_MAP says that {case} runs podnet {p}; the lane does not call it")
    return {case: set(found) for case, found in LANE_MAP.items()}


# The commands that a step runs through another program (a unit of
# systemd): the step, the command, and the text that the step must hold.
VIA = [
    ("Autostart with a systemd user unit", "autostart run", "grep -q ' autostart run '"),
]


def derive(paths):
    """Each path and its scenarios."""
    table = {p: [] for p in paths}
    live = steps(LIVE)
    for name, text in live:
        found = called(with_scripts(text), paths)
        for step, p, proof in VIA:
            if step == name and proof in text:
                found.add(p)
        for p in sorted(found):
            table[p].append(f"live:{name}")
    names = [n for n, _ in live]
    for step, p, proof in VIA:
        if step not in names:
            raise SystemExit(f"gate-scenarios: VIA names the step {step!r}, which live.yml does not have")
    for name, text in steps(CI, "static"):
        for p in sorted(called(with_scripts(text), paths)):
            table[p].append(f"ci:{name}")
    for case, found in lane_calls(paths).items():
        for p in sorted(found):
            table[p].append(f"lane:{case}")
    return table


def render(table):
    lines = [HEADER]
    for p, scenarios in table.items():
        lines.append(f"{p} | {'; '.join(scenarios)}".rstrip())
    return "\n".join(lines) + "\n"


def notes(lane_file):
    rows = [line.split(None, 2) for line in Path(lane_file).read_text().splitlines()]
    passed = {r[1] for r in rows if len(r) >= 3 and r[2].startswith("pass")}
    table = {}
    for line in TABLE.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            p, _, rest = line.partition("|")
            table[p.strip()] = [s.strip() for s in rest.split(";") if s.strip()]
    tested, experimental = [], []
    for p, scenarios in table.items():
        good = [s for s in scenarios if not s.startswith("lane:") or s[5:] in passed]
        (tested if good else experimental).append((p, good))
    print("## The scenarios of each command\n")
    print(
        f"{len(tested)} of {len(table)} commands run in a scenario of the release binary: a step of `live.yml` "
        "(live services, each push), a step of the job `static` of `ci.yml`, or a row of the lane of the gate that "
        "passed above."
    )
    print(
        f"{len(experimental)} commands have no scenario that passed: they are **experimental**. The help of a "
        "command with no scenario says so. Only the tests of `cargo test` (stand-ins) check them.\n"
    )
    print("Experimental: " + ", ".join(f"`podnet {p}`" for p, _ in experimental) + ".\n")
    print("<details><summary>Each command and its scenarios</summary>\n")
    print("| Command | Scenarios |\n| --- | --- |")
    for p, good in tested:
        cell = "; ".join(good).replace("|", "\\|")
        print(f"| `podnet {p}` | {cell} |")
    print("\n</details>")


def main(argv):
    if len(argv) == 1 and argv[0] in ("--check", "--write"):
        paths = commands()
        text = render(derive(paths))
        if argv[0] == "--write":
            TABLE.write_text(text)
            return 0
        if TABLE.read_text() != text:
            print("gate-scenarios: src/cli/scenarios.txt does not agree with the workflows and the lane;")
            print("run: python3 scripts/gate-scenarios.py --write")
            return 1
        return 0
    if len(argv) == 2 and argv[0] == "--notes":
        notes(argv[1])
        return 0
    print(__doc__.strip().splitlines()[2], file=sys.stderr)
    return 64


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
