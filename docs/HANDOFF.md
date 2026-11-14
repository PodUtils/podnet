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
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.41.0` (T-137), with `pod-warp`: `src/warp/` is glue
  over the crate and takes fixes only (each fix goes to podcrates as a COORD issue).
* The newest release is `v0.4.0-beta.10` (T-336).
* The record is `TODO/`. `cargo todo check` agrees: 337 entries, 13 open, 11 partial, 28 blocked, 285 done.

## Unfinished work (not on `main`)

None. The patches of the earlier session are merged: T-153 (085f5c8) and the WARP parts of the router, T-097, T-180, T-099
and T-061 (3855573).

T-287 and T-301 are done.

## CI

A push of the cloud session starts no workflow run. Ask the owner for runs with an issue titled `[gh action] requested`
(a block of `gh workflow run` and `gh run watch` pinned to one commit; see GitHub #528 to #530). Map each request in
`TODO/github.txt` and `TODO/issues.md`, and close it when its results are in. Do not change the SHA of a request
after it is sent: the owner can run it at once (GitHub #531). Put later commits into the next request.
A release request (`docs/maintenance.md`, "Release") chains: the checks, the dry run of `release.yml`, the publish
(`-f publish=vX`), the release on the tag, and `release-proofs.yml -f tag=vX`. The release on the tag is a run of the event
`workflow_dispatch` on the branch `vX` (not `push`): find it with `gh run list --workflow release.yml --branch vX`.
`release.yml` tags the head of `main` at the publish: push nothing to `main` from the request to the tag.

Each workflow passes on 3061c7c: ci 43521791090, deny 43521797247, secrets 43521803788, live 43521809728 (with the
router step of T-101 and the 200 MiB step of T-087).

## Next steps

1. Ask for runs of each new head with a `[gh action] requested` issue after a batch of changes.
2. At each tag of podcrates: `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is at the tag (T-137).
   With the tag that brings the udp carrier, set `[profile.dev.package.kcp] overflow-checks = false` (T-276,
   PodUtils/podcrates#139) and answer GitHub #527.
3. Go on with the work order of `TODO/PROGRESS.md`. The disk holds one build: run one agent at a time in this checkout,
   with `CARGO_INCREMENTAL=0`. Close the GitHub issues whose entries are done, with the commits and the CI runs
   (`docs/maintenance.md`, "The issue loop"). Issues #484 to #501 are deferred by the owner: leave them open.
4. When each non-deferred issue is closed and `live` is green: cut the final complete beta (the steps of T-331).

## Known flakes under load

`quick_tunnel_standin_serves_a_visitor_over_http2`, `service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection` and `ws_relay::...::a_demoted_relay_host_recovers_on_success`
and `autostart::sigterm_is_a_clean_stop` fail at times in a full run and pass alone. Run a failed test alone three times before a push.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #517 (COORD-060, to podswarm) | Each ask is answered; close it at the next beta. |
| #522, #526 (takes of v0.37.0 and v0.39.0) | podcrates records the take. |
| #527 (COORD-069) | The KCP answers; the `kcp` profile comes with the take of the udp carrier. |
| #413, #425, #437, #439, #440, #448, #460 | Older proposals and asks; see `TODO/issues.md`. |
