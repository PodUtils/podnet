# Handoff

The state at 2026-11-21, 07:30 (UTC). Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md` and
`docs/decisions.md`. Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`; PodUtils/podcrates#225 COORD-134,
PodUtils/podcrates#227 COORD-136). podcrates is the coordinator. COORD issues are the one channel. The order:

1. A red run of podnet's CI.
2. Each ask of podrunner (a COORD issue `(to podnet)`).
3. The take of each tag of podcrates (T-137): `sh scripts/agent/podcrates.sh`.
4. podnet's own work, in the order of `TODO/PROGRESS.md`.

The gate of correctness of the QA round (T-450 to T-455) is done (2026-11-20), and the release freeze of 2026-11-17
ends with it (`docs/decisions.md`, 2026-11-20, The end of the release freeze). The rulings "Shared code first" and
"Quality first" (2026-11-17, in `CLAUDE.md`) hold for each change. A defect closes only with a test that reproduces it
and fails on the code before the fix; `cargo todo check` refuses a done defect from 2026-11-17 whose Done section names
no test (`defect_tests` in `TODO/todo.toml`).

## Where the code is

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (`rustup default 1.99.1`; add
  `rustfmt`, `clippy` and the target `x86_64-unknown-linux-musl` in a new container).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.81.0` (T-505). The installer core in
  `scripts/install/` has the same SHA-256 at each tag since `v0.70.0`.
* The newest release is `v0.4.0-beta.17` (T-481).
* The record is `TODO/`. `cargo todo check` agrees: 504 entries, 14 open, 13 partial, 35 blocked, 442 done.

## Unfinished work (not on `main`)

None. The tree is clean; each commit is on `origin/main`. The remote branch `claude/quirky-mccarthy-ad1v62` holds no
commit that `main` does not have (it points at `18f9a70`): a stale branch of a session. `CLAUDE.md` lets a session push
`main` only, so it stays for the owner (`scripts/agent/delete-stale-branches.ps1`).

## Runs: a push of the session starts no workflow

Since 2026-11-20 a push of the cloud session starts no run, and the session cannot dispatch one. Request each run by a
`[gh action] requested` issue (`docs/maintenance.md`, "A run when a push starts no workflow"), one workflow for each
issue, with the full SHA of the head in its block. Push nothing until the owner's agent writes "Dispatched" (the block
finds its run by that SHA); then map the issue in `TODO/issues.md` and `TODO/github.txt`, watch the run, and close the
issue through the record with the run and its result.

* `ci` has a concurrency group: a new dispatch cancels the `ci` that runs. Request the next `ci` after the last one
  ends.
* `issue-sync` starts on a push only. After a commit that closes or opens an issue in the record, request
  `issue-sync.yml -f dry_run=false`; else the daily `record-issues` fails on each difference.
* At the end of this session the record closes #672 and #677, which GitHub still shows open: the last request of the
  session asks `issue-sync` for them (see "Open requests" below).

## CI on the head

* `ci` 44871490037 passed on `9a9a0a9` (the take of `v0.81.0`). The commits after it change the record only.
* `gate` 44782773447 passed on `3836971`: the full lane on the GitHub runner, with the case `errors` (20 error rows in
  each lab) and the row `testers`.
* `live` 44713609259 passed on `3f14a38`. About one `live` run of six fails at the public relay or at WARP's MASQUE
  door (T-406, T-488; `live` 44733644822 on 2026-11-20 is one, in T-406). GitHub #606 maps to T-406 to T-411.

## The work of 2026-11-20 and 2026-11-21 (all on `main`)

* The gate of the QA round: T-450 (the rule and the check `defect_tests`), T-452 (`scripts/lab-errors.sh`, the case
  `errors` of the lane), T-453 (the row `testers`), T-454 (`scripts/agent/qa-pass.md`), T-455 (`scripts/promises.py`).
* Defects that the error rows found, each with a test that fails on the code before: T-482, T-496, T-497 (in
  podcrates), T-498, T-499, T-500, T-502, T-503.
* The takes of podcrates `v0.79.0` (T-501), `v0.80.0` (T-504) and `v0.81.0` (T-505), each written into COORD-107 (#654).
* T-473 (partial): the date code takes `pod_core::clock`, the retry waits of `warp run` and `warp tun` take
  `pod_core::backoff`, and the two atomic replaces take `pod_core::files::replace` (the answer to COORD-112, #677).

## Next steps

`docs/research/podnet/hard-problems.md` (2026-11-20) lists 32 hard or blocked problems (H-01 to H-32). The owner's local
agent researches them; read its answers first when they arrive.

1. Read the `ci` and `live` of the head. Fix each failure first.
2. Start the watcher of the organization (the pinned commit and the command are in `CLAUDE.md`, "Talk with the other
   sessions of PodUtils"):

   ```text
   python3 -I .work/podcrates-agent/watch.py podnet --state .work/agent
   ```

   Take each COORD issue `(to podnet)` of podrunner first. Run `sh scripts/agent/podcrates.sh` at the start and at each
   tag (the next tag after `v0.81.0`); take the tag (T-137), and write the take into COORD-107 (#654) as a `state:`
   line with its CI run.
3. The next beta (`v0.4.0-beta.18`). Before its dry run (`docs/maintenance.md`, Release, step 0):
   * The black-box QA pass: give `scripts/agent/qa-pass.md` as it is to a new agent with the binary of the commit, the
     documents and podlab only (not a session that has read the source). Triage each finding into an entry.
   * `gate.yml` on the commit.

   Then the release steps. The release must prove T-487: the job `image` passed (the attestation check and the
   pull-back of each image), the notes have the section "The image, by digest", `release-proofs.yml` passed, and the
   hand-off COORD issue names the digest of the manifest list. Then close COORD-109 (#656) through the record.
4. T-473 (P2), what remains: the CONNECT client of `crates/podnet-core/src/connect.rs` (the `--connect-proxy` path of
   `ssh-relay`; to `pod-proxy`, a change of tested behaviour), the HTTP head readers of
   `crates/podnet-netns/src/tailscale.rs` and `src/router/control.rs` (`pod_proxy::http`), the request parser of
   `crates/podnet-core/src/util/http.rs`, and the groups and the lead of the help (`pod-manual`). The `Backoff` of
   `crates/podnet-serve/src/relay/supervise.rs` stays (its tests give it short waits).
5. T-494 (P2): take `pod-release` in place of `src/update.rs` and `src/update/cosign.rs`. Read
   PodUtils/podcrates@T-880, PodUtils/podcrates@T-966 and PodUtils/podcrates@T-964 (Rekor v2) first. The ruling in
   `CLAUDE.md` (SHA256SUMS, certificate identity and cosign signature checked; no Rekor proof) must hold, or the ruling
   changes.
6. Optional takes of `v0.80.0`: `pod-unit` for `src/cli/cmd/autostart.rs`, `pod-control` for a status socket.
7. T-476 (P2): the cap test of the relay transfer fails under load; measure the bytes of the old leg during a move.
8. T-492 step 3 (P3): the users of `free_port` in `tests/commands.rs` and `tests/warp_stand_in.rs` take a port that the
   child reports, where the command can report one.
9. P3: T-488 (sporadic HTTP 530 of a quick tunnel; measure with `--ha-connections 1`), T-414 (the WARP proxy ends on
   a failed health check; its evidence goes to podcrates). T-406 and T-478: read the logs of a failed `live` step and
   the answer of podcrates on COORD-098 (#604).

## How to work here

* The disk allowance holds about one debug build: build with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
  CARGO_PROFILE_TEST_DEBUG=0`. After a change of a document or a help text, run
  `cargo test --bin podnet write_the_llms_files -- --ignored`.
* A control on old code (a stash, or a file moved out and back) can leave times that make cargo take a stale build as
  current: touch each changed file before the full check (`sh scripts/agent/check.sh`). This hid the failure of
  `ci` 44701681199.
* Run the lane here: `sh scripts/get-podlab.sh v0.68.0 .work/podlab-v0.68.0`; build podnet; copy the binary, the
  podlab asset, `scripts/lab-lane.sh`, `scripts/lab-errors.sh` and `tests/consumers/` to a directory that uid 65534
  can read (for example `/var/tmp/lane/`, not in `/tmp` or HOME); then as root: `setpriv --reuid=65534 --regid=65534
  --clear-groups env -i PATH=/usr/bin:/bin HOME=OTHERDIR LANE_DIR=DIR/out sh scripts/lab-lane.sh PODLAB DIR/podnet
  [PROFILE...]`. One case in one lab: `podlab run PROFILE -- /bin/sh lab-lane.sh --case errors PODNET OUTDIR`. The labs
  `arm64-tcg`, `cgroup-v1` and `ipv6-only` do not hold in this container; the GitHub runner has `ipv6-only`.
* A namespace that refuses new namespaces, for a test: `unshare -rU` and `echo 0 > /proc/sys/user/max_net_namespaces`
  (and `max_user_namespaces`). A deleted file mounted over `/etc/resolv.conf`: `unshare -rm`, `mount --bind`, `rm`.
* Issues #484 to #501 are deferred by the owner: leave them open and unlabelled.

## Known flakes under load

`quick_tunnel_standin_serves_a_visitor_over_http2`, `service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection`, `ws_relay::...::a_demoted_relay_host_recovers_on_success`,
`autostart::sigterm_is_a_clean_stop`, `doctor_ssh_relay_row_dials_through_the_proxy`,
`transfer_over_the_relay_crosses_the_cap` (T-476) and `guest_wire_unix_socket_serves_a_guest_and_the_next` (keep the
default `TMPDIR`) fail at times in a full run and pass alone. Run a failed test alone three times before a push. A
test that frees a port and binds it again is a fault of the class of T-479 and T-492: take `pod_testkit::refused`, or
bind port 0, or a port that nothing listens on and needs no bind (`127.0.0.1:1`).

## Open requests and COORD issues

| Issue | Waits for |
| --- | --- |
| The last `[gh action] requested` of this session | `issue-sync -f dry_run=false` on the last head, so GitHub closes #672 and #677. Map it and close it through the record when its run passes. |
| #654 (COORD-107, the start report and each take) | A `state:` line for each take (its last is `v0.81.0`). Close it when the coordinator says that it read it. |
| #656 (COORD-109, the image on ghcr.io: T-487) | The next release proves the attestation, the pull-back check and the digests (next step 3). |
| #651 (COORD-104, the answer to PodUtils/podcrates#221) | T-473 and podcrates' entries 877, 879 and 880 (the lock, `pod-detach`, `pod-release`: T-494). |
| #604 (COORD-098, the relay holds a lost socket) | The owner of ask 1 (the relay server) and a date (T-406, T-478). |
| #608 (COORD-101, `pod-tls`: `Trust::Default` ignores the CA file) | The answer of podcrates. T-421 is done in podnet (`ws_trust()`). |
| #585 (COORD-091), #592 (COORD-095) | T-358 (the release gate; steps 2 and 3 are partial). |
| #413 (COORD-006) | T-161 and T-200 (blocked on `pod-relay`). |
| #437 (COORD-019) | The proposals of the donor review that podcrates has not taken. |

The other open issues (#89, #140, #149, #151, #161, #222, #358, #360, #375, #397, #407, #435, #461, #469, #471, #507,
#606, #620 and the deferred #484 to #501) are in `TODO/issues.md`.

## Waiting on the owner

* The first black-box QA pass before the next beta (`scripts/agent/qa-pass.md`): a session with the binary and the
  documents only.
* The stale remote branch `claude/quirky-mccarthy-ad1v62` (no own commit).
