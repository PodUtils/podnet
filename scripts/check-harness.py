#!/usr/bin/env python3
"""Check the table of `podnet harness` (podnet#501).

    python3 -I scripts/check-harness.py TABLE.json
    python3 -I scripts/check-harness.py --self-test

The table must have a date, 33 rows, and a score from each of the three
classifiers or a named skip. Each scored row must be distinguishable from
the other scored rows. A padded road must not score above its unpadded
counterpart, and at least one road must change under padding. The benign
false-positive rate must be at or under its bound, and the run must finish
at or under its budget.

Exit 0: the table passes. Exit 1: a fault, one line for each. Exit 2: the
file cannot be read or is not JSON.
"""

import json
import sys

ROWS = 33
CLASSIFIERS = ("field", "flow", "prefix")


def check(doc):
    faults = []
    if not doc.get("date"):
        faults.append("the table has no date")
    rows = doc.get("rows")
    if not isinstance(rows, list):
        faults.append("the table has no rows array")
        rows = []
    elif len(rows) != ROWS:
        faults.append(f"the table has {len(rows)} rows, not {ROWS}")
    seen = {}
    field = {}
    for row in rows:
        name = f"{row.get('road')} / {row.get('profile')}"
        if row.get("state") == "skipped":
            if not row.get("reason"):
                faults.append(f"{name} is skipped with no reason")
            continue
        if row.get("state") != "scored":
            faults.append(f"{name} is neither scored nor skipped")
            continue
        scores = row.get("scores")
        if not isinstance(scores, list) or len(scores) != len(CLASSIFIERS):
            faults.append(f"{name} does not have the three classifiers")
            continue
        key = tuple((score.get("classifier"), score.get("score")) for score in scores)
        if key in seen:
            faults.append(f"{name} and {seen[key]} have identical scores")
        else:
            seen[key] = name
        by = {score.get("classifier"): score.get("score") for score in scores}
        for classifier in CLASSIFIERS:
            if classifier not in by:
                faults.append(f"{name} has no {classifier} score")
        if "field" in by:
            field[(row.get("road"), row.get("profile"))] = by["field"]
    changed = 0
    for (road, profile), direct in field.items():
        if profile != "direct":
            continue
        padded = field.get((road, "proxy"))
        if padded is None:
            continue
        if direct < padded:
            faults.append(f"{road}: unpadded {direct} is below padded {padded}")
        if direct > padded:
            changed += 1
    if changed == 0:
        faults.append("no road changed under padding")
    benign = doc.get("benign") or {}
    rate = benign.get("rate")
    limit = doc.get("benign_limit")
    if not isinstance(rate, (int, float)):
        faults.append("the table has no benign rate")
    elif not isinstance(limit, (int, float)):
        faults.append("the table has no benign limit")
    elif rate > limit:
        faults.append(f"the benign rate {rate} is over the limit {limit}")
    budget = doc.get("budget_secs")
    elapsed = doc.get("elapsed_secs")
    if not isinstance(elapsed, (int, float)):
        faults.append("the table has no elapsed time")
    elif not isinstance(budget, (int, float)):
        faults.append("the table has no budget")
    elif elapsed > budget:
        faults.append(f"the run took {elapsed} s, over the budget {budget} s")
    return faults


def good():
    rows = []
    for index in range(ROWS):
        road = f"road{index // 3}"
        profile = ("direct", "proxy", "restricted")[index % 3]
        rows.append(
            {
                "road": road,
                "profile": profile,
                "state": "scored",
                "scores": [
                    {"classifier": "field", "score": 1.0 if profile != "proxy" else 0.7},
                    {"classifier": "flow", "score": index / 100.0},
                    {"classifier": "prefix", "score": 1.0},
                ],
            }
        )
    return {
        "date": "2026-11-21",
        "rows": rows,
        "benign": {"samples": 32, "flagged": 0, "threshold": 0.5, "rate": 0.0},
        "benign_limit": 0.1,
        "budget_secs": 600,
        "elapsed_secs": 1.0,
    }


def self_test():
    failed = 0
    if check(good()):
        print(f"self-test: the control fails: {check(good())!r}")
        failed += 1
    cases = [
        ("the date", lambda d: d.pop("date")),
        ("a duplicate row", lambda d: d["rows"][1].update(d["rows"][0])),
        (
            "the padding",
            lambda d: d["rows"][0].update(
                {
                    "scores": [
                        {"classifier": "field", "score": 0.5},
                        {"classifier": "flow", "score": 1.0},
                        {"classifier": "prefix", "score": 1.0},
                    ]
                }
            ),
        ),
        (
            "the benign rate",
            lambda d: d["benign"].update({"rate": 0.5}),
        ),
        (
            "the budget",
            lambda d: d.update({"elapsed_secs": 601.0}),
        ),
    ]
    for name, plant in cases:
        doc = good()
        plant(doc)
        if not check(doc):
            print(f"self-test: {name}: the fault passes")
            failed += 1
    if failed:
        print(f"self-test: {failed} case(s) failed")
        return 1
    print(f"self-test: {len(cases) + 1} cases pass")
    return 0


def main(argv):
    if "--self-test" in argv:
        return self_test()
    if len(argv) != 1 or argv[0].startswith("-"):
        print(__doc__.strip())
        return 2
    path = argv[0]
    try:
        with open(path, encoding="utf-8") as handle:
            doc = json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        print(f"check-harness: {path}: {error}")
        return 2
    faults = check(doc)
    for fault in faults:
        print(f"check-harness: {fault}")
    if faults:
        return 1
    rows = doc["rows"]
    scored = sum(1 for row in rows if row.get("state") == "scored")
    skipped = len(rows) - scored
    print(
        "harness: {date}, {scored} scored, {skipped} skipped, "
        "benign rate {rate:.3} over {samples} samples, elapsed {elapsed:.3} s".format(
            date=doc["date"],
            scored=scored,
            skipped=skipped,
            rate=doc["benign"]["rate"],
            samples=doc["benign"]["samples"],
            elapsed=doc["elapsed_secs"],
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
