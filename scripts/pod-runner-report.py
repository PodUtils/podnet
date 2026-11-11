#!/usr/bin/env python3
"""The report of the measure of T-229 (scripts/pod-runner-measure.sh).

  python3 -I scripts/pod-runner-report.py WORK PODNET BACKEND RUNNER CHECKOUT SETUP_NODE

WORK holds out/NAME/ for each payload: exit, output.txt, flow.jsonl (the
flow log of the pod), witness.log (the CONNECT lines of the witness proxy)
and strace.txt (when strace ran). The report goes to stdout, in Markdown.
Each path under WORK prints as `WORK`. Exit 1 when a check fails.
"""

import ipaddress
import json
import pathlib
import platform
import re
import subprocess
import sys
from collections import Counter

PAYLOADS = [
    ("runner-help", "`config.sh --help` of the runner (.NET)"),
    ("runner-config", "`config.sh --unattended` with a token that is not valid: the registration is refused"),
    ("runner-run", "`run.sh` of a runner that has no configuration"),
    ("node-fetch", "`fetch` of Node.js to the API of GitHub"),
    ("checkout", "`actions/checkout`, run by `node`: a clone of PodUtils/podnet with depth 1"),
    ("setup-node", "`actions/setup-node`, run by `node`: Node.js 20 into a tool cache"),
]

FAKE_V4 = ipaddress.ip_network("198.18.0.0/15")
FAKE_V6 = ipaddress.ip_network("fd63:6672:7300::/96")

SOCKET = re.compile(r"^(?P<pid>\d+)\s+socket\((?P<family>AF_\w+), (?P<type>SOCK_\w+)[^)]*\)\s+=\s+(?P<fd>\d+)")

ADDRESS = re.compile(
    r'^(?P<pid>\d+)\s+(?P<call>connect|bind|sendto|sendmsg)\((?P<fd>\d+), .*?'
    r'(?:sa_family=AF_UNIX, sun_path="(?P<unix>[^"]*)"'
    r'|sa_family=AF_NETLINK'
    r'|sa_family=AF_INET, sin_port=htons\((?P<p4>\d+)\), sin_addr=inet_addr\("(?P<a4>[^"]+)"\)'
    r'|sa_family=AF_INET6, sin6_port=htons\((?P<p6>\d+)\).*?inet_pton\(AF_INET6, "(?P<a6>[^"]+)")'
)


def fake(host: str) -> bool:
    host = host.strip("[]")
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return False
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
        ip = ip.ipv4_mapped
    return ip in FAKE_V4 or ip in FAKE_V6


def target_host(target: str) -> str:
    return target.rsplit(":", 1)[0]


def way(match: re.Match, work: str, kind: str) -> str:
    """The class of one address of strace. `kind` is the type of the socket
    (`SOCK_STREAM`, `SOCK_DGRAM`), or empty when strace did not show it."""
    call = match["call"]
    if match["unix"] is not None:
        path = match["unix"].replace(work, "WORK")
        path = re.sub(r"-\d+-\d+-socket$", "-PID-socket", path)
        return f"{call} unix socket `{path}`"
    port = match["p4"] or match["p6"]
    addr = match["a4"] or match["a6"]
    if port is None:
        return f"{call} netlink (local: the addresses and routes of the host)"
    ip = ipaddress.ip_address(addr)
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
        ip = ip.ipv4_mapped
    if port == "53":
        return f"{call} DNS to {addr}:53 (the pod answers it)"
    if call == "connect" and (port == "0" or kind == "SOCK_DGRAM"):
        where = "a fake address" if fake(addr) else addr
        return f"connect of a UDP socket to {where}, port {port} (an address choice: no packet)"
    if ip.is_loopback:
        return f"{call} loopback {addr}:{port}"
    if fake(addr):
        return f"{call} a fake address of the pod, port {port} (the pod holds it)"
    return f"{call} {addr}:{port} (not a fake address)"


def lines(path: pathlib.Path) -> list[str]:
    return path.read_text(encoding="utf-8", errors="replace").splitlines() if path.exists() else []


def main() -> int:
    if len(sys.argv) != 7:
        print(__doc__.strip(), file=sys.stderr)
        return 64
    work, podnet, backend, runner, checkout, setup_node = sys.argv[1:]
    try:
        version = subprocess.run([podnet, "--version"], capture_output=True, text=True, timeout=30).stdout.strip()
    except OSError:
        version = "unknown"
    out = []
    w = out.append
    failed = []
    w("# podnet pod with the runner of actions/runner and CI payloads (T-229)")
    w("")
    w("The output of `scripts/pod-runner-measure.sh`. Do not edit it by hand: run the script again.")
    w("")
    w(f"* podnet: `{version}`; backend `{backend}`; kernel `{platform.release()}`.")
    w(f"* Payloads: the runner `v{runner}` (the release asset of `actions/runner`), `actions/checkout` "
      f"`{checkout}` and `actions/setup-node` `{setup_node}` (their `dist` bundles, run by `node`).")
    w("* Each payload runs in its own pod: `--default-action reject`, the rules `domain:.github.com`, "
      "`domain:.githubusercontent.com` and `domain:.nodejs.org` to the witness proxy, and `-x` the "
      "witness proxy (`scripts/pod-runner-proxy.py`, HTTP CONNECT on a Unix socket).")
    w("* Three witnesses: the flow log of the pod, the CONNECT lines of the witness proxy, and strace "
      "(each `connect`, `bind`, `sendto` and `sendmsg` with an address).")
    w("")
    w("## Summary")
    w("")
    w("| Payload | Exit | Flows (flow log) | Refused | CONNECT (proxy) | Checks |")
    w("| --- | --- | --- | --- | --- | --- |")
    details = []
    for name, about in PAYLOADS:
        d = pathlib.Path(work) / "out" / name
        code = (d / "exit").read_text().strip() if (d / "exit").exists() else "not run"
        dials = []
        for line in lines(d / "flow.jsonl"):
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("event") == "dial":
                dials.append(event)
        allowed = Counter(e["target"] for e in dials if e.get("verdict") == "allow")
        refused = [e for e in dials if e.get("verdict") == "reject"]
        witness = Counter(l.split(" ", 1)[1] for l in lines(d / "witness.log") if l.startswith("CONNECT "))
        checks = []
        if witness != allowed:
            checks.append("the proxy and the flow log differ")
        fakes = [e["target"] for e in dials if fake(target_host(e["target"]))]
        if fakes:
            checks.append(f"{len(fakes)} flow(s) to a fake address (T-233)")
        if checks:
            failed.append(name)
        verdict = "; ".join(checks) if checks else "pass"
        w(f"| {name} | {code} | {len(dials)} | {len(refused)} | {sum(witness.values())} | {verdict} |")
        ways = Counter()
        kinds = {}
        for line in lines(d / "strace.txt"):
            s = SOCKET.search(line)
            if s:
                kinds[(s["pid"], s["fd"])] = s["type"]
                continue
            m = ADDRESS.search(line)
            if m:
                ways[way(m, work, kinds.get((m["pid"], m["fd"]), ""))] += 1
        details.append((name, about, dials, witness, ways, (d / "strace.txt").exists()))
    w("")
    w("A check passes when each CONNECT of the witness proxy has a flow of the flow log with the verdict "
      "`allow` (the same targets, the same counts), and no flow goes to a fake address of the pod.")
    for name, about, dials, witness, ways, traced in details:
        w("")
        w(f"## {name}")
        w("")
        w(f"{about}.")
        w("")
        w("Flows of the flow log (target, rule, verdict):")
        w("")
        if dials:
            for e in dials:
                rule = str(e.get("rule", "")).replace(work, "WORK")
                rule = re.sub(r"=http://unix:\S+", "=PROXY", rule)
                w(f"* `{e.get('proto')}` `{e.get('target')}`: `{rule}`, `{e.get('verdict')}`")
        else:
            w("* none")
        w("")
        w("Other ways (strace, with the count):")
        w("")
        if not traced:
            w("* strace is not installed: not measured")
        elif ways:
            for text, count in sorted(ways.items()):
                w(f"* {count} x {text}")
        else:
            w("* none")
    print("\n".join(out))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
