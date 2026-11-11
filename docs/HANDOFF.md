# Handoff

The state of the session of 2026-11-11. Read this file first, then `CLAUDE.md`,
`TODO/PROGRESS.md`, `docs/maintenance.md` and `docs/decisions.md`. Remove this
file when each step below is done.

## Where the code is

* All work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1
  (`rustup default 1.99.1`; the image has 1.97 as its default).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.22.0`
  (T-137), with the watches of `pod-services` (0b945a3, #448) and `pod-wg`
  (089e1c9, #447).
* The newest release is `v0.4.0-beta.3` at 649c8e3. The work after it is in no
  release.

## Work of this session after v0.4.0-beta.3

Done: T-100 (`podnet route --config`), T-177 (`route --check --to`), T-174 (the
veth wire), T-216 and T-217 (`doctor --json` schema 1, `--quick`, `lanes`),
T-218 (the connector profile, `docs/protocol.md` section 10), T-222 (`--help
--json`, `llms.txt`, `llms-full.txt`), T-239, T-240, T-241, T-243, T-118 (the
research on NetBird, Firezone and ZeroTier; it opened T-246 and T-247).

Partial: T-097 (thresholds and `route --status`), T-098 (the DNS door of a
door), T-099 (UDP forwards, the DNS door, `podnet expose`), T-178 (group
policies), T-221 (`podnet bench`), T-242 (the lint of shadowed rules).

Test fixes: the shim datagram test takes a port of its run and reaps its
server; the claim test makes its stale socket with `mknod`; each probe child of
`pod doctor` closes its inherited descriptors; the veth test and the register
tests skip on the CI jobs that cannot run them.

## Next steps (the night queue of 2026-11-11, unattended)

Run two worktree agents at a time at most. Merge each as before: rebase on `main`, take `--ours` for
`TODO/INDEX.md` and `TODO/PROGRESS.md` and run `cargo todo set`, cite the rebased commit, run
`gen-llms.py`, run `check.sh`, push. Push in batches: each push replaces the queued run of `live`.

1. **The cut of v0.4.0-beta.4 (T-245):** when `ci` and `live` pass on the head, follow steps 4 to 6 of
   "Release" in `docs/maintenance.md`; then the Done section of T-245. Map #447, #452 and #453 as closed
   in `TODO/issues.md` and `TODO/github.txt` in the same batch.
2. **T-254 (housekeeping):** an agent removes each mention of cfrs but one footnote in README.md, and the
   narrative history of the documents. It leaves `docs/releases/`, `docs/reports/`, the record and the
   frozen files. A second pass may cover `docs/research/`. Each cfrs mention in a frozen file goes to
   podcrates as a COORD issue.
3. **T-249 (an agent):** the netns router onto `pod-netstack`. Then a COORD issue with the commit for
   podcrates T-170 step 2 (PodUtils/podnet#451).
4. **T-250:** the first `ci` run with the step "Netns tests as root" green. Then a COORD issue with the
   run for podcrates T-201, and T-250 done.
5. **Then, one at a time beside one other:** T-178 (the byte meter of the doors; finish), T-179 (the
   direction split, an option, off by default), T-165 (the CLI of the SSH forward-relay leg), T-253 (a live
   probe of the hosted services), T-248 (the guest-wire router, after T-249).
6. **podcrates:** at each new tag, `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is
   at the tag (T-137). #448 waits for the `taken` line of the watches of `pod-services`.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #413 (COORD-006) | podcrates: the asks for `pod-relay`, `pod-dns`, `pod-proxy`, `pod-services` and `pod-warp`. |
| #425 (COORD-016, to podrunner) | podrunner reads it. |
| #444 (COORD-026) | podcrates: the umask window of `pod-broker`. |
| #449 (COORD-031) | podcrates: the freeze at 7c5f87e of the files of `pod-relay`, `pod-warp`, `pod-cloudflare`; T-057 and T-242 in its T-028; T-244 in its T-029. |
| #450 (COORD-032) | podcrates: an owner for the file-transfer of magic-wormhole; no hand-off for `pod-session`, `pod-e2e`, `pod-identity`. |
| #451 (COORD-033) | podnet: T-250, then T-249; then a COORD issue with the commits. podcrates accepted #449 to #451 in its #74. |
| #448 (COORD-030) | podcrates: the `taken` line of the watches of `pod-services` (0b945a3). |

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
