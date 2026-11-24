# Handoff

The state at 2026-11-24 10:00 (UTC). Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`,
`docs/maintenance.md`, and `docs/decisions.md`. Remove this file when the lead session has no unfinished step.

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

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1. podnet takes the crates of
  `PodUtils/podcrates` by the git tag `v0.82.0`. The tag `v0.82.1` is out (COORD-152, PodUtils/podcrates#257): its take
  is the first step of the next session (below).
* The release `v0.4.0-beta.18` (T-533) was published from `de6027b` on 2026-11-24: see "The release" below.
* The record: `cargo todo check` agrees: 536 entries.

## The release of v0.4.0-beta.18 (T-533)

* `release.yml` 45366921104 (`publish=v0.4.0-beta.18`) made the tag; the release run is 45366987263 on `de6027b`.
  At this handoff the run builds the binaries (each target, the gate lane, then the image and the release).
* Evidence: `ci` 45360283838 and the dry run 45360290009 passed on `de6027b`; `live` 45353877401 passed on `27d2e3a`
  (the same code: `de6027b` changes only `release.yml` and the record); `gate` 45322155916 passed on `cfbc215` (the same
  code and lane scripts).
* After the run: check the release files and `release-proofs.yml` (each file OK, Verified OK), the job `image` (the
  attestation, the pull-back by digest, the section "The image, by digest" in the notes: T-487). Then set T-533 done
  with its Done section, write the digest of the manifest list beside the SHA-256 of each file in a hand-off COORD issue
  `(to all)`, and close COORD-109 (#656) and its record row (T-487).

## The next steps, in order

1. **Take podcrates `v0.82.1`** (T-137; COORD-152). A worktree of the last session held a finished take (commit
   `2552b93`, not pushed, lost with the container); do it again from these facts:
   * Bump each `pod-*` pin to `v0.82.1` (the manifests, `Cargo.lock`, `fuzz/Cargo.lock`, `scripts/install/SOURCES`:
     tag and commit only, `core.sh`, `core-net.sh` and `pins.py` keep their SHA-256; `.github/workflows/issue-sync.yml`:
     `uses:` at `0d7832b9...` and `podcrates_ref: v0.82.1`). The tag has no `adoption/podnet/` patch.
   * COORD-113 (#685) is answered: a refusal with retry: yes gets 30 s of tries; "no edge leg worked" is TempFail
     (exit 75) when each leg asked for a retry, with the typed `fail::Refused` of the last leg in its chain.
     `crates/podnet-core/src/hop.rs` `refusals` must read the typed refusal first and then the text of the chain for
     the other legs (an early return on the typed one gives exit 75 by mistake when an earlier leg said retry: no).
     `src/tunnel.rs` `again` stays as the second path. `standin::Reply` is `non_exhaustive` (podnet does not match on
     it), with `RefuseRetry` and `Edge::script`: add a row to `tests/quick_tunnel_standin.rs` (4 refusals, then an
     accept: ready with 1 provision). `docs/tunnels.md` and `docs/protocol.md` 3.3: the attempt limits are 110 s
     (`quic`) and 210 s (`http2`, `http2+relay`). Take `pod_testkit::need_route!` in place of the copy in
     `tests/test_host/mod.rs`.
   * COORD-115 (#707): `v0.82.1` answers none of its asks. T-516 and T-522 stay partial.
   * Record a new entry for the take, a new entry for the ask PodUtils/podcrates@T-1118 (`release.yml` writes
     `release-index.json` before `SHA256SUMS`), a line in T-137, and a Correction in T-513. Then a COORD issue
     `(to podcrates)` with the commit, and close COORD-113 (#685) and COORD-114 (#686) when podcrates has read them.
2. **Open follow-ups of this session:** T-535 (`scripts/live-ws-url.sh` prints no line of podnet on a timeout), T-536
   (a refused unix bind names a false cause), T-537 (the build image of Dependabot #680 cannot link loongarch64; close
   each Dependabot PR of that image with a link to T-537 until it is fixed), and a COORD ask `(to podcrates)` from
   T-534: one `socket_answers` in `pod-core` for the one-connect probes of `pod-control` (`src/unix.rs:27`), `pod-link`
   (`src/unix.rs:154`) and `pod-warp` (`src/find.rs:112`).
3. The rest of the work order of `TODO/PROGRESS.md`: T-516 and T-522 (wait for COORD-115), T-476, T-492 step 3, T-367,
   T-488, T-414, T-415, T-406, T-478.

## Facts for the next session

* `live` fails often at outside services: WARP's door (a certificate of Let's Encrypt, T-406), WARP's API and the
  quick-tunnel service (HTTP 429, error 1015, for one hour after many runs from one address), quick tunnels (HTTP 530,
  T-488). Read the failed step before a fix; do not run `live` again and again from one address.
* The GitHub MCP tools add a "Generated by" line to each issue body and comment; an edit adds it again. `CLAUDE.md`
  forbids it: remove the lines by hand, or write through another tool.
* The forge mirror (`forge.web.pkgforge.dev`) answers 403 at times, and its token cannot read job logs of PodUtils
  (the token lifetime of the organization). Read job logs with `mcp__github__get_job_logs` (`return_content: false`
  gives a URL to curl).
* Parallel fix agents in worktrees: each edits the record (`TODO/INDEX.md`, `TODO/PROGRESS.md`, `TODO/issues.md`,
  `TODO/github.txt`); at the cherry-pick keep the most advanced status of each row, run `cargo todo counts`, and keep
  one `| **All** |` row. An issue row closes only with the real commit.
* Run `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` before each push (an agent skipped it once).

## How to work here

* Build with `TMPDIR=/tmp CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
* After a change of a document or help text, run `cargo test --bin podnet write_the_llms_files -- --ignored`.
* Run the local gate before pushing: `TMPDIR=/tmp sh scripts/agent/check.sh`, plus
  `cargo clippy --workspace --all-targets --features full -- -D warnings`,
  `cargo test --features full --test sidecar`, `python3 -I scripts/check-harness.py --self-test`, and
  `python3 -I scripts/check-claims.py`.
* Touch each changed file before `sh scripts/agent/check.sh` if a stash or file move leaves stale timestamps.
* Run the podlab lane locally: `sh scripts/get-podlab.sh v0.68.0 .work/podlab-v0.68.0`, build podnet, copy the binary,
  podlab asset, `scripts/lab-lane.sh`, `scripts/lab-errors.sh`, and `tests/consumers/` to a directory readable by uid
  65534 (such as `/var/tmp/lane/`), then run as root:
  `setpriv --reuid=65534 --regid=65534 --clear-groups env -i PATH=/usr/bin:/bin HOME=OTHERDIR LANE_DIR=DIR/out sh scripts/lab-lane.sh PODLAB DIR/podnet [PROFILE...]`.
  Note `arm64-tcg`, `cgroup-v1`, and `ipv6-only` do not hold in local containers (`ipv6-only` runs on GitHub runners).
* Test refused namespaces with `unshare -rU` and `echo 0 > /proc/sys/user/max_net_namespaces` (and
  `max_user_namespaces`); test a deleted `/etc/resolv.conf` mount with `unshare -rm`, `mount --bind`, and `rm`.

## Known flakes under load

`service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection`,
`ws_relay::...::a_demoted_relay_host_recovers_on_success`,
`autostart::sigterm_is_a_clean_stop`,
`doctor_ssh_relay_row_dials_through_the_proxy`,
`supervisor::a_planned_end_is_not_restarted`,
`doctor_quick_ends_in_the_limit`, and
`guest_wire_unix_socket_serves_a_guest_and_the_next` (keep short `TMPDIR=/tmp`)
can fail under heavy parallel load and pass alone. Run a failed test alone three times before concluding a regression.
Use `pod_testkit::refused`, bind port `0`, or use `127.0.0.1:1` instead of freeing and rebinding a port (`T-479`, `T-492`).

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| `#707` (COORD-115, the asks of the QA pass of beta.18) | A tag of podcrates; T-516 and T-522 partial. |
| `#686` (COORD-114, the take of `pod-release`) | Done in podcrates (T-966); close after the next take. |
| `#685` (COORD-113, the retry budget of `pod-cloudflare`) | Answered in `v0.82.1`; close with the take. |
| `#654` (COORD-107, takes of podcrates) | A `state:` line at each take. |
| `#656` (COORD-109, ghcr.io image: T-487) | The result of the release of beta.18. |
| `#604` (COORD-098, relay holds a lost socket) | The relay fix (T-406, T-478). |
| `#413` (COORD-006) | T-161 and T-200 (`pod-relay`). |
