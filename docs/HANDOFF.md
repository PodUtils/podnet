# Handoff

The product work state at 2026-11-26 10:40 (UTC). Start a hosted checkout with
[TODO/RESUME.md](../TODO/RESUME.md). Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`). The order: a red run of podnet's CI; each ask of
podrunner (a COORD issue `(to podnet)`); the take of each tag of podcrates (T-137); podnet's own work in the order of
`TODO/PROGRESS.md`. No new ask of podrunner and no new tag of podcrates (the newest is `v0.86.0`, taken in T-612) at 10:40.

## Where the code is

* All finished work is on `main`. Use the pinned Rust toolchain from [Hosted sessions](hosted-sessions.md).
* Done on 2026-11-26 (10:40): T-611 (the TCP direction split), T-126 (the networks `https://` and `wstunnel:` of
  `pod-egress`; T-614 blocked on COORD-134 #779), T-358 step 2 (b, c) (the `live` and `root` blocks of the documents in
  `live.yml`, the offline blocks in the labs), T-613, T-615, T-616, T-617. T-123 and T-124 are blocked (no `pod-ts`).
* Done on 2026-11-26 after beta.20: T-612 (the take of podcrates `v0.86.0`; T-575, T-579, T-580, T-476, T-598 done by
  it; T-244 open again), T-610, T-613, T-179 (the design of the TCP split), T-358 step 3 (the lane case
  `consumer-podrunner`) and step 2 (a) (the class `scenario`: 23 documented blocks run), T-321 item 4 dropped
  (`docs/decisions.md`). T-609 and the coturn step of T-321 wait for their runs.
* The release `v0.4.0-beta.20` is out (T-608; tag at `be2c0df`; COORD-132 #777): the carriers of the feature `release`
  (#620) and the fixes of the QA pass of 2026-11-26 (#761 to #775, T-593 to T-607; T-598 blocked on COORD-131 #776).
  T-281 is done (the proof of the DNS carrier).
* Done on 2026-11-26 (later): T-589 (proven by `live` 45738194994), T-398 (each command of a release binary but `net
  tailscale` has a scenario: the lane cases `local-files`, `local-udp` and `local-tcp`), T-590 (the podcrates version of
  the crash log and the harness), T-591 (`cover` with no `--name`), T-592 (`link p2p` to the end of the answer; `main`
  ends the runtime in 1 s), T-321 item 3 (a TURN account for each router network).
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

1. **Three agents work in worktrees** (`.claude/worktrees/`): T-244 (`tunnel --inspect` over the capture hook of
   `pod-cloudflare` `v0.86.0`), T-129 (ACME for `serve` and `git serve`, `pod-acme`) and T-127 (VLESS, Shadowsocks and
   Wisp, `pod-egress`). Review each commit, take it onto `main`, and write the record (they do not edit
   `TODO/INDEX.md`, `PROGRESS.md`, `issues.md`, `github.txt`). Then T-128 (the DERP carrier, `pod-derp`).
2. **The proofs that wait for a run:** T-609 (the dry run 45786027233 passed; the `live` step "A link over ICMP echo
   (root)", fixed in `0248aa61` with `link status --create`), T-321 (the `live` step of coturn: UDP and TCP over RFC
   6062). Mark each done when its run passes.
3. **T-358:** step 1 `ipv6-only` (no IPv6 on the runner).
4. **Each tag of podcrates:** COORD-130 (#760, the headscale harness: T-114, T-115), COORD-134 (#779, T-614).
5. **The next beta (`v0.4.0-beta.21`):** the black-box QA pass first, when the batch above is green. The notes name the
   breaking change: a network spec `https://` is now a TLS proxy.
6. Waits for a run or a tag: T-414 and T-415 (a warning of their `live` steps), T-478 (blocked on T-406), T-488 (runs
   that the quota limits), T-123 and T-124 (`pod-ts`).

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
| `#777` (COORD-132, the hand-off of beta.20) | The other sessions read it; then close it through the record. |
| `#776` (COORD-131, `pod-link`: the TCP probe names the IPv6 try) | A tag with the fix (T-598). |
| `#760` (COORD-130, a shared headscale harness) | podcrates writes it (T-114, T-115). |
| `#758` (COORD-128, the capture hook of `pod-cloudflare`) | A tag with it (T-244). |
| `#759` (COORD-129, the hand-off of beta.19) | The other sessions read it; then close it through the record. |
| `#604` (COORD-098, relay holds a lost socket) | The relay fix (T-406, T-478). |
| `#413` (COORD-006) | T-161 and T-200 (`pod-relay`). |
