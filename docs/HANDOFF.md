# Handoff

The state at 2026-11-23. Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md`, and
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
  `rustfmt`, `clippy`, and the target `x86_64-unknown-linux-musl` in a new container).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.82.0` (`51c09390`, T-506), including `pod-cover`,
  `pod-harness`, `pod-offline`, `pod-shape`, optional `pod-sidecar` (feature `sidecar`), `pod-link` (`ntp`, `smtp`,
  `imap`), `pod-turn` (RFC 6062), `pod-warp` (HTTP/3 control-stream fix), and `pod-beneath`. `url` is `2.5.9`
  (`RUSTSEC-2026-0592`). The installer core in `scripts/install/` keeps the same SHA-256 hashes at `v0.82.0`.
* The newest release is `v0.4.0-beta.17` (T-481).
* The record is `TODO/`. `cargo todo check` passes on `508` entries (`TODO/INDEX.md`).

## Consolidated work on `main` (2026-11-23)

* **Take `podcrates` `v0.82.0` (T-506):** updated all manifests, `Cargo.lock`, `fuzz/Cargo.lock`, `scripts/install/SOURCES`,
  and `.github/workflows/issue-sync.yml`. Aligned `podnet offline pair` with 8-word PBKDF2 codes and `Error::CodeExpired`,
  and updated `src/warp/h3/serve.rs` and `docs/quic.md` for the `v0.82.0` HTTP/3 control-stream fix.
* **Deferred feature set (`T-365`, `T-366`, `T-369` to `T-373`, `T-376` to `T-378` done; `T-367` partial):**
  * NTPv4, SMTP, and IMAP carriers (`#484`, features `ntp`/`smtp`/`imap`/`full`).
  * TURN RFC 8656/6062 relay and NAT traversal fallback (`#485`, `tests/link_p2p*.rs`).
  * QUIC HTTP/3 server shape (`podnet warp h3`), qlog control stream, TUIC v5 road with bounded queue drop policy,
    Juicity road with 30-stream fallback, and connection-ID rotation (`#486` partial: MASQUE `QUIC-1` to `QUIC-5` remain).
  * Cover listener with TLS exporter-bound switch, cover scripts, and multiplexed cover (`#491`, `pod-cover`).
  * Record-layer shaping: padding profiles, entropy tail, measured first flight, pacing, and `doctor --shape` (`#493`, `pod-shape`).
  * Application-layer shaping: deferred cache, TOML traffic generator, and HTTP payload carrier (`#494`, `src/defer/`).
  * Optional packet sidecar (`#495`, feature `sidecar`, `pod-sidecar`, off by default).
  * End-to-end opaque destinations, rekey epoch/ceiling checks, and hop-exposure table (`#496`, `pod-e2e`, `SECURITY.md`).
  * Offline carriers (`serial:`, `hf:`) and stores (`spool`, `discover`, `bundle`, `capture`, `pair`, `replica`) (`#499`, `pod-offline`).
  * Host runtime: `host probe`/`stat`, crash logs, deployment egress policy enforced before direct dials, `init` TLS
    enrolment with rollback, `forge clone`, `ci check`, and derived status page (`#500`).
  * Verification harness: `podnet harness`, per-road captures, relocatable archives, `scripts/check-harness.py`,
    `scripts/check-claims.py`, and exit-code contract (`#501`, `pod-harness`).
* **Hard-problems research and fixes (`docs/research/podnet/hard-problems-results.md`):**
  * **H-02 (T-507):** `pod --backend seccomp` refuses `listen(2)` on `Sock::Kernel` with `EPERM`, verified by 3 cases
    and `/proc/<pid>/fd` inode checks in `tests/pod_seccomp/kernel_listen.rs`.
  * **H-04 (T-508):** `pod --backend netns` reassembles IPv4 UDP fragments up to 65,507-byte payloads
    (`NETNS_UDP_LIMIT = 65_507`), verified by 17 unit tests (including oldest-set eviction) and `tests/pod_seccomp/netns_udp.rs`.
  * **H-05 (T-509):** `net shim` generates per-run `PODNET_VNET_PREFIX` (capped at 59 bytes), validates invalid and
    non-UTF-8 prefixes before build with exit 64, copies full 107-byte abstract names, prints ignored-prefix warnings
    unconditionally to stderr, and writes notes to stderr so stdout holds only exports.
  * **H-10:** `docs/warp.md` corrects the TCP probe description to reflect live TCP carriage over MASQUE.
  * **H-11 & H-19:** `scripts/live-throughput.sh` captures non-200 status, `cf-ray`, and error bodies instead of dropping
    them with `curl -f`; `tests/transfer_relay.rs` lowers the 1 MiB cap test move threshold to 160 KiB (`<= 191 KiB`, T-476).

## Next steps

1. Push `main` and verify `ci.yml`, `gate.yml`, `live.yml`, and `issue-sync.yml` (to close `#672`, `#677`, `#679`,
   `#484`, `#485`, `#491`, `#493`, `#494`, `#495`, `#496`, `#499`, `#500`, `#501` on GitHub).
2. Update COORD-107 (`#654`) with a `state:` line for the `v0.82.0` take and CI run.
3. Cut `v0.4.0-beta.18` after the black-box QA pass (`scripts/agent/qa-pass.md`) and green `gate.yml`, proving T-487
   (ghcr.io attestation, pull-back check, manifest digest) and closing COORD-109 (`#656`).
4. Finish remaining `T-367` (`#486`) MASQUE `QUIC-1` to `QUIC-5` items when unblocked.
5. Finish remaining `T-473` shared-code adoptions (`connect.rs`, `tailscale.rs`, `control.rs`, `util/http.rs`, `pod-manual` groups) and `T-494` (`pod-release`).
6. Follow up on `T-476` (`pod-session` `move_under(cap, window)`), `T-488` (paired H-11 530 test), `T-492` step 3,
   and blocked deferred issues (`T-368` `#489`, `T-374` `#497`, `T-375` `#498`).

## How to work here

* Build with `TMPDIR=/tmp CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
* After a change of a document or help text, run `cargo test --bin podnet write_the_llms_files -- --ignored`.
* Run the local gate before pushing: `TMPDIR=/tmp sh scripts/agent/check.sh`, plus
  `cargo clippy --workspace --all-targets --features full -- -D warnings`,
  `cargo test --features full --test sidecar`, `python3 -I scripts/check-harness.py --self-test`, and
  `python3 -I scripts/check-claims.py`.

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

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #654 (COORD-107, takes of podcrates) | A `state:` line for `v0.82.0` after CI passes. |
| #656 (COORD-109, ghcr.io image: T-487) | Next beta (`v0.4.0-beta.18`) proves attestation, pull-back, and digests. |
| #651 (COORD-104, shared code copies) | Remaining T-473 items and `pod-release` (T-494). |
| #604 (COORD-098, relay holds lost socket) | Relay server fix (T-406, T-478). |
| #608 (COORD-101, `pod-tls` CA file) | Answer from podcrates (T-421 done in podnet). |
| #585 (COORD-091), #592 (COORD-095) | T-358 release gate steps 2 and 3. |
| #413 (COORD-006) | T-161 and T-200 (`pod-relay`). |
| #437 (COORD-019) | Donor proposals open in podcrates. |
