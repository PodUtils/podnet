# Progress

`TODO/INDEX.md` holds 618 entries: 1 open, 9 partial, 28 blocked, 580 done.

## State

- The repository is `PodUtils/podnet`; GitHub redirects `Azathothas/cfrs` (T-021).
- The ruling "Order" of 2026-11-06 (`docs/decisions.md`): the work on podnet's own code goes on,
  beside the take of the shared crates (T-137). Betas are allowed; no crates.io (2026-11-09).
- podnet takes the crates of `PodUtils/podcrates` by the tag `v0.86.0` (T-612): `pod-todo`, `pod-core`, `pod-tls`,
  `pod-proxy`, `pod-dns`, `pod-services`, `pod-ws`, `pod-quic`, `pod-hold`, `pod-wg`, `pod-netstack`, `pod-relay`,
  `pod-identity`, `pod-session`, `pod-e2e`, `pod-link`, `pod-turn`, `pod-devtunnel`, `pod-warp`, `pod-wormhole`
  (the transfer), `pod-extract`, `pod-manual`, `pod-cloudflare`, `pod-cover`, `pod-harness`, `pod-offline`,
  `pod-shape`, optional `pod-sidecar` (feature `sidecar`), `pod-release` (T-494, `podnet update`), and
  `pod-testkit`. Not taken yet: `pod-egress`, `pod-derp`, `pod-ts` and `pod-acme`. A session reads the state of
  podcrates with `sh scripts/agent/podcrates.sh` (`docs/maintenance.md`, Shared crates). A file that a block of `crates/*/SOURCES`
  names with no `taken` line is frozen: it takes fixes only, each as a COORD issue.
- The newest release is `v0.4.0-beta.20` (T-608, 2026-11-26, the tag at `be2c0df`): the carriers of the feature
  `release` (#620) and the fixes of the QA pass #761 to #775. COORD-132 (#777) hands off the files and the digests.
- Deferred feature consolidation (`4df28952`): T-365, T-366, T-369 to T-373, and T-376 to T-378 are done; T-367
  (GitHub #486) stays partial on MASQUE `QUIC-1` to `QUIC-5`; T-368 (#489), T-374 (#497), and T-375 (#498) stay
  blocked.
- Hard-problems fixes (`docs/research/podnet/hard-problems-results.md`, `4df28952`): T-507 (H-02), T-508 (H-04), T-509
  (H-05), H-10 (`docs/warp.md`), H-11 (`scripts/live-throughput.sh`), and H-19 (`tests/transfer_relay.rs` `160 KiB` cap
  test move; T-476 is blocked on `pod-session` `move_under`, COORD-126 #756).
- The goal (the owner, 2026-11-13): close each issue except the deferred ones, then a final complete beta. The takes
  follow the plan of podcrates (PodUtils/podcrates#133; `docs/decisions.md`, 2026-11-13).
- Keep this section short. Write the current state only; git keeps the history.

## Work order

Do the entries below one after the other, with no stop for a confirmation of the owner (`CLAUDE.md`, "Work without a
stop"). End a turn only when each remaining entry is blocked.

00. **Before all: podrunner first** (`docs/decisions.md`, 2026-11-18, podrunner first and Coordinator): a red run of podnet's CI, then each ask of podrunner (a COORD issue `(to podnet)`; the coordinator is the session of podcrates), then each ask of githome and cfb, then the take of each tag of podcrates. Other work waits.
0. **The QA round of 2026-11-17 is done** (`docs/decisions.md`, The QA round and the release freeze): the gate of
   correctness (the entries 450 to 455) is done (2026-11-20). The freeze of releases and features ends with it. Each next
   beta runs the black-box QA pass first (`scripts/agent/qa-pass.md`; `docs/maintenance.md`, Release, step 0), and
   the full lane in `gate.yml` (the case `errors`, the row `testers`).
1. **At each release of podcrates:** T-137, the take of each crate whose hand-off
   `adoption/podnet/CRATE.patch` is at the tag (`docs/maintenance.md`, Shared crates). It goes
   before other work. The next tag after `v0.86.0` is the next take.
2. **podnet's own work:** the order below. Skip an entry whose Correction says that its work is
   the take of a crate of podcrates: it waits for the release of that crate. An entry that changes
   frozen code (`sh scripts/agent/podcrates.sh`) waits for the take, except a fix. Add no new
   hardcoded host or service of another party (`docs/decisions.md`, Services crate).
3. **Releases:** the gate of the QA round is done (2026-11-20). The newest is `v0.4.0-beta.20`. The next beta
   (`v0.4.0-beta.21`) runs the black-box QA pass first (`docs/maintenance.md`, Release, step 0).

### The order of podnet's own work

Finish each entry, close it in place, change this page in the same commit, then take the next
one. In a milestone: P1, then P2, then P3. At one priority, an S entry goes first when it does
not wait for a larger one.

1. **M0, the record and the guards:** done.
2. **M1, the rename:** done (the public image was measured on 2026-11-11).
3. **M5, the fixes, at any time from M0, beside M1 to M4:** the entries of the triage of GitHub #347 to #412 are done.
   The entries that stay (P1 first, then P2, then P3):
   - The findings of the QA pass of `v0.4.0-beta.19` (#713 to #751): each is done (the last three by the take of podcrates `v0.86.0`).
   - The findings of the QA pass of `v0.4.0-beta.20` (#761 to #775): each is done.
   - CI: T-316 (partial: the stand-in runs in ci; its QUIC leg waits for `PodUtils/podnet@7fe60e0:src/cloudflare/`, frozen).
   - WARP: T-210 (blocked: a device with a paid licence).
   - the donor review (GitHub #429 to #434): done, except #435: T-244 (partial: `tunnel --inspect` is built and measured; the gaps of the hook wait for COORD-136 #781).
   - open and partial follow-ups (`TODO/fixes.md`):
     T-478 and T-406 (blocked, #604), T-488, T-414 and T-415
     (partial), T-358 (partial).
4. **M2, the splits:** T-044 (blocked:
   the rows left go with the takes, T-137). The split of the tunnel is done by the take of `pod-cloudflare`; each file has
   at most 800 lines, and no allowlist has an entry.
5. **M3, the crates:** done (the facade, the path map, and the prerelease after the split: `v0.4.0-beta.19`).
6. **M4, the adapters:** done.
7. **M6, the services:** done: the file server, git, the router, the resumable layer and `--seal` of the reverse road, and the
   transfer by code (`podnet send` and `podnet receive` over `pod-wormhole`; `docs/transfer.md`; `docs/decisions.md`,
   2026-11-13, The transfer layer). Each entry is done (the TCP part of the direction split too, 2026-11-26).
8. **M7, the pods:** no open entry. The hold of a SYN in `pod-hold` (PodUtils/podcrates#68) can start.
9. **M8, the overlays:** T-114 and T-115 (blocked: a shared headscale harness, COORD-130 #760), T-247 (blocked: a NetBird account).
10. **M9, later:** T-128 (the DERP road of `pod-derp`: built; its `live` transfer step waits for a run). T-123 and
    T-124 are blocked: podcrates `v0.86.0` has no `pod-ts` (the entry of `pod-ts` in podcrates is open). The proxy of
    the process `https://` is T-614, blocked on COORD-134 #779. T-001: edit GitHub #89 into a guide that a new client
    tests from (blocked: the owner).
11. **Blocked on `PodUtils/podcrates` (a COORD issue (to podcrates) for each):** T-161 and T-200
    (`pod-relay`: a relay that serves the call; the control host only), T-244 (`pod-cloudflare`); and the link
    entry T-295.
12. **The link workstream (GitHub #466 to #476), after podnet's own work (M9, P3):** the open entries of `TODO/link.md`:
    T-283 and T-284 (blocked: each is a take of `pod-link`), and T-295 (blocked: a parser of a Trojan request in `pod-egress`, COORD-140 #785). The
    other entries of `TODO/link.md` are done. `docs/link.md` describes the built part. `docs/decisions.md`, 2026-11-12,
    The link workstream, has the calls and the asks for podcrates.
13. **The asks of cfb, after podnet's own work** (the owner, 2026-11-09, Priority: podvm, podnet and
    podssh first; cfb and githome wait):
    T-221 (`bench`), T-219 and T-220 (blocked on `pod-unit`, `pod-relay`).
14. **The asks of podswarm (PodUtils/podcrates#125), after podnet's own work (P3; each has a fallback in podswarm):**
    N-4 (one flow over several uplinks) is answered in `docs/router.md`: per flow only;
    the optional packet sidecar (feature `sidecar`, off by default) adds per-flow link bonding.

Releases (2026-11-09: betas, no crates.io): cut the next prerelease (`v0.4.0-beta.N`) unattended when
a batch is green (`CLAUDE.md`).

## Parked entries

These entries stay open. The condition in each entry tells when it can start.

| Entry | Start condition |
| --- | --- |
| (none) | |

## Calls made without a ruling

Each call is in its entry, and the work goes on with it until the owner rules otherwise:

- 2026-11-09, the triage of #347 to #412: the `## Decision` of T-144, T-151, T-152, T-153,
  T-159, T-160, T-161, T-165, T-166, T-169, T-173, T-174, T-179, T-182, T-183, T-187, T-196,
  T-200, T-204, T-206 and T-055 (the gsr transport: no entry; `pod-gsr` is deferred).

- 2026-11-10: the start conditions of T-055, T-056 (T-053) and T-092 (T-084) do not stop their work
  (`docs/decisions.md`, Start conditions); the time of pod-hold has come (T-232).

- 2026-11-23, the triage of the QA pass of 0.4.0-beta.18: the `## Premise` of T-518 (a missing policy file exits 78),
  T-525 (an explicit `--out` on a `noexec` directory exits 78), T-527 (the SOCKS5 replies of RFC 1928) and T-528 (`doctor`
  and `bench` keep exit 1; the manual changes) name the call. The owner can rule otherwise.

- T-016: the shim names `podnet-vnet` (`PODNET_VNET_*`) and `podnet-socks` (`PODNET_SOCKS_*`).
- T-046: the test helpers go into `podnet-core` behind a feature; `pod-testkit` of
  `PodUtils/podcrates` is read first.
- T-065: the handoff of an upstream descriptor for Unix-socket upstreams only.
- T-068: podnet renews a WARP session by itself after three failed TCP probes to 1.1.1.1:443.
- T-070: a pointer file of `warp up` in the runtime directory of the user, outside the state
  directory.
- T-087: the resumable layer alone first; the end-to-end channel as an option.
- T-100: the config file of the router in YAML, read by the reader of the tree.
- T-103: an inflate decoder for the zip of a directory (own code or a pure Rust crate), in the
  shared crate that reads the zip.
- T-112: `--host-gateway` off by default.
- T-126 to T-129: the extra egress roads, ACME and the DERP carrier start on the released crates (`docs/decisions.md`,
  2026-11-26, The work of wave 4 starts). T-130 (iroh, a relay of podnet's own) still asks the owner first.

## Owner actions

None now. T-025 (the public image) and the crate names of T-007 were measured done on 2026-11-11.

## Questions for the owner

None now. The owner ruled Q1 and Q2 on 2026-11-16 (`docs/decisions.md`).

Write each question as `- **Qn.** QUESTION`, with the entries
that it blocks and a recommendation. Number the questions in sequence
from Q1.
