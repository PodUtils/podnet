# Handoff

The product work state at 2026-11-27 17:20 (UTC). Start a hosted checkout with
[TODO/RESUME.md](../TODO/RESUME.md). Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`). The order: a red run of podnet's CI; each ask of
podrunner (a COORD issue `(to podnet)`); the take of each tag of podcrates (T-137); podnet's own work in the order of
`TODO/PROGRESS.md`. The newest tag of podcrates is `v0.86.0` (taken in T-612). The open ask of podrunner,
PodUtils/podrunner#369, is fixed and shipped in `v0.4.0-beta.21` (T-619; COORD-139 #784 updated; COORD-143 #801 hands
off the release).

## Where the code is

* All finished work is on `main` (`3d04883` is the tag `v0.4.0-beta.21`). `ci` 46020756469 (all 13 jobs), `live`
  46020756523, `gate` 46020756813, `deny` 46020756439, `secrets` 46020756442, `issue-sync` 46020756972,
  `record-issues` 46025989718, dry run `release` 46027210279, publish `release` 46032920415 and `release-proofs`
  46038872762 (`proofs` and both reproducible `rebuild` jobs) all passed.
* The newest release is `v0.4.0-beta.21` (tag at `3d04883`; COORD-143 #801; manifest list digest
  `sha256:7b07bc89c549d48bfeeaf28794e5186a5d8cd6d4978a63bae62fbf7a411534a5`).
* Done in this session: all 13 findings of the QA pass of `v0.4.0-beta.21` (T-620 to T-632, GitHub #786 to #798 closed
  via `issue-sync`), `T-621` follow-up fix for full unread stdout pipes in `exit --stdio` (`3d04883`, `live`
  46015389946), `T-628` `need_loopback!()` guard (`86d1990`), `T-488` direct HTTP/2 and QUIC measurements plus
  `cloudflared` source check, `COORD-142` (#800) to podcrates for four shared-crate follow-ups, and the release of
  `v0.4.0-beta.21` (`COORD-143` #801, `COORD-139` #784 updated).
* Remaining entries are all blocked behind `PodUtils/podcrates`, sibling projects, outside services, or the owner:
  * **Blocked on `PodUtils/podcrates`:** T-137 (next tag after `v0.86.0`), T-114 and T-115 (COORD-130 #760, shared
    headscale harness), T-123 and T-124 (no `pod-ts` crate), T-161 and T-200 (COORD-006 #413, `pod-relay`), T-219 and
    T-220 (`pod-unit`, `pod-relay`), T-244 (COORD-136 #781, `pod-cloudflare` capture hook gaps), T-283 and T-284
    (`pod-link`), T-295 (COORD-140 #785, `pod-egress` Trojan accept parser), T-316 (QUIC stand-in leg on
    `pod-cloudflare`), T-406 and T-478 (COORD-098 #604, relay lost-socket reset), T-586 (`pod-link` p2p), T-614
    (COORD-134 #779, `pod-proxy` TLS hop for process `https://` proxy), plus COORD-135 (#780), COORD-137 (#782),
    COORD-138 (#783), COORD-141 (#799), COORD-142 (#800).
  * **Waiting for a `live` warning or outside resource:** T-414 and T-415 (partial: wait for next `live` warning to
    report `pod-warp` logs), T-488 (partial: direct measurements saw 0 HTTP 530s; 20-round WARP measurement waits to
    avoid unnecessary WARP device registration).
  * **Blocked on owner or external accounts/devices/captures:** T-001 (#89), T-002 (#140), T-003 (#149), T-004 (#151),
    T-005 (#161), T-006 (#222), T-007 (#224), T-044, T-210 (#407, paid WARP+ device), T-221 (cfb ask, live step),
    T-247 (NetBird account), T-367 (#486), T-368 (#489), T-374 (#497), T-375 (#498).

## The next steps, in order

1. **Each new tag of `PodUtils/podcrates` (> `v0.86.0`):** take it (T-137), close the COORD issues that it answers, and
   unblock the entries above (such as T-614, T-295, T-244, T-114/T-115, T-123/T-124, T-406/T-478).
2. **Each new ask of `podrunner` (or `cfb` / `githome`):** handle before other podnet work (`docs/decisions.md`,
   2026-11-18, podrunner first).
3. **Any red run of `ci` or `live`:** triage; capture T-414 or T-415 warnings if they fire in `live`.
4. **Next beta (`v0.4.0-beta.22`):** after a new green batch, run the black-box QA pass (`scripts/agent/qa-pass.md`)
   first (`docs/maintenance.md`, Release, step 0).

## Facts for the next session

* Start each worktree agent with the shell in the repository root (`/workspace/podnet` on Claude Cloud;
  `/home/antigravity/PodUtils/podnet` on Antigravity): on Antigravity, the lead creates each local worktree
  (`git worktree add -b NAME .claude/worktrees/NAME main`) before invoking the worker (`docs/hosted-sessions.md`), and
  workers serialize Cargo builds on 2-vCPU VMs with `flock /tmp/podnet-cargo.lock` sharing `CARGO_TARGET_DIR`.
* A step of `live.yml` runs under `bash -eo pipefail` (`scripts/live-step.sh`): give `grep` `|| true` where no match is
  normal.
* `report::tests::the_log_filter_names_each_pod_crate_of_podnet` needs each new `pod-*` crate of `Cargo.toml` in
  `crates/podnet-core/src/report.rs`; `each_variable_of_the_code_is_a_fact` needs each new variable in
  `src/manual/facts.rs`; every test that binds loopback must start with `need_loopback!()`.
* After a change of a document, help text, or version, run `cargo test --bin podnet -- --ignored write_the_llms_files`.
* On Antigravity, the VM kernel has no `nf_tables` module, so the three `router::kernel` namespace tests fail locally
  with `nf_tables table podnet_router: No such file or directory`; CI runners have `nf_tables` and run them cleanly.
* The forge mirror answers 403 at times for Actions lists and job logs: use `gh run list` / `gh run view` /
  `gh api --allow-escape-sequences`.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| `#801` (COORD-143, hand-off of `v0.4.0-beta.21`) | The other sessions read it; then close through the record. |
| `#800` (COORD-142, QA beta.21 follow-ups in `pod-proxy`, `pod-derp`, `pod-core`, `pod-warp`) | A tag of podcrates with the fixes. |
| `#799` (COORD-141, Gemini / Antigravity Cloud role table and startup flags) | podcrates adds the column to its shared `docs/hosted-sessions.md` table. |
| `#785` (COORD-140, `pod-egress` Trojan accept parser and Shadowsocks accept API) | A tag of podcrates (T-295). |
| `#784` (COORD-139, `podnet pod` inbound socket cleanup for PodUtils/podrunner#369) | podrunner takes `v0.4.0-beta.21` and closes #369. |
| `#783` (COORD-138, `pod-derp` take and neutral derper test names) | podcrates reads / updates tests. |
| `#782` (COORD-137, `pod-egress` Wisp reopen, stand-ins feature, SS-over-WS key file, Wisp password) | A tag of podcrates. |
| `#781` (COORD-136, `pod-cloudflare` capture hook gaps) | A tag of podcrates (T-244). |
| `#780` (COORD-135, `pod-acme` account contact and stand-in feature) | A tag of podcrates. |
| `#779` (COORD-134, `pod-proxy` TLS hop for process `https://` proxy) | A tag of podcrates (T-614). |
| `#778` (COORD-133, take of podcrates `v0.86.0`) | podcrates reads it; then close through the record. |
| `#777` (COORD-132, hand-off of `v0.4.0-beta.20`) | The other sessions read it; then close through the record. |
| `#760` (COORD-130, shared headscale harness) | podcrates writes it (T-114, T-115). |
| `#759` (COORD-129, hand-off of `v0.4.0-beta.19`) | The other sessions read it; then close through the record. |
| `#654` (COORD-107, coordinator channel / takes of podcrates) | A `state:` line at each take. |
| `#604` (COORD-098, relay holds a lost socket) | The relay fix (T-406, T-478). |
| `#413` (COORD-006, asks for `pod-relay`, `pod-dns`, `pod-proxy`, `pod-services`) | T-161 and T-200 (`pod-relay`). |
