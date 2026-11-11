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
**Status:** blocked

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

## Blocker

The session of `PodUtils/podcrates` (the crate `pod-relay`). The relay node (`Target`, `node`, `session`) is in
`src/relay_reverse.rs` and `src/relay_reverse/session.rs`. These files go to `pod-relay` and take fixes only
(`docs/decisions.md`, 2026-11-06, Order). A handler target is a new variant of `Target`: a feature of that code.
No part of the entry lands in the crates of podnet: the users of the handler (T-091, T-094, T-104) do not exist.
The entry is unblocked when podnet takes `pod-relay` (T-049, through T-137). Then add the target to `pod-relay`
or use its handler target.

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
**Status:** open

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

# T-087: `relay node` and `relay connect` resume a session past the limits of the relay

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.3, 3 (C5) and 5 (G2); PodUtils/podcrates#35 (cfb), ask A-N-12.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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
**Status:** open

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
**Status:** partial

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
   of `tcp_dead` (`src/warp/live.rs:279` and `:387`).
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

# T-098: Router: doors with a default network, and rules to a network, a group or a refusal

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 to 2.3; GitHub #372 and #374.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** partial

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

# T-099: Router: `podnet forward` for each network, UDP forwards, a DNS door, and `podnet expose`

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 to 2.3;
`docs/research/podnet/mwan-field.md`, section 3 (F4b); GitHub #372.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** partial

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
(`src/warp/forward.rs` is frozen); the UDP forwards, the pump of item 4 and the DNS door need
`Dial::datagrams` of each network; `podnet expose` (item 3).

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
**Status:** open

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

# T-102: Take the mailbox client, the code words and the SPAKE2 phases from `pod-wormhole`

**Source:** `docs/research/podnet/podssh-copy.md`, sections 2.2 and 3 (C8);
`docs/research/podnet/capabilities.md`, sections 4.2 to 4.4; the owner's rulings of 2026-11-05
(`docs/decisions.md`, File transfer and Crypto crates).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-103: The file-transfer protocol and the transit protocol of magic-wormhole

**Source:** `docs/research/podnet/capabilities.md`, section 4.3;
`docs/research/podnet/podssh-copy.md`, sections 2.2 (the wire choice) and 3 (C9); the owner's
ruling of 2026-11-05 (`docs/decisions.md`, File transfer).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** L
**Status:** open

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

## Start condition

T-102 is done.

## Correction

2026-11-05, The file-transfer protocol and the transit protocol are work of `pod-wormhole` and `pod-transit` in
PodUtils/podcrates. podnet takes the crates (T-137, step 3). The inflate decoder (step 2) is a call made
without a ruling: it belongs to the crate that reads the zip.

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

# T-105: `podnet send` and `podnet receive`

**Source:** `docs/research/podnet/capabilities.md`, sections 4.2, 4.5 and 4.7; the owner's ruling
of 2026-11-05 (`docs/decisions.md`, File transfer).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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
- The variable list is in `src/manual/facts.rs:311`. `docs/virtual-network.md` has no text on socksify
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
- `docs/target-environment.md:104` names only the Tailscale shim for `getpwuid`. README.md:923 uses `ts-shims`.
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
  (`:122`), `Table::for_ip` (`:133`). The router rules do not exist (T-098).

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
**Status:** open

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
**Status:** open

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

## Start condition

For the live step only: a Cloudflare test account exists (`docs/decisions.md`, 2026-11-09, Test accounts). The local part can start now.

# T-236: `serve --ingress relay --detach` and `git serve --ingress relay --detach` report ready before the node is connected

**Source:** the live run 42937309964 at c0180cc, step "Git over SSH through the reverse relay".
**Category:** defect
**Milestone:** M5
**Priority:** P2
**Effort:** S
**Status:** open

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
  (`src/warp/rules.rs:100`) joins the flags and the file and compares nothing. `for_name`
  (`src/warp/rules.rs:122-129`) takes `.find`.
- The pod rules: `select_chain` (`crates/podnet-netns/src/rules.rs:242-262`) is the same loop;
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
