# Handoff

The state at 2026-11-13. Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md` and
`docs/decisions.md`. Remove this file when the lead session has no unfinished step.

## The goal

The owner (2026-11-13, `docs/decisions.md`, The complete beta): the beta test round is cancelled. Close each GitHub
issue except the deferred ones, then cut a final complete beta. The takes of PodUtils/podcrates follow its plan
(PodUtils/podcrates#133): `v0.40.0` brings the rest of the carriers, TURN over TCP and `pod-wormhole`; `v0.41.0` and later
bring `pod-cloudflare`, `pod-egress`, `pod-derp`, `pod-ts` and `pod-acme`.

## Where the code is

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (`rustup default 1.99.1`).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.39.0` (T-137), with `pod-warp`: `src/warp/` is glue
  over the crate and takes fixes only (each fix goes to podcrates as a COORD issue).
* The newest release is `v0.4.0-beta.9` (T-331).
* The record is `TODO/`. `cargo todo check` agrees: 333 entries, 27 open, 14 partial, 32 blocked, 260 done.

## Unfinished work (not on `main`)

None. The patches of the earlier session are merged: T-153 (085f5c8) and the WARP parts of the router, T-097, T-180, T-099
and T-061 (3855573).

Not started: the rest of T-287 (a test with three podnet processes) and of T-301 (RESUME inside the session resume of
T-263). A worktree brief for them is in the record (their Corrections).

## CI

A push of the cloud session starts no workflow run. Ask the owner for runs with an issue titled `[gh action] requested`
(a block of `gh workflow run` and `gh run watch` pinned to one commit; see GitHub #528 to #530). Map each request in
`TODO/github.txt` and `TODO/issues.md`, and close it when its results are in.

`ci`, `deny` and `secrets` pass on bf01ac6 (ci 43459872995). `live` is red since the take of `pod-warp` (GitHub #523,
T-332); each fix moved the failure to a later step (the Corrections of T-332 list each one). The run on bf01ac6
(43459893595, request #530) holds each fix so far. When `live` passes, set T-332 done and close #523 and #524 with the
run.

## Next steps

1. Read the results of #530; fix the next step of `live` that fails and ask for a new run, or close T-332, #523 and #524.
2. At each tag of podcrates: `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is at the tag (T-137).
   With the tag that brings the udp carrier, set `[profile.dev.package.kcp] overflow-checks = false` (T-276,
   PodUtils/podcrates#139) and answer GitHub #527.
3. Judge T-057 against the network `warp` (3855573), and steps 2 and 4 of T-086 against the code; close T-086 and T-131
   when they hold.
4. Go on with the work order of `TODO/PROGRESS.md`. The disk holds one build: run one agent at a time in this checkout,
   with `CARGO_INCREMENTAL=0`. Close the GitHub issues whose entries are done, with the commits and the CI runs
   (`docs/maintenance.md`, "The issue loop"). Issues #484 to #501 are deferred by the owner: leave them open.
5. When each non-deferred issue is closed and `live` is green: cut the final complete beta (the steps of T-331).

## Known flakes under load

`quick_tunnel_standin_serves_a_visitor_over_http2`, `service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection` and `ws_relay::...::a_demoted_relay_host_recovers_on_success`
and `autostart::sigterm_is_a_clean_stop` fail at times in a full run and pass alone. Run a failed test alone three times before a push.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #517 (COORD-060, to podswarm) | Each ask is answered; close it at the next beta. |
| #522, #526 (takes of v0.37.0 and v0.39.0) | podcrates records the take. |
| #524 (COORD-066) | Closes when `live` passes with the fix of `pod-warp`. |
| #527 (COORD-069) | The KCP answers; the `kcp` profile comes with the take of the udp carrier. |
| #413, #425, #437, #439, #440, #448, #460 | Older proposals and asks; see `TODO/issues.md`. |
