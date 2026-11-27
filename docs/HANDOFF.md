# Handoff

The product work state at 2026-11-27 21:15 (UTC). Start a hosted checkout with
[TODO/RESUME.md](../TODO/RESUME.md). Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`). The order: a red run of podnet's CI; each ask of
podrunner (a COORD issue `(to podnet)`); the take of each tag of podcrates (T-137); podnet's own work in the order of
`TODO/PROGRESS.md`. The newest tag of podcrates is `v0.86.0` (taken in T-612). The open ask of podrunner,
PodUtils/podrunner#369, is fixed and shipped in `v0.4.0-beta.21` (T-619; COORD-139 #784 updated; COORD-143 #801 hands
off the release).

## Where the code is

* All finished work is on `main` (`d8d2cac`): `ci` `46071689164` (all 13 jobs), `live` `46071689068` (including
  `Benchmark comparison, podnet vs cloudflared`: `podnet` 84.42 rps / 43.70 ms p50 vs `cloudflared 2026.11.2` 53.12
  rps / 77.63 ms p50, 0 errors each), `deny` `46071689190`, `secrets` `46071689317` and `issue-sync` `46071689493`
  all passed. The newest release is `v0.4.0-beta.21` (tag at `3d04883`; COORD-143 #801; manifest list digest
  `sha256:7b07bc89c549d48bfeeaf28794e5186a5d8cd6d4978a63bae62fbf7a411534a5`).
* Done in this session after `v0.4.0-beta.21`:
  * **T-367 (GitHub #486 closed):** provider-agnostic MASQUE client (`QUIC-1` to `QUIC-5`, `CONNECT-IP`,
    `CONNECT-UDP`, WebTransport, `--masque-endpoint`, `MasqueEnrolment` with two providers, `IpPolicy::Ipv6Only`
    zero-IPv4-attempt enforcement, `MasqueError::{Enrolment, Transport}` in `warp status`, `tests/quic_masque.rs`).
  * **T-368 (GitHub #489, partial on COORD-145 #803):** all six third-party road and status capabilities in podnet
    with zero hardcoded third-party hostnames (`src/roads/third_party/`, IRC/MQTT 3.1.1/Syncthing Relay v1 codecs
    and stand-ins, `link carriers --third-party`, `WatchState::{Ok, Degraded, Stale}`, `front=` `< 1 s` probe,
    `RoadClass::FallbackAdapter` never selected by default, deduplicated 7-class dependency counts and oldest
    verified date in `podnet status`, `tests/third_party_roads.rs`). Upstream `pod-services` `irc` entry and
    multi-vantage watch schedule asked in COORD-145 (#803).
  * **T-221 (done):** `scripts/live-bench-compare.sh` (anonymous quick tunnels for both `podnet` and `cloudflared`
    `2026.11.2`, SHA-256 verified, masked comparison table, `podnet bench --json`, 429/1015 skip) wired into
    `.github/workflows/live.yml`.
  * **T-247 (done):** live measurement of NetBird `v0.84.0` netstack mode against self-hosted `netbird-server` on a
    strictly proxy-only host (`docs/reports/netbird-netstack-2026-11-27.md`, `docs/networks.md`), plus fix in
    `crates/podnet-netns/src/upstream.rs` so `podnet pod -x netbird:[PORT]` and `--egress netbird:[PORT]` expand to
    `127.0.0.1:PORT`.
  * **T-414 & T-488 (blocked on COORD-144 #802):** reproduced T-414 idle MASQUE TCP RST gap 100% locally and ran
    T-488 direct + `quic+warp` measurements plus `cloudflared` source check; handed both off to `podcrates` in
    COORD-144 (#802). T-415 checked 5/5 live trials (`warp=on`, 0 resets).
  * **T-001 (#89):** refreshed the living test guide on GitHub #89 to `v0.4.0-beta.21` (added `podnet link` +
    `restricted-host`/`no-loopback` carrier table, `derp`, `send`/`receive`, `serve --acme`, `tunnel --inspect`,
    `exit --stdio`, `ssh-relay`, `--network` specs, and T-247 NetBird note).
* Remaining entries are all blocked behind `PodUtils/podcrates`, sibling projects, or the owner (accounts/licences/rulings):
  * **Blocked on `PodUtils/podcrates` / relay:** T-137 (next tag after `v0.86.0`), T-114 and T-115 (COORD-130 #760,
    shared headscale harness), T-123 and T-124 (no `pod-ts` crate), T-161 and T-200 (COORD-006 #413, `pod-relay`),
    T-219 and T-220 (`pod-unit`, `pod-relay`), T-244 (COORD-136 #781, `pod-cloudflare` capture hook gaps), T-283 and
    T-284 (`pod-link`), T-295 (COORD-140 #785, `pod-egress` Trojan accept parser), T-316 (QUIC stand-in leg on
    `pod-cloudflare`), T-368 (COORD-145 #803, `pod-services` `irc` + multi-vantage watch), T-406 and T-478
    (COORD-098 #604, relay lost-socket reset), T-414 and T-488 (COORD-144 #802, `pod-warp` idle reset &
    `pod-cloudflare` quick-tunnel `ha_connections`), T-415 (partial: waits for any recurrence in `live`), T-586
    (`pod-link` p2p), T-614 (COORD-134 #779, `pod-proxy` TLS hop for process `https://` proxy), T-044 (`podcrates`
    / T-137), T-374 (#497, relay operator + owner), T-375 (#498, `pod-session` + owner).
  * **Blocked on owner rulings / accounts / licences / captures:** T-001 (#89), T-002 (#140, L4 capture), T-003
    (#149, Zero Trust team token capture), T-004 (#151, ECH HPKE ruling), T-005 (#161), T-006 (#222), T-007 (#224),
    T-210 (#407, paid WARP+ device).

## The next steps, in order

1. **Each new tag of `PodUtils/podcrates` (> `v0.86.0`):** take it (T-137), close the COORD issues that it answers, and
   unblock the entries above (such as T-614, T-295, T-244, T-114/T-115, T-123/T-124, T-368, T-414/T-488, T-406/T-478).
2. **Each new ask of `podrunner` (or `cfb` / `githome`):** handle before other podnet work (`docs/decisions.md`,
   2026-11-18, podrunner first).
3. **Any red run of `ci` or `live`:** triage; capture T-415 warnings if they fire in `live`.
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
| `#803` (COORD-145, `pod-services` `irc` road entry & multi-vantage watch for T-368) | A tag of podcrates (T-368). |
| `#802` (COORD-144, `pod-warp` idle MASQUE reset detection & `pod-cloudflare` quick-tunnel `ha_connections`) | A tag of podcrates (T-414, T-488). |
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
