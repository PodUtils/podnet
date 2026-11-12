# Handoff

The state of the session of 2026-11-11. Read this file first, then `CLAUDE.md`,
`TODO/PROGRESS.md`, `docs/maintenance.md` and `docs/decisions.md`. Remove this
file when each step below is done.

## Where the code is

* All work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1
  (`rustup default 1.99.1`; the image has 1.97 as its default).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.33.0` (`pod-netstack` closes the gaps of #457 and #464)
  (T-137), with the watches of `pod-services` (0b945a3, #448) and `pod-wg`
  (089e1c9, #447).
* The newest release is `v0.4.0-beta.5` at 8f53288 (T-256; release run 43122020833,
  release-proofs 43123633932). The work after it is in no release.

## Work of this session after v0.4.0-beta.3

Done: T-100 (`podnet route --config`), T-177 (`route --check --to`), T-174 (the
veth wire), T-216 and T-217 (`doctor --json` schema 1, `--quick`, `lanes`),
T-218 (the connector profile, `docs/protocol.md` section 10), T-222 (`--help
--json`, `llms.txt`, `llms-full.txt`), T-239, T-240, T-241, T-243, T-118 (the
research on NetBird, Firezone and ZeroTier; it opened T-246 and T-247).

Partial: T-097 (thresholds, `route --status`, the metrics rows, the egress
check; the WARP parts wait for `pod-warp`), T-099 (UDP forwards, the DNS door,
`podnet expose`; `warp forward` as an alias waits for WARP as a network), T-178 (group
policies), T-221 (`podnet bench`), T-242 (the lint of shadowed rules).

Test fixes: the shim datagram test takes a port of its run and reaps its
server; the claim test makes its stale socket with `mknod`; each probe child of
`pod doctor` closes its inherited descriptors; the veth test and the register
tests skip on the CI jobs that cannot run them.

## Next steps

Run two worktree agents at a time at most. Merge each as before: rebase on `main`, take `--ours` for
`TODO/INDEX.md` and `TODO/PROGRESS.md` and run `cargo todo set`, cite the rebased commit, run
`gen-llms.py`, run `check.sh`, push. Push in batches.

1. **T-249:** done on the local branch `worktree-agent-a35ab25d8cf7315bb`, not merged. After the merge, write
   the commit in #451 (COORD issue to podcrates, for T-170 step 2).
2. **Replies:** sent as #456. `crates/podnet-netns/src/netns/veth.rs` takes fixes only until the take of
   `pod-hold` with the wire (podcrates T-201).
3. **T-248** (the guest-wire router): partial, on a local worktree branch, not merged. See its Done section.
4. **podcrates:** at each new tag, `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is at
   the tag (T-137). #448 waits for the `taken` line of the watches of `pod-services`; #455 is its T-224.
5. **The next beta:** when a batch after 8f53288 is green.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #413 (COORD-006) | podcrates: the asks for `pod-relay`, `pod-dns`, `pod-proxy`, `pod-services` and `pod-warp`. |
| #425 (COORD-016, to podrunner) | podrunner reads it. |
| #444 (COORD-026) | podcrates: the umask window of `pod-broker`. |
| #451 (COORD-033) | podnet: T-250, then T-249; then a COORD issue with the commits. podcrates accepted #449 to #451 in its #74. |
| #457 (COORD-039) | podcrates: three gaps of `pod-netstack` (close of a listener, close of a stream, the accept pool) for T-249. |
| #456 (COORD-038) | podcrates and podrunner: the take of v0.25.0, and T-255. Close it when both read it. |
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
