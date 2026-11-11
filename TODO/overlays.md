This file holds the entries of milestone M8: the overlay networks. The owner ruled on 2026-11-05
(`docs/decisions.md`, Tailscale): the `tailscaled` sidecar now, with a headscale harness in CI; an
in-process Tailscale later, behind a feature that is off by default (T-123, T-124). The in-process
code is `pod-ts` of `PodUtils/podcrates`. Sources: `docs/research/podnet/podssh.md`, section 2;
`docs/research/podnet/podssh-copy.md`, section 2.5 (readings).

# T-114: A headscale harness in CI for the `tailscaled` sidecar

**Source:** `docs/research/podnet/podssh.md`, section 2.3, step 3;
`docs/research/podnet/podssh-copy.md`, sections 2.5 and 3 (C11); the owner's ruling of 2026-11-05
(`docs/decisions.md`, Tailscale).
**Category:** chore
**Milestone:** M8
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The sidecar paths have no CI test with a real `tailscaled`: `net tailscale --socket`,
`pod -x tailscale:SOCK` and `net ts-shims`. CI has no Tailscale account.

## Premise

- podssh starts headscale with its embedded DERP and a CA of the run, a `tailscaled` target with
  Tailscale SSH and a userspace `tailscaled` peer, in Podman or Docker, with pre-auth keys in
  files only (`Azathothas/podssh:scripts/ts-headscale.sh`,
  `Azathothas/podssh:scripts/ts-headscale-lib.sh`,
  `Azathothas/podssh:.github/workflows/ts-headscale.yml`, and two images pinned by digest).
- The sidecar code is `crates/podnet-netns/src/tailscale.rs` and `crates/podnet-vnet/src/tailscale_ssh.rs`.

## Approach

1. Write the harness. Read first whether `pod-testkit` of PodUtils/podcrates has a headscale harness. Take it when it has. Do not copy the scripts of podssh.
2. Add runs of `net tailscale` (SOCKS5 and HTTP CONNECT to a service of a peer), of
   `pod -x tailscale:SOCK -- curl`, and of `net ts-shims` with `tailscaled --ssh`.
3. Keep the plant `TS_HS_PLANT=unknown-key`. It must fail.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Tailscale): the `tailscaled` sidecar now, with a
headscale harness in CI.

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the shared crates of `PodUtils/podcrates` hold this code. podnet depends on the shared crate; it does not copy the code.

## Prove

```sh
gh workflow run ts-headscale.yml
gh workflow run ts-headscale.yml -f plant=unknown-key   # must fail
```

## Start condition

The shared crate that this entry reads has a release in `PodUtils/podcrates` (T-137; `sh scripts/agent/podcrates.sh`).

## Correction

2026-11-05, podnet copies no file of podssh (`docs/decisions.md`, Shared crates). The harness is a script and a
workflow, not a crate: the shared crate that holds it is not known (`pod-testkit` is the candidate).
Ask the podcrates session before the work. The Premise lists the harness of podssh as a reading.

# T-115: The tailnet as a network of the registry and of the router

**Source:** `docs/research/podnet/podssh.md`, section 2.3, step 1;
`docs/research/podnet/capabilities.md`, section 2.1.
**Category:** feature
**Milestone:** M8
**Priority:** P3
**Effort:** S
**Status:** open

## Problem

After M4, the tailnet dialer is a `Dialer` (T-056). It is not yet a network with a spec, a row of
`doctor` and a member of a group.

## Premise

- `pod -x tailscale:SOCK` accepts the tailnet only as the only hop.
- T-081 reads the login state of `tailscaled`.

## Approach

1. Add `tailscale:SOCK` to the registry, with the login state as its row of `doctor`.
2. Accept it in `--via`, in `--net` of the router and in a group.
3. Test it in the harness of T-114.

## Prove

```sh
cargo test --workspace -- network_spec_tailscale
gh workflow run ts-headscale.yml
```

## Start condition

T-059 and T-114 are done.

# T-116: WireGuard: read a wg-quick file into a peer, with optional reserved bytes

**Source:** `docs/research/podnet/podssh.md`, section 2.3, step 2.
**Category:** feature
**Milestone:** M8
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The WireGuard code of podnet speaks to WARP only. Its reserved bytes are fixed, and its
configuration comes from a WARP registration. A user cannot use a self-hosted peer or the export
of a VPN.

## Premise

- `src/warp/wg.rs` is an own initiator of Noise IKpsk2, with BLAKE2s and the AEAD of ring. It
  takes the reserved bytes from the `client_id` of the device.
- `src/warp/wg.rs` has a responder for its tests (`respond`).

## Approach

1. Parse a wg-quick file: `[Interface]` (PrivateKey, Address, DNS, MTU) and `[Peer]` (PublicKey,
   PresharedKey, Endpoint, AllowedIPs, PersistentKeepalive).
2. Make the reserved bytes optional, with zeros by default.
3. Read a private key from a file path only.

## Prove

```sh
cargo test --workspace -- wg_quick_file_parses wg_reserved_bytes_are_zero_by_default wg_handshake_with_the_test_responder
```

## Correction

2026-11-05, The wg-quick reader and the reserved bytes belong to the WireGuard code, which moves to `pod-wg`
(wave 2 of PodUtils/podcrates) from `src/warp/wg.rs`. Do this work in `pod-wg` after the take, or as an
issue in PodUtils/podcrates. podnet only takes the crate (T-137).

2026-11-10, read in the tree: the reader of the format is not WireGuard code. It lands in
`crates/podnet-vnet/src/wg_quick.rs` and changes no file of `src/warp/` (the files that go to
`pod-warp` and `pod-wg` take fixes only). Step 2 needs no change of `src/warp/wg.rs`: its `Keys`
takes the reserved bytes from the caller (`src/warp/wg.rs:214`), and the reader gives zeros when
the file has no `Reserved`. The handshake test builds a `Peer` of `src/warp/wg.rs` from the file
in a test module of the root crate (`src/wg_quick_tests.rs`). When `pod-wg` exists, the reader
can move there with the take (T-137).

- The new dependency of `podnet-vnet` is `base64 = "0.23"`, for the keys. The root crate has the
  same version, so the tree gets no new crate.
- `src/warp/wg.rs` uses a zero preshared key (`src/warp/wg.rs:324`). The reader keeps
  `PresharedKey`; T-117 must refuse a file with a `PresharedKey` until the WireGuard code takes
  one.

## Done

2026-11-09, commit 1d44c85. `crates/podnet-vnet/src/wg_quick.rs` reads a wg-quick file;
`src/wg_quick_tests.rs` makes a handshake with a peer from a file. The network form (`wg:FILE`)
stays for T-117.

- `cargo test --workspace -- wg_quick_file_parses wg_reserved_bytes_are_zero_by_default
  wg_handshake_with_the_test_responder`: 3 passed.
- New tests (5): `wg_quick_file_parses`, `wg_reserved_bytes_are_zero_by_default`,
  `wg_quick_file_errors_name_the_line`, `wg_quick_file_reads_by_path` (in
  `crates/podnet-vnet/src/wg_quick.rs`), `wg_handshake_with_the_test_responder` (in
  `src/wg_quick_tests.rs`, with `need_loopback!`).
- `sh scripts/agent/check.sh`: exit 0. `unshare -n -- cargo test --offline --workspace
  --no-fail-fast`: exit 0. `sh scripts/restricted-host.sh --no-fail-fast`: exit 0. CI: at the
  merge.

# T-117: WireGuard: `wg:FILE` as a network over UDP or over a carrier

**Source:** `docs/research/podnet/podssh.md`, section 2.3, step 2.
**Category:** feature
**Milestone:** M8
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

A peer of T-116 needs a network form: a dialer over its tunnel, a spec and a carrier for hosts
with no UDP.

## Premise

- WireGuard needs UDP to its endpoint. A host behind an HTTP proxy has no UDP.
- MASQUE CONNECT-IP of WARP carries UDP (`src/warp/ip.rs`). The relay carries TCP only.

## Approach

1. Put the peer on a smoltcp stack, as the WARP leg does.
2. Send its UDP direct when a probe finds UDP, else through a WARP chain (`--via`).
3. Add `wg:FILE` to the registry.

## Prove

```sh
cargo test --workspace -- wg_network_reaches_a_service_behind_the_peer
```

A userspace responder on the loopback serves an HTTP service behind the peer.

## Start condition

T-116 and T-059 are done.

## Correction

2026-11-05, The stack is `pod-netstack` and the WireGuard code is `pod-wg` (wave 2). Step 1 puts the peer on
the smoltcp 0.14 stack of `pod-netstack`, not on a smoltcp 0.12 stack of podnet.

# T-118: Research: NetBird, Firezone and ZeroTier as networks

**Source:** `docs/research/podnet/podssh.md`, sections 2.1 and 2.3 (steps 5 and 6).
**Category:** research
**Milestone:** M8
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

podnet has no answer for NetBird, Firezone or ZeroTier. Each can need UDP, an account or a TUN.

## Premise

- podssh records them as research only (`Azathothas/podssh:docs/carriers.md`): NetBird relays over
  WebSocket on TCP 443 with a token of a management server; Firezone relays with TURN on UDP,
  behind an account; ZeroTier roots on UDP 9993, with a slow TCP fallback relay.
- Not verified: a netstack mode of NetBird with a SOCKS5 listener and no TUN; the need of the
  Firezone headless client for a TUN.

## Approach

1. For each overlay, find whether a client runs with no TUN and no root, and exposes SOCKS5 or a
   dial API; which ports it needs; and which account it needs.
2. Write the result in `docs/research/` with the commands that ran.
3. Write an entry for each overlay that passes: as a `socks5://` sidecar, or as an adapter.

## Prove

```sh
cargo todo check
```

The new entries and the research note pass the checker.

# T-243: `net tailscale`: `--tailnet NAME` refuses to start or to dial when the daemon is in another tailnet, a doctor row shows the status, and the docs name `TS_FORCE_NOISE_443` (GitHub #434)

**Source:** GitHub #434 (https://github.com/PodUtils/podnet/issues/434); donor review, tailsplit.md block 7
(idy/tailsplit at 0bd3a74, GPL-3.0-or-later)
**Category:** feature
**Milestone:** M8
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

`podnet net tailscale` opens a SOCKS5 and HTTP CONNECT door that dials through the LocalAPI of the
`tailscaled` that answers on the socket. It reads `BackendState` and warns once. It never asks which
tailnet the daemon is logged in to. Tailscale addresses are allocated inside a tailnet, so the same address
can name another machine in another tailnet (Inferred in the issue). Add `--tailnet NAME`: the door starts
only when the daemon is `Running` in that tailnet, and it refuses new dials when the tailnet changes. Add
the facts to a doctor row. Document `TS_FORCE_NOISE_443` for the hosts that reach only the port 443 (the
control connection tries the port 80 first, and a middlebox can accept it and stall; Inferred from the
upstream comment, not reproduced). The check that proves the fix: with a fake LocalAPI that answers
`{"BackendState":"Running","CurrentTailnet":{"Name":"lab"}}`, `podnet net tailscale --tailnet office`
exits 78 before it binds the listener, and the message names both tailnets.

## Premise

- Implement from the description, not from the donor source (the donor is GPL-3.0-or-later). The upstream
  facts about `ipnstate.Status` and `TS_FORCE_NOISE_443` come from the Tailscale source, not from the donor.
- `crates/podnet-netns/src/tailscale.rs:122-144` (`Dialer::backend_state`) and
  `crates/podnet-netns/src/tailscale.rs:151-170` (`backend_state_of`) read `/localapi/v0/status?peers=false`
  and keep only the string `BackendState`. `crates/podnet-netns/src/tailscale.rs:172-191` (`state_warning`,
  `check_state`) is called at the start and after a failed dial (`crates/podnet-netns/src/tailscale.rs:317`
  and `crates/podnet-netns/src/tailscale.rs:413`); it warns once and never refuses.
- `src/cli/cmd/net/shim.rs:56-88` (`net_tailscale`) reads no tailnet. `README.md:1103` says only that
  `tailscaled` must run with `--tun=userspace-networking`. The test with a fake LocalAPI:
  `crates/podnet-netns/src/tailscale.rs:624-655`.
- `Fault::Config` (exit 78) and `Fault::Unavailable` (exit 69) are in `crates/podnet-core/src/exit.rs`.
- T-081 (done) did the warning and left its step 3 ("the state in a doctor of the door") undone
  (`TODO/fixes.md:1108`). This entry closes that step. A done entry is not reopened. T-114 (open, the
  headscale harness) and T-115 (open, a tailnet as a network) are context. T-123 (`pod-ts`) is another path.
- Not run: any real `tailscaled`. `CurrentTailnet` in the status is from the upstream source of the `main`
  branch, not pinned. Old daemons carry only the top-level `MagicDNSSuffix`, so read both.

## Approach

1. Replace `backend_state()` with `status()` that returns `{ backend_state, tailnet_name?,
   magic_dns_suffix? }`, with the bounds that exist (3 s, 4 MiB). A body that is too large or is not JSON
   gives "unreadable status", never `Ready`.
2. `guard(expected, status)`: not `Running` gives `Wait`; no expected name gives `Ready`; no name and no
   suffix yet gives `Wait`; an expected name equal (case-insensitive) to the name or to the suffix gives
   `Ready`; else `Mismatch`.
3. `--tailnet NAME` (and `PODNET_TAILNET`): at the start, poll `guard` for up to 20 s (a choice, not a
   measurement). `Ready` opens the door. `Mismatch` exits 78: `the daemon is in tailnet "lab", not "office";
   nothing was bound`. A wait that ends exits 69 and names the last `BackendState`.
4. After the start, run `guard` before each dial that follows a failed dial, and each 30 s. On `Mismatch`
   the door answers new CONNECT requests with the SOCKS5 reply "connection not allowed" and with HTTP 403,
   and logs one line. Open connections stay.
5. Add a doctor row for the door: the socket, `BackendState`, the name and the suffix of the tailnet, and the
   value of `TS_FORCE_NOISE_443` as the podnet process sees it (the environment of the daemon cannot be
   read). The row says `unknown` when the daemon does not answer.
6. Docs: under `README.md:1103`, add that a host whose outbound traffic is limited to the port 443 starts
   `tailscaled` with `TS_FORCE_NOISE_443=true`. The help text says that the DNS suffix is the safer value
   of the flag (Inferred).
7. The check reads the status only. It never changes the login, the exit node or the prefs. The hop
   `pod -x tailscale:SOCKET` is not covered.

## Decision

Area and milestone: the triage proposed `TODO/fixes.md` (M5, a follow-on of T-081) or this file (M8, next
to T-114). The lead session chose M8.

## Prove

```sh
cargo test -p podnet-netns --lib -- tailscale_tailnet_mismatch_refuses_before_the_bind tailscale_tailnet_match_opens_the_door
cargo test -p podnet-netns --lib -- tailscale_needs_login_ends_with_69 tailscale_tailnet_change_refuses_new_dials tailscale_unreadable_status_is_not_ready
cargo test -p podnet-netns --lib -- tailscale_front_door_warns_when_logged_out
```

With no `--tailnet`, the behaviour is unchanged (the existing warning test passes). A fake LocalAPI in
`office` (by name, or by the suffix `office.ts.net`) opens the door. `NeedsLogin` for longer than the wait
exits 69. A fake that changes to `lab` after the start: the next dial gets the refusal and one log line, and
a connection opened before stays up.

## Done

2026-11-11, commit ccd55ae. `podnet net tailscale --tailnet NAME` (or `PODNET_TAILNET`) waits up to 20 s
for `tailscaled` to be `Running` in NAME (the tailnet name or the DNS suffix, case-insensitive), before it
binds. Another tailnet exits 78 with `the daemon is in tailnet "lab", not "office"; nothing was bound`. A
daemon that is not ready exits 69 and names the last `BackendState`. After the start, the door reads the
status each 30 s and after each failed dial. In another tailnet it answers new dials with SOCKS5 code 2 and
HTTP 403, logs one line, and keeps open connections. A status that is too large, not JSON or not 200 is an
error, never `Ready`. `podnet net doctor` has the row `tailscaled (LocalAPI)` (socket, `BackendState`, name,
suffix, `TS_FORCE_NOISE_443`; `unknown` when the daemon does not answer). The README names
`TS_FORCE_NOISE_443`. The code is in `crates/podnet-netns/src/tailscale/status.rs`.

Tests (`crates/podnet-netns/src/tailscale/guard_tests.rs`, fake LocalAPI on a unix socket):
`tailscale_tailnet_mismatch_refuses_before_the_bind`, `tailscale_tailnet_match_opens_the_door`,
`tailscale_needs_login_ends_with_69`, `tailscale_tailnet_change_refuses_new_dials`,
`tailscale_unreadable_status_is_not_ready`, `tailscale_guard_rules_and_describe`. With the guard disabled,
three of the first four fail.

Not proven: any real `tailscaled`; the `CurrentTailnet` field name (from upstream source, not pinned); the
exit codes 78 and 69 end to end through the binary (the tests prove the error type that the command maps);
the stall of port 80 (not reproduced). The hop `pod -x tailscale:SOCKET` has no guard.
