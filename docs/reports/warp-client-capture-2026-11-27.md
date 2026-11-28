# Official `cloudflare-warp` 2026.8.2115.0 capture report

Date: 2026-11-27  
Rig: `scripts/capture-rig/rig.sh --client cloudflare-warp`  
Package: `cloudflare-warp_2026.8.2115.0_amd64.deb` (Ubuntu 24.04 Noble; SHA-256 `0209107860c3c6851a9ab8fd03690121a3dea3199b084f8197620072c671cd68`, verified `OK`)  
Binary version: `warp-cli 2026.8.2115.0` (build date `2026-10-19 22:57:54.0 +00:00:00`)  
Redaction audit: `leaks_found: 0`, `redactions_applied: 57`; temporary anonymous device deleted on exit.

## 1. Purpose

This capture resolves two open issues that depended on observing the official Cloudflare WARP Linux client on the wire and in its state records:

1. **GitHub `#140` (`T-002`, `H-13`)**: why `podnet warp proxy --l4` received HTTP 403 on `consumer-masque-proxy.cloudflareclient.com`.
2. **GitHub `#407` (`T-210`, `H-31`)**: the exact closed set of `account_type` values that distinguish free accounts from paid WARP+ accounts.

It also records QUIC transport parameters, HTTP/3 SETTINGS, and TLS ClientHello fingerprints from the official client for future byte-for-byte parity work.

## 2. L4 proxy wire protocol (`#140` / `T-002`)

The official daemon (`warp-svc`) was configured with `tunnel protocol set MASQUE`, `mode proxy`, `proxy port 40000`, and `debug qlog enable`. It registered an anonymous free device and opened a QUIC v1 / HTTP/3 connection to `162.159.198.2:443` with SNI `consumer-masque-proxy.cloudflareclient.com` (`tunnel_usage: L4Proxy`).

Captured QLOG (`57d30641b3c82140bfca54e4bb2380c73cb85e38.sqlog`) shows the exact HTTP/3 frame sequence:

### 2.1 QUIC transport parameters

| Parameter | Local (`warp-svc`) | Remote (Cloudflare edge) |
| --- | --- | --- |
| `max_idle_timeout` | `5000` ms | `56000` ms |
| `max_udp_payload_size` | `1350` | `1500` |
| `initial_max_data` | `10000000` | `10000000` |
| `initial_max_stream_data_bidi_local` | `1000000` | `1000000` |
| `initial_max_stream_data_bidi_remote` | `1000000` | `1000000` |
| `initial_max_stream_data_uni` | `1000000` | `1000000` |
| `initial_max_streams_bidi` | `100` | `25000` |
| `initial_max_streams_uni` | `100` | `100` |
| `disable_active_migration` | `true` | `true` |
| `active_connection_id_limit` | `2` | `2` |
| `ack_delay_exponent` / `max_ack_delay` | `3` / `25` | `3` / `25` |

### 2.2 HTTP/3 SETTINGS

- Client (stream 2, control): `MAX_FIELD_SECTION_SIZE = 32768`, `H3_DATAGRAM = 1`, plus one GREASE setting.
- Server (stream 3, control): `MAX_FIELD_SECTION_SIZE = 32768`, `SETTINGS_ENABLE_CONNECT_PROTOCOL = 1`, `H3_DATAGRAM = 1`, plus one GREASE setting.

### 2.3 Stream 0 session bootstrap (root cause of `#140`)

Before sending any per-flow TCP `CONNECT`, `warp-svc` opens **Stream 0** with an extended `CONNECT` that establishes the MASQUE proxy session on the QUIC connection:

```text
:method: CONNECT
:protocol: cf-connect-ip
:scheme: http
:authority: cloudflareaccess.com
:path: /
pq-enabled: true
cf-client-version: l-2026.8.2115.0
```

The edge responds on Stream 0 with `:status: 200`, `cf-warp-metal`, `cf-warp-colo`, and `cf-team`. Stream 0 stays open for the lifetime of the QUIC connection.

### 2.4 Per-flow TCP `CONNECT`

On a subsequent bidirectional stream (for example Stream 8), `warp-svc` sends a plain RFC 9114 HTTP/3 `CONNECT` with only `:method` and `:authority`:

```text
:method: CONNECT
:authority: 162.159.197.4:443
```

The edge responds with `:status: 200` and relays raw TCP bytes inside HTTP/3 `DATA` frames on that stream.

**Root cause of HTTP 403 in `pod-warp::l4`:** `pod-warp::l4::L4` previously sent per-flow `CONNECT` directly on Stream 0 without first opening the Stream 0 `cf-connect-ip` session bootstrap.

## 3. Account types and WARP+ classification (`#407` / `T-210`)

Live anonymous registration with `warp-cli registration new` and `warp-cli registration show` returned:

- `warp-cli registration show`: `Account type: Free`
- `conf.json`: `{"account": {"account_type": "free", ...}}`

The official `warp-svc` binary defines the closed `warp_primitives::warp_account::WarpAccount` enum with four variants:

| Variant | `account_type` | Fields | WARP+ state |
| --- | --- | --- | --- |
| `WarpAccount::Free` | `"free"` | `id`, `license` | `"no"` (even when raw `warp_plus: true`) |
| `WarpAccount::Limited` | `"limited"` | `id`, `license`, `premium_data`, `quota` | `"yes"` only when `premium_data > 0`, else `"no"` |
| `WarpAccount::Unlimited` | `"unlimited"` | `id`, `license`, `plus_expires_at` | `"yes"` |
| `WarpAccount::Team` | `"team"` | `id`, `managed`, `organization` | `"yes"` (Zero Trust organization) |

`src/warp/identity.rs` (`warp_plus_label`) now matches the closed paid set (`"unlimited"`, `"team"`, `"premium"`) plus `premium_data > 0`, failing closed to `"no"` for `"free"`, `"limited"` without `premium_data`, empty strings, and unknown types.

## 4. Zero Trust registration findings (`#149` / `T-003`)

The official `warp-svc` binary also confirms `AccessJwtClaims` (`iss`, `account_id`), `RegistrationAuthMethod::{Access, ServiceToken, WarpConnectorToken}`, headers `CF-Access-Jwt-Assertion`, `cf-access-client-id`, and `cf-access-client-secret`, and Zero Trust SNIs `zt-masque.cloudflareclient.com` and `zt-masque-proxy.cloudflareclient.com`. `pod-warp` already exposes `Api::with_team(token)` for `CF-Access-Jwt-Assertion`; full CLI wiring remains tracked in `T-003` (`#149`) pending `pod-warp::Account` `managed`/`organization` fields and a Zero Trust team token for live verification.
