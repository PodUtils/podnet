# Client capture rig

`scripts/capture-rig/` is a hermetic, idempotent capture harness for official third-party clients
(`cloudflare-warp`, `cloudflared`, `netbird`, or any future binary). It records wire protocols without
changing the host and scrubs all secrets before writing the output bundle.

## Design

Each run records all wire planes inside one ephemeral container so future protocols can be compared
byte-for-byte:

1. **Passive packet capture (`tcpdump -U --immediate-mode`):** writes a packet-buffered, full-length
   PCAP across all interfaces inside the container, so pinned TLS and QUIC/UDP traffic are captured
   even for short-lived commands or clients that ignore proxy variables.
2. **TLS/QUIC key logging and QLOG (`SSLKEYLOGFILE` + `QLOGDIR`):** captures session secrets for
   clients that honor `SSLKEYLOGFILE` (`curl`, Go, OpenSSL) and structured QLOG/SQLOG event streams
   for QUIC/HTTP/3 clients (`warp-svc` via `debug qlog enable`, `quiche`, and `QLOGDIR` clients).
3. **Auto-fallback MITM and passthrough proxy (`mitm_proxy.py`):** generates an ephemeral per-run CA,
   records decrypted HTTP exchanges to `http_transcript.jsonl`, and automatically falls back to
   SNI/ALPN-logging passthrough when a client rejects the ephemeral CA (certificate pinning).
4. **Four-layer wire dissector (`quic_dissect.py`):** combines QLOG HTTP/3 frames and QUIC transport
   parameters (layer 0), `tshark` decrypted HTTP/3 headers and SETTINGS, HTTP/2 headers and SETTINGS,
   and HTTP/1.1 exchanges (layer 1), passive `tshark` TLS ClientHello ciphers, extensions, ALPN,
   supported groups, signature algorithms, and QUIC Initial transport parameters (layer 2), and a
   zero-dependency pure-Python PCAP SNI scanner (layer 3). Optional `--strace` records network
   syscalls in `generic` mode.

Before exiting, `redact.py` runs inside the container (so mode-0600 daemon state files are scrubbed
before touching host disk), drops raw keylogs, CA private keys and raw PCAPs unless `--keep-raw` is
passed, redacts tokens, API keys, licenses, JWTs, cookies, authorization headers and PEM keys across
every JSON, JSONL and text file, verifies zero leaked secrets (`redaction_audit.json`), and writes a
SHA-256 `manifest.json`. An `EXIT` trap removes the ephemeral container and (for `cloudflare-warp`)
deletes the temporary anonymous device registration.

## Usage

```sh
# Offline self-test (no Docker and no internet needed)
sh scripts/capture-rig/rig.sh --self-test

# Capture official cloudflare-warp in proxy mode (downloads SHA-256 verified .deb)
sh scripts/capture-rig/rig.sh --client cloudflare-warp --out /tmp/warp-capture

# Capture any future client binary or command
sh scripts/capture-rig/rig.sh --client generic --cmd 'curl -fsS https://example.com/' --out /tmp/generic-capture

# Remove the local Docker image
sh scripts/capture-rig/rig.sh --clean
```

## Adding a new client recipe

Add a POSIX shell script at `scripts/capture-rig/clients/<name>.sh` that writes raw artifacts into
`$CAPTURE_RAW_DIR` and invokes `python3 /rig/quic_dissect.py` before returning. `rig.sh` handles image
build, container lifecycle, redaction, zero-leak verification and cleanup automatically.

## Continuous verification

`tests/capture_rig.rs` runs offline on every CI pass to verify `mitm_proxy.py` (HTTP and CONNECT
passthrough SNI capture), `quic_dissect.py` (pure-Python PCAP SNI extraction) and `redact.py`
(redaction, keylog removal, SHA-256 manifest and zero-leak audit).
