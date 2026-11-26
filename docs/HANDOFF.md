# Handoff

The product work state at 2026-11-26 21:30 (UTC). Start a hosted checkout with
[TODO/RESUME.md](../TODO/RESUME.md). Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`). The order: a red run of podnet's CI; each ask of
podrunner (a COORD issue `(to podnet)`); the take of each tag of podcrates (T-137); podnet's own work in the order of
`TODO/PROGRESS.md`. The newest tag of podcrates is `v0.86.0` (taken in T-612). The open ask of podrunner,
PodUtils/podrunner#369, has its fix on `main` (T-619) and its answer (COORD-139 #784); it closes with the release of
`v0.4.0-beta.21`.

## Where the code is

* All finished work is on `main` (`55f72144` at 21:10). `ci` 45856630710 (`bffca86`), `live` 45861374486 and
  `gate.yml` 45866261787 (both `8712a9c`) pass: the batch for `v0.4.0-beta.21` is green. The commits after `8712a9c` are
  records and documents only.
* The newest release is `v0.4.0-beta.20` (T-608; tag at `be2c0df`; COORD-132 #777).
* Done since beta.20: T-612 (podcrates `v0.86.0`), T-609, T-610, T-611, T-613, T-615 to T-619, T-126 to T-129 (the
  networks `https://`, `wstunnel:`, `vless://`, `ss://`, `wisp:`; the DERP road; ACME), T-244 to its hook gaps, T-294
  (the service `ss`), T-321 (TURN), T-358 (the release gate). Each new road has a `live` step against a real server
  (coturn, Pebble, ssserver, Xray-core, wisp-server-python, derper).
* Partial or blocked on podcrates: T-244 (COORD-136 #781), T-295 (COORD-140 #785), T-614 (COORD-134 #779), T-114 and
  T-115 (COORD-130 #760), T-123 and T-124 (no `pod-ts`). COORD-135 (#780), COORD-137 (#782) and COORD-138 (#783) ask
  for extensions of taken crates.
* Backups on GitHub: `wip/beta21-release-notes` (the draft of the notes of beta.21,
  `docs/releases/v0.4.0-beta.21.draft.md`); `wip/worktree-agent-a0fa474aa2d48b577` and
  `wip/worktree-agent-a1b55fa9cb5648e19` (the T-358 commits of two agents before the resolution of their conflicts; `main`
  has their content).

## The next steps, in order

1. **The findings of the QA pass of `v0.4.0-beta.21`** (#786 to #798; the pass ended, its summary is the last comment
   of #786): the entries are T-620 to T-632 (P2: T-620, T-621, T-622, T-623, T-624, T-629; the rest P3). Four agents
   work in worktrees: the triage of the entries (TODO only), group A (T-620, T-622, T-625, T-629: the error lines of a
   dial), group B (T-621, T-624, T-632: `exit --stdio`, `ssh-relay`, `pod --log-format json`), group C (T-623, T-626,
   T-627, T-628, T-630, T-631: texts and documents). Take each commit onto `main`, write the Done sections, and close the
   issues through the record. With no agent running (a new session), take the entries one by one.
2. **The release of `v0.4.0-beta.21`** (`docs/maintenance.md`, Release): set the version; take the notes from
   `wip/beta21-release-notes` into `docs/releases/v0.4.0-beta.21.md` and add the fixes of the QA pass; push; wait for
   `ci`, `live` and `gate.yml` on that commit; the dry run of `release.yml`; then `publish=v0.4.0-beta.21`; check
   `release-proofs.yml`. Then a COORD issue `(to all)` with the files, their SHA-256 and the image by digest, and a line
   `state:` on COORD-139 (#784) with the version for podrunner.
3. **Each tag of podcrates:** take it (T-137) and close the COORD issues that it answers.
4. Waits for a run or a ruling: T-414 and T-415 (a warning of their `live` steps), T-478 (blocked on T-406), T-488,
   T-001 (the owner).

## Facts for the next session

* Start each worktree agent with the shell in `/workspace/podnet`: the `isolation: worktree` of the Agent tool takes
  the repository of the current directory. A `cd` into `.work/cloud/cargo/...` once made the worktree in the cargo cache
  of podcrates.
* The permission policy of this host refuses a run of a downloaded third-party binary (Pebble) and a change of
  `/etc/hosts`. Do not work around it: a test against a real server goes into a step of `live.yml`.
* A push cancels the `ci` run of the commit before it (`cancel-in-progress`); `live` waits instead. A commit of the record
  or of a workflow alone takes `[skip ci]`, and `live` can then start by `workflow_dispatch` (`actions_run_trigger`).
* A step of `live.yml` runs under `bash -eo pipefail` (`scripts/live-step.sh`): a `grep` with no match in a pipe ends
  the step; give it `|| true` where no match is normal.
* The test `report::tests::the_log_filter_names_each_pod_crate_of_podnet` needs each new `pod-*` crate of `Cargo.toml`
  in the log filter of `crates/podnet-core/src/report.rs`; `each_variable_of_the_code_is_a_fact` needs each new
  variable in `src/manual/facts.rs`; a test that binds the loopback starts with `need_loopback!()`.
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
