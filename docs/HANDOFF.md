# Handoff

The state at 2026-11-18 13:30 (UTC), in the session of 2026-11-18. Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md` and
`docs/decisions.md`. Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`; PodUtils/podcrates#225 COORD-134, #227 COORD-136):
the CI work of githome and cfb is podrunner's work, and it comes first. podcrates is the coordinator; COORD issues are
the one channel. podnet's order: a red run of its CI; each ask of podrunner (a COORD issue `(to podnet)`); the take of
each tag of podcrates; the asks of githome and cfb (none owned by podnet for their CI; #461 asks wait on podcrates'
`pod-unit`); then the work below. podnet's start report and take are in COORD-107 (#654). A beta with a fix for
podrunner alone is allowed under the freeze (COORD-136, call C782).

The owner's review of the QA round (2026-11-17, `docs/decisions.md`, The QA round and the release freeze): no release
and no new feature until T-450 to T-455 are done; an issue closes only with a test that reproduces it and fails on the
code before the fix. The rulings "Shared code first" and "Quality first" (2026-11-17, in `CLAUDE.md`) hold for each
change.

## Where the code is

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (`rustup default 1.99.1`; add
  `rustfmt`, `clippy` and the target `x86_64-unknown-linux-musl` in a new container).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.74.0` (T-489). The installer core in
  `scripts/install/` is that of `v0.70.0` and `v0.71.0` (the same file; T-469, GitHub #645, done). The replacement of podnet's copies by the new
  shared items of `v0.70.0` (`pod_proxy::hop`, `pod_dns::txt::error_answer`, `pod_ws::handshake::build_probe_request`,
  `pod_tls::env_bundle`, `pod_core::lock`, `pod_core::clock`) is T-473; it waits behind podrunner's asks.
* The newest release is `v0.4.0-beta.17` (T-481, tag at `cfc0924`): the fixes of PodUtils/podrunner#213 and #142 and
  the QA round, under the coordinator's call C782. COORD-108 (#655) asks podrunner to pin it and run #142 again.
* The record is `TODO/`. `cargo todo check` agrees: 488 entries, 20 open, 11 partial, 35 blocked, 422 done.

## Unfinished work (not on `main`)

None. The head is the commit of this handoff.

## CI

* A push to `main` starts `ci`, `deny`, `secrets`, `live` and `issue-sync`. `ci` has a concurrency group since
  2026-11-17: a new push cancels the `ci` of the commit before it. `live` keeps one running and one pending run: a new
  push replaces the pending one. Let a `ci` run end before the next push when its result matters.
* On `fb2184c` (the head before this handoff): `live` 44208545007 passed. Its `ci` was replaced by the `ci` of this handoff commit (the same code and this file): read that result first (T-479 step 1 is the change that it tests).
* `gate` 44198354501 on `b2160d4` passed: the lane, its plant, and the contracts on the release binary.
* About one `live` run of six fails at the public relay (T-406: the relay keeps a lost socket of a node; COORD-098, #604,
  asks podcrates who owns the fix and when).
* `record-issues` (each day): green when each issue number has a line in `TODO/github.txt`.

## The work of 2026-11-17 (all on `main`)

* The QA round: GitHub #609 to #648 (40 issues of outside testers). Each is closed through the record except #645 (T-469,
  the installer of podcrates: COORD-103, #650). podrunner#213 (T-456) and #142 (T-457) are fixed and proven with a
  .NET 8 `HttpClient`.
* COORD-102 (#649) is to all: the lesson of the QA round; podrunner, podssh and podvm answered; podcrates answered asks
  1 to 3 in COORD-131 (PodUtils/podcrates#221).
* T-473 (the survey of copies of shared code): `src/transport/` is removed, `scripts/retry.sh` is podcrates' copy;
  T-474 (`ssh-relay` takes `--proxy` and `--egress`) and T-475 (the CA bundles of `warp up`) are done; the asks are
  podcrates' entries 896 to 901 (COORD-105, #652).
* T-451 is done: step 1, `src/cli/help_tests.rs::each_flag_that_a_help_names_exists` (69 false flag references
  fixed); step 2, `tests/contracts.rs` (the rules of the JSON documents, `docs/cli.md`; 20 faults fixed); step 3,
  `gate.yml` runs the contracts on the release binary (`PODNET_TEST_BINARY`).
* T-479: the relay tests keep a refused port bound (a freed port was taken by another test); the helper for each
  project is asked of `pod-testkit` (COORD-106, #653).
* T-478: the live step "SSH through the reverse relay" gives each parallel session a limit and a log; the cause is most
  likely T-406.
* T-477: a refusal for now (exit 75) gets its own hint. T-449: each grep of the workflows and the scripts reads the log
  lines of `pod-cloudflare`. T-414: the WARP proxy step outlived its `--run-for`.

## Next steps

1. Read the runs of "CI" above. Fix each failure first.
1b. Keep the watcher running. Take each COORD issue `(to podnet)` of podrunner first (its census, PodUtils/podrunner@T-490 and PodUtils/podrunner@T-491).
2. T-450 to T-455, the gate of correctness (no release and no new feature before they are done), in this order:
   T-452 (error rows of the lane: a dead upstream, a refused connect, a reject rule, an unreachable target, a name with
   no answer), T-453 (a lab of the testers' host class: no AF_INET bind, no netns, no write of the memory of a
   program; ask 1 of COORD-102 to podcrates), T-450, T-454 (a black-box QA pass), T-455 (the measure).
3. T-478 / T-406: when `live` fails in "SSH through the reverse relay" or in the resume step, read the logs that the
   step prints; read the answer of podcrates on COORD-098 (#604).
4. T-473, what remains: the doctor row of the SSH relay and `--connect-proxy` go to `pod-proxy` (a SOCKS or
   authenticated proxy then works: a change of tested behaviour); the copies of the clock, the files and the backoff go
   to `pod-core`; the rest waits for podcrates' entries 896 to 901. 
5. T-476: the cap test of the relay transfer fails under load; measure the bytes of the old leg during a move.
5b. T-488 (P3): sporadic HTTP 530 of a quick tunnel with two edge connections (the step "Throughput through WARP" of
   `live`); measure with `--ha-connections 1`. T-482 (P3): the netns backend prints a second, false cause after a refused namespace. T-414: a third form (the WARP
   proxy ends on a failed health check); its evidence goes to podcrates.
6. At each tag of podcrates (`v0.75.0` is next): `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is
   at the tag (T-137).
7. The disk allowance holds about one debug build: build with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
   CARGO_PROFILE_TEST_DEBUG=0`. Give an agent, and a control build of an old commit, its own `CARGO_TARGET_DIR`: a
   build of other sources in the shared `target/` leaves old files that cargo takes as current (then `cargo clean -p`
   each workspace package). After a change of a document or a help text, run
   `cargo test --bin podnet write_the_llms_files -- --ignored`. Issues #484 to #501 are deferred by the owner: leave
   them open and unlabelled.

Run the lane here: `sh scripts/get-podlab.sh v0.68.0 .work/podlab-v0.68.0`; build podnet; copy the binary and the
podlab asset to a directory that uid 65534 can read (for example `/var/tmp/lane/`, not in `/tmp` or HOME); then as
root: `setpriv --reuid=65534 --regid=65534 --clear-groups env -i PATH=/usr/bin:/bin HOME=OTHERDIR LANE_DIR=DIR/out sh
scripts/lab-lane.sh PODLAB DIR/podnet [PROFILE...]`.

## Known flakes under load

`quick_tunnel_standin_serves_a_visitor_over_http2`, `service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection` and `ws_relay::...::a_demoted_relay_host_recovers_on_success`
`autostart::sigterm_is_a_clean_stop`, `doctor_ssh_relay_row_dials_through_the_proxy`, `transfer_over_the_relay_crosses_the_cap` (T-476) and
`guest_wire_unix_socket_serves_a_guest_and_the_next` (keep the default `TMPDIR`) fail at times in a full run and pass alone. Run a failed test alone three times before a push.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #522, #526, #532 (takes of v0.37.0, v0.39.0 and v0.40.0) | podcrates records the take. |
| #560 (COORD-081, the take of v0.45.0) | The answer of podcrates on the ws listener (a refused pin: `Refused` or no peer). |
| #555 (COORD-079) | The fuzz crash of `pod-proxy` is fixed in v0.45.0 (T-349); the crash of `pod-cloudflare` waits for its take (their entry 029). |
| #597 (COORD-096, the takes of v0.58.0 to v0.66.0) | podcrates reads it; close it when podcrates closes #200 to #216. |
| #607 (COORD-100, `remote-port` = 80 for localhost-run) | podcrates adds the fact; podnet takes the next tag. |
| #604 (COORD-098, the relay holds a lost socket) | The owner of ask 1 (the relay server) and a date (T-406, T-478); ask 2 is in v0.65.0 (T-410). |
| #649 (COORD-102, the QA round, to all) | podcrates' asks 1 to 3 (their entries 866, 899); close it when T-450 to T-455 are done. |
| #656 (COORD-109, the image on ghcr.io: T-487) | The coordinator confirms the form (2) for all projects; the next release proves the attestation and the digests in the notes. |
| #654 (COORD-107, the start report, the takes of v0.70.0 and v0.71.0, the answer to PodUtils/podcrates@T-931) | The asks of podrunner; close it when the coordinator reads it. |
| #651 (COORD-104, the answer to COORD-131) | podcrates' entries 877, 879, 880 (the lock, `pod-detach`, `pod-release`); T-473. |
| #592 (COORD-095) | T-358 (the release gate; step 4 is done); T-380 and T-381 are done. |
| #587, #589 | podcrates closes #185 and #188. |
| #413, #425, #437, #439, #440, #448, #460 | Older proposals and asks; see `TODO/issues.md`. |

## Waiting on the owner

None. The owner ruled on 2026-11-16: the entry 305 is removed (#475 closed, T-396), `--seal` stays opt-in (Q1), and each
triaged issue of Nemo-010 gets the label `confirmed` (not the deferred #484 to #501).
