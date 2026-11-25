# Handoff

The state at 2026-11-25 00:50 (UTC). Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`,
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

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (a cloud container can have an
  older default: `rustup default 1.99`; with Rust 1.97, cargo says "no matching package named `pod-release`").
  podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.83.0` (T-542; COORD-154, PodUtils/podcrates#259).
* The newest release is `v0.4.0-beta.18` (T-533), published from `de6027b`. `release-proofs.yml` 45374896321 passed.
  Its job `image` failed after the push (T-538: a `docker logout` before `actions/attest`; fixed in `fefb28e`, with a
  rule in `scripts/check-workflows.py`). The images of beta.18 have no attestation; their digests are in COORD-109 (#656).
* The release dry run 45389095107 passed on `c6b5bdf` (T-540). Read the runs of `ci` and `live` on the head before
  each release. `live` fails often at outside services (see "Facts").
* The last session (2026-11-24) took podcrates `v0.83.0` (`a6a6ccb`, T-542; T-516 and T-522 are done; #690, #696 to
  #698, COORD-115 #707 and COORD-117 #709 are closed) and the shared `runs.py` (`1bf4161`, T-544). At its end, `ci`
  45518282235 and `live` 45518282050 on `1bf4161` were running: read them first. The local full suite of the take
  passed (172 test binaries). `live` 45412239533 and 45422220326 failed at the quick-tunnel service (HTTP 429, error
  1015): an outside rate limit; podnet gave the true line and exit 75.
* The record: `cargo todo check` agrees.

## The next steps, in order

These steps are the start of the work, not its end. After them, continue with the work order of `TODO/PROGRESS.md`.
Do not stop for a confirmation and do not end the turn after a report (`CLAUDE.md`, "Work without a stop").

1. **Read the runs of `ci` and `live` on the last commits** (the watcher prints each end). A red run goes first.
2. **T-537** (the newer build image `rust:1-alpine@sha256:8d0e82ab` of Dependabot #680 cannot link loongarch64).
   Found: the job log of dry run 45347322318 (job 138284933209) shows that rustc gives `-Wl,--no-rosegment` as the
   first argument to the wrapper of cargo-zigbuild `0.23.4` with zig `0.16.0`, and zig cc refuses it ("unsupported
   linker arg: --no-rosegment"). The old digest `0cce0a5e` gives no such argument. Next: find the rustc version of
   the new image (the target spec of `loongarch64-unknown-linux-musl`), then try a newer cargo-zigbuild (a filter of
   the argument) or a zig version that takes it, in a release dry run on a commit with the new digest. Until then,
   close each Dependabot PR of that image with a link to T-537.
3. **The next beta `v0.4.0-beta.19`.** Step 0 is the black-box QA pass: give `scripts/agent/qa-pass.md` as it is to
   a new agent, with a static binary of the head (`rustup target add x86_64-unknown-linux-musl`, then
   `CC_x86_64_unknown_linux_musl=clang cargo build --release --locked --target x86_64-unknown-linux-musl`), a copy of
   `README.md` and `docs/`, and podlab (`sh scripts/get-podlab.sh v0.83.0 DIR`), in a directory that uid 65534 can
   read (for example `/var/tmp/qa`). The agent must not read the source. Each finding becomes an entry; each P1 and
   P2 finding is done before the release. Then the steps 1 to 6 of `docs/maintenance.md`, Release. The notes name
   `release-index.json` (T-540). Its job `image` proves T-487 (the attestation, `gh attestation verify`, the section
   "The image, by digest"); then write the digests in a hand-off COORD issue `(to all)` and close COORD-109 (#656)
   through the record.
4. **T-545** (P2: temp paths named by the pid alone, lesson 5 of PodUtils/podvm#567). The ask for the shared
   function is COORD-119 (#711); write podnet's second path in `podnet-core` and the check of the class now, and
   take the function of podcrates when its tag is out.
5. **T-543** (P2: take `pod-natprobe` for a NAT row of `doctor` and the path choice of `link p2p`).
6. The rest of the work order of `TODO/PROGRESS.md`: T-476, T-492 step 3 (`warp forward`, `warp up`,
   `tests/commands.rs`), T-367, T-488, T-414, T-415, T-406, T-478.
7. **The daily run of `issue-sync`** has never started on its schedule (the audit of 2026-11-24). `781959e` aligned
   its `dry_run` line with podrunner's caller. Check the `schedule` runs of `issue-sync` after 06:17 UTC of
   2026-11-25; when none started, find the cause and write a COORD issue `(to podcrates)` if the fault is in the
   reusable workflow.

## Facts for the next session

* An issue closes through the record: set its row in `TODO/issues.md` and its line in `TODO/github.txt` to `closed`
  before any close on GitHub. `issue-sync` opens again an issue whose row is open (`docs/maintenance.md`, "The issue loop").
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
* `runs.py` of podcrates (in `.work/podcrates-agent/`, beside the watcher) follows the runs of a commit:
  `python3 -I .work/podcrates-agent/runs.py --repo PodUtils/podnet --until SHA --min-runs 5`.
* A cloud container can lack `ip` (iproute2) and the Python module `websockets`: `apt-get install -y iproute2` and
  `pip install --break-system-packages websockets` (the scripts run `python3 -I`, which reads no user site). The
  full suite takes about 90 minutes here; run it with `--no-fail-fast`, as `cargo test` stops at the first failed
  test binary.
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
| `#654` (COORD-107, takes of podcrates) | A `state:` line at each take. |
| `#710` (COORD-118, the take of `v0.83.0`) | podcrates reads it; then close it through the record. |
| `#711` (COORD-119, a temp directory and a temp file with a random name in `pod-core`) | A tag of podcrates with them (T-545). |
| `#656` (COORD-109, ghcr.io image: T-487) | The job `image` of the release of beta.19. |
| `#604` (COORD-098, relay holds a lost socket) | The relay fix (T-406, T-478). |
| `#413` (COORD-006) | T-161 and T-200 (`pod-relay`). |
