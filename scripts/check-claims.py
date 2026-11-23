#!/usr/bin/env python3
"""Check the claims of a document (podnet#501, CLAIM-1 to CLAIM-4).

    python3 -I scripts/check-claims.py [--self-test] [FILE ...]

A claim is a measurement or a comparative outside a heading, a table row and
a code span. It needs four things:

* one of the four labels: `measured here`, `read from source`, `reported by
  a third party` or `inferred` (CLAIM-1);
* a condition: host, version, input or the number of runs (CLAIM-2), in the
  two lines before or after the claim;
* a number and an ISO date for a comparative (CLAIM-4).

A claim with no condition is removed, not softened (CLAIM-3). This check
fails it, so the writer must remove it or state the condition.

The default files are the documents of this work: docs/harness.md,
docs/exit-codes.md and docs/host-runtime.md.

Exit 0: each claim passes. Exit 1: a finding, one line for each. Exit 2: a
usage error.
"""

import datetime
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT = ["docs/harness.md", "docs/exit-codes.md", "docs/host-runtime.md"]

LABELS = [
    "measured here",
    "read from source",
    "reported by a third party",
    "inferred",
]
COMPARATIVES = ["faster", "more secure", "robust", "better"]
CONDITIONS = [
    "host",
    "version",
    "input",
    "run",
    "runs",
    "measured",
    "fixture",
    "platform",
    "build",
    "sample",
]
UNITS = ["s", "ms", "us", "min", "minutes", "%", "MiB", "MB", "GiB", "GB", "runs"]
DATE = re.compile(r"\b\d{4}-\d{2}-\d{2}\b")
NUMBER = re.compile(r"\d")


def real_date(token):
    try:
        datetime.date.fromisoformat(token)
    except ValueError:
        return False
    return True


def dates(text):
    return [token for token in DATE.findall(text) if real_date(token)]


def number_without_date(text):
    """A digit that is not part of an ISO date."""
    return NUMBER.search(DATE.sub(" ", text)) is not None


def strip_code(line):
    out = []
    inside = False
    for char in line:
        if char == "`":
            inside = not inside
        elif not inside:
            out.append(char)
    return "".join(out)


def holds_word(text, word):
    return re.search(rf"(?<![0-9A-Za-z]){re.escape(word)}(?![0-9A-Za-z])", text) is not None


def is_measurement(line):
    tokens = line.split()
    for index, token in enumerate(tokens):
        token = token.strip(".,;:()[]{}<>\"'")
        if token.endswith("%") and _float(token[:-1]):
            return True
        for unit in UNITS:
            if token.endswith(unit) and _float(token[: -len(unit)]):
                return True
        if _float(token) and index + 1 < len(tokens):
            if tokens[index + 1].strip(".,;:()[]{}<>\"'") in UNITS:
                return True
    return False


def _float(text):
    try:
        float(text)
    except ValueError:
        return False
    return True


def has_conditions(text):
    lower = text.lower()
    tokens = re.split(r"[^0-9a-z]+", lower)
    return any(condition in tokens for condition in CONDITIONS)


def has_label(text):
    lower = " ".join(text.lower().split())
    return any(label in lower for label in LABELS)


def scan(text):
    """Each finding: (line, rule, text)."""
    lines = text.splitlines()
    clean = []
    fenced = False
    for line in lines:
        if line.lstrip().startswith("```"):
            fenced = not fenced
            clean.append("")
        elif fenced:
            clean.append("")
        else:
            clean.append(strip_code(line))
    findings = []
    for index, line in enumerate(clean):
        trimmed = line.lstrip()
        if trimmed.startswith("#") or trimmed.startswith("|"):
            continue
        lower = line.lower()
        comparative = next(
            (word for word in COMPARATIVES if holds_word(lower, word)), None
        )
        measurement = is_measurement(line)
        if comparative is None and not measurement:
            continue
        start = max(0, index - 2)
        end = min(len(clean), index + 3)
        window = " ".join(clean[start:end])
        number = number_without_date(window)
        date = bool(dates(window))
        conditions = has_conditions(window)
        label = has_label(window)
        rule = None
        if comparative is not None and (not number or not date):
            rule = "bare-comparative"
        elif not conditions:
            rule = "no-conditions"
        elif not label:
            rule = "no-label"
        if rule:
            findings.append((index + 1, rule, line.strip()))
    return findings


def report(path, findings):
    for line, rule, text in findings:
        print(f"{path}:{line}: {rule}: {text}")
    return 1 if findings else 0


def self_test():
    cases = [
        ("The tunnel is faster.", "bare-comparative"),
        ("The harness takes 10 minutes.", "no-conditions"),
        (
            "The run took 42 ms on host lab-a, version 1.0, input f, 1 run.",
            "no-label",
        ),
    ]
    failed = 0
    for text, rule in cases:
        findings = scan(text)
        if len(findings) != 1 or findings[0][1] != rule:
            print(f"self-test: {rule}: {text!r}: {findings!r}")
            failed += 1
    control = (
        "The tunnel is faster: 12 MiB/s on host lab-a, version 1.0, input f, "
        "1 run, on 2026-11-20 (measured here)."
    )
    if scan(control):
        print(f"self-test: the control fails: {scan(control)!r}")
        failed += 1
    quoted = "The labels `faster` and `better` are quotations.\n\n```\nA is faster.\n```\n"
    if scan(quoted):
        print(f"self-test: a quotation fails: {scan(quoted)!r}")
        failed += 1
    if failed:
        print(f"self-test: {failed} case(s) failed")
        return 1
    print("self-test: 5 cases pass")
    return 0


def main(argv):
    args = list(argv)
    if "--self-test" in args:
        return self_test()
    if "-h" in args or "--help" in args:
        print(__doc__.strip())
        return 0
    files = args or DEFAULT
    status = 0
    for name in files:
        path = Path(name)
        if not path.is_absolute():
            path = ROOT / path
        try:
            text = path.read_text()
        except OSError as error:
            print(f"check-claims: {name}: {error}")
            return 2
        status |= report(name, scan(text))
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
