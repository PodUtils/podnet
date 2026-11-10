# Handoff

The state at the pause of 2026-11-10 (the owner paused each session of
PodUtils). Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`,
`docs/maintenance.md` and `docs/decisions.md`. Remove this file when each
step below is done.

## Where the code is

* All work is on `main` of `PodUtils/podnet`. No local worktree, branch or
  stash holds work. The workspace has the crates `podnet` (root),
  `podnet-core`, `podnet-netns`, `podnet-serve`, `podnet-vnet` and
  `podnet-todo`. The toolchain is Rust 1.99.1.
* podnet takes `pod-todo`, `pod-core`, `pod-tls`, `pod-proxy`, `pod-dns`,
  `pod-services`, `pod-ws` and `pod-quic` of `PodUtils/podcrates` by the
  git tag `v0.14.0` (T-137). podnet uses noq through `pod-quic`; no quinn
  is in the tree.
* The newest release is `v0.4.0-beta.2` at 5465007 (T-227; release run
  42833480545, release-proofs 42836050242). The work after 5465007 (T-055,
  T-056, T-088 to T-094, T-109 to T-113, T-228, the take of `pod-quic`) is
  in no release: it is the batch of `v0.4.0-beta.3`.
* The record holds 232 entries (`cargo todo counts`). `cargo todo check`
  agrees at the head of `main`.

## The CI at the pause

* `main` at 8383095 is green: ci 42851837015 and live 42851836974 (each
  step, with the new steps of the file server and git). T-091 and T-094
  are done; #423 (two failed live runs) is closed.
* The runs at the head of `main` (the hand-off commits) are record and
  document changes only. **Step 1:** read the runs of `ci.yml` and
  `live.yml` at the head of `main`
  (`mcp__github__actions_list`, `list_workflow_runs`). If one fails, read
  the log (`mcp__github__get_job_logs`), fix, push, and wait again.

## Partial entries and their next step

| Entry | Next step |
| --- | --- |
| T-137 | Take each crate of podcrates at its release (the hand-off `adoption/podnet/CRATE.patch` and `.run`), then write a COORD issue (to podcrates). Next: `pod-hold` (podcrates T-166), `pod-session` and `pod-wormhole` (podcrates v0.15.0), `pod-relay`, `pod-warp`, `pod-cloudflare`. |
| T-169 | Item 2 (`tunnel --masque-endpoint`) waits for `pod-warp` and `pod-cloudflare` (T-050, T-052). The ask is in #413 (item 10). |
| T-173 | Item 1 (`PODNET_FOREIGN_IDENTITY=allow`) waits for `pod-warp` (T-050). The ask is in #413 (item 9). |

## The new entries of 2026-11-10

* T-229, T-230, T-231 (`blocked`): the asks of podvm (PodUtils/podvm#228)
  for podrunner: the measure of `podnet pod` with the official runner, the
  `hold` object of `pod doctor --json`, a refusal with no hold and rules on
  loopback. Each waits for the owner's decisions DR-1 and DR-2 of the
  podrunner design review (PodUtils/podcrates#47, #51). Next step: when the
  owner rules, set them `open` and do T-229 first.
* T-232 (`open`): the hand-off of the generic hold to `pod-hold`. podnet
  agreed that the time has come (#422, `docs/decisions.md`, 2026-11-10).
  podcrates imports it now as its T-166, from `main` at 9e8d177 or later.
  From that import, the generic core of `crates/podnet-netns` (the seccomp
  notification loop, the read of the child memory, the descriptor table
  and the reply, the netns hold of a `connect`) takes fixes only; run
  `sh scripts/agent/podcrates.sh` to see the files. Next step: when
  podcrates releases `pod-hold` with `adoption/podnet/pod-hold.patch`, take
  it (T-137) and set T-232 `done`.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #413 (COORD-006) | podcrates: the asks for `pod-relay`, `pod-dns`, `pod-proxy`, `pod-services` and `pod-warp` (T-083, T-161 to T-163, T-166, T-169, T-172, T-173, T-175, T-182, T-200). Close it when each ask is in a release that podnet took. |
| #422 (COORD-014) | podvm: it must read the answer. podcrates read it (state line of 2026-11-10 16:00 UTC). Close it when podvm has read it. |

## The order of work after the pause

1. Step 1 above (the CI at the head).
2. Each take of podcrates that a release brings (T-137 first, as
   `CLAUDE.md` says).
3. The work order of `TODO/PROGRESS.md`. Most open entries of M4, M6 and
   M8 chain to T-057 to T-062 (M4); T-057 waits for `pod-warp`. The call of
   2026-11-10 (`docs/decisions.md`, Start conditions) lets podnet's own new
   code start before a start condition that waits for a crate, when it
   needs no frozen file. T-058 (the combinators) and T-059 (the registry)
   can start on that call.
4. A beta (`v0.4.0-beta.3`) when a batch is green (`docs/maintenance.md`,
   Release).

## Tools of the session

* `.work/` holds local tools (not in git): the org watcher
  (`.work/podcrates-agent/`, see `CLAUDE.md`), `.work/fix-brief.md` (the
  brief of a worktree agent) and `.work/wait-ci.sh SHA` (waits for ci,
  live and secrets at a commit). A new session gets the watcher again
  from `CLAUDE.md`.
* A worktree agent needs its own `CARGO_TARGET_DIR`, a `TMPDIR` outside
  the worktree, and `CARGO_PROFILE_DEV_DEBUG=0`. The disk fills with three
  debug targets: remove the target of a merged worktree.
* A push cancels the running `live` run of the push before. Hold commits
  on a local branch while a release or a live run must finish.
