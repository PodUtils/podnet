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
**Status:** open

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

# T-083: A relay node serves a handler in the process, with no socket file

**Source:** `docs/research/podnet/capabilities.md`, section 6 (enabler 2).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** open

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

# T-084: One HTTP authentication layer for the local listener, the tunnel and the relay

**Source:** `docs/research/podnet/capabilities.md`, sections 3.2 and 6 (enabler 3).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

HTTP Basic authentication lives only in the dispatch of the tunnel, and `serve --local` refuses
it. The PIN gate works only for `share`. A service on a local listener or a relay node has no
authentication.

## Premise

- Read: the Basic check is in `src/proxy.rs`, and its credentials in `PodUtils/podnet:src/proxy_auth.rs`
  (`Credentials`).
- Read: `serve --local` refuses `--auth-user` (`src/main.rs`).
- The PIN gate is `src/gate.rs`.

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

# T-085: A ustar writer and reader, and a directory as one tar download of the file server

**Source:** `docs/research/podnet/capabilities.md`, sections 3.1, 3.4 and 6 (enabler 6).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

podnet has no tar writer or reader. The file server cannot give a directory as one download.

## Premise

- No crate of the lock gives tar today, and the design needs none (about 250 lines).
- `src/service.rs` holds the static router and its guard.

## Approach

1. Write a ustar writer and reader in `podnet-serve`: regular files and directories, long names
   with the ustar prefix, no link out of the root.
2. With `--tar`, the file server gives a directory as one tar stream.

## Prove

```sh
cargo test -p podnet-serve -- ustar_round_trip_keeps_odd_names a_directory_downloads_as_a_tar
```

The second test compares `tar -tf` of the download with the tree.

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
**Status:** open

## Problem

`serve` gives 404 for a directory with no `index.html`. A user cannot see the files of a
directory.

## Premise

- Read: the static router and its guard are in `src/service.rs`. The guard refuses a dot segment
  and a path out of the root.
- `src/gate.rs` has `escape_html`.

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

# T-089: File server: users with read and write roles, and a PIN, on each listener

**Source:** `docs/research/podnet/capabilities.md`, sections 3.1 to 3.3.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-090: File server: uploads into the served tree with PUT and a multipart form

**Source:** `docs/research/podnet/capabilities.md`, sections 3.1 to 3.5.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

A user cannot upload into the served tree. `share --mode upload` writes to its own directory with
random names.

## Premise

- Read: `upload_receive` streams a multipart body with limits and files of mode 0600
  (`src/share.rs`).
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

# T-091: `podnet serve DIR --via`, a local `share`, a directory in `share`, and the live step of the file server

**Source:** `docs/research/podnet/capabilities.md`, sections 3.2, 3.3 and 3.5.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-092: Git: a CGI runner for `git http-backend`, and the smart HTTP router

**Source:** `docs/research/podnet/capabilities.md`, sections 5.2, 5.3 (design A) and 5.8; the
owner's ruling of 2026-11-05 (`docs/decisions.md`, Git hosting).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** L
**Status:** open

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

# T-093: Git: an SSH server for upload-pack and receive-pack

**Source:** `docs/research/podnet/capabilities.md`, sections 1.5, 5.4 and 5.8;
`docs/research/podnet/podssh-copy.md`, sections 2.7 and 3 (C22); the owner's ruling of 2026-11-05
(`docs/decisions.md`, Git hosting).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-094: `podnet git serve`: the repository root, `git init`, `git urls`, the tunnel, the relay and the live step

**Source:** `docs/research/podnet/capabilities.md`, sections 5.5 to 5.8.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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
**Status:** open

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

# T-097: Router: a TCP health check for each member, and `podnet route status`

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 and 2.5;
`docs/research/podnet/mwan-field.md`, sections 1.4 (D5, D6, D10), 1.6 and 2.2 (R2, R5); GitHub #373 and #374.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

A group needs the state of each member. An ICMP check lies for a WARP session that carries no
TCP (T-068).

## Premise

- The probe of T-068 renews a WARP session that carries no TCP.
- `src/metrics.rs` and `src/decisions.rs` give counters and decision lines.
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

# T-098: Router: doors with a default network, and rules to a network, a group or a refusal

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 to 2.3; GitHub #372 and #374.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-099: Router: `podnet forward` for each network, UDP forwards, a DNS door, and `podnet expose`

**Source:** `docs/research/podnet/capabilities.md`, sections 2.1 to 2.3;
`docs/research/podnet/mwan-field.md`, section 3 (F4b); GitHub #372.
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-100: Router: a config file, and `podnet route` with `--detach` and autostart

**Source:** `docs/research/podnet/capabilities.md`, section 2.3;
`docs/research/podnet/mwan-field.md`, section 2.2 (R3).
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The parts of T-095 to T-099 have no command. A router with several networks, groups and doors is
too long for a command line.

## Premise

- `src/yaml.rs` reads YAML with no new crate.
- `src/detach.rs` and `src/autostart.rs` supervise a command and start it at boot.

## Approach

1. Add `podnet route [--config FILE]`, with the keys of the flags in the file.
2. Read the file with `src/yaml.rs`.
3. Make `--detach --name NAME` and `autostart add` work with it.

## Prove

```sh
cargo test --workspace -- route_config_errors_name_the_key
cargo test --test detach -- route_detaches_and_lists_its_members
```

## Start condition

T-097 and T-098 are done.

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
3. Check the SHA-256 of each file. Map each fault to an exit code of `src/exit.rs`.

## Prove

```sh
cargo test --test cli -- send_and_receive_on_the_loopback a_wrong_code_ends_the_code
```

## Start condition

T-103 is done.

## Correction

2026-11-05, The command stays in podnet. It uses `pod-wormhole` and `pod-transit` (T-102, T-103).

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
**Status:** open

## Problem

The socks shim routes a stream `connect` to the front door only for `100.64.0.0/10` and
`fd7a:115c:a1e0::/48`, or for each connect with `PODNET_SOCKS_ALL=1` (`shim/podnet-socks.c:206-223`).
A client with a hardcoded origin (`127.0.0.1:3000`, a fixed LAN address) needs only that address routed.
The abstract-name shim has no option to hand a chosen address to the SOCKS door.

## Premise

- `parse_config` reads `PODNET_SOCKS_PROXY`, `_LOG`, `_ALL`, `_MAX_FDS` (`shim/podnet-socks.c:81-92`).
- `net socksify` prints a fixed environment (`src/cli/cmd/net/shim.rs:104-110`). `ShimOptions`
  (`src/vnet/shim.rs:30-45`) has no hand-off list. `podnet-vnet.c:676` translates each IP of a virtual socket.
- The variable list is in `src/manual/facts.rs:296`. `docs/virtual-network.md` has no text on socksify
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

# T-168: A passwd shim for any client on a host with no `/etc/passwd` (GitHub #365)

**Source:** GitHub #365 (https://github.com/PodUtils/podnet/issues/365)
**Category:** feature
**Milestone:** M6
**Priority:** P2
**Effort:** S
**Status:** open

## Problem

On a host with no `/etc/passwd`, `ssh` fails and `ssh-keygen` exits 0 and writes no key (`getpwuid`
fails). The preload `shim/fakepwd.c` exists, but the only command is `net ts-shims`, named for
Tailscale and described only for Tailscale SSH.

## Premise

- `net ts-shims` already takes `--out --user --uid --gid --shell --json` and prints `LD_PRELOAD`
  (merged with an old value) and `SANDHOME_PASSWD` (`src/cli/cmd/net/shim.rs:118-157`,
  `src/vnet/tailscale_ssh.rs:160-202`). It also writes `getent`, `id` and a compiled shim: more than a client needs.
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

# T-177: Router: `podnet route check`, a dry run of the rules (GitHub #375)

**Source:** GitHub #375 (https://github.com/PodUtils/podnet/issues/375)
**Category:** feature
**Milestone:** M6
**Priority:** P3
**Effort:** S
**Status:** open

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
- `STATS` is global (`src/warp/proxy.rs:120`). The loopback and credential gate is at
  `src/warp/proxy.rs:412-417`; the PIN gate is `src/gate.rs`.
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

- Read: `src/autostart.rs` writes a systemd user unit or a crontab line, with no root. It has no system unit and no own user.
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
