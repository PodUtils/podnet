#!/usr/bin/env python3
"""Triple-redundant wire dissector for PCAP + SSLKEYLOGFILE captures.

Layer 1 (primary): tshark with tls.keylog_file -> decrypted HTTP/3, HTTP/2, HTTP/1.1, TLS.
Layer 2 (secondary): tshark without keylog -> TLS/QUIC ClientHello SNI, ALPN, DNS, flows.
Layer 3 (fallback, zero external tools): pure-Python PCAP reader + TLS ClientHello SNI/ALPN.

Writes structured wire_summary.json combining all available layers so the rig
continues to produce evidence even if decryption or tshark is unavailable.
"""

from __future__ import annotations

import argparse
import json
import shutil
import struct
import subprocess
from pathlib import Path
from typing import Any, Optional


def run_tshark(
    pcap: Path,
    keylog: Optional[Path],
    display_filter: str,
    fields: list[str],
    warnings: Optional[list[str]] = None,
) -> list[dict]:
    cmd = ["tshark", "-r", str(pcap), "-Y", display_filter, "-T", "fields", "-E", "separator=\t", "-E", "occurrence=a"]
    if keylog and keylog.is_file() and keylog.stat().st_size > 0:
        cmd.extend(["-o", f"tls.keylog_file:{keylog}"])
    for f in fields:
        cmd.extend(["-e", f])
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        if warnings is not None:
            msg = (proc.stderr or proc.stdout).strip().splitlines()
            warnings.append(f"tshark({display_filter}): rc={proc.returncode} {msg[-1] if msg else ''}")
        return []
    rows = []
    for line in proc.stdout.splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        row = {fields[i]: (parts[i] if i < len(parts) else "") for i in range(len(fields))}
        rows.append(row)
    return rows


def pure_python_pcap_scan(pcap: Path) -> dict[str, Any]:
    """Layer 3 fallback: parse classic libpcap or pcapng enough to count packets and find TLS SNI."""
    data = pcap.read_bytes()
    if len(data) < 24:
        return {"format": "empty", "packets": 0, "sni_hints": []}
    magic = data[:4]
    packets = 0
    sni_hints: set[str] = set()
    if magic in (b"\xd4\xc3\xb2\xa1", b"\xa1\xb2\xc3\xd4"):
        endian = "<" if magic == b"\xd4\xc3\xb2\xa1" else ">"
        pos = 24
        while pos + 16 <= len(data):
            _ts_s, _ts_u, incl_len, _orig_len = struct.unpack(endian + "IIII", data[pos : pos + 16])
            pos += 16
            pkt = data[pos : pos + incl_len]
            pos += incl_len
            packets += 1
            _scan_tls_sni(pkt, sni_hints)
        return {"format": "pcap", "packets": packets, "sni_hints": sorted(sni_hints)}
    if magic == b"\x0a\x0d\x0d\x0a":
        pos = 0
        while pos + 12 <= len(data):
            btype, blen = struct.unpack("<II", data[pos : pos + 8])
            if blen < 12 or pos + blen > len(data):
                break
            if btype == 6:  # Enhanced Packet Block
                packets += 1
                _scan_tls_sni(data[pos + 28 : pos + blen - 4], sni_hints)
            pos += blen
        return {"format": "pcapng", "packets": packets, "sni_hints": sorted(sni_hints)}
    return {"format": "unknown", "packets": 0, "sni_hints": []}


def _scan_tls_sni(pkt: bytes, out: set[str]) -> None:
    idx = pkt.find(b"\x16\x03")
    if idx < 0 or idx + 48 >= len(pkt):
        return
    seg = pkt[idx:]
    # Scan for SNI extension (0x00 0x00 <ext_len> <list_len> 0x00 <name_len> <ascii>)
    pos = 43
    while pos + 9 < len(seg):
        if seg[pos : pos + 2] == b"\x00\x00":
            ext_len = int.from_bytes(seg[pos + 2 : pos + 4], "big")
            list_len = int.from_bytes(seg[pos + 4 : pos + 6], "big")
            name_type = seg[pos + 6]
            name_len = int.from_bytes(seg[pos + 7 : pos + 9], "big")
            if (
                name_type == 0
                and 4 <= name_len <= 128
                and list_len == name_len + 3
                and ext_len == list_len + 2
                and pos + 9 + name_len <= len(seg)
            ):
                cand = seg[pos + 9 : pos + 9 + name_len]
                if all(33 <= b < 127 for b in cand) and b"." in cand:
                    out.add(cand.decode("ascii"))
                    return
        pos += 1


def dissect(pcap: Path, keylog: Optional[Path]) -> dict[str, Any]:
    warnings: list[str] = []
    summary: dict[str, Any] = {
        "pcap": str(pcap),
        "keylog_present": bool(keylog and keylog.is_file() and keylog.stat().st_size > 0),
        "layers_used": [],
        "http3_headers": [],
        "http3_settings": [],
        "http2_headers": [],
        "http2_settings": [],
        "http1_requests": [],
        "tls_handshakes": [],
        "quic_handshakes": [],
        "tshark_warnings": warnings,
        "fallback_scan": {},
    }

    if not pcap.is_file():
        return summary

    summary["fallback_scan"] = pure_python_pcap_scan(pcap)
    summary["layers_used"].append("layer3_python_pcap")

    if not shutil.which("tshark"):
        return summary

    summary["layers_used"].append("layer2_tshark_passive")
    summary["tls_handshakes"] = run_tshark(
        pcap,
        None,
        "tls.handshake.type == 1",
        [
            "frame.number",
            "ip.dst",
            "ipv6.dst",
            "tcp.dstport",
            "udp.dstport",
            "tls.handshake.version",
            "tls.handshake.ciphersuite",
            "tls.handshake.comp_method",
            "tls.handshake.extension.type",
            "tls.handshake.extensions_server_name",
            "tls.handshake.extensions_alpn_str",
            "tls.handshake.extensions_supported_group",
            "tls.handshake.extensions_ec_point_formats",
            "tls.handshake.sig_hash_alg",
        ],
        warnings,
    )
    summary["quic_handshakes"] = run_tshark(
        pcap,
        None,
        "quic.long.packet_type == 0",
        [
            "frame.number",
            "ip.dst",
            "ipv6.dst",
            "udp.dstport",
            "quic.version",
            "tls.handshake.extensions_server_name",
            "tls.handshake.extensions_alpn_str",
            "tls.handshake.ciphersuite",
            "tls.handshake.extension.type",
            "tls.handshake.extensions_supported_group",
            "tls.handshake.sig_hash_alg",
            "tls.quic.parameter.type",
            "tls.quic.parameter.max_idle_timeout",
            "tls.quic.parameter.max_udp_payload_size",
            "tls.quic.parameter.initial_max_data",
            "tls.quic.parameter.initial_max_stream_data_bidi_local",
            "tls.quic.parameter.initial_max_stream_data_bidi_remote",
            "tls.quic.parameter.initial_max_stream_data_uni",
            "tls.quic.parameter.initial_max_streams_bidi",
            "tls.quic.parameter.initial_max_streams_uni",
            "tls.quic.parameter.active_connection_id_limit",
        ],
        warnings,
    )[:50]

    if summary["keylog_present"]:
        summary["layers_used"].append("layer1_tshark_decrypted")
        summary["http3_headers"] = run_tshark(
            pcap,
            keylog,
            "http3.frame_type == 1",
            [
                "frame.number",
                "ip.src",
                "ip.dst",
                "udp.dstport",
                "quic.stream.stream_id",
                "http3.headers.method",
                "http3.headers.authority",
                "http3.headers.scheme",
                "http3.headers.path",
                "http3.headers.status",
                "http3.header.header.name",
                "http3.headers.header.value",
            ],
            warnings,
        )
        summary["http3_settings"] = run_tshark(
            pcap,
            keylog,
            "http3.frame_type == 4",
            [
                "frame.number",
                "ip.src",
                "ip.dst",
                "quic.stream.stream_id",
                "http3.settings.id",
                "http3.settings.value",
                "http3.settings.max_field_section_size",
                "http3.settings.extended_connect",
                "http3.settings.h3_datagram",
            ],
            warnings,
        )
        summary["http2_headers"] = run_tshark(
            pcap,
            keylog,
            "http2.headers",
            [
                "frame.number",
                "ip.dst",
                "tcp.dstport",
                "http2.streamid",
                "http2.header.name",
                "http2.header.value",
            ],
            warnings,
        )
        summary["http2_settings"] = run_tshark(
            pcap,
            keylog,
            "http2.settings",
            [
                "frame.number",
                "ip.dst",
                "tcp.dstport",
                "http2.settings.id",
                "http2.settings.header_table_size",
                "http2.settings.max_concurrent_streams",
                "http2.settings.initial_window_size",
                "http2.settings.max_frame_size",
                "http2.settings.max_header_list_size",
                "http2.settings.extended_connect",
            ],
            warnings,
        )
        summary["http1_requests"] = run_tshark(
            pcap,
            keylog,
            "http.request || http.response",
            [
                "frame.number",
                "ip.dst",
                "http.request.method",
                "http.host",
                "http.request.uri",
                "http.response.code",
            ],
            warnings,
        )

    return summary


def summarize_qlogs(qlog_dir: Optional[Path]) -> list[dict[str, Any]]:
    """Extract structured HTTP/3 frame and transport-parameter events from qlog/sqlog files."""
    if not qlog_dir or not qlog_dir.is_dir():
        return []
    out: list[dict[str, Any]] = []
    for path in sorted(qlog_dir.rglob("*")):
        if not path.is_file():
            continue
        if not (path.name.endswith(".qlog") or path.name.endswith(".sqlog") or path.name.endswith(".json")):
            continue
        events = []
        raw = path.read_text(encoding="utf-8", errors="replace")
        for line in raw.splitlines():
            line = line.strip().lstrip("\x1e")
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            name = obj.get("name", "")
            if any(
                k in name
                for k in (
                    "http:",
                    "http3:",
                    "parameters_set",
                    "transport:parameters",
                    "version_information",
                    "alpn_information",
                    "frame_parsed",
                    "frame_created",
                    "stream_type",
                )
            ):
                events.append(obj)
        out.append({"file": path.name, "matched_events": events[:200]})
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="Triple-redundant PCAP + SSLKEYLOGFILE + QLOG dissector")
    ap.add_argument("--pcap", required=True, type=Path)
    ap.add_argument("--keylog", default=None, type=Path)
    ap.add_argument("--qlog-dir", default=None, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()
    result = dissect(args.pcap, args.keylog)
    qlogs = summarize_qlogs(args.qlog_dir)
    if qlogs:
        result["layers_used"].append("layer0_qlog")
        result["qlog_events"] = qlogs
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
