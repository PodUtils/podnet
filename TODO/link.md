This file holds the entries of the `link` workstream (GitHub #466 to #476), milestone M9: a generic peer-to-peer link for
podnet. The parts are the identity and the session of a peer (E0), the stream carriers (E1), the datagram carriers and the ARQ
(E2), the carriers for DNS, ICMP, WireGuard and NAT traversal (E3), the mesh (E4), the services at a peer (E5), the record layer
(E6), the direction split and the chains (E7), the C ABI (E8), the portable build (E9) and the control plane, the features and
the inventory (E10). There is one entry for each row of the tables of the issues. `docs/decisions.md`, 2026-11-12, "The link
workstream (GitHub #466 to #476)", has the calls of the triage and the asks for `PodUtils/podcrates`. The work order is in
`TODO/PROGRESS.md`. An entry that is blocked on a crate of podcrates is the link side of the take of that crate: it does not
copy the code of the crate (`CLAUDE.md`, Project).

# T-261: link: an Ed25519 key for each install, its fingerprint and an allow list (GitHub #466)

**Source:** GitHub #466 (https://github.com/PodUtils/podnet/issues/466), row E0-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E0-1 of GitHub #466 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Ed25519 key per install, fingerprint, allow list. The status in the issue is "planned". The row is done when: tests/link_identity.rs: mode 0644 exits 71, missing key 66, wrong peer 77.

The rules of the issue that apply to this row:

- The fingerprint is base32 in five groups plus a 4-byte checksum.
- The seed is never printed, never logged and never in `--json`.
- The allow list is default deny. The handshake checks it before any service is offered. It holds 4096 entries at most.

## Premise

- `pod-identity` of `PodUtils/podcrates` v0.28.0 holds the key files, the Ed25519 key and its fingerprint in the OpenSSH form (`SHA256:` prefix), and an allow list (`PodUtils/podcrates:crates/pod-identity/src/allow.rs`, `Allowlist::admits`). podnet does not depend on it: `Cargo.toml` has no `pod-identity` line.
- The OpenSSH form is not the form of the issue (base32, five groups, a checksum). No other form exists in `pod-identity`.
- The exit codes 66, 68, 71 and 77 that the issue uses are not in the table `TABLE` of `crates/podnet-core/src/exit.rs`. The table has 0, 1, 3, 64, 69, 75, 78 and 124.
- A key file with a wide mode is refused today for the WARP key: src/warp/private.rs at `9c1fc4b` tests `mode & 0o077 != 0`. This is the pattern for exit 71.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Take `pod-identity` (T-086, T-137). Do not copy its code.
2. Add the exit codes 66 (`EX_NOINPUT`), 68 (`EX_NOHOST`), 71 (`EX_OSERR`) and 77 (`EX_NOPERM`) to `TABLE` and to `Fault`. T-264, T-266, T-268 and T-274 use them.
3. Keep one key for each install in the state directory, created with mode 0600 on first use. A wider mode exits 71. A missing key, where a command needs an existing one, exits 66. A peer that the allow list refuses exits 77.
4. Show the fingerprint in the form of the issue. Ask podcrates for this form in `pod-identity` (ask A1). Until the answer comes, write the display function in the link crate (T-262) and move it later.
5. Read the allow list from a file: one fingerprint for each line, 4096 lines at most. Check it in the handshake before any service is offered (T-268).
6. Test that `--json`, the logs and `Debug` output never hold the seed.

## Decision

The key store, the fingerprint and the allow list are the work of `pod-identity`, not of podnet (`CLAUDE.md`, Project; `docs/decisions.md`, 2026-11-12, The link workstream). This entry is the link side of the take T-086.

## Prove

```sh
cargo test --test link_identity
sh scripts/agent/check.sh
```

## Start condition

None. `pod-identity` is in podcrates v0.32.0, the tag podnet takes.

## Correction

2026-11-12: the Blocker was wrong. A hand-off patch exists only when podcrates moves podnet code into a crate. `sh scripts/agent/podcrates.sh` shows no block `repo PodUtils/podnet` for `pod-identity`, `pod-session` or `pod-e2e`, so podnet holds no code to replace, and no hand-off comes. The Blocker and the Start condition (T-086) are gone. `pod-identity` v0.32.0 holds the key files (`file::load_in`, `existing_in`), the allow list (`allow::Allowlist`), and the display form (`DisplayFingerprint::{of, parse, matches}`, `PublicKey::display_fingerprint`); ask A1 is answered. The entry is `open`. The allow list of `pod-identity` has no limit of entries and no display-form line: podnet wraps it (a limit of 4096 and a line in the display form) and asks podcrates to take both (ask A11, below).

## Done

2026-11-12, commit b33718d. `Cargo.toml` takes `pod-identity` (tag v0.32.0, version 0.32.0). `src/identity.rs` (`podnet::identity`) is the glue: `create` and `open` keep one key for each install in the state directory (`link-identity.key`, mode 0600, made on first use); a key file with a wider mode exits 71 and is not replaced; `open` with no key exits 66. `Identity::fingerprint` is the display form of `pod-identity` (base32, five groups of eight characters, a checksum). `AllowList` is default deny, holds 4096 entries at most (the 4097th entry exits 78), reads the display form, the `SHA256:` form and the key in hex or base32, and `check` exits 77 for a peer that it does not hold. `AllowList::read` exits 66 for a missing file and 71 for a file that others can change. `Identity` has a `Debug` output with no seed, and `Report` (for `--json`) has no seed. `podnet_core::exit` has the new codes 66 (`no-input`), 68 (`no-host`), 71 (`os-error`) and 77 (`no-permission`) in `TABLE` and in `Fault` (`NoInput`, `NoHost`, `OsError`, `NoPermission`); `docs/cli.md`, `docs/protocol.md` and `docs/library.md` have them. 68 has no user yet (T-264, T-266, T-268 and T-274 use it).
`cargo test --test link_identity`: 13 passed, among them `a_key_with_the_mode_0644_exits_71`, `a_missing_key_exits_66`, `a_wrong_peer_exits_77`, `the_allow_list_holds_4096_entries_at_most`, `the_seed_is_never_shown`, `the_table_of_exit_codes_lists_66_68_71_and_77`. Before the module the test file did not compile (`unresolved import podnet::identity`). With the mode check, the entry limit and the class 77 disabled, 4 tests failed (the mode, the limit, the default deny and the wrong peer).
Not proven: the check in a real handshake before any service (the handshake is T-268; `AllowList::check` is the call it makes); a command that prints the fingerprint (T-265); the write of the allow list by a command. Ask A11 for podcrates: `pod-identity::allow::Allowlist` takes a limit of entries and the display form on a line, so that podnet drops its wrapper.


# T-262: link: the session of streams with credit flow control and keepalive (GitHub #466)

**Source:** GitHub #466 (https://github.com/PodUtils/podnet/issues/466), row E0-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

Row E0-2 of GitHub #466 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Session: streams, credits, keepalive, 10-minute resume (this entry: streams, credits and keepalive; the resume is T-263). The status in the issue is "planned". The row is done when: tests/link_session.rs: 300 echoes, a stalled reader isolated, 1 MiB resume digest equal (the digest part is T-263).

The rules of the issue that apply to this row:

- The dialer uses odd stream ids. The listener uses even ids.
- The initial window is 16 KiB. The maximum window is 8 MiB.
- A session has 256 streams by default and 4096 at most.
- A record is 1 MiB at most before the carrier fragments it.
- Flow control is per stream and per session. A stream that stops reading cannot starve another stream.

## Premise

- No multiplexed session exists in the tree. The connection types are `Dial` and `BoxedIo` (`crates/podnet-core/src/network.rs`), one stream for each connection.
- `pod-session` of `PodUtils/podcrates` v0.28.0 is a resume layer for one byte stream: offsets, a secret and proofs (`PodUtils/podcrates:crates/pod-session/src/lib.rs`). It has no stream ids and no credits. No crate of podcrates holds a multiplexer (`PodUtils/podcrates:docs/design.md` lists none).
- podnet has 5 crates and a layer check (`scripts/check-layers.py`). A link crate is not in the plan.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Send the COORD ask A2 to podcrates: does podcrates want the multiplexer, the carriers and the ARQ in a shared crate (`pod-link`)? Write the answer into this entry as a Correction.
2. Without a shared crate: add the crate crates/podnet-link above `podnet-core` and below `podnet`, and add it to `scripts/check-layers.py`.
3. Write the frame codec and the stream table: odd and even ids, the limits above, `OPEN`, `DATA`, `WINDOW`, `CLOSE`, `PING`.
4. Write the credit windows for each stream and for the session. Test that one stalled reader does not stop 299 other echoes.
5. Write the keepalive. Give the session API (`open`, `accept`, `close`) that the C ABI uses (T-304).
6. Add docs/link.md for the topic, and a row in the map of `README.md`.

## Decision

2026-11-12, a call of the lead: podnet owns the multiplexer until podcrates answers ask A2 (`docs/decisions.md`, The link workstream). The owner can rule otherwise.

## Prove

```sh
cargo test -p podnet-link
cargo test --test link_session
sh scripts/agent/check.sh
```

## Start condition

None. Send ask A2 first. The resume part waits for T-263.

## Correction

2026-11-12: podcrates answered ask A2 (PodUtils/podcrates#88): `pod-link` is a crate of podcrates. No local crate podnet-link. The entries of this file that need the multiplexer, a carrier or the ARQ are takes of `pod-link`.

2026-11-12: podnet started the workstream (GitHub #480, COORD-049): `pod-link` with the unix carrier (T-274) and the memory carrier (T-275) first.

2026-11-12: podcrates v0.32.0 (PodUtils/podcrates#93) has `pod-link`: `Link::new(carrier, Role, Settings)`, `open`,
`accept`, `close`; `Stream` (`AsyncRead`, `AsyncWrite`, `reset`); the `Carrier` trait with `available()`; the unix and
memory carriers (calls C120 to C124). podnet takes it; the Blocker is gone. A link over a session of `pod-session`
crosses the loss of its carrier (`dead_after` takes the `resume_deadline`).

2026-11-12: the Approach is a take, not a local crate. Steps 1 to 3 and the session API (step 5) are `pod-link`. Left: the topic document of the link and the row in the map of `README.md` (step 6). They come with T-265, the first user-facing command. Left: the C ABI (T-304).

## Done

2026-11-12, commit 69c9c7f. `Cargo.toml` takes `pod-link` (tag v0.32.0). `src/link.rs` (`podnet::link`) is the glue: `dial`, `listen`, `Door::accept` and `from_io` open a link over a `Carrier` (the carrier says first with `available()` whether the host can use it), and `fault` and `carrier_fault` give each failure the exit class of podnet (64 for a bad value, 69 for an unavailable path, 75, 78 and 124 for the matching classes). `scripts/check-layers.py` puts the module in the layer `podnet`. No frame, window or keepalive code is in podnet.
`cargo test --test link_session`: 6 passed: `three_hundred_echoes_cross_one_link` (300 streams of 40 KB or more over one link), `a_stalled_reader_does_not_stop_the_other_echoes` (a write of 8 MiB to a stream that is not read waits; 299 echoes finish), `the_dialer_has_odd_ids_and_the_listener_even_ids`, `a_silent_peer_ends_the_link_as_dead`, `a_live_peer_keeps_the_link_and_answers_the_ping`, `a_setting_out_of_its_limit_is_a_usage_error` (exit 64). Before the glue the test file did not compile (`unresolved import podnet::link`).
Not proven: the 1 MiB resume digest (T-263); the docs; a link over a real carrier at the limit of 4096 streams (the limits are proven in the tests of `pod-link`).
2026-11-12, commit 4bc2fa7 (the parts that waited for T-265). The topic document `docs/link.md` and its row in the map of `README.md` (and in the list of documents of `CLAUDE.md`) are written. The link has its user: `podnet link listen` and `dial` run a link over a carrier, and one stream carries the bytes of `--to`. `cargo test --test cli link::`: 19 passed (see T-265), and `cargo test --test docs`: `the_readme_is_short_and_links_each_topic_document` failed ("the README does not link docs/link.md") before the row and passes with it.
Not proven: the C ABI (T-304 has it); a link at the limit of 4096 streams over a real carrier.


# T-263: link: the 10-minute resume of a session through pod-session (GitHub #466)

**Source:** GitHub #466 (https://github.com/PodUtils/podnet/issues/466), row E0-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E0-2 of GitHub #466 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Session: 10-minute resume (the part of E0-2 that `pod-session` holds). The status in the issue is "planned". The row is done when: tests/link_session.rs: 1 MiB resume digest equal; the resume is refused after 10 minutes with `ResumeExpired`.

The rules of the issue that apply to this row:

- Resume works within 10 minutes. After that it is refused with `ResumeExpired`.
- The record layer has its own RESUME record with a single-use token (T-301). The two parts use one token and one window.

## Premise

- `pod-session` has the resume: offsets that never reset, a 256-bit secret and the proof on a new link (`PodUtils/podcrates:crates/pod-session/src/resume.rs`, `offset.rs`, `secret.rs`). podnet does not depend on it.
- T-087 uses the same layer for the relay node. The relay road stays in T-087.
- The layer carries one byte stream. The link session carries many streams (T-262), so the layer sits under the multiplexer.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Take `pod-session` (T-086, T-137).
2. Put the layer between the carrier and the multiplexer: a lost carrier keeps the stream table for 10 minutes, and a new carrier resumes it.
3. Ask podcrates (ask A2) that the window (10 minutes) and the use of the token (once) are settings.
4. Refuse a resume after the window with `ResumeExpired`. Test with a fake clock.

## Decision

The resume belongs to `pod-session`. This entry is the link side of the take T-086.

## Prove

```sh
cargo test --test link_session -- resume
sh scripts/agent/check.sh
```

## Start condition

T-262 is done (the link over a carrier).

## Correction

2026-11-12: the Blocker was wrong. A hand-off patch exists only when podcrates moves podnet code into a crate. `sh scripts/agent/podcrates.sh` shows no block `repo PodUtils/podnet` for `pod-session`, so no hand-off comes. `pod-session` v0.32.0 is in the tag that podnet takes. `Settings::resume_deadline` is the window (600 s, `resume::RESUME_DEADLINE`). Ask A2, part 1, is answered: map `REFUSE` `UNKNOWN_SESSION` to `ResumeExpired`. A link over the application side of a session crosses the loss of its carrier when `dead_after` of the link takes the `resume_deadline`. The Blocker and the Start condition (T-086) are gone. The entry is `open`.

2026-11-12: podcrates answered A13 (PodUtils/podcrates#101): no setting. Each resume proves the secret with an HMAC over both fresh nonces of the new link, so a proof works once, on its link, and a replayed proof is refused (`BAD_PROOF`). `Settings::resumes = false` ends a session with its link. A12 (a typed cause in `End::GaveUp`) is in work; podnet then drops its match on the text.

## Done

2026-11-12, commit 75b49ba. `Cargo.toml` takes `pod-session` (tag v0.32.0, version 0.32.0). `src/session.rs` (`podnet::session`) is the glue. `Session::dial` starts a session on a first carrier and runs the link of T-262 over its application side; `connect` gives a new carrier after each loss. `Far::accept` takes each carrier: a new session gives a `Link`, a resume joins its session. `RESUME_WINDOW` is `pod-session`'s `resume_deadline`, 600 s, at both sides (`session::settings`). `link_settings` sets `dead_after` of the link to the window, so that the link does not end as dead before the window ends. After the window, `Session::finish` returns `ResumeExpired` (exit 69) for a `GaveUp` of `pod-session` ("no such session" at the far end, or "no new link within N s" at the dialer) and for a `REFUSE` `UNKNOWN_SESSION`. `scripts/check-layers.py` puts `identity` and `session` in the layer `podnet`. The clock of the far end is the `now` of `Far::expire_at`, as in `pod-session`.
`cargo test --test link_session -- resume`: 4 passed: `resume_inside_the_window_carries_one_mebibyte_with_an_equal_digest` (a carrier cut at 300 KB, 5 s before the end of the window the far end keeps the session, the resume carries 1 MiB each way with an equal SHA-256), `resume_after_the_window_is_refused_with_resume_expired` (the far end clock runs 601 s, the far end forgets the session, the resume gives `ResumeExpired` with window 600 s, exit 69), `resume_expires_at_the_client_when_no_carrier_comes` (window 2 s), `the_window_is_ten_minutes_and_the_link_outlives_it`. Before the module the test file did not compile (`unresolved import podnet::session`). With `expired()` returning `None` and `dead_after` unchanged, 3 of the 4 tests failed.
Not proven: a resume over a real carrier (the tests use duplex streams); the single-use token (the record layer, T-301); a CLI command (T-265). Ask A12 for podcrates: a typed cause in `End::GaveUp` (podnet reads its text now); ask A13: the single-use of the secret as a `Settings` value. One observation: when the dialer application does not read a stream, the transfer stopped after about 64 KiB in a test (the test reads, and passes); this is not analysed (T-262 proves a stalled reader for the link alone).


# T-264: link: a registry of carriers with an available() reason (GitHub #466)

**Source:** GitHub #466 (https://github.com/PodUtils/podnet/issues/466), row E0-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E0-3 of GitHub #466 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Carrier registry with an `available()` reason. The status in the issue is "partial". The row is done when: tests/link_registry.rs: unknown name 64, disabled carrier 69 with reason.

The rules of the issue that apply to this row:

- Each carrier reports `framing`, `duplex`, `mtu`, `cost` and `available(reason)`. `link carriers` and `doctor` print the diagnosis, not "unknown carrier".
- A carrier declares its addressing mode. A transport that is bound to a peer secret does not receive an arbitrary destination (the same reason as GitHub #377).
- An unknown name exits 64. A disabled carrier exits 69 with its reason.

## Premise

- The registry of networks exists: `src/networks.rs` holds six rows in `ROWS` (line 46 at `505dfa0`). It has no carrier kind.
- The type `Addressing` (`Destination` or `Peer`) exists in `crates/podnet-core/src/network.rs:65`. A carrier uses `Peer`.
- The type `Check` (`crates/podnet-core/src/network.rs:256`) is the row of `podnet doctor` for a network.
- The issue marks this row "partial" for these parts. No carrier trait exists.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add the carrier trait and a static table in the link crate (T-262), in the form of `ROWS`: name, kind, framing, duplex, ordered, mtu, cost class, needs-privilege, build feature (T-311).
2. Write `available() -> Result<(), Reason>` for each carrier. A carrier that is not built, or is not allowed on this host, gives a named reason.
3. Map an unknown name to exit 64 with the list of the names. Map a disabled carrier to exit 69 with its reason.
4. Add the carrier rows to `doctor` through `Network::checks` style rows.

## Prove

```sh
cargo test --test link_registry
sh scripts/agent/check.sh
```

## Start condition

T-262 has the crate.

2026-11-12: the registry is `src/carriers.rs` (`podnet::carriers`). The trait and `available()` are in `pod-link` (podcrates call C117); the order, the flags and the lines are podnet's. Left: the rows of `podnet doctor` (step 4) and the command `podnet link carriers`, which print `carriers::report`. They come with T-265. The field `feature` of a row is `None` until T-311.

## Done

2026-11-12, commit 528235c. `carriers::ROWS` has the rows `unix` and `memory` in this order (name, about, address form, cost, privilege, feature, maker). `carriers::open(name, &Options)` exits 64 for an unknown name (the text lists the names) or a bad option, and 69 with the reason of `available()` for a carrier that the host refuses. `carriers::report` gives one `Report` for each carrier (the facts of `pod-link`, the addressing `Peer`, the diagnosis, and `line()`).
`cargo test --test link_registry`: 7 passed, among them `an_unknown_name_exits_64_and_lists_the_names`, `a_disabled_carrier_exits_69_with_its_reason` (a stub carrier that the host refuses), `the_report_gives_the_reason_of_a_disabled_carrier`. Before the module the test file did not compile (`unresolved import podnet::carriers`).
Not proven: a real host that refuses `AF_UNIX` (the 69 path is proven with a stub carrier); the doctor rows.
2026-11-12, commit 4bc2fa7 (the parts that waited for T-265). `podnet link carriers [--json]` prints `carriers::report` (one line for each carrier, with `available` or `unavailable:` and the reason). `podnet doctor` has a section `carriers` in its text and the key `carriers` in its JSON (one object for each carrier: `carrier`, `verdict`, `status`, `detail`; `docs/doctor.md`, `docs/protocol.md`). `cargo test --lib doctor::`: 30 passed, among them `the_doctor_has_a_row_for_each_carrier_with_its_reason` and `the_report_lists_the_carriers_of_the_registry` (before the module `carrier_check`, the test module did not compile: `cannot find module carrier_check`). `cargo test --test cli link::carriers_lists_each_carrier_with_its_diagnosis` and `a_ring_out_of_its_limit_is_a_usage_error_and_a_reason`: pass. `tests/doctor.rs` `doctor_json_keys_are_fixed` has the key `carriers` (it needs a loopback host).
Not proven: a real host that refuses `AF_UNIX` (the doctor row of a refusal is proven with a report built by hand).


# T-265: link: the commands listen, dial, status, close and carriers (GitHub #466)

**Source:** GitHub #466 (https://github.com/PodUtils/podnet/issues/466), row E0-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E0-4 of GitHub #466 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: `podnet link listen`, `dial`, `status`, `close`, `carriers`. The status in the issue is "planned". The row is done when: tests/cli/link.rs: a unix listener prints its 0600 path, a duplicate listener exits 69.

The rules of the issue that apply to this row:

- `link listen` prints the unix path and its mode.
- `--detach` uses the existing background record, so `ps` and `logs` keep working. Do not add a second record.
- The manual lint in `tests/cli/manual.rs` must cover every new command and flag.

## Premise

- The tree has no `link` or `identity` command (`src/cli/cmd/mod.rs`).
- The background record is `crates/podnet-core/src/detach.rs` (`register`, `update_record`) and `crates/podnet-core/src/instances.rs` (`Record`).
- `tests/cli/manual.rs` checks that each command and flag is documented (`the_man_page_names_each_flag`).
- `bind_private` in `crates/podnet-netns/src/listen.rs:127` binds a unix socket at mode 0600.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add the command `podnet link` with `listen`, `dial`, `status`, `close` and `carriers` in `src/cli/cmd/`.
2. `listen` binds with `bind_private` and prints the path and the mode. A second listener on a live path exits 69.
3. `--detach` calls `detach::register`. Test `ps` and `logs`.
4. Add the rows to `src/manual/facts.rs` and the topic document docs/link.md. Add the command to `docs/cli.md` and to the map of `README.md`.

## Prove

```sh
cargo test --test cli link
cargo test --test cli manual
sh scripts/agent/check.sh
```

## Start condition

T-261, T-262 and T-264 are done, and one carrier exists (T-274 or T-275).

## Correction

2026-11-12: the start condition is met in this form (a call of the lead). T-261 is done (`src/identity.rs`), T-275 is done (the memory carrier), and T-262, T-264 and T-274 were partial only because their CLI and doctor parts wait for this entry. The entry builds those parts. The peer handshake of the TCP carrier (T-268) does not exist, and the carriers `unix` and `memory` have no peer key. So this entry adds the hello (`src/hello.rs`): the first stream of a link carries a challenge in which each end signs a transcript with its Ed25519 key, so that the allow list of T-261 is checked before any service. The TLS handshake of T-268 stays its own work. The control socket and the live counters of `status` are T-267: until then `status` reads the background record.

## Done

2026-11-12, commit 4bc2fa7. `podnet link` has `listen`, `dial`, `status`, `close` and `carriers` (`src/cli/cmd/link/`).
* `listen ADDRESS --allow FILE [--to TARGET] [--json] [--detach ...]` checks the carrier (64, 69), the target (64) and the allow list (66, 71, 78) before it binds, makes the key of the install on first use, binds, and prints the address, its mode (`0600` for the unix carrier) and the fingerprint. A second listener at a live address exits 69. For each link it runs the hello, names a peer that it refuses on stderr, and sends each stream of an admitted peer to `--to` (`host:port` or `unix:/path`). The ready document has `carrier`, `address`, `listen`, `mode`, `fingerprint`, `allow` and `to`. `--detach` and `--name` use the background record, so `ps`, `logs`, `wait` and `stop` work.
* `dial ADDRESS (--peer FINGERPRINT | --allow FILE) [--check [--json]]` runs the hello and checks the key of the listener (exit 77 for a listener that the list does not hold; exit 64 with neither flag). It copies stdin and stdout over one stream, or with `--check` prints the fingerprint of the peer.
* `status [NAME] [--create] [--json]` shows the fingerprint and the link instances; `close NAME` stops a link instance (a name that is no link instance exits 64, none that runs exits 69); `carriers [--json]` prints one line for each carrier of the registry with `available()` and its reason.
* `src/hello.rs`: the hello of a link (dialer and listener each sign a transcript; the listener checks its allow list before any service, and the dialer checks the key of the listener against its list). `AllowList::check` gives exit 77 at both ends.
* The seed is in no output: `status --json`, the ready document, the `Debug` form and the errors.
`cargo test --test cli link::`: 19 passed, among them `a_unix_listener_prints_its_0600_path`, `a_duplicate_listener_exits_69`, `a_dialer_in_the_allow_list_reaches_the_service_of_the_listener` (bytes through a far unix origin), `a_dialer_that_the_list_does_not_hold_exits_77`, `a_listener_that_the_dialer_does_not_trust_exits_77`, `a_detached_listener_is_in_ps_logs_status_and_close`, `carriers_lists_each_carrier_with_its_diagnosis`, `status_shows_the_fingerprint_and_never_the_seed`. `cargo test --test link_hello`: 6 passed. Before the change, 18 of the 18 tests of the first version failed (`error: unrecognized subcommand 'link'`, exit 64), and `link_hello` did not compile (`unresolved import podnet::hello`). The first run found two defects that the tests then kept out: a SIGTERM that comes right after the ready line killed the listener (the stop is armed before the line is printed now), and a refused dialer saw "the carrier ended" (exit 69) in place of 77 (the listener closes the link, so the verdict byte is sent first). `sh scripts/restricted-host.sh --test link_hello --test link_unix --test link_memory --test relay_listen --test cli`: 11 tests skipped (bind(AF_INET) refused), every test passed, among them the 19 above.
Not proven: the hello over a real TCP carrier (T-268 has its own handshake); the live counters of a link and the control socket (T-267); a peer on another host; `--detach` with the supervisor restart after a crash of the listener.


# T-266: link: `peer:FINGERPRINT/SERVICE` as a network target and `forward -L LISTEN=peer:...` (GitHub #466 and #471)

**Source:** GitHub #466 (https://github.com/PodUtils/podnet/issues/466), row E0-5; GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E0-5 and E5-4 of GitHub #466 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: `peer:<fp>/<service>` as a network target (E0-5), and forward a local port to a peer service (E5-4). The two rows are one work item: the forward is the test of the target.. The status in the issue is "partial". The row is done when: tests/link_network.rs: forward through a peer to a far unix origin; tests/link_forward_peer.rs: one stream for each accepted connection.

The rules of the issue that apply to this row:

- `peer:<fingerprint>/<service>` resolves to one stream.
- It is refused with exit 77 before the service opens, when the peer is not allowed.
- `podnet forward -L 8080=peer:<fp>/http` (or `--name`) opens one stream for each accepted connection.

## Premise

- `podnet forward` maps `-L LISTEN=HOST:PORT` and takes `--network SPEC` (`src/cli/cmd/forward.rs:26`). The registry has no peer kind (`src/networks.rs:50`).
- `Addressing::Peer` exists (`crates/podnet-core/src/network.rs:65`): the host of the target names a peer and the port is 0.
- T-099 covers `podnet forward` for the networks of the registry. It does not cover a peer target.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add the scheme `peer` to the registry (`src/networks.rs`) with `Addressing::Peer`. The dialer opens one session stream for the service (T-262).
2. Check the allow list (T-261) before the `OPEN` of the service. A refusal exits 77.
3. Let `forward -L LISTEN=peer:FP/SERVICE` and `--name` use the target. Open one stream for each accepted connection.
4. Test a forward through a peer to a far unix origin, on the memory carrier (T-275) and the unix carrier (T-274).

## Prove

```sh
cargo test --test link_network
cargo test --test link_forward_peer
sh scripts/agent/check.sh
```

## Start condition

T-261, T-262 and T-265 are done.

## Correction

2026-11-12: the entry says that the dialer "opens one session stream for the service", but no process held a link for another process to use, and the hello binds no service. The entry builds both. (1) The first line of a stream names the service (`PNSVC1 NAME`), and the other end answers with one byte before any data (`src/link_service.rs`; `link listen --service NAME=TARGET`, `--to` is the service `raw`; `link dial --service NAME`). (2) `podnet link connect` holds a link as the dialer and runs the control socket of T-267. The control socket has the verbs `peer` (check the link) and `open` (the connection becomes the bytes of one stream); they are the way for `forward`, which is another process, to reach the link. These two verbs are not the write verbs of T-307. (3) `peer:FINGERPRINT/SERVICE` is a peer way (`Addressing::Peer`). A consumer of destinations refuses it (`forward --network peer:...` exits 64), so `forward` takes the form `-L LISTEN=peer:FINGERPRINT/SERVICE`. The form `--name` of the issue needs names for live links (T-308). (4) The instance is found by `PODNET_LINK_CONTROL` (one socket), else by the `control` of the ready document of each link instance in the run directory.

## Done

2026-11-12, commits fa84e60 and 4ec4c5d. The registry has the scheme `peer` (`src/networks.rs`, `src/networks/peer.rs`; form `peer:FINGERPRINT/SERVICE`; the fingerprint is any form that an allow list takes; a bad spec exits 64; `peer:PORT` stays the SOCKS5 hop of a host named peer). `up()` asks a running instance to check the link first: a link to a peer that is not the fingerprint exits 77 and no stream opens; no instance, or no link that is up, exits 69. A dial opens one stream through the control socket and asks the far end for the service; a service that the far end lacks or whose origin does not answer is a failure of that dial (69 class) and the link stays up. `podnet forward -L LISTEN=peer:FINGERPRINT/SERVICE` (TCP or `unix:` listener) opens one stream for each accepted connection; the check runs before the listener binds. `podnet link connect`, `link listen --service`, `link dial --service`.
`cargo test --test link_network`: 4 passed (`a_peer_network_reaches_a_far_unix_origin_and_refuses_a_stranger_with_77` over the memory carrier, `a_peer_spec_names_the_fingerprint_and_the_service`, `a_bad_peer_spec_is_a_usage_failure`, `the_registry_has_a_row_for_the_scheme_peer`). `cargo test --test link_forward_peer`: 8 passed over the unix carrier, among them `a_forward_through_a_peer_reaches_a_far_unix_origin`, `one_stream_opens_for_each_accepted_connection` (5 connections at one time, the counter `opened` is 5, then 6), `a_peer_that_is_not_the_named_fingerprint_exits_77_before_it_listens`, `no_instance_that_runs_exits_69`, `a_service_that_the_peer_lacks_closes_the_connection_and_the_forward_stays`, `a_tcp_forward_reaches_a_service_of_a_peer` (it needs a loopback and prints a named skip without one). `cargo test --lib forwards::`: 7 passed, among them `a_rule_to_a_peer_keeps_the_spec_and_the_port_0_target`. Fail first: before the registry row, 3 of the 4 tests of `link_network` failed (no row `peer`; the spec parsed as a SOCKS5 hop; no way); before the change of `forward`, 6 of the 8 tests of `link_forward_peer` failed (`-L ...=peer:...` gave "give HOST:PORT after =").
The full run of `cargo test --workspace --no-fail-fast` found one more defect, and the test now keeps it out: a fingerprint in the SSH form is base64 and can hold `/`, so the first `/` of `peer:FINGERPRINT/SERVICE` cut the fingerprint (the test failed for a random key). The last `/` starts the service now (`a_peer_spec_names_the_fingerprint_and_the_service` uses a fixed fingerprint with a `/`).
Not proven: a peer over a carrier that crosses a network (T-268); the form `--name` (T-308); a second instance that holds a link to the same peer (the first that answers wins); the services of E5 beyond `--service` (T-291 to T-297); a stream that the listener opens to the connector (the connector closes it).


# T-267: link: the control socket, its discipline and the status verb (GitHub #466)

**Source:** GitHub #466 (https://github.com/PodUtils/podnet/issues/466), row E0-6.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E0-6 of GitHub #466 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Link control socket, `status` and `dump` (this entry: the socket and `status`; the dump is T-309; the write verbs are T-307). The status in the issue is "partial". The row is done when: tests/link_ctl.rs: socket 0600, bad token 77, no seed in JSON.

The rules of the issue that apply to this row:

- The socket follows the discipline of the router: mode 0600, a random token for each run, `SO_PEERCRED`.
- It carries one JSON object for each line, not the HTTP paths of the router.
- Limits: a request is 4 KiB, a response is 64 KiB, 16 clients, one command at a time for each object. There is no TCP listener and no HTTP endpoint by default.

## Premise

- The router has a control socket: `src/router/control.rs:300` (`peer_allowed`), `:331` (`peer_cred`) and `:371` (it answers "HTTP/1.0" with JSON bodies).
- The socket is bound with `bind_private` (`crates/podnet-netns/src/listen.rs:127`).
- The issue marks this row "partial" for these parts. No link socket exists.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the line-JSON server in the link crate: bind with `bind_private`, a token of 32 random bytes for each run, the `SO_PEERCRED` check, the limits above.
2. Add the verb `status` (the links, the carrier, the peer fingerprint, the counters). Never put a seed or a token in the JSON.
3. Make the other verbs plug in (T-307, T-309).
4. A bad token exits 77.

## Prove

```sh
cargo test --test link_ctl
sh scripts/agent/check.sh
```

## Start condition

T-262 is done.

## Correction

2026-11-12: the control socket lives in the root crate (`src/link_ctl.rs`, `src/link_ctl/client.rs`), not in a link crate: T-262 built the link in `podnet::link` over `pod-link`. The socket also has the verbs `peer` and `open`, which T-266 needs so that `forward` (another process) can reach a link; the write verbs (T-307) and `dump` (T-309) plug in at `Hub::verb`. The token is in `PATH.token` (mode 0600), not in the ready document or in `ps`. A host with no run directory runs with no socket unless `--control PATH` is given.

## Done

2026-11-12, commits fa84e60 and 4ec4c5d. `podnet link listen` and `link connect` run the control socket (`--control PATH`, default `link-PID.ctl` in the run directory): mode 0600 through `bind_private`, a token of 32 random bytes in hex for each run in `PATH.token` (mode 0600), the uid check of the router (`peer_allowed`: the user of the instance, or root), one JSON object for each line, a request of 4 KiB at most, an answer of 64 KiB at most, 16 clients (the 17th gets code 69), no TCP listener, no HTTP. The verb `status` gives the fingerprint, the pid and a row for each held link (carrier, address, role, peer fingerprint, state, streams, rtt, opened, accepted, since); it holds no seed and no token. A bad or missing token gets code 77 and the client exits 77. `podnet link status` reads the socket of each instance that has one and shows it under `live`; when the socket does not answer it shows the background record (`live: null`, `live_error`); `--control PATH [--token-file FILE]` asks one socket.
`cargo test --test link_ctl`: 11 passed, among them `the_socket_and_the_token_file_have_mode_0600`, `a_bad_or_missing_token_gets_77_and_a_good_one_gets_status`, `the_client_maps_a_bad_token_to_exit_77`, `a_user_that_the_gate_refuses_gets_77_with_the_right_token`, `a_request_over_4_kib_is_refused_and_a_bad_line_gets_64`, `the_seventeenth_client_is_refused_and_a_closed_one_frees_its_place`, `status_shows_a_held_link_its_peer_and_its_counters_and_no_seed`, `the_listener_runs_a_control_socket_that_status_reads` (a detached listener with a live link; a wrong token file exits 77) and `status_falls_back_to_the_record_when_the_socket_is_gone`. Fail first: before the module the test file did not compile (`unresolved import podnet::link_ctl`); before the change of the commands, the two tests of the commands failed (the ready document had no `control`, and `status` had no `live`). `cargo test --test cli link::`: 19 passed as before.
Not proven: a client of another uid (the gate is proven with a closure that refuses every uid, and `peer_allowed` is the function of the router); a response that exceeds 64 KiB (the code returns the error answer; no test builds that many links); the verbs of T-307 and T-309.


# T-268: link: the TCP carrier with TLS 1.3, the accept side and the peer handshake (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

Row E1-1 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: TCP with TLS 1.3. The status in the issue is "partial". The row is done when: tests/link_tcp.rs; a test that needs TCP loopback prints a named skip with the host error.

The rules of the issue that apply to this row:

- TLS 1.3 with ALPN `podnet/1`. The certificate carries the peer key. The dialer checks it against the expected fingerprint, not a public CA.
- Plaintext only on loopback or on a unix socket. A public bind with `--plaintext` exits 68.
- One session for each carrier connection. The session multiplexes the streams inside.
- The carrier declares its addressing mode (GitHub #377).

## Premise

- The dial side exists: `src/dialers/direct.rs` and the egress registry (`src/networks.rs:50`). The accept side, the peer handshake and the wrapper are absent.
- `pod-tls` makes only the verifier of the web PKI. A caller brings its own configuration with `Trust::caller`. Its module `mutual` pins the SHA-256 of a P-256 SubjectPublicKeyInfo, not an Ed25519 identity (`PodUtils/podcrates:crates/pod-tls/src/mutual.rs`).
- On the test host, TCP loopback and IPv6 loopback are denied with `EACCES` 13 (measured by the issue on 2026-11-12). UDP loopback and `AF_UNIX` are allowed. The helpers of `tests/test_host/mod.rs` skip a test on such a host.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Ask podcrates (ask A7) whether `pod-tls` can pin an Ed25519 identity, or a self-signed certificate that the identity key signs. If it can, take it. If not, write the verifier in the link crate with `Trust::caller`.
2. Write the acceptor and the connector with ALPN `podnet/1`, and the `Hello` of the peer handshake (suite, limits). Check the allow list (T-261) before any service.
3. Refuse `--plaintext` on a public bind with exit 68.
4. Test on a host with TCP loopback. On a host without it, print the named skip.

## Prove

```sh
cargo test --test link_tcp
sh scripts/agent/check.sh
```

## Start condition

T-261, T-262 and T-264 are done. Ask A7 is answered.

## Correction

2026-11-12, the lead: the TCP carrier does not reuse the hello of T-265 (`src/hello.rs`) as its proof: the hello does not bind the streams after it. The pinned Ed25519 key goes into the TLS handshake of the carrier.

2026-11-12, the build of the entry: ask A7 is answered by `pod-link` of podcrates v0.34.0 (feature `tcp`): it pins an Ed25519 key in TLS 1.3 (`pod_tls::peer`). Podnet writes no verifier. The ALPN is `podlink/1` (the crate's ALPN replaces `podnet/1` of the Problem). The hello of `src/hello.rs` does not run on this carrier. There is no plaintext mode (the crate always runs TLS), so the rule "a public bind with `--plaintext` exits 68" has no case, and the commands have no `--plaintext` flag. Not done: the `Hello` of suite and limits that Approach step 2 names (the TLS handshake has no such field; T-265 settings stay as they are); the session over the carrier (T-263 resume) is not wired for `tcp` in the commands; the carrier takes no `--proxy` flag of its own (it uses the way out of the process).

## Done

2026-11-12, commits e146f31 and the commit after it (the registry tests). Feature `tcp` of podnet is `pod-link/tcp` (`Cargo.toml`; `tcp` is in `OPTIONAL` and `FULL` of `src/build_features.rs`, and in `full`). `src/link_tcp.rs` (`podnet::link_tcp`) has `carrier(me, allow)` (the key of the install, the client keys of the listener from the allow list, the way out of the process), `listen`, `dial` (pins `KEY` of `KEY@HOST:PORT`, then checks the proved key against the allow list) and `Probe` (the registry row: facts and the host check, no key). `AllowList::names` feeds `with_clients`. `link::Door::accept_peer` gives the key that the carrier proved; `link::carrier_fault` maps a refused key (the pin of `pod_tls::peer`, or the alert of a refused certificate) to exit 77, and a denied socket (`EACCES`) stays 69. `link listen`, `link dial` and `link connect` take `--carrier tcp`: the TLS handshake proves the keys and the hello does not run. The registry has the row `tcp`. `docs/link.md` (The TCP carrier, the exit table, the library), `docs/build.md` and the help text have it.

Tests (`tests/link_tcp.rs`, build `--features tcp`; this host allowed TCP loopback, so none skipped): `the_allow_list_feeds_the_client_keys_in_each_form`, `the_carrier_is_a_stream_and_says_why_a_host_cannot_use_it`, `the_registry_row_has_no_key_and_refuses_a_dial`, `a_dial_address_with_no_key_exits_64`, `a_refused_key_maps_to_77_and_a_denied_socket_does_not`, `the_dial_of_the_cli_with_no_key_in_the_address_exits_64` (no socket); `a_link_crosses_tcp_and_each_end_learns_the_key_of_the_other`, `a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open`, `a_listener_with_another_key_than_the_pinned_one_exits_77`, `a_listener_that_the_dialer_list_does_not_hold_exits_77`, `a_change_of_the_client_keys_applies_to_the_next_handshake`, `a_peer_that_speaks_no_tls_is_not_a_link`, `link_dial_check_meets_link_listen_over_tcp_and_a_stranger_exits_77` (TCP loopback, with the named skip). With the 77 mapping disabled, 5 of these 13 fail. `tests/link_features.rs`, `tests/link_registry.rs` and `tests/cli/link.rs` list the row.

Not proven: a host that denies TCP loopback (the skip line is the host's own text; the denied case is tested with a built `EACCES` error); a link over a proxy chain; the resume of a session over `tcp`; the TLS limits (handshake count and time) that `pod-link` tests itself.


# T-269: link: the WebSocket carrier, its server half and its wrapper (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E1-2 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: WebSocket. The status in the issue is "partial". The row is done when: tests/link_ws.rs.

The rules of the issue that apply to this row:

- Binary frames only. The subprotocol is `podnet/1`. Ping every 20 s, pong within 10 s.
- A text frame is a protocol error.
- The tests exercise the egress paths that the restricted CI jobs cannot run, so a test prints a named skip where it cannot.

## Premise

- `src/ws.rs` has `serve_websocket` (`src/ws.rs:95`), `dial` (`src/ws.rs:151`) and `bridge`. It is the thin wrapper of `pod-ws` (taken at 5eace6e). `pod-ws` has the client, the server (`PodUtils/podcrates:crates/pod-ws/src/server.rs`), the limits and the bridge.
- The relay uses the client in `src/ws_relay.rs` (frozen: it goes to `pod-relay`).
- No carrier wraps a WebSocket session as a link carrier.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the carrier on `pod-ws`: the server half with the subprotocol `podnet/1`, the client with `pod_ws::url`.
2. Reject a text frame with a protocol error. Send a ping every 20 s and close at no pong in 10 s.
3. Do not edit `src/ws_relay.rs` (frozen, `docs/decisions.md`, 2026-11-11).

## Prove

```sh
cargo test --test link_ws
sh scripts/agent/check.sh
```
## Start condition

T-261, T-262 and T-264 are done.

## Correction

2026-11-13: The carrier `ws` of `pod-link` is in podcrates `v0.38.0` (taken, c65090d): the block is gone.

## Done

2026-11-13, commit 12a06de (one code commit with T-270). Feature `ws` of podnet is `["tcp", "pod-link/ws"]` (`Cargo.toml`; `ws` is in `OPTIONAL` and `FULL` of `src/build_features.rs`, and in `full`). `src/link_ws.rs` (`podnet::link_ws`) has `carrier(me, allow)`, `carrier_via`, `listen`, `dial` (pins `KEY` of `KEY@wss://HOST[:PORT]/PATH`, then checks the proved key against the allow list) and `Probe` (the registry row). The listener runs the accept loop of `pod_ws::serve`. The registry has the row `ws`, `link listen|dial|connect` take `--carrier ws` (`Chosen` in `src/cli/cmd/link/mod.rs`), `src/manual/inventory.rs` has the entry, `docs/link.md` (The WebSocket carrier) and `docs/build.md` have it. Podnet writes no carrier of its own: the binary frames, the ping and the text-frame rule are those of `pod-ws`.

Tests (`tests/link_ws.rs`, build `--features ws`; this host allowed TCP loopback, none skipped), 13 tests: `a_link_crosses_ws_and_each_end_learns_the_key_of_the_other`, `a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open`, `a_listener_with_another_key_than_the_pinned_one_exits_77`, `a_listener_that_the_dialer_list_does_not_hold_exits_77`, `a_change_of_the_client_keys_applies_to_the_next_handshake`, `a_peer_that_speaks_no_tls_is_not_a_link`, `link_dial_check_meets_link_listen_over_ws_and_a_stranger_exits_77`, and six with no socket. `tests/link_features.rs` and `tests/link_registry.rs` list the row. These tests cannot run on the old code: the carrier is new.

Not proven: the text-frame protocol error and the ping and pong timing (tests of `pod-ws`); a host that denies TCP loopback; a link over a proxy chain (`--egress` is for `tcp`); the resume of a session over `ws`. The listener gets no error for a refused peer: the accept loop of `pod-ws` cuts the connection and goes on (the tests wait 3 s).

# T-270: link: the HTTP/2 carrier with one bidirectional POST (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E1-3 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: HTTP/2 single bidirectional POST. The status in the issue is "partial". The row is done when: tests/link_h2.rs.

The rules of the issue that apply to this row:

- The shared rules of the carriers apply (TLS 1.3, ALPN `podnet/1`, one session for each connection).

## Premise

- `src/edge_h2.rs` speaks HTTP/2 to one service: `serve_connection` (`src/edge_h2.rs:419`), `connect` (`src/edge_h2.rs:580`), `body_reader` and `body_writer` (`:855`, `:883`). It does not speak to a peer.
- `h2` 0.4 is a direct dependency (`Cargo.toml`).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write a carrier that opens one POST and uses the request body and the response body as the two directions. Reuse `body_reader` and `body_writer`.
2. Check the peer fingerprint in the TLS handshake (T-268).

## Prove

```sh
cargo test --test link_h2
sh scripts/agent/check.sh
```
## Start condition

T-268 is done.

## Correction

2026-11-13: The carrier `h2` of `pod-link` is in podcrates `v0.38.0` (taken, c65090d): the block is gone.

## Done

2026-11-13, commit 12a06de (the code commit of T-269 holds both). Feature `h2` of podnet is `["tcp", "pod-link/h2"]`, in `OPTIONAL`, `FULL` and `full`. `src/link_h2.rs` (`podnet::link_h2`) has `carrier`, `carrier_via`, `listen`, `dial` (`KEY@https://HOST[:PORT]/PATH`) and `Probe`. The carrier is one POST of HTTP/2 over TLS 1.3 (ALPN `h2`) with the pinned keys; the request body and the response body are the two directions. The registry row `h2`, `--carrier h2`, the inventory entry, `docs/link.md` (The HTTP/2 carrier) and `docs/build.md` have it. `src/edge_h2.rs` is not used.

Tests (`tests/link_h2.rs`, build `--features h2`; none skipped), 13 tests with the names of `tests/link_ws.rs` (`h2` in place of `ws`), among them `a_link_crosses_h2_and_each_end_learns_the_key_of_the_other` and `link_dial_check_meets_link_listen_over_h2_and_a_stranger_exits_77`. `cargo test --features full --test link_ws --test link_h2 --test link_tcp --test link_features --test link_registry` passes.

Not proven: a host that denies TCP loopback; a link over a proxy chain; the resume of a session over `h2`; the HTTP/2 limits of `pod-link`.

# T-271: link: the HTTP/1.1 long-poll carrier (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

Row E1-4 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: HTTP/1.1 long-poll. The status in the issue is "partial". The row is done when: tests/link_poll.rs.

The rules of the issue that apply to this row:

- Two lanes: `POST /up` and `POST /down?since=N`.
- The hold is 1 to 60 s (default 25). A batch is 256 KiB.
- A `429` honours `Retry-After`. It is not a failure until three arrive in a row.

## Premise

- No link carrier of this kind exists. `crates/podnet-core/src/util/http.rs` parses and writes HTTP/1.1 requests. `pod-proxy` has an HTTP/1.1 client and a server-side reading of a request (`PodUtils/podcrates:crates/pod-proxy/src/lib.rs`).
- T-280 (the simplex HTTP poll) uses the same lane codec.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the two lanes with the sequence `since=N` and the ack. Write the 429 rule.
2. Share the lane codec with T-280.

## Prove

```sh
cargo test --test link_poll
sh scripts/agent/check.sh
```

## Start condition

T-268 is done (the secure stream).

## Correction

2026-11-13: podnet takes podcrates v0.40.0 (1c88531, T-137): the carrier is in `pod-link` behind its own feature (PodUtils/podcrates#140). The block is gone. The work is the glue: a podnet feature, the carrier in `link listen`, `link connect` and `link dial`, the docs and the tests.
# T-272: link: the SSE and chunked-stream carrier (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-5.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

Row E1-5 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: SSE and chunked stream. The status in the issue is "partial". The row is done when: tests/link_sse.rs.

The rules of the issue that apply to this row:

- The SSE body is base64. The `streamhttp` body is raw.
- An SSE event is 8 KiB decoded. One upstream POST is in flight. A `: ping` line keeps the stream alive.
- Where server push is refused, `available()` says `server_push=false` and the session picks long-poll (T-271).

## Premise

- No link carrier of this kind exists. `crates/podnet-core/src/util/http.rs` has chunked bodies.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the SSE downstream and the POST upstream. Write the raw chunked variant.
2. Report `server_push=false` through `available()` (T-264). Make the session fall back to T-271.

## Prove

```sh
cargo test --test link_sse
sh scripts/agent/check.sh
```

## Start condition

T-268 and T-271 are done.

## Correction

2026-11-13: podnet takes podcrates v0.40.0 (1c88531, T-137): the carrier is in `pod-link` behind its own feature (PodUtils/podcrates#140). The block is gone. The work is the glue: a podnet feature, the carrier in `link listen`, `link connect` and `link dial`, the docs and the tests.
# T-273: link: the QUIC carrier (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-6.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E1-6 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: QUIC. The status in the issue is "planned". The row is done when: tests/link_quic.rs.

The rules of the issue that apply to this row:

- The shared rules apply (TLS 1.3, ALPN `podnet/1`, the fingerprint check).
- The carrier is behind the build feature `quic` (T-311).

## Premise

- WARP is a QUIC client to a fixed edge: `src/warp/quic.rs` on `pod-quic` (taken at b7b0aef). `pod-quic` has an endpoint on a UDP socket or over any datagrams, and a server configuration (`PodUtils/podcrates:crates/pod-quic/src/endpoint.rs`).
- UDP loopback is allowed on the test host, so the carrier can be tested there.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the carrier on `pod-quic`: one connection, bidirectional QUIC streams for the session, the fingerprint check (ask A7).
2. Gate it with the feature `quic`.

## Prove

```sh
cargo test --test link_quic --features quic
sh scripts/agent/check.sh
```

## Start condition

T-268 is done (the verifier) and T-311 is done (the feature).

## Correction

2026-11-13: podnet takes podcrates v0.40.0 (1c88531, T-137): the carrier is in `pod-link` behind its own feature (PodUtils/podcrates#140). The block is gone. The work is the glue: a podnet feature, the carrier in `link listen`, `link connect` and `link dial`, the docs and the tests.

2026-11-13, calls of the glue (the owner is away): the ALPN stays `podlink/1`, the default of `pod-link` and the ALPN of the `tcp` carrier of podnet, not the `podnet/1` of the issue; the two carriers of one install keep one ALPN. A dial sends from a UDP socket of the system: the way out of the process (`crate::egress`) has no datagrams, and `--egress` stays for `tcp`. The close of a QUIC connection by the peer in the handshake (the peer refused the key of this end) comes as `noq::ConnectionError::ConnectionClosed` in a `Refused` error, with no rustls alert: `src/link.rs` (`key_refused`) reads it, so it exits 77, not 69. Ask for podcrates: one typed mark for "the peer refused the key of this end" on each carrier, so a consumer does not read the errors of `noq`.

## Done

2026-11-13, commit 95cbf6a. Feature `quic` of podnet is `["tcp", "pod-link/quic"]` (in `full`; `quic` was already in `OPTIONAL` and `FULL` of `src/build_features.rs`). `src/link_quic.rs` (`podnet::link_quic`): `carrier`, `listen` (`IP:PORT`, UDP), `dial` (`KEY@HOST:PORT`) and `probe`. `src/link_keyed.rs` (`podnet::link_keyed`): the carriers of v0.40.0 by name (`NAMES`, `feature_of`, `Tune`, `dial`, `listen`); a name whose feature is off exits 69 and names the feature. `src/link_probe.rs`: the generic registry probe (no key of this install). `link listen|dial|connect --carrier quic` (`Chosen::Peer` in `src/cli/cmd/link/mod.rs`), the registry row and the inventory entry, `docs/link.md` (The QUIC carrier), `docs/build.md`.

Tests (`tests/link_quic.rs`, build `--features quic`; UDP loopback allowed here, none skipped), 8 passed: `a_link_crosses_quic_and_each_end_learns_the_key_of_the_other` (256 KiB there and back on one stream), `a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open`, `a_listener_with_another_key_than_the_pinned_one_exits_77`, `link_dial_check_meets_link_listen_over_quic_and_a_stranger_exits_77`, `a_dial_address_with_no_key_exits_64`, `a_query_key_of_another_carrier_exits_64`, `the_registry_row_has_no_key_and_refuses_a_dial`, `the_carrier_is_a_stream_with_the_alpn_of_tcp`. `tests/link_peer/mod.rs` is the shared part of the tests of the carriers of `link_keyed`. Plant: with the arms of `quic` in `link_keyed` off, 4 tests failed (the link, the stranger, the other key, the command); with the read of `ConnectionClosed` in `key_refused` off, the stranger tests got 69 for 77. `sh scripts/agent/check.sh`: each step 0.

Not proven: a host that denies UDP (the row of `link carriers` says why; the CI job `no-loopback`); a link over a proxy; the resume of a session over `quic`.

# T-274: link: the unix-socket carrier and its socketpair half (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-7.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Row E1-7 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Unix socket. The status in the issue is "planned". The row is done when: tests/link_unix.rs; it must pass in the `restricted-host` and `no-loopback` CI jobs.

The rules of the issue that apply to this row:

- The path is 1 to 107 bytes. The mode is 0600.
- `SO_PEERCRED` checks the uid unless `--allow-other-uid` is set.
- A regular file at the path exits 64. A live socket exits 69. A stale socket is replaced.

## Premise

- `bind_private` and `bind_claimed` in `crates/podnet-netns/src/listen.rs:127` and `:250` bind at mode 0600, refuse a regular file and replace a stale socket. The peer check exists in `src/router/control.rs:331`.
- `AF_UNIX` is allowed on the test host (measured by the issue). The CI jobs `restricted-host` and `no-loopback` exist (`scripts/restricted-host.sh`, `.github/workflows/ci.yml`).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the carrier on the two functions above. Map the refusals to exit 64 and exit 69.
2. Add the `--allow-other-uid` flag. Add the socketpair half for in-process use.
3. Run the test first: this carrier and T-275 are the two that run on the restricted host.

## Prove

```sh
cargo test --test link_unix
sh scripts/restricted-host.sh --test link_unix
sh scripts/agent/check.sh
```

## Start condition

T-262 and T-264 are done.

2026-11-12: the library part is done. Left: the flag `--allow-other-uid` of the command (it is the field `allow_other_uid` of `carriers::Options` now); it comes with T-265.

## Done

2026-11-12, commit 2477e5a. The unix carrier of `pod-link` is the first row of `carriers::ROWS`. `link::unix_pair` is the socket-pair half. The path rules are those of `pod-link` and are proven here through the registry: a path of 1 to 107 bytes (108 exits 64), the mode 0600, a regular file exits 64 and stays, a live socket exits 69, a stale socket is replaced, a missing listener exits 69, a peer of another uid is refused (exit 69) unless `allow_other_uid` is set.
`cargo test --test link_unix`: 8 passed, among them `a_peer_of_another_uid_is_refused_unless_allowed` (root only: a thread takes the uid 65534). `sh scripts/restricted-host.sh --test link_unix ... --test relay_listen`: the 8 tests pass with bind(AF_INET) refused. The tests start no TCP listener, so they cannot skip.
Not proven: the CI jobs `restricted-host` and `no-loopback` (not run here; the tests bind no inet socket). `scripts/restricted-host.sh` exits 1 when no test skips, so a run with only these tests fails its last check: add a test that skips (as `relay_listen`).
2026-11-12, commit 4bc2fa7 (the flag that waited for T-265). `--allow-other-uid` is a flag of `link listen`, `link dial` and `link carriers` (the field `allow_other_uid` of `carriers::Options`). Each end checks the uid of the other end, so the flag is needed at both ends. `cargo test --test cli link::a_dialer_of_another_uid_needs_allow_other_uid` (root only; it skips with a line `skipping, ...` otherwise): a dialer of uid 65534 gets exit 69 from a listener without the flag, exit 69 from a dialer without the flag, and exit 0 when both have it. `cargo test --bin podnet the_flags_reach_the_options_of_the_carriers`: pass. `sh scripts/restricted-host.sh` with `link_unix`, `link_memory`, `link_hello`, `relay_listen` and `cli`: every test passed with bind(AF_INET) refused (11 skipped, so the check of the script holds).
Not proven: the CI jobs `restricted-host` and `no-loopback` (not run here; the script ran locally).


# T-275: link: the in-process memory carrier (GitHub #467)

**Source:** GitHub #467 (https://github.com/PodUtils/podnet/issues/467), row E1-8.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Row E1-8 of GitHub #467 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Memory. The status in the issue is "planned". The row is done when: tests/link_memory.rs; it must pass in the `restricted-host` and `no-loopback` CI jobs.

The rules of the issue that apply to this row:

- A ring of 1 MiB for each direction (64 KiB to 64 MiB). A full ring blocks the writer and does not truncate.
- The C ABI (T-304) and the tests need this carrier.

## Premise

- No in-process duplex carrier exists. `src/edge_h2.rs:855` uses `tokio::io::DuplexStream` for a body, not as a carrier.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the pair of rings with back-pressure and a configurable size. Close one end and the other reads end of stream.
2. Run the test first (see T-274).

## Prove

```sh
cargo test --test link_memory
sh scripts/restricted-host.sh --test link_memory
sh scripts/agent/check.sh
```

## Start condition

T-262 and T-264 are done.

## Done

2026-11-12, commit 2477e5a. The memory carrier of `pod-link` is the second row of `carriers::ROWS`; `Options::ring` sets the ring (64 KiB to 64 MiB, other values exit 64). `link::memory_pair` runs two links in one process.
`cargo test --test link_memory`: 6 passed: `a_full_ring_blocks_the_writer_and_truncates_nothing`, `the_end_of_one_side_is_the_end_of_stream_of_the_other`, `the_size_of_the_ring_is_64_kib_to_64_mib`, `a_name_with_no_listener_exits_69_and_a_held_name_exits_69`, `a_link_crosses_the_memory_carrier_by_name`, `a_link_over_a_memory_pair_ends_when_a_side_drops`. `sh scripts/restricted-host.sh` with these tests: pass.
Not proven: the CI jobs `restricted-host` and `no-loopback` (not run here; the tests bind no inet socket).


# T-276: link: the UDP carrier with KCP (GitHub #468)

**Source:** GitHub #468 (https://github.com/PodUtils/podnet/issues/468), row E2-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

## Problem

Row E2-1 of GitHub #468 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: UDP carrier with KCP. The status in the issue is "partial". The row is done when: tests/link_udp.rs (it runs here, UDP loopback is allowed): an 8 MiB round trip over a 5% drop and 1% reorder shim; a 20% drop run within 3 times the clean time; exit 69 when UDP is denied.

The rules of the issue that apply to this row:

- MTU 1200. KCP window 32 to 1024. A record is 1 MiB. Idle time is 20 s. No IP fragmentation (KCP fragments at 1 KiB).
- `--kcp-nodelay 1` exists and is documented as unfair to competing flows. It is not the default. Congestion control is the KCP default.
- Where UDP is refused, `available()` says so and the carrier sends no packet.
- GitHub #372 covers the router datagram framing on a partial write. This entry is the peer carrier, not the guest path of the router.

## Premise

- UDP paths exist: `crates/podnet-core/src/udp.rs`, `src/forwards/udp.rs`. No KCP code and no KCP crate are in the tree (`Cargo.lock`).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Choose the KCP code: a pure Rust crate that passes `cargo deny`, or own code. Write the choice in a Correction.
2. Write the carrier with the settings above, a loss shim for the tests, and the `available()` reason for denied UDP.
3. Land the selective-repeat ARQ (T-277) before this entry or in the same change.

## Prove

```sh
cargo test --test link_udp
sh scripts/agent/check.sh
```

## Start condition

T-277 is done (or lands in the same change), and T-262 and T-264 are done.

## Correction

2026-11-13 (PodUtils/podcrates#139, their entry 526): with the take of the udp carrier, the root `Cargo.toml` sets `[profile.dev.package.kcp] overflow-checks = false` (the crate `kcp` wraps its clock and sequence numbers by design; with overflow checks a debug build ends a long stream). The profile goes when `kcp` fixes its arithmetic.

2026-11-13, the lead (PodUtils/podcrates#138, answered in GitHub #527): the congestion window of KCP is the default (`nodelay` off). The 5 % gate runs with the default; the 20 % gate runs with `nodelay` on, with the bound that a release build holds. `--carrier udp` takes `?kcp-nodelay=1`.

2026-11-13: podnet takes podcrates v0.40.0 (1c88531, T-137): the carrier is in `pod-link` behind its own feature (PodUtils/podcrates#140). The block is gone. The work is the glue: a podnet feature, the carrier in `link listen`, `link connect` and `link dial`, the docs and the tests.
# T-277: link: a selective-repeat ARQ for any lossy carrier (GitHub #468)

**Source:** GitHub #468 (https://github.com/PodUtils/podnet/issues/468), row E2-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

## Problem

Row E2-2 of GitHub #468 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Selective-repeat ARQ. The status in the issue is "planned". The row is done when: tests/link_arq.rs over the in-process lossy shim: a fragment count, an isolated stream close, a wrap-triggered rekey.

The rules of the issue that apply to this row:

- A frame carries a 32-bit sequence, a 16-bit fragment index and a fragment count.
- SACK is a 32-byte bitmap (256 bits) for a window of 256 frames. Retransmission is for each fragment.
- The RTO floor is 200 ms and the ceiling is 8 s. There are 8 retries.
- A stream that exhausts its retries closes with `arq-timeout`, not the session, unless the control channel is also lost.
- A 32-bit sequence wraps after 2^32 frames (4 TiB at the 1 KiB fragment ceiling). The wrap forces a rekey and never a silent reuse.

## Premise

- No link ARQ exists. TCP recovery is the only reliability today. The rekey is the job of the record layer (T-298).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the ARQ as a layer between the carrier and the session. The carrier reports `ordered=false` or `true` (T-264).
2. Write the SACK window, the RTO with the bounds above and the per-fragment resend.
3. Close one stream on `arq-timeout`. Test that the session lives.
4. Ask the record layer for a rekey before the wrap (T-298). Test with a start sequence near 2^32.

## Prove

```sh
cargo test --test link_arq
sh scripts/agent/check.sh
```
## Start condition

T-262 and T-264 are done. The rekey part waits for T-298.

## Correction

2026-11-13: `pod_link::arq` (Arq, ArqStream, ArqHandle) is in podcrates `v0.39.0` (taken): the block is gone. It runs a `Link` over a lossy `datagram::Datagrams`: 256 frames in flight, the RTO of RFC 6298, a fast retransmit. Open in podcrates: an `arq-timeout` ends the whole link (its entry 514), no congestion window yet (its entry 515). The first lossy carrier of podnet that takes it is T-276 (UDP), still blocked.

# T-278: link: the DNS simplex carrier (GitHub #468)

**Source:** GitHub #468 (https://github.com/PodUtils/podnet/issues/468), row E2-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** open

## Problem

Row E2-3 of GitHub #468 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: DNS simplex. The status in the issue is "planned". The row is done when: tests/link_dns.rs against a local authoritative server.

The rules of the issue that apply to this row:

- The QNAME is base32 with a sequence and a fragment index. Answers come from TXT.
- A QNAME is 253 bytes at most with 63-byte labels. An answer is 512 bytes without EDNS and 1232 with it.
- There are 8 fragments in flight and a token bucket of 20 queries a second, with a backoff of 0 to 2 s.
- `TC` is retried over TCP/53 only if it is permitted. Otherwise the batch shrinks and `truncated` is reported.

## Premise

- No QNAME encoding, no TXT reader and no poll loop exist. The DNS codecs of the tree build A and AAAA only: `crates/podnet-netns/src/dns.rs`, `crates/podnet-vnet/src/dns.rs`. `pod-dns` has a wire module for queries, addresses and SRV (`PodUtils/podcrates:crates/pod-dns/src/wire.rs`), no TXT reader.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Ask podcrates (ask A10): does `pod-dns` want a TXT and CNAME reader with EDNS0 and `TC`? If yes, take it. If no, write the codec in the link crate.
2. Write the encoder, the poll loop, the token bucket and the TC rule. Write a small authoritative server for the test.

## Prove

```sh
cargo test --test link_dns
sh scripts/agent/check.sh
```

## Start condition

T-277, T-262 and T-264 are done. Ask A10 is answered.

## Correction

2026-11-13: podnet takes podcrates v0.40.0 (1c88531, T-137): the carrier is in `pod-link` behind its own feature (PodUtils/podcrates#140). The block is gone. The work is the glue: a podnet feature, the carrier in `link listen`, `link connect` and `link dial`, the docs and the tests.
# T-279: link: the object-storage simplex carrier (GitHub #468)

**Source:** GitHub #468 (https://github.com/PodUtils/podnet/issues/468), row E2-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

Row E2-4 of GitHub #468 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Object-storage simplex. The status in the issue is "planned". The row is done when: tests/link_s3.rs against a local S3-compatible server.

The rules of the issue that apply to this row:

- Two prefixes, one for each direction. Objects are named `<prefix>/<direction>/<seq>-<nonce>`.
- A conditional `PUT` (`If-None-Match: *`) and a conditional `GET` on the known ETag.
- Objects are 256 KiB, 8 in flight. The poll interval is 100 ms to 5 s. The budget is 1 GiB for each prefix with a warning at 80%. A page lists 1000 keys.
- Pruning is opt-in (`--s3-prune`), because the bucket can be shared.
- Credentials come from the environment or a profile file, never from `argv`. A missing credential exits 77 and prints nothing about it.
- The carrier is behind the feature `s3` (T-311).

## Premise

- No S3 client for a link exists, and the feature `s3` does not exist. `Cargo.lock` has no S3 client crate.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write a small S3 client (SigV4 with `ring`, `PUT`, `GET`, `LIST`) or choose a pure Rust crate that passes `cargo deny`. Write the choice in a Correction.
2. Write the two prefixes, the object names, the poll loop and the budget.
3. Do not add a hardcoded host of a provider: the endpoint comes from the operator.

## Prove

```sh
cargo test --test link_s3 --features s3
sh scripts/agent/check.sh
```

## Blocker

The carrier is a take of `pod-link` of PodUtils/podcrates, its entry 494 in the record of podcrates (PodUtils/podcrates#133,
2026-11-13; estimate: tag `v0.39.0`).

## Start condition

T-277, T-262, T-264 and T-311 are done.

# T-280: link: the HTTP poll simplex carrier (GitHub #468)

**Source:** GitHub #468 (https://github.com/PodUtils/podnet/issues/468), row E2-5.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

Row E2-5 of GitHub #468 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: HTTP poll simplex. The status in the issue is "planned". The row is done when: tests/link_httppoll.rs.

The rules of the issue that apply to this row:

- `POST` upstream with `202` and an ack sequence. `GET /link/out?since=N` with `204` when empty.
- 64 KiB for each request, one request in flight for each lane, 4 KiB headers, backoff 100 ms to 5 s.
- This carrier has the highest overhead for each byte. It reports that cost class.

## Premise

- No `POST /up` and `GET /out` pair exists. The long-poll carrier of T-271 has the same two lanes with a hold.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Share the lane codec with T-271. Write the simplex pairing (one carrier for each direction) so that the direction split (T-302) can use it.
2. Report the cost class `highest` through the registry (T-264).

## Prove

```sh
cargo test --test link_httppoll
sh scripts/agent/check.sh
```

## Start condition

T-271, T-277 and T-264 are done.

## Correction

2026-11-13: podnet takes podcrates v0.40.0 (1c88531, T-137): the carrier is in `pod-link` behind its own feature (PodUtils/podcrates#140). The block is gone. The work is the glue: a podnet feature, the carrier in `link listen`, `link connect` and `link dial`, the docs and the tests.
# T-281: link: the DNS stream carrier with a pipelined query window (GitHub #469)

**Source:** GitHub #469 (https://github.com/PodUtils/podnet/issues/469), row E3-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

Row E3-1 of GitHub #469 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: DNS stream. The status in the issue is "planned". The row is done when: tests/link_dnsstream.rs: 256 KiB, reorder, `SERVFAIL` backoff, rate ceiling.

The rules of the issue that apply to this row:

- 32 queries in flight. Answers are reordered by sequence. The carrier reports `ordered=false`, and the ARQ of the session restores the order.
- TXT first. `CNAME` chains where TXT is filtered. Answers are 1232 bytes. The ceiling is 200 queries a second. A deliberate re-resolve happens every 5 minutes.
- This carrier is a last resort. The operator is told what the narrow hole costs.

## Premise

- No pipelined query window exists. T-278 is a different carrier (simplex, 8 fragments in flight). The DNS codec question is ask A10.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Reuse the DNS codec and the authoritative test server of T-278.
2. Write the 32-query window, the reorder by sequence, the CNAME fallback, the backoff on `SERVFAIL` and the rate ceiling.
3. Report the cost class in the registry (T-264).

## Prove

```sh
cargo test --test link_dnsstream
sh scripts/agent/check.sh
```

## Blocker

The carrier is a take of `pod-link` of PodUtils/podcrates, its entry 496 in the record of podcrates (PodUtils/podcrates#133,
2026-11-13; estimate: tag `v0.40.0`).

## Start condition

T-277 and T-278 are done.

# T-282: link: the ICMP echo carrier (GitHub #469)

**Source:** GitHub #469 (https://github.com/PodUtils/podnet/issues/469), row E3-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

Row E3-2 of GitHub #469 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: ICMP echo. The status in the issue is "planned". The row is done when: tests/link_icmp.rs: a named skip without `CAP_NET_RAW`; 64 KiB when present; a wrong session id is dropped; the seccomp rule test fails when the rule is missing.

The rules of the issue that apply to this row:

- The payload is 1400 bytes. 16 echoes are in flight. The default rate is 10 a second. The reply timeout is 1 s.
- `--allow-raw icmp` is required, and `doctor` prints it.
- `available()` checks, in this order: the build feature, `CAP_NET_RAW` (or uid 0; the binary is not setuid), and one echo with a 2 s timeout.
- The seccomp profile of `podnet pod` must allow the ICMP socket family. Otherwise the row says `seccomp denies AF_INET/SOCK_RAW`.
- A new socket family needs a row in the seccomp profile and in the guidance of GitHub #89 (T-001).
- The carrier is behind the feature `icmp` (T-311).

## Premise

- No raw socket path exists for a link. `crates/podnet-netns/src/icmp.rs` serves ICMP datagram sockets of a pod (T-066). It does not open a raw socket.
- The seccomp filter is in `crates/podnet-netns/src/seccomp/filter.rs` (it goes to `pod-hold`: frozen, fixes only).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the raw echo carrier with the session id in the payload. Drop a wrong id.
2. Write the `available()` order above. Add the `doctor` row.
3. Test the seccomp rule. A change in the frozen seccomp files goes to podcrates as a COORD issue (`docs/maintenance.md`, Shared crates).

## Prove

```sh
cargo test --test link_icmp --features icmp
sh scripts/agent/check.sh
```

## Blocker

The carrier is a take of `pod-link` of PodUtils/podcrates, its entry 497 in the record of podcrates (PodUtils/podcrates#133,
2026-11-13; estimate: tag `v0.39.0`).

## Start condition

T-277, T-264 and T-311 are done.

# T-283: link: a WireGuard peer as a carrier (GitHub #469)

**Source:** GitHub #469 (https://github.com/PodUtils/podnet/issues/469), row E3-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** blocked

## Problem

Row E3-3 of GitHub #469 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: WireGuard peer as a carrier. The status in the issue is "partial". The row is done when: tests/link_wg.rs: a userspace pair carries 1 MiB; a missing file exits 66; a wider mode exits 71.

The rules of the issue that apply to this row:

- Dial the tunnel address of the peer through the userspace WireGuard network. Then run the normal stream handshake.
- The MTU comes from the interface (default 1280). The carrier uses UDP only. One session for each peer address.
- The key stays in the `wg-quick` file at mode 0600. It is never read into `--json`.

## Premise

- `wg:FILE` is a network of the registry: `src/networks/wg.rs` reads the file with `wg_quick::read_file` and runs the handshake with `pod-wg` and a stack of `pod-netstack` (T-117, partial).
- No carrier runs on top of it. The responder half is T-125 (`pod-wg`).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Dial half: connect through the `wg:` network (T-117), then run the peer handshake (T-268).
2. Listen half: it needs the responder of `pod-wg` (T-125). Write it when T-125 is done.
3. Map a missing file to exit 66 and a wider mode to exit 71 (T-261).

## Prove

```sh
cargo test --test link_wg
sh scripts/agent/check.sh
```

## Blocker

The carrier is a take of `pod-link` of PodUtils/podcrates, its entry 498 in the record of podcrates (PodUtils/podcrates#133,
2026-11-13; estimate: tag `v0.40.0`).

## Start condition

T-117 has its network (steps 1 and 3 done). The listen half waits for T-125.

# T-284: link: NAT traversal with a relay fallback (GitHub #469)

**Source:** GitHub #469 (https://github.com/PodUtils/podnet/issues/469), row E3-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

Row E3-4 of GitHub #469 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Peer-to-peer NAT traversal. The status in the issue is "planned". The row is done when: tests/link_p2p.rs in two network namespaces: a direct path for a cone NAT; the relay with `symmetric-nat`; the fingerprint is checked on both.

The rules of the issue that apply to this row:

- Exchange candidate addresses through the rendezvous. Send UDP at the same time from both sides. Verify the path with a nonce.
- If traversal fails within 5 s, fall back to the configured relay. `status` says `path: relay` and `reason: symmetric-nat`.
- The fallback is a change of the carrier path, not of the peer. The fingerprint check is the same on both paths.
- The relay alternates are chosen once at startup (GitHub #359, T-162). The fallback does not hardcode one relay, and does not switch peers when it switches paths.

## Premise

- The overlay and the relay exist. Candidate exchange and traversal do not.
- STUN hosts are data of `pod-services` (`PodUtils/podcrates:crates/pod-services/services/google-stun.toml`). Add no hardcoded host (`docs/decisions.md`, Services crate).
- The relay client is glue over `pod-relay` (`src/ws_relay.rs`; T-049, taken at podcrates v0.30.0).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Ask podcrates (ask A5) whether the relay contract can carry the candidate exchange, and for the take of `pod-relay`.
2. Write the candidate gathering from the STUN data of `pod-services`, the simultaneous send and the nonce check.
3. Write the fallback as a path change below the session. Test two network namespaces with a cone NAT and a symmetric NAT.

## Decision

The relay half is the work of `pod-relay`. This entry waits for the take (T-049).

## Prove

```sh
cargo test --test link_p2p
sh scripts/agent/check.sh
```

## Blocker

The carrier is a take of `pod-link` of PodUtils/podcrates, its entry 499 in the record of podcrates (PodUtils/podcrates#133,
2026-11-13; estimate: tag `v0.40.0`).

## Start condition

T-261, T-262 and T-049 are done.

## Correction

2026-11-12: podcrates answered ask A5 (PodUtils/podcrates#88, podcrates T-303): the relay contract carries the
exchange of candidates as a message of opaque binary data between the two ends before the stream. The relay gives no
reflexive address: the peers learn their public addresses from STUN (the services of the kind `stun` of
`pod-services`). The relay carries TCP only and takes no inbound TCP, so the direct path (a UDP hole punch) is the
peers' own. A session needs the node online and its `ready` within 15 s, and lasts at most 720 min and 64 MiB.

2026-11-12: `pod-relay` is released and taken (v0.30.0, 83e0c93), and ask A5 is answered (above). The blocker is gone. The entry waits for its start condition: T-261 and T-262.


# T-285: link: signed route announcements (GitHub #470)

**Source:** GitHub #470 (https://github.com/PodUtils/podnet/issues/470), row E4-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

Row E4-1 of GitHub #470 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Signed route announcement. The status in the issue is "planned". The row is done when: mesh_announce (tests/mesh_announce.rs).

The rules of the issue that apply to this row:

- Each node announces its direct neighbours (the cost is the measured RTT in milliseconds, or a configured weight) and the services it offers.
- The origin signs with Ed25519. `seq` strictly increases. The announcement has a `path` of peer ids and an expiry.
- A receiver verifies the signature against the `PeerId` of the origin. It drops an announcement whose `path` has itself, a `seq` already seen, and an expired announcement.
- Only the announcement of a direct neighbour is a neighbour claim. A multi-hop announcement updates reachability, not adjacency.
- Bounds: 64 neighbours, 16 services, 16 hops, 8 KiB before the signature, an interval of 10 s with jitter, an expiry of 3 intervals, a clock skew of 120 s either way.
- The signed bytes are a deterministic encoding, not JSON. `--announce` turns it on.

## Premise

- No announcement type, no signature and no `--announce` exist. Nothing in the tree signs a route.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Define the binary encoding with fixed field order. Sign it with the identity key (T-261).
2. Write the receive checks and the bounds. Test each drop rule and the skew.
3. Add `--announce` to `link listen` and `link dial` (T-265).

## Prove

```sh
cargo test --test mesh_announce
sh scripts/agent/check.sh
```

## Start condition

T-261, T-262 and T-265 are done.

## Correction

Steps 1 and 2 were done first. Step 3 (`--announce`) is done 2026-11-13.


2026-11-13, the lead: the origin signs the body only. The path and the cost of a hop are not signed, so a forwarder can drop
hops and give a shorter path, or a lower cost, than the real one. A loop through a dropped hop is then not seen. T-287 (the
forwarding) closes this: each hop signs its addition to the path (its key, the cost of its link, and the signature of the
hop before it), and `verify` checks the chain.
## Done

2026-11-13, commit 6103521 (steps 1 and 2). `src/mesh_announce.rs` (`podnet::mesh_announce`): the deterministic binary encoding (fixed field order, big-endian, sorted neighbours and services, no JSON), `encode` (Ed25519 signature of a label and the body with the key of `pod-identity`), `append_hop`, `verify` and `Receiver::accept`. The bounds: 64 neighbours, 16 services, a name of 1 to 64 bytes, 16 path nodes, 8 KiB body, a life of at most 30 s, a skew of 120 s each way. `verify` drops: bad shape, a path that does not start at the origin, a repeat in the path, this node in the path, a time of issue too far ahead or behind, a bad lifetime, expiry, a bad signature. `Receiver` keeps the newest `seq` of up to 4096 origins and drops a seen `seq`. Only the path `[origin]` is a neighbour claim. `Verified::route` gives the announcement for `mesh_table`. The path is not signed (a forwarder adds to it). `docs/link.md` has the section "The route announcement".
`cargo test --test mesh_announce`: 25 passed, among them `skew_ahead_boundary`, `skew_behind_boundary`, `path_with_self_is_dropped`, `seen_seq_is_dropped`, `neighbour_bound`, `service_bounds`, `path_bounds`, `tampered_cost_is_a_bad_signature`, `every_flipped_byte_is_dropped`, `verified_announcement_feeds_the_route_table`. With the signature check and the seen-`seq` check disabled, 5 tests failed. `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test all 0.
Not proven: `--announce`, a send or receive over a real link, the interval with jitter (the constants exist, no timer), the use of the unsigned `cost` of a multi-hop announcement (the caller gives it to `route`), the signature of the path.

2026-11-13, step 3, commit ebd0ceb. `src/link_announce.rs` (`podnet::link_announce`): `Mesh` (neighbours, `seq`, `Receiver`, `RouteTable`; pure, time given), `run_sender` (the stream of the service `announce`, each 10 s with jitter, the first at once), `receive_stream`, `serve_streams` (the dialer), `Guard` (the neighbour is up while it lives; the end of the link ends the direct route at once). The receiver takes only the path `[origin]` with the origin equal to the peer (T-287 adds multi-hop), checks `verify`, then the `seq` (`Receiver::note`, so a refused announcement does not use a `seq`), then `RouteTable::learn`. The cost is `--announce-weight`, else the RTT, else 10. `link listen --announce [--announce-weight N]` and `link dial --announce [--announce-weight N]`; the control verb `routes` (listener; `RouteTable::best_routes`). `docs/link.md` has the section "Announcements on a link"; `llms.txt` files regenerated.
`cargo test --test link_announce`: 6 passed: `two_ends_with_announce_learn_each_other`, `a_tampered_announcement_is_dropped_and_the_link_stays_up`, `an_announcement_of_another_origin_is_not_direct`, `the_cost_and_the_delay_follow_their_rules`, `the_commands_announce_and_the_control_socket_shows_the_routes`, `announce_flags_are_checked`. With the receive step replaced by a drop, 3 tests failed (the two-ends, the tampered and the command test). `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test all 0.
Not proven: `link connect --announce` (not built); a dialer that shows its routes (it has no control socket); multi-hop announcements and their forwarding (T-287); the signature of each hop (T-287); a measured RTT as the cost (only the configured weight is tested); the carriers other than unix.

# T-286: link: the path-vector route table (GitHub #470)

**Source:** GitHub #470 (https://github.com/PodUtils/podnet/issues/470), row E4-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E4-2 of GitHub #470 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Path-vector table. The status in the issue is "planned". The row is done when: mesh_table (tests/mesh_table.rs).

The rules of the issue that apply to this row:

- Selection prefers the lowest cost, then the fewest hops, then the newest `seq`.
- A route is withdrawn when its announcement expires or its next-hop session closes: at once for a direct neighbour, after one expiry interval for a multi-hop route.
- The table holds 4096 origins and 16 hops. Eviction starts at 90% and has a counter.

## Premise

- No route table exists besides the egress policy of the router (`crates/podnet-core/src/network/policy.rs`).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the table keyed by origin, the selection order and the withdrawal rules. Count the evictions.
2. Test the order, the expiry with a fake clock, and the eviction at 90%.

## Prove

```sh
cargo test --test mesh_table
sh scripts/agent/check.sh
```

## Start condition

T-285 is done.

## Done

2026-11-12, commit e51d147. `src/mesh_table.rs` (`podnet::mesh_table`) is the route table: a pure data structure with no I/O. The caller gives the time in milliseconds. The key type is `PublicKey` of `pod-identity`. `RouteTable::learn` takes a verified `Announcement` (origin, seq, cost, path, expiry) from a next hop with the cost of the link. `best` and `ranked` select: fresh before held, then lowest cost, then fewest hops, then newest `seq`. `learn` refuses a loop (the path holds this node or a node two times), more than 16 nodes, an expired announcement, a stale `seq`, a bad path and an own origin. A direct route ends at its expiry. A multi-hop route is held one expiry interval (30 s) after its expiry. `session_closed` ends the direct route at once and downs the multi-hop routes through the peer for one interval. The table holds 4096 origins and 8 routes for each origin. At 90% (3686) each new origin evicts the worst origin (ended routes are purged first, direct routes are never evicted), and `evictions()` counts them. `scripts/check-layers.py` puts the module in the layer `podnet`. `docs/link.md` has the section "The route table" (a library part, no command yet).
`cargo test --test mesh_table`: 22 passed, among them `selection_lowest_cost_first`, `selection_fewest_hops_on_equal_cost`, `selection_newest_seq_on_equal_cost_and_hops`, `loop_through_this_node_is_refused`, `repeated_node_in_path_is_refused`, `direct_route_ends_at_expiry`, `multi_hop_route_is_held_one_interval_after_expiry`, `session_close_ends_direct_route_at_once`, `session_close_downs_multi_hop_routes_for_one_interval`, `eviction_starts_at_90_percent_and_counts`, `direct_routes_are_not_evicted`. With the loop check and the eviction disabled, 5 tests failed (the two loop tests and the three eviction tests). `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test all 0.
Not proven: the table with real signed announcements (T-285 has not started; the caller checks the signature, the skew and the seen `seq`); a use in forwarding (T-287), the status view (T-289) and any command. The newest `seq` of an origin is forgotten when its last route ends, so a replay of an old `seq` after that needs the seen-`seq` check of T-285.


# T-287: link: multi-hop forwarding of a session stream (GitHub #470)

**Source:** GitHub #470 (https://github.com/PodUtils/podnet/issues/470), row E4-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** partial

## Problem

Row E4-3 of GitHub #470 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Multi-hop forwarding. The status in the issue is "planned". The row is done when: mesh_three_hops (tests/mesh_three_hops.rs): A, B, C, D carry 1 MiB with equal SHA-256 while B sees records only; a hop limit of 8 refuses a 9-hop path with `HopsExceeded`; a closed middle link fails one stream, not the process.

The rules of the issue that apply to this row:

- Each hop decrements a hop counter and re-opens the stream to the next peer.
- The end-to-end session is between the two ends. A middle hop sees ciphertext records, not plaintext, because the records are end-to-end. A hop off the path cannot decrypt.
- Back-pressure runs end to end.
- A hop that cannot allocate a stream returns `RESOURCE_LIMIT`, and the previous hop tries the next route.
- 8 hops by default, 16 at most. 1 MiB in flight for each stream and hop. A per-hop open timeout of 30 s. No loops.

## Premise

- No forwarding of a session stream through a peer exists.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the `OPEN` with a hop counter and the re-open to the next peer from the table (T-286).
2. Keep the end-to-end record layer (T-298) between the ends. Test that the middle hop cannot decrypt.
3. Write the `RESOURCE_LIMIT` retry and the hop caps.

## Prove

```sh
cargo test --test mesh_three_hops
sh scripts/agent/check.sh
```

## Start condition

T-262, T-268, T-285, T-286 and T-298 are done. The claim of end-to-end secrecy needs T-298.

## Correction

2026-11-13: item 1 of the Correction (the secrecy from end to end) is done by T-298 (f748eb4): with `--seal`, a stream is sealed between its two ends, and a middle hop forwards records only (`tests/link_record_mesh.rs`). Left: the test with three podnet processes, now possible because a `link connect` accepts streams (T-297).

2026-11-13, the lead (from T-285): the path and the cost of a hop in an announcement are not signed. This entry adds a signature for each hop: a forwarder signs its key, the cost of its link and the signature of the hop before it, and `verify` of `src/mesh_announce.rs` checks the chain.

2026-11-13, partial. Done in commit c9581c1: the chain of hop signatures (`append_hop(wire, identity, cost)`, `verify` returns `BadHopSignature`, `Verified::route` uses the signed cost sum, `Verified::path_cost`). `cargo test --test mesh_announce`: 29 passed, among them `tampered_hop_is_dropped`, `dropped_hop_is_a_bad_hop_signature`, `lowered_cost_is_a_bad_hop_signature`, `hop_signature_of_another_key_is_dropped`, `append_hop_signs_the_hop_and_the_chain_verifies`. With the hop check disabled, 4 tests failed.
Done in commit cc68ec8: the forwarding of announcements (`Mesh::receive` takes a path that ends at the peer and sends it on with its hop; hop limit 8 by default) and of streams (`src/link_forward.rs`: `PNSVC1 fwd` and the header `PNFWD1`, hop count, loop rule with the list of visited nodes, retry of the next route on `RESOURCE_LIMIT` or `NO_ROUTE` or a 30 s timeout, answers 3, 4 and 5). `link listen --announce` relays; `link dial --announce --node FINGERPRINT [--hops N]` opens a stream to a node by its fingerprint. `cargo test --test mesh_three_hops`: 7 passed: `a_reaches_a_service_of_c_through_b` (1 MiB through B, equal bytes, cost 3 + 5), `the_hop_limit_refuses_a_longer_path` (4 links, `HopsExceeded` for 0, 1 and 3), `the_hop_limit_bounds_the_forwarding_of_announcements`, `a_loop_is_refused`, `a_closed_middle_link_fails_one_stream_not_the_process`, `a_hop_without_a_stream_makes_the_sender_try_the_next_route`, `a_multi_hop_announcement_needs_a_valid_chain`. `cargo test --test link_announce`: 7 passed (`dial_node_reaches_the_service_of_the_listener_by_its_fingerprint` is new). With the forwarding of announcements disabled, 5 of the 7 tests of `mesh_three_hops` failed.
Left: (1) the end-to-end record layer (T-298, blocked): a middle hop sees the bytes of a stream, so "B sees records only" is not proven. (2) A SHA-256 test over four nodes A, B, C, D with 8 hops and a 9-hop path (the test uses a chain of 5 nodes, 4 links, and 1 MiB over 2 links, compared byte for byte, not by SHA-256). (3) A test with three podnet processes: the CLI has no node that both listens and dials, so a chain needs a listener that also dials (an `--uplink` flag or the like). (4) The route table keeps one `seq` for each origin, so two routes of one round (a diamond) are not both held: the later path of the same `seq` is dropped. A diamond with two live next hops needs the seen-`seq` rule of `Receiver` to take the same `seq` from another next hop. (5) The carriers other than unix. (6) The 1 MiB limit in flight for each stream and hop is the credit window of `pod-link`; no test shows it.

# T-288: link: load balancing across link groups (GitHub #470)

**Source:** GitHub #470 (https://github.com/PodUtils/podnet/issues/470), row E4-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E4-4 of GitHub #470 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Load balancing across links. The status in the issue is "partial". The row is done when: link_lb (tests/link_lb.rs); the tests of the router engine stay unchanged.

The rules of the issue that apply to this row:

- The policies are `weighted` (default), `rr`, `minrtt` and `leastload`. `minrtt` moves only for `max(5 ms, 10%)` over three rounds.
- The choice happens once for each stream and the stream is pinned.
- A member that fails a control probe is dropped at once. A member that fails a data probe is dropped after the configured number of failures in a row.
- If every member is down, the stream is refused. There is no silent direct fallback.
- 16 members, weights 1 to 100, a probe interval of 1 to 60 s.
- Reuse the router engine. Do not rewrite it.

## Premise

- The policy engine exists: `Policy` and `choose` in `crates/podnet-core/src/network/policy.rs:23` and `:139`, with the balance, health and meter modules beside it (`crates/podnet-core/src/network/`).
- No link group exists.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Make a link a member of a group of the router engine. Pin the choice to the stream.
2. Add the member limits and the probes of the issue. Test that the router tests pass unchanged.

## Prove

```sh
cargo test --test link_lb
cargo test -p podnet-core network
sh scripts/agent/check.sh
```

## Start condition

T-262 and T-264 are done.

## Done

2026-11-13, commit 7568a04. A link is a member of a group of the router engine. `Balance::peers` builds a group of peer ways, and `Balance::way` gives the group the addressing mode of its members. A group does not mix peer ways and destination ways. The router file takes a group of `peer:FINGERPRINT/SERVICE` networks (`src/router.rs`), with the limits of a link group. `src/link_group.rs` (`podnet::link_group::LinkGroup`) has the limits (1 to 16 members, weights 1 to 100, interval 1 to 60 s) and the probe round: the control probe is the verb `peer` of the control socket, and a member that fails it is down at once (`Balance::drop_member`); the data probe opens one stream to the service, and `down_after` failures in a row (default 4) take the member down. A member that is down goes through probation. The pick, the pin of a stream to its member, the policies, the health steps and the kill switch are the code of the router engine, and the router tests are unchanged. `docs/router.md` has "Groups of links", `docs/link.md` has "Link groups", and `llms-full.txt` is regenerated.
`cargo test --test link_lb`: 6 passed (3 runs): `link_lb_weighted_pick_pins_each_stream_to_one_link`, `link_lb_rr_and_leastload_policies_spread_streams`, `link_lb_control_probe_drops_a_member_at_once_and_it_comes_back`, `link_lb_data_probe_drops_a_member_after_k_misses_and_no_member_refuses`, `link_lb_limits_are_16_members_weights_1_to_100_interval_1_to_60`, `link_lb_the_router_file_takes_a_group_of_links`. Without the change the test file does not compile (`unresolved import podnet::link_group`). `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test all 0.
Not proven: a group over real TCP carriers; the `minrtt` move over a link group (the engine tests cover `minrtt`); the router probe loop of a router file still uses its own probe (one stream to the service, with the group control of the router), not the control probe of `LinkGroup::probe_round`; no command or flag builds a `LinkGroup` outside a router file (`src/cli/cmd/link/` belongs to T-291).


# T-289: link: the local topology and the status view of a mesh (GitHub #470)

**Source:** GitHub #470 (https://github.com/PodUtils/podnet/issues/470), row E4-5.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E4-5 of GitHub #470 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Topology and status. The status in the issue is "planned". The row is done when: mesh_status (tests/mesh_status.rs).

The rules of the issue that apply to this row:

- The view is local. It is never served on a network address. There is no endpoint without `--control unix:PATH`, and that socket is 0600.
- `--json` prints the nodes (fingerprint, name, last seq, hops, services) and the edges (peer, cost, last probe, state). `--metrics` prints counters.
- Limits: 4096 nodes, 64 edges for each node, 256 KiB response.

## Premise

- No mesh view exists. `crates/podnet-core/src/metrics.rs` has the counters of the process.
- The control socket is T-267.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add a mesh verb to the control socket (T-267) and the `--json` and `--metrics` output over the route table (T-286).
2. Truncate at the limits with a count.

## Prove

```sh
cargo test --test mesh_status
sh scripts/agent/check.sh
```

## Start condition

T-267, T-285 and T-286 are done.

## Done

2026-11-13, commit a64f158. The verb `mesh` of the control socket and the command `podnet link mesh --control unix:PATH [--json | --metrics] [--token-file FILE] [--max-nodes N] [--max-edges N]`. `src/link_announce/status.rs`: `Mesh::view` (nodes: fingerprint, name, self, seq, hops, cost, next_hop, state, services, age_ms, edges; edges: peer, cost, last_probe_ms, state), `ViewLimits` (4096 nodes, 64 edges for each node, 262143 bytes; a request can only lower them), `metrics_text` (Prometheus text). The edges of this node are its links (`up`); the edges of another node are the neighbours that it signed (`claimed`). `truncated` counts the nodes, the edges and the nodes cut to fit the size. `Mesh` keeps what each accepted announcement said (services, neighbours, time) and local names (`Mesh::set_name`). The answer uses `Reply::Big` (256 KiB); the client reads up to that size for the verb. `docs/link.md` has the section "The status view of a mesh"; `llms.txt` files regenerated.
`cargo test --test mesh_status`: 4 passed (3 runs after the last change): `mesh_json_has_the_nodes_and_the_edges`, `the_command_prints_json_text_and_metrics_from_a_socket_of_mode_0600`, `a_view_over_the_limits_is_cut_with_a_count`, `without_announce_the_verb_exits_69`. With the node and edge cuts disabled, `a_view_over_the_limits_is_cut_with_a_count` failed. `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc and layers 0; test: `quick_tunnel_standin_serves_a_visitor_over_http2` failed once under load and passed alone 3 times; the first run of `mesh_status` under load failed once (a wait that was too weak, fixed: the test waits for the claims of B and C).
Not proven: the carriers other than unix; a view of a node with 4096 origins (the limits are tested with small values and with a small size); the view of a dialer (it has no control socket); `last_probe_ms` is the time of the last announcement, not an active probe; `name` is a local name only, the announcement carries no name; a multi-hop node shows the neighbours that it signed (`claimed`), not proven adjacency.


# T-290: link: per-hop access control for services and forwarding (GitHub #470)

**Source:** GitHub #470 (https://github.com/PodUtils/podnet/issues/470), row E4-6.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E4-6 of GitHub #470 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Per-hop access control. The status in the issue is "planned". The row is done when: mesh_acl (tests/mesh_acl.rs).

The rules of the issue that apply to this row:

- Default deny. An allowed peer opens only the services listed for it. Forwarding is a separate permission.
- A frame from a peer that may not forward is dropped and counted. The session is not closed, so a wrong configuration does not stop an unrelated stream.
- 4096 entries, 64 services for each peer, one decision for each `OPEN`.

## Premise

- No per-peer service grant or forward grant exists. The allow list of T-261 says who may connect, not what it may open.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Extend the allow-list file with a service list and a `forward` flag for each peer.
2. Decide once for each `OPEN` (T-287). Drop and count a forbidden frame.

## Prove

```sh
cargo test --test mesh_acl
sh scripts/agent/check.sh
```

## Start condition

T-261 and T-287 are done.

## Correction

2026-11-13: the default of the grant. A bare key keeps the old behaviour: every service, and forwarding, so no allow-list file breaks. A key with the option `services=NAME,...` (or `*`) or `forward=yes|no` is restricted: what it does not name is denied (no `services=` means no service, no `forward=` means no forwarding). The first line that admits a key is its grant. A word after the key that is not an option stays a comment. The answer byte for a refused stream is 6 (`FORBIDDEN`); the opener exits 77.

## Done

2026-11-13, commit 9ddeb34. `src/identity.rs`: `Grant` (`open`, `closed`, `allows_service`, `may_forward`), the options `services=` and `forward=` in `AllowList::parse`, `AllowList::grant(&PublicKey)`, `MAX_SERVICES` (64; the limit of 4096 entries is unchanged). `src/link_service.rs`: `FORBIDDEN` (6). `src/link_forward.rs`: `accept_with(mesh, stream, may_forward, refused)` and `ForwardError::Forbidden`; `accept` is `accept_with` with forwarding allowed. `src/cli/cmd/link/listen.rs`: one decision for each `OPEN` in `bridge` (the service of the peer; the forward flag for a stream that goes to another node; the announce stream is never refused). `src/link_announce.rs`: `serve_streams_with` (the dialer checks the grant of its listener). `src/link_ctl.rs`: the counters `refused_service` and `refused_forward` in each row of `status`; `link status` prints them. `docs/link.md`, the help of `--allow`, `llms-full.txt` updated.
`cargo test --test mesh_acl`: 3 passed: `a_peer_allowed_raw_only_cannot_open_socks5` (exit 77, counted, the link stays up, a bare key with a comment word opens socks5), `a_peer_without_forward_cannot_forward_and_the_link_stays_up` (A - B - C: refused with 6, `refused_forward` 1, the A-B link serves raw after it; a peer with `forward=yes` is relayed to C), `the_limits_and_the_old_form_of_the_file` (64 and 65 services, 4096 and 4097 entries, bad options, old forms). With `grant_of` forced to the open grant, the 3 tests failed. `sh scripts/agent/check.sh`: fmt (after `cargo fmt`), clippy, record, size, doc, layers and test 0.
Not proven: the carriers other than unix; the view `link mesh` does not show the counters; the `dump` of T-309 (src/link_ctl/dump.rs, not edited here) has no total of the refusals; a `forward` check at the destination for a stream that a relay opened (the last hop checks the service of its neighbour, by design); a dialer refuses with the grant of `--peer` only for a stream of `--announce` relays.


# T-291: link: a SOCKS5 service at a peer (GitHub #471)

**Source:** GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E5-1 of GitHub #471 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: SOCKS5 (CONNECT, BIND, UDP ASSOCIATE at a peer). The status in the issue is "partial". The row is done when: link_socks5 (tests/link_socks5.rs).

The rules of the issue that apply to this row:

- `CONNECT` to a host. `BIND` through a peer listener. `UDP ASSOCIATE` through a peer UDP forward.
- Name targets resolve at the peer unless `--resolve local` is set.
- Credentials are optional and are compared in constant time.
- There is no local fallback when the peer refuses.
- Shared rules: no service listens on a public address by default. A peer that is not on the allow list is refused before the service opens. No credential appears in `--json`, `status` or a dump. A service that cannot start exits with its named code and leaves no half-open listener.

## Premise

- SOCKS5 exists for local and egress use: the front door in `crates/podnet-netns/src/frontdoor.rs` and the wire code in `crates/podnet-netns/src/socks.rs`. It takes a local or network address, not a session stream. `UDP ASSOCIATE` exists at `crates/podnet-netns/src/door/udp.rs`.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Register a service `socks5` that accepts a session stream and runs the server side of the front door on it.
2. Add `BIND` through a peer listener. Add the `--resolve local` flag.
3. Test the shared rules.

## Prove

```sh
cargo test --test link_socks5
sh scripts/agent/check.sh
```

## Start condition

T-266 is done.

## Correction

2026-11-12: this entry builds `CONNECT` only. `BIND` through a peer listener, `UDP ASSOCIATE` through a peer UDP forward, and `--resolve local` are left: `BIND` needs a listener of the peer that the opener can reach (a second stream), and `--resolve local` needs a SOCKS5 parser at the opener (the connector end of a stream is raw bytes). The door answers both commands with reply 7 (command not supported).

2026-11-13: UDP is built on the service side with the command `0x83` of `pod-proxy` (T-329): datagrams are frames on the stream (`docs/proxies.md`, "UDP in the stream"). Left, and why:
1. A local door for UDP. `link dial` copies stdin and stdout and has no SOCKS5 code. A plain SOCKS5 client with a UDP socket needs a local listener that turns its datagrams (command 3) into frames. No such listener exists in `link dial` or `forward`. The opener writes the frames itself.
2. `--resolve local`. The connector end carries raw bytes and cannot parse SOCKS5: the name stays in the request, and the peer resolves it. It needs the local door of item 1.
3. `BIND`. It needs a second stream from the peer to the opener. The connector does not accept streams (the same limit as T-297). The door answers with reply 7.
`UDP ASSOCIATE` (command 3) gets reply 7 at the peer, as its UDP socket would be at the peer.

2026-11-13, later: the three items are built, with three changes to the text above. (1) `BIND` needs no second stream: RFC 1928 puts the incoming connection on the control connection, which is the stream. The service listens at the peer (loopback, or `--socks-bind-ip`), and replies 1 and 2 and the bytes of the incoming connection pass on the one stream. (2) The local door is `link dial --service socks5 --listen ADDRESS`; no `forward -L` form was added. (3) The flag for names is `--resolve-at peer|local`: `--resolve` is the global flag of the pins (`HOST=IP`).

## Done

2026-11-12, commit 97c7196. `--service socks5` (no target; `socks5=TARGET` exits 64) of `link listen` runs `podnet_netns::door::Door` (the code of the front door, no second SOCKS5 server) on the session stream with a direct dial (`src/link_socks.rs`). A name resolves at the peer. The stream takes SOCKS5 only (another first byte ends it with no answer). The allow list admits the peer before any service (a stranger exits 77). Also fixed: `link dial` did not exit when the far end ended the stream while stdin was open (a blocking stdin read held the runtime); and a stream that ends in error now gets a clean shutdown at the listener.
`cargo test --test link_socks5`: 7 passed: `a_connect_through_the_peer_reaches_an_origin_at_the_peer`, `a_name_resolves_at_the_peer`, `a_connect_to_a_closed_port_gets_a_failure_reply_and_the_stream_ends`, `bind_and_udp_associate_are_not_served_and_say_command_not_supported`, `a_stream_that_does_not_speak_socks5_ends_with_no_answer`, `a_peer_that_is_not_on_the_allow_list_gets_77_before_the_service`, `the_service_socks5_takes_no_target`. Fail first: on the old code 6 of the 7 failed and the seventh hung (no service `socks5`). The tests need a loopback and skip without one.
Not proven: BIND, UDP ASSOCIATE, `--resolve local`, user and password, a carrier that crosses a network.

2026-11-13, UDP in the stream: see the second Correction.

2026-11-13, commit 6432a62. UDP in the stream at the service `socks5`: the door takes the command `0x83` of `pod-proxy` with a UDP socket of the host of the listener (`src/link_socks.rs`). Command 3 gets reply 7 (`exit_stdio::NoAssociate`). Help and `docs/link.md` are updated.
`cargo test --test link_socks5`: 8 passed: the 7 of the first commit (`bind_and_udp_associate_say_command_not_supported` has the old name `bind_and_udp_associate_are_not_served_and_say_command_not_supported` and now also checks command 3), and `a_udp_datagram_goes_through_the_peer_to_an_echo_and_back` (two frames in one write, a name in a frame). Fail first: on the old code the new test failed (reply 7 to command 0x83).
Not proven or not built: BIND, the local door for UDP, `--resolve local` (see the Correction of 2026-11-13), a carrier that crosses a network.

2026-11-13, commit ce6a5ac (the entry is `done`). `BIND` at the service `socks5` (`src/link_socks.rs`, an `Extension` of the door for command 2): reply 1 with the listening address, one connection (30 s, else reply 6), reply 2 with its source, then the copy. `--socks-bind-ip IP` (default 127.0.0.1) sets the address. The local door `src/link_door.rs` (`podnet::link_door`, `src/cli/cmd/link/door.rs`): `podnet_netns::door::Door` over a `Way` that opens one stream of the link for each request. `CONNECT` goes as a SOCKS5 `CONNECT` to the service of the peer; `--resolve-at local` resolves a name at the opener and sends an address; `BIND` passes the request and the replies; `UDP ASSOCIATE` gives the client a local UDP socket and carries the datagrams as the frames of the command `0x83` (`Frames`, a `Datagrams` over one stream; a name in a datagram resolves at the opener).
`cargo test --test link_socks5`: 9 passed, among them `a_bind_through_the_peer_listens_at_the_peer_and_relays_an_incoming_connection` (the old test `bind_and_udp_associate_say_command_not_supported` is `udp_associate_says_command_not_supported` and no longer checks BIND). `cargo test --test link_door`: 7 passed: `a_connect_through_the_local_door_reaches_an_origin_at_the_peer`, `a_connect_to_a_closed_port_gets_a_failure_reply`, `a_name_goes_to_the_peer_and_resolves_there_or_here`, `the_resolve_choice_decides_the_address_type_the_peer_gets` (address type 3 for `peer`, 1 for `local`), `a_bind_through_the_local_door_listens_at_the_peer`, `a_udp_datagram_goes_through_the_local_door_to_an_echo_and_back`, `listen_needs_the_service_socks5`. Fail first: on the old code BIND got reply 7, and `--listen` did not exist.
Not proven: user and password (the link admits the peer); a carrier that crosses a network; the reply code of a failed `CONNECT` at the client is coarse (the door maps the text of the failure); the source of the incoming `BIND` connection is not checked against the address of the request; the `ASSOCIATE` association with many clients at one door.


# T-292: link: an HTTP forward proxy service at a peer (GitHub #471)

**Source:** GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E5-2 of GitHub #471 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: HTTP forward proxy. The status in the issue is "partial". The row is done when: link_http (tests/link_http.rs).

The rules of the issue that apply to this row:

- One request for each framing. Absolute-form and `CONNECT`. A target is a peer service by name, with no DNS at the proxy.
- Request headers are 8 KiB at most. A `CONNECT` has no request body. No caching.
- The shared rules of T-291 apply.

## Premise

- The proxy exists in `src/proxy.rs` (`serve_stream`, `src/proxy.rs:43`) and in the front door of `crates/podnet-netns/src/frontdoor.rs`. A peer-scoped name target does not exist.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Register a service `http` on a session stream. Resolve a target name against the services of the peer, not DNS.
2. Apply the header and body limits. Test absolute-form and `CONNECT`.

## Prove

```sh
cargo test --test link_http
sh scripts/agent/check.sh
```

## Start condition

T-266 is done.

## Done

2026-11-13, commit ff0d7c0. `src/link_http.rs` (`podnet::link_http`) is the service `http`: `--service http` (no target) runs `podnet_netns::door::Door` on a session stream with a dialer that maps a host to the origin of a service of the listener (`--service NAME=TARGET`, `--to` is `raw`). The door takes `CONNECT` and the absolute form (`GET http://NAME/path`, sent on in origin form with `Connection: close`); no second HTTP proxy was written. A host that is no service name, and each IP address, gets 403 and there is no DNS lookup. The request port is not used. The service reads the head first: 8 KiB at most (431), a `CONNECT` with a body gets 400; the door gives 400 for origin form. The allow list is applied before the service (exit 77 for a stranger). `--service http=TARGET` exits 64. `docs/link.md` ("The service `http`"), the help of `--service`, `llms-full.txt` and `scripts/check-layers.py` are updated.
`cargo test --test link_http`: 6 passed: `a_connect_through_the_peer_reaches_an_origin_at_the_peer`, `an_absolute_form_request_reaches_an_http_origin_in_origin_form`, `a_host_that_is_no_service_name_gets_403_and_no_lookup`, `a_request_the_service_does_not_take_is_refused`, `a_peer_that_is_not_on_the_allow_list_gets_77_before_the_service`, `the_service_http_takes_no_target`. Without the change (no `http` service) the run did not finish: the listener took `http=...` as a target and the test waited. `sh scripts/agent/check.sh`: all 0. Unit test `a_connect_with_a_body_is_bad`.
Not proven: keep-alive or pipelined requests (one request for each stream); a request body larger than the head limit in absolute form (it passes unchecked); HTTP over a TCP or other carrier (only the unix carrier is tested); `Proxy-Authorization` (the link admits the peer).


# T-293: link: a raw passthrough service at a peer (GitHub #471)

**Source:** GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Row E5-3 of GitHub #471 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Raw passthrough. The status in the issue is "partial". The row is done when: link_raw (tests/link_raw.rs).

The rules of the issue that apply to this row:

- Raw bytes, no framing. Half-close is propagated. A record is 1 MiB.
- The shared rules of T-291 apply.

## Premise

- A forward path exists (`src/cli/cmd/forward.rs`). A `raw` service of a peer does not exist.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Register a service `raw` that copies the stream to a configured target and back, with half-close. Test the half-close in both directions.

## Prove

```sh
cargo test --test link_raw
sh scripts/agent/check.sh
```

## Start condition

T-266 is done.

## Done

2026-11-12, commit 931277a. What was there before: T-266 built the service `raw` (`--to TARGET` is `--service raw=TARGET`; `bridge` in `src/cli/cmd/link/listen.rs` copies with `tokio::io::copy_bidirectional`, which passes the end of one direction as a shutdown of the write side of the other). No code needed a change. This entry adds the tests that prove it and the text in `docs/link.md` (the part "The service `raw`"), the help of `--to` and `llms-full.txt`.
`cargo test --test link_raw`: 4 passed: `a_close_of_the_opener_reaches_the_origin_and_the_answer_still_comes` (stdin of `link dial` ends, the origin reads to the end, then answers), `a_close_of_the_origin_reaches_the_opener_and_the_opener_can_still_send` (the origin closes its write side, the opener reads the end, then sends and the origin reads it), `four_mebibytes_each_way_arrive_unchanged`, `the_service_raw_has_no_framing_a_single_byte_stays_a_single_byte`. Fail first: with `bridge` changed to stop at the first end of one direction (no half-close), 3 of the 4 tests failed; the old code passes all 4.
Not proven: a record of 1 MiB as a limit (the copy has no record size of its own; a frame limit is the link's); a carrier that crosses a network.


# T-294: link: a Shadowsocks AEAD service at a peer (GitHub #471)

**Source:** GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-5.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** blocked

## Problem

Row E5-5 of GitHub #471 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Shadowsocks AEAD. The status in the issue is "planned". The row is done when: service_ss (tests/service_ss.rs), with the published SIP004 vectors, not a hand-written round trip against itself.

The rules of the issue that apply to this row:

- AEAD per SIP004: `chacha20-poly1305` and `aes-256-gcm`. HKDF-SHA1 per the spec. A replay window. TCP only.
- This is the SIP004 AEAD construction, not the later `2022-blake3-*` protocol. Do not label it "AEAD-2022".
- The service is behind the feature `ss` (T-311).
- The shared rules of T-291 apply.

## Premise

- No Shadowsocks service or client exists in podnet. The clients are planned in `pod-egress` (`PodUtils/podcrates:docs/design.md`, the row of `pod-egress`: `shadowsocks/` of podssh). T-127 covers the client road.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Take `pod-egress` (T-127, T-137). Ask podcrates (ask A4) for the chunk codec of SIP004 as a part that works for the accept side, with the published vectors in its tests.
2. Write the service on a session stream: read the salt, derive the subkey, open the chunks, connect to the target.

## Decision

The codec belongs to `pod-egress`. This entry is the service side of the take T-127.

## Prove

```sh
cargo test --test service_ss --features ss
sh scripts/agent/check.sh
```

## Blocker

PodUtils/podcrates must release `pod-egress` (T-127, T-137) and answer ask A4 (a COORD issue to podcrates): the server half of the SIP004 codec. It unblocks when the release exists.

## Start condition

T-127 is done (the take of `pod-egress`), and T-266 and T-311 are done.

# T-295: link: a Trojan service at a peer (GitHub #471)

**Source:** GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-6.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** blocked

## Problem

Row E5-6 of GitHub #471 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Trojan. The status in the issue is "planned". The row is done when: service_trojan (tests/service_trojan.rs).

The rules of the issue that apply to this row:

- TLS 1.3. The Trojan password is hashed. The inner request is parsed.
- A static response on an authentication failure. No version and no peer echo that identifies the service.
- The service is behind the feature `trojan` (T-311).
- The shared rules of T-291 apply.

## Premise

- No Trojan service or client exists in podnet. The client road is planned in `pod-egress` (T-127).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Take `pod-egress` (T-127, T-137). Ask podcrates (ask A4) for the Trojan request codec (hash, command, address) as a part that works for the accept side.
2. Write the service: TLS accept, password check, the static response, the splice.

## Decision

The codec belongs to `pod-egress`. This entry is the service side of the take T-127.

## Prove

```sh
cargo test --test service_trojan --features trojan
sh scripts/agent/check.sh
```

## Blocker

PodUtils/podcrates must release `pod-egress` (T-127, T-137) and answer ask A4 (a COORD issue to podcrates): the server half of the Trojan codec. It unblocks when the release exists.

## Start condition

T-127 is done (the take of `pod-egress`), and T-266 and T-311 are done.

# T-296: link: an exec adapter that runs a program for each stream (GitHub #471)

**Source:** GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-7.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E5-7 of GitHub #471 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Exec adapter. The status in the issue is "planned". The row is done when: service_exec (tests/service_exec.rs).

The rules of the issue that apply to this row:

- An absolute program path. No shell. No inherited environment beyond an allow list.
- `user` only with privilege. `argv` is an array. stdin and stdout are the stream. stderr goes to the log.
- The shared rules of T-291 apply.

## Premise

- No adapter runs a program for each stream. `SCHEME+exec:COMMAND` of the egress registry runs `sh -c` for a proxy connection (`docs/egress.md`). It uses a shell and is a dial, not a service.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Register a service `exec` from a config entry with the path, the argv array, the environment allow list and the optional user.
2. Refuse a relative path and a shell. Test the environment cut and the stderr to the log.

## Prove

```sh
cargo test --test service_exec
sh scripts/agent/check.sh
```

## Start condition

T-266 is done.

## Done

2026-11-13, commit 2295ce4. `src/link_exec.rs` (`podnet::link_exec`) is the service `exec`: `link listen --service NAME=exec:/ABS/PATH` with `--exec-arg NAME=ARG` (an array, repeat it), `--exec-env NAME=VAR` (the allow list; the environment is empty beyond it) and `--exec-user NAME=USER` (root only; the setup exits 64 without root; the program runs with the uid and gid of the user and no supplementary groups). A relative path, a `..`, a missing file and a shell by name exit 64. stdin and stdout are the stream, stderr goes to the log, the working directory is `/`. At most 64 programs run at once (answer 4 over the limit; answer 2 when the program cannot start). `docs/link.md` has the section "The service `exec`"; `llms-full.txt` is generated again.
`cargo test --test service_exec`: 7 passed: `a_program_that_echoes_answers_on_the_stream`, `the_arguments_are_an_array_and_no_shell_expands_them`, `a_relative_path_is_refused`, `the_environment_holds_only_the_allowed_names`, `the_stderr_of_the_program_does_not_reach_the_stream`, `a_program_that_cannot_start_gets_exit_69`, `a_user_needs_root`. With `env_clear` disabled, `the_environment_holds_only_the_allowed_names` failed (the variables of the test process reached the program). `cargo test --lib link_exec`: 1 passed. `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test all 0.
Not proven: `--exec-user` as root (the test runs as root in one session and skips; without privilege it checks the refusal); the text of the log line in a test (the stderr is proven to stay out of the stream); a program that ignores a reset (the kill is by `kill_on_drop`).


# T-297: link: serve a file tree at a peer (GitHub #471)

**Source:** GitHub #471 (https://github.com/PodUtils/podnet/issues/471), row E5-8.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E5-8 of GitHub #471 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Serve a file tree at a peer. The status in the issue is "partial". The row is done when: service_serve_at (tests/service_serve_at.rs).

The rules of the issue that apply to this row:

- `serve DIR --at peer:<fp>/http` (or `--name`). Index, range and `HEAD` work as today. No directory listing unless asked.
- The shared rules of T-291 apply.

## Premise

- `serve` takes `--local` and `--ingress` (`crates/podnet-serve/src/ingress.rs`, `src/cli/cmd/serve.rs`). `--at` does not exist.
- The ruling "Way out and way in" (2026-11-11, T-060) set `--ingress tunnel|relay|none` as the road to the visitors of `serve`. The flag `--via` is gone (`src/cli/help.rs:190`).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add the ingress kind `peer:<fp>/<name>` to `--ingress`, and accept the spelling `--at` of the issue as an alias if the owner wants it. The call is in the Decision.
2. Serve the tree on a session stream with the file server of `podnet-serve`. Keep the listing off unless `--list` style options ask for it.

## Decision

2026-11-12, a call of the lead: the way in stays `--ingress`, with the new kind `peer`. `--at` is an alias at most. The owner can rule otherwise.

## Prove

```sh
cargo test --test service_serve_at
sh scripts/agent/check.sh
```

## Start condition

T-266 is done.

## Correction

2026-11-13: the entry is `partial`. The way in is `podnet serve DIR --at peer:FINGERPRINT/SERVICE --at-listen ADDRESS`, not a kind `peer` of `--ingress`: a link has a listener that accepts streams and a connector that only opens them (T-266), so the serving process must be the listener, and `--at-listen` gives its address. The kind `peer:FP/NAME` of `--ingress` and the form `--name` of the issue are left: `--name` names the instance of `serve` (`--detach`), and a live name of a link is T-308. The call "the way in stays `--ingress`" of 2026-11-12 is not followed; the owner can rule otherwise.

2026-11-13, later: the form `--ingress peer:FINGERPRINT/SERVICE` is built (the ruling of 2026-11-12 holds). `--at` and `--at-listen` stay. `--name` stays out: `serve --name` names the instance of `serve` (`--detach`, T-308), so a service name for a link has no flag of its own; the SERVICE of the spec is the name.

## Done

2026-11-13, commit d8960cb. `src/cli/cmd/serve_at.rs`: `--at peer:FP/SERVICE` with `--at-listen ADDRESS` runs the router of the file server of `podnet serve` (`podnet::files::router`, no second server) on a unix socket in a private directory (mode 0700, removed at the end), and runs `link listen` in the process (`ListenArgs` parsed from a list, so a new flag of `link listen` needs no change here) with the allow list `{FP}` and the service SERVICE whose target is that socket. Index, range, `HEAD`, `--list`, `--spa`, `--tar`, `--upload`, `--users` and `--pin` are the ones of the file server; a directory has no listing unless `--list` is given. A bad spec, `--at` without `--at-listen`, and `--at` with `--ingress`, `--pair`, `--local` or `--check` exit 64. `docs/serve.md` has "Serve at a peer (`--at`)"; `docs/link.md` points to it.
`cargo test --test service_serve_at`: 8 passed: `a_get_of_a_file_answers_the_bytes`, `a_directory_with_an_index_answers_the_index`, `a_range_answers_206_with_the_bytes_of_the_range`, `a_head_answers_the_head_and_no_body`, `a_directory_has_no_listing_by_default`, `a_listing_comes_only_with_list`, `a_peer_that_is_not_the_one_of_at_is_refused` (exit 77), `at_needs_at_listen_and_a_good_spec`. Without the change 7 of the 8 failed (`error: unexpected argument '--at' found`). `cargo test --lib serve_at`: 1 passed. `sh scripts/agent/check.sh`: all 0 (the record step after `cargo todo remap` of `TODO/fixes.md`).
Not proven: the kind `peer` of `--ingress` and `--name` (see the Correction); a carrier that crosses a network; uploads and users on a link stream (the router is the same, no test).

2026-11-13, commits 3fe43ed and fa5f2c2 (the entry is `done`). 3fe43ed: `podnet link connect --service NAME=TARGET` serves the streams that the far end opens, with the service forms of `link listen` (raw target, `socks5`, `http`, `exec:`, `--exec-arg`, `--exec-env`, `--exec-user`, `--socks-bind-ip` came later). The code of the services moved to `src/cli/cmd/link/services.rs` and is shared by `link listen` and `link connect`. The grant of the peer in the `--allow` file applies before any service (answer 6, counted in `refused_service`); a stream for a service that the connector lacks gets answer 1. fa5f2c2: the verb `accept` of the control socket (`src/link_ctl/accept.rs`) registers a service that maps to a unix socket and lives while the client keeps the connection; `podnet serve DIR --ingress peer:FP/SERVICE` binds the file server on a unix socket in a private directory and registers it at the running instance (found by `PODNET_LINK_CONTROL` or the ready documents, as `forward` finds it). The target is a unix socket only (no TCP target and no program through the control socket). A name that the instance serves already, or that another client holds, exits 64; no instance exits 69; a link to another peer exits 77.
`cargo test --test link_accept`: 5 passed: `a_stream_that_the_far_end_opens_reaches_the_service_of_the_connector`, `a_service_that_the_connector_lacks_closes_the_connection_and_the_link_stays`, `a_connector_with_no_service_refuses_each_stream`, `a_grant_of_the_allow_list_applies_before_the_service`, `the_connector_gives_a_bad_service_flag_exit_64`. Fail first (old `src`): 3 of 5 failed (`--service` unknown at `link connect`); the two others pass on the old code too (a refusal and a usage exit).
`cargo test --test service_serve_ingress`: 6 passed: `a_get_and_a_range_answer_on_streams_that_the_peer_opens`, `a_grant_that_does_not_name_the_service_refuses_the_stream`, `the_service_ends_with_the_serve_process`, `a_name_that_the_connector_serves_already_exits_64`, `with_no_link_instance_serve_exits_69`, `a_bad_spec_or_a_second_road_exits_64`. Fail first (old `src`): 4 of 6 failed.
Not proven: a carrier that crosses a network (only the unix carrier is tested); uploads and users on a stream of this road (the router is the same); `--ingress peer:` with `--detach`.


# T-298: link: the AEAD record layer with negotiated suite and rekey (GitHub #472)

**Source:** GitHub #472 (https://github.com/PodUtils/podnet/issues/472), row E6-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

Row E6-1 of GitHub #472 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: AEAD record with rekey. The status in the issue is "partial". The row is done when: link_record (tests/link_record.rs); with T-287, mesh_three_hops shows the middle hop receiving records it cannot decrypt; a suite mismatch exits 78.

The rules of the issue that apply to this row:

- TLS protects a carrier hop. A mesh hop must not read the streams it forwards, so the record layer sits between the session and the carrier with its own AEAD, counters, a replay window and a resume record.
- The wire header is 24 bytes: `version` u8, `type` u8, `flags` u16, `stream` u32, `counter` u64, `length` u32, `pad` u16, `reserved` u16. `length` counts the payload plus the padding. `pad` gives the padding length, so the payload is `length - pad`. The tag is 16 bytes.
- The suites are `chacha20-poly1305` (default) and `aes-256-gcm`. They are negotiated in `Hello` and in the TLS exporter.
- Keys come from `EXPORTER-podnet-record` with the suite name and the transcript hash as context. There is no separate key exchange.
- The nonce is a 4-byte prefix for each direction plus the 64-bit counter.
- Rekey at 2^31 records or 1 GiB, whichever comes first. The new key is active before the old key is dropped.
- A suite mismatch exits 78.

## Premise

- TLS is present: `pod-tls` of podcrates v0.28.0 and `rustls 0.23` (`Cargo.toml`). The record AEAD is absent.
- A flaw in the rule: the TLS exporter belongs to one TLS connection. A middle hop ends the TLS connection on each side. It knows both exporter keys. A key from the exporter of a hop cannot make a record that the middle hop cannot read.
- `pod-e2e` of podcrates v0.28.0 is an end-to-end channel: Noise XX over the bytes of a session, with the identity key of `pod-identity` (`PodUtils/podcrates:crates/pod-e2e/src/lib.rs`). It gives a key that only the two ends hold.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Take the end-to-end keys from the handshake of `pod-e2e` between the two ends (T-086). Use the TLS exporter only on a single hop. Ask podcrates (ask A3) for the handshake hash and the transport keys of `pod-e2e` as an interface, so that the record layer can run its own AEAD.
2. Write the header, the two suites, the nonce, the counters and the rekey. A suite mismatch exits 78.
3. Test that a tampered header or tag fails, and that the key is replaced at the limit.

## Decision

2026-11-12, a call of the lead: the record keys of a multi-hop path come from an end-to-end handshake, not from the exporter of a hop. The issue text ("keys come from the TLS exporter") holds for a single hop only. The owner can rule otherwise.

## Prove

```sh
cargo test --test link_record --test link_record_mesh --test link_seal
sh scripts/agent/check.sh
```
## Start condition

T-086 is done (the take of `pod-e2e`), and T-262 is done.

## Correction

2026-11-13: The record layer of `pod-e2e` is in podcrates `v0.38.0` (taken, c65090d): the block is gone.

2026-11-12, the lead: the hello of T-265 (`src/hello.rs`) proves the keys of the two ends but makes no key for the link and does not bind the streams after it. It holds for the unix and memory carriers only. A carrier that crosses a network does not rely on it: this entry gives the link its keys from the end-to-end handshake of `pod-e2e`, bound to the session, before the carrier is offered.

## Done

2026-11-13, commits ea9b6c0 and e109559 (branch t298). `Cargo.toml` takes `pod-e2e` (tag v0.38.0). `src/link_record.rs` (`podnet::link_record`) is the glue: `dial`, `accept` and `accept_with` run the handshake of `pod-e2e` on one stream (Noise XX with the key of each end), agree on the suite (`chacha20-poly1305` by default, `aes-256-gcm`), take the records from the exporter, and give a `Sealed` byte stream. The keys are the two ends' keys, so a hop that forwards the stream sees records only. A suite mismatch exits 78, a key that is not expected or not in the allow list exits 77. A stream that ends with no FIN record is a cut (`UnexpectedEof`). `--seal` of `link listen`, `link connect` and `link dial` runs it on the streams of a service (`src/cli/cmd/link/services.rs`, `dial.rs`).
`cargo test --test link_record`: 8 passed: `a_sealed_stream_echoes` (300 KB), `each_suite_carries_the_stream`, `a_suite_mismatch_exits_78`, `a_peer_that_is_not_the_expected_key_exits_77`, `a_tampered_record_is_refused`, `a_changed_header_is_refused`, `a_cut_stream_is_not_a_clean_end`, `the_key_changes_at_the_limit_and_the_stream_goes_on` (rekey after 3 records, both phases seen). `cargo test --test link_record_mesh`: 2 passed: `a_middle_hop_sees_no_plaintext_of_a_sealed_stream` (A to C through B with the real `link_forward`; the bytes that B reads from its links hold no marker) and its control `without_the_layer_the_middle_hop_sees_the_plaintext`. `cargo test --test link_seal`: 5 passed (CLI), among them `reconnect_of_a_sealed_link_connect_gives_a_sealed_new_link` (T-307: the redial state of `link connect` keeps the seal setting; the new link serves the sealed service, and with the seal setting dropped the test fails). The end-to-end item of T-287 is done by this: B sees records only.
Not proven: a rekey at 2^31 records or 1 GiB (the test uses a limit of 3 records; `pod-e2e` tests the real limits); a chain of three podnet processes (the CLI has no node that listens and dials). Not done: the local door of `link dial --listen` and the control socket do not seal; `--seal` is an option at both ends, not a default.



# T-299: link: snappy compression of records (GitHub #472)

**Source:** GitHub #472 (https://github.com/PodUtils/podnet/issues/472), row E6-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Row E6-2 of GitHub #472 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Compression. The status in the issue is "planned". The row is done when: link_compress (tests/link_compress.rs).

The rules of the issue that apply to this row:

- Snappy only. Off by default (`--compress snappy`, the query key `compress`).
- Never on a record under 256 bytes. The decompressed size is capped at 8 MiB and the ratio at 64 to 1. A record over either cap closes the stream with `decompress-limit`.
- A compressor that expands the record sends it uncompressed and sets no flag.

## Premise

- No snappy record path and no `compress` key exist. `Cargo.lock` has no snappy crate.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Choose a pure Rust snappy crate that passes `cargo deny`. Write the choice in a Correction.
2. Write the flag, the caps and the no-expansion rule in the record layer (T-298). Test each cap.

## Prove

```sh
cargo test --test link_compress
sh scripts/agent/check.sh
```
## Start condition

T-298 and T-306 are done.

## Correction

2026-11-13: The record layer of `pod-e2e` is in podcrates `v0.38.0` (taken, c65090d): the block is gone; the compression runs before its seal.

2026-11-13, the choice of the crate: no snappy crate was in `Cargo.lock`. `snap` 1 (pure Rust, BSD-3-Clause) is added; `cargo deny --locked check bans sources licenses` passes.

## Done

2026-11-13, commits ea9b6c0 and e109559 (branch t298). `src/link_record/pack.rs`: `pack` compresses a payload of 256 bytes or more and returns `None` when the result is not smaller (no flag); `unpack` refuses a payload that says more than 8 MiB or more than 64 times its own size (`decompress-limit`). The flag is `0x0002` of the record. The query key `compress=snappy` has an effect with `--seal` (without `--seal`: exit 64); `compress` of `src/link_query.rs` is wired. `cargo test --test link_compress`: 3 passed: `compression_shrinks_a_compressible_payload_and_the_data_stays_equal` (380000 bytes go out as under 95000; with the compression disabled the test failed: "380520 bytes for 380000"), `a_payload_that_does_not_shrink_or_is_short_goes_with_no_flag`, `a_compressed_record_over_the_cap_closes_the_stream_with_decompress_limit` (a real sealed record that says 9 MiB). Unit tests of `pack.rs` (6) test each cap, also the ratio of 64 to 1.
Not proven: a compressed stream over a mesh hop; a decompress bomb inside the caps.

# T-300: link: padding of records to buckets (GitHub #472)

**Source:** GitHub #472 (https://github.com/PodUtils/podnet/issues/472), row E6-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Row E6-3 of GitHub #472 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Padding. The status in the issue is "planned". The row is done when: link_pad (tests/link_pad.rs): a tampered `pad` fails authentication.

The rules of the issue that apply to this row:

- The buckets are 0, 128, 512, 1024 and 2048 bytes. The record is padded to the next bucket.
- `length` includes the padding. `pad` gives its length. `pad` is in the AEAD associated data, so it cannot be edited.
- Padding is removed before the record reaches the session.

## Premise

- No padded record and no `pad` key exist.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the bucket choice, the associated data and the removal in the record layer (T-298). Test a tampered `pad`.

## Prove

```sh
cargo test --test link_pad
sh scripts/agent/check.sh
```
## Start condition

T-298 and T-306 are done.

## Correction

2026-11-13: The padding of `pod-e2e` is in podcrates `v0.38.0` (taken, c65090d): the block is gone.

## Done

2026-11-13, commits ea9b6c0 and e109559 (branch t298). The padding is the one of `pod-e2e` (`Settings::pad`); `Options::pad` and the query key `pad=on` (with `--seal`) set it. `cargo test --test link_pad`: 2 passed: `padding_gives_sizes_from_the_buckets` (payloads of 1, 100, 128, 200, 600, 1500 and 3000 bytes give bodies of 128, 128, 128, 512, 1024, 2048 and 4096 bytes, as the middle hop sees them; the reader gets the exact bytes) and `a_tampered_pad_fails_authentication` (the relay lowers `pad` by 1; the reader gets an error with "check").
Not proven: the padding of a stream with the compression on at the same time.



# T-301: link: the replay window and the RESUME record (GitHub #472)

**Source:** GitHub #472 (https://github.com/PodUtils/podnet/issues/472), row E6-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E6-4 of GitHub #472 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Replay window and resume records. The status in the issue is "planned". The row is done when: link_replay (tests/link_replay.rs): a replayed record is counted and dropped.

The rules of the issue that apply to this row:

- A bitmap of 1024 bits behind the highest counter. A record at or below `highest - 1024` is dropped. A duplicate inside the window is dropped and counted.
- RESUME carries the resume token, the next expected counter and a fresh prefix. The token is valid for 10 minutes and is single-use. Three failures close the link with `ResumeExpired`.

## Premise

- No record-level replay bitmap and no RESUME record exist. The session resume (offsets, secret) is T-263.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the bitmap and the drop rules in the record layer (T-298).
2. Write the RESUME record. Share the token and the 10-minute window with T-263: there is one resume, not two.

## Prove

```sh
cargo test --test link_replay
cargo test --test link_session -- resume
sh scripts/agent/check.sh
```
## Start condition

T-298 and T-263 are done.

## Correction

2026-11-13: The replay window and RESUME of `pod-e2e` are in podcrates `v0.38.0` (taken, c65090d): the block is gone.

2026-11-13, partial. Done: the replay window (1024 counters; a repeat inside it and a record below it are dropped and counted: `Stats::replayed`), the RESUME record (`Sealed::resume`; the peer sends again, in new records, the payloads from the counter that the RESUME names, from a log of 1 MiB; a copy of a RESUME is refused and counted: `Stats::refused`). Commits ea9b6c0 and e109559 (branch t298). `cargo test --test link_replay`: 4 passed: `a_replayed_record_is_refused`, `a_record_below_the_window_is_dropped_and_counted` (a record kept back for 1100 records), `resume_sends_again_what_the_peer_did_not_get_and_is_used_once`, `a_resume_that_comes_twice_is_refused_the_second_time`.
Left: (1) the RESUME is not sent by the resume of T-263: the CLI does not send it after a lost carrier, and the layer runs over one stream, not over a new carrier of `pod-session`. The token and the 10-minute window are those of `pod-e2e`; they are not yet shared with `session::RESUME_WINDOW` in one code path. (2) The close of the link with `ResumeExpired` after three failed RESUME records is `Fault::ResumeExpired` of `pod-e2e`; the glue ends the stream with an error, not with the exit 69 of `ResumeExpired`.

2026-11-13, call 1 (where the RESUME belongs): the link of `podnet::session` runs over the application side of `pod-session`, and a sealed stream is a stream of that link. After a lost carrier, the session sends the lost bytes again from offsets, so the record layer sees no loss: a RESUME record there is not necessary and would repeat bytes. Measured: `a_sealed_stream_over_a_cut_session_carries_one_mebibyte_each_way` passes with no code of this entry. The RESUME record belongs where a sealed stream runs over a carrier with no `pod-session` below it (for example a path of the mesh, T-287): the library call `Sealed::rebind` (and `Sealed::rebinder`, a handle for a task that reads and writes the stream), with `Options::carry_over`. The CLI runs no session below a link and has no second carrier for a stream yet (T-265): it does not call `rebind`.

2026-11-13, call 2 (one window): `link_record::SETTINGS` is `pod-e2e`'s `Settings::DEFAULT` with `resume_window` set to `session::RESUME_WINDOW` (which is `pod-session`'s `RESUME_DEADLINE`, 600 s). The default `Options` use it. The wait for a new carrier after a cut reads the same `resume_window`. The crates do not allow one shared value at the source: `pod-e2e` has a 600 s constant in its own `Settings::DEFAULT`; podnet sets it from the session constant.

2026-11-13, call 3 (the order of a rebind): the receiver stops to read the old carrier first, and only then gives the write half of the new carrier to the sender, which seals the RESUME record. A first build let the sender seal the RESUME while the receiver still opened records of the old carrier: the peer sent those records again, and the test got 32 KiB twice (1081344 bytes, not 1048576). After a RESUME on a new carrier, the sender sends no new record until the RESUME of the peer comes, so that the records sent again come before the new ones.

2026-11-13, call 4 (the end of a sealed stream): `Fault::ResumeExpired` of `pod-e2e` (the third failed RESUME, or a RESUME after the window) and no new carrier in the window give an `io::Error` that holds `session::ResumeExpired` (with the window). `link_record::ended` makes it exit 69 with `ResumeExpired` in the chain. `link dial` exits with it; before, it ended a read error as an end of the stream (exit 0). The `link listen` side ends only the stream, not the process. A failed RESUME that ends the stream counts in `Stats::refused`.

2026-11-13, call 5 (`carry_over`): off by default. Off, a cut ends the stream at once (`UnexpectedEof`), as before; the CLI keeps this. On, a cut waits for a new carrier for the window, and the receiver stays after the FIN of the peer, so that a RESUME can still come.

Observation, not changed: `Sealed::resume` on the same stream (no new carrier) sends again each record from the counter of the RESUME. Records that were on their way, not lost, then come twice with new counters. It is correct only when the records on the stream were lost; a rebind does not have this case (call 3).

## Done

2026-11-13, commit b9bfd02. `src/link_record.rs`: `SETTINGS` (one window), `Options::carry_over`, `Sealed::rebind`, `Sealed::rebinder` and `Rebinder`, `ended` (exit 69). `src/link_record/pump.rs`: the sender and the receiver take a new carrier (the RESUME first, then a hold until the RESUME of the peer), the wait of the window after a cut, and `Fault::ResumeExpired` to `ResumeExpired`. `src/cli/cmd/link/dial.rs`: a read that `ResumeExpired` ended exits 69. `docs/link.md`, "Sealed streams": how a sealed stream resumes. `docs/link.md` is not in `llms-full.txt`: `scripts/gen-llms.py --check` agrees with no change.
`cargo test --test link_replay`: 8 passed, with the new `the_record_layer_and_the_session_share_one_window`, `a_sealed_stream_moves_to_a_new_carrier_with_no_lost_byte` (the first carrier is cut after 300 kB, both ends rebind to a new one: 1 MiB each way, equal SHA-256, no refused or repeated record), `three_failed_resume_records_end_the_stream_with_exit_69` (a relay changes each RESUME: 3 refused, exit 69, `ResumeExpired` with the window of 600 s), `no_new_carrier_in_the_window_ends_with_resume_expired` (window 1 s). `cargo test --test link_session -- resume`: 5 passed, with the new `a_sealed_stream_over_a_cut_session_carries_one_mebibyte_each_way` (carrier cut at 300 kB, 1 MiB each way, equal SHA-256, no RESUME and no repeat in the record layer). Each test passed 8 times in a row.
Plant: with the RESUME of a rebind off, the rebind test got 950272 of 1048576 bytes; with the map of `Fault::ResumeExpired` off, the exit 69 test failed with "not ResumeExpired"; with the window error off, the window test failed with "not ResumeExpired". With `resumes: false` in `session::settings`, the session test failed with `BrokenPipe`. The test of the one window has no plant: the two constants are 600 s in the crates; the test fails when one of them changes.
`sh scripts/agent/check.sh`: each step 0.



# T-302: link: split one session across an up carrier and a down carrier (GitHub #473)

**Source:** GitHub #473 (https://github.com/PodUtils/podnet/issues/473), row E7-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

Row E7-1 of GitHub #473 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Direction split. The status in the issue is "partial". The row is done when: link_split (tests/link_split.rs): 1 MiB down over a fast half and 64 KiB up over a slow half; killing the up half closes the session with `up-carrier lost`; the router split tests pass unchanged.

The rules of the issue that apply to this row:

- `--up-carrier` and `--down-carrier` join one session.
- The control channel runs on the downstream carrier, with an upstream fallback.
- Flow control counts each direction apart. One carrier for each direction. The slower half sets the throughput.
- A half failure is detected within 30 s and reported as `up-carrier lost` or `down-carrier lost`. There is no silent promotion of one half to both directions.

## Premise

- The router has a UDP direction split, off by default (`splits:`, T-179). The link session split does not exist.
- T-179 lists as left: a TCP split needs a far end that joins two connections. A link session is such a far end.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Let the session (T-262) bind two carriers by role. Account the credits for each direction.
2. Write the half-failure detector (30 s) and the two reports.
3. Test with a fast down carrier and a slow up carrier. Run the existing split tests of the router without a change.

## Prove

```sh
cargo test --test link_split
cargo test -p podnet-core network::split
sh scripts/agent/check.sh
```

## Blocker

The carrier is a take of `pod-link` of PodUtils/podcrates, its entry 500 in the record of podcrates (PodUtils/podcrates#133,
2026-11-13; estimate: tag `v0.39.0`).

## Start condition

T-262 is done and two carriers of E1 exist (T-268, T-274, T-275).

# T-303: link: chain composition with a peer element and --check (GitHub #473)

**Source:** GitHub #473 (https://github.com/PodUtils/podnet/issues/473), row E7-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E7-2 of GitHub #473 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Chain composition. The status in the issue is "partial". The row is done when: link_chain (tests/link_chain.rs): two elements reach a local proxy then a local carrier; a missing element exits 69 and names it; a cycle exits 78; `--check` prints each element and leaves no listener.

The rules of the issue that apply to this row:

- Elements apply in order. The last element reaches the carrier address. There is no implicit `direct` at the end.
- A chain that cannot be built exits 69 before the carrier opens, and names the element.
- `--check` resolves and dials each element once, prints the result and leaves no listener.
- 8 elements, 10 s for each element, no cycles. A duplicate element is refused.

## Premise

- The chain exists: `--egress` and `UpstreamChain` (`crates/podnet-vnet/src/dialer.rs:172`, `docs/egress.md`).
- A flaw in the issue: it says "`--via` is an alias for the existing `--egress`". The flag `--via` is gone (`src/cli/help.rs:190`), by the ruling "Way out and way in" of 2026-11-11.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Use `--egress` for the chain of a link. Do not add `--via`.
2. Add the `peer:` element (T-266), the limits, the duplicate and cycle checks (exit 78) and `--check`.
3. Map an element that cannot be built to exit 69 with its name.

## Decision

2026-11-12, a call of the lead: `--egress` is the one spelling (`docs/decisions.md`, 2026-11-11, Way out and way in). The owner can rule otherwise.

## Prove

```sh
cargo test --test link_chain
sh scripts/agent/check.sh
```

## Start condition

T-266 is done.

## Done

2026-11-13, commit 218d3f1. `podnet link dial` and `podnet link connect` take each `--egress` as one element before the `tcp` carrier (`src/cli/way.rs` keeps the elements in order; `src/link_chain.rs` is `podnet::link_chain`). The chain is the `Chain` of podnet-core: each element is a layer (`direct`, a SOCKS5 or HTTP proxy that opens its connection over the layer below, or `peer:FINGERPRINT/SERVICE`). The last element reaches the carrier address and no `direct` is added. `Plan::parse` exits 64 for more than 8 elements or a network that is not an element, and 78 for a duplicate element, two elements at one place, or an element at the carrier address. `Plan::open` resolves each element (10 s each) and brings the chain up before the carrier opens; a failure exits 69 and names the element. `Plan::check` resolves and dials each element once. `--check` is on `link dial` (already there) and now on `link connect` (no control socket, no name, no listener). `link_tcp::carrier_via` makes the carrier over a dial. `docs/link.md`, `docs/egress.md`, the help and the llms files have it.
`cargo test --features tcp --test link_chain`: 6 passed: `two_elements_reach_a_local_proxy_then_a_local_carrier`, `a_chain_of_two_elements_dials_through_the_cli_and_check_prints_each`, `check_of_connect_prints_each_element_and_leaves_no_listener`, `a_missing_element_exits_69_names_it_and_opens_no_carrier`, `a_cycle_or_a_duplicate_exits_78_and_too_many_elements_exit_64`, `a_chain_with_a_carrier_that_opens_no_connection_exits_64`. The unit test `egress_of_link_dial_and_connect_is_a_chain_in_order` is in `src/cli/way_tests.rs`. With the change of `way.rs` undone, 5 of the 6 tests failed (a chain of more than one `--egress` is "for podnet pod only"). `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers and test all 0; clippy and the two link tcp test files also pass with `--features full`.
Not proven: a `peer:` element in a running chain (only the missing-peer case, exit 69, is tested); an HTTP proxy as a later element and a SOCKS5 proxy with a password over a way below (the code is the same path as the SOCKS5 test, with no test of its own); the 10 s limit of an element (no test waits for it); the chain with `link listen` (it dials nothing, so it takes no chain).


# T-304: link: a C ABI with a header, a shared library and a static library (GitHub #474)

**Source:** GitHub #474 (https://github.com/PodUtils/podnet/issues/474), row E8.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** done

## Problem

Row E8 of GitHub #474 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: C ABI: `include/podnet.h`, `libpodnet.so` and `libpodnet.a`. The status in the issue is "planned". The row is done when: tests/c_abi.c opens a memory link, echoes 1 MiB and compares the digest; tests/c_abi.py does the same through `ctypes`; a `c-abi` CI job runs both, an ASan build and a valgrind run with no leaks; a test asserts that the exported symbols equal the list in the header.

The rules of the issue that apply to this row:

- The API: `podnet_init(config_json, err)`, `podnet_free`, `podnet_link_new(identity_path, peer_fp, out)`, `podnet_link_open(l, service, out)`, `podnet_link_accept(l, out)`, `podnet_stream_read`, `podnet_stream_write`, `podnet_stream_close`, `podnet_link_close`, `podnet_last_error`, `podnet_version`.
- Handles are opaque pointers. A function returns 0 on success and a negative code on failure. `podnet_last_error` is thread-local and valid until the next call on the same thread.
- No Rust panic crosses the boundary. Every entry point uses `catch_unwind` and maps a panic to `PODNET_PANIC`.
- `podnet_init` is idempotent. Strings are UTF-8 and NUL-terminated. Buffers belong to the caller. The library never frees a caller pointer.
- `read` and `write` block. `read` returns 0 at end of stream. Both return the byte count or a negative code.
- Bounds: 1 MiB for each call, a 64 KiB error string, no callback into the caller, one thread for each stream.
- The crate gets `crate-type = ["rlib", "cdylib", "staticlib"]`. A version script hides every symbol not in the header.

## Premise

- No `include/` directory and no `podnet.h` exist. `Cargo.toml` has `[lib]` with no `crate-type` (`Cargo.toml:56`).
- A flaw to resolve: the release binaries are static musl builds (`docs/build.md`). A `cdylib` does not build for a static musl target in the default setup. The shared library needs a dynamic target, or `-C target-feature=-crt-static`.
- `catch_unwind` needs `panic = "unwind"`. The release profile sets no `panic` key (`Cargo.toml` `[profile.release]`), so it unwinds.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the header and the entry points over the session API (T-262). Test on the memory carrier (T-275).
2. Add the feature `c-abi` (T-311) so that the shared library is opt-in. Build the shared library for a glibc target, and the static library for musl. Write the choice in `docs/build.md`.
3. Add the version script and the symbol-list test. Add the CI job with ASan and valgrind.

## Prove

```sh
cargo test --test c_abi --features c-abi
sh scripts/agent/check.sh
```

## Start condition

T-262, T-275 and T-311 are done.

## Correction

2026-11-13 (the lead's call; the owner can rule otherwise; `docs/decisions.md`, 2026-11-13): the crate types are not added to the root package, because a `cdylib` would build in every build. The C ABI is the workspace member `crates/podnet-capi` (crate types `cdylib`, `staticlib`, `rlib`), not a default member, over `podnet` with no feature. The header is `include/podnet.h`. The library crate is `podnet_capi` (a library named `podnet` collides with the root crate): the files are `libpodnet_capi.so` (soname `libpodnet.so`) and `libpodnet_capi.a`. The feature `c-abi` of the root stays a marker that gates no code, because a dependency that turns it on turns it on in each `--workspace` build (the test `the_default_build_holds_no_optional_feature` fails then). Prove: `cargo test -p podnet-capi` (not `--test c_abi --features c-abi`).

2026-11-13: the version script of the Approach is not used. For a `cdylib`, rustc writes its own version script and exports only the `#[no_mangle]` items of the crate. A second `--version-script` fails under rust-lld (`attempt to reassign symbol`). `crates/podnet-capi/tests/c_abi.rs` checks that `nm -D --defined-only` of the shared library equals the list of the header. A static library keeps the other symbols of Rust; the test checks the symbols `podnet_*` of it.

2026-11-13: the API of the issue has no way to learn the fingerprint of an install and no way to join two ends in one process. Two functions are added: `podnet_identity_fingerprint(dir, buf, len)` and `podnet_link_pair(dialer, listener)` (a memory carrier with the hello of each end). `podnet_free()` stops the library (it exits `PODNET_E_STATE` while a handle is open), and `podnet_stream_service(s)` gives the service name of a stream. `podnet_link_new` makes an end with the key of `identity_path` and one admitted `peer_fp`. A network carrier has no C function yet.

## Done

2026-11-13, commit 09ef55e. `include/podnet.h` declares 14 functions (the 11 of the issue, `podnet_identity_fingerprint`, `podnet_link_pair`, `podnet_stream_service`); `crates/podnet-capi/src/lib.rs` implements them over `podnet::link`, `hello` and `link_service` on a tokio runtime that `podnet_init` starts. Each entry point runs in `guard` (`catch_unwind`, thread-local error text of at most 64 KiB, codes -1 to -6 and -99). `docs/build.md` ("The C library"), `docs/link.md`, `docs/library.md`, `docs/decisions.md` and `README.md` say it; `llms-full.txt` is regenerated. `scripts/check-layers.py` has the crate (one module `capi`). The CI job `c-abi` is in `.github/workflows/ci.yml` and in `needs` of `all`.
Fail first: with a planted line `int podnet_planted(void);` in the header, `cargo test -p podnet-capi --test c_abi symbols` fails: `assertion left == right failed: dynamic symbols against the header` (the header has `podnet_planted`, the library does not). Without the plant: `cargo test -p podnet-capi`: 5 unit tests passed (`a_panic_is_mapped_to_the_panic_code_and_the_error_text`, `the_error_text_is_cut_to_64_kib_on_a_character_boundary`, `a_null_argument_is_a_bad_argument`, `init_is_idempotent_and_a_bad_config_is_refused`, `the_version_is_the_version_of_the_crate`) and 6 passed in `crates/podnet-capi/tests/c_abi.rs` (`the_header_lists_the_functions_of_the_api`, `the_exported_symbols_of_the_shared_library_equal_the_header`, `the_static_library_defines_the_functions_of_the_header`, `a_c_program_echoes_1_mib_over_a_memory_link_with_the_shared_library`, `a_c_program_echoes_1_mib_over_a_memory_link_with_the_static_library`, `ctypes_echoes_1_mib_over_a_memory_link`). The C test prints `c_abi: ok, 1 MiB echoed, digest ac51aa6881728030`; the ctypes test prints `c_abi.py: ok, 1 MiB echoed, sha256 ...`. Run by hand: the C test under `-fsanitize=address,undefined`: exit 0; under valgrind 3.22 (`--leak-check=full`, `--errors-for-leak-kinds=definite,indirect,possible`): 0 errors, no leak (the runtime has no signal driver and `podnet_free` joins its threads); `cargo rustc -p podnet-capi --target x86_64-unknown-linux-musl --crate-type staticlib` builds, and `cargo build -p podnet-capi --target x86_64-unknown-linux-musl` drops the `cdylib` with a warning.
Not proven: the CI job `c-abi` (not run here: the release build for the glibc target, the apt install of valgrind, the musl static library in CI); the ASan and valgrind runs were on a debug build of the library (the job uses the release build); no C function for a network carrier, and no `portable` build (T-305); the library has no test of a panic across the C boundary from a C program (a unit test checks `guard`); aarch64 and the other targets of the release set are not built.


# T-305: link: a portable build subset for Windows, macOS and FreeBSD (GitHub #475)

**Source:** GitHub #475 (https://github.com/PodUtils/podnet/issues/475), row E9.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** L
**Status:** blocked

## Problem

Row E9 of GitHub #475 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: A build that runs on Windows, macOS and FreeBSD, with the privileged and Linux-specific parts excluded. The status in the issue is "planned". The row is done when: a `portable` CI job builds and runs `podnet components --json`, a memory-carrier round trip and the C test on Windows, macOS and FreeBSD; a command that needs `netns` exits 69 with the feature name; the job is skipped only if the platform is absent from the runner matrix, and the skip is printed with the platform name.

The rules of the issue that apply to this row:

- `portable` excludes `podnet-netns`, the TUN path in `warp`, `podnet pod`, the vnet switch and raw ICMP. It keeps the session, the stream carriers the platform supports (`tcp`, `ws`, `h2`, `poll`, `sse`, `memory`, `unix` where available), the services that need no privilege, and the C entry point.
- A command that needs an excluded feature exits 69 with `built without feature X`. It is not a panic and not an empty result.
- Windows uses `AF_UNIX` where the build supports it. Named pipes are not added in the first version.
- Peer credential checks differ for each platform and are documented for each row.
- No static libc on macOS. Static on Linux. The MSRV job applies to the portable target too.
- The release notes record the binary size difference between the default build and the portable build.
- `podnet components --json` reports the excluded features and the reason for each.
- GitHub #89 (T-001) is the test guide for hosts with limited networking. A row that cannot run on a platform is named there, not omitted.

## Premise

- The workspace has no `[features]` table and no `cfg(feature)` gate. No `portable` feature exists.
- A conflict with a ruling of the owner: `CLAUDE.md`, Decisions, says "Platforms: Linux only. No Windows or macOS builds." The ruling covers Windows and macOS. FreeBSD is not named.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. The owner ruled Linux only (2026-11-12, `docs/decisions.md`, Platforms and the link crate); nothing is built for another platform.
2. If the owner allows it: add the feature `portable` (T-311) and the inventory (T-310). Gate the excluded modules with `cfg`. Add the exit-69 message. Add the CI job with a runner matrix.
3. If the owner refuses: the owner removes this entry (`TODO/RULES.md`, Nothing closes as out of scope).

## Prove

```sh
cargo test --test cli components
sh scripts/agent/check.sh
```

## Blocker

The owner, 2026-11-12 (`docs/decisions.md`, Platforms and the link crate): the ruling "Platforms: Linux only" stands. The entry unblocks only when the owner rules for another platform.

## Start condition

T-311, T-310, T-262 and T-275 are done, and the owner rules for another platform.

# T-306: link: the query keys of a carrier URL (GitHub #476)

**Source:** GitHub #476 (https://github.com/PodUtils/podnet/issues/476), row E10-1.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E10-1 of GitHub #476 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: URL query keys. The status in the issue is "planned". The row is done when: tests/cli/query.rs.

The rules of the issue that apply to this row:

- The allowed keys are `carrier`, `tls`, `retry`, `lb`, `compress`, `pad`, `mtu`, `maxhops`, `poll`, `window` and `name`.
- Each key is parsed once. A repeated key exits 64. An unknown key exits 64 and lists the allowed keys.
- `password`, `key`, `token` and `identity` are refused even with a value. The value is never echoed.
- Values are validated before any socket opens.
- 16 keys at most, a URL of 1 KiB at most, ASCII values, no nested encoding.

## Premise

- No query parser exists. A carrier URL is an address only. The spec parser of the registry is `src/networks.rs` (`parse`).
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Write the parser in the link crate with the checks above. Test each refusal and that the value of a refused key never appears in the message.
2. The carrier trait (T-264) validates the values of its own keys.

## Prove

```sh
cargo test --test cli query
sh scripts/agent/check.sh
```

## Start condition

T-264 is done.

## Correction

- The ruling of 2026-11-12 (`docs/decisions.md`, "The link crate: pod-link of podcrates") puts the multiplexer and the carriers in `pod-link`. podnet has no link crate. The parser is the CLI surface of podnet, so it is `src/link_query.rs` in podnet, not a new crate.
- `name` is parsed and checked (1 to 32 characters of `a-z 0-9 -`). T-308 binds a name before the command starts, so the query key has no effect: a command that gets it exits 64 and names `--name`.
- The key `carrier` has an effect: it takes the place of `--carrier`. Each other key is parsed and checked, and a command that gets one exits 64 and names the entry that adds it (`podnet::link_query::KEYS`). `retry` names T-263 and `window` names T-262: no entry of their own exists, so the entry named is the nearest one. The owner can add the entries.
- `mtu` is checked against the carrier (`carriers::check_query`): a stream carrier has no unit, and a value over the unit exits 64. `window` is checked against the range of `pod_link::Settings`, in the parser.

## Done

2026-11-12, commit 6524965. `src/link_query.rs` (`podnet::link_query`) splits `ADDRESS?key=value&...` at the first `?` and checks the query: the 11 allowed keys, each key once, an unknown key lists the allowed keys (and does not show the unknown name), `password`, `key`, `token` and `identity` refused with or without a value, 16 keys and 1024 bytes at most, printable ASCII values with one level of percent-encoding, and a range or a word list for each key. No message shows a value. `carriers::check_query` checks `mtu` against the facts of the carrier. `link listen`, `link dial` and `link connect` call `apply_query` first, before any socket opens. `docs/link.md` has the table of keys, the help text of the three commands has the form, and `llms-full.txt` is regenerated.
Tests: `cargo test --test cli query`: 9 passed (`a_repeated_key_exits_64`, `an_unknown_key_exits_64_and_lists_the_keys`, `a_secret_key_is_refused_with_or_without_a_value`, `a_bad_value_exits_64_and_is_not_echoed`, `the_limits_exit_64`, `a_key_with_no_effect_names_its_entry`, `mtu_of_a_stream_carrier_exits_64`, `the_carrier_key_takes_the_place_of_the_flag`, `a_valid_carrier_key_reaches_the_carrier`), and 10 unit tests in `link_query` and 2 in `carriers`. With the three commands not calling `apply_query`, 8 of the 9 tests failed. Each refusal test runs the three commands, checks exit 64, that the secret value `zzsecretzz` is not in stdout or stderr, and that the path was not tried.
Not proven or not done: `tls`, `retry`, `lb`, `compress`, `pad`, `maxhops`, `poll`, `window` and `name` have no effect (see Correction). A `window` check by a carrier is not done: the range is the one of the link settings. A unix path that holds `?` cannot be used.


# T-307: link: the write verbs of the control plane (GitHub #476)

**Source:** GitHub #476 (https://github.com/PodUtils/podnet/issues/476), row E10-2.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E10-2 of GitHub #476 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Link control plane. The status in the issue is "partial". The row is done when: link_ctl_plane (tests/link_ctl_plane.rs).

The rules of the issue that apply to this row:

- The commands are `status`, `dump`, `rotate`, `close-stream`, `close-link`, `select` and `reconnect`.
- The socket is 0600 with a random 32-byte token for each run and a `SO_PEERCRED` check. There is no TCP listener and no HTTP endpoint by default.
- A request is 4 KiB, a response is 64 KiB, 16 clients, one command at a time for each object.
- `rotate` is make-before-break. `select` moves new streams only.

## Premise

- The router provides `--control`, `--select`, `--rotate` and `--reconnect` (`docs/router.md`, T-180). Its API is HTTP/1.0 with JSON bodies (`src/router/control.rs`). The link socket is T-267: one JSON object for each line.
- T-180 (router write-side control) stays a separate entry.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add the five write verbs to the socket of T-267. `rotate` opens the new carrier before it closes the old one.
2. Test `select` (new streams only) and `rotate` (no stream fails).

## Prove

```sh
cargo test --test link_ctl_plane
sh scripts/agent/check.sh
```

## Start condition

T-267 is done.

## Correction

2026-11-12: the entry is `partial`. Done: the three verbs that the control socket can carry out with what an instance holds: `close-link`, `close-stream` and `select`. Left: `rotate` and `reconnect`. An instance holds links and has no way to make a link again (the carrier, the address and the identity are in the command that dialed), so both verbs answer 69 with `not built`. They land with the entry that gives `link connect` a loop that dials again (a start condition for a new entry); `rotate` is make-before-break and `reconnect` makes a new link, so each opens the new carrier first. The rule "one command at a time for each object" is the rule of a client (one command at a time on a connection); a second client can call a write verb on the same link at the same time, and `close-link` of a link that has ended is not an error, so the case is safe but a lock for each link is not built.

## Done

2026-11-12, commits 302cad5 and 55d5c14. The socket (`src/link_ctl.rs`, `src/link_ctl/verbs.rs`) has the verbs `close-link` (`"link":ID`: a GOAWAY after the frames that wait; a link that has ended is not an error), `close-stream` (`"link":ID,"stream":ID`: a reset of one stream; the other streams and the link stay) and `select` (`"link":ID` or `null`: the link that serves the streams opened from now on, so `peer:FINGERPRINT/SERVICE`; the streams that run stay; a link that has ended exits 69; the pin ends with its link). Each answers the document of `status` after the change, with `verb` set to its name. The rows of `status` have `stream_ids` (64 at most) and the top has `selected`. A stream is in the table when a task serves it and waits on `Tracked::cancelled` (the verb `open` and the bridge of `link listen`). A missing or bad id, or an id that names no link or stream, exits 64; `rotate` and `reconnect` exit 69; an unknown verb exits 64 and lists the verbs. The verbs run after the token and the uid check of T-267: a bad token exits 77 and a refused user exits 77, and the link is not changed. `podnet link close NAME` calls `close-link` for each link that is up (one line `closed link ID  PEER`) before the stop by signal; a socket that does not answer is one line on stderr. `docs/link.md` has the verbs.
Fail first: before the change 8 of the 11 tests of `tests/link_ctl_plane.rs` failed (the verbs were unknown: exit 64, no `stream_ids`, no `selected`; the 3 that passed were the refusals, which hold for an unknown verb too). `cargo test --test link_ctl_plane`: 11 passed, 3 runs: `close_link_ends_the_link_and_answers_the_status_after_the_change`, `close_link_leaves_the_other_links_up`, `a_write_verb_with_a_bad_token_exits_77_and_changes_nothing`, `a_write_verb_from_a_user_that_the_gate_refuses_exits_77`, `a_bad_link_or_stream_id_exits_64`, `rotate_and_reconnect_exit_69_and_an_unknown_verb_exits_64`, `close_stream_resets_one_stream_and_leaves_the_others`, `select_moves_new_streams_only`, `select_refuses_a_link_that_has_ended_and_a_pin_falls_back_when_it_ends`, `close_stream_ends_the_stream_of_a_peer_of_a_listener_process` (a listener process, a dialer in the test: the stream gets a reset and the link stays up), `link_close_uses_the_close_link_verb_when_the_socket_answers`. `cargo test --test link_ctl`: 11 passed as before. Suite: default build: 1650 passed, 2 failed (the llms files that I had not regenerated after a doc edit, and quick_tunnel_standin_falls_back_to_http2_when_quic_has_no_listener under load), 2 ignored, 69 test binaries; both pass after the regeneration and alone (3 runs); --features full: 1652 passed, 0 failed, 2 ignored, 69 binaries; `sh scripts/agent/check.sh` at the end: fmt, clippy, record, size, doc, layers and test all 0 (its default `cargo test --workspace` passed whole).
Not proven: `rotate` and `reconnect`; a client of another uid (the gate is a closure, as in T-267); two clients that call a write verb on one link at the same time; the pin over two links of two different peers (the tests use one peer).

2026-11-13, commit 1aeacc3. `reconnect` and `rotate` are built (`src/link_ctl/redial.rs`). `link connect` keeps a `Dialer` (carrier, address, key, allow list, services, egress chain) and attaches it to the hub as a `Redial`. `reconnect` (`"link":ID`) resets the streams, ends the link, dials again, and answers the status document with `link` (new id), `replaced` and `reset_streams`. `rotate` dials a new link first, pins it for new streams (a pin on another link stays), and ends the old link when its streams end. The new link must prove the key of the old link (else 77), and the name of T-308 is checked again. A listener answers both verbs 69 (`a listener does not dial`). `podnet link reconnect NAME [--link ID]` and `podnet link rotate NAME [--link ID]` call the verbs. The instance of `link connect` now ends (exit 69) when no link is left, not when the first link ends. `docs/link.md`, the help and the llms files are updated.
Fail first: with the two verbs set back to the answer `not built`, 4 tests failed (`a_listener_refuses_rotate_and_reconnect`, `reconnect_gives_a_new_link_id_and_a_new_stream_works`, `rotate_keeps_an_open_stream_running_until_it_ends_and_puts_a_new_stream_on_the_new_link`, `reconnect_and_rotate_of_a_connect_process_keep_the_name_and_a_peer_that_changed_exits_77`). With the change: `cargo test --test link_ctl_plane` 13 passed, `cargo test --test link_ctl_redial` 2 passed; `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc, layers, test all 0.
Not proven or not built: the resume of a session (T-263): `link connect` does not run over `session`, so each reconnect is a new session; a failed dial in `reconnect` leaves no link and the instance exits 69; the name check on a live carrier of tcp, ws or h2 (the tests use unix); a lock for each link against two clients at the same time.


# T-308: link: names for live links (GitHub #476)

**Source:** GitHub #476 (https://github.com/PodUtils/podnet/issues/476), row E10-3.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

Row E10-3 of GitHub #476 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Alias. The status in the issue is "partial". The row is done when: link_alias (tests/link_alias.rs).

The rules of the issue that apply to this row:

- A name is unique among live links. It has 1 to 32 characters of `[a-z0-9-]`.
- The name is stored with the background record, so `ps` shows it. A duplicate exits 64.
- A name never replaces the fingerprint. The first dial still needs `--peer`. A stored name maps to a stored fingerprint that is checked again on reconnect.
- 64 live names. No reuse while a record exists.

## Premise

- `--name` exists for pods and detached commands (`crates/podnet-core/src/detach.rs:224`, `register`). No link alias exists.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Store the name in the background record. Check the character set, the length, the uniqueness and the limit of 64. Test the check of the fingerprint on reconnect.

## Prove

```sh
cargo test --test link_alias
sh scripts/agent/check.sh
```

## Start condition

T-267 and T-265 are done.

## Done

2026-11-12, commit 24d3af3. `src/link_alias.rs`: `check` (1 to 32 characters of `a-z 0-9 -`, else 64), `precheck` (a live instance that holds the name exits 64 unless `--replace`; the 65th live link instance exits 64), `bind` (a name maps to the fingerprint that the dialer proved first). `src/main.rs` runs `precheck` for `link listen` and `link connect` with `--name`, before the start. `link connect` binds the name after the hello; the file `NAME.peer` in the run directory holds the fingerprint and lives as long as the record (`instances::remove`). A dial under the same name to another fingerprint exits 77 and keeps the file (`instances::KEEP_PEER`). A name does not replace `--peer`: the first dial still needs `--peer` or `--allow` (64). `podnet ps` and `link status NAME` show the name. `docs/link.md` has the section "Names for live links"; the help of `link connect` and the llms files are current.
`cargo test --test link_alias`: 4 passed (`a_bad_name_exits_64`, `a_duplicate_name_exits_64_and_ps_shows_the_name`, `a_name_does_not_replace_the_peer_key`, `a_name_keeps_its_fingerprint_and_a_changed_key_exits_77`). `cargo test --lib link_alias`: 3 passed (`a_name_has_1_to_32_characters_of_a_z_0_9_dash`, `the_65th_live_link_exits_64`, `a_binding_keeps_its_fingerprint`). Fail first: with `src/main.rs` and `connect.rs` as before, `a_duplicate_name_exits_64_and_ps_shows_the_name` and `a_name_keeps_its_fingerprint_and_a_changed_key_exits_77` failed, and `a_bad_name_exits_64` did not end (the listener ran with the name `Web`).
Not proven: a name over a carrier that crosses a network (T-268); a supervisor restart that meets a changed key (the test uses a process that ended with no clean stop, which leaves the same record and binding); the stored name as a way to a peer (a name never replaces the fingerprint, so no such way exists); a listener has no binding (it takes many peers).

# T-309: link: the redacted state dump and bug-report --include-link (GitHub #476)

**Source:** GitHub #476 (https://github.com/PodUtils/podnet/issues/476), row E10-4.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E10-4 of GitHub #476 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Redacted state dump. The status in the issue is "partial". The row is done when: link_dump (tests/link_dump.rs).

The rules of the issue that apply to this row:

- The dump holds the carrier, the peer fingerprint, the session counters, the stream table, the route table, the last 20 errors and the build features.
- It excludes seeds, passwords, tokens and record contents.
- Every string passes the redaction of `src/report.rs` and the strip of control characters. The cap is 256 KiB. A larger table is cut with a count.

## Premise

- `bug-report` and `report::mask` exist: `crates/podnet-core/src/report.rs:129` (`mask`) and `:81` (`clean`), `src/bug_report.rs`. The flag `--include-link` does not exist.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add the control verb `dump` (T-267) and the flag `bug-report --include-link`.
2. Pass every string through `mask` and `clean`. Test that no seed, token or record content is in the dump.

## Prove

```sh
cargo test --test link_dump
sh scripts/agent/check.sh
```

## Start condition

T-267 is done. The route table part waits for T-286.

## Correction

2026-11-12: the entry is `partial`. Done: the verb `dump` and the flag `bug-report --include-link`. Left: the route table (it waits for T-286; the dump has `routes.built: false` with the reason). The cap of the dump is 256 KiB (`DUMP_LIMIT`), as this entry says, and not the 64 KiB of the other answers of T-267: only the answer to `dump` may be longer, and the client of `dump` reads up to 256 KiB. The dump has no "record contents": the hub holds fingerprints, addresses and counters only, and no record of the background record.

2026-11-13: T-286 is done (`podnet::mesh_table`, the route table). The condition "the route table part waits for T-286" holds. The dump still has `routes.built` false (`src/link_ctl/dump.rs:169`). What is left: fill `routes` of the dump from the table of a listener that runs `--announce` (the verb `routes` has the data), and test it. No other wait.

## Done

2026-11-12, commits 9a9222c and 55d5c14. The verb `dump` (`src/link_ctl/dump.rs`) answers `build` (the version, the optional `features` of the binary, the `carriers` with `feature` and `built`), `fingerprint`, `pid`, `settings` (`selected`, the `limits`), `totals`, `links` (the row of `status`, `stream_table` with `id` and `owner`, and `streams_cut`), the last 20 `errors` (`at`, `text`; 512 characters at most; the listener notes a refused peer and each stream that ends in an error), `routes` and `cut`. Every string passes `report::mask` and `report::clean`. The token of the run (`Control::start`) and the text of the key file (`identity::load`) go to `report::hide`, so no report holds them. A table that does not fit the cap is cut from its last rows (steps 1024, 256, 64, 16, 4, 1, 0 rows for each link), and `streams_cut` and `cut.stream_rows` count what was cut; the counters stay whole. `podnet bug-report --include-link` adds the dump of each link instance that runs (`link_ctl::dump_all`), one line with the error for an instance that does not answer; `docs/link.md` and `docs/doctor.md` have it, and the llms files are regenerated.
Fail first: before the change `tests/link_dump.rs` did not compile (`unresolved import podnet::link_ctl::DUMP_LIMIT`, no `Hub::note_error`); with the call of `redact` removed 2 tests failed (`the_dump_holds_no_seed_no_token_and_no_secret`, `every_string_loses_its_control_characters`). `cargo test --test link_dump`: 8 passed: `the_dump_holds_the_carrier_the_peer_the_counters_the_streams_and_the_build`, `the_dump_needs_the_token_and_the_user_check`, `the_dump_holds_no_seed_no_token_and_no_secret`, `every_string_loses_its_control_characters`, `the_dump_keeps_the_last_20_errors_in_order`, `a_table_over_the_cap_is_cut_with_a_count`, `an_answer_over_64_kib_and_under_the_cap_reaches_the_client` (4000 streams, about 105 KB through the socket), `bug_report_include_link_adds_the_dump_of_a_running_instance_and_no_token` (a detached listener; no token and no seed in the text). Suite: default build: 1650 passed, 2 failed (the llms files that I had not regenerated after a doc edit, and quick_tunnel_standin_falls_back_to_http2_when_quic_has_no_listener under load), 2 ignored, 69 test binaries; both pass after the regeneration and alone (3 runs); --features full: 1652 passed, 0 failed, 2 ignored, 69 binaries; `sh scripts/agent/check.sh` at the end: fmt, clippy, record, size, doc, layers and test all 0 (its default `cargo test --workspace` passed whole).
Not proven: the route table (T-286); a dump of 256 KiB or more through the socket (the cut is proven with `dump_within` and a small cap); a dump of an instance with many links (the dump lists 512 links at most and counts the rest in `cut.links`; no test builds that many).

2026-11-13, commit 6e6be92. The route table: `routes` of the dump is the mesh view of T-289 (`Mesh::view`, no second view) when the instance runs `--announce`: `built` true, `node`, `limits`, `counters`, `truncated`, `nodes` with the edges (fingerprints, hops, cost, state, services, local names). It is at most 96 KiB; the stream-table cut steps also set its node and edge limits. Without `--announce`: `{built: false, reason: "this instance runs without --announce"}`. Every string stays in the redaction of the dump.
Fail first: with `src/link_ctl/dump.rs` at the old state, 3 tests failed (`the_dump_of_an_instance_with_announce_holds_the_route_table`, `the_dump_without_announce_says_why_the_routes_are_not_built`, `the_route_table_of_the_dump_is_redacted`); with the change `cargo test --test link_dump`: 11 passed.
Not proven: a dump of an instance with the route table at its 96 KiB limit.

# T-310: link: the component inventory command (GitHub #476)

**Source:** GitHub #476 (https://github.com/PodUtils/podnet/issues/476), row E10-5.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E10-5 of GitHub #476 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Component inventory: `podnet components`. The status in the issue is "planned". The row is done when: tests/cli/components.rs, and the manual lint.

The rules of the issue that apply to this row:

- One row for each carrier and service: name, kind, framing, duplex, needs-privilege, available, reason, cost class, build feature.
- Stable field names, 64 rows, 4 KiB for each row in JSON.
- `tests/cli/manual.rs` already checks that every command and flag is documented. Extend it to require a section for every compiled row.

## Premise

- No `components` command exists. `tests/cli/manual.rs` and `src/manual/facts.rs` hold the manual checks.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add `podnet components` and `--json` over the registry (T-264).
2. Extend `tests/cli/manual.rs`: each compiled row has a section in the manual.
3. Report the excluded features and the reason for each, for the portable build (T-305).

## Prove

```sh
cargo test --test cli components
cargo test --test cli manual
sh scripts/agent/check.sh
```

## Start condition

T-264 is done.

## Done

2026-11-12, commit 6edc8b6. `podnet components [--json]` lists one row for each carrier of the registry (`src/carriers.rs`, read only) and each service (`ss`, `trojan`): `name`, `kind`, `framing`, `duplex`, `needs_privilege`, `available`, `reason`, `cost`, `feature`, `built`, `about`, `address`. The code is in `src/components.rs` (`podnet::components`, with `check_limits`: 64 rows, 4 KiB for each row) and `src/cli/cmd/components.rs`. A row of a feature that is off says `available: false`, `built: false`, the reason, and `null` for `framing` and `duplex`. The JSON has `features`, `excluded` (each feature that is off, with its reason, for the portable build) and `portable`. The manual has a section INVENTORY (`podnet man inventory`, `src/manual/inventory.rs`): text, man page and JSON. `docs/link.md`, `docs/cli.md`, the README map, `llms.txt` and `llms-full.txt` are updated.
Tests: `cargo test --test cli components`: 3 passed (`components_json_has_one_row_for_each_carrier_and_service`, `a_row_of_a_feature_that_is_off_says_why`, `the_text_lists_each_row`). `cargo test --test cli manual`: `the_manual_has_a_section_for_each_compiled_component` passed; with the entry of `memory` removed from the manual it failed with `no section in the manual for "memory"`. Unit tests in `src/components.rs`: `a_row_has_the_stable_fields_and_the_limits`, `a_feature_that_is_off_is_unavailable_with_its_reason`, `the_manual_has_an_entry_for_each_row`. `sh scripts/agent/check.sh`: fmt, clippy, record, size, doc and layers are 0. The test step has 4 failing targets (`cli` `a_dialer_of_another_uid_needs_allow_other_uid`, `link_unix`, two shim tests) only because the agent TMPDIR is under a directory of root with mode 0700, so uid 65534 cannot enter it; with `TMPDIR=/tmp/t310tmp` (mode 755) the four targets pass (89, 8, 1 and 11 tests).
Not proven: a build with `--features full` (the rows of the compiled features were not run; the test code reads `built` from the output, so it covers them); the row of a service whose code exists (T-294, T-295 add it); the `portable` subset (T-305, blocked).

# T-311: link: Cargo features for the link carriers and services (GitHub #476)

**Source:** GitHub #476 (https://github.com/PodUtils/podnet/issues/476), row E10-6.
**Category:** feature
**Milestone:** M9
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Row E10-6 of GitHub #476 is part of the `link` workstream (GitHub #466 to #476): a generic peer-to-peer link for podnet. The row reads: Build features. The status in the issue is "planned". The row is done when: `cargo test --features full` fails at the base commit and passes after this entry; the issue is not closed on the strength of a command that exists but is not gated.

The rules of the issue that apply to this row:

- The default set is: session, `tcp`, `ws`, `h2`, `poll`, `sse`, `unix`, `memory`, `socks5`, `http`, `raw`, `forward`, `ctl`.
- The optional features are `quic`, `s3`, `icmp`, `ss`, `trojan`, `c-abi`, `portable` and `full`.
- `full` is a build feature for CI. No runtime `--full` flag is added, because a binary cannot enable code that it was not built with.
- A feature-gated command in a default build exits 69 with the feature name.

## Premise

- The workspace has no `[features]` table, no `cfg(feature)` gate and no `--features` in CI (`Cargo.toml`, `.github/workflows/ci.yml`).
- Releases build static musl binaries for many targets with `cargo-zigbuild` (`CLAUDE.md`, Decisions). Each optional feature must build on all of them, or be off in the release set.
- Read on 2026-11-12 at 505dfa0. Nothing was built or measured for this entry.

## Approach

1. Add the `[features]` table to the link crate and to `podnet`. Gate each optional carrier and service.
2. Add a CI job `cargo test --features full`. Map a gated command in a default build to exit 69 with the feature name.

## Prove

```sh
cargo test --workspace --features full
cargo test --workspace
sh scripts/agent/check.sh
```

## Start condition

T-262 is done (the crate). Each optional entry lands after this one.

## Done

2026-11-12, commits d1b5587 and 55d5c14. `Cargo.toml` of `podnet` has a `[features]` table: `default = []`, the optional features `quic`, `s3`, `icmp`, `ss`, `trojan`, `c-abi` and `portable`, and `full` (each optional feature but `portable`, which takes code away). `src/build_features.rs` (`podnet::build_features`) has `OPTIONAL`, `FULL`, `built`, `enabled`, `reason` and `require` (a feature that is off exits 69 and names the feature; a name that is no feature exits 64). The registry `src/carriers.rs` has the rows `quic`, `s3` and `icmp` with the field `feature` filled (`Some("quic")` and so on; `unix` and `memory` keep `None`). A carrier whose feature is off is listed by `podnet link carriers` as `unavailable: not built: the feature NAME is off (build with --features NAME)`, the JSON has `built: false`, `feature` and `reason`, `podnet doctor` shows it as `unknown` (not `fail`), and `--carrier NAME` exits 69 with the same reason. With the feature on, the row says that the code of the carrier is not in this build and names its entry (T-273, T-279, T-282). The CI job `features` runs clippy and `cargo test --locked --workspace --features full`. `docs/build.md` has the section Features; `docs/link.md` has the three rows.
Fail first: at base `cargo test --workspace --features full` exits 101 with `none of the selected packages contains this feature: full`; before the change `tests/link_features.rs` did not compile (`unresolved import podnet::build_features`). `cargo test --test link_features`: 10 passed in the default build and 10 passed with `--features full`, among them `a_gated_command_in_a_default_build_exits_69_with_the_feature_name`, `a_carrier_whose_feature_is_off_is_listed_as_not_built_and_exits_69_when_named`, `link_carriers_lists_a_gated_carrier_with_not_built_and_the_json_has_the_feature`, `a_command_that_names_a_carrier_that_is_not_built_exits_69` (the binary: `link dial --carrier quic` exits 69), `full_holds_each_optional_feature_but_portable`. `cargo test --workspace` (default) and `cargo test --workspace --features full`: default build: 1650 passed, 2 failed (the llms files that I had not regenerated after a doc edit, and quick_tunnel_standin_falls_back_to_http2_when_quic_has_no_listener under load), 2 ignored, 69 test binaries; both pass after the regeneration and alone (3 runs); --features full: 1652 passed, 0 failed, 2 ignored, 69 binaries; `sh scripts/agent/check.sh` at the end: fmt, clippy, record, size, doc, layers and test all 0 (its default `cargo test --workspace` passed whole).
Not proven: the static musl build and the zig builds (not run; the default features add no code and no dependency, and `Cargo.lock` did not change); the default carriers `unix` and `memory` do not sit behind a feature (they stay in each build); no optional feature gates code yet, because no optional carrier or service exists (T-273, T-279, T-282, T-294, T-295 and T-304 land behind their features and replace the `pending` makers); `portable` gates nothing (T-305 is blocked).
