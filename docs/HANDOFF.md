# Handoff

The state at 2026-11-17 21:50 (UTC). Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md` and
`docs/decisions.md`. Remove this file when the lead session has no unfinished step.

## The goal

The owner's review of the QA round (2026-11-17, `docs/decisions.md`, The QA round and the release freeze): no release
and no new feature until T-450 to T-455 are done; an issue closes only with a test that reproduces it and fails on the
code before the fix. The rulings "Shared code first" and "Quality first" (2026-11-17, in `CLAUDE.md`) hold for each
change.

## Where the code is

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (`rustup default 1.99.1`; add
  `rustfmt`, `clippy` and the target `x86_64-unknown-linux-musl` in a new container).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.69.0` (with `pod-cloudflare`). The installer core
  in `scripts/install/` is that of `v0.69.0`. The next tag of podcrates (`v0.70.0`) brings `pod_core::lock`,
  `pod_core::clock::Clock` and rows of `pod-hostprobe` (their entries 866, 877, 878).
* The newest release is `v0.4.0-beta.16` (T-422; its notes name the known problems).
* The record is `TODO/`. `cargo todo check` agrees: 477 entries, 18 open, 12 partial, 36 blocked, 411 done (at 21:50 UTC).

## Unfinished work (not on `main`)

None. The head is the commit of this handoff.

## CI

* A push to `main` starts `ci`, `deny`, `secrets`, `live` and `issue-sync`. `ci` has a concurrency group since
  2026-11-17: a new push cancels the `ci` of the commit before it. `live` keeps one running and one pending run: a new
  push replaces the pending one. Let a `ci` run end before the next push when its result matters.
* On `d147102`: `ci` 44177188362 and `live` 44177188471 passed. About one `live` run of six fails at the public relay
  (T-406: the relay keeps a lost socket of a node; COORD-098, #604, asks podcrates who owns the fix).
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
* T-451 step 1: `src/cli/help_tests.rs::each_flag_that_a_help_names_exists` (69 false flag references fixed). Step 2:
  `tests/contracts.rs` (the rules of the JSON documents, `docs/cli.md`; 20 faults fixed).
* T-477: a refusal for now (exit 75) gets its own hint. T-449: each grep of the workflows and the scripts reads the log
  lines of `pod-cloudflare`. T-414: the WARP proxy step outlived its `--run-for`.

## Next steps

1. T-478 (most likely T-406): when `live` fails in "SSH through the reverse relay", read the logs of the two sessions
   and of the node that the step prints now. Read the answer of podcrates on COORD-098 (#604).
2. T-450 to T-455, the gate of correctness, in this order: T-451 step 3 (the tests on the release binary), T-452 (error
   rows of the lane), T-453 (the lab of the testers' host), T-450, T-454, T-455.
3. T-473, what remains: the doctor row of the SSH relay and `--connect-proxy` go to `pod-proxy` (a SOCKS or
   authenticated proxy then works: a change of tested behaviour); the copies of the clock, the files and the backoff go
   to `pod-core`; the rest waits for podcrates' entries 896 to 901.
4. T-476: the cap test of the relay transfer fails under load; measure the bytes of the old leg during a move.
5. At each tag of podcrates: `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is at the tag (T-137).
6. The disk allowance holds about one debug build: build with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
   CARGO_PROFILE_TEST_DEBUG=0`. After a change of a document or a help text, run
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
| #604 (COORD-098, the relay holds a lost socket) | podssh answers ask 1 (T-406); ask 2 is in v0.65.0 (T-410). |
| #592 (COORD-095) | T-358 (the release gate; step 4 is done); T-380 and T-381 are done. |
| #587, #589 | podcrates closes #185 and #188. |
| #413, #425, #437, #439, #440, #448, #460 | Older proposals and asks; see `TODO/issues.md`. |

## Waiting on the owner

None. The owner ruled on 2026-11-16: the entry 305 is removed (#475 closed, T-396), `--seal` stays opt-in (Q1), and each
triaged issue of Nemo-010 gets the label `confirmed` (not the deferred #484 to #501).
