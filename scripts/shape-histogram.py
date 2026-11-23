#!/usr/bin/env python3
"""Write the recorded target histogram of the shaped record bodies.

  python3 scripts/shape-histogram.py --profiles DIR [--date YYYY-MM-DD]
  python3 scripts/shape-histogram.py --profiles DIR --check

The target is computed from the four profile files of pod-shape, not sampled
from the shaper and not from `Shape::of_profile`. For each profile the
payloads of the workload below are mapped through the profile's declared
bucket weights and the record layer's `body_for`: the chosen bucket when it
holds the payload, else the next declared bucket, else the next multiple of
2048. The fractions are the target of the committed fixture
`tests/data/shape/histogram.toml`, which the `link_shape` test compares with
the shaper's samples.

`--check` exits 1 when the generated text differs from the fixture. It reads
the profile directory because podnet pins podcrates as a git dependency and
does not vendor its profile files.
"""

import argparse
import datetime
import sys
import tomllib
from pathlib import Path

# The bucket set of the record layer (PAD-4). `body_for` falls back to the
# next multiple of the largest bucket, 2048.
BUCKETS = [0, 128, 512, 1024, 2048]

# The order of `pod_shape::NAMES`.
NAMES = ["page-load", "video-call", "terminal", "file-transfer"]

# One session is eight records for a profile that pads only its first
# records. Resetting there keeps every sampled record a padded one.
WORKLOADS = {
    "page-load": ([1, 100, 700, 1500, 3000], 8),
    "video-call": ([1, 100, 700, 1500, 3000], 0),
    "terminal": ([1, 100, 700], 0),
    "file-transfer": ([1, 100, 700, 1500, 3000], 0),
}

SAMPLES = 2000
COVER_VERSION = "1"


def body_for(buckets, payload):
    """The body that holds `payload`, as the record layer maps it."""
    sizes = [bucket["size"] for bucket in buckets]
    for size in sizes:
        if size >= payload:
            return size
    largest = BUCKETS[-1]
    return ((payload + largest - 1) // largest) * largest


def target(profile, payloads):
    """The fraction of each body size over one cycle of `payloads`."""
    buckets = profile["buckets"]
    total = sum(bucket["weight"] for bucket in buckets)
    mass = {}
    for payload in payloads:
        for bucket in buckets:
            size = body_for(buckets, payload) if bucket["size"] < payload else bucket["size"]
            mass[size] = mass.get(size, 0.0) + bucket["weight"] / total / len(payloads)
    return sorted(mass.items())


def fraction(value):
    text = f"{value:.6f}".rstrip("0")
    return text + "0" if text.endswith(".") else text


def render(profiles_dir, date):
    out = [
        "# An independently declared target for the shaped body sizes of each profile.",
        "#",
        "# The target is not sampled from the shaper and not Shape::of_profile: it is",
        "# computed from the profile's declared bucket weights and the body_for",
        "# mapping (the next declared bucket at or above the payload, else the next",
        "# multiple of 2048) over the workload below, by a separate script.",
        "# produced_by: python3 scripts/shape-histogram.py --profiles DIR",
        f"# produced_on: {date}",
        f"# cover_version: {COVER_VERSION}",
        f'date = "{date}"',
        "",
    ]
    for name in NAMES:
        path = profiles_dir / f"{name}.toml"
        with path.open("rb") as handle:
            profile = tomllib.load(handle)
        if profile["name"] != name:
            raise SystemExit(f"{path}: names {profile['name']!r}, expected {name!r}")
        payloads, session = WORKLOADS[name]
        out += [
            "[[profile]]",
            f'name = "{name}"',
            f"samples = {SAMPLES}",
            f"session = {session}",
            f"payloads = {payloads}",
            "",
        ]
        for size, value in target(profile, payloads):
            out += [
                "[[profile.target]]",
                f"size = {size}",
                f"fraction = {fraction(value)}",
                "",
            ]
    return "\n".join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profiles", required=True, type=Path)
    parser.add_argument("--date", default=datetime.date.today().isoformat())
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = render(args.profiles, args.date)
    if args.check:
        fixture = Path(__file__).resolve().parent.parent / "tests/data/shape/histogram.toml"
        if fixture.read_text() != text:
            print(f"{fixture}: the generated histogram differs", file=sys.stderr)
            return 1
        print(f"{fixture}: matches the generated histogram")
        return 0
    print(text, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
