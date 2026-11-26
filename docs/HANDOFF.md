# Handoff

The product work state at 2026-11-26 02:00 (UTC). Start a hosted checkout with
[TODO/RESUME.md](../TODO/RESUME.md). Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`). The order: a red run of podnet's CI; each ask of
podrunner (a COORD issue `(to podnet)`); the take of each tag of podcrates (T-137); podnet's own work in the order of
`TODO/PROGRESS.md`. No new ask of podrunner and no new tag of podcrates (the newest is `v0.85.0`, taken in T-588) at 16:30.

## Where the code is

* All finished work is on `main`. Use the pinned Rust toolchain from [Hosted sessions](hosted-sessions.md).
* Done on 2026-11-26: T-428 (the feature `release`; #620 closed), T-053 and T-054, T-398 (the lane case `local-files`:
  15 commands get a scenario), T-163 (#360 closed), T-321's TCP half (docs). T-114 and T-244 blocked on COORD-130 (#760)
  and COORD-128 (#758).
* The release `v0.4.0-beta.19` is out (T-585; tag at `2ef8e3a`; COORD-129 #759). T-487 is proven; #656 is closed.
  T-163 is done (by the take of `pod-cloudflare`); T-244 is blocked on COORD-128 (#758).
* Done later on 2026-11-25: T-588 (podcrates `v0.85.0`, COORD-127 #757), T-587 and T-589 (`live` robustness),
  T-492, T-543 (done), T-584; T-476 blocked on COORD-126 (#756); T-586 recorded.
* Done in the session of 2026-11-25 (afternoon): T-583 (the take of podcrates `v0.84.0`, `405d47c`; COORD-122 #752 read
  by podcrates), T-546 and T-550 (by the tag), the reviewed fixes of the QA pass of `0.4.0-beta.19` (T-547, T-572 to
  T-574, T-576 to T-578, T-581), T-564, T-582, T-584 (#720), and T-543 steps 1 and 2 (the row `nat` of `doctor`).
* Blocked on podcrates: T-575 (COORD-123 #753, `pod-dns`), T-580 (COORD-124 #754, `pod-proxy`), T-579 (COORD-125 #755,
  `pod-cloudflare`). The release notes of beta.19 name them (`docs/decisions.md`, 2026-11-25).

## The next steps, in order

1. **The proof of T-589 and T-587** (both in `scripts/live-step.sh`): the next `live` run that meets the quota of the
   quick-tunnel service (HTTP 429) or WARP's door with a certificate of Let's Encrypt gives a warning and stays green.
   The autostart step prints the journal of a unit that stopped (`live` 45733368792: exit 75 with no cause). If the
   journal shows a cause that is not a quota, it is a defect of `autostart run` or of the tunnel: record it.
2. **Each tag of podcrates:** COORD-123 to COORD-126 (#753 to #756), COORD-128 (#758) and COORD-130 (#760) wait for
   one. Take it first (T-137), then close each answered ask and unblock T-575, T-579, T-580, T-476, T-244, T-114.
3. **The next beta (`v0.4.0-beta.20`):** the black-box QA pass first (`scripts/agent/qa-pass.md`); its notes list the
   carriers of the feature `release` (T-428).
4. The rest of the work order is open research or waits for a run or a ruling: T-281 (a proof of the DNS carrier needs
   hooks of `pod-link`), T-321 (a doctor row of TURN: `route --check` makes no connection now), T-358 step 3, T-414 and
   T-415 (a warning of their `live` steps), T-478 (the logs of its next failure), T-488 (runs that the quota limits),
   T-123 to T-129 (a ruling of the owner before the work).
5. **The daily run of `issue-sync`:** no run of the event `schedule` came on 2026-11-24 and 2026-11-25; podrunner's
   come 11 to 12 h late. Check again.

## Facts for the next session

* `check.sh` must pass before the commit: a chained `git commit` after it does not stop on a fault.
* Do not run the test suite under `nohup`: SIGHUP is then ignored for each child, and the SIGHUP cases of
  `tests/pod_signals.rs` and `tests/signals.rs` fail. Use `setsid` or a background shell.
* `cargo test` stops at the first failed test binary: use `--no-fail-fast` for a full run (about 60 minutes here).
* An issue closes through the record (`TODO/issues.md`, `TODO/github.txt`); `issue-sync` closes it on GitHub at the push.
* The forge mirror answers 403 at times for Actions lists and job logs: use `mcp__github__actions_list` and
  `mcp__github__get_job_logs` (`return_content: false` gives a URL to curl).
* The labs: `sh scripts/get-podlab.sh v0.84.0 DIR`; run as uid 65534 with
  `setpriv --reuid=65534 --regid=65534 --clear-groups env -i PATH=/usr/bin:/bin HOME=DIR PODLAB run PROFILE -- CMD`,
  with the binary in a directory that uid 65534 can read (`/var/tmp/qa`).
* The direct TCP of this container goes through a TLS middlebox and an HTTPS proxy; UDP 7844 is refused.

## How to work here

* Build with `TMPDIR=/tmp CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
* After a change of a document or help text, run `cargo test --bin podnet write_the_llms_files -- --ignored`.
* The local gate: `TMPDIR=/tmp sh scripts/agent/check.sh` (or `notest` and the tests of the touched crates),
  `cargo clippy --workspace --all-targets --features full -- -D warnings`.

## Known flakes under load

`service_serve_ingress::the_service_ends_with_the_serve_process`, `shim_direct::a_perl_accept_loop_serves_each_connection`,
`ws_relay::...::a_demoted_relay_host_recovers_on_success`, `autostart::sigterm_is_a_clean_stop`,
`doctor_ssh_relay_row_dials_through_the_proxy`, `supervisor::a_planned_end_is_not_restarted`, `doctor_quick_ends_in_the_limit`,
`guest_wire_unix_socket_serves_a_guest_and_the_next`. Run a failed test alone three times before you conclude a regression.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| `#756` (COORD-126, `pod-session` `move_under`) | A tag with it (T-476). |
| `#753` (COORD-123, `pod-dns` cuts a line inside a cause) | A tag with the fix (T-575). |
| `#754` (COORD-124, `pod-proxy`, a proxy command that does not exist) | A tag with the fix (T-580). |
| `#755` (COORD-125, `pod-cloudflare`, the DoH wait of the edge discovery) | A tag with the fix (T-579). |
| `#654` (COORD-107, takes of podcrates) | A `state:` line at each take. |
| `#760` (COORD-130, a shared headscale harness) | podcrates writes it (T-114, T-115). |
| `#758` (COORD-128, the capture hook of `pod-cloudflare`) | A tag with it (T-244). |
| `#759` (COORD-129, the hand-off of beta.19) | The other sessions read it; then close it through the record. |
| `#604` (COORD-098, relay holds a lost socket) | The relay fix (T-406, T-478). |
| `#413` (COORD-006) | T-161 and T-200 (`pod-relay`). |
