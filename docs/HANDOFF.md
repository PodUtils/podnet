# Handoff

The state at 2026-11-23 (UTC), at `HEAD` `1325d5bb`. Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`,
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

* All finished work is on `main` of `PodUtils/podnet` (`HEAD` `1325d5bb`). The toolchain is Rust 1.99.1
  (`rustup default 1.99.1`; add `rustfmt`, `clippy`, and the target `x86_64-unknown-linux-musl` in a new container).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.82.0` (`51c09390`, T-506), adding `pod-cover`,
  `pod-harness`, `pod-offline`, `pod-shape`, optional `pod-sidecar` (feature `sidecar`), `pod-link` (`ntp`, `smtp`,
  `imap`), `pod-turn` (RFC 6062), and `pod-warp` (HTTP/3 control-stream fix), with transitive `pod-beneath` and
  `pod-hostprobe` in `Cargo.lock`. `url` is `2.5.9` (`RUSTSEC-2026-0592`), and `deny.toml` ignores `RUSTSEC-2026-0542`
  (`75f69f51`). The installer core in `scripts/install/SOURCES` keeps `core.sh` (`9238e041...`) and `core-net.sh`
  (`85bbaf3d...`) at `v0.82.0`.
* The newest release is `v0.4.0-beta.17` (T-481).
* The record is `TODO/`. `cargo todo check` agrees: 508 entries: 14 open, 14 partial, 24 blocked, 456 done.
* Remote branches on `origin` (`git ls-remote --heads origin`): `main` and the four open Dependabot branches (`#680`,
  `#681`, `#682`, `#684`; `#683` is closed). The former stale branch `claude/quirky-mccarthy-ad1v62` is deleted on
  `origin`.

## CI and runs on the head

* **`4df28952`** (`Take podcrates v0.82.0, consolidate deferred features, and apply hard-problems fixes`):
  `issue-sync` `45141846884` passed (closed `#484`, `#485`, `#491`, `#493` to `#496`, `#499` to `#501`, `#672`, `#677`,
  and `#679` on GitHub); `secrets` `45141846144` passed; `deny` `45141846402` failed (`RUSTSEC-2026-0542`); `live`
  `45141846391` failed (`warp tun --default-route` with no default route); `ci` `45141846137` was cancelled.
* **`75f69f51`** (`ci: gate turn_tests on loopback, disable debug symbols in ci, ignore RUSTSEC-2026-0542`):
  `issue-sync` `45144945528`, `deny` `45144945139`, and `secrets` `45144945103` passed; `ci` `45144945248` was
  cancelled by `1325d5bb`; `live` `45144945104` was in progress at handoff.
* **`1325d5bb`** (`warp: allow front doors when an environment proxy is active without a default route`):
  `issue-sync` `45146227312`, `deny` `45146226566`, and `secrets` `45146226502` passed; `ci` `45146226769` is queued
  and `live` `45146226467` is pending.
* Prior baseline runs before today: `ci` `44871490037` passed on `9a9a0a9` (`v0.81.0`); `gate` `44782773447` passed
  on `3836971` (the full lane on the GitHub runner, with the case `errors` and the row `testers`); `live` `44713609259`
  passed on `3f14a38`. About one `live` run of six fails at the public relay or at WARP's MASQUE door (T-406, T-488,
  GitHub `#604`).
* Pushes on `2026-11-23` started workflows directly. In a cloud session whose push starts no workflow, request runs via
  `[gh action] requested` issues (`docs/maintenance.md`, "A run when a push starts no workflow"), respecting `ci`
  concurrency and requesting `issue-sync.yml -f dry_run=false` when the record changes GitHub states (otherwise daily
  `record-issues` fails, as in `45028265942` on `2026-11-22`).

## Consolidated work on `main` (2026-11-23)

* **Take `podcrates` `v0.82.0` (`4df28952`, T-506):** updated all manifests, `Cargo.lock`, `fuzz/Cargo.lock`,
  `scripts/install/SOURCES`, and `.github/workflows/issue-sync.yml`. Aligned `podnet offline pair` with 8-word PBKDF2
  codes and `Error::CodeExpired`, and updated `src/warp/h3/serve.rs` and `docs/quic.md` for the `v0.82.0` HTTP/3
  control-stream fix.
* **Deferred feature set (`T-365`, `T-366`, `T-369` to `T-373`, `T-376` to `T-378` done; `T-367` partial):**
  * NTPv4, SMTP, and IMAP carriers (`#484`, features `ntp`, `smtp`, `imap`, `full`).
  * TURN RFC 8656/6062 relay and NAT traversal fallback (`#485`, `tests/link_p2p*.rs`).
  * QUIC HTTP/3 server shape (`podnet warp h3`), qlog control stream, TUIC v5 road with bounded queue drop policy,
    Juicity road with 30-stream fallback, and connection-ID rotation (`#486` partial: MASQUE `QUIC-1` to `QUIC-5` remain).
  * Cover listener with TLS exporter-bound switch, cover scripts, and multiplexed cover (`#491`, `pod-cover`).
  * Record-layer shaping: padding profiles, entropy tail, measured first flight, pacing, and `doctor --shape` (`#493`,
    `pod-shape`).
  * Application-layer shaping: deferred cache, TOML traffic generator, and HTTP payload carrier (`#494`, `src/defer/`).
  * Optional packet sidecar (`#495`, feature `sidecar`, `pod-sidecar`, off by default).
  * End-to-end opaque destinations, rekey epoch and ceiling checks, and hop-exposure table (`#496`, `pod-e2e`,
    `SECURITY.md`).
  * Offline carriers (`serial:`, `hf:`) and stores (`spool`, `discover`, `bundle`, `capture`, `pair`, `replica`)
    (`#499`, `pod-offline`).
  * Host runtime: `host probe` and `stat`, crash logs, deployment egress policy enforced before direct dials, `init` TLS
    enrolment with rollback, `forge clone`, `ci check`, and derived status page (`#500`).
  * Verification harness: `podnet harness`, per-road captures, relocatable archives, `scripts/check-harness.py`,
    `scripts/check-claims.py`, and exit-code contract (`#501`, `pod-harness`).
* **Hard-problems research and fixes (`docs/research/podnet/hard-problems-results.md`):**
  * **H-02 (T-507):** `pod --backend seccomp` refuses `listen(2)` on `Sock::Kernel` with `EPERM`, verified by 3 cases
    and `/proc/<pid>/fd` inode checks in `tests/pod_seccomp/kernel_listen.rs`.
  * **H-04 (T-508):** `pod --backend netns` reassembles IPv4 UDP fragments up to `65,507`-byte payloads
    (`NETNS_UDP_LIMIT = 65_507`), verified by 17 unit tests (including oldest-set eviction) and
    `tests/pod_seccomp/netns_udp.rs`.
  * **H-05 (T-509):** `net shim` generates per-run `PODNET_VNET_PREFIX` (capped at 59 bytes), validates invalid and
    non-UTF-8 prefixes before build with exit 64, copies full 107-byte abstract names, prints ignored-prefix warnings
    unconditionally to stderr, and writes notes to stderr so stdout holds only exports.
  * **H-10:** `docs/warp.md` corrects the TCP probe description to reflect live TCP carriage over MASQUE.
  * **H-11 and H-19:** `scripts/live-throughput.sh` captures non-200 status, `cf-ray`, and error bodies instead of
    dropping them with `curl -f`; `tests/transfer_relay.rs` lowers the `1 MiB` cap test move threshold to `160 KiB`
    (`<= 191 KiB`, T-476).
* **CI and WARP proxy fixes (`75f69f51`, `1325d5bb`):**
  * `75f69f51`: gated `src/networks/turn_tests.rs` on loopback, disabled debug symbols in `.github/workflows/ci.yml`,
    and ignored `RUSTSEC-2026-0542` in `deny.toml`.
  * `1325d5bb`: allowed WARP front doors when an environment proxy is active without a default route (`src/warp/h3.rs`,
    `src/warp/mod.rs`, `src/warp/session.rs`).

## Next steps

`docs/research/podnet/hard-problems.md` (2026-11-20) and `docs/research/podnet/hard-problems-results.md` hold the
findings and follow-ups for `H-01` to `H-32`.

1. Read `ci` (`45146226769`) and `live` (`45146226467`) on `1325d5bb` (plus `live` `45144945104` on `75f69f51`). Fix any
   failure first, then append a `state:` line to COORD-107 (`#654`) once `ci` and `live` pass on `1325d5bb`.
2. Start the organization watcher (the pinned commit and command are in `CLAUDE.md`, "Talk with the other sessions of
   PodUtils"):

   ```text
   python3 -I .work/podcrates-agent/watch.py podnet --state .work/agent
   ```

   Take each COORD issue `(to podnet)` of podrunner first. Run `sh scripts/agent/podcrates.sh` at start and at each new
   tag after `v0.82.0`; take the tag (T-137), and write the take into COORD-107 (`#654`) as a `state:` line with its CI
   run.
3. Cut `v0.4.0-beta.18`. Before its dry run (`docs/maintenance.md`, Release, step 0):
   * Run the black-box QA pass: give `scripts/agent/qa-pass.md` as it is to a new agent with the binary of the commit,
     the documents, and `podlab` only (not a session that read the source). Triage each finding into an entry.
   * Run `gate.yml` on the commit.

   Then run the release steps. The release must prove T-487: the job `image` passes (the attestation check and the
   pull-back of each architecture image), the release notes include "The image, by digest", `release-proofs.yml` passes,
   and the hand-off COORD issue names the digest of the manifest list. Then close COORD-109 (`#656`) through the record.
4. Finish remaining `T-473` (P2, partial) shared-code adoptions: the CONNECT client of `crates/podnet-core/src/connect.rs`
   (the `--connect-proxy` path of `ssh-relay`; to `pod-proxy`, a change of tested behaviour), the HTTP head readers of
   `crates/podnet-netns/src/tailscale.rs` and `src/router/control.rs` (`pod_proxy::http`), the request parser of
   `crates/podnet-core/src/util/http.rs`, and the groups and lead of the help (`pod-manual`). `Backoff` in
   `crates/podnet-serve/src/relay/supervise.rs` stays (its tests give it short waits). Note: `pod_core::clock`,
   `pod_core::backoff` (`src/cli/cmd/warp/run.rs`, `src/cli/cmd/warp/tun.rs`), and `pod_core::files::replace`
   (`crates/podnet-netns/src/report.rs`, `src/warp/ssh_config.rs`) landed before `1a4781eb`.
5. Finish `T-494` (P2, open): take `pod-release` in place of `src/update.rs` and `src/update/cosign.rs`. Read
   `PodUtils/podcrates@T-880`, `PodUtils/podcrates@T-966`, and `PodUtils/podcrates@T-964` (Rekor v2) first. The ruling in
   `CLAUDE.md` (`SHA256SUMS`, certificate identity, and cosign signature checked; no Rekor inclusion proof) must hold, or
   the ruling changes.
6. Optional takes from `v0.80.0` onward: `pod-unit` for `src/cli/cmd/autostart.rs`, and `pod-control` for a status
   socket.
7. Follow up on `T-476` (P2, open): `tests/transfer_relay.rs` uses the `160 KiB` (`<= 191 KiB`) move threshold from
   `4df28952` (H-19); follow up on `pod-session` `resume::move_under(cap, window)` and 20-run parallel load checks.
8. Finish `T-492` step 3 (P3, partial): update `free_port` users in `tests/commands.rs` and `tests/warp_stand_in.rs` to
   take a port that the child reports where the command can report one.
9. P3 and blocked follow-ups:
   * `T-367` (`#486`, partial): finish the remaining MASQUE `QUIC-1` to `QUIC-5` items when unblocked.
   * `T-488` (open): sporadic HTTP 530 of quick tunnels; measure with `--ha-connections 1` and the paired
     early-vs-steady design using the H-11 body/`cf-ray` capture in `scripts/live-throughput.sh`.
   * `T-414` (partial): the WARP proxy ends on a failed health check (send evidence to podcrates); `T-415` (partial).
   * `T-406` (blocked) and `T-478` (open): read logs of a failed `live` step and the answer of podcrates on COORD-098
     (`#604`).
   * `#489` (`T-368`), `#497` (`T-374`), and `#498` (`T-375`): deferred and blocked (need outside resources per H-27);
     leave open and unlabelled (`docs/decisions.md`, 2026-11-16).
10. Open Dependabot PRs (`#680`, `#681`, `#682`, `#684`; note `#684` failed `ci` `45142689882` and `deny` `45142689681`):
    apply each judged update as an own commit on `main` (checks green) and close the PR with a link to the commit
    (`CLAUDE.md`).

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
| `#654` (COORD-107, takes of podcrates) | Amended on 2026-11-23T08:57:21Z with `v0.82.0` (`4df28952`). Add a `state:` line after `1325d5bb` `ci` and `live` pass, and on future tags. |
| `#656` (COORD-109, ghcr.io image: T-487) | Next beta (`v0.4.0-beta.18`) proves attestation, pull-back, and digests. |
| `#651` (COORD-104, shared code copies) | Remaining T-473 items and `pod-release` (T-494), `pod-detach`, `pod_core::lock`. |
| `#604` (COORD-098, relay holds lost socket) | Relay server fix (T-406, T-478). |
| `#413` (COORD-006) | T-161 and T-200 (`pod-relay`). |

Closed on GitHub via `issue-sync` `45141846884` on `4df28952`: `#672`, `#677` (`COORD-112`), `#679`, `#484`, `#485`,
`#491`, `#493` to `#496`, and `#499` to `#501`.
The coordinator of podcrates closed `#437`, `#585`, `#592`, `#606`, and `#608` on 2026-11-23. The record agrees
(`34015bb`).

The other open issues in `TODO/issues.md` are `#89`, `#140`, `#149`, `#151`, `#161`, `#222`, `#358`, `#360`, `#375`,
`#397`, `#407`, `#435`, `#461`, `#469`, `#471`, `#486`, `#489`, `#497`, `#498`, `#507`, and `#620`. Open
Dependabot PRs are `#680`, `#681`, `#682`, and `#684`.

## Waiting on the owner

* The first black-box QA pass before `v0.4.0-beta.18` (`scripts/agent/qa-pass.md`): a session with the binary, the
  documents, and `podlab` only.
