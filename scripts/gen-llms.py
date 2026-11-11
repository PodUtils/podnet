#!/usr/bin/env python3
"""Write llms.txt and llms-full.txt of the repository root (T-222).

  python3 -I scripts/gen-llms.py [--check] [--out DIR] BINARY

BINARY is a podnet binary. Both files come from that binary and from the
documents of DOCS:

* llms.txt: an index. It has the summary, the commands of `podnet man --json`
  (one line each) and the documents (one link each).
* llms-full.txt: the manual of `podnet man --no-pager`, then the text of each
  document.

--out DIR writes the files to DIR (default: the repository root), for the
release.

--check writes nothing. It exits 1 when a file differs from what the script
would write, and it prints the name of the file. Without --check the script
writes the files. Run it again after a change of the command tree, of the
manual or of a document. The release workflow publishes both files.

Exit 0: done, or the files agree. Exit 1: a file is out of date. Exit 2: the
binary or a document cannot be read.
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE = "https://github.com/PodUtils/podnet/blob/main/"

# (path, what the document gives)
DOCS = [
    ("README.md", "Install, quick start, and the use of each command."),
    ("docs/protocol.md", "The edge protocol and the relay protocol."),
    ("docs/virtual-network.md", "The virtual network: design and use."),
    ("docs/target-environment.md", "The hosts that podnet is for, and their limits."),
]


def run(binary, *args):
    out = subprocess.run(
        [binary, *args],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={"PATH": "/usr/bin:/bin", "PAGER": "cat", "HOME": "/nonexistent"},
        check=False,
    )
    if out.returncode != 0:
        raise OSError(f"{binary} {' '.join(args)}: exit {out.returncode}")
    return out.stdout.decode("utf-8")


def short_version(manual):
    """The version without the commit: the commit changes with each build."""
    return manual["version"].split(" (")[0]


def index(manual):
    about = manual["about"].rstrip(".")
    lines = [
        "# podnet",
        "",
        f"> {about}. Version {short_version(manual)}.",
        "",
        *manual["description"],
        "",
        "Run `podnet man` for the manual, `podnet man --json` for the manual as JSON,",
        "and `podnet COMMAND --help --json` for one command as JSON.",
        "llms-full.txt has the manual and the documents in one file.",
        "",
        "## Commands",
        "",
    ]
    for page in manual["commands"]:
        if page["path"]:
            about = " ".join(page["about"].split())
            lines.append(f"- `podnet {' '.join(page['path'])}`: {about}")
    lines += ["", "## Documents", ""]
    for path, meaning in DOCS:
        lines.append(f"- [{path}]({BASE}{path}): {meaning}")
    lines += ["", "## Manual", "", "- [llms-full.txt](llms-full.txt): the manual and the documents.", ""]
    return "\n".join(lines)


def full(text):
    parts = [
        "# podnet: manual and documents",
        "",
        "The manual is the output of `podnet man --no-pager`.",
        "",
        "```text",
        text.rstrip("\n"),
        "```",
    ]
    for path, _ in DOCS:
        body = (ROOT / path).read_text(encoding="utf-8").rstrip("\n")
        parts += ["", f"<!-- document: {path} -->", "", body]
    return "\n".join(parts) + "\n"


def main(argv):
    argv = list(argv)
    out_dir = ROOT
    if "--out" in argv:
        at = argv.index("--out")
        if at + 1 >= len(argv):
            print(__doc__, file=sys.stderr)
            return 2
        out_dir = Path(argv[at + 1])
        del argv[at : at + 2]
    check = "--check" in argv
    rest = [a for a in argv if a != "--check"]
    if len(rest) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        manual = json.loads(run(rest[0], "man", "--json"))
        want = {
            "llms.txt": index(manual),
            "llms-full.txt": full(
                run(rest[0], "man", "--no-pager").replace(
                    manual["version"], short_version(manual)
                )
            ),
        }
    except OSError as err:
        print(f"gen-llms: {err}", file=sys.stderr)
        return 2
    stale = False
    for name, text in want.items():
        path = out_dir / name
        if check:
            have = path.read_text(encoding="utf-8") if path.exists() else None
            if have != text:
                print(f"gen-llms: {name} is out of date: run scripts/gen-llms.py")
                stale = True
        else:
            path.write_text(text, encoding="utf-8")
    return 1 if stale else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
