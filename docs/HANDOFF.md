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
* The record is `TODO/`. `cargo todo check` agrees: 332 entries, 27 open, 18 partial, 33 blocked, 254 done.

## Unfinished work (not on `main`)

None. The patches of the earlier session are merged: T-153 (085f5c8) and the WARP parts of the router, T-097, T-180, T-099
and T-061 (WARPCOMMIT).

Not started: the rest of T-287 (a test with three podnet processes) and of T-301 (RESUME inside the session resume of
T-263). A worktree brief for them is in the record (their Corrections).

## Live CI

`live` is red since the take of `pod-warp` (GitHub #523, T-332). Each fix so far moved the failure to a later step:

1. QUIC over WARP and `warp nc -u` (the `[::]` socket of `WarpNet`): fixed in `pod-warp` `v0.39.0` (GitHub #524).
2. `warp nc -u` with no `warp up`: the session was dropped while the pipe ran (ef240bb).
3. The relay line and the TUN line of `pod-warp` were hidden: the job's `RUST_LOG` and podnet's default log filter now
   name `pod_warp` and the other `pod-*` crates (e1c5b34, 6fec3df).
4. "WARP run": a step's `RUST_LOG=podnet=debug` hid the debug line of `pod_warp` (ecbd88b).

The run on b4c8b23 holds each fix and had not finished. Check it first. When `live` passes, close GitHub #523 with the
run. Push in batches: each push cancels the pending `live` run (`cancel-in-progress` is false, so a running one goes on).

## Next steps

1. Read the live run of b4c8b23; fix the next step that fails, or close #523.
2. Apply and finish the two patches above, one at a time (they share no files).
3. At each tag of podcrates: `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is at the tag (T-137).
   With the tag that brings the udp carrier, set `[profile.dev.package.kcp] overflow-checks = false` (T-276,
   PodUtils/podcrates#139) and answer GitHub #527.
4. Go on with the work order of `TODO/PROGRESS.md`. Run two worktree agents at a time at most. Merge each as before:
   rebase on `main`, take `--ours` for `TODO/INDEX.md` and `TODO/PROGRESS.md` and run `cargo todo set`, cite the rebased
   commit, run `gen-llms.py`, run `check.sh`, push.
5. Close the GitHub issues whose entries are done, with the commits and the CI runs (`docs/maintenance.md`, "The issue
   loop"). Issues #484 to #501 are deferred by the owner: leave them open.
6. When each non-deferred issue is closed and `live` is green: cut the final complete beta (the steps of T-331).

## Known flakes under load

`quick_tunnel_standin_serves_a_visitor_over_http2`, `service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection` and `ws_relay::...::a_demoted_relay_host_recovers_on_success`
fail at times in a full run and pass alone. Run a failed test alone three times before a push.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #517 (COORD-060, to podswarm) | Each ask is answered; close it at the next beta. |
| #522, #526 (takes of v0.37.0 and v0.39.0) | podcrates records the take. |
| #524 (COORD-066) | Closes when `live` passes with the fix of `pod-warp`. |
| #527 (COORD-069) | The KCP answers; the `kcp` profile comes with the take of the udp carrier. |
| #413, #425, #437, #439, #440, #448, #460 | Older proposals and asks; see `TODO/issues.md`. |
