# Handoff

The state at 2026-11-20, 03:30 (UTC). Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md` and
`docs/decisions.md`. Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`; PodUtils/podcrates#225 COORD-134,
PodUtils/podcrates#227 COORD-136). podcrates is the coordinator. COORD issues are the one channel. The order:

1. A red run of podnet's CI.
2. Each ask of podrunner (a COORD issue `(to podnet)`).
3. The take of each tag of podcrates (T-137): `sh scripts/agent/podcrates.sh`.
4. The gate of correctness: T-450, T-452, T-453, T-454, T-455. No release and no new feature before they are done
   (`docs/decisions.md`, 2026-11-17, The QA round and the release freeze). A beta of fixes for podrunner is allowed
   (2026-11-18, call C782).
5. podnet's own work, in the order of `TODO/PROGRESS.md`.

The rulings "Shared code first" and "Quality first" (2026-11-17, in `CLAUDE.md`) hold for each change. An issue closes
only with a test that reproduces it and fails on the code before the fix.

## Where the code is

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (`rustup default 1.99.1`; add
  `rustfmt`, `clippy` and the target `x86_64-unknown-linux-musl` in a new container).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.78.1` (T-495). `sh scripts/agent/podcrates.sh`
  shows a newer tag. The installer core in `scripts/install/` is that of `v0.70.0` (the same file at each tag since).
* The newest release is `v0.4.0-beta.17` (T-481). It has the fixes of PodUtils/podrunner#213 and #142. podrunner's
  user-path gate passes with it on each lane (PodUtils/podrunner#309).
* The record is `TODO/`. `cargo todo check` agrees: 499 entries, 16 open, 14 partial, 36 blocked, 433 done.

## Unfinished work (not on `main`)

None. The head is the commit of this handoff.

## Runs: a push of the session starts no workflow

Since 2026-11-20 a push of the cloud session starts no run (measured: `177336b` and `0dc34d0` got none). Request
each run by a `[gh action] requested` issue (`docs/maintenance.md`, "A run when a push starts no workflow"), one
workflow for each issue. #659 asks `ci` on `0dc34d0`. Read its comment, then watch the run; set its row in
`TODO/issues.md` and its line in `TODO/github.txt` to `closed` with the run and its result.

## The work of the session of 2026-11-20 (02:00 to 03:30 UTC)

* T-450 (partial): the rule "a defect closes with a test that reproduces it" is in `TODO/RULES.md`; the Done sections
  of the defects from 2026-11-17 name their tests; T-433 got its test. The check in `cargo todo` waits for podcrates:
  COORD-110 (#657) asks an opt-in key `defect_tests = "DATE"` of `pod-todo`. When a tag has it: take the tag, set the
  key in `TODO/todo.toml`, close T-450.
* T-452 (partial): the Correction of the entry has what each dialing command does with a dead proxy, a refused port,
  a name with no answer, a reject rule and an unreachable target. Found and fixed: T-496 (`pod` printed no cause at the
  default level; `tests/pod_error_lines.rs` runs the rows on each backend of the host) and T-498 (a timed-out direct
  dial named its step two times). Found in podcrates code: T-497 (blocked; COORD-111, #658: `pod-cloudflare`,
  `pod-warp`, `pod-ws`). The lane has the case `errors` (`scripts/lab-errors.sh`). It found T-499 (done): in the lab
  `no-proxy-no-dns` the netns backend resolved no name (a deleted file mounted over `/etc/resolv.conf`); the pod now
  answers port 53 of the loopback of its namespace. What remains of T-452: the rows of T-497 and the serve paths in a
  lab.

## CI

* A push to `main` starts `ci`, `deny`, `secrets`, `live` and `issue-sync`. Dispatch `gate.yml` by hand on the commit
  that a release needs. `ci` has a concurrency group: a new push cancels the `ci` of the commit before it. `live` keeps
  one running and one pending run: a new push replaces the pending one. Let a `ci` run end before the next push when
  its result matters.
* On `2f1ea82` (T-495, the take of `v0.78.1`): `ci` 44644887040, `gate` 44644888355, `live` 44644887005, `deny`,
  `secrets` and `issue-sync` passed. The commits after it change documents, the record and scripts only: read the
  `ci` of the head first.
* About one `live` run of six fails at the public relay (T-406; COORD-098, #604). GitHub #606 ("live: two runs in a
  row failed") maps to T-406, T-407, T-408 and T-411.
* `record-issues` (each day): green when each issue number has a line in `TODO/github.txt`.

## The work of the session of 2026-11-18 to 2026-11-20 (all on `main`)

* The takes of podcrates `v0.70.0` to `v0.78.1`: T-480, T-483, T-485, T-486, T-489, T-490, T-491, T-493 and T-495. Each
  take is a tag bump with no change of code, except the installer core (T-469, GitHub #645, in `v0.70.0`) and the
  `pod_testkit::refused` helper (T-479, in `v0.71.0`).
* The release `v0.4.0-beta.17` (T-481) for podrunner's PodUtils/podrunner#213 and #142, under call C782.
* T-484: the version line is a contract that podrunner reads (a test of its form and its place in `docs/cli.md`).
* T-487: the image on ghcr.io (the owner's ruling "ghcr.io", call C789 of podcrates). The job `image` of `release.yml`
  attests the manifest list in the registry, pulls each image of one architecture back by its digest and compares its
  binary with the file of the release, and writes the section "The image, by digest" into the notes. Only the next
  release runs these steps.
* T-492 (partial): the road test listens on port 0, and the preflight test takes `pod_testkit::refused`.

## The work of the session of 2026-11-20 (04:00 to 09:00 UTC)

* T-453 (done): the lane row `testers` (`bind-denied` with `PODNET_NETNS_TEST_MEM=none`).
* T-454 (done): `scripts/agent/qa-pass.md`, the prompt of the black-box QA pass, and its place in Release, step 0. The
  first pass runs before the next beta.
* T-455 (done): `scripts/promises.py` (`--report PODNET`, `--notes PODNET`): 384 of 651 promises have a test. It found
  T-500 (done): `pod --help --json` printed text, and the manual of `pod` had no flag of `pod`.
* CI: `ci` passed on `826475c` (#660), `1413960` (#661, T-499) and `a520510` (#662). Request a run of `ci` on the
  commit of T-455 and T-500 (it changes `src/main.rs`, the manual, the llms files and `ci.yml`).
* The gate of the QA round now waits for T-450 (COORD-110) and T-452 (the rows of COORD-111, the serve paths).

## Next steps

`docs/research/podnet/hard-problems.md` (2026-11-20) lists 32 hard or blocked problems (H-01 to H-32), each with its
constraints, research questions and owner. The owner's local agent researches them; read its answers first when they
arrive.

1. Read the `ci` and `live` of the head. Fix each failure first.
2. Start the watcher of the organization (the pinned commit and the command are in `CLAUDE.md`, "Talk with the other
   sessions of PodUtils"):

   ```text
   python3 -I .work/podcrates-agent/watch.py podnet --state .work/agent
   ```

   Take each COORD issue `(to podnet)` of podrunner first. Run `sh scripts/agent/podcrates.sh` at the start and at
   each tag (the next tag after `v0.78.1`), and take each crate whose hand-off is at the tag (T-137). Write each take
   into COORD-107 (#654) as a `state:` line with the CI runs.
3. The next release (`v0.4.0-beta.18`, only for a fix that podrunner needs, until T-450 and T-452 to T-455 are done)
   must prove T-487. Check, in this order:
   * The job `image` passed: the attestation check (`gh attestation verify oci://...`) and the pull-back of each image
     of one architecture (the SHA-256 of its binary equals the file of the release).
   * The notes of the release have the section "The image, by digest".
   * `release-proofs.yml` passed.
   * The hand-off COORD issue of the release names the digest of the manifest list beside the SHA-256 of each file.
   * Then close COORD-109 (#656) by hand, and set its row in `TODO/issues.md` and its line in `TODO/github.txt` to
     `closed`.
   If a step of `image` fails, fix `release.yml` in the next commit: the first run of these steps is the release.
4. The gate of correctness: T-452 (what remains: see above), T-453 (a lab of the testers' host class: no AF_INET bind, no netns, no
   write of the memory of a program), T-450, T-454 (a black-box QA pass), T-455 (the measure). T-451 is done.
5. T-478 and T-406: when `live` fails in "SSH through the reverse relay" or in the resume step, read the logs that the
   step prints and the answer of podcrates on COORD-098 (#604).
6. T-494 (P2): take `pod-release` in place of `src/update.rs` and `src/update/cosign.rs`. Read the Done sections of
   PodUtils/podcrates@T-880 and PodUtils/podcrates@T-966 first. The ruling in `CLAUDE.md` (SHA256SUMS, certificate
   identity and cosign signature checked; no Rekor proof) must hold, or the ruling changes.
7. T-473 (P2), what remains: the replacement of podnet's copies by the shared items of `v0.70.0` (`pod_proxy::hop`,
   `pod_dns::txt::error_answer`, `pod_ws::handshake::build_probe_request`, `pod_tls::env_bundle`, `pod_core::lock`,
   `pod_core::clock`). The doctor row of the SSH relay and `--connect-proxy` go to `pod-proxy` (a SOCKS or
   authenticated proxy then works: a change of tested behaviour). The copies of the clock, the files and the backoff
   go to `pod-core`.
8. T-476 (P2): the cap test of the relay transfer fails under load; measure the bytes of the old leg during a move.
9. T-492 step 3 (P2): the users of `free_port` in `tests/commands.rs` and `tests/warp_stand_in.rs` take a port that the
   child reports, where the command can report one.
10. P3: T-482 (the netns backend prints a second, false cause after a refused namespace; one table test of the
    refusals); T-488 (sporadic HTTP 530 of a quick tunnel with two edge connections, in the step "Throughput through
    WARP" of `live`; measure with `--ha-connections 1`; a change of the default goes to podcrates); T-414 (a third
    form: the WARP proxy ends on a failed health check; its evidence goes to podcrates).
11. The disk allowance holds about one debug build: build with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
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
`shim_direct::a_perl_accept_loop_serves_each_connection`, `ws_relay::...::a_demoted_relay_host_recovers_on_success`,
`autostart::sigterm_is_a_clean_stop`, `doctor_ssh_relay_row_dials_through_the_proxy`,
`transfer_over_the_relay_crosses_the_cap` (T-476) and `guest_wire_unix_socket_serves_a_guest_and_the_next` (keep the
default `TMPDIR`) fail at times in a full run and pass alone. Run a failed test alone three times before a push. A
test that frees a port and binds it again is a fault of the class of T-479 and T-492: take `pod_testkit::refused`, or
bind port 0.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #657 (COORD-110, `pod-todo`: a done defect names its test) | The key in a tag of podcrates; then T-450 closes. |
| #658 (COORD-111, repeated causes in `pod-cloudflare`, `pod-warp`, `pod-ws`) | The fixes in a tag of podcrates; then T-497 adds the rows. |
| #656 (COORD-109, the image on ghcr.io: T-487) | The next release proves the attestation, the pull-back check and the digests in the notes (next step 3). Close it then. |
| #654 (COORD-107, the start report and each take) | The asks of podrunner. Add a `state:` line for each take (its last take is `v0.77.0`; add `v0.78.1`). Close it when the coordinator reads it. |
| #649 (COORD-102, the QA round, to all) | T-450 and T-452 to T-455. Close it when they are done. |
| #651 (COORD-104, the answer to PodUtils/podcrates#221) | T-473 and podcrates' entries 877, 879 and 880 (the lock, `pod-detach`, `pod-release`: T-494). |
| #604 (COORD-098, the relay holds a lost socket) | The owner of ask 1 (the relay server) and a date (T-406, T-478); ask 2 is in `v0.65.0` (T-410). |
| #608 (COORD-101, `pod-tls`: `Trust::Default` ignores the CA file) | The answer of podcrates. T-421 is done in podnet (`ws_trust()`). |
| #585 (COORD-091), #592 (COORD-095) | T-358 (the release gate; partial). T-359, T-380 and T-381 are done. |
| #413 (COORD-006) | T-161 and T-200 (blocked on `pod-relay`). |
| #437 (COORD-019) | The proposals of the donor review that podcrates has not taken. T-417 took one (`replace_private`). |

The other open issues (#89, #140, #149, #151, #161, #222, #358, #360, #375, #397, #407, #435, #461, #469, #471, #507,
#606, #620 and the deferred #484 to #501) are in `TODO/issues.md`.

## Waiting on the owner

None. The owner ruled on 2026-11-16: the entry 305 is removed (#475 closed, T-396), `--seal` stays opt-in (Q1), and
each triaged issue of Nemo-010 gets the label `confirmed` (not the deferred #484 to #501).
