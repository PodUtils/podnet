# Handoff

The product work state at 2026-11-25 14:30 (UTC). Start a hosted checkout with
[TODO/RESUME.md](../TODO/RESUME.md). Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`). The order: a red run of podnet's CI; each ask of
podrunner (a COORD issue `(to podnet)`); the take of each tag of podcrates (T-137); podnet's own work in the order of
`TODO/PROGRESS.md`. No new ask of podrunner and no new tag of podcrates (the newest is `v0.84.0`, taken) at 14:30.

## Where the code is

* All finished work is on `main`. Use the pinned Rust toolchain from [Hosted sessions](hosted-sessions.md).
* Done in the session of 2026-11-25 (afternoon): T-583 (the take of podcrates `v0.84.0`, `405d47c`; COORD-122 #752 read
  by podcrates), T-546 and T-550 (by the tag), the reviewed fixes of the QA pass of `0.4.0-beta.19` (T-547, T-572 to
  T-574, T-576 to T-578, T-581), T-564, T-582, T-584 (#720), and T-543 steps 1 and 2 (the row `nat` of `doctor`).
* Blocked on podcrates: T-575 (COORD-123 #753, `pod-dns`), T-580 (COORD-124 #754, `pod-proxy`), T-579 (COORD-125 #755,
  `pod-cloudflare`). The release notes of beta.19 name them (`docs/decisions.md`, 2026-11-25).

## The next steps, in order

1. **The release `v0.4.0-beta.19`:** `docs/releases/v0.4.0-beta.19.md` is written. Then the steps 1 to 6 of
   `docs/maintenance.md`, Release: the version, `ci` and `live` green on the commit, `gate.yml`, the dry run, the
   publish, `release-proofs.yml`. The job `image` proves T-487; then the digests in a hand-off COORD issue `(to all)`,
   and close COORD-109 (#656) through the record.
2. **T-543 steps 3 and 4:** `link p2p` takes the relay first for two dependent mappings; the table test over the
   stand-in NATs of `tests/link_p2p_namespaces.rs`.
3. The rest of the work order: T-476, T-492 step 3, T-367, T-488, T-414, T-415, T-406, T-478.
4. **The daily run of `issue-sync`:** check that the event `schedule` runs.

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
| `#753` (COORD-123, `pod-dns` cuts a line inside a cause) | A tag with the fix (T-575). |
| `#754` (COORD-124, `pod-proxy`, a proxy command that does not exist) | A tag with the fix (T-580). |
| `#755` (COORD-125, `pod-cloudflare`, the DoH wait of the edge discovery) | A tag with the fix (T-579). |
| `#654` (COORD-107, takes of podcrates) | A `state:` line at each take. |
| `#656` (COORD-109, ghcr.io image: T-487) | The job `image` of the release of beta.19. |
| `#604` (COORD-098, relay holds a lost socket) | The relay fix (T-406, T-478). |
| `#413` (COORD-006) | T-161 and T-200 (`pod-relay`). |
