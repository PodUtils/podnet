# Handoff

The state at 2026-11-13. Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md` and
`docs/decisions.md`. Remove this file when the lead session has no unfinished step.

## Where the code is

* All work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (`rustup default 1.99.1`).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.36.0` (T-137). The next take is `pod-warp`: its
  patch waits for the COORD issue of the freeze with the next tag (COORD-063, GitHub #520).
* The newest release is `v0.4.0-beta.9` (T-331, published on 2026-11-13).
* The record is `TODO/`. `cargo todo check` agrees. `TODO/PROGRESS.md` has the work order.

## Next steps

1. Finish T-331 (the release steps of `docs/maintenance.md`, "Release").
2. At each tag of podcrates: `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is at the tag (T-137).
3. Go on with the work order of `TODO/PROGRESS.md`. Run two worktree agents at a time at most. Merge each as before:
   rebase on `main`, take `--ours` for `TODO/INDEX.md` and `TODO/PROGRESS.md` and run `cargo todo set`, cite the
   rebased commit, run `gen-llms.py`, run `check.sh`, push. Push in batches.
4. Close the GitHub issues whose entries are done, with the commits and the CI runs (`docs/maintenance.md`, "The issue
   loop"). Issues #484 to #501 are deferred by the owner: leave them open.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #413 (COORD-006) | podcrates: the asks for `pod-relay` (T-161, T-200), `pod-warp` (T-163) and the rest of its list. |
| #425 (COORD-016, to podrunner) | podrunner reads it. T-231 item 2 waits for `pod-hold`. |
| #437, #439, #440 (COORD-019, 021, 022) | podcrates and podrunner read the proposals of the donor review. |
| #448 (COORD-030) | podcrates: the `taken` line of the watches of `pod-services` (0b945a3). |
| #460 (COORD-041) | podcrates: the check of the pinggy URL rule (its T-225). |
| #517 (COORD-060) | podswarm reads the answer. N-1 is done (T-327). N-2 waits for #518. |
| #518 (COORD-061) | podcrates: the port rules in the rules reader (its entry 475). podnet takes them with the tag. |
| #520 (COORD-063) | podcrates: the COORD issue of the freeze of `src/warp/`. |

## Tools of the session

* `.work/` holds local tools (not in git): the org watcher (`.work/podcrates-agent/`, see `CLAUDE.md`) and the brief of
  the worktree agents (`.work/fix-brief.md`).
* Each worktree agent uses its own `CARGO_TARGET_DIR` and a `TMPDIR` of mode 0755 outside `/tmp/claude-0` (the shim
  tests switch the uid). Remove the target of an agent after its merge: the disk fills.
* Each push queues a run of `ci` and cancels a run of `live`.
* The mirror `forge.web.pkgforge.dev` answers 403 at times. Try again, or read the same data with the GitHub MCP tools
  (`actions_list`, `get_job_logs`).
