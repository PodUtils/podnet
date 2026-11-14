# Progress

`TODO/INDEX.md` holds 350 entries: 11 open, 9 partial, 25 blocked, 305 done.

## State

- The repository is `PodUtils/podnet`; GitHub redirects `Azathothas/cfrs` (T-021).
- The ruling "Order" of 2026-11-06 (`docs/decisions.md`): the work on podnet's own code goes on,
  beside the take of the shared crates (T-137). Betas are allowed; no crates.io (2026-11-09).
- podnet takes the crates of `PodUtils/podcrates` by the tag `v0.41.0`: `pod-todo`, `pod-core`, `pod-tls`,
  `pod-proxy`, `pod-dns`, `pod-services`, `pod-ws`, `pod-quic`, `pod-hold`, `pod-wg`, `pod-netstack`, `pod-relay`,
  `pod-identity`, `pod-session`, `pod-e2e`, `pod-link`, `pod-turn`, `pod-devtunnel`, `pod-warp` and `pod-testkit`.
  The tag also holds `pod-wormhole` (the transfer, T-102 to T-106; podnet does not use it yet). Next
  (PodUtils/podcrates#133): `pod-cloudflare`, `pod-egress`, `pod-derp`, `pod-ts` and `pod-acme`. A session
  reads the state of podcrates with `sh scripts/agent/podcrates.sh` (`docs/maintenance.md`, Shared crates).
- Frozen (fixes only, each as a COORD issue): `src/warp/` (glue over `pod-warp`), `src/cloudflare/`, `src/proxy.rs`,
  `src/tunnel.rs`, `crates/podnet-netns/src/rules.rs` and `tests/test_host/mod.rs` (in `pod-testkit`).
- The newest release is `v0.4.0-beta.10` (T-336).
- The goal (the owner, 2026-11-13): close each issue except the deferred ones, then a final complete beta. The takes
  follow the plan of podcrates (PodUtils/podcrates#133; `docs/decisions.md`, 2026-11-13).
- Keep this section short. Write the current state only; git keeps the history.

## Work order

1. **At each release of podcrates:** T-137, the take of each crate whose hand-off
   `adoption/podnet/CRATE.patch` is at the tag (`docs/maintenance.md`, Shared crates). It goes
   before other work.
2. **podnet's own work:** the order below. Skip an entry whose Correction says that its work is
   the take of a crate of podcrates: it waits for the release of that crate. An entry that changes
   frozen code (`sh scripts/agent/podcrates.sh`) waits for the take, except a fix. Add no new
   hardcoded host or service of another party (`docs/decisions.md`, Services crate).
3. **Releases:** cut a beta when a batch of fixes is green (betas, 2026-11-09). The newest is `v0.4.0-beta.10`.

### The order of podnet's own work

Finish each entry, close it in place, change this page in the same commit, then take the next
one. In a milestone: P1, then P2, then P3. At one priority, an S entry goes first when it does
not wait for a larger one.

1. **M0, the record and the guards:** done.
2. **M1, the rename:** done (the public image was measured on 2026-11-11).
3. **M5, the fixes, at any time from M0, beside M1 to M4:** each
   entry of this list is done. A fix in a pod backend lands now: the split of the pod backends is done.
   The list (the triage of GitHub #347 to #412; P1 first, then P2, then P3):
   - CI: T-316 (partial: the stand-in runs in ci; its QUIC leg waits for `src/cloudflare/`, frozen).
   - WARP: T-210 (blocked: a device with a paid licence).
   - the donor review (GitHub #429 to #434): done, except #433 and #435: T-242 is partial (its WARP half is an ask to
     podcrates: `pod-warp` is taken), and T-244 is blocked on `pod-cloudflare`.
4. **M2, the splits:** T-039 (moved files: podcrates); T-044 (blocked:
   the rows left go with the takes, T-137); T-045.
5. **M3, the crates:** T-053, then the prerelease T-054. T-052 is the take of `pod-cloudflare` of
   `PodUtils/podcrates` (T-137): do not do it in podnet.
6. **M4, the adapters:** done.
7. **M6, the services:** the shared part (the resumable layer and `--seal` of the reverse road) is done; the file
   server and git: done; the router: done; the transfer by code: `podnet send` and `podnet receive` are done
   over `pod-wormhole` v0.40.0 (`docs/transfer.md`; `docs/decisions.md`, 2026-11-13, The transfer layer).
   The transit key is magic-wormhole's (`Wormhole::derive_key` of `pod-wormhole` v0.42.0): a file and a
   directory go to and from each client of magic-wormhole. The podnet relay is the last way of the transit from podnet to podnet (done:
   `podnet send --relay-pair NAME`); a piping server is the last way of all (`--piping`). Open:
   T-106 (partial: a text, a file and a directory both ways with the Python magic-wormhole are done; the live
   step of 100 MiB is written and waits for a run of `live`).
    The donor review adds T-242 (the lint of shadowed rules).
    The owner's rulings of 2026-11-11 add: T-179 (the
    direction split, an option, off by default).
8. **M7, the pods:** no open entry. The hold of a SYN in `pod-hold` (PodUtils/podcrates#68) can start.
9. **M8, the overlays:** T-114, T-115, T-247 (blocked: a NetBird account).
10. **M9, later:** T-123, T-124 and T-126 to T-129, each when its start condition holds; T-001: edit GitHub #89 into a guide
    that a new client tests from.
    T-123, T-124, T-126, T-127, T-128 and T-129 take `pod-ts`, `pod-egress`, `pod-derp` and
    `pod-acme` (wave 4).
10b. **Networks over the new crates (podcrates v0.33.0):** T-321 (`turn://`, `pod-turn`).
11. **Blocked on `PodUtils/podcrates` (a COORD issue (to podcrates) for each):** T-161 and T-200
    (`pod-relay`: a relay that serves the call; the control host only), T-244 (`pod-cloudflare`), T-326 (a release of `russh` that takes `rsa` 0.10.0); T-163 is
    partial (its edge leg waits for `pod-cloudflare`); and the link entries T-294, T-295.
12. **The link workstream (GitHub #466 to #476), after podnet's own work (M9, P3):** the open entries of `TODO/link.md`, in the order of the entries. The entries of E0 (the identity, the link, the carriers, the commands, the peer target and the control socket) are done. The carriers ws, h2, quic, udp, dns, icmp and the poll carriers are done (the glue of `pod-link` v0.40.0), and the record layer of E6; the seccomp rule of a raw socket in a pod is done (`pod --allow-raw icmp`); the ARQ is done (`podnet::link_arq`). The mesh keeps the two next hops of one round in a diamond. Go on with the carrier T-279 (it waits for an object store), the services T-294 and T-295 (blocked on `pod-egress`), the rest of E3 and E7 to E10. Each optional carrier sits behind its Cargo feature (done). Blocked: T-294 and T-295 (`pod-egress`, T-127), T-305 (the owner: Linux only, 2026-11-12). `docs/decisions.md`, 2026-11-12, The link workstream, has the calls and the asks for podcrates.
13. **The asks of cfb, after podnet's own work** (the owner, 2026-11-09, Priority: podvm, podnet and
    podssh first; cfb and githome wait):
    T-221 (`bench`), T-219 and T-220 (blocked on `pod-unit`, `pod-relay`).
14. **The asks of podswarm (PodUtils/podcrates#125), after podnet's own work (P3; each has a fallback in podswarm):**
    N-4 (one flow over several uplinks) is answered in `docs/router.md`: per flow only;
    GitHub #495 stays deferred (the owner, 2026-11-12).

Releases (2026-11-09: betas, no crates.io): cut the next prerelease (`v0.4.0-beta.N`) unattended when
a batch is green (`CLAUDE.md`).

## Parked entries

These entries stay open. The condition in each entry tells when it can start.

| Entry | Start condition |
| --- | --- |
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

- **Q1.** Is `--seal` (the end-to-end channel, T-087) the default of `relay node --resume` and `relay connect
  --resume`? It blocks nothing: T-087 is done with `--seal` off by default. The recommendation: no, keep it opt-in until
  `relay pair` gives the node fingerprint with the connect token (`docs/decisions.md`, Questions for the owner).

Write each question as `- **Qn.** QUESTION`, with the entries
that it blocks and a recommendation. Number the questions in sequence
from Q1.
