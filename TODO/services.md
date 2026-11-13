This file holds the entries of milestone M6: the services that podnet builds from its pieces. The
order is the owner's (2026-11-05): the shared parts first, then the file server, git hosting, the
router, and file transfer. `docs/research/podnet/capabilities.md` gives the design of each role.
`docs/research/podnet/podssh-copy.md` is a reading of the podssh code. podnet does not copy that code:
it comes through the shared crates of `PodUtils/podcrates` (`docs/decisions.md`, Shared crates).

# T-082: `connect --stdio` for an SSH `ProxyCommand` through `tunnel --forward`

**Source:** `docs/research/podnet/capabilities.md`, sections 1.1 and 6 (enabler 1).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`connect` has `-L` listeners only. An SSH `ProxyCommand` through `tunnel --forward` needs a
stream on stdin and stdout. Git over SSH through the tunnel needs it.

## Premise

- Read: `-L` is required in `ConnectArgs` (`src/main.rs`).
- `relay connect` has a mode on stdin and stdout already (`src/relay_reverse.rs`).
- `src/ws.rs` holds the WebSocket client of the forward.

## Approach

1. Add `connect URL --stdio`: one stream, stdin to the WebSocket and the WebSocket to stdout.
2. Exit at the end of either side. Take the Basic credentials as `-L` does.

## Prove

```sh
cargo test --test connect -- connect_stdio_carries_a_stream
```

The test runs a WebSocket forward on the loopback and pipes bytes through `connect --stdio`.

## Done

2026-11-09, commit f16a790. `podnet connect URL --stdio` carries one stream on stdin and stdout
(`src/cli/cmd/connect_stdio.rs`). The end of stdin closes the WebSocket; the close of the remote ends the
command. `cargo test --test connect -- connect_stdio_carries_a_stream`: 1 passed. Tests added:
`connect_stdio_carries_a_stream`, `connect_stdio_ends_when_the_remote_closes`,
`connect_stdio_exits_69_and_refuses_listeners`. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-083: A relay node serves a handler in the process, with no socket file

**Source:** `docs/research/podnet/capabilities.md`, section 6 (enabler 2).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`relay node` serves one target: a TCP address or a unix socket. A file server, a git SSH server or
a transfer in the same process must open a socket file to be reached.

## Premise

- Read: `Target` has the variants `Tcp` and `Unix` (`src/relay_reverse.rs`).
- `tests/relay_reverse.rs` runs a node against a fake relay.

## Approach

1. Add a target that calls a handler with the stream of each session.
2. Use it for the file server (T-091), git (T-094) and the transfer (T-104).

## Prove

```sh
cargo test --test relay_reverse -- a_node_serves_an_in_process_handler
```

## Correction

2026-11-12, the take of `pod-relay` v0.30.0 (83e0c93). The crate has the handler target: `pod_relay::reverse::local::channel(depth)` gives a `ChannelHandler` and a receiver (PodUtils/podcrates#89, ask 11 of GitHub #413, podcrates T-150). The node code is generic over `Handler`, so the work in podnet is glue: `relay_reverse::run_node_with` runs a node over any handler, and `run_node` builds the handler of a target. The users of the handler (T-091, T-094, T-104) still use the unix socket. They can move to the channel when they change.

## Done

2026-11-12, commit 7e462ec (the glue and the tests) over the take 83e0c93. `relay_reverse::run_node_with(follow, pair, handler, shown)` and the re-exports `channel`, `ChannelHandler`, `Handler` and `SessionId`. `cargo test --test relay_reverse -- a_node_serves_an_in_process_handler a_closed_channel_refuses_the_session`: 2 passed. `a_node_serves_an_in_process_handler`: two operator sessions get their own pipe and their own id from the service of the process, and no socket file is made. `a_closed_channel_refuses_the_session`: the operator learns the refusal ("the service of the process is gone"). Before the change the test target does not compile (E0432: `channel` and `run_node_with` do not exist). `cargo test --test relay_reverse`: 8 passed. `sh scripts/agent/check.sh`: exit 0. Not proven: a service of podnet (T-091, T-094, T-104) that uses the channel.

# T-084: One HTTP authentication layer for the local listener, the tunnel and the relay

**Source:** `docs/research/podnet/capabilities.md`, sections 3.2 and 6 (enabler 3).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

HTTP Basic authentication lives only in the dispatch of the tunnel, and `serve --local` refuses
it. The PIN gate works only for `share`. A service on a local listener or a relay node has no
authentication.

## Premise

- Read: the Basic check is in `src/proxy.rs`, and its credentials in `PodUtils/podnet:src/proxy_auth.rs`
  (`Credentials`).
- Read: `serve --local` refuses `--auth-user` (`src/main.rs`).
- The PIN gate is `crates/podnet-serve/src/gate.rs`.

## Approach

1. Write one axum layer over `Credentials` and the gate, with the roles read and write.
2. Use it in the tunnel, on the local listener and on the relay node.
3. Let `serve --local` take `--auth-user` and `--auth-password-file`.

## Prove

```sh
cargo test --workspace -- auth_layer_ serve_local_takes_basic_auth
```

With no credentials the answer is 401. A PUT of a read user is 403. The same layer works on a
unix-socket listener.

## Start condition

T-062 is done.

## Correction

2026-11-11: T-089 gave the file server the users, the roles and the PIN on each listener (with
`--local`), and `serve --local` takes `--auth-user` and `--auth-password-file`
(`serve_local_checks_users_and_the_pin`, `tests/cli/serve.rs`): Approach 3 is done there. The tunnel keeps
its own Basic check of `src/proxy.rs` (the Cloudflare adapter moves to `pod-cloudflare`, T-052); a road
gets the layer on its router (T-062), so the tunnel road, the relay road and the vnet road take it with no
change of the edge code. The Prove filter is `auth_layer_` and `serve_local_checks_users_and_the_pin`.

## Done

2026-11-11, commit 700e2b6. `podnet_serve::files::guard(router, access)`: the access layer of the file server
for any router (401 with no right credential, 403 for a change by a read user, the PIN gate and its unlock
path, the anonymous read). `files::router` uses it; `podnet::road` and `podnet::roads` export it with
`Access`, `Users` and `Role`. Tests: `cargo test -p podnet-serve --lib auth_layer_` (2: on the local road
over a unix socket, 401, 401 with a bad password, 200 for a read, 403 for a PUT of a read user, 200 for a
PUT of a write user; an open access adds no layer), `cargo test -p podnet --test cli -- serve` (8).


# T-085: A ustar writer and reader, and a directory as one tar download of the file server

**Source:** `docs/research/podnet/capabilities.md`, sections 3.1, 3.4 and 6 (enabler 6).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

podnet has no tar writer or reader. The file server cannot give a directory as one download.

## Premise

- No crate of the lock gives tar today, and the design needs none (about 250 lines).
- `crates/podnet-serve/src/service.rs` holds the static router and its guard.

## Approach

1. Write a ustar writer and reader in `podnet-serve`: regular files and directories, long names
   with the ustar prefix, no link out of the root.
2. With `--tar`, the file server gives a directory as one tar stream.

## Prove

```sh
cargo test -p podnet-serve -- ustar_round_trip_keeps_odd_names a_directory_downloads_as_a_tar
```

The second test compares `tar -tf` of the download with the tree.

## Done

2026-11-09, commit 6fafd3f. `crates/podnet-serve/src/tar.rs`: a ustar writer and reader (files and directories,
the ustar prefix, base-256 sizes; the reader and `unpack` refuse links, absolute names and `..`).
`crates/podnet-serve/src/tar_http.rs`: `serve --tar` gives `GET /PATH/?tar` as one stream.
`cargo test -p podnet-serve -- ustar_round_trip_keeps_odd_names a_directory_downloads_as_a_tar`: 2 passed (the
second compares `tar --quoting-style=literal -tf` with the tree). Tests added: `ustar_round_trip_keeps_odd_names`,
`ustar_refuses_names_that_do_not_fit_or_leave_the_root`, `ustar_big_sizes_use_base_256`,
`unpack_writes_below_the_root_only`, `a_directory_downloads_as_a_tar`. `sh scripts/agent/check.sh`: exit 0.
No new dependency. CI: at the merge.

# T-086: Take the resumable layer, the end-to-end channel, the key identity and UDP in a stream from the shared crates

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.3, 3 (C4, C6, C7) and 5 (G1);
`docs/research/podnet/capabilities.md`, section 6 (enabler 5).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** L
**Status:** done

## Problem

The relay closes a session at 64 MiB in each direction, after 12 h, or after 180 s with no
traffic. podnet has no resume, so a clone, a push or a transfer over the relay that is larger
fails. The relay sees the bytes.

## Premise

- podssh has a resumable layer (`Azathothas/podssh:crates/podssh-relay/src/session/`, 7,468
  lines, 6,820 with no handover), an end-to-end channel on Noise XX (its core, about 1,700
  lines), a key identity (694 lines) and UDP in a stream (352 lines).
- A trial built the layer alone with three shims. 135 tests of 18 test files passed.
- The only new crate of the layer is `x25519-dalek 3`. `snow 0.10` brings the older RustCrypto
  line, with about 10 warnings of duplicate versions.

## Approach

1. Take `pod-session` (the layer) of PodUtils/podcrates.
2. Add the rows of the variables to `src/manual/facts.rs`.
3. Take `pod-e2e` (the channel) and `pod-identity` (the key identity). The podnet helpers for
   private files stay in podnet. UDP in a stream comes with the crate that owns it: read `pod-session`
   and `pod-relay` first.
4. The tests stay in the shared crates (each crate keeps the tests of its source). Add one
   test of podnet that runs a session over a fake relay.
5. Run `cargo deny check`.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Code of podssh and Crypto crates): copy podssh code
and change it as necessary; no wire compatibility with podssh is promised. `snow` 0.10 is
accepted. `cargo deny` stays green with no ignored advisory.

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the shared crates of `PodUtils/podcrates` hold this code. podnet depends on the shared crate; it does not copy the code.

## Prove

```sh
cargo tree -p podnet --depth 1 | grep 'pod-session\|pod-e2e\|pod-identity'
cargo test --test relay_reverse
cargo deny check
gh workflow run release.yml --ref main   # the dry run builds the eight targets
```

## Start condition

T-049 is done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The work of the Approach is in PodUtils/podcrates. The title and the steps are changed. There is
no crate `podnet-relay`. The start condition names T-049 (the take of `pod-relay`).

2026-11-12, GitHub #466 (rows E0-1 and E0-2) and #472 (row E6-1), the `link` workstream. The link needs this take for the key identity (`pod-identity`), the resume (`pod-session`) and the end-to-end channel (`pod-e2e`). The link side of each take is its own entry, blocked on this one: T-261, T-263 and T-298. `pod-session` holds a resume layer for one byte stream. It holds no multiplexer of streams with credits: that is T-262. `pod-identity` shows the OpenSSH fingerprint, and the issue asks for base32 in five groups with a checksum. Asks for podcrates: A1 (the display form of the fingerprint), A2 (the resume window and the use of the token as settings; whether a shared multiplexer crate is wanted), A3 (the handshake hash and the transport keys of `pod-e2e` as an interface). `docs/decisions.md`, 2026-11-12, The link workstream.

2026-11-13: partial. Taken at v0.36.0: the key identity (`pod-identity`, T-261), the resumable layer (`pod-session`, T-263) and UDP in a stream (command 0x83 of `pod-proxy`, T-329). Left: the end-to-end channel `pod-e2e` (podcrates T-301, ask A3). T-298 waits for it.

2026-11-13: `pod-e2e` is a dependency of podnet since T-298 (ea9b6c0): the link record layer uses
`pod_e2e::record`. Next: check steps 2 and 4 of the Approach against the tree, then write the Done section.

## Done

2026-11-13, by the takes b33718d (`pod-identity`, T-261), 75b49ba (`pod-session`, T-263) and ea9b6c0 (`pod-e2e`, T-298).
`cargo tree -p podnet --depth 1` names `pod-identity`, `pod-session` and `pod-e2e` (tag v0.39.0). Step 2: the three
crates read no environment variable, so `src/manual/facts.rs` needs no row. Step 4: `tests/link_session.rs`,
`resume_inside_the_window_carries_one_mebibyte_with_an_equal_digest`, runs a session through a middle pipe that cuts
the carrier (the fake relay). Step 5: deny 43459879363 on bf01ac6 passed. ci 43459872995 on bf01ac6 passed. The release
dry run of the Prove comes with the next beta (T-331).

# T-087: `relay node` and `relay connect` resume a session past the limits of the relay

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.3, 3 (C5) and 5 (G2); PodUtils/podcrates#35 (cfb), ask A-N-12.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** partial

## Problem

A session through the relay ends at its byte or time limit, and the stream breaks. Git over the
relay and the transfer need more than 64 MiB.

## Premise

- Both ends of the reverse road are podnet, so the layer of T-086 can run for each session above
  the framing of `src/relay_reverse.rs`.
- The forward road (`warp nc --via-relay`, the leg http2+relay, MASQUE through the relay) ends at
  the relay Worker. It cannot resume, and it reconnects as today.

## Approach

1. The node runs the far end of the layer for each session.
2. `relay connect` runs the client of the layer, with a link maker that opens
   `/v1/connect/NAME` again.
3. Move to a second link before 48 MiB or 11 h. Print each move with `-v`. Warn before the pair
   expires.
4. Run the layer alone first. Add the end-to-end channel with keys (`--key`, `--allow`) as an
   option, and ask the owner whether it is the default.
5. PodUtils/podcrates#35 (cfb), ask A-N-12, routed by PodUtils/podcrates#40: the resume must hold
   past 64 MiB and 12 h on a node that runs as a system service (T-220). Test one session of 12 h
   with a fake clock.

## Prove

```sh
cargo test --test relay_reverse -- relay_session_moves_past_the_cap
gh workflow run live.yml
```

A fake relay with a cap of 1 MiB carries 8 MiB in each direction with equal SHA-256. The live
step carries 200 MiB in each direction through the real relay.

## Start condition

T-086 is done.

## Correction

2026-11-05, The layer is `pod-session` and the channel is `pod-e2e` (T-086). The framing of the
reverse road moves with `pod-relay` (T-049): the node and the client code of `src/relay_reverse.rs` is in that crate
after the take.

2026-11-12, GitHub #466 (row E0-2). The link session (T-262, T-263) uses the same `pod-session` layer for its resume. The relay road of this entry does not change.

2026-11-12, the take of `pod-relay` v0.30.0 (83e0c93). The crate holds the node end of the layer (`reverse::Layered`, over `pod-session`), and `src/relay_reverse.rs` is glue over `reverse::run` and `operator::run` in the plain mode. The work of this entry is glue over the crate: a node over `Layered::new(handler, settings, budget)` with `run_node_with`, and a link maker for `relay connect`. The start condition stays: T-086 makes `pod-session`, `pod-e2e` and `pod-identity` dependencies of podnet itself. Read, not run: `reverse/legs.rs` and `reverse/layered.rs`. Not measured: whether the relay serves a second leg of one session. The fake relay of `tests/relay_reverse/fake.rs` serves none.

2026-11-13, the work (calls of the session; the owner can rule otherwise). (1) The layer is opt-in: `relay node --resume`
and `relay connect --resume` (`docs/decisions.md`, 2026-11-13). A node of the layer greets first, so an operator with no
layer gets bytes of the layer. A podnet operator with no `--resume` reads the `GREETING` in the first frame of the node
(`src/relay_reverse/flush_each.rs`, `LayerGuard`) and exits 78. An operator with `--resume` and a plain node keeps the
plain mode, as `pod-session` finds no `GREETING` (a target that waits for the client gets its first bytes after 30 s).
(2) The first leg keeps the wait of the plain mode after the end of its input (`eof_idle`); each later leg is a leg of
the layer, and the node ends it (`node_ends`, 35 s, as podssh). (3) The client asks for the end of one direction
(`end.v1`): the answer of the target comes after the end of the input, as in the plain mode (issue 109). (4) Moves print
at `-v` (debug); a loss and a resume print at the default level. (5) The operator warns before the end of the pair only
when it reads the local pair file; with `--token`, the end of the pair is not known. (6) `serve --ingress relay`,
`git serve --ingress relay` and `expose --via relay` keep the plain mode. (7) The fake relay (`tests/relay_reverse/fake.rs`)
serves each leg as a new relay session, and closes a session at a cap with 1009, as the relay does at 64 MiB. Measured:
with a cap of 1 MiB and moves at 256 KiB, one run opened 39 legs, and 25 of them met the cap before the move: the window
of the pair (768 KiB each way) is near the cap of the test, so a loss and a resume carry the session at the cap too. At
the relay, the margin is 16 MiB. (8) The 12 h test runs on the paused clock of tokio, over the far end of `Layered` in
memory: the crate reads the clock of tokio (`tokio::time::Instant`) for `move_age`, so no other hook is necessary.

Left: the end-to-end channel (`--key`, `--allow`, approach item 4), for later; the live step of 200 MiB through the real
relay (`gh workflow run live.yml`), for later: `.github/workflows/` does not change in this work.

2026-11-13, the lead: `.github/workflows/live.yml` has the step "200 MiB each way through the reverse relay with
--resume" (a relay node with `--resume` in front of a TCP echo, and `relay connect --resume`; `cmp` of 200 MiB). It waits
for a run (GitHub #531).

2026-11-13, the lead: the live run 43471960138 on 6f8e69e passed the step of 200 MiB each way (equal bytes, 30 s), but it
counted 0 moves: the job's `RUST_LOG` (`podnet=info`) takes priority over `-v` and hid the debug line of each move. The
step now sets `RUST_LOG=podnet=debug` for `relay connect` and fails with no move.

## Done

2026-11-13, commit 030c7ae. `src/relay_reverse/resume.rs`: `run_node_resumable` (the node over
`Layered::new(handler, settings, NODE_BUDGET)` with `run_node_with`), `connect_resumable` (the client of the layer, a
leg maker that opens `/v1/connect/NAME` again for each new leg), `carry` and `Resume` (48 MiB, 11 h). The CLI flags
`relay node --resume` and `relay connect --resume` (also with `--listen`). Docs: `docs/relay.md` (Sessions past the
limits of the relay), `docs/serve.md`, the README map, the llms files.
`cargo test --test relay_reverse -- relay_session_moves_past_the_cap`: 1 passed (8 MiB each way through a fake relay
with a cap of 1 MiB, equal SHA-256). The plant `resumes: false` in `Resume::settings` (the layer carries the session on
its first leg alone): the test fails ("8 MiB back: early eof"). Tests added: `resume::relay_session_moves_past_the_cap`,
`resume::relay_session_lasts_12_h_on_a_fake_clock` (13 h on the paused clock, legs that end at 12 h: a move, no loss,
26 echoes), `resume::a_plain_operator_learns_that_the_node_resumes` (exit 78),
`resume::a_resume_operator_keeps_the_plain_mode_with_a_plain_node`; the unit tests
`the_guard_stops_only_a_greeting_of_the_layer` and `the_settings_move_under_the_limits_of_the_relay`: each passed.
`sh scripts/agent/check.sh`: each step 0 (`cargo test --workspace`: 1857 passed, 0 failed). CI: no run (a push of the
cloud session starts none).

CI: ci 43471936738, deny 43471944208, secrets 43471952954 and live 43471960138 on 6f8e69e passed (GitHub #531).

# T-088: File server: a directory listing in HTML and JSON

**Source:** `docs/research/podnet/capabilities.md`, sections 3.1 to 3.5.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`serve` gives 404 for a directory with no `index.html`. A user cannot see the files of a
directory.

## Premise

- Read: the static router and its guard are in `crates/podnet-serve/src/service.rs`. The guard refuses a dot segment
  and a path out of the root.
- `crates/podnet-serve/src/gate.rs` has `escape_html`.

## Approach

1. Move the static router and its guard to a files module of `podnet-serve`.
2. Add a listing in HTML (each name escaped) and in JSON (with `Accept: application/json`).
3. Hide the dotfiles and each link out of the root.

## Prove

```sh
cargo test -p podnet-serve -- a_listing_escapes_names_and_hides_dotfiles a_listing_in_json
```

A name with `<script>` is escaped in the HTML listing.

## Start condition

T-062 is done.

## Correction

2026-11-10, The listing does not use the traits of T-062. It is a part of the router of the files
(`crates/podnet-serve/src/files/mod.rs`). The entry is done before T-062. `serve` takes the file server
as the new service `Service::Files` (`crates/podnet-serve/src/ingress.rs`); `static:` and `spa:` keep
`Service::Static`.

## Done

2026-11-10, commit c1d4daf. The static router and its guard are in `crates/podnet-serve/src/files/mod.rs`.
`serve --list`: a directory with no `index.html` answers a listing (`crates/podnet-serve/src/files/listing.rs`).
HTML escapes each name and percent-encodes each link; `Accept: application/json` gives
`{"path", "entries": [{"name", "type", "size", "modified"}]}`. The listing hides the dotfiles, each link out
of the root and each special file. A directory path with no `/` at its end gives 308 to the path with the `/`.
`cargo test -p podnet-serve -- a_listing_escapes_names_and_hides_dotfiles a_listing_in_json`: 2 passed. Tests
added: `a_listing_escapes_names_and_hides_dotfiles`, `a_listing_in_json`. The three tests of the static
router moved with their names. `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-089: File server: users with read and write roles, and a PIN, on each listener

**Source:** `docs/research/podnet/capabilities.md`, sections 3.1 to 3.3.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`serve` has one Basic user, through the tunnel only. It has no roles and no PIN.

## Premise

- The layer of T-084 gives Basic users and the PIN gate on each listener.

## Approach

1. Add `--users FILE`: lines `USER:ROLE:HASH`, or `USER:ROLE` with a password file. The roles are
   read and write.
2. Add `--pin` (the gate of `share`) and `--anon-read`.

## Prove

```sh
cargo test -p podnet-serve -- serve_users_have_roles serve_pin_unlocks
```

## Start condition

T-084 is done.

## Correction

2026-11-10, T-084 is open: the shared layer does not exist. The access of the file server is a layer of its
own router (`crates/podnet-serve/src/files/access.rs`), so it works on each listener. T-084 can make it the
shared layer. A users line with a password file is `USER:ROLE:@PASSWORD-FILE`. `--auth-user` of `serve` is a
write user of the file server: the tunnel does not check it for `serve`.

## Done

2026-11-10, commit 73e250e. `crates/podnet-serve/src/files/access.rs`: Basic users with the roles read and
write (a PBKDF2-HMAC-SHA256 hash, or a password file of mode 0600; the users file has mode 0600), the PIN
gate of `share`, and an anonymous read. 401 with a Basic challenge for no user or a wrong password; 403 for
a change by a read user. CLI: `serve --users FILE`, `--pin` (the PIN is printed, and is in the `--json`
line), `--anon-read`, `--hash-password`; `--auth-user` works with `--local`. The command moved to
`src/cli/cmd/serve.rs`. `cargo test -p podnet-serve -- serve_users_have_roles serve_pin_unlocks`: 2 passed.
`cargo test --test cli -- serve_local_checks_users_and_the_pin`: 1 passed. Tests added:
`serve_users_have_roles`, `serve_pin_unlocks`, `serve_local_checks_users_and_the_pin`.
`sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-090: File server: uploads into the served tree with PUT and a multipart form

**Source:** `docs/research/podnet/capabilities.md`, sections 3.1 to 3.5.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

A user cannot upload into the served tree. `share --mode upload` writes to its own directory with
random names.

## Premise

- Read: `upload_receive` streams a multipart body with limits and files of mode 0600
  (`crates/podnet-serve/src/share/mod.rs`).
- The tunnel takes at most 100 MB in one request body.

## Approach

1. Accept PUT (`curl -T`) and a multipart form into the tree, for the write role.
2. Write to a temporary file, then rename it. Reuse the streaming core of `upload_receive`.
3. Answer 413 before the body when it is over `--max-upload`, and leave no partial file.
4. Refuse `..` and each dot segment.

## Prove

```sh
cargo test -p podnet-serve -- put_into_the_tree_is_atomic_and_bounded
```

A PUT gives 201. A PUT over the limit gives 413 and leaves no file. A PUT with `..` is refused.

## Start condition

T-089 is done.

## Done

2026-11-10, commit 03bae2f. `crates/podnet-serve/src/files/upload.rs`: `serve --upload` takes a PUT of one
file (201 new, 204 replaced, 409 with no directory) and a multipart form POST to a directory (a name in use
gets a number; no file is replaced), for the write role. Each file goes to a hidden part file of mode 0600,
then a rename gives it its name. `--max-upload BYTES` (default 100000000): a larger `Content-Length` gets
413 before the body; a larger body gets 413 at the limit; no file and no part file stay. A dot segment
(also `..` and `%2e%2e`), a link out of the root and a read user are refused. The listing shows the form to
a write user. The streaming core reuses `next_chunk` and `create_unique` of `share`.
`cargo test -p podnet-serve -- put_into_the_tree_is_atomic_and_bounded`: 1 passed. Tests added:
`put_into_the_tree_is_atomic_and_bounded`, `a_form_uploads_into_a_directory`. `sh scripts/agent/check.sh`:
exit 0. CI: at the merge.

# T-091: `podnet serve DIR --via`, a local `share`, a directory in `share`, and the live step of the file server

**Source:** `docs/research/podnet/capabilities.md`, sections 3.2, 3.3 and 3.5.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`serve` reaches users through a tunnel or a local listener only. `share` has no local listener.
`share --mode download` takes one regular file only.

## Premise

- The relay exposure works today only by a composition of three commands
  (`docs/research/podnet/capabilities.md`, section 3.2).
- T-083 gives a node a handler in the process. T-085 gives a tar stream.

## Approach

1. Give `serve` the form `podnet serve DIR [--local ADDR]... [--via tunnel, relay or none]`, with
   `--list`, `--upload`, `--tar` and the flags of authentication.
2. `--via relay` serves through a relay node with the handler of T-083.
3. Add `share --local ADDR`, and a directory in `share --mode download` as a tar stream.
4. Extend the live step "Quick tunnel with --auth-user": the listing, a range, a PUT of 1 MiB and
   a 401; then the same through the relay.

## Prove

```sh
cargo test --test cli -- serve_dir_with_list_upload_and_users share_local_serves_a_directory
gh workflow run live.yml
```

## Start condition

T-083, T-085, T-088, T-089 and T-090 are done.

## Correction

2026-11-10, T-083 is blocked (the code of the relay node is frozen). `--via relay` does not need it: the
file server listens on a private unix socket of the run directory, and `run_node` of the frozen relay code
connects each session to it, with no change of that code (`src/cli/cmd/serve_relay.rs`). When podnet takes
`pod-relay`, a handler target can replace the socket.

Done in commit 3d9e225 (steps 1 to 3, and the text of step 4):

- `podnet serve DIR` (or `--dir DIR`), `--local ADDR` more than one time, `--via tunnel|relay|none`
  (`src/cli/cmd/serve.rs`), `--pair NAME` for the relay, with `--list`, `--upload`, `--tar` and the flags
  of T-089. `--via tunnel` or `relay` with `--local` serves the local listeners too.
- `share --local ADDR` for upload and download (`src/cli/cmd/share.rs`); `share --file DIR` gives the
  directory as one tar stream (`crates/podnet-serve/src/share/download.rs`).
- `.github/workflows/live.yml`: the steps "File server through a quick tunnel" (the listing, a range, a PUT
  of 1 MiB, a 401) and "File server through the reverse relay".
- Tests: `cargo test --test cli -- serve_dir_with_list_upload_and_users share_local_serves_a_directory`:
  2 passed; `cargo test --test relay_reverse -- serve_via_relay_answers_through_the_relay` (a fake relay):
  1 passed; `cargo test -p podnet-serve -- a_directory_downloads_as_one_tar`: 1 passed.
  `sh scripts/agent/check.sh`, `unshare -n -- cargo test --offline --workspace --no-fail-fast` and
  `sh scripts/restricted-host.sh --no-fail-fast`: exit 0.

Left: the run of `gh workflow run live.yml` on a live host. Set the entry to done after a green run of
the two new steps.

2026-11-10, the live run 42843879079 at 31d040f: the step "File server through the reverse relay" failed at once, as `scripts/live-throughput.sh` holds `127.0.0.1:18091`. The step now uses 18191 and stops its `filesrelay` instance. The entry stays partial until a live run passes.

## Done

2026-11-10: the live steps "File server through a quick tunnel" and "File server through the reverse relay" passed in the live run 42851836974 at 8383095 (ci 42851837015).


# T-092: Git: a CGI runner for `git http-backend`, and the smart HTTP router

**Source:** `docs/research/podnet/capabilities.md`, sections 5.2, 5.3 (design A) and 5.8; the
owner's ruling of 2026-11-05 (`docs/decisions.md`, Git hosting).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** L
**Status:** done

## Problem

podnet has no git server. A host behind a proxy cannot serve a repository to others.

## Premise

- `git http-backend` serves the smart HTTP protocol as a CGI program.
- `tokio` has the feature `process` (`Cargo.toml`).
- `git http-backend` inflates a body with `Content-Encoding: gzip`. A chunked body works with git
  2.19 or later; a test must confirm the minimum version.
- `src/proxy.rs` streams the body of an axum router behind the tunnel.

## Approach

1. Write a generic CGI runner: the environment (`GIT_PROJECT_ROOT`, `PATH_INFO`,
   `REQUEST_METHOD`, `QUERY_STRING`, `CONTENT_TYPE`, `CONTENT_LENGTH` unset for a chunked body,
   `GIT_HTTP_EXPORT_ALL`, `HTTP_GIT_PROTOCOL`, `REMOTE_ADDR`, and `REMOTE_USER` only after a good
   authentication). Parse the CGI head (`Status:`). Stream the body in both directions.
2. Write an axum router: anonymous clone and fetch; 401 for `service=git-receive-pack` with no
   credentials, so that git asks for them; protocol v2.
3. Refuse `..`, each dot segment and `/.git/config`.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Git hosting): `podnet git serve` uses the installed
`git` now (`git http-backend`). A native server comes later (T-122).

## Prove

```sh
cargo test --workspace -- git_http_clone_fetch_and_push
```

The test is skipped, with one line, when `git` is not on `PATH`. It clones, fetches with protocol
v2, and pushes with credentials. An anonymous push and a wrong password give 401. `/../` is
refused. A push of 8 MiB with `http.postBuffer=1024` (a chunked body) and an upload-pack request
over 1 KiB (a gzip body) pass.

## Start condition

T-084 is done.

## Correction

2026-11-10, the lead's call: T-084 is open (it waits on M4), and T-092 does not wait for it. The git
router uses the access layer of the file server (`crates/podnet-serve/src/files/access.rs`, T-089: the
read and write roles, the users file, `--anon-read`). The use is one call (`Access::grant`), so T-084 can
replace it.

2026-11-10, Premise: the tests ran with git 2.43.0 only. The minimum version of git for a chunked body is
not measured here.

## Done

2026-11-10, commit 60a1d80. `crates/podnet-serve/src/git/cgi.rs` is a CGI/1.1 runner: a clean environment,
the meta-variables of the request (`CONTENT_LENGTH` only with a `Content-Length`; no `Authorization` and no
`Cookie`), `REMOTE_USER` only after a good authentication, the parse of the CGI head (`Status:`), and the
body in both directions as a stream. `crates/podnet-serve/src/git/http.rs` answers only the smart protocol
(`info/refs?service=`, `git-upload-pack`, `git-receive-pack`); each other path, `..`, a dot segment and
`/.git/config` get 404. A fetch needs the read role, a push the write role; a push with no credentials or a
wrong password gets 401 with a Basic challenge. `create_repo` (for `--create-on-push` of T-094) makes a bare
repository on a push of a write user, over HTTP and SSH.

- `cargo test --workspace -- git_http_clone_fetch_and_push`: 1 passed (the git client: a clone, a fetch with
  protocol v2 and a gzip body of 100 haves, an anonymous push and a wrong password refused, a push of
  8 MiB with `http.postBuffer=1024` as a chunked body).
- Tests added (in `crates/podnet-serve/src/git/http_tests.rs`): `git_http_clone_fetch_and_push`,
  `git_http_refuses_paths_and_bad_credentials`, `git_http_create_on_push`,
  `parse_takes_the_smart_protocol_only`, `cgi_head_and_variables`.
- `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-093: Git: an SSH server for upload-pack and receive-pack

**Source:** `docs/research/podnet/capabilities.md`, sections 1.5, 5.4 and 5.8;
`docs/research/podnet/podssh-copy.md`, sections 2.7 and 3 (C22); the owner's ruling of 2026-11-05
(`docs/decisions.md`, Git hosting).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

podnet has no SSH server. The server module of russh is compiled in, and nothing calls it.

## Premise

- `russh 0.64` is a dependency with the feature `ring` and no `rsa` (`Cargo.toml`).
- `ssh-key` parses an `authorized_keys` file (`AuthorizedKeys`).
- podssh has a russh 0.65 server for its tests
  (`Azathothas/podssh:crates/podssh-cli/tests/ssh_harness/`), and no server in its product.
- The `rsa` feature brings an advisory (RUSTSEC-2023-0071), and `deny.toml` ignores none.

## Approach

1. Make an ed25519 host key once. Keep it in the state directory with mode 0600.
2. Check `auth_publickey` against the `authorized_keys` file. With `--anon-read`, `auth_none`
   gives read access only.
3. Keep `GIT_PROTOCOL` from `env_request`. Accept in `exec_request` only `git-upload-pack 'PATH'`,
   `git-receive-pack 'PATH'` and `git-upload-archive 'PATH'`. Run git, pump stdin, stdout and
   stderr, and send the exit status.
4. Refuse a shell, a pty, `direct-tcpip` and `tcpip-forward`.
5. Take ed25519 and ECDSA keys only.
6. Use the SSH test server of `pod-testkit` (PodUtils/podcrates) for the tests. Do not copy the
   test server of podssh.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Git hosting and Crypto crates): upload-pack and
receive-pack under a russh SSH server; `cargo deny` stays green with no ignored advisory, so no
RSA keys.

## Prove

```sh
cargo test --workspace -- git_ssh_upload_pack_matches_git git_ssh_refuses_an_unknown_key
```

The russh client of the tree runs `exec git-upload-pack` over a unix socket. Its ref
advertisement equals the output of `git upload-pack --advertise-refs`. An unknown command is
refused.

## Correction

2026-11-05, step 6: no copy of podssh code. When `pod-testkit` has no SSH server, open an issue in
PodUtils/podcrates.

2026-11-10, read in the tree:

- `russh` is 0.65.1 (`Cargo.lock`), not 0.64. `crates/podnet-serve/Cargo.toml` has it with the
  feature `ring`.
- Step 6: `pod-testkit` v0.11.0 has no SSH server (its files are `lib.rs`, `host.rs`,
  `screen.rs`). The tests need none: the server of this entry is the server under test, and the
  russh client of the tree is the client. No issue to PodUtils/podcrates is necessary.
- Step 2: an OpenSSH client sends the method `none` first. When `none` gives access for each
  user, a client with a key gets read access and cannot push. With `anon_read`, `none` gives read
  access only to the user `anonymous`, and a key that is not in `authorized_keys` gets read
  access.
- Prove: `git upload-pack --advertise-refs` is the stateless form and adds the capability
  `no-done`. The test compares the bytes with `git upload-pack DIR` on a pipe, and the refs with
  `--advertise-refs`.

## Done

2026-11-09, commit 93751c9. `crates/podnet-serve/src/git/ssh.rs` is the russh server;
`crates/podnet-serve/src/git/mod.rs` reads the exec command and maps the path under the root. The
command `podnet git serve` stays for T-094.

- `cargo test --workspace -- git_ssh_upload_pack_matches_git git_ssh_refuses_an_unknown_key`: 2
  passed.
- New tests (7, in `crates/podnet-serve/src/git/tests.rs`): `git_ssh_upload_pack_matches_git`
  (the bytes, the refs, protocol v2, receive-pack with a key),
  `git_ssh_refuses_an_unknown_key` (also `anon_read` and the user `anonymous`),
  `git_ssh_refuses_a_shell_and_other_commands`, `parse_exec_takes_the_git_commands_only`,
  `repo_dir_stays_in_the_root`, `host_key_is_made_once_with_mode_0600`,
  `authorized_keys_takes_ed25519_and_ecdsa_only`. They use a unix socket: they pass under
  `unshare -n`.
- `sh scripts/agent/check.sh`: exit 0. CI: at the merge.

# T-094: `podnet git serve`: the repository root, `git init`, `git urls`, the tunnel, the relay and the live step

**Source:** `docs/research/podnet/capabilities.md`, sections 5.5 to 5.8.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The parts of T-092 and T-093 have no command.

## Premise

- The quick tunnel takes at most 100 MB in one request body, so a larger push over HTTP fails.
  A clone is not limited.
- Until T-087, a clone or a push over the relay ends at 64 MiB.

## Approach

1. Add `podnet git serve --root DIR` with `--local ADDR`, `--ssh LISTEN`, `--via`,
   `--anon-read`, `--push-user` with `--push-password-file` or `--users FILE`,
   `--authorized-keys FILE`, `--create-on-push` and `--detach`.
2. Add `podnet git init NAME` and `podnet git urls` (the clone URLs and the fingerprint of the
   host key).
3. Map each request to a repository under the root. Refuse `..`.
4. Make `doctor` report a missing `git`. Write the limits in the manual.
5. Add a live step: through a quick tunnel, a clone and a push over HTTPS with credentials, and
   an anonymous push refused; through a relay node and `ssh -o ProxyCommand` with
   `relay connect NAME`, a clone and a push.

## Prove

```sh
cargo test --test cli -- git_serve_init_and_urls
gh workflow run live.yml
```

## Start condition

T-082, T-092 and T-093 are done.

## Correction

2026-11-10, the lead's call: T-082, T-092 and T-093 are done, so the start condition holds. `--via tunnel`
and `--via relay` reuse the glue of `podnet serve DIR --via` (T-091): the ingress service `git:` behind the
quick tunnel (the router of T-092), and a private unix socket of the run directory that `run_node` of the
frozen relay code connects to, with no change of a frozen file. `--via relay` carries the SSH server (the
live step of the Approach uses ssh through the relay); smart HTTP stays on `--local` and the tunnel.

2026-11-10, read in the tree: `git urls` cannot ask a server for its URLs. `git serve` writes a record of its
URLs (mode 0600) to `git/serve-HASH.json` in the state directory, and removes it at its end. `git urls`
reads it, lists the repositories under the root, and prints the fingerprint of the host key. `--auth-user`
of the tunnel options is refused: `--push-user` gives the push user.

Done in commit 84b5c5e (steps 1 to 4, and the text of step 5):

- `podnet git serve --root DIR` with `--local ADDR` (more than one), `--ssh LISTEN`, `--via
  tunnel|relay|none`, `--pair`, `--anon-read`, `--push-user` with `--push-password-file`, `--users`,
  `--authorized-keys`, `--create-on-push`, `--host-key` and `--detach` (`src/cli/cmd/git_serve.rs`);
  `podnet git init` and `podnet git urls` (`src/cli/cmd/git.rs`).
- `podnet doctor` reports the version of git, or that git is missing (`src/doctor/git_check.rs`). The
  limits are in the help of `git serve` (the manual) and in `README.md`, Git hosting.
- `.github/workflows/live.yml`: the steps "Git over HTTPS through a quick tunnel" and "Git over SSH through
  the reverse relay".
- Tests: `cargo test --test cli -- git_serve_init_and_urls`: 1 passed; `cargo test --test relay_reverse --
  git_serve_via_relay_gives_the_ssh_server` (a fake relay): 1 passed; `cargo test --lib --
  doctor_reports_a_missing_git`: 1 passed. `sh scripts/agent/check.sh`,
  `unshare -n -- cargo test --offline --workspace --no-fail-fast` and
  `sh scripts/restricted-host.sh --no-fail-fast`: exit 0.

Left: the run of `gh workflow run live.yml` on a live host. Set the entry to done after a green run of the
two new steps.

2026-11-10, the live run 42849917945 at 9e8d177: the step "Git over HTTPS through a quick tunnel" pushed `main` with the password, but the clone had no commit: `git init` of the runner makes the HEAD of the bare repository `master`. The server is correct. Both git steps now clone with `-b main`. The entry stays partial until a live run passes.

## Done

2026-11-10: the live steps "Git over HTTPS through a quick tunnel" and "Git over SSH through the reverse relay" passed in the live run 42851836974 at 8383095 (ci 42851837015).


# T-095: Router: several WARP identities in one process, each with its own counters

**Source:** `docs/research/podnet/capabilities.md`, section 2;
`docs/research/podnet/mwan-field.md`, sections 1.6 (mode A) and 2.2 (R1, R6).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`warp proxy --listen-identity LISTEN=PATH` gives one listener for each device, but the counters
are global, and no other command can use a second device.

## Premise

- Read: `LINK_STATS` in `src/warp/ip.rs` and `STATS` in `src/warp/proxy.rs` are global.
- Each identity keeps its device in its own file, so a device is not registered again.

## Approach

1. Keep N identities in one process, each in its own directory of the state directory.
2. Make one WARP network for each identity, with its own counters.
3. Accept `--net NAME=warp:PATH` through the registry of T-059.

## Prove

```sh
cargo test --workspace -- two_warp_identities_keep_their_counters
```

The edge stand-in of T-012 serves two identities.

## Start condition

T-057 is done.

## Correction

2026-11-13: the take of `pod-warp` moved the session and its counters into the crate, and the router meters each member
(`Balance`, `crates/podnet-core/src/network/meter.rs`, T-097). The spec of item 3 is `warp:identity=FILE` (3855573), not
`warp:PATH`. A network `warp` in a router with `state_dir` keeps its device in `state_dir/NETWORK/warp.json`.

## Done

2026-11-13, commit a10d70c. Items 1 to 3 are in 3855573 (the network `warp` and its device for each network); this
commit adds the proof. `src/router/control/tests.rs`, `two_warp_identities_keep_their_counters`: a router with two
members `warp` in one group, on the edge stand-in of `tests/warp_edge`: two devices, two identity files that differ, and
the bytes of each member count only its own flows (an echo of 1000 bytes through `a`, of 5000 bytes through `b`).
`cargo test -p podnet --lib two_warp_identities_keep_their_counters`: 1 passed.

CI: ci 43471936738, deny 43471944208, secrets 43471952954 and live 43471960138 on 6f8e69e passed (GitHub #531).

# T-096: Router: groups of networks with weights and a kill switch

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 and 2.5;
`docs/research/podnet/mwan-field.md`, sections 1.4 (D1) and 1.6; the owner's ruling of 2026-11-05
(`docs/decisions.md`, Router); GitHub #373 and #376.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

podnet has no groups, no weights and no kill switch. The kill switch of the mwan experiment leaks
to the host route.

## Premise

- `docs/research/podnet/mwan-field.md`, section 1.6: a userspace router starts each flow again on
  one uplink, so it needs no kernel route, no masquerade and no `rp_filter`.
- Balancing is per flow. One flow uses one uplink.

## Approach

1. Add a `Balance` combinator: a weighted pick for each flow; the same member for each pair of
   client and destination of UDP; a member that is down leaves the pick.
2. When no member is healthy, refuse the flow. Never fall back to direct.
3. Accept `--group NAME=a*2,b*1`.
3. A member in `Cooldown` or `Probation` (T-097) still counts as healthy. The pick refuses when no
   member is `Up` or timed (GitHub #373, #376).
4. The pick returns `Selection::{One, Replicate, None}`. `None` refuses and counts the refusal, and never
   dials direct. `Replicate` (a backup, the first answer wins) is optional.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Router): the userspace router first, per flow, with no
`CAP_NET_ADMIN`: groups, weights, TCP health checks, failover and a kill switch.

## Prove

```sh
cargo test --workspace -- router_group_splits_by_weight_and_fails_closed
```

The unit tests pick with a seed. The loopback test runs two fake SOCKS5 upstreams.

## Start condition

T-058 is done.

## Correction

2026-11-11: `Selection` has `One` and `None`; `Replicate` (optional in the Approach) waits for a measured
need. The flag `--group NAME=a*2,b*1` belongs to `podnet route` (T-099, T-100): `parse_group` reads it now,
and the command takes it with its doors. The same member for each pair of a UDP client and a destination
is the affinity of the UDP door (T-099); `Balance::datagrams` keeps one member for each socket.

## Done

2026-11-11, commit 07c21b4. `crates/podnet-core/src/network/balance.rs`: `Balance` (a `Dial`: a weighted pick
of splitmix64, seeded from `getrandom` or by `with_seed`; `set_health`; `refused`), `Member`, `Health`
(`Up`, `Cooldown`, `Probation` take flows; `Down` does not), `Selection` and `parse_group`. With no healthy
member a flow and a datagram socket get one line with the class `policy`, and no way is dialed. Tests:
`cargo test -p podnet-core --lib router_group` (3: the split 2:1 of 3000 picks within 5 %, the down member,
the kill switch with no dial; the parse; the refusals of a peer way and a weight 0) and
`cargo test -p podnet-vnet --lib router_group` (two fake SOCKS5 upstreams on unix sockets: 20 flows on both,
and no flow to either after the kill switch).


# T-097: Router: a TCP health check for each member, and `podnet route status`

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 and 2.5;
`docs/research/podnet/mwan-field.md`, sections 1.4 (D5, D6, D10), 1.6 and 2.2 (R2, R5); GitHub #373 and #374.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

A group needs the state of each member. An ICMP check lies for a WARP session that carries no
TCP (T-068).

## Premise

- The probe of T-068 renews a WARP session that carries no TCP.
- `crates/podnet-core/src/metrics.rs` and `crates/podnet-core/src/decisions.rs` give counters and decision lines.
- The mwan experiment leaks a socket for each probe, keeps no round-trip time, and puts a link
  back after one good probe.

## Approach

1. Probe each member: a TCP connect to `HOST:PORT` through it, each N s. Close the socket. Keep
   the round-trip time.
2. Mark a member down after K misses, and up after R successes.
3. A failed probe of a WARP member calls the renew of T-068.
4. Add `podnet route status [--json]`: the state, the round-trip time, the flows and the bytes of
   each member. Add rows to the metrics.
5. Set K, R and the thresholds so that K > R > 1 and R > K/2 (GitHub #373). A probe that flaps must not
   re-admit a member at once.
6. Give a member the states `Up`, `Probation`, `Cooldown` and `Down`. Cooldown and probation give a
   reduced weight (`degraded_weight`). `Down` follows only a clean failure (refused, unreachable, reset)
   that a control corroborates. The renewal of a WARP member fires on `Down`.
7. Add a control probe: one extra probe for each interval, through a second member or to a second target.
   A correlated failure of all members stops the down step and the renewals. The control gates both sites
   of `tcp_dead` (src/warp/live.rs:279 at `9c1fc4b` and :387 at `9c1fc4b`).
8. The probe is a method of the member (`Uplink::probe`). It never re-enters the group pick (GitHub #374).
   The probe goes through its own member, and the resolver of that member resolves its name.
9. `ProbeResult` is `Ok`, `ConnectFailed`, `WrongEgress` or `AuthExpired`. The egress check is on for WARP
   members (the trace request of `src/warp/exit.rs`) and off by default for other members. An echo
   endpoint of a member that is not WARP is operator input. Add no hardcoded host.
10. `AuthExpired` renews the member, make-before-break, with the stored registration (`register_or_load`).
    It makes no new device for each failure.
11. The thresholds, the cooldown and the probation go in the config file of T-100. Tests inject the time.

## Prove

```sh
cargo test --workspace -- router_health_moves_flows_off_a_dead_member router_status_json_keys
cargo test --workspace -- router_health_member_with_flapping_probe_stays_in_cooldown router_probe_dials_through_its_own_member
```

## Start condition

T-096 and T-068 are done.

## Correction

2026-11-11, the call of 2026-11-10 (`docs/decisions.md`, Start conditions): the generic part needs only
`podnet-core`, so it starts before the rest of T-068 waits for `pod-warp`. Done so far (4669f9c):
`crates/podnet-core/src/network/health.rs` has the probe through the way of its own member (item 8; it
never enters the pick), `Thresholds` with the check K > R > 1 and R > K/2 (item 5), `ProbeResult` (item
9, the types), and `Tracker` with `Up`, `Cooldown`, `Probation` and `Down` (item 6: a flapping probe stays
in `Cooldown`, `Down` follows K clean misses with a passed control, item 7 for the group). `Balance`
takes each probe (`observe`), counts the flows of each member and gives `status()` (the document of item
4). The tests give the time (item 11). Tests: `cargo test -p podnet-core --lib router_` (12).

Left, each with the crate or entry that it waits for:

- Items 3, 7 (the gate of `tcp_dead` in `src/warp/live.rs`), 9 (the egress check of a WARP member) and 10
  (the renew, make-before-break): the WARP adapter, frozen for `pod-warp` (T-050).
- Item 4: the command `podnet route status` and the metrics rows need the router process (T-099, T-100);
  the bytes of each member need the meter of the doors.
- Item 11: the thresholds in the config file (T-100).

2026-11-11, the comment on GitHub #375 from the donor review (airhand.md block 4; MIT;
https://github.com/PodUtils/podnet/issues/375#issuecomment-6789133721): `Tracker` keeps a short window of passing probe times (for example the last 8 to
40, a setting) and gives `rtt_floor()`, their least, beside `rtt()`. `observe` keeps the pass or fail result
and does not read the window, so the window changes no health transition. `podnet route status` prints the
floor and the last sample. The input is for `minrtt` (T-178). Test: `observe` gives equal results with the
window on and off for one sequence of passes and fails.

2026-11-11, 10831a3: item 11 and most of item 4 are done. The key `health` of the router file (T-100) takes
`down_after` (K), `up_after` (R), `cooldown` and `timeout` beside `interval`; `Health::thresholds` checks
K > R > 1 and R > K/2, and the probe of the router uses them. The key `control: unix:/PATH` makes the
router serve `GET /route/status` (the document of `Router::status`) on a socket of mode 0600. `podnet route
--status unix:/PATH [--json]` prints each door, each group and each member (health, round-trip time,
flows, weight); it exits 69 when no router answers. Tests: `route_config_thresholds_reach_the_router`,
`router_status_json_keys_over_the_control_socket`, and new cases of `route_config_errors_name_the_key`.
2026-11-11, 184fa71 (T-178): the window of passing probe times and `rtt_floor()` are done, as is the floor in the
status document. Test: `router_health_rtt_window_changes_no_step`.
Left: the bytes of each member (the meter of the doors), the rows of the metrics, and items 3, 7, 9 and 10 (the WARP adapter, frozen for
`pod-warp`, T-050).

2026-11-12, f4dd392 and 400cd64: the bytes and the rows are done, and item 9 is done for a member that is not
WARP. (1) The bytes of a member counted TCP flows only. `Balance::datagrams` now gives the socket in a
`MeteredDatagrams` (`crates/podnet-core/src/network/meter.rs`): both directions count for the member, and with a split
`up` counts what it sends and `down` what it receives. (2) The rows: `GET /route/metrics` on the control socket and
`podnet route --status unix:/PATH --metrics` give the Prometheus text, one row for each member and metric
(`src/router/metrics.rs`; `docs/router.md`, "State"). (3) Item 9: `health.egress` (`NETWORK: HOST:PORT=IP`) is the
operator input for a member that is not WARP. `probe_egress` connects through the member to the echo endpoint of the
operator and compares the address that it names; another address is `WrongEgress`. The router names no echo service.
`WrongEgress` takes the member to `Down` at once, whatever the control says, and no direct way replaces it (T-098
item 3). Left, each with what it waits for:

- Items 3 and 10 (the renew of a WARP member, make-before-break, with the stored registration): the WARP adapter,
  frozen for `pod-warp` (T-050). `Step::renew` already tells when to renew.
- Item 7, the gate of `tcp_dead` at the two sites of `src/warp/live.rs`: frozen for `pod-warp` (T-050).
- Item 9 for a WARP member (the trace request of `src/warp/exit.rs`): frozen for `pod-warp` (T-050). The generic
  check and the step to `Down` are in place; the WARP member only has to give `WrongEgress`.

## Done

2026-11-12, partial, commits f4dd392 (UDP bytes, `probe_egress`, `health.egress`, `WrongEgress`) and 400cd64 (the
metrics rows). Tests: `group_meter_counts_udp_bytes_in_both_directions`, `group_meter_counts_split_udp_bytes_for_each_member`,
`router_probe_egress_passes_when_the_echo_names_the_expected_address`, `router_probe_egress_fails_on_another_address_or_no_address`,
`router_health_wrong_egress_leaves_the_pick_at_once`, `route_config_health_egress_errors_name_the_key`,
`route_health_wrong_egress_member_leaves_the_pick`, `router_metrics_rows_follow_the_status`,
`router_metrics_rows_hold_the_round_trip_time`, `router_metrics_over_the_control_socket` and, in `tests/detach`,
`route_status_metrics_prints_the_rows`. The UDP byte tests failed first (0 bytes counted), the tracker test failed on a stub
(the member stayed `Cooldown`), and `route_status_metrics_prints_the_rows` failed first with "unexpected argument
'--metrics'". Not proven: the echo protocol against a real echo service; `WrongEgress` from a real WARP member;
the metrics in a scrape by a Prometheus server (the text is checked by string).

2026-11-13, done, the work of the handoff (the patch wip-warprouter), rebased on b1e2b80, commit 3855573. `warp` and `warp:identity=FILE` are networks of the registry
(`src/networks/warp.rs`, over `pod-warp` `Live`). Items 3 and 10: the probe of the router renews a WARP member on `Down`,
`WrongEgress` and `AuthExpired`, make-before-break, from the stored identity file, with a gap of 30 s; a refused key
enrols a new key for the same device once (`src/router/probe.rs`). Item 7: the session of a WARP member runs with the
probe of `Live` off, so the control of the router gates the renewal; an outage of each member renews nothing. Item 9: the
egress check of a WARP member is the exit trace (`warp=on` or `warp=plus`); an entry of `health.egress` goes first.
Tests: `router_health_renews_a_member_that_goes_down`, `router_health_outage_of_each_member_renews_nothing`,
`router_health_auth_expired_renews_at_once_and_keeps_a_gap`, `router_health_does_not_renew_a_network_that_does_not_renew`,
`router_health_runs_the_egress_check_of_the_network`, `router_health_egress_of_the_operator_goes_before_the_network`,
`router_health_probes_and_renews_a_warp_member`, `warp_network_egress_check_reads_the_trace`,
`router_probe_result_of_a_failure_names_the_credential` (on the WARP edge stand-in of `pod-warp`). Not proven: a renewal
against the real WARP edge.

# T-098: Router: doors with a default network, and rules to a network, a group or a refusal

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 to 2.3; GitHub #372 and #374.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The rules of `warp proxy` have the actions tunnel, direct and reject only. A door cannot send a
flow through another network.

## Premise

- `src/warp/rules.rs` holds the rule actions. T-061 gives one door over a dialer.

## Approach

1. Add `--door LISTEN=NET` (a network or a group): SOCKS5, HTTP CONNECT, UDP ASSOCIATE and an
   optional DNS, on TCP or `unix:`.
2. Add rules `domain:` and `cidr:` to a network, a group or `reject`. The first match wins. A
   change of the rules file takes effect with no restart.
3. Each member carries its resolver (`pod_dns::Resolver`). A flow pinned to a member resolves through
   it (GitHub #374). Land it with the DNS door of T-099. A `WrongEgress` member (T-097) leaves the pick
   and no direct dial replaces it.

## Prove

```sh
cargo test --workspace -- router_rules_pick_the_network
```

The test uses unix sockets only, so it runs in the job `restricted-host`.

## Start condition

T-061 and T-096 are done.

## Correction

2026-11-11: T-061 is partial (the door over a dialer is done; the WARP door waits for `pod-warp`), and this
entry needs only the generic door. Done so far (02c4bec): `crates/podnet-vnet/src/routes.rs`, `parse` (the rules
`domain:` and `cidr:` of `pod -r` to a named way, a group or `reject`) and `RulesFile` (a `Dial`: the file is
read again when its time of change moves; a broken file keeps the last rules with a warning; an unknown
way is an error at the start). A door of T-061 over a `RulesFile` is a door of the router. Test:
`cargo test -p podnet-vnet --lib router_rules_pick_the_network` (unix sockets only). Left: the flag
`--door LISTEN=NET` belongs to the command `podnet route` (T-099, T-100); item 3, the resolver of each
member, lands with the DNS door of T-099.

2026-11-11, 463aecd: item 1 is the key `doors` of the router file (T-100), not a flag: each door has `listen`
(TCP or `unix:`), `via` (a network or a group) and `rules`. A door can have a DNS door: `dns: ADDR:PORT` and
`dns_upstream: HOST:PORT` answer on UDP and TCP over the dial of the door (`forwards::serve_dns` of T-099).
The upstream is needed in the file: the router names no resolver of another party. Item 3: a name of a
flow goes to its member (`names_inside`), and the DNS door resolves over the way that the rules give, so a
member resolves through its own network. Test: `route_door_dns_reaches_the_router`, new cases of
`route_config_errors_name_the_key`. Measured: a router with a DNS door over `direct` gave the answer of a
local upstream (id 1234, flags 8180). Left: UDP ASSOCIATE in the door (`crates/podnet-netns/src/door.rs`
answers "command not supported"; T-061), and the `WrongEgress` member of item 3 (the egress check of T-097
item 9, which waits for `pod-warp`).

2026-11-12, d80e561 and bc4b06a: every item is done. UDP ASSOCIATE: `crates/podnet-netns/src/door/udp.rs` relays the
SOCKS5 datagrams of one client over `Dial::datagrams` of the door (the group's pick, the byte meter counts them). The
first datagram fixes the client; other sources and fragments are dropped; an answer goes back only from an address that
the client sent to; the control stream ends the association (300 s idle). A way with no datagrams answers 0x07, a group
that the kill switch closed answers 0x02. A door on a TCP address answers with that address (`Door::with_udp_bind`). Item
3: `resolvers: NETWORK: HOST:PORT` (`src/router/resolving.rs`) makes a member resolve the names of its flows through its own
network: A and AAAA over DNS on TCP over the dial of the member, IPv4 first, up to 8 addresses, no fallback to the host
resolver; a way that resolves names at the far end gets the name as it is. The `WrongEgress` member leaves the pick
(T-097, f4dd392), and the group never falls back to direct. The WARP trace check that makes a WARP member give
`WrongEgress` stays in T-097 (`pod-warp`).

## Done

2026-11-12, commits d80e561 and bc4b06a. Tests: `front_door_udp_associate_carries_datagrams`,
`front_door_udp_associate_relays_answers_from_known_peers_only`, `front_door_udp_associate_replies_for_a_way_with_no_datagrams`
(`podnet-vnet`), `route_door_udp_associate_reaches_the_router`, `route_config_resolvers_errors_name_the_key`,
`route_resolver_pinned_member_resolves_through_its_network`, `route_resolver_name_resolves_through_the_member`,
`route_resolver_leaves_addresses_and_far_end_names_alone`, `route_resolver_unknown_name_is_an_error_with_no_fallback`,
`route_resolver_tries_the_next_address` and the existing `router_rules_pick_the_network`. The door tests failed first on the
old answer (0x07 for each association); `route_door_udp_associate_reaches_the_router` failed first with the door
address 127.0.0.1 in place of 127.0.0.2. Not proven: UDP ASSOCIATE with a real SOCKS5 client program (the tests speak the
protocol by hand); a member resolver against a real DNS server of a tunnel; `--door LISTEN=NET` is the key `doors` of the file
(T-100), not a flag.

# T-099: Router: `podnet forward` for each network, UDP forwards, a DNS door, and `podnet expose`

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 to 2.3;
`docs/research/podnet/mwan-field.md`, section 3 (F4b); GitHub #372.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

A port forward exists for WARP only (`warp forward`, `src/warp/forward.rs`). No forward carries
UDP. The DNS door works for WARP only. Each way in to a local service is its own command.

## Premise

- `warp forward -L` dials through WARP. `warp proxy --dns` answers DNS through WARP.
- `tunnel`, `relay node` and `net proxy --forward` each expose a local service.

## Approach

1. Add `podnet forward -L LISTEN=NET:HOST:PORT` for each network. Keep `warp forward` as an
   alias.
2. Add UDP forwards, and a DNS door for each network.
3. Add `podnet expose TARGET` with a choice of the tunnel, the relay or the switch, as an alias of
   the three commands.
4. A pump that carries UDP over a byte stream keeps a pending buffer with an offset. It writes the rest on
   the next poll, bounds the queue, and pauses the flow (no failover, no close) when the queue is full. A
   full queue stops reading the source (GitHub #372). The existing writers use `write_all` and are safe.
   The smoltcp socket items are in T-176.

## Prove

```sh
cargo test --workspace -- forward_over_a_network forward_udp_echo dns_door_over_a_network expose_via_relay_is_a_relay_node
cargo test --workspace -- a_partial_stream_write_keeps_the_record_stream_aligned
```

## Start condition

T-059 and T-062 are done.

## Correction

2026-11-11: the network is a flag of its own, `--network SPEC`, not a prefix of the target: `LISTEN=NET:HOST:PORT`
cannot tell the end of a spec of a URL (`socks5h://HOST:PORT`) from the host. Done so far (1058f08): `src/forwards.rs`
(`Rule`, `serve_tcp`, `serve_unix`) and `podnet forward -L LISTEN=HOST:PORT [--network SPEC] [--run-for N]`
(`src/cli/cmd/forward.rs`). Tests: `cargo test -p podnet --lib forward_` (the parse; `forward_over_a_network`).
Measured in this sandbox: a forward on a unix socket through the HTTP proxy of the sandbox to
`api.github.com:443` answered HTTP 200 to curl. Left: `warp forward` as an alias waits for `pod-warp`
(`src/warp/forward.rs` is frozen). Left after 2026-11-11: only `warp forward` as an alias (it waits for
`pod-warp`).

2026-11-11, commits cc28071 and 81a3ecd. Item 2: `podnet forward -L udp:LISTEN=HOST:PORT` and
`podnet forward --dns ADDR:PORT [--dns-upstream HOST:PORT]` (`src/forwards/udp.rs`). Item 3: `podnet expose
TARGET [--via tunnel|relay|switch] [--as VIRT] [-- ARGS]` (`src/cli/cmd/expose.rs`) builds the arguments of
`tunnel`, `relay node` or `net proxy --forward`, parses them and runs that command. Item 4: the UDP forward
has no byte stream, so its queue is a bounded channel for each flow (64 datagrams); a full flow drops the
datagram (UDP has no flow control). The pump of the byte stream (`carry_in_stream` in
`crates/podnet-netns/src/netns/udpflow.rs`) already writes each frame with `write_all`; its test
`a_partial_stream_write_keeps_the_record_stream_aligned` passes (a stream of 5 bytes for each write, 200
datagrams in order). A name target of a UDP forward resolves with the resolver of the host, because
`Datagrams` takes an address. A SOCKS or HTTP proxy network carries no UDP: the forward logs the refusal and
ends the flow.

2026-11-12: the entry was read again with T-097 and T-098. The only item left is the alias `warp forward` (item 1). It waits
for WARP to be a network of the registry (`podnet forward --network warp` would be the alias): `src/warp/forward.rs` and the
WARP session are frozen for `pod-warp` (T-050), and the registry has no `warp` network yet (T-028). `warp forward` keeps its
own options (`--list`, the detached instances), so an alias in the CLI only would change them. Docs: `docs/forward.md` now
lists `netbird:` and `wg:FILE` as networks and says which networks carry UDP. The SOCKS5 UDP ASSOCIATE of the router
door (T-098) and the resolver of a member (T-098) are the router's answers for UDP and names through a chosen network.

2026-11-12, GitHub #466 (row E0-5) and #471 (row E5-4). The forward of a local port to a peer service (`forward -L 8080=peer:FP/http`) is T-266. This entry does not cover a peer target.

## Done

2026-11-11, commits cc28071 and 81a3ecd (partial: `warp forward` as an alias waits for `pod-warp`). Tests:
`cargo test -p podnet --lib forwards::` (5 passed: `forward_rule_parses_each_form`, `forward_over_a_network`,
`forward_udp_echo`, `an_idle_udp_flow_ends`, `dns_door_over_a_network`), `cargo test -p podnet --bin podnet
expose` (3 passed: `expose_via_relay_is_a_relay_node`, `expose_via_tunnel_picks_the_origin_flag`,
`expose_via_switch_needs_a_virtual_address`), `cargo test -p podnet-netns a_partial_stream_write_keeps_the_record_stream_aligned`
(passed on the existing code). Each new test failed first on a stub (a 10 s timeout, or an error). By hand:
`podnet forward --network direct -L udp:45353=127.0.0.1:P --dns 127.0.0.1:45354` answered an echo on both
doors. Not proven: UDP through a real WARP, relay or tailnet network (the tests use `direct`); `expose` runs
the other commands as they are (it starts no tunnel in a test). `sh scripts/agent/check.sh`: exit 0.

2026-11-13, done, the work of the handoff (the patch wip-warprouter), rebased on b1e2b80, commit 3855573. `podnet forward --network warp` takes the network `warp` of the
registry: TCP, `-L udp:...` and `--dns` through the session; `warp forward` stays the alias of `podnet forward` with the
options of `podnet warp` (`docs/forward.md`). Tests: `network_spec_warp_is_in_the_registry`,
`network_spec_warp_forms_are_checked`, `warp_network_carries_tcp_and_a_name_inside_the_tunnel`,
`warp_network_carries_udp_datagrams`.

# T-100: Router: a config file, and `podnet route` with `--detach` and autostart

**Source:** `docs/research/podnet/capabilities.md`, section 2.3;
`docs/research/podnet/mwan-field.md`, section 2.2 (R3).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The parts of T-095 to T-099 have no command. A router with several networks, groups and doors is
too long for a command line.

## Premise

- `crates/podnet-core/src/yaml.rs` reads YAML with no new crate.
- `crates/podnet-core/src/detach.rs` and `crates/podnet-core/src/autostart.rs` supervise a command and start it at boot.

## Approach

1. Add `podnet route [--config FILE]`, with the keys of the flags in the file.
2. Read the file with `crates/podnet-core/src/yaml.rs`.
3. Make `--detach --name NAME` and `autostart add` work with it.

## Prove

```sh
cargo test --workspace -- route_config_errors_name_the_key
cargo test --test detach -- route_detaches_and_lists_its_members
```

## Start condition

T-097 and T-098 are done.

## Done

2026-11-11, 5c9c9e6. `src/router.rs` reads the config file with `podnet_core::yaml`. The keys are
`networks` (NAME: SPEC of the registry), `groups` (NAME: MEMBER*WEIGHT, ...), `health` (`target`,
`interval`) and `doors` (`listen`, `via`, `rules`). An unknown key fails. An error names the key
(`doors[0].via`, `groups.NAME`, `health.target`). `podnet route --config FILE` brings up each network,
each group (a `Balance` with the probe of `health`) and each door (SOCKS5 and HTTP CONNECT, with the
rules file of T-098). `--check` reads the file only. `--detach --name NAME` gives a ready document with
the doors and the status of each group. `podnet autostart add NAME -- route --config FILE` takes the
command. Tests: `route_config_errors_name_the_key`, `route_config_reads_networks_groups_doors_and_health`,
`route_router_up_builds_groups_and_doors`, and `route_detaches_and_lists_its_members` (`ps`, `stop`, a
connection through the door, exit 64 for a bad file). Measured: a door on a unix socket over a group
answered `HTTP/1.1 200 Connection Established`, and curl through the door got HTTP 200.

# T-101: Router: the live step with WARP, the relay and direct, and a failover

**Source:** `docs/research/podnet/capabilities.md`, section 2.5;
`docs/research/podnet/mwan-field.md`, section 4 (rank 13); the owner's ruling of 2026-11-05
(`docs/decisions.md`, Live WARP devices).
**Category:** measurement
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** partial

## Problem

The loopback tests use fake upstreams. Only a live run proves the router with a real WARP session
and the real relay.

## Premise

- The live job drops UDP 7844 already, with iptables (`.github/workflows/live.yml`).
- `http://1.1.1.1/cdn-cgi/trace` shows `warp=on` through WARP.
- The WARP service limits new devices for each address: live run 41987488063 got HTTP 429
  (error 1015) at the nested session step. Each device that a step registers adds to this risk.

## Approach

1. Run `podnet route` with the networks `warp`, `relay` and `direct`, a door on TCP and a group.
   Register the second device in the step that needs it, and honour the `Retry-After` of a 429
   once before the step fails.
2. Check `warp=on` through `warp` and `warp=off` through `relay`.
3. Drop TCP 443 to the MASQUE front door with iptables. Check that the group fails over to the
   relay. Restore the rule.
4. Register a second WARP device for the group, and delete it at the end of the step.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Live WARP devices): a live test of the router
registers and deletes a WARP device in each run.

## Prove

```sh
gh workflow run live.yml
```

The step "router" passes.

## Start condition

T-100 is done.

## Correction

2026-11-11: the start condition is not enough. Step 1 needs the network `warp` of the registry
(`src/networks.rs` has no WARP spec), and that network is T-057 (a dialer and datagrams for WARP), which
waits for `pod-warp` (T-050). The step with `relay` and `direct` alone proves no WARP. T-101 starts when
T-057 is done.

2026-11-13, the lead: T-057 is done (the network `warp`, 3855573), so the entry starts. `.github/workflows/live.yml` has
the step "Router with WARP, the relay and a failover": it registers its own device (one wait for a 429), runs `podnet
route` with `w: warp:identity=FILE`, `r: relay` and a group `g` (policy `protect`) with a door for each, checks
`warp=on` through `w` and `warp=off` through `r`, refuses TCP 443 to the MASQUE front door (a reset rule), waits at most
90 s for `warp=off` through `g`, restores the rule, and deletes the device. The config was checked with `direct` in
place of `warp` (the router starts; `down_after: 3`, `up_after: 2` hold K > R > 1 and R > K/2). It waits for a run
(GitHub #531).


# T-102: Take the mailbox client, the code words and the SPAKE2 phases from `pod-wormhole`

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.2 and 3 (C8);
`docs/research/podnet/capabilities.md`, sections 4.2 to 4.4; the owner's rulings of 2026-11-05
(`docs/decisions.md`, File transfer and Crypto crates).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

podnet has no rendezvous: no short code and no PAKE. `share` gives a long URL, and Cloudflare
sees the plain bytes.

## Premise

- podssh has the client of the magic-wormhole mailbox over a WebSocket
  (`wss://relay.magic-wormhole.io/v1`, then `wss://mailbox.mw.leastauthority.com/v1`), SPAKE2
  (`spake2 0.4`), phases sealed with SecretBox (`crypto_secretbox`), and the PGP word list
  (`Azathothas/podssh:crates/podssh-relay/src/code/`, about 1,185 lines).
- Its run and exchange files are the pairing of podssh, not a file transfer.

## Approach

1. Take `pod-wormhole` (wave 3 of PodUtils/podcrates): the mailbox client, the code words
   and the SPAKE2 phases.
2. Give the crate a dialer of podnet (`egress::dial`, later `pod-proxy`), so that the proxy applies.
3. Set the AppID of the magic-wormhole file transfer (`lothar.com/wormhole/text-or-file-xfer`).
   Add a mailbox variable with its row in `src/manual/facts.rs`.
4. The unit tests stay in `pod-wormhole`. Add a loopback test of podnet against a mailbox stand-in.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, File transfer and Crypto crates): `podnet send` and
`podnet receive` speak magic-wormhole (mailbox, SPAKE2, transit), with the public mailbox and
transit servers as the default. `spake2` 0.4 and `crypto_secretbox` are accepted.

The owner, 2026-11-05 (`docs/decisions.md`, Shared crates): the shared crates of `PodUtils/podcrates` hold this code. podnet depends on the shared crate; it does not copy the code.

## Prove

```sh
cargo tree -p podnet --depth 1 | grep pod-wormhole
cargo test --workspace -- wormhole_rendezvous_
cargo deny check
```

A wrong code fails at both ends. A loopback test runs against a stand-in of the mailbox.

## Start condition

T-049 is done.

The crate of this entry has a release in `PodUtils/podcrates`, with its hand-off `adoption/podnet/CRATE.patch` (T-137; `docs/maintenance.md`, Shared crates).

## Correction

2026-11-05, The work goes to `pod-wormhole` in PodUtils/podcrates. podnet only takes the crate (T-137). Do not do it in podnet. The title and the steps are changed. There is no crate `podnet-relay`.

2026-11-13, The take (the lead's order; `docs/decisions.md`, 2026-11-13, The transfer layer). podnet depends on
`pod-wormhole` v0.40.0 and copies none of its code. The glue is `src/transfer/mod.rs`: the `Config` of the crate
with the AppID `lothar.com/wormhole/text-or-file-xfer`, the client `podnet` and the way out of podnet
(`podnet::ws::way()`: `--proxy`, `--egress`, `NO_PROXY`, `--resolve`, `--ca-file`); the mailboxes of the kind
`mailbox` of `pod-services`, or `--mailbox` and `PODNET_MAILBOX` (its row is in `src/manual/facts.rs`); the exit
code of each `Failed`. The crate keeps its stand-in mailbox in its own tests, which a dependent crate cannot use,
and `pod-testkit` has none: the stand-in of podnet is `tests/transfer_harness/mod.rs` (an ask for podcrates: a
stand-in mailbox behind a feature). A wrong code exits 77 at both ends; no giver at a code exits 69; no receiver
within `--timeout` exits 124.

## Done

2026-11-13, commit d5c38c1. `cargo tree -p podnet --depth 1 | grep pod-wormhole` gives
`pod-wormhole v0.40.0 (https://github.com/PodUtils/podcrates?tag=v0.40.0#1335e9af)`. `cargo test -p podnet --test
transfer`: 9 passed, among them `wormhole_rendezvous_two_ends_move_a_text` and
`wormhole_rendezvous_a_wrong_code_fails_at_both_ends` (exit 77 at the two ends, the mood `scary` at the stand-in);
the unit tests `wormhole_rendezvous_the_mailboxes_come_from_the_flags_the_variable_or_pod_services` and
`wormhole_rendezvous_a_failure_has_the_exit_code_of_podnet` passed. Plant: with the check of the peer's `version`
removed from the receiver (`src/transfer/receive.rs`), `wormhole_rendezvous_a_wrong_code_fails_at_both_ends` failed
(exit 1, "the other end said nothing for 30 s", in place of 77). `cargo deny check` (cargo-deny 0.20.2, the
release of `deny.yml`, checked by its SHA-256): advisories ok, bans ok, licenses ok, sources ok; the duplicate
warnings go from 27 to 31 (`salsa20` and the crates of `crypto_secretbox` of `pod-wormhole`).
Not proven: a live meeting at the public mailboxes (T-253, T-106).


# T-103: The file-transfer protocol and the transit protocol of magic-wormhole

**Source:** `docs/research/podnet/capabilities.md`, section 4.3;
`docs/research/podnet/podssh-copy.md`, sections 2.2 (the wire choice) and 3 (C9); the owner's
ruling of 2026-11-05 (`docs/decisions.md`, File transfer).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** L
**Status:** blocked

## Problem

podssh has the mailbox and SPAKE2 parts, but not the file-transfer protocol of magic-wormhole: the
offer, the answer, the transit hints, the transit handshake and the records. Without them,
`wormhole send` and `podnet receive` cannot talk.

## Premise

- podssh has a transit carrier (`Azathothas/podssh:crates/podssh-relay/src/transit/`, 1,013
  lines). Its hello and proof are its own words after the relay line of the transit relay.
- magic-wormhole sends a directory as a zip file. No crate of the lock reads deflate.
- A host with TCP 443 only through a proxy needs the transit relay on 443 (the WebSocket relay of
  Least Authority).

## Approach

1. Write the version phase and the offer and answer phases: a text, a file and a directory.
2. Write a directory as a zip with no compression. Read a deflated zip with an inflate decoder:
   choose between own code and a pure Rust crate, and write the choice with its `cargo deny`
   result in the Decision section.
3. The transit carrier is `pod-transit` (wave 3). The transit protocol of magic-wormhole (the hints,
   the relay handshake, the sender and receiver handshakes, and the records sealed with SecretBox
   under the transit key) is in `pod-transit` or `pod-wormhole`: read both crates first.

## Prove

```sh
cargo test --workspace -- wormhole_transfer_
```

Each phase is checked against messages captured from the Python magic-wormhole. A changed record
fails.

## Blocker

PodUtils/podcrates must act: `pod_wormhole::Wormhole` gives no derivation of its key (magic-wormhole's
`derive_key(purpose, length)`, HKDF-SHA256 of the key of the wormhole). magic-wormhole's transit key is
`derive_key("lothar.com/wormhole/text-or-file-xfer/transit-key", 32)`. Without it, a file or a directory goes
from podnet to podnet only. The lead opens the COORD issue (to podcrates). PodUtils/podcrates T-214 (the
file transfer in `pod-wormhole` and `pod-transit`) is open too.

## Start condition

T-102 is done.

## Correction

2026-11-05, The file-transfer protocol and the transit protocol are work of `pod-wormhole` and `pod-transit` in
PodUtils/podcrates. podnet takes the crates (T-137, step 3). The inflate decoder (step 2) is a call made
without a ruling: it belongs to the crate that reads the zip.

2026-11-13, The layer is in podnet (the lead's order; `docs/decisions.md`, 2026-11-13, The transfer layer): no
crate of podcrates v0.40.0 holds it (`pod-transit` does not exist; PodUtils/podcrates T-214 and T-027 are open).
Done in `src/transfer/`: `messages.rs` (the `transit`, `offer`, `answer` and `error` messages, a text, a file and
a directory; a hint of an unknown kind is ignored), `transit.rs` (the keys `transit_sender`, `transit_receiver`,
`transit_relay_token`, `transit_record_sender_key` and `transit_record_receiver_key`; the handshakes; the records
with a big-endian nonce from 0, as transit.py writes it, with a bound of 64 MiB), `connect.rs` (a listener and the
direct hints of the addresses of the host, the dials through the way of podnet, the relays after 2 s, the relay
handshake over TCP and over a WebSocket of `pod-ws` with `pod_session::ws_link`, `go` and `nevermind`),
`zip.rs` (a stored zip of a directory; stored and deflated entries read with `flate2`, its Rust backend, the one
of `pod-extract`; no zip64). The forms were read from magic-wormhole/magic-wormhole-protocols:
file-transfer-protocol.md and transit.md and from magic-wormhole/magic-wormhole:src/wormhole/transit.py,
cmd_send.py and cmd_receive.py (0.24). The Prove's "messages captured from the Python magic-wormhole" are not
available: no Python `magic-wormhole` is on this host, and it is not installed from the network. In their place:
the HKDF keys are checked against values computed with the hmac and hashlib of Python, a zip of Python's
`zipfile` (deflated) is a fixture (`tests/data/transfer/python-deflated.zip`), and Python's `zipfile` reads the
zip of podnet (`transfer_land_python_reads_the_zip_of_podnet`). The interim transit key: a podnet sender draws 32
random bytes and sends them as `podnet-transit-key-v1` in its sealed `transit` message; the two ends say
`{"podnet": {"transit-key": 1}}` in their `version`, and a podnet end that meets another client for a file stops
with a line that says why. What is left: the derivation of the key (Blocker), then the interop test (T-106).

The proof so far (2026-11-13, commit d5c38c1): `cargo test --workspace -- wormhole_transfer_` passed: the unit tests of `src/transfer/` (11, among
them `wormhole_transfer_the_keys_are_hkdf_of_the_transit_key`,
`wormhole_transfer_records_go_both_ways_and_a_changed_one_fails`, `wormhole_transfer_a_record_out_of_order_fails`,
`wormhole_transfer_hints_of_unknown_kinds_are_ignored`) and `tests/transfer.rs`:
`wormhole_transfer_a_file_lands_with_the_sha256_of_the_sender` (700,000 bytes, direct),
`wormhole_transfer_a_directory_lands_with_the_sha256_of_each_file`,
`wormhole_transfer_goes_through_the_relay_when_no_end_listens` (a stand-in TCP transit relay),
`wormhole_transfer_with_no_way_fails_with_69`, `wormhole_transfer_a_refused_offer_writes_nothing`. Plant: with the
check of the nonce of a record removed (`src/transfer/transit.rs`), `wormhole_transfer_a_record_out_of_order_fails`
failed.

# T-104: The podnet relay as a carrier of a transfer

**Source:** the owner's ruling of 2026-11-05 (`docs/decisions.md`, File transfer);
`docs/research/podnet/capabilities.md`, section 4.3 (road 1).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

Two hosts that reach only TCP 443 through a proxy may not reach a transit relay. The owner named
the podnet relay as one more carrier.

## Premise

- T-087 resumes a relay session past the cap of 64 MiB.
- A client of magic-wormhole ignores a transit hint of a kind that it does not know.

## Approach

1. Add a transit hint of podnet's own kind. The sender pairs a relay node and sends the connect
   token inside a sealed phase.
2. The receiver connects through the relay. The layer resumes past the cap.

## Prove

```sh
cargo test --workspace -- transfer_over_the_relay_crosses_the_cap
```

## Start condition

T-087 and T-103 are done.

## Correction

2026-11-05, The transit carrier is `pod-transit`, and the relay client is `pod-relay`. The hint of podnet's own
kind is a change of `pod-transit` (PodUtils/podcrates). There is no crate `podnet-relay`.

2026-11-13, Nothing of this entry is done. The transit of magic-wormhole is now in podnet (`src/transfer/connect.rs`,
T-103): a new kind of hint is a variant of `Hint` in `src/transfer/messages.rs` and one more dial in
`connect::connect`; a client of magic-wormhole ignores it. The words of the hint `podnet-relay-v1` are in
PodUtils/podcrates T-214 (`relay`, `pair`, `resume`; the connect token in a sealed phase with the key
`podnet-relay-token`). What is left: the hint, the pair of a relay node by the sender, the connect of the receiver
through `pod-relay` with `pod-session` past the cap, and the test of the Prove.

# T-105: `podnet send` and `podnet receive`

**Source:** `docs/research/podnet/capabilities.md`, sections 4.2, 4.5 and 4.7; the owner's ruling
of 2026-11-05 (`docs/decisions.md`, File transfer).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The parts of T-102 and T-103 have no command.

## Premise

- A code in argv is visible to other users in the process list.
- One wrong guess must end the code.

## Approach

1. Add `podnet send PATH...` with `--via`, `--mailbox URL`, `--words N` and `--text TEXT`. It
   prints the code.
2. Add `podnet receive` with `--out DIR`, `--yes` and `--via`. It reads the code from the prompt
   or from stdin, never from argv.
3. Check the SHA-256 of each file. Map each fault to an exit code of `crates/podnet-core/src/exit.rs`.

## Prove

```sh
cargo test --test cli -- send_and_receive_on_the_loopback a_wrong_code_ends_the_code
```

## Start condition

T-103 is done.

## Correction

2026-11-05, The command stays in podnet. It uses `pod-wormhole` and `pod-transit` (T-102, T-103).

2026-11-11, GitHub #436 (https://github.com/PodUtils/podnet/issues/436; donor review, tunnet.md block 6;
the donor is MPL-2.0 and serves as a counter-example: implement from the description). This entry has no rule
for the names of the received entries. Write the rules before the code, so that a later retrofit does not
change the behaviour. Add them to the Approach:

1. Run `pod_extract::safety::components` on each name. A refusal ends the whole transfer before any byte is
   written, with `refused entry "NAME": REASON`. Do not rewrite a bad name.
2. Also refuse, in podnet code: a backslash in a component, a component over 255 bytes, a path over 4096
   bytes, and two entries that are equal after ASCII case folding.
3. Refuse a symlink, a hard link, a device and a fifo. Accept regular files and directories only.
4. Land a file with `open_parent` of the output directory, then `openat` with
   `O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW` and the mode `0666 & ~umask`, never with an executable or set-id bit.
   `EEXIST` is an error that names the file. The receiver never overwrites.
5. The top name of a directory transfer follows the same rules. If it exists, refuse unless `--into` names
   an existing directory.
6. Caps: `--max-files N` and `--max-bytes N`, checked from the zip directory before the write and again
   during the write, because a header can lie. The maintainer chooses the defaults.
7. Read the zip directory, validate each entry, then write. A hostile zip changes nothing on the disk. If a
   write fails midway, remove the files that this run created, never a file that it did not create.
8. `receive` without `--yes` shows the names and the total size, then asks on the terminal. With a stdin that
   is not a terminal and no `--yes`, refuse.

Add to the Approach: the dependency on `pod-extract` (it is not a dependency of podnet today; or the reader of
the zip of T-103 brings it), and a new exit code for refused input in `crates/podnet-core/src/exit.rs`
(EX_DATAERR is 65; the file has `Usage`, `Unavailable`, `Config`, `TempFail` and `Timeout`, and no class
for a refused archive). Add to the Prove a table test over zips built in the test, each against an empty
output directory, with the listing of the directory and of its parent compared before and after:
`../x`, `a/../../x`, `/etc/x`, `a\..\x`, a NUL in the name, a component of 300 bytes, `A` and `a` together
(each refused, nothing written); an entry `evil` that is a symlink to `/tmp` followed by `evil/f`; a zip with
an existing `f` (refused with `EEXIST`, the old bytes stay); a header that says 10 bytes with 10 MiB of data
under `--max-bytes 1048576`; two receivers that write one name at once (one wins; the planted defect `exists()`
then `create` must fail the test); and a positive control (`a/b.txt` and `c.txt` land with the bytes of the
sender and the mode `0666 & ~umask`). Not read: `safety.rs` of `pod-extract` (the issue cites its lines at
podcrates 63348de; Unverified).

2026-11-13, The calls of the commands (the lead's order; the owner is away):

1. `podnet send PATH` takes one path, as `wormhole send`: a directory goes as one zip. `--text TEXT` sends a
   text. `--timeout SECS` (default 1800) is the wait for the receiver and its answer.
2. No `--via`: the way out of a transfer is the way of podnet (`--egress`, `--proxy`, `NO_PROXY`). The carriers of
   the transit are `--listen ADDR`, `--no-listen` and `--transit-relay URL` (`none` for no relay of this end).
3. No `--words N`: `pod_wormhole::meet::opened` makes each code with `Code::make`, which takes
   `pod_wormhole::WORDS` (3) words. An ask for podcrates: a count of words in `Ends` or `Config`.
4. `podnet receive` reads the code from the terminal, or one line of stdin; never from argv (the Premise). A code
   on the command line is a usage error (64).
5. The rules of GitHub #436 (the Correction of 2026-11-11) are `src/transfer/land.rs`. The defaults of the caps
   are `--max-files 100000` and `--max-bytes 68719476736` (64 GiB). The new exit class is `Fault::DataError`, 65,
   `data-error` (`crates/podnet-core/src/exit.rs`). `--into` is not added: a top name that exists is always
   refused (EEXIST), and `--out DIR` names another directory. `pod-extract` v0.40.0 is a dependency for
   `pod_extract::safety::components`.
6. `--json` prints one object for each event on stdout (`code`, `sent`, `received`).

## Done

2026-11-13, commit d5c38c1. `src/cli/cmd/transfer.rs` (`podnet send`, `podnet receive`), `src/transfer/`,
`docs/transfer.md` (with its row in the map of `README.md`, the list of `scripts/gen-llms.py`, and the llms files),
the exit code 65 in `docs/cli.md` and `podnet man exit-status`. `cargo test --test cli -- send_and_receive_on_the_loopback
a_wrong_code_ends_the_code`: both passed (a file of 400,000 bytes with the sender's SHA-256, a directory and a text
between two processes of podnet at a stand-in mailbox; a wrong code exits 77 at both ends, and the right code after
it exits 69). Also passed: `receive_takes_no_code_from_the_command_line` (64),
`receive_of_a_file_needs_yes_when_stdin_is_not_a_terminal` (64, the sender 1, nothing written),
`receive_refuses_a_name_that_exists_with_65`. The table test of GitHub #436, `cargo test --test transfer_land`: 7
passed (`transfer_land_refuses_each_hostile_name_and_writes_nothing`: `../x`, `a/../../x`, `/etc/x`, `a\..\x`, a NUL,
a component of 300 bytes, `A` with `a`, a symlink `evil` then `evil/f`, a fifo, a device, each refused with 65 and
the listing of the directory and of its parent unchanged; `transfer_land_a_header_that_lies_is_cut_at_its_size`: a
header of 10 bytes with 10 MiB of data under a cap of 1 MiB; `transfer_land_an_existing_name_is_refused_and_keeps_its_bytes`;
`transfer_land_two_receivers_of_one_name_one_wins`, 50 races; `transfer_land_a_good_zip_lands_with_the_bytes_and_the_mode_of_the_rules`,
mode `0666 & ~umask` for an entry of mode 04755; `transfer_land_a_deflated_zip_of_python_lands`;
`transfer_land_python_reads_the_zip_of_podnet`). Plant: with `O_EXCL` replaced by `O_TRUNC` in `Landing::file`,
`transfer_land_an_existing_name_is_refused_and_keeps_its_bytes` and `transfer_land_two_receivers_of_one_name_one_wins`
failed.
The exit code 65 is also in the connector profile (`docs/protocol.md`, section 10, and `tests/cli/profile.rs`), and
the log filter names `pod_wormhole` and `pod_extract` (`crates/podnet-core/src/report.rs`). `sh scripts/agent/check.sh`:
fmt, clippy, record, size, doc and layers 0; the tests of the root package: 1472 passed, 0 failed (102 targets); then
`cargo test -p podnet-core -p podnet-netns -p podnet-serve -p podnet-todo -p podnet-vnet -p podnet-capi`: 599
passed, 0 failed; the doc tests of podnet: 2 passed. The disk of the session filled in the first whole run, so the
packages after the root ran in a second command.
Not proven: a transfer with a client of magic-wormhole for a file (T-103, Blocker; T-106); a transfer over the
public mailbox and relay (T-106); `--into` (not added, item 5 above).

# T-106: An interop test with the Python magic-wormhole, and the live step of a transfer of 100 MiB

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.2 and 3 (C15);
`docs/research/podnet/capabilities.md`, section 4.7.
**Category:** measurement
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

Unit tests against captured messages do not prove that `wormhole send` and `podnet receive` talk.
Only a real public mailbox and the real relay prove the default path.

## Premise

- podssh runs magic-wormhole's Python library and mailbox server on the loopback
  (`Azathothas/podssh:scripts/wormhole-peer.py`,
  `Azathothas/podssh:.github/workflows/pair-code.yml`).

## Approach

1. Add a workflow: a venv with `magic-wormhole`, its mailbox server and its transit relay on the
   loopback. Run `wormhole send` to `podnet receive`, and `podnet send` to `wormhole receive`.
2. Add a live step: a random file of 100 MiB through the public mailbox and transit, and through
   the relay. Compare the SHA-256.

## Prove

```sh
gh workflow run wormhole-interop.yml
gh workflow run live.yml
```

## Start condition

T-104 and T-105 are done.

## Correction

2026-11-05, `wormhole-peer.py` and `pair-code.yml` are scripts and a workflow of podssh. Do not copy them into
podnet. Read whether `pod-testkit` or `pod-wormhole` of PodUtils/podcrates has the interop harness; when it
has not, write the harness in podnet (it is a CI file, no code of a crate).

2026-11-13, Nothing of this entry is done. No Python `magic-wormhole` is on this host, and the entry does not say to
install it from the network. A text between podnet and `wormhole send` can go now; a file or a directory waits for
the derivation of the key of `pod-wormhole` (T-103, Blocker). The stand-ins of `tests/transfer_harness/mod.rs` (a
mailbox and a TCP transit relay) serve the loopback tests; the workflow of this entry runs the server and the
relay of magic-wormhole in place of them.

# T-167: `net socksify`: route chosen destinations, not only the tailnet or all (GitHub #364)

**Source:** GitHub #364 (https://github.com/PodUtils/podnet/issues/364)
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The socks shim routes a stream `connect` to the front door only for `100.64.0.0/10` and
`fd7a:115c:a1e0::/48`, or for each connect with `PODNET_SOCKS_ALL=1` (`crates/podnet-vnet/shim/podnet-socks.c:356-374`).
A client with a hardcoded origin (`127.0.0.1:3000`, a fixed LAN address) needs only that address routed.
The abstract-name shim has no option to hand a chosen address to the SOCKS door.

## Premise

- `parse_config` reads `PODNET_SOCKS_PROXY`, `_LOG`, `_ALL`, `_MAX_FDS` (`crates/podnet-vnet/shim/podnet-socks.c:239-251`).
- `net socksify` prints a fixed environment (`src/cli/cmd/net/shim.rs:125-134`). `ShimOptions`
  (`crates/podnet-vnet/src/shim.rs:30-50`) has no hand-off list. `podnet-vnet.c:676` translates each IP of a virtual socket.
- The variable list is in `src/manual/facts.rs:346`. `docs/virtual-network.md` has no text on socksify
  (README.md:921 only), so item 4 of the issue needs a new section.
- Names are set by T-016 (`PODNET_SOCKS_*`).
- Not measured: the external shim of the issue.

## Approach

1. `PODNET_SOCKS_CIDRS` (CIDR or `ip:port`, comma list, fixed table). Keep the tailnet default.
2. `net socksify --cidr C` (repeatable) puts it in the printed environment.
3. `net shim --socks-route ADDR` with the front-door path, in `ShimOptions::environment`. Ship 1 and 3 together.
4. Log each route with `PODNET_SOCKS_LOG`.
5. Update `src/manual/facts.rs`; add the three-tier model to `docs/virtual-network.md`.

## Prove

```sh
cargo test --test shim_socks -- cidr_list_routes_only_listed
cargo test --test cli -- socksify_takes_cidrs
```

## Done

2026-11-09, commit 571d7b5. `PODNET_SOCKS_CIDRS` in `crates/podnet-vnet/shim/podnet-socks.c` (a fixed table of 32
routes: `tailnet`, `ADDR`, `ADDR/BITS`, `IPV4:PORT`, `[IPV6]:PORT`; the list replaces the tailnet default; each
route is logged with `PODNET_SOCKS_LOG`). `net socksify --cidr` (repeatable). `net shim --socks-route` and
`--socks-socket` give `PODNET_VNET_SOCKS` and `PODNET_VNET_SOCKS_ROUTES` (`ShimOptions::environment`); the vnet shim
sends a listed stream connect to the door. `src/manual/facts.rs` and `docs/virtual-network.md` section 5.5 (the
three tiers). `cargo test --test shim_socks -- cidr_list_routes_only_listed`: 1 passed (the IPv6 lines need an
AF_INET6 socket; this host has none, so they ran only in `vnet_shim_sends_a_socks_route_to_the_door`).
`cargo test --test cli -- socksify_takes_cidrs`: 1 passed. Tests added: `cidr_list_routes_only_listed`,
`vnet_shim_sends_a_socks_route_to_the_door`, `socksify_takes_cidrs`, `socks_routes_are_checked`.
`sh scripts/agent/check.sh`, `unshare -n -- cargo test --offline --workspace --no-fail-fast`,
`sh scripts/restricted-host.sh --no-fail-fast`: exit 0. Not measured: the external shim of the issue. CI: at the merge.

# T-168: A passwd shim for any client on a host with no `/etc/passwd` (GitHub #365)

**Source:** GitHub #365 (https://github.com/PodUtils/podnet/issues/365)
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

On a host with no `/etc/passwd`, `ssh` fails and `ssh-keygen` exits 0 and writes no key (`getpwuid`
fails). The preload `crates/podnet-vnet/shim/fakepwd.c` exists, but the only command is `net ts-shims`, named for
Tailscale and described only for Tailscale SSH.

## Premise

- `net ts-shims` already takes `--out --user --uid --gid --shell --json` and prints `LD_PRELOAD`
  (merged with an old value) and `SANDHOME_PASSWD` (`src/cli/cmd/net/shim.rs:142-181`,
  `crates/podnet-vnet/src/tailscale_ssh.rs:160-202`). It also writes `getent`, `id` and a compiled shim: more than a client needs.
- `docs/target-environment.md:116` names only the Tailscale shim for `getpwuid`. README.md:923 uses `ts-shims`.
  No doctor row names the silent `ssh-keygen`.
- `tools/ts-sshd.sh:118-128` uses the preload for `ssh-keygen`.
- Not measured: `ssh-keygen` on a host without passwd in this session.

## Approach

1. Add `net passwd-shim` as the name of the same code (`ts-shims` stays as an alias). No new build code.
2. Document the failure signature (silent exit 0 of `ssh-keygen`) in README and `docs/target-environment.md`.
3. `doctor`: one line when `/etc/passwd` has no entry for the uid.
4. Keep the name `SANDHOME_PASSWD` and the `LD_PRELOAD` merge.

## Prove

```sh
cargo test --test cli -- passwd_shim_prints_the_preload
sh -c 'podnet net passwd-shim --json | grep -q SANDHOME_PASSWD'
```

## Done

2026-11-09, commit 0c9d3ff. `podnet net passwd-shim` runs the code of `net ts-shims` (which stays): stdout has only
`export` lines (for eval) or one JSON object; `LD_PRELOAD` keeps an old value after the shim. `net doctor` has
the row `passwd entry for this uid` (`crates/podnet-vnet/src/doctor.rs` `passwd_probe`). README ("A user with no
passwd entry") and `docs/target-environment.md` give the silent exit 0 of `ssh-keygen`.
`cargo test --test cli -- passwd_shim_prints_the_preload`: 1 passed.
`sh -c 'podnet net passwd-shim --json | grep -q SANDHOME_PASSWD'` (the debug binary): exit 0. Tests added:
`passwd_shim_prints_the_preload`, `passwd_probe_names_the_shim`. `sh scripts/agent/check.sh`: exit 0.
Not measured: `ssh-keygen` on a host with no passwd entry. CI: at the merge.

# T-177: Router: `podnet route check`, a dry run of the rules (GitHub #375)

**Source:** GitHub #375 (https://github.com/PodUtils/podnet/issues/375)
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

T-098 has doors and rules with no way to ask "which member takes this flow" without traffic. mwan3 has such a simulator.

## Premise

- `src/warp/rules.rs:30-56` (`Action`: `reject` and `block` both parse), `Table::for_name`
  (:122 at `9c1fc4b`), `Table::for_ip` (:133 at `9c1fc4b`). The router rules do not exist (T-098).

## Approach

1. `route check --door D [--to HOST:PORT] [--proto tcp|udp]`: print the rule (position, pattern), the
   member or group, the seeded pick, the health row, the resolver. Exit 0 for an up member; exit 1 for
   `reject`, no member up, or a down member. No dial.
2. The check and the router share one function and one rules snapshot.
3. The output is the table of the test of the rule engine.

## Prove

```sh
cargo test --workspace -- route_check_agrees_with_the_router
```

## Start condition

T-097 and T-098 are done.

## Correction

2026-11-11: the dry run is a flag of `podnet route`, not a subcommand: `podnet route --config FILE --check
--to HOST:PORT [--door INDEX]`. `route` has no subcommands (T-100). The protocol (`--proto tcp|udp`) is not
read: the rules of T-098 match a host and a port only. The resolver is not printed: a name rule matches the
name, and the network resolves it. The health of a dry run is the state at the start (`up`): a dry run has
no probe of a running router.

## Done

2026-11-11, f56826c. `Router::plan` builds the router of the file with ways that only name each network (no
network up, no dial). `Router::up` and `Router::plan` share one build. `Router::decide(door, target)` gives
the rule, the network or the group, and the pick of the group with its health, from the rules snapshot of
the door (`RulesFile::rules`). The router dials with the same rules. `route --check --to` prints one row for
each door (door, listen, rule, way, member, health). It exits 0 when each door passes, 1 when a door
refuses the target or its group has no member up, and 64 for a bad `--to` or `--door`. Tests:
`route_check_agrees_with_the_router` (fake SOCKS5 networks record the dial; the test fails when `decide`
ignores the rules) and `route_check_picks_a_member_of_a_group`. Measured: the binary printed `reject` and
exited 1 for a domain of a `reject` rule, and exited 64 for `--door 5` of a file with 2 doors.

# T-180: Router: write-side control of the members (select, rotate, reconnect, interval) (GitHub #376)

**Source:** GitHub #376 (https://github.com/PodUtils/podnet/issues/376)
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

T-097 gives a read-only `route status`. An operator of an N-identity pool cannot pin a member, rotate
its identity, force a reconnect or change the probe interval without a restart, which drops each flow.

## Premise

- T-095 keeps one state directory for each identity. `warp rotate` and `--session-max-age` exist for one session.
- `STATS` is global (`src/warp/proxy.rs:118`). The loopback and credential gate is at
  `src/warp/proxy.rs:410-415`; the PIN gate is `crates/podnet-serve/src/gate.rs`.
- The WARP service limits new devices for each address (T-101 Premise; `docs/decisions.md`).

## Approach

1. A control listener with the same gate: `GET /route/status`, `POST /route/select`, `/route/rotate`,
   `/route/reconnect`, `PATCH /route/interval`. The JSON is the one of `route status --json`.
2. Rotate swaps make-before-break and reuses the state directory. Rate-limit and log each registration.

## Prove

```sh
cargo test --workspace -- route_control_rotate_reuses_the_state_dir route_control_needs_the_gate
```

## Start condition

T-095 and T-097 are done.

## Correction

2026-11-12: partial. The control side is done in the router; the proof with a real WARP identity waits for
the take of `pod-warp` (T-050).

* Approach 1: the control socket of T-097 (`control: unix:/PATH`, mode 0600) takes `POST /route/select`,
  `/route/rotate`, `/route/reconnect` and `PATCH /route/interval` (`src/router/control.rs`). The gate is the
  gate of the status socket: the mode of the socket, and a check of the user of the peer (the user of the
  router, or root); a peer that is not allowed gets 403 for a read and for each write. The router has no TCP
  control listener, so the loopback and credential gate of `src/warp/proxy.rs` and the PIN gate of
  `podnet-serve` do not apply. Each write answers with the status document (`route --status --json`).
  `route --control unix:/PATH` has the flags `--select GROUP=MEMBER|auto`, `--rotate MEMBER`, `--reconnect
  MEMBER` and `--interval SECONDS`. A pin is `Balance::pin`: a pinned member that is down refuses the flows.
* Approach 2: each network sits behind a `Swap` (`crates/podnet-core/src/network/swap.rs`), so a swap reaches
  each group, rule and door. A rotate and a reconnect bring up the new link, give it to the new flows, and
  end the old link after 30 s. `Ctx.state_dir` (the key `state_dir`, `DIR/NETWORK`) is the same for the first
  link and for each rotate. `Network::rotate` is the hook for an identity; its default is `up`. A rotate of a
  network keeps a gap of 60 s (429, exit 75) and each rotate and reconnect is logged on stderr.
* Left: a network of the registry has no identity, so a rotate there is a new link. WARP is not a network of
  the registry (`src/networks.rs`; `src/warp/` waits for `pod-warp`, T-050), so the rotate of a real WARP
  device with the state of T-095 is not proven. When `pod-warp` gives the WARP network, it implements
  `Network::rotate` over `Ctx.state_dir`; then add a test that two rotates register no new device on the edge
  stand-in of T-012.

2026-11-12, GitHub #476 (row E10-2) and #466 (row E0-6). The control socket of a link is a separate socket with one JSON object for each line (T-267, T-307). It follows the discipline of the router socket of this entry (mode 0600, a token, `SO_PEERCRED`). It does not share the HTTP paths.

## Done

2026-11-12, commit 31b74c0. Tests: `cargo test -p podnet --lib route_control` (3:
`route_control_rotate_reuses_the_state_dir`, `route_control_needs_the_gate`,
`route_control_select_and_interval`), `cargo test -p podnet-core --lib route_control_pin` (1),
`cargo test -p podnet-core --lib swap` (1), `cargo test -p podnet --test detach route_control_flags` (1: the
flags against a running router, exit 75 for a second rotate, 64 for an unknown member, 69 with no router).
With the state directory and the gate disabled, `route_control_rotate_reuses_the_state_dir` and
`route_control_needs_the_gate` fail; with the code, they pass. `sh scripts/agent/check.sh`: exit 0. CI: at the
merge. Not proven: the rotate of a real WARP device (see Left), and a flow that lasts past the 30 s drain of
the old link.

2026-11-13, done, the work of the handoff (the patch wip-warprouter), rebased on b1e2b80, commit 3855573. The WARP network implements `Network::rotate` over `Ctx.state_dir`:
it loads `state_dir/NETWORK/warp.json`, enrols a new MASQUE key for the stored device, saves the file and opens a session
with the new key. Tests: `route_control_rotate_of_a_warp_member_registers_no_new_device` (two rotates, one registration
on the edge stand-in), `warp_network_rotate_reuses_the_stored_device`. Not proven: a rotate against the real WARP edge.

# T-219: `podnet service install`: a hardened system unit (PodUtils/podcrates#35, A-N-11)

**Source:** PodUtils/podcrates#35 (cfb), ask A-N-11, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M6
**Priority:** P1
**Effort:** M
**Status:** blocked

## Problem

Ask A-N-11 (P1): "Install a connector as a system service: a hardened `systemd` system unit (own user, `EnvironmentFile` for `TUNNEL_TOKEN`, `LimitNOFILE` 70000, `NoNewPrivileges`, `ProtectSystem=strict`), a fixed state directory for `warp.json`, and no new WARP device at a restart."

## Premise

- Read: `crates/podnet-core/src/autostart.rs` writes a systemd user unit or a crontab line, with no root. It has no system unit and no own user.
- The library that renders and installs a unit is `pod-unit` (PodUtils/podcrates, T-102). It does not exist in a release. The unit text is code of that crate.
- Not read: how `warp.json` picks its directory (`src/warp/`, frozen for `pod-warp`).

## Approach

1. Take `pod-unit` with T-137 when it is released.
2. Add `podnet service install|remove|status NAME -- COMMAND`. It writes a system unit with: `User`, `EnvironmentFile` (mode 0600, the token), `LimitNOFILE=70000`, `NoNewPrivileges=yes`, `ProtectSystem=strict`, `StateDirectory`, `Restart=always`. `--print-unit` prints the text and writes nothing.
3. Set the state directory so that a restart reads the same `warp.json`. Test that a restart makes no new device.
4. Keep `podnet autostart` for hosts with no root.

## Prove

```sh
cargo test -p podnet --test cli -- service_print_unit_is_hardened service_install_keeps_no_secret_in_the_unit
cargo todo check
```

## Blocker

The session of `PodUtils/podcrates` (the crate `pod-unit`, T-102 there). Send a COORD issue (to podcrates) for the release. The entry is unblocked when podnet takes `pod-unit` (T-137).

# T-220: `relay node` as a system service, with no operator act (PodUtils/podcrates#35, A-N-12)

**Source:** PodUtils/podcrates#35 (cfb), ask A-N-12, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M6
**Priority:** P1
**Effort:** S
**Status:** blocked

## Problem

Ask A-N-12 (P1): "Operator SSH with no inbound port: `relay node` as a system service for `127.0.0.1:22`, renewal of the pair with no operator act, and resume past 64 MiB and 12 h (T-087)."

## Premise

- The service needs T-219 (the unit). The renewal is T-161 (a step with this ask). The resume is T-087 (a step with this ask).
- The relay client is code of `pod-relay` (T-049). A feature there is not developed in podnet.

## Approach

1. After T-219: `podnet service install NAME -- relay node --target 127.0.0.1:22` runs the node under the system unit.
2. Test the three parts together on a fake relay: the node outlives a pair (T-161) and a session past the cap (T-087).

## Prove

```sh
cargo test -p podnet --test relay_reverse -- relay_node_service_outlives_pair_and_cap
cargo todo check
```

## Blocker

T-219 (`pod-unit`), T-161 and T-087 (`pod-relay`, PodUtils/podcrates). The entry is unblocked when these three are done.

# T-221: `podnet bench URL --json` (PodUtils/podcrates#35, A-N-15)

**Source:** PodUtils/podcrates#35 (cfb), ask A-N-15, routed by PodUtils/podcrates#40.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** blocked

## Problem

Ask A-N-15 (P2): "A bench command, and a live step that runs it against podnet and `cloudflared` on the same runner."

## Premise

- Read: `podnet` has no `bench` command (`src/cli/mod.rs`). `tests/tcp_window.rs` measures a window, not a URL.
- The live step needs a Cloudflare test account and waits (`docs/decisions.md`, 2026-11-09, Test accounts).

## Approach

1. Add `podnet bench URL [--json] [--connections N] [--duration S]`. It sends requests through the chosen road and measures the latency (p50, p95, p99), the rate and the errors.
2. Print one stamped document with `schema` 1. Fix its keys with a test.
3. Test it against a local server of podnet (`serve`).
4. The live step in `live.yml` (podnet and `cloudflared` on one runner) starts when a test account exists.

## Prove

```sh
cargo test -p podnet --test commands -- bench_json_has_the_fixed_keys
cargo todo check
```

## Blocker

The owner, 2026-11-11 (`docs/decisions.md`, Deferred and blocked work): steps 1 to 3 are done (861ebc4); step 4, the
live step with `cloudflared` on one runner, needs a Cloudflare test account. A local server cannot stand
in for the edge.

## Start condition

For the live step only: a Cloudflare test account exists (`docs/decisions.md`, 2026-11-09, Test accounts). The local part can start now.

## Correction

2026-11-11, steps 1 to 3 are done in commit 861ebc4 (`src/cli/cmd/bench.rs`). `podnet bench URL` sends GET
requests on `--connections N` (default 4) for `--duration S` (default 10) through the way out of the process:
`--egress SPEC`, `--proxy` or `--no-proxy` (the registry of networks, no host in the code). `--timeout S` limits
one request. A status below 400 is a good request. Any other status, or a failed request, is an error. The
document has the keys `schema`, `podnet`, `url`, `connections`, `duration_s`, `requests`, `ok`, `errors`,
`first_error`, `rate` (good requests for each second), `bytes` and `latency_ms` (`min`, `p50`, `p95`, `p99`,
`max`, nearest rank, `null` with no good request). Exit 0 when one request was good, else 1.
`bench_json_has_the_fixed_keys` and `bench_counts_http_errors` (`tests/commands.rs`) test it against
`podnet serve --local`. Not done: step 4, the live step in `live.yml` with `cloudflared` on one runner. It waits
for the Cloudflare test account. The keys are not in `docs/protocol.md` (the entry does not ask for it).

# T-236: `serve --ingress relay --detach` and `git serve --ingress relay --detach` report ready before the node is connected

**Source:** the live run 42937309964 at c0180cc, step "Git over SSH through the reverse relay".
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`git serve --ingress relay --detach` returned, and the `git push` 0.2 s later got: "the relay refused the
session to p-<hidden>: HTTP 503: reverse: node offline". The command reports its ready document before
`run_node` connects the node of the pair to the relay. `serve --ingress relay --detach` has the same order
(its live step hides it with `curl --retry`).

## Premise

- Read: `src/cli/cmd/git_serve.rs` and `src/cli/cmd/serve_relay.rs` call `detach::ready` before
  `run_node`; `node_once` (`src/relay_reverse.rs`) calls `detach::ready` when the relay has the socket of
  the node, and `ready` sends only its first call.
- `relay node --detach` reports ready at the connection: its live step has no race.
- `src/relay_reverse.rs` has 891 lines (the size allowlist): it can only shrink.

## Approach

1. `detach::ready_when_reachable(doc)`: hold the ready document of the command; the next call of `ready`
   (the connection of the node) sends it.
2. `serve` and `git serve` hold their document on the relay road.

## Prove

```sh
cargo test -p podnet-core --lib a_held_ready_document_goes_with_the_next_ready
cargo test -p podnet --test relay_reverse
```

The live step "Git over SSH through the reverse relay" passes.

## Done

2026-11-11, commit e6eda36 (`detach::ready_when_reachable`; `serve` and `git serve` on the relay road hold their
ready document until the node is connected). Tests: `a_held_ready_document_goes_with_the_next_ready`,
`cargo test -p podnet --test relay_reverse`. The live step "Git over SSH through the reverse relay" passed in
the live runs 43012672866 (25f615f), 43016924769 (52cd473) and 43021850166 (10a586c), with "SSH through the
reverse relay" and "File server through the reverse relay".


# T-242: Rules: warn when an earlier domain or cidr rule shadows a later one, and fail when the shadowed rule is a `reject` (GitHub #433)

**Source:** GitHub #433 (https://github.com/PodUtils/podnet/issues/433); donor review, tailsplit.md block 6
(idy/tailsplit at 0bd3a74, GPL-3.0-or-later)
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** M
**Status:** partial

## Problem

Both rule engines of podnet (the `--rule` and `--rules` list of `podnet warp proxy`, and the rule list of
`podnet pod`) are first match wins. A broad `domain:` or `cidr:` rule before a narrow one makes the narrow
rule dead, with no message. When the dead rule is `reject`, the refusal that the operator wrote never
happens, and the flow goes through the action of the earlier rule. Example:

```text
domain:.example.com=direct
domain:admin.example.com=reject
```

`admin.example.com` matches the first line, so it goes direct. Check the table at load time and report each
shadowed rule with both positions. Make it an error for `reject`. The check that proves the fix: the same
file gives a start-up warning that names the second rule as shadowed by the first rule, and exit code 64 (the code of a bad
rule today) when the shadowed action is `reject` and the flag that allows it is absent.

## Premise

- Implement from the description, not from the donor source (the donor is GPL-3.0-or-later). The donor
  removes the order dependence by a sort. The issue does not propose that: first match stays.
- `src/warp/rules.rs:1-12` documents that the first rule that matches wins. `Table::build`
  (src/warp/rules.rs:100 at `9c1fc4b`) joins the flags and the file and compares nothing. `for_name`
  (src/warp/rules.rs:122-129 at `9c1fc4b`) takes `.find`.
- The pod rules: `select_chain` (`crates/podnet-netns/src/rules.rs:111-148`) is the same loop;
  `domain_matches` (`crates/podnet-netns/src/rules.rs:199-206`): `NAME` matches the name only, `.NAME`
  matches the name and each name under it.
- `warp proxy --check` is handled in `src/cli/cmd/warp/front.rs:25-41` (the flag `check: bool` is in the
  arguments of the command): it builds the table, prints each rule and exits 0, or 64 for a bad rule.
- By reading the loop, the example behaves as described. Not run: the timing of a file with 10 000 lines.
- T-177 (open, `podnet route check`) shows the match for one query and does not lint the table. T-098
  (partial) holds the router rules. T-156 and T-157 (done) are the pod `reject` action and the default
  action. No entry and no issue is about a shadowed rule.
- The WARP half (`src/warp/rules.rs`) is code that goes to `pod-warp` (T-050): a new feature there is not
  developed in podnet. It waits for the take, then a COORD issue (to podcrates) carries the ask. The pod
  half and the router rules stay in podnet.

## Approach

1. Add a pure function `shadowed(rules)` to `crates/podnet-netns/src/rules.rs`: for each pair `i < j` of
   rules of the same kind, report `(j, i)` when `covers(rule[i], rule[j])` and the actions differ.
   `covers` for domains: `.X` covers `X`, each name under `X`, and each `.Y` with `Y` equal to `X` or under
   `X`; `X` covers only `X`. For cidr: the prefix of `i` is not longer and the network bits are the same.
   A domain and a cidr rule never shadow each other. Expand `cidr:@FILE` to its prefixes first.
2. Print one line for each hit. It names the shadowed rule and the earlier rule that matches first. Use the
   number of the line of the file, and the position of the flag for the rules of the command line.
3. A shadowed rule with the same action is not reported (a harmless duplicate).
4. A shadowed `reject` is an error: exit 64, with the text that says to move the narrow rule above the
   broad one, unless `--allow-shadowed-rules` is given.
5. Run the lint in the load of the pod rules and of the router rules (T-098). Leave the WARP half for the
   take of `pod-warp`.
6. A table test in the shape of the donor: a list of (name, expected rule index) for each layout, run against
   the real lookup, so that the lint and the engine agree by construction.
7. Size: a pair loop costs time quadratic in the number of rules. Compare rules of the same kind only.
   Measure a generated file of 10 000 lines (time, machine, number of runs) before choosing the pair loop or
   a sort by prefix length or by label count.
8. Later and optional: `--rules-order specific-first`. First match stays the default.

## Decision

The exit code 64 and the flag `--allow-shadowed-rules` come from the issue. The check handles `domain:` and
`cidr:` only. It ignores the built-in refusals (loopback, link-local, `localhost`) and does not change the
order of evaluation.

## Prove

```sh
cargo test -p podnet-netns --lib -- shadowed_rule_is_reported shadowed_reject_is_an_error
cargo test -p podnet-netns --lib -- reversed_order_gives_no_report same_action_gives_no_report exact_rule_does_not_cover_a_subdomain
cargo test -p podnet-netns --lib -- rule_lint_agrees_with_the_lookup
```

The two-line file above gives the warning and exit 64. The reverse order gives no output.
`cidr:10.0.0.0/8=direct` before `cidr:10.1.0.0/16=reject` is reported and is an error.
`domain:example.com=direct` before `domain:www.example.com=reject` gives no output.

## Start condition

The pod half has none. The WARP half waits for the take of `pod-warp` (T-050).

## Correction

2026-11-11. The pod half and the router half are done. The WARP half is left: `warp proxy -r` and `--rules`
(`src/warp/rules.rs`, `Table::build`) get no lint, because that code goes to `pod-warp` (T-050). After the take,
call `podnet_netns::rules::shadow::check` from the table build, with the place `--rule N` or `FILE:LINE`, and write a COORD
issue (to podcrates) with the ask. The router file has no command flag yet: `RulesFile::open_with` takes
`allow_shadowed`, and the caller that builds the router from a command line (T-098) must pass the flag.

## Done

2026-11-11, commit e7ec95c. `crates/podnet-netns/src/rules/shadow.rs` holds the lint (`covers`, `shadowed`, `check`).
`podnet pod -r` warns for a shadowed rule and exits 64 for a shadowed `reject`; `--allow-shadowed-rules` makes it a
warning. The router file (`routes::parse_with`, `RulesFile::open_with`) does the same, with the line numbers of the file.
Tests: `shadowed_rule_is_reported`, `shadowed_reject_is_an_error`, `reversed_order_gives_no_report`,
`same_action_gives_no_report`, `exact_rule_does_not_cover_a_subdomain`, `rule_lint_agrees_with_the_lookup`,
`pod_args_shadowed_reject_is_an_error`, `router_shadowed_reject_is_an_error`. With `covers` forced to false, 3 of the
first 6 failed. Time of 10 000 rules (debug build, 4 cores, 1 run): 17 ms for domains, 92 ms for prefixes (an index by
label and by prefix, not a pair loop). Not proven: the WARP half; a prefix file of 1 000 000 lines; the
exit code 64 of the real binary (the test calls `parse_args`). `check.sh`: fmt, clippy, record, size, doc, layers 0; test:
the shim tests `shim_datagram_of_another_user_is_dropped` and `the_shim_takes_the_same_user_only` fail with
"Permission denied" (user switch in this sandbox; not run on old code), each other test passes.

2026-11-11, dc5d221: the router half has its switch. A door of the router file (T-100) takes
`allow_shadowed_rules: true`, as `pod --allow-shadowed-rules`: the file then loads with a shadowed `reject`,
with the warning. Test: `route_door_shadowed_reject_needs_the_key`. Left: the WARP half (after `pod-warp`).


# T-251: Transfer: the hosted magic-wormhole mailboxes on 443 as the default, with a choice that the code carries

**Source:** `docs/research/podnet/hosted-relays.md` (2026-11-11); the owner, 2026-11-11 (`docs/decisions.md`,
Relays and mailboxes).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** done

## Problem

`podnet send` and `receive` (T-105) need a mailbox that a proxy-only host reaches. The public mailbox of
magic-wormhole answers WSS on TCP 443 (`wss://relay.magic-wormhole.io/v1`, the default of its Python client
since 2026-05-13), and Winden runs a second one. podnet names neither.

## Premise

- Probed on 2026-11-11 from a proxy that allows only TCP 443: both mailboxes answer. Read: the operator of the
  first is one person ("free until volume or abuse makes it infeasible").
- podnet names no host of another party in its code: the hosts are facts of `pod-services` (PodUtils/podcrates).

## Approach

1. Read the mailboxes from `pod-services`: the magic-wormhole mailbox first, Winden second, podnet's own mailbox
   server (`relay serve --mailbox`) when the owner runs one.
2. The sender picks the mailbox; the code word carries a short id of it, so that the receiver uses the same one.
3. Read magic-wormhole issue #754 (different mailboxes for one nameplate) before the default ships.

## Prove

```sh
cargo test --workspace -- transfer_mailbox_choice_rides_in_the_code transfer_mailbox_falls_back
```

## Start condition

T-102 is done (the mailbox client of `pod-wormhole`).

## Correction

2026-11-13, Step 2 is replaced (the lead's agent; `docs/decisions.md`, 2026-11-13, The transfer layer): the code
carries no id of its mailbox, so that a code of magic-wormhole works as it is. The sender takes the first mailbox
that answers (`pod_wormhole::opened`); the receiver looks for the code at each mailbox in order
(`pod_wormhole::joined`: a mailbox that lists the nameplate has the giver, one that lists others has not, at one
that lists none the receiver waits 10 s for the giver). The Prove's `transfer_mailbox_choice_rides_in_the_code` is
`transfer_mailbox_choice_is_found_by_the_receiver`. Step 3: magic-wormhole/magic-wormhole#754 (read 2026-11-13;
open, no comment): two clients of the public mailbox can reach two instances that do not share their mailboxes;
then the receiver finds nobody (69). `docs/transfer.md` says so, and gives `--mailbox` as the way out. podnet's own
mailbox server waits for PodUtils/podcrates T-215.

## Done

2026-11-13, commit d5c38c1. `transfer::default_mailboxes` reads the services of the kind `mailbox` of
`pod-services` in their order (the public mailbox of magic-wormhole, then Least Authority's), and
`transfer::default_relays` the kind `transit`. `cargo test --workspace -- transfer_mailbox_choice_is_found_by_the_receiver
transfer_mailbox_falls_back`: both passed (a dead first mailbox; a sender at the second mailbox, found by the
receiver after the first, whose nameplate it released).

# T-252: Transfer: a piping-server relay as a transit carrier of last resort

**Source:** `docs/research/podnet/hosted-relays.md` (2026-11-11).
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** M
**Status:** open

## Problem

The public transit relay of magic-wormhole is on TCP 4001 only. Two proxy-only hosts have no direct path and
no transit relay that they both reach. A piping-server relay carries an HTTPS byte stream on 443 between two
peers (GET and PUT on one path), and it crossed the proxy of this sandbox end to end.

## Premise

- Probed on 2026-11-11: `ppng.io` and `piping.onrender.com` (piping-server 1.12.9) answer on 443. No terms are
  published for them. The server is MIT, so the owner can host one.
- The transit layer encrypts each record with the transit key: the relay reads no plain text.

## Approach

1. Add a transit carrier "piping": the path is derived from the transit key; the sender PUTs, the receiver
   GETs, each in chunks that a buffering proxy passes.
2. Use it only when no direct path and no transit relay works, and let the user turn it off.
3. Test against a local piping-server.

## Prove

```sh
cargo test --workspace -- transit_over_piping_server
```

## Start condition

T-103 is done (the transit of `pod-transit`).

## Correction

2026-11-13, Nothing of this entry is done. The transit is now in podnet (`src/transfer/connect.rs`, T-103): a carrier
"piping" is one more dial in `connect::connect` and one more kind of hint (`piping-v1`, the words of
PodUtils/podcrates T-214). The piping servers are services of the kind `piping` of `pod-services` (`ppng-piping`,
`onrender-piping`). What is left: the PUT and the GET in chunks through the way of podnet, the path of the transit
key, the switch to turn it off, and a local piping-server for the test.

# T-253: Live: probe the hosted mailboxes and relays from a proxy-only runner

**Source:** `docs/research/podnet/hosted-relays.md` (2026-11-11).
**Category:** measurement
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

The facts of the hosted services rest on one probe from one sandbox. A service can go away (one of them is
a personal server). Nothing watches them.

## Premise

- `live.yml` runs on a GitHub runner; a step can drop all egress but TCP 443 through a local CONNECT proxy.

## Approach

1. Add a live step: through a CONNECT proxy that allows only TCP 443, open the magic-wormhole and Winden
   mailboxes (a WSS hello), a piping-server stream, and an n0 iroh relay (HTTPS 200).
2. Print one line for each service; a failed service warns and does not fail the run (the services are not
   podnet's).

## Prove

```sh
python3 scripts/check-workflows.py
```

The step prints a line for each service in a run of `live`.

## Start condition

None.

## Done

2026-11-11, commit 614740c. The script `scripts/ci/probe-hosted.py` (python3 standard library only) opens each
mailbox (WebSocket upgrade, then the `welcome` message), a piping-server stream (PUT and GET of 32 random
bytes on a random path; the bytes are compared) and one n0 iroh relay (HTTPS 200 on `/`). It prints one line
for each service: name, ok or fail, ms, reason. A failed service prints `::warning::`; the exit code stays 0.
`live.yml` has the step "Hosted mailboxes and relays through a proxy for TCP 443 only". It starts
`scripts/connect-proxy.py` with a new fourth argument (the allowed ports, here 443). The proxy answers 403 to
any other port and logs it with the prefix `denied `; the step fails if one such line exists.
`python3 scripts/check-workflows.py`: 0 faults. Hand run in the sandbox (proxy of the sandbox):
wormhole-mailbox ok 430 ms; winden-mailbox ok 857 ms; piping-ppng ok 1330 ms; piping-onrender ok 986 ms
(13018 ms in an earlier run); iroh-relay (`use1-1`) ok 326 ms. `euw1-1.relay.n0.iroh.link` gave 502 here, so
the script uses `use1-1`. The local proxy with port 443 only gave 200 for port 443 and 403 for port 8443.
Not proven: the run of the step on a GitHub runner; the first run of `live` after the merge proves it.

# T-327: Router: add and remove a door and a network while the router runs (N-1)

**Source:** PodUtils/podcrates#125 (podswarm), ask N-1.
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

podswarm runs `podnet route` as a child, with one door for each policy and member set. It asks for calls on the
control socket that add and remove a door and a network while the router runs, with the bytes and the flows of each
door. Acceptance: a test adds a door, sends a flow through it, reads its counters and removes it, with no restart;
the flows of the other doors go on.

## Premise

- The control socket of `podnet route` (`control: unix:/PATH`) has `--select`, `--rotate`, `--reconnect`,
  `--interval`, `--status` and `--metrics` (`docs/router.md`, "Control"). It cannot add or remove a door or a network.
- The meter counts bytes and flows for each member (`crates/podnet-core/src/network/meter.rs`), not for each door.

## Approach

1. Add the verbs `add-door`, `remove-door`, `add-network` and `remove-network` to the control socket. A door takes the
   keys of a door of the router file. A network that a door or a group uses cannot be removed (exit 64).
2. Count the bytes and the flows of each door; `--status` and `--metrics` show them.
3. A removed door stops its listener; its open flows end at their own end.

## Decision

The lead (2026-11-13): an ask of a new product of PodUtils (podswarm) is P3, after podnet's own work. The ask has a
fallback in podswarm, and no milestone of podswarm waits for it. The owner can rule otherwise.

## Prove

```sh
cargo test --test router_control -- door_add_remove
```

## Done

2026-11-13, commit 361060d. The control socket of `podnet route` has the verbs `add-door`, `remove-door`,
`add-network` and `remove-network` (`src/router/change.rs`, `src/router/live.rs`), and `podnet route --control` has
the flags `--add-door`, `--remove-door`, `--add-network` and `--remove-network`. A door takes the keys of a door of the
file, as JSON or as YAML. A network of the file or an added network can be removed when no group, door or guest uses
it; else the answer is 409 and the exit code is 64. Each door counts flows, open flows and bytes
(`src/router/door_stats.rs`); `--status` and `--metrics` show them (`podnet_route_door_*`). A removed door stops its
listener; its open flows and the flows of the other doors go on. `cargo test --test router_control`: 3 passed
(`door_add_remove`, `network_add_remove`, `route_control_flags_add_and_remove`). With the verb `add-door` disabled,
`door_add_remove` fails (404). `sh scripts/agent/check.sh`: exit 0. Not proven: a door with a `dns` key added at run
time (the DNS bind error shows in the log, not in the answer); a flow through a `rules` door over an added network.

# T-328: Router: rules by port, also on the loopback (N-2)

**Source:** PodUtils/podcrates#125 (podswarm), ask N-2.
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** done

## Problem

podswarm asks for the rules `tcp:HOST:PORT=WAY` and `port:N=WAY`, also for `127.0.0.1`. Acceptance: with
`tcp:127.0.0.1:5432=home`, a connect to `127.0.0.1:5432` goes to the way `home`; a connect to another loopback port
does not.

## Premise

- A rule of a door is `domain:NAME=WAY`, `domain:.NAME=WAY`, `cidr:NET=WAY` or `cidr:@FILE=WAY`, or `reject`
  (`docs/router.md`, "Doors"). No rule matches a port. `cidr:127.0.0.1/32` matches each port of the loopback.

## Approach

1. Add `port:N=WAY` (a port or a range `N-M`) and `tcp:HOST:PORT=WAY` (`HOST` is a name or an address) to the
   rules reader of the router and of the pod. The shadow check covers them.
2. Test the acceptance case, and a rule of a port that a `cidr` rule shadows.

## Decision

The lead (2026-11-13): an ask of a new product of PodUtils (podswarm) is P3, after podnet's own work. The ask has a
fallback in podswarm, and no milestone of podswarm waits for it. The owner can rule otherwise.

## Prove

```sh
cargo test --workspace -- rules_port
```
## Correction

2026-11-13: podcrates takes the ask as its entry 475 (P3, PodUtils/podcrates#126): `RuleMatch::Port(lo, hi)` and
`RuleMatch::Tcp(host, port)` in the rules reader of `pod-warp`, with the acceptance test of podswarm. podnet changes
`shadow.rs` with the take of the tag that has it.

2026-11-13: the port rules are in `pod-warp` at podcrates `v0.38.0` (taken, c65090d): the block is gone. The doors and the
pods of podnet read the frozen `crates/podnet-netns/src/rules.rs`, not `pod-warp`'s reader: the work moves them onto
`pod_warp`'s `Table` and `RuleMatch` (the freeze of that file ends with it), and changes `rules/shadow.rs` for the kinds
`port:` and `tcp:`.

## Done

2026-11-13, commit 163ab15. The doors, the router and the pods read the matcher of `pod-warp`
(`pod_warp::rules::matcher`): `crates/podnet-netns/src/rules.rs` re-exports `RuleMatch`, `Net`, `TcpHost`,
`load_nets`, `domain_matches` and `in_cidr`, and keeps only the action, the rule and `select_chain`. This ends
the freeze of that file (the file of `pod-warp`): there is one reader. A CONNECT passes its port to
`RuleMatch::matches`; a datagram flow passes `None` (`select_chain_udp`, `Matcher::matches_datagram` of
`podnet-core`, used by the UDP flows of a router guest). `Rule.matcher` is an `Option` (`None`: the rule of
`--default-action reject`). `rules/shadow.rs` covers `port:` and `tcp:`: a `cidr:` or `domain:` rule covers a
`tcp:` rule of an address or a name it contains; a port range covers a port or a smaller range in it; `tcp:` covers
the same `tcp:`. The old rules files load unchanged (`domain:` and `cidr:` kinds are the same code as `pod-warp`).
Docs: `docs/router.md`, `docs/pods.md`, `crates/podnet-netns/README.md`, the help of `-r`, `llms-full.txt`.
Tests: `rules_port_router_door_sends_a_loopback_port_to_its_way` (`src/router/tests.rs`: `tcp:127.0.0.1:5432=home`
goes to `home`, `127.0.0.1:5433` does not), `rules_port_tcp_rule_picks_the_way_of_a_loopback_port` (door of a
pod), `rules_port_rules_match_the_port_of_the_target`, `rules_port_pod_rule_matches_the_port_of_the_target`,
`rules_port_shadow_check_covers_port_and_tcp_rules`. With the port forced to `None` the door and the
`select_chain` tests fail. Not proven: a `seccomp` pod with `--loopback-rules` end to end with a program; the
`netns` backend with a `tcp:` rule.

# T-329: `podnet exit --stdio`: a SOCKS5 exit on stdin and stdout (N-3)

**Source:** PodUtils/podcrates#125 (podswarm), ask N-3.
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

podswarm asks for a SOCKS5 server on stdin and stdout, with UDP ASSOCIATE, so a machine of a run can be the exit
over any road (ssh, `exec:`) with no listener. Acceptance: a test sends a UDP datagram through the exit on stdio and
reads the answer; a TCP connect through it works.

## Premise

- The SOCKS5 server of podnet is the front door (`crates/podnet-netns/src/frontdoor.rs`, `socks.rs`); UDP ASSOCIATE
  needs a UDP socket next to the TCP control connection, which a pipe does not have.
- T-291 (a SOCKS5 service at a peer) runs the front door on a stream of a link.

## Approach

1. One stdio stream carries one SOCKS5 session: CONNECT is the stream itself. For UDP ASSOCIATE, use the frames of
   `pod-proxy` UDP over a stream (a length before each datagram), as the relay roads do, and say so in the help.
2. Or reuse T-291: `podnet exit --stdio` is the `socks5` service of a link on the stdio carrier. Choose by what T-291
   builds; write the choice in a Correction.

## Decision

The lead (2026-11-13): an ask of a new product of PodUtils (podswarm) is P3, after podnet's own work. The ask has a
fallback in podswarm, and no milestone of podswarm waits for it. The owner can rule otherwise.

## Prove

```sh
cargo test --test exit_stdio
```

## Done

2026-11-13, commit 5df0bb2. `podnet exit --stdio` serves one SOCKS5 session on stdin and stdout with no listener
(`src/exit_stdio.rs`, `src/cli/cmd/exit.rs`). It reuses the door (`Door::with_udp_in_stream`,
`door::udp::in_stream`): CONNECT is the stream; UDP uses the command 0x83 of `pod-proxy` with a frame (2 bytes of
length, a SOCKS5 UDP datagram) on the same stream; UDP ASSOCIATE (3) gets 0x07. The frame is in
`docs/proxies.md`, "Exit on stdio". Choice of the Approach: step 1 (the frames of `pod-proxy`), because T-291 has no
UDP yet. UDP leaves from the host when the process has no proxy; with a proxy it gets 0x07. Tests:
`cargo test --test exit_stdio` (`exit_stdio_connects_tcp`, `exit_stdio_carries_udp_in_frames`,
`exit_stdio_refuses_udp_associate`, `exit_stdio_ends_with_stdin`), `exit_stdio_serves_connect_and_refuses_associate`,
`front_door_udp_in_stream_carries_frames`, `front_door_udp_in_stream_replies_for_a_door_that_refuses`. Without
`with_udp_in_stream` the UDP test fails (the reply is 0x07). `sh scripts/agent/check.sh`: exit 0. Not proven: UDP to
IPv6 (the socket is IPv4), a client of podswarm.

