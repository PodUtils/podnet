# Progress

`TODO/INDEX.md` holds 311 entries: 73 open, 14 partial, 29 blocked, 195 done.

## State

- The repository is `PodUtils/podnet`; GitHub redirects `Azathothas/cfrs` (T-021).
- The ruling "Order" of 2026-11-06 (`docs/decisions.md`): the work on podnet's own code goes on,
  beside the take of the shared crates (T-137). Betas are allowed; no crates.io (2026-11-09).
- podnet takes `pod-todo`, `pod-core`, `pod-tls`, `pod-proxy`, `pod-dns`, `pod-services`, `pod-ws`, `pod-quic`
  and `pod-hold` (`PodUtils/podcrates` v0.20.0); `podnet-netns` calls `pod_proxy::udp` (T-224). Next: `pod-relay`,
  then `pod-session` and `pod-wormhole` when podcrates writes their hand-off for podnet. A session reads the state
  of podcrates with `sh scripts/agent/podcrates.sh` (`docs/maintenance.md`, Shared crates).
- Frozen (fixes only, each as a COORD issue): `tests/test_host/mod.rs` (in `pod-testkit`).
- The session paused on 2026-11-10: `docs/HANDOFF.md` gives the state and the next steps.
- The newest release is `v0.4.0-beta.3` (T-235).
- Keep this section short. Write the current state only; git keeps the history.

## Work order

1. **At each release of podcrates:** T-137, the take of each crate whose hand-off
   `adoption/podnet/CRATE.patch` is at the tag (`docs/maintenance.md`, Shared crates). It goes
   before other work.
2. **podnet's own work:** the order below. Skip an entry whose Correction says that its work is
   the take of a crate of podcrates: it waits for the release of that crate. An entry that changes
   frozen code (`sh scripts/agent/podcrates.sh`) waits for the take, except a fix. Add no new
   hardcoded host or service of another party (`docs/decisions.md`, Services crate).
3. **Releases:** cut a beta when a batch of fixes is green (betas, 2026-11-09). The newest is `v0.4.0-beta.5`.

### The order of podnet's own work

Finish each entry, close it in place, change this page in the same commit, then take the next
one. In a milestone: P1, then P2, then P3. At one priority, an S entry goes first when it does
not wait for a larger one.

1. **M0, the record and the guards:** T-012 (blocked: the WARP stand-ins of `pod-warp`, T-137).
2. **M1, the rename:** done (the public image was measured on 2026-11-11).
3. **M5, the fixes, at any time from M0, beside M1 to M4:** each
   entry of this list is done. A fix in a pod backend lands now: the split of the pod backends is done.
   The list (the triage of GitHub #347 to #412; P1 first, then P2, then P3):
   - tunnel, share, serve and the CLI: T-169.
   - WARP: T-173, T-210.
   - the donor review (GitHub #429 to #434): done, except #433 and #435: T-242 is partial (its WARP half waits for the take of `pod-warp`), and T-244 is blocked on `pod-cloudflare`.
4. **M2, the splits:** T-035, T-036, T-039 (moved files: podcrates); T-043 and T-044 (blocked:
   the rows left go with the takes, T-137); T-045.
5. **M3, the crates:** T-053, then the prerelease T-054. T-049, T-050
   and T-052 are the takes of `pod-relay`, `pod-warp` and `pod-cloudflare` of `PodUtils/podcrates`
   (T-137): do not do them in podnet.
6. **M4, the adapters:** T-057, T-061 (partial: only the WARP door is left, it waits for the take of `pod-warp`, T-050).
7. **M6, the services:** the shared parts T-083, T-086 (a take of
   `pod-session`, `pod-e2e` and `pod-identity`), T-087; the file
   server and git: done; the router T-095, T-097,
   T-099, T-101; the transfer T-102 and T-103 (takes of `pod-wormhole` and `pod-transit`), T-104, T-105, T-106.
    The triage adds
    T-180 (the router control; partial: the rotate of a real WARP device waits for T-050); the router additions are in T-097 to T-099 (GitHub #372 to #374).
    The donor review adds T-242 (the lint of shadowed rules) and a correction of T-105 (GitHub #436).
    The owner's rulings of 2026-11-11 add: T-179 (the
    direction split, an option, off by default).
    The study of hosted relays (`docs/research/podnet/hosted-relays.md`) adds T-251 (the hosted mailboxes on 443,
    after T-102) and T-252 (a piping-server transit, after T-103).
8. **M7, the pods:** no open entry. The hold of a SYN in `pod-hold` (PodUtils/podcrates#68) can start.
9. **M8, the overlays:** T-114, T-115, T-117, T-247 (blocked: a NetBird account).
10. **M9, later:** T-120 to T-129, each when its start condition holds; T-248 (the
    guest-wire router, the owner's ruling of 2026-11-11; partial: the TCP and UDP paths are there; the veth wire
    and a proof of a real TUN stay). T-001: edit GitHub #89 into a guide
    that a new client tests from.
    T-123, T-124, T-126, T-127, T-128 and T-129 take `pod-ts`, `pod-egress`, `pod-derp` and
    `pod-acme` (wave 4). T-131 closes with the take of `pod-session`, `pod-e2e` and `pod-identity`.
11. **Blocked on `PodUtils/podcrates` (a COORD issue (to podcrates) for each):** T-161, T-162, T-163,
    T-200 (`pod-relay`), T-244 (`pod-cloudflare`); and the link entries T-261, T-263, T-298, T-294, T-295, T-284.
12. **The link workstream (GitHub #466 to #476), after podnet's own work (M9, P3):** T-261 to T-311 in `TODO/link.md`, in the order of the entries (E0 first). Start with T-262 (the session) and the two carriers that run on a restricted host, T-274 and T-275. Then T-264, T-265, T-266 and T-267 (E0), the stream carriers (T-268 to T-275), the ARQ and the datagram carriers (T-276 to T-280), the record layer (T-298 to T-301), the mesh (T-285 to T-290), the services (T-291 to T-297), the rest of E3 and E7 to E10. The features entry T-311 goes before each optional carrier. Blocked: T-261 and T-263 (the takes of `pod-identity` and `pod-session`, T-086), T-298 (`pod-e2e`, T-086), T-294 and T-295 (`pod-egress`, T-127), T-284 (`pod-relay`, T-049), T-305 (the owner: Linux only, 2026-11-12), T-262 (ask A2 of #479: a shared crate `pod-link`). `docs/decisions.md`, 2026-11-12, The link workstream, has the calls and the asks for podcrates.
13. **The asks of cfb, after podnet's own work** (the owner, 2026-11-09, Priority: podvm, podnet and
    podssh first; cfb and githome wait):
    T-221 (`bench`), T-219 and T-220 (blocked on `pod-unit`, `pod-relay`).

Releases (2026-11-09: betas, no crates.io): cut the next prerelease (`v0.4.0-beta.N`) unattended when
a batch is green (`CLAUDE.md`).

## Parked entries

These entries stay open. The condition in each entry tells when it can start.

| Entry | Start condition |
| --- | --- |
| T-131 | T-137 takes `pod-session`, `pod-e2e` and `pod-identity` (the owner chose the source: a git tag) |
| T-153 | T-152 gives a pass on the target hosts (the netfilter plane; T-111 stays) |
| T-179 | partial: the UDP split is built (an option, off by default); the TCP split waits for a far end that joins two connections |

## Calls made without a ruling

Each call is in its entry, and the work goes on with it until the owner rules otherwise:

- 2026-11-09, the triage of #347 to #412: the `## Decision` of T-144, T-151, T-152, T-153,
  T-159, T-160, T-161, T-165, T-166, T-169, T-173, T-174, T-179, T-182, T-183, T-187, T-196,
  T-200, T-204, T-206 and T-055 (the gsr transport: no entry; `pod-gsr` is deferred).

- 2026-11-10: the start conditions of T-055, T-056 (T-053) and T-092 (T-084) do not stop their work
  (`docs/decisions.md`, Start conditions); the time of pod-hold has come (T-232).

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
- T-126 to T-130: the extra egress roads, the DERP carrier, ACME, iroh and a relay of podnet's
  own have no ruling; each asks the owner before its work starts.

## Owner actions

None now. T-025 (the public image) and the crate names of T-007 were measured done on 2026-11-11.

## Questions for the owner

None now.

Write each question as `- **Qn.** QUESTION`, with the entries
that it blocks and a recommendation. Number the questions in sequence
from Q1.
