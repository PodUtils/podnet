# Handoff

The state of the session of 2026-11-11. Read this file first, then `CLAUDE.md`,
`TODO/PROGRESS.md`, `docs/maintenance.md` and `docs/decisions.md`. Remove this
file when each step below is done.

## Where the code is

* All work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1
  (`rustup default 1.99.1`; the image has 1.97 as its default).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.21.0`
  (T-137), with the watches of `pod-services` (0b945a3, COORD-030 #448).
* The newest release is `v0.4.0-beta.3` at 649c8e3. The work after it is in no
  release.

## Work of this session after v0.4.0-beta.3

Done: T-100 (`podnet route --config`), T-177 (`route --check --to`), T-174 (the
veth wire), T-216 and T-217 (`doctor --json` schema 1, `--quick`, `lanes`),
T-218 (the connector profile, `docs/protocol.md` section 10), T-222 (`--help
--json`, `llms.txt`, `llms-full.txt`), T-239, T-240, T-241, T-243.

Partial: T-097 (thresholds and `route --status`), T-098 (the DNS door of a
door), T-099 (UDP forwards, the DNS door, `podnet expose`), T-178 (group
policies), T-221 (`podnet bench`), T-242 (the lint of shadowed rules).

Test fixes: the shim datagram test takes a port of its run and reaps its
server; the claim test makes its stale socket with `mknod`; each probe child of
`pod doctor` closes its inherited descriptors; the veth test and the register
tests skip on the CI jobs that cannot run them.

## Next steps

1. **CI:** the runs after fc97ef9 were queued at 11:45 UTC. Read the first
   completed run at or after 0a2b2a8. Then close #431 (T-240), #432 (T-241),
   #369 (T-174) and #375 for T-177 only (T-178 and T-179 stay), with the
   commits and the run.
2. **T-236:** the code is in e6eda36. The entry waits for the live step "Git
   over SSH through the reverse relay" to pass. Read the next live run.
3. **Agents in flight** (worktrees under `.claude/worktrees/`): T-119 (the
   kernel mode of the router). T-126 waits for `pod-egress` (cb80aca). Merge
   each: rebase on `main`, take `--ours` for
   `TODO/INDEX.md` and `TODO/PROGRESS.md` and run `cargo todo set`, cite the
   rebased commit, run `gen-llms.py`, run `check.sh`, push.
4. **T-121** (the mwan export) needs the interface and the table of each WAN:
   do it after T-119.
5. **podcrates:** at each new tag, read `sh scripts/agent/podcrates.sh` and take the
   crates whose hand-off is at the tag (T-137).
6. **The next beta:** the batch above is green when CI is green. Name the
   speed limit of `warp --cc reno|cubic` on smoltcp 0.14 (PodUtils/podcrates#66,
   COORD-041: keep `CongestionControl::None`) in the notes.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #413 (COORD-006) | podcrates: the asks for `pod-relay`, `pod-dns`, `pod-proxy`, `pod-services` and `pod-warp`. |
| #425 (COORD-016, to podrunner) | podrunner reads it. |
| #444 (COORD-026) | podcrates: the umask window of `pod-broker`. |
| #448 (COORD-030) | podcrates: the `taken` line of the watches of `pod-services` (0b945a3). |
| #447 (COORD-029) | podcrates: the take of `pod-wg`; `src/warp/wg.rs` takes fixes only until then. |

## Tools of the session

* `.work/` holds local tools (not in git): the org watcher
  (`.work/podcrates-agent/`, see `CLAUDE.md`) and the brief of the worktree
  agents (`.work/fix-brief.md`).
* Run two worktree agents at a time at most. Each uses its own
  `CARGO_TARGET_DIR` and a `TMPDIR` of mode 0755 outside `/tmp/claude-0` (the
  shim tests switch the uid). Remove the target of an agent after its merge:
  the disk filled twice this session.
* Push in batches: each push queues a run of `ci` and cancels a run of `live`.
* The mirror `forge.web.pkgforge.dev` answers 403 at times; the GitHub MCP
  tools (`actions_list`, `get_job_logs`) read the same data.
