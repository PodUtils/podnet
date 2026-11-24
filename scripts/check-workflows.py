#!/usr/bin/env python3
"""Check the rules of the workflows in a directory.

  python3 scripts/check-workflows.py [DIR]

DIR is .github/workflows when it is not given. The rules:

* Each `uses:` of an action names a commit (40 hex digits), with the version
  in a comment. A local action (`./...`) and a `docker://` image with a
  digest are accepted.
* The top level of each workflow has `permissions:` with `contents: read`
  and no other right, or with no right (`{}`).
* Each job has `timeout-minutes`.
* A job named in READ_ONLY has no right to write.
* A step with `push-to-registry: true` (actions/attest) comes after a
  `docker login` (or docker/login-action) of its job, with no
  `docker logout` between them: the action reads the credentials of the
  registry from the docker config (the image job of the release of
  v0.4.0-beta.18 failed with "No credentials found for registry ghcr.io").

Exit 1 with one line for each fault. Exit 2 when a file cannot be read.
"""

import re
import sys
from pathlib import Path

import yaml

# The jobs that run the code of the repository or the built binary.
READ_ONLY = {
    "test", "restricted-host", "live", "static", "msrv", "plant", "workflows", "all", "secrets", "build",
    "record", "lane",
}
SHA = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


def writes(permissions) -> list[str]:
    if permissions is None:
        return []
    if isinstance(permissions, str):
        return [] if permissions == "read-all" else [permissions]
    return [k for k, v in permissions.items() if v == "write"]


def registry_faults(path: Path, name: str, job: dict) -> list[str]:
    faults = []
    logged_in = False
    for i, step in enumerate(job.get("steps") or [], 1):
        if not isinstance(step, dict):
            continue
        if str(step.get("uses", "")).startswith("docker/login-action@"):
            logged_in = True
        run = str(step.get("run", ""))
        login, logout = run.rfind("docker login"), run.rfind("docker logout")
        if login > logout:
            logged_in = True
        elif logout > login:
            logged_in = False
        push = (step.get("with") or {}).get("push-to-registry")
        if push in (True, "true") and not logged_in:
            what = step.get("name") or step.get("uses") or f"step {i}"
            faults.append(f"{path}: job {name}, {what}: push-to-registry with no docker login before it")
    return faults


def check(path: Path) -> list[str]:
    faults = []
    text = path.read_text(encoding="utf-8")
    for n, line in enumerate(text.splitlines(), 1):
        m = re.match(r"^\s*(?:-\s+)?uses:\s*(\S+)(.*)$", line)
        if not m:
            continue
        ref, rest = m.group(1), m.group(2)
        if ref.startswith("./") or re.match(r"^docker://\S+@sha256:[0-9a-f]{64}$", ref):
            continue
        if not SHA.match(ref):
            faults.append(f"{path}:{n}: {ref} is not pinned to a commit")
        elif "#" not in rest:
            faults.append(f"{path}:{n}: {ref} has no version comment")
    doc = yaml.safe_load(text)
    top = doc.get("permissions")
    if not isinstance(top, dict) or top not in ({"contents": "read"}, {}):
        faults.append(f"{path}: the top-level permissions are not exactly contents: read, or {{}}")
    for name, job in (doc.get("jobs") or {}).items():
        if "uses" in job:
            continue
        if "timeout-minutes" not in job:
            faults.append(f"{path}: job {name} has no timeout-minutes")
        if name in READ_ONLY and writes(job.get("permissions")):
            faults.append(f"{path}: job {name} may write: {', '.join(writes(job.get('permissions')))}")
        faults += registry_faults(path, name, job)
    return faults


def main(argv: list[str]) -> int:
    root = Path(argv[0]) if argv else Path(".github/workflows")
    files = sorted(list(root.glob("*.yml")) + list(root.glob("*.yaml")))
    if not files:
        print(f"check-workflows: no workflow in {root}", file=sys.stderr)
        return 2
    faults = []
    for f in files:
        try:
            faults += check(f)
        except (OSError, yaml.YAMLError) as e:
            print(f"check-workflows: {f}: {e}", file=sys.stderr)
            return 2
    for fault in faults:
        print(fault)
    print(f"check-workflows: {len(files)} files, {len(faults)} faults")
    return 1 if faults else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
