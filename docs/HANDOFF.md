# Handoff

The state of the session of 2026-11-11. Read this file first, then `CLAUDE.md`,
`TODO/PROGRESS.md`, `docs/maintenance.md` and `docs/decisions.md`. Remove this
file when each step below is done.

## Where the code is

* All work is on `main` of `PodUtils/podnet`. A local worktree
  `/home/user/wt-t058` (branch `wt-t058`, not pushed) follows `main`; it
  holds no work of its own. The toolchain is Rust 1.99.1 (`rustup default
  1.99.1`; the image has 1.97 as its default).
* podnet takes `pod-todo`, `pod-core`, `pod-tls`, `pod-proxy`, `pod-dns`,
  `pod-services`, `pod-ws`, `pod-quic` and `pod-hold` of
  `PodUtils/podcrates` by the git tag `v0.15.0` (T-137, T-232).
* The newest release is `v0.4.0-beta.3` at 649c8e3 (T-235; release run
  42949492119, release-proofs 42952184194). The work after 649c8e3 (T-061
  partial, T-097 partial) is in no release.

## Work of this session

T-058 (the combinators), T-059 (the registry of networks), T-060
(`--egress SPEC`; `serve` and `git serve` take `--ingress` in place of
`--via`), T-062 (the roads), T-084 (one access layer), T-096 (the group
with a kill switch), T-097 (partial: the generic health), T-229 (the
measure with the runner), T-230 (the object `hold`), T-231 item 1, T-232
(the take of `pod-hold`), T-233, T-234 and T-236 (defects).

## Next steps

1. **Live steps that failed once each** (no code of them changed): "SSH
   through the reverse relay" at 520b307 (a reset session, then a hang of
   5 minutes), and "warp up with its TCP listener refused" at 05bcf44 (no
   JSON line on stdout; the step does not print `up97.err`). Read the next
   live runs; open an entry when one fails again. T-236 fixed the third
   (git over the relay at c0180cc).
2. **T-137:** podcrates v0.16.0 (PodUtils/podcrates#57, COORD-032) carries
   `pod_hold::connect` and `pod_hold::packet`; the move of the tag is
   optional, and the patch that makes `connect.rs` and `packet.rs` glue
   comes with a later release. Move the tag after the beta.3 release (all
   `pod-*` dependencies, `cargo todo remap Cargo.lock`, the fuzz lock), then
   write a COORD issue (to podcrates). T-234 fixed the defect of
   `parse_tcp_syn` that COORD-032 names. T-231 item 2 (the rules on
   loopback) waits for the adoption patch.
3. The work order of `TODO/PROGRESS.md`: M6, the router (T-098 to T-101,
   T-177, T-180) on the combinators, the group and the health of
   `podnet-core`. T-061 needs `src/warp/proxy.rs`, which waits for
   `pod-warp`.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #413 (COORD-006) | podcrates: the asks for `pod-relay`, `pod-dns`, `pod-proxy`, `pod-services` and `pod-warp`. Close it when each ask is in a release that podnet took. |
| #425 (COORD-016, to podrunner) | podrunner reads it. Close it when podrunner answers or closes its COORD-001. |

## Tools of the session

* `.work/` holds local tools (not in git): the org watcher
  (`.work/podcrates-agent/`, see `CLAUDE.md`).
* A worktree build needs its own `CARGO_TARGET_DIR` and
  `CARGO_PROFILE_DEV_DEBUG=0`. The disk filled once this session: remove
  `target/debug/incremental` and the target of a worktree first.
* The mirror `forge.web.pkgforge.dev` answers 403 at times; the GitHub MCP
  tools (`actions_list`, `get_job_logs`) read the same data.
