#!/usr/bin/env python3
"""Deterministic secret redactor and zero-leak auditor for capture rig bundles.

Scrubs tokens, credentials, private keys, JWTs, WARP device secrets, cookies, and
authorization headers across JSON, JSONL, and text capture artifacts, then verifies
that zero secret patterns remain and writes manifest.json + redaction_audit.json.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any

SECRET_HEADER_KEYS = {
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "cf-access-jwt-assertion",
    "cf-access-client-secret",
    "x-auth-key",
    "x-auth-token",
    "x-api-key",
}

SECRET_JSON_KEYS = {
    "token",
    "api_token",
    "access_token",
    "refresh_token",
    "private_key",
    "secret",
    "secret_key",
    "license",
    "auth_client_secret",
    "warp_connector_token",
    "jwt",
    "password",
    "client_secret",
}

TEXT_PATTERNS: list[tuple[str, re.Pattern[str], str]] = [
    (
        "pem_private_key",
        re.compile(
            r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----"
        ),
        "[REDACTED_PRIVATE_KEY]",
    ),
    (
        "jwt",
        re.compile(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b"),
        "[REDACTED_JWT]",
    ),
    (
        "bearer",
        re.compile(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}"),
        "Bearer [REDACTED_TOKEN]",
    ),
    (
        "header_line",
        re.compile(
            r"(?im)^(authorization|proxy-authorization|cookie|set-cookie|cf-access-jwt-assertion|cf-access-client-secret|license)\s*:\s*.*$"
        ),
        r"\1: [REDACTED]",
    ),
    (
        "rust_debug_license",
        re.compile(r'(license:\s*)"[^"]*"'),
        r'\1"[REDACTED]"',
    ),
    (
        "sslkeylog",
        re.compile(
            r"(?m)^(CLIENT_RANDOM|CLIENT_HANDSHAKE_TRAFFIC_SECRET|SERVER_HANDSHAKE_TRAFFIC_SECRET|CLIENT_TRAFFIC_SECRET_0|SERVER_TRAFFIC_SECRET_0|EXPORTER_SECRET)\s+[0-9a-fA-F]+\s+[0-9a-fA-F]+$"
        ),
        r"\1 [REDACTED_KEYLOG]",
    ),
    (
        "json_secret_field",
        re.compile(
            r'(?i)("(?:token|api_token|access_token|refresh_token|private_key|secret|secret_key|license|auth_client_secret|password|jwt)"\s*:\s*)"[^"]*"'
        ),
        r'\1"[REDACTED]"',
    ),
]


def redact_text(text: str) -> tuple[str, int]:
    count = 0
    out = text
    for _name, pat, repl in TEXT_PATTERNS:
        out, n = pat.subn(repl, out)
        count += n
    return out, count


def redact_obj(obj: Any) -> tuple[Any, int]:
    if isinstance(obj, dict):
        total = 0
        out = {}
        for k, v in obj.items():
            kl = k.lower()
            if kl in SECRET_HEADER_KEYS or kl in SECRET_JSON_KEYS:
                if isinstance(v, str) and v and v != "[REDACTED]":
                    out[k] = "[REDACTED]"
                    total += 1
                else:
                    out[k] = v
            else:
                rv, n = redact_obj(v)
                out[k] = rv
                total += n
        return out, total
    if isinstance(obj, list):
        total = 0
        out_list = []
        for item in obj:
            ri, n = redact_obj(item)
            out_list.append(ri)
            total += n
        return out_list, total
    if isinstance(obj, str):
        return redact_text(obj)
    return obj, 0


def redact_file(src: Path, dst: Path) -> int:
    raw = src.read_text(encoding="utf-8", errors="replace")
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.suffix == ".json":
        try:
            obj = json.loads(raw)
            redacted, count = redact_obj(obj)
            dst.write_text(json.dumps(redacted, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            return count
        except json.JSONDecodeError:
            pass
    if src.suffix == ".jsonl":
        total = 0
        lines_out = []
        for line in raw.splitlines():
            if not line.strip():
                continue
            try:
                obj = json.loads(line)
                redacted, n = redact_obj(obj)
                total += n
                lines_out.append(json.dumps(redacted, sort_keys=True))
            except json.JSONDecodeError:
                rline, n = redact_text(line)
                total += n
                lines_out.append(rline)
        dst.write_text("\n".join(lines_out) + ("\n" if lines_out else ""), encoding="utf-8")
        return total
    out, count = redact_text(raw)
    dst.write_text(out, encoding="utf-8")
    return count


def audit_bundle(bundle_dir: Path) -> list[dict[str, Any]]:
    leaks = []
    for path in sorted(bundle_dir.rglob("*")):
        if not path.is_file() or path.name in ("manifest.json", "redaction_audit.json"):
            continue
        if path.suffix in (".pcap", ".pcapng", ".bin"):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for name, pat, _repl in TEXT_PATTERNS:
            for m in pat.finditer(text):
                matched = m.group(0)
                if "[REDACTED" in matched:
                    continue
                leaks.append(
                    {
                        "file": str(path.relative_to(bundle_dir)),
                        "pattern": name,
                        "offset": m.start(),
                    }
                )
    return leaks


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description="Redact secrets and audit a capture rig bundle")
    ap.add_argument("--in-dir", required=True, type=Path)
    ap.add_argument("--out-dir", required=True, type=Path)
    args = ap.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)

    redactions = 0
    files_manifest = []
    for src in sorted(args.in_dir.rglob("*")):
        if not src.is_file():
            continue
        rel = src.relative_to(args.in_dir)
        # Never publish raw keylogs, CA private keys, or raw pcaps into redacted bundle.
        if src.name in ("tls_keylog.txt", "ca.key") or src.suffix in (".key", ".pcap", ".pcapng"):
            continue
        dst = args.out_dir / rel
        redactions += redact_file(src, dst)
        files_manifest.append({"path": str(rel), "bytes": dst.stat().st_size, "sha256": sha256_file(dst)})

    leaks = audit_bundle(args.out_dir)
    audit = {"redactions_applied": redactions, "leaks_found": len(leaks), "leaks": leaks}
    (args.out_dir / "redaction_audit.json").write_text(
        json.dumps(audit, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    manifest = {"schema": 1, "files": files_manifest, "redactions_applied": redactions, "clean": len(leaks) == 0}
    (args.out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    if leaks:
        print(f"redaction audit failed: {len(leaks)} potential leaks", flush=True)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
