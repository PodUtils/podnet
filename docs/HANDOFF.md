# Handoff

The product work state at 2026-11-28 02:00 (UTC). Start a hosted checkout with
[TODO/RESUME.md](../TODO/RESUME.md). Remove this file when the lead session has no unfinished step.

## The goal

**podrunner first** (the owner, 2026-11-18; `docs/decisions.md`). The order: a red run of podnet's CI; each ask of
podrunner (a COORD issue `(to podnet)`); the take of each tag of podcrates (T-137); podnet's own work in the order of
`TODO/PROGRESS.md`. The newest tag of podcrates is `v0.86.0` (taken in T-612). The open ask of podrunner,
PodUtils/podrunner#369, is fixed and shipped in `v0.4.0-beta.21` (T-619; COORD-139 #784 updated; COORD-143 #801 hands
off the release).

## Where the code is

* `TODO/INDEX.md`: 631 entries — **0 open, 6 partial, 24 blocked, 601 done**.
* Newest release: `v0.4.0-beta.21` (tag at `3d04883`; COORD-143 #801; manifest list digest
  `sha256:7b07bc89c549d48bfeeaf28794e5186a5d8cd6d4978a63bae62fbf7a411534a5`).
* Done after `v0.4.0-beta.21` on `main`:
  * **Client capture rig (`ae09cb3`):** hermetic, idempotent four-layer capture harness (`scripts/capture-rig/`,
    `docs/capture-rig.md`, `tests/capture_rig.rs`) combining packet-buffered `tcpdump`, `SSLKEYLOGFILE` + `QLOGDIR`,
    auto-fallback MITM/passthrough proxy (`mitm_proxy.py`), verified Wireshark 4.2 + pure-Python PCAP/QLOG dissector
    (`quic_dissect.py`), and zero-leak in-container secret redaction + SHA-256 manifest (`redact.py`).
  * **T-002 (GitHub #140 closed, `465bc0f`):** live official `cloudflare-warp` `2026.8.2115.0` QLOG capture
    (`docs/reports/warp-client-capture-2026-11-27.md`) proved `consumer-masque-proxy.cloudflareclient.com` requires a
    Stream 0 `CONNECT :protocol cf-connect-ip :scheme http :authority cloudflareaccess.com :path /` bootstrap before
    per-flow plain `CONNECT` with IP-literal `:authority`. Implemented in `src/warp/l4.rs` with DoH-over-L4 IP
    resolution (honoring `--resolve` pins, zero host DNS leak), verified live (`warp=on` on `1.1.1.1` and
    `www.cloudflare.com`, 20 MB HTTP 200 download at 5.90 MB/s), upgraded `scripts/live-batch2.sh` `--l4` to a real
    check, and handed upstream sync to `pod-warp` in COORD-146 (#804).
  * **T-004 (GitHub #151 closed, `715720c`):** enabled `features = ["ech"]` on `pod-tls`, `pod-dns`, `pod-proxy`, and
    `pod-ws` (`podcrates v0.86.0` pure-Rust `ring` HPKE, zero new external crates), added `src/ech.rs` (`--ech`,
    `--ech-grease`, `PODNET_ECH*`, `max_name_length` checks), wired `doctor` ECH status and `docs/proxies.md`, and
    added unit + loopback integration tests (`tests/ech.rs`).
  * **T-210 (GitHub #407 closed, `465bc0f`):** official `cloudflare-warp 2026.8.2115.0` live capture + `WarpAccount`
    enum (`free`, `limited`, `unlimited`, `team`) backed the closed paid-type classifier in `src/warp/identity.rs`
    (`warp_plus_label`), with unit tests in `src/warp/identity.rs` and `src/cli/cmd/warp/cli_tests.rs`; synced upstream
    in COORD-146 (#804).
  * **Doc refresh:** updated `README.md`, `docs/research/podnet/hard-problems-results.md`, `docs/research/podnet/hard-problems.md`,
    `docs/warp.md`, `docs/protocol.md`, `docs/capture-rig.md`, `TODO/`, `llms.txt`, and `llms-full.txt`.
  * Earlier post-beta.21 batches (verified green on `ci 46071689164`, `live 46071689068`): **T-367** (`#486` MASQUE
    `QUIC-1..5`), **T-368** (`#489` third-party roads; COORD-145 `#803`), **T-221** (`live-bench-compare.sh`),
    **T-247** (NetBird netstack + `netbird:[PORT]`), **COORD-144** (`#802`, T-414 + T-488), and **T-001** (`#89` guide
    refresh).
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
  * **Blocked on owner rulings / accounts / licences:** T-001 (#89), T-003 (#149, Zero Trust team token +
    `pod-warp::Account` fields noted in COORD-146 #804), T-005 (#161), T-006 (#222), T-007 (#224).

## The next steps, in order

1. **Each new tag of `PodUtils/podcrates` (> `v0.86.0`):** take it (T-137), close the COORD issues that it answers
   (including COORD-146 #804 once `pod-warp` syncs L4 and `warp_plus_label`), and unblock the entries above.
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
| `#804` (COORD-146, `pod-warp` L4 Stream 0 `cf-connect-ip` bootstrap & `WarpAccount` paid types for T-002 / T-210) | A tag of podcrates. |
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
