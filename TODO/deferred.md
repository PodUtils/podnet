The deferred GitHub issues: the owner keeps each one open, and each entry
waits for the owner or for a condition outside the repository
(`docs/decisions.md`, "Deferred issues"). The Problem of each entry keeps
the text of its issue that the work needs.

# T-001: Tracking: dev work in a sandbox with basic networking, and launch once and forget (GitHub #89)

**Source:** GitHub #89, the tracking issue of the owner.
**Category:** chore
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** blocked

## Problem

The goal of the issue: "A sandbox with basic networking (an allowlisted
HTTPS proxy only, or TCP 80/443 only, no UDP, no IPv6, no root) gets
working dev networking from one command, and podnet runs in the background
with no further care."

The rulings in the issue: MASQUE through the WebSocket relay with an
automatic fallback (`--no-relay` turns it off); later shells join through
`podnet warp env`, a `--persist` shell hook, `podnet warp exec` and
`podnet warp shell`; detached instances restart by default and record a new
quick-tunnel URL; tests of network paths run on GitHub runners
(`.github/workflows/live.yml`), not through a way around the policy of a
sandbox.

The items of the issue are #70 to #88: detach with a ready document,
the instance registry, `ps`/`stop`/`logs -f`/`wait`, the supervisor,
autostart, the WARP reconnect, MASQUE through the relay, pod loopback and
UDP, later shells, `warp up` with root, netns raw IP with no root, IPv6,
`doctor --agent`, `warp nc`, the CA bundle inside WARP, proxy detection
and Digest, UDP with no kernel `AF_INET` bind, the reverse relay, and
`podnet agent up`. GitHub has each of them closed.

## Premise

Read: the issue and its comments, and `docs/decisions.md`. A tester ran
`podnet agent up` end to end on a host of the target class with
0.3.0-beta.6 (no root, no TUN, TCP bind refused, TCP egress 80/443 only):
WARP registration, the detached `warp proxy` on a Unix socket, then
`warp exec`, `warp shell` and `warp nc`. Not measured in this sandbox.

## Approach

1. Make each new request of the goal an entry of its own, with the
   Source `GitHub #89`.
2. When the owner closes the issue, write `## Done` with the state of the
   issue as the evidence.

## Prove

```sh
curl -qfsS "https://forge.web.pkgforge.dev/github/repos/PodUtils/podnet/issues/89?n=$RANDOM" |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["state"])'
```

The command prints `closed`.

## Blocker

The owner. The ruling "Deferred issues" of 2026-11-04 in
`docs/decisions.md` keeps the issue open as the tracking issue of the
owner. The entry is unblocked when the owner closes the issue, or names
the work that remains.

## Correction

2026-11-11, the owner (`docs/decisions.md`, #89): edit #89 into a guide that a new client can test from, and
keep it open. Done the same day: the body of GitHub #89 is now "Test guide: podnet on a host with limited
networking": the install and its checks, the doctors, `agent up`, WARP with no root, the publish of a local
service, the relay, `--detach` and autostart, the pods, the router (with a full router file), forwards,
`bench`, the overlays, the manual, what to report, and the work that needs an account. Each command of it was
checked against the help of the binary of 0.4.0-beta.4. The old tracking list stays at its end (each item
done). The entry stays blocked: #89 stays open as the guide of the owner.

2026-11-12, GitHub #475 asks that a row which cannot run on a platform is named in the guide of #89, and GitHub #467 names the CI jobs `restricted-host` and `no-loopback` for the carriers that run there (T-274, T-275). Add the link rows to the guide when the link commands exist (T-265).

2026-11-27: refreshed GitHub #89 to `v0.4.0-beta.21`: added `podnet link` and the `restricted-host` / `no-loopback` carrier table, `derp`, `send`/`receive`, `serve --acme`, `tunnel --inspect`, `exit --stdio`, `ssh-relay`, all `--network` egress specs, and the T-247 proxy-only NetBird note (removed NetBird from the untested list). Each command checked against `./target/debug/podnet --help`. The entry stays blocked because #89 stays open as the owner's guide.
# T-002: `warp proxy --l4`: the L4 door answers 403 to each form of the CONNECT (GitHub #140)

**Source:** GitHub #140.
**Category:** feature
**Milestone:** M9
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The issue: each TCP client of `podnet warp proxy` becomes IP packets in the
userspace stack over one CONNECT-IP stream. Each flow has TCP buffers of
512 KiB in each direction, all flows share one driver task and one HTTP/2
stream window, and a name costs two DNS queries in the tunnel before the
connect. The issue asks for `warp proxy --l4`: one HTTP/3 `CONNECT` stream
for each TCP client to the L4 door of WARP (UDP 443, TLS name
`consumer-masque-proxy.cloudflareclient.com`), with no IP stack. The name
goes in `:authority`, so the service resolves it. UDP is not carried in
this mode.

The client side is in `src/warp/l4.rs` (commits `9d30496`, `81e56d0`,
`3e1e1a3`; the rules apply to `--l4` from `db6dbe6`; clap refuses `--l4`
with `--dns` or `--listen-identity`). The live runs show that QUIC, TLS
and the pinned key of the device work with the TLS names
`consumer-masque-proxy.cloudflareclient.com` and
`consumer-masque.cloudflareclient.com`. The door answers 403 to each
CONNECT, in each of three forms (`PODNET_WARP_L4_FORM`):

- `plain`: `:method CONNECT`, `:authority host:port` (RFC 9114,
  section 4.4);
- `extended`: `:protocol cf-connect-tcp` with `:scheme` and `:path`
  (RFC 9220);
- `header`: a plain CONNECT with `cf-connect-proto: cf-connect-tcp`.

## Premise

Read: `src/warp/l4.rs`, `src/warp/proxy.rs`, and the report of the three
forms in `scripts/live-batch2.sh`. `docs/protocol.md` says that the L4
proxy mode of the front door is HTTP/3 only. Not measured here: this
sandbox cannot reach WARP. The door knows the device and refuses the
request. Possible causes: an anonymous consumer device has no right to
the L4 door, or the door wants a form that is not one of the three.

## Approach

1. Get one captured request of an official client in proxy mode: the
   pseudo-headers and the headers, with the secrets replaced.
2. Send that form from `src/warp/l4.rs`, and add a loopback test of the
   field section of that form.
3. Make the `--l4` report of `scripts/live-batch2.sh` a check: a download
   through the door gets HTTP 200.

## Prove

```sh
cargo test -p podnet --lib warp::l4
```

Then the `--l4` step of `scripts/live-batch2.sh` in a run of
`.github/workflows/live.yml` prints HTTP 200 for the download through the
door.

## Done

2026-11-27 (`ae09cb3`, `465bc0f`): built hermetic, idempotent four-layer capture rig (`scripts/capture-rig/`, `docs/capture-rig.md`, `tests/capture_rig.rs`) and captured live official `cloudflare-warp` `2026.8.2115.0` in `WarpProxy` mode (`docs/reports/warp-client-capture-2026-11-27.md`, 0 redaction leaks). QLOG proved `consumer-masque-proxy.cloudflareclient.com` requires Stream 0 extended `CONNECT` (`:protocol cf-connect-ip`, `:scheme http`, `:authority cloudflareaccess.com`, `:path /`, `cf-client-version`) kept open for the QUIC connection, followed by per-flow plain RFC 9114 `CONNECT` with IP-literal `:authority`. Implemented Stream 0 bootstrap + DoH-over-L4 IP resolution (honoring `--resolve` pins, zero host DNS leak) in `src/warp/l4.rs`, wired `src/warp/mod.rs` and `src/warp/proxy.rs`, added unit + loopback HTTP/3 stand-in tests (`bootstrap_headers_match_official_wire_format`, `l4_bootstrap_and_connect_round_trip`), verified live (`warp=on` on `1.1.1.1` and `www.cloudflare.com`, 20 MB `speed.cloudflare.com` download HTTP 200 at 5.90 MB/s), upgraded `scripts/live-batch2.sh` `--l4` to a real check, and filed COORD-146 (`#804`) to sync `pod-warp::l4`.

# T-003: `warp register`: enrol through a Zero Trust team token (GitHub #149)

**Source:** GitHub #149.
**Category:** feature
**Milestone:** M9
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

`src/warp/register.rs` creates an anonymous consumer device only; the
requests have no field for a team credential. An organization that runs
Zero Trust cannot enrol a host with podnet: the device must come from the
dashboard or the official client, and the front-door name must be set by
hand (`--masque-sni`) on each command. `Account` has `id`,
`account_type` and `license` only, so a managed account prints as a
consumer account with no licence.

The proposal of the issue:

- Flags: `--team-token TOKEN` (`PODNET_WARP_TEAM_TOKEN`; hidden from `ps`
  and the logs), `--team-token-file PATH` (`PODNET_WARP_TEAM_TOKEN_FILE`,
  mode 0600), `--team-sni NAME` (`PODNET_WARP_TEAM_SNI`; default: the host
  of the `iss` of the token). The two token forms conflict with each other
  and with `--ephemeral`. With an existing identity file, the flag has no
  effect (one debug line).
- Parse the token locally (a JWT: `iss`, `aud`, `exp`; no signature
  check). A malformed or expired token exits 78 before a network call, and
  the message gives the expiry date. Another refusal of the API exits 69.
- Send the token only on the `POST /reg` of a new device. The issue
  proposes the header `CF-Access-Jwt-Assertion`.
- The MASQUE TLS name of a team device defaults to `zt-masque.DOMAIN`;
  `--masque-sni` has priority.
- `Account` gets `managed` and `organization`; the identity file records
  `"team": {"domain": ..., "enrolled_at": ...}` and never the token.
  `warp license` on a managed device exits 78 with the reason.
- `--ephemeral` with `--team-token` exits 64.

## Premise

Read: `src/warp/register.rs`. The enrolment through a team token is not
documented. podnet has no Zero Trust organization to test with. A wrong
request can register devices that a team does not want. The header name
of the proposal is not measured.

## Approach

1. Get a captured exchange of the official client against a test team:
   the request and the answer of the team token step (the path, the
   headers without the token, the JSON body with the secrets replaced),
   and the `POST /reg` request and answer of that device.
2. Add the flags to `podnet warp register`. Keep the anonymous path as it
   is.
3. Test the parse of the token, the header and the exit codes against a
   loopback API stand-in (`PODNET_WARP_API`).

## Prove

```sh
cargo test -p podnet --lib warp::register
```

## Blocker

A captured exchange of the official client against a test Zero Trust
team. A tester or the owner gives it. The ruling "#149" of 2026-11-04 in
`docs/decisions.md` holds the work until it comes.

# T-004: TLS: send an encrypted ClientHello on the hops that publish a config (GitHub #151)

**Source:** GitHub #151.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Each TLS connection of podnet sends the server name in the clear. The
builders are `client_config` in `crates/podnet-core/src/trust.rs` (the WARP API, the edge
legs, the relay), `tls_config` in `src/warp/session.rs` (MASQUE, with the
pinned key) and `client_config` in `PodUtils/podnet@7fe60e0:src/cloudflare/quic.rs` (QUIC to the
edge). None of them enables ECH.

The proposal of the issue: `--ech auto|off|CONFIG` (`PODNET_ECH`, default
`off`), `--ech-front NAME` (default `cloudflare-ech.com`),
`--ech-hops LIST` (`api,edge,masque`), `--ech-allow-plaintext`,
`--ech-grease`. Fetch the config from the HTTPS record (type 65) through
the DoH client and cache it with its TTL. Refuse a name that is longer
than the `max_name_length` of the config, with both numbers in the
message. Retry once with the retry configs of a rejection. Report the ECH
status of each hop in `doctor`. The ClientHello stays the same with ECH
off.

A limit: the public config of `cloudflare-ech.com` has
`max_name_length` 18, and the MASQUE name
`consumer-masque.cloudflareclient.com` has 36 characters.

## Premise

Read: `crates/podnet-core/src/trust.rs`, `src/warp/session.rs`, `PodUtils/podnet@7fe60e0:src/cloudflare/quic.rs`.
ECH needs HPKE. The crypto provider of podnet is ring, which has no HPKE.
The HPKE of rustls is in the aws-lc-rs provider (a C build on each of the
8 targets) or in another crate. The MASQUE front door and the WARP API
publish no ECH config that podnet can use, so ECH changes nothing on them.
Done in place of ECH: the ClientHello of MASQUE can be split into records
(#135, commit `b811b8e`), and the session can go over HTTP/3 or WireGuard
(#157, #152).

## Approach

1. Look for an `ech=` parameter in the HTTPS records of the hops that
   podnet uses.
2. When a hop publishes a config: write a client HPKE over ring (X25519,
   HKDF-SHA256, AES-128-GCM) with the test vectors of RFC 9180, then add
   `--ech` (off by default) for that hop.

## Prove

```sh
cargo test -p podnet --lib ech
```

## Correction

2026-11-05, `crates/podnet-core/src/trust.rs` is glue over `pod-tls` (PodUtils/podcrates), and the client configurations are
in `pod-tls`. An ECH change goes to `pod-tls` as an issue or a change there (the ruling "Who edits a
shared crate"), not into `crates/podnet-core/src/trust.rs`. The MASQUE configuration of `src/warp/session.rs` goes to `pod-warp`
and the QUIC configuration of `PodUtils/podnet@7fe60e0:src/cloudflare/quic.rs` to `pod-cloudflare` (T-050, T-052). The blocker
stays.

## Done

2026-11-27 (`715720c`): enabled `features = ["ech"]` on `pod-tls`, `pod-dns`, `pod-proxy`, and `pod-ws` (`podcrates v0.86.0` pure-Rust RFC 9180 HPKE over `ring`, zero new external crates). Added `src/ech.rs` and `src/ech/tests.rs` implementing `--ech off|auto|strict|CONFIG` (`PODNET_ECH`), `--ech-grease` (`PODNET_ECH_GREASE`), `PODNET_ECH_FRONT`, `PODNET_ECH_HOPS`, `max_name_length` validation, `EchConfig` and `EchGreaseConfig` wiring across `podnet-core::trust`, `fetch`, `ws`, `ws_relay`, and `warp`, plus ECH hop reporting in `doctor` and `docs/proxies.md`. Verified with `cargo test -p podnet --lib ech` (7 passed) and loopback integration tests `tests/ech.rs` (3 passed).

# T-005: crates.io: publish the library and the CLI (GitHub #161)

**Source:** GitHub #161 (from #160, item K3).
**Category:** release
**Milestone:** M9
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

`Tunnel`, `WarpClient` and the userspace network stack are public API of
the library crate. The issue asks to publish the CLI and `podnet-netns` on
crates.io. Facts of the issue:

- The name `cfrs` on crates.io belongs to another project; `podnet` is free (T-007).
- A publish needs a crates.io token as a secret of the repository, and a
  manual step in the release (or a `publish` input).
- `podnet-netns` is a path dependency. It must be published first, with the
  same version.

## Premise

Read: `Cargo.toml`, `crates/podnet-netns/Cargo.toml`, and the step "Package
lists" of `.github/workflows/ci.yml`. The packaging defects that a publish
shows are fixed (commit `6080d99`), and CI runs `cargo package --list` for
both crates.

## Approach

1. Use the package names of the rename (T-007).
2. Set the stable version (T-006).
3. Set `[workspace.metadata.podnet] crates-io = true` at the tag. The workflow
   `.github/workflows/publish.yml` publishes by trusted publishing, with no
   token and no step in `release.yml`.

## Prove

```sh
cargo publish --dry-run --locked -p podnet-netns
cargo publish --dry-run --locked -p podnet
```

## Blocker

The owner. The ruling "crates.io" of 2026-11-05 in `docs/decisions.md`:
publish nothing now, and keep the packaging ready for a publish.

## Correction

2026-11-05, step 3 is done in part: `.github/workflows/publish.yml` exists (trusted publishing, a dry run
of `cargo publish --workspace` in each run). The real publish waits for the owner. A
`pod-*` dependency by a git tag stops the dry run until podcrates publishes its crates to crates.io (the
ruling "Git dependencies and the publish"). The names of the other crates are in T-007.

# T-006: crates.io: the workspace version and the `podnet-netns` requirement are prereleases (GitHub #222)

**Source:** GitHub #222.
**Category:** release
**Milestone:** M9
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

`Cargo.toml` has the workspace version `0.3.0-beta.12`, and the
dependency `podnet-netns` requires `0.3.0-beta.12`. crates.io marks a version
with a pre-release identifier as a prerelease: it does not satisfy
`^0.3.0`, and `cargo install` skips it unless `--version` names it. A
requirement `^0.3.0-beta.N` matches only prereleases, so it does not match
a stable `podnet-netns` `0.3.0`. The first stable version of a line is
`X.Y.0` of that line (`0.3.0` now; podnet starts at `v0.4.0-beta.1`, the
ruling "Rename" of 2026-11-05). Change the workspace version and each
requirement between the crates in one commit. The release notes tell the
users who pinned a prerelease to change to the stable version.

## Premise

Read: `Cargo.toml`. `cargo publish --dry-run` does not find either
problem: crates.io decides at the upload.

## Approach

At the first stable release, in one commit: set the workspace version and
each requirement between the crates to `X.Y.0` (step 1 of "Release" in
`docs/maintenance.md`), and write the note in the release notes.

## Prove

```sh
grep -n '^version = "[0-9]*\.[0-9]*\.0"$' Cargo.toml
cargo package --locked --list --workspace
```

## Blocker

The owner: the first stable release. The ruling "Deferred issues" of
2026-11-04 in `docs/decisions.md` keeps the issue open until that release.

# T-007: crates.io: the name `cfrs` belongs to another project; choose a package name (GitHub #224)

**Source:** GitHub #224.
**Category:** release
**Milestone:** M9
**Priority:** P2
**Effort:** S
**Status:** blocked

## Problem

The crate `cfrs` on crates.io is a different, older project (created
2024-04-14, version 1.1.0). `cargo publish --dry-run` does not find a name
collision. A publish needs a new package name (the binary keeps its name
through `[[bin]] name`), or a transfer of the name through crates.io
support. The same commit changes the install text of `README.md`, the
program-name strings of the CLI, the cross-references of
`crates/podnet-netns/README.md`, and adds a crates.io install section
(`cargo install NAME`, `cargo add`) and a docs.rs badge to `README.md`.

## Premise

Read: the answer of `https://crates.io/api/v1/crates/cfrs` that the issue
quotes. The ruling "Name" of 2026-11-05 in `docs/decisions.md` names the
project podnet, and the ruling "Shared crates" gives it five crates (`podnet`,
`podnet-core`, `podnet-netns`, `podnet-vnet` and `podnet-serve`). Measured on 2026-11-05:
`https://crates.io/api/v1/crates/podnet` and
`https://crates.io/api/v1/crates/podnet-netns` answer 404 (no such crate).
Another project can take a free name before the publish.

## Approach

1. The rename (milestone M1) sets the package names of the ruling
   "Crates".
2. Before the publish: read the crates.io API for each name again.
3. In the commit of the publish: the install text of `README.md`, a
   crates.io install section and a docs.rs badge.

## Prove

```sh
curl -qsS -o /dev/null -w '%{http_code}\n' https://crates.io/api/v1/crates/podnet
cargo package --locked --list --workspace
```

## Blocker

The owner's ruling: no real version goes to crates.io now (the rulings "crates.io" of 2026-11-05 and "Releases" of
2026-11-09 in `docs/decisions.md`). The placeholders of the crate names are done (see the Correction).

## Correction

Measured on 2026-11-05, after the rename: `https://crates.io/api/v1/crates/podnet`
answers 200. The owner published the placeholder `podnet` 0.0.0 (repository
`PodUtils/podnet`). `podnet-core`, `podnet-netns`, `podnet-vnet` and
`podnet-serve` answered 404 then: they wait in the queue of the owner. The names
`podnet-relay`, `podnet-warp` and `podnet-cloudflare` are not used: `pod-relay`, `pod-warp` and
`pod-cloudflare` are crates of PodUtils/podcrates.

Measured on 2026-11-11: `https://crates.io/api/v1/crates/NAME` gives the placeholder 0.0.0 for `podnet`,
`podnet-core`, `podnet-netns`, `podnet-vnet` and `podnet-serve`. The owner action of the names is done.


# T-365: Deferred: carry a session in NTPv4 extension fields, SMTP and IMAP (GitHub #484)

**Source:** GitHub #484 (https://github.com/PodUtils/podnet/issues/484), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #484 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
Some networks permit time synchronisation and mail, and nothing else. Both services are already present on the network, and both can carry a payload.

| Carrier | Service | Status | What is missing |
| --- | --- | --- | --- |
| NTPv4 extension field | UDP 123 | absent | the carrier, the field type rule, the poll loop, the clock discipline |
| SMTP submission | TCP 587 with STARTTLS, TCP 465 implicit TLS | absent | the carrier, batch framing, size-limit reporting |
| IMAP retrieval | TCP 143 with STARTTLS, TCP 993 implicit TLS | absent | the carrier, fetch framing, size-limit reporting |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #484".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #484`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --features full --test link_carriers_drive
cargo todo check
```

Each carrier (NTPv4 extension fields, SMTP submission, and IMAP retrieval) builds behind its feature (`ntp`, `smtp`, `imap`, and `full`) and passes its tests.

## Done

2026-11-23, consolidated onto `main` with `podcrates` `v0.82.0`. Added the `ntp`, `smtp`, and `imap` features in `Cargo.toml` (wired to `pod-link/ntp`, `pod-link/smtp`, `pod-link/imap`, and included in `full`), registered the three carriers in `src/carriers.rs`, and verified SMTP rate/size-limit reporting and 100 NTP exchanges in `tests/link_carriers_drive.rs`. Documented in `docs/link.md`.


# T-366: Deferred: carry a session through a TURN allocation, with RFC 6062 (GitHub #485)

**Source:** GitHub #485 (https://github.com/PodUtils/podnet/issues/485), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #485 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A conferencing network permits TURN. A TURN allocation is a relay the network already trusts, and it carries a session when every other path is closed.

| Capability | Status | What is missing |
| --- | --- | --- |
| UDP allocation (RFC 8656) | absent | allocation, permission, refresh, channel data |
| TCP allocation to a peer (RFC 6062) | absent | the relayed stream and its framing |
| Traversal with a relay fallback | partial | #469 scopes traversal; the relay fallback path does not exist |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #485".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #485`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --test link_p2p_cli --test link_p2p_udp_refused --test link_p2p_namespaces
cargo todo check
```

TURN allocation (RFC 8656 and RFC 6062 TCP relay both ways) and NAT traversal with automatic relay fallback pass their tests.

## Done

2026-11-23, consolidated onto `main` with `podcrates` `v0.82.0`. Wired RFC 8656 UDP and RFC 6062 TCP TURN relay fallback in `podnet link p2p` and `pod-turn` `v0.82.0`, verified refusal fallback in `tests/link_p2p_udp_refused.rs` and `tests/link_p2p_cli.rs`, and the three-namespace NAT lab in `tests/link_p2p_namespaces.rs`. Documented in `docs/link.md`.


# T-367: Deferred: close four QUIC gaps: MASQUE enrolment, HTTP/3 shape, version, connection ID (GitHub #486)

**Source:** GitHub #486 (https://github.com/PodUtils/podnet/issues/486), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #486 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
The QUIC roads share one stack. Four gaps remain on it: the MASQUE client is bound to one provider, the HTTP/3 shape is not a server shape, the UDP-over-QUIC-stream roads stop at one protocol version, and the connection ID never rotates.

| Capability | Status | What is missing |
| --- | --- | --- |
| MASQUE `CONNECT-IP` and `CONNECT-UDP` to any endpoint | partial | `--masque-endpoint`, one client split from the enrolment |
| HTTP/3 server shape | partial | a selectable shape and a conformance check |
| Juicity | absent | the road |
| Connection-ID rotation and migration | absent | the schedule, the migration path, the fallback |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #486".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #486`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Decision

2026-11-23: HTTP/3 server shape (`podnet warp h3`, `src/warp/h3/serve.rs`, `src/warp/h3/qlog.rs`, `tests/quic_qlog.rs`, `tests/quic_h3_stock.rs`), the `podcrates` `v0.82.0` control-stream fix, the TUIC v5 road and bounded drop policy (`src/roads/tuic/`, `tests/quic_tuic.rs`, `tests/quic_tuic_cli.rs`, `tests/quic_tuic_reference.rs`), the Juicity road with 30-stream fallback (`src/roads/juicity.rs`, `tests/quic_juicity.rs`, `tests/quic_juicity_reference.rs`), and connection-ID rotation with refused-migration fallback (`tests/quic_rotation.rs`) are consolidated on `main` (`docs/quic.md`). This entry stays `partial` because MASQUE `QUIC-1` to `QUIC-5` (provider enrolment split, `--masque-endpoint`, `CONNECT-UDP` to a local echo, IPv6-only host, and a second provider) remain open.

## Prove

```sh
cargo test -p podnet --test quic_qlog --test quic_tuic --test quic_tuic_cli --test quic_juicity --test quic_rotation --test quic_masque
cargo todo check
```

## Done

2026-11-27, commit `219b244`. Closed the final gap of GitHub #486 (MASQUE `QUIC-1` to `QUIC-5`) on the existing
`pod-quic` (`noq`) and `pod-warp::h3` stack with no new dependencies:

- `src/warp/masque/` (`mod.rs`, `client.rs`, `session.rs`, `enrol.rs`, `family.rs`, `error.rs`, `standin.rs`):
  provider-agnostic MASQUE client carrying `CONNECT-IP` (RFC 9484), `CONNECT-UDP` (RFC 9298) and WebTransport
  over one QUIC stack; `MasqueEnrolment` trait with two providers (`WarpEnrolment` and `TokenEnrolment` /
  `EndpointEnrolment`); `--masque-endpoint <IP:PORT|[IPv6]:PORT|HOST:PORT|https://HOST[:PORT]>` wired into
  `src/warp/mod.rs` and `src/cli/cmd/warp/`; `IpPolicy::Ipv6Only` enforcing `[::]:0` bind and zero IPv4 socket
  attempts; `MasqueError::{Enrolment, Transport}` surfaced in `warp status` and `warp status --json`; docs in
  `docs/quic.md` and `docs/warp.md`.
- Tests: `warp::masque` (8 passed) and `tests/quic_masque.rs` (6 passed, all guarded with `need_loopback!()`,
  covering `CONNECT-IP`, `CONNECT-UDP` echo, WebTransport, IPv6-only `[::1]` zero-IPv4-attempt, second
  provider enrolment, and enrolment vs transport failure reporting).


# T-368: Deferred: register third-party services as roads, with a watch, a date and a count (GitHub #489)

**Source:** GitHub #489 (https://github.com/PodUtils/podnet/issues/489), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** partial

## Problem

The owner deferred GitHub #489 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
Third-party services are free capacity that somebody else operates. Each is a road, and each is also a dependency that can vanish. The tree has the mechanisms and no register: a road with no watch, no date and no count is a road that fails without warning.

| Capability | Status | What is missing |
| --- | --- | --- |
| IRC as a registered road | partial | the road registration and the cost class |
| MQTT and Syncthing relays as roads | absent | the roads |
| Watch every third-party road | partial | a watch entry per road, and the date in `status` |
| `front=` health for a fronted road | partial | a refusal that names a dead front |
| Network adapters as a fallback class | partial | one class, and a prohibition on default selection |
| Dependency count in `status` | absent | the count |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #489".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #489`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --test third_party_roads
cargo todo check
```

## Correction

2026-11-27, commit `8818d27`: implemented all six capabilities of GitHub #489 in podnet with zero hardcoded
third-party hostnames (`src/roads/third_party/{mod,irc,mqtt,syncthing,watch,front,fallback}.rs`,
`src/carriers.rs`, `src/cli/cmd/link/mod.rs`, `src/status.rs`, `docs/link.md`, `tests/third_party_roads.rs`,
9 passed): IRC, MQTT 3.1.1 and Syncthing Relay v1 codecs with loopback stand-ins, `podnet link carriers --third-party`
(cost class and last-verified date from `pod-services`), non-panicking `WatchState::{Ok, Degraded, Stale}`,
`front=` health probe clamped to `<= 1 s`, `RoadClass::FallbackAdapter` never selected by default, and
deduplicated 7-class dependency counts plus oldest verified date in `podnet status`. What stays: upstream
`pod-services` `irc` entry and multi-vantage scheduled watch runs asked in COORD-145 (PodUtils/podnet#803,
to podcrates).


# T-369: Deferred: switch after a real handshake, with a cover script and multiplexed cover (GitHub #491)

**Source:** GitHub #491 (https://github.com/PodUtils/podnet/issues/491), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #491 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A handshake that completes and then switches defeats a scanner that only opens a connection and looks. A TLS-in-TLS form defeats a scanner that completes a handshake and then looks at the record layer. Both are the same idea applied at two depths, and one cover site serves both.

| Capability | Status | What is missing |
| --- | --- | --- |
| Real handshake, then switch on an authenticated byte | absent | the HMAC, the timestamp window, the fall-through |
| Cover script for the record layer | absent | the script format and its loader |
| Multiplexed cover with a volume budget | absent | the multiplexer |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #491".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #491`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Decision

2026-11-23: consolidated `pod-cover` `v0.82.0` (`src/cover.rs`, `src/cli/cmd/cover.rs`, `tests/cover.rs`, `tests/cover_harness/mod.rs`, `tests/data/cover/`): real TLS handshake with exporter-bound switch (`authenticate_bound`), cover script replay (`pod_cover::Script`), and multiplexed cover (`pod_cover::Multiplex` with volume budget).

## Prove

```sh
cargo test -p podnet --test cover
cargo todo check
```

The cover tests pass, including exporter-bound switch fallback, cover script replay, and multiplexed cover budget checks.

## Done

2026-11-23, consolidated onto `main` with `pod-cover` `v0.82.0`. `src/cover.rs`, `src/cli/cmd/cover.rs`, and `tests/cover.rs` verify TLS exporter-bound switching, script replay, and multiplexed cover with volume budget.


# T-370: Deferred: shape the record layer: padding profiles, entropy, first flight, pacing (GitHub #493)

**Source:** GitHub #493 (https://github.com/PodUtils/podnet/issues/493), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #493 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A record layer that pads to one fixed bucket is a record layer with a signature. Padding has to follow the protocol it claims to be, and the session has to know what shape it declared.

| Capability | Status | What is missing |
| --- | --- | --- |
| Padding profiles as data per protocol | absent | the profile files and the selector |
| Content-entropy control | absent | the band and the printable run |
| First-flight sizing | partial | the coalesce target and the rate bound |
| Pacing with jittered keepalives | partial | a per-channel window pacer for SSH (`podssh-ssh/src/pace/`, PodUtils/podssh@T-661); record pacing by measured rate and keepalive jitter are absent |
| Declared versus observed shape in `doctor` | absent | the check and its three states |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #493".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #493`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Decision

2026-11-23: consolidated all five rows onto `main` with `pod-shape` `v0.82.0`.

## Prove

```sh
cargo test -p podnet --test link_shape
cargo test -p podnet --test link_record --test link_pad --test link_compress --test link_replay
cargo todo check
```

Each row of the table of the issue has a test, and the check passes.

## Done

2026-11-23, consolidated onto `main` with `pod-shape` `v0.82.0`. Padding profiles (`link_record::Shaper`), entropy band and printable tail (`pod_shape::Entropy`), measured first-flight sizing (`src/link_record/measure.rs`), 100 ms window pacing with jittered keepalives (`src/link_record/pace.rs`), and `podnet doctor --shape` (`src/doctor_shape.rs`) pass `tests/link_shape.rs`, `tests/link_record.rs`, `tests/link_pad.rs`, `tests/link_compress.rs`, and `tests/link_replay.rs`, with measured histograms in `tests/data/shape/histogram.toml`.


# T-371: Deferred: shape the application: deferred responses, a generator, carriers in the payload (GitHub #494)

**Source:** GitHub #494 (https://github.com/PodUtils/podnet/issues/494), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #494 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A tunnel that answers a request faster than the origin could is a tunnel. A tunnel that answers from a prepared copy, on the next request, is not. The first is a detector's easiest case, and the second is a change at the application layer with no new cryptography.

| Capability | Status | What is missing |
| --- | --- | --- |
| Deferred response replacement | absent | the cache, the serve-next rule, the overhead bound |
| Traffic generator with an application profile | absent | the generator and its profile files |
| Carriers inside an application's own bytes | absent | the class and its first member |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #494".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #494`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Decision

2026-11-23: consolidated all three rows onto `main` (`src/defer/`).

## Prove

```sh
cargo test -p podnet --test defer
cargo todo check
```

Each row of the table of the issue has a test, and the check passes.

## Done

2026-11-23, consolidated onto `main`. Deferred response cache (`src/defer/cache.rs`), TOML traffic generator and profiles (`src/defer/generator.rs`, `src/defer/profiles/`), and HTTP payload carrier (`src/defer/carrier.rs`) pass `tests/defer.rs`, including measured wire overhead and classifier score checks.


# T-372: Deferred: add a packet sidecar: desync search, disagreement probe, fake TCP, bonding (GitHub #495)

**Source:** GitHub #495 (https://github.com/PodUtils/podnet/issues/495), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #495 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A network that rewrites packets cannot be trusted to carry a flow unmangled, and a network that refuses UDP cannot be trusted to carry a datagram. Both answers need a component that sits below the stream and above the link, and neither exists in the tree.

| Capability | Status | What is missing |
| --- | --- | --- |
| Desync sidecar with a genetic search | absent | the crate, the middlebox simulator, the search |
| Disagreement probe with a third outcome | partial | the probe and its third value |
| Split-and-tamper as one library | absent | the library and one config format |
| Fake TCP datagram channel | absent | the channel and its framing |
| Link bonding | absent | the scheduler |
| Stated limits | absent | the page and the conditions (`podvm/docs/limits.md` covers the chroot, not the carriers) |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #495".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #495`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --features full --test sidecar
cargo todo check
```

The packet sidecar builds behind `--features sidecar` / `--features full` and passes its suite; default builds hold no packet path.

## Done

2026-11-23, consolidated onto `main` with `pod-sidecar` `v0.82.0`. Wired `src/sidecar.rs` behind optional Cargo feature `sidecar` (included in `full`, off by default), covering desync genetic search, three-valued disagreement probe including `FirstInboundRst`, split-and-tamper, fake TCP error handling, link bonding, and stated limits (`docs/limits-carriers.md`). Verified by `tests/sidecar.rs` under `--features full`.


# T-373: Deferred: keep the session end to end: opaque destination and keys from the ends (GitHub #496)

**Source:** GitHub #496 (https://github.com/PodUtils/podnet/issues/496), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #496 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A relay that can read the session is a relay that can be asked for it. The tree already has the end-to-end handshake; the record layer does not use its keys, and the destination is in the request path.

| Capability | Status | What is missing |
| --- | --- | --- |
| End-to-end session authentication under the relay | absent | the dependency and the handshake |
| Opaque destination | absent | the envelope and the identifier |
| Record keys from the ends | absent | the key source |
| A stated table of what each hop learns | absent | the table and the check |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #496".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #496`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --test e2e
cargo todo check
```

End-to-end session authentication, opaque destination envelopes, end-derived record keys with rekey epoch and record ceiling checks, and the hop-exposure table pass.

## Done

2026-11-23, consolidated onto `main` with `pod-e2e` `v0.82.0`. Wired opaque destinations, end-to-end key derivation, rekey epoch and record ceiling assertions (`tests/e2e.rs`, `docs/e2e.md`), and the hop-knowledge table in `SECURITY.md`.


# T-374: Deferred: make the pool live, the budget per deployment, tokens expiring, hosts movable (GitHub #497)

**Source:** GitHub #497 (https://github.com/PodUtils/podnet/issues/497), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

The owner deferred GitHub #497 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A relay pool that is chosen once at startup is a pool of one, whatever it lists. Three defects share one cause: the alternates are read and never re-read, the budget is a compile-time constant, and the reverse road's calls reach one host.

| Capability | Status | What is missing |
| --- | --- | --- |
| Live pool with a health score | partial | the re-read, the score, the attribution |
| Per-deployment budget | absent | the configuration key and the override |
| Tokens with a lifetime | absent | expiry, scope, renewal |
| A deployment that moves hosts | partial | the state file and the pair rule |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #497".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #497`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
grep -l 'GitHub #497' TODO/*.md
cargo todo check
```

Each row of the table of the issue has an entry, and the check passes.

## Blocker

The owner deferred the issue on 2026-11-12 (`TODO/issues.md`). Who acts:
the owner and the relay service operator. What unblocks it: the owner lifts the deferral and the relay service deploys live pool scoring, pair renewal, and token expiry per the H-18 and H-27 notes (`docs/research/podnet/hard-problems-results.md`).


# T-375: Deferred: resume a session across hours, and serve the full service surface at a peer (GitHub #498)

**Source:** GitHub #498 (https://github.com/PodUtils/podnet/issues/498), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

The owner deferred GitHub #498 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A session that survives a ten-minute gap is a session that survives a network hiccup. A deployment on a satellite link or a scheduled radio window is offline for eight to twelve hours by design. The same session layer has to serve both, and it has to survive losing its carrier rather than its connection.

| Capability | Status | What is missing |
| --- | --- | --- |
| Resume measured in hours | partial | the policy value and the bound |
| Carrier handover without a new session | absent | the identity, the sequence, the inherit |
| Degraded-link profile with three carriers | absent | the profile and its derived timeouts |
| Store-and-forward shell | absent | the queue and the collect-by-id client |
| Framing for lossy carriers | partial | the shared layer |
| Full SOCKS5 surface at a peer | partial | `BIND` refused by the ruleset; tests for `UDP ASSOCIATE`, the header cap and the half-close |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #498".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #498`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
grep -l 'GitHub #498' TODO/*.md
cargo todo check
```

Each row of the table of the issue has an entry, and the check passes.

## Blocker

The owner deferred the issue on 2026-11-12 (`TODO/issues.md`). Who acts:
the owner and `podcrates` (`pod-session` multi-hour resume and carrier handover). What unblocks it: the owner lifts the deferral per the H-27 notes (`docs/research/podnet/hard-problems-results.md`).


# T-376: Deferred: carry a session with no network: serial, HF, spool, discovery, pairing (GitHub #499)

**Source:** GitHub #499 (https://github.com/PodUtils/podnet/issues/499), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #499 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A host with no path has a serial port, a radio and a disk. Those three carriers are what a disconnected deployment actually has, and each one needs the session layer to work without a network at all.

| Capability | Status | What is missing |
| --- | --- | --- |
| Serial line as a `--carrier` address | partial | the `serial:` parse arm; the end itself exists in `pipe` |
| AX.25/KISS framing over that line | absent | the framing and the baud-derived timer |
| HF radio carrier | absent | the modem driver and the busy handling |
| Store-and-forward spool | absent | the spool and its checksum |
| Local discovery and one radio interface | absent | the signed announcement |
| Offline bundle | absent | the manifest and the delta update |
| Capture sinks | absent | the record format and the merge |
| Pairing with a short code | partial | `pod-wormhole` and the relay mailbox exist; the code is not wired to the peer identity |
| Replication across a partition | absent | the model and the merge rule |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #499".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #499`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --test offline_serial --test offline_hf --test offline_spool --test offline_discover --test offline_bundle --test offline_capture --test offline_pair --test offline_replica
cargo todo check
```

All eight offline carrier and store test suites pass.

## Done

2026-11-23, consolidated onto `main` with `pod-offline` `v0.82.0`. Added `src/offline/` and `src/cli/cmd/offline/` (`spool`, `discover`, `bundle`, `capture`, `pair` with 8-word PBKDF2 single-use codes and expiry handling, and `replica`), plus the `serial:` and `hf:` carriers of `podnet link` (`docs/offline.md`). Verified by all 8 `tests/offline_*.rs` suites.


# T-377: Deferred: state the host runtime: no open port, classified egress, stated limits (GitHub #500)

**Source:** GitHub #500 (https://github.com/PodUtils/podnet/issues/500), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #500 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
A runtime that opens a port nobody asked for is a runtime that is reachable because it exists. A runtime that classifies its egress before it dials is a runtime whose policy is checkable. Both are configuration and plumbing, and both are prerequisites for putting this on a host that someone else owns.

| Capability | Status | What is missing |
| --- | --- | --- |
| Filesystem expectations stated | absent | the five properties and the mount probe |
| Crash logs safe to send | absent | the hook, the redaction, the bound |
| Measurement from outside | absent | the observer and the file |
| No inbound port by default | absent | the default and the enrolment |
| A forge without one host | absent | the content-addressed mirror |
| CI under an egress policy | absent | the declaration and the refusal |
| A runtime that states its limits | partial | the stated limits beside the existing page |
| Proxy-only egress as a tested invariant | partial | the test |
| A status page derived from the tree | absent | the generation and the staleness check |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #500".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #500`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --test host_runtime --test egress_refusal --test forge --test ci --test init_enrol
cargo todo check
```

The host runtime, egress policy enforcement, forge clone, CI egress check, and init enrolment suites pass.

## Done

2026-11-23, consolidated onto `main`. Added `podnet host probe`, `podnet host stat`, redacted bounded crash logs (`src/crashlog.rs`), deployment egress policy checked before every direct dial (`src/policy.rs`, `tests/egress_refusal.rs`), `podnet init` HTTPS TLS enrolment with rollback (`tests/init_enrol.rs`), `podnet forge clone` content-addressed mirror verification (`src/forge.rs`, `tests/forge.rs`), `podnet ci check` egress declarations (`src/ci.rs`, `tests/ci.rs`), and derived status page (`src/page.rs`, `docs/host-runtime.md`).


# T-378: Deferred: build the verification harness: classifier, exit codes, conformance matrix (GitHub #501)

**Source:** GitHub #501 (https://github.com/PodUtils/podnet/issues/501), a deferred issue.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

The owner deferred GitHub #501 on 2026-11-12. The issue holds work: one
row for each capability. Its first paragraph and its table:

```text
Every claim in this set is measured by something. Without a harness, a classifier model and an exit-code contract, the measurements are one person's run on one day, and nobody can repeat them.

| Capability | Status | What is missing |
| --- | --- | --- |
| Classifier harness in CI | absent | the classifiers and the job |
| Exit codes as one contract | partial | the constant, the help text, the tests |
| Claims with a label and a date | partial | the check |
| Relocatable conformance archive | absent | the matrix and the artefact |
```

The rules, the gate, the dependencies and the relations of the issue are
in `docs/reports/deferred-issues-2026-11-12.md`, section "GitHub #501".

## Premise

Read: the issue on 2026-11-15. Its author is outside the team
(`author_association` NONE), and it has no label `confirmed`. The issue
names reads of the trees of podnet, podcrates and podssh on 2026-11-12;
the record did not check them. Nothing was built or measured for this
entry.

## Approach

1. When the owner lifts the deferral, verify the issue (the rule of
   strangers, `CLAUDE.md`).
2. Make each row of its table an entry of its own, with the Source
   `GitHub #501`, and add the ids to its row in `TODO/issues.md`.
3. Close this entry with the ids of those entries in its Done section.

## Prove

```sh
cargo test -p podnet --test harness_cli
python3 -I scripts/check-claims.py --self-test
python3 -I scripts/check-claims.py
python3 -I scripts/check-harness.py --self-test
cargo todo check
```

The harness, claims gate, exit-code contract, and relocatable conformance archive pass.

## Done

2026-11-23, consolidated onto `main` with `pod-harness` `v0.82.0`. Added `podnet harness` (`src/harness.rs`, `tests/harness_cli.rs`, `tests/data/harness/captures/`), per-road captures, relocatable archive output, 600 s budget refusal, CI harness matrix jobs (`.github/workflows/ci.yml`), `scripts/check-harness.py`, `scripts/check-claims.py`, and the unified exit-code contract (`docs/harness.md`, `docs/exit-codes.md`).
