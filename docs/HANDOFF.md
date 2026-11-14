# Handoff

The state at 2026-11-14. Read this file first, then `CLAUDE.md`, `TODO/PROGRESS.md`, `docs/maintenance.md` and
`docs/decisions.md`. Remove this file when the lead session has no unfinished step.

## The goal

The owner (2026-11-13, `docs/decisions.md`, The complete beta): the beta test round is cancelled. Close each GitHub
issue except the deferred ones, then cut a final complete beta. The takes of PodUtils/podcrates follow its plan
(PodUtils/podcrates#133): the next tags bring `pod-cloudflare`, `pod-egress`, `pod-derp`, `pod-ts` and `pod-acme`.

## Where the code is

* All finished work is on `main` of `PodUtils/podnet`. The toolchain is Rust 1.99.1 (`rustup default 1.99.1`).
* podnet takes the crates of `PodUtils/podcrates` by the git tag `v0.45.0` (T-137, e84ebdf), with `pod-warp`: `src/warp/` is glue
  over the crate and takes fixes only (each fix goes to podcrates as a COORD issue).
* The newest release is `v0.4.0-beta.11` (T-351, 6491f9d).
* The record is `TODO/`. `cargo todo check` agrees: 351 entries, 11 open, 9 partial, 25 blocked, 306 done.

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
For `v0.4.0-beta.11` (GitHub #563) that lookup gave the run of the publish itself (43621310830): one run tags and publishes.
`release.yml` tags the head of `main` at the publish: push nothing to `main` from the request to the tag.

Each workflow passes on 6491f9d, the tag `v0.4.0-beta.11` (GitHub #563): ci 43619108933, deny 43619113053, secrets
43619116496, live 43619120794; fuzz 43610680236 passed on 7860081 (GitHub #561).

## Next steps

1. Ask for runs of each new head with a `[gh action] requested` issue after a batch of changes.
2. At each tag of podcrates: `sh scripts/agent/podcrates.sh`, and take the crates whose hand-off is at the tag (T-137).
3. Go on with the work order of `TODO/PROGRESS.md`. The disk holds one build: run one agent at a time in this checkout,
   with `CARGO_INCREMENTAL=0`. Close the GitHub issues whose entries are done, with the commits and the CI runs
   (`docs/maintenance.md`, "The issue loop"). Issues #484 to #501 are deferred by the owner: leave them open.
4. When each non-deferred issue is closed and `live` is green: cut the final complete beta (the steps of T-331).

## Known flakes under load

`quick_tunnel_standin_serves_a_visitor_over_http2`, `service_serve_ingress::the_service_ends_with_the_serve_process`,
`shim_direct::a_perl_accept_loop_serves_each_connection` and `ws_relay::...::a_demoted_relay_host_recovers_on_success`
`autostart::sigterm_is_a_clean_stop`, `doctor_ssh_relay_row_dials_through_the_proxy` and
`guest_wire_unix_socket_serves_a_guest_and_the_next` (keep the default `TMPDIR`) fail at times in a full run and pass alone. Run a failed test alone three times before a push.

## Open COORD issues

| Issue | Waits for |
| --- | --- |
| #522, #526, #532 (takes of v0.37.0, v0.39.0 and v0.40.0) | podcrates records the take. |
| #560 (COORD-081, the take of v0.45.0) | The answer of podcrates on the ws listener (a refused pin: `Refused` or no peer). |
| #555 (COORD-079) | The fuzz crash of `pod-proxy` is fixed in v0.45.0 (T-349); the crash of `pod-cloudflare` waits for its take (their entry 029). |
| #413, #425, #437, #439, #440, #448, #460 | Older proposals and asks; see `TODO/issues.md`. |
