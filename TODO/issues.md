# Issues

Each GitHub issue of the record, with its entries. The State starts with the
state on GitHub: `open`, `closed` or `deleted`. `TODO/github.txt` lists the
numbers: each number from #311 on, and each open number before #311. The
record does not hold the closed issues before #311; their text is on GitHub
or in `docs/reports/`. `TODO/RULES.md` gives the rules.

| Issue | Subject | State | Entries |
| --- | --- | --- | --- |
| #89 | tracking: dev work in a sandbox with basic networking, and launch once and forget | open (deferred: the tracking issue of the owner) | T-001 |
| #140 | warp proxy: add --l4, one HTTP/3 CONNECT per flow, no IP stack | open (deferred: a captured request of an official client) | T-002 |
| #149 | warp: enrol through a Zero Trust team token | open (deferred: a captured exchange of the official client) | T-003 |
| #151 | tls: send an encrypted ClientHello on the hops that publish a config | open (deferred: no HPKE in ring, no ECH config on the hops) | T-004 |
| #161 | Publish the library and the CLI on crates.io (deferred) | open (deferred: publish nothing now) | T-005, T-007 |
| #222 | crates.io: the workspace version is a prerelease, and the beta requirement cannot match a stable pod-netns | open (deferred: the first stable release) | T-006 |
| #224 | crates.io: the crate name cfrs is registered to another project | closed (the owner chose and reserved `podnet`; the publish is #161, with T-007) | T-016 |
| #311 | live: two runs in a row failed | closed | T-136 |
| #316 | live: two runs in a row failed | closed | T-138 |
| #317 | COORD-001: a test of the fallback channel | closed (a COORD issue between sessions; no work) | none |
| #319 | [gh action] requested: a run of ci.yml | closed (a request of a run; run 42183709673) | none |
| #320 | COORD-002: podnet took pod-dns and pod-services of v0.4.0 | closed (podcrates v0.7.0 has the `taken bf0c7ca` lines) | T-137 |
| #321 | COORD-003: podnet moved each podcrates tag to v0.5.0 | closed (superseded by #333) | T-137 |
| #322 | COORD-004: pod-dns: name the DoH server in the debug line | closed (pod-dns v0.6.0; live run 42265938003 at 4094fee) | T-137 |
| #323 | [gh action] requested: a run of ci.yml | closed (run 42222918056 at 61c70aa: success) | none |
| #324 | [gh action] requested: a run of live.yml | closed (run 42222905277 at 61c70aa: failure, T-141) | none |
| #325 | live: two runs in a row failed | closed (live run 42226303514 at 3db91c7) | T-141 |
| #326 | [gh action] requested: a run of live.yml | closed (run 42226303514 at 3db91c7: success) | none |
| #327 | [gh action] requested: a run of ci.yml | closed (run 42245546264 at 581e4a5: success) | none |
| #328 | [gh action] requested: a run of ci.yml | closed (run 42253187093 at 3bf268a: success) | none |
| #329 | [gh action] requested: a run of live.yml | closed (run 42253174816 at 3bf268a: success) | none |
| #330 | [gh action] requested: a run of ci.yml | closed (not the form of the rule of 2026-11-06; #332 in its place) | none |
| #331 | [gh action] requested: a run of live.yml | closed (not the form of the rule of 2026-11-06; #332 in its place) | none |
| #332 | [gh action] requested: a run of ci.yml and live.yml | closed (ci 42259197563 and live 42259203680 at 0c4ff75: success) | none |
| #333 | COORD-005: podnet moved each podcrates tag to v0.6.0 | closed (ci 42263403147 at 872a276, live 42265938003 at 4094fee: success) | T-137 |
| #334 | [gh action] requested: a run of ci.yml and live.yml | closed (ci 42263403147 at 872a276: success; live did not start) | none |
| #335 | [gh action] requested: a run of live.yml | closed (live 42264247691 at 872a276: failure, the form of the grep; 4094fee) | none |
| #336 | [gh action] requested: a run of live.yml | closed (live 42265938003 at 4094fee: success) | none |
| #337 | [gh action] requested: a run of ci.yml and live.yml | closed (ci 42272728934 at 68098d1: failure, a test with no need_loopback; dfded5b) | none |
| #338 | [gh action] requested: a run of live.yml | closed (live 42275332991 at 68098d1: success) | none |
| #339 | [gh action] requested: a run of ci.yml | closed (ci 42278745845 at dfded5b: success) | none |
| #340 | [gh action] requested: a run of ci.yml | closed (ci 42283721494 at 093938f: success) | none |
| #341 | [gh action] requested: a run of ci.yml | closed (ci 42286291297 at 8a3ec2c: success) | none |
| #342 | [gh action] requested: a run of ci.yml | closed (ci 42290515084 at b385489: success) | none |
| #343 | [gh action] requested: a run of ci.yml | closed (ci 42292498940 at 416574b: success) | none |
| #344 | [gh action] requested: a run of live.yml | closed (live 42292490990 at 416574b: success) | none |
| #345 | [gh action] requested: a run of ci.yml | closed (ci 42296502041 at c52b8e0: success) | none |
| #346 | [gh action] requested: a run of live.yml | closed (live 42296493409 at c52b8e0: success) | none |
| #347 | seccomp: write(2) on UDP dropped; closed UDP leaks 3 fds | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-142, T-143 |
| #348 | seccomp: UDP bind records nothing; getsockname port 0 | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-144 |
| #349 | fake-IP pool wraps at 131072; names case-sensitive | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-145 |
| #350 | seccomp UDP: AF_UNSPEC, source filter, sockopts, ICMP | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-146, T-147, T-148, T-149 |
| #351 | seccomp TCP copied in user space | closed | T-150, T-151 |
| #352 | netns: in-namespace netfilter in place of smoltcp | closed (ci 43459872995 at bf01ac6) | T-152, T-153 |
| #353 | no fuzz targets for seccomp handlers | closed | T-154, T-155 |
| #354 | pod rules: no reject action (also no `block`) | closed (ci 42767983253, live 42767982942 at 5bf153d) | T-156 |
| #355 | pod: default-deny, flow log, learn | closed | T-157, T-158 |
| #356 | research documents cite old paths | closed (ci 42638635724, live 42638635442 at dbfcfd6) | T-159 |
| #357 | verify: young name exits 69, tears down a working tunnel | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-160 |
| #358 | relay node: pair cannot be renewed | open | T-161 |
| #359 | relay alternates chosen once (cost is the 2 s stagger) | closed | T-162 |
| #360 | relay session budget is a constant | open | T-163 |
| #361 | `--no-verify` undocumented, name misleads | closed (ci 42638635724, live 42638635442 at dbfcfd6) | T-164 |
| #362 | ssh relay leg: no supervision (no caller) | closed | T-165 |
| #363 | ssh relay leg: one relay, one user | closed | T-166 |
| #364 | net socksify: no per-destination routing | closed (ci 42767983253, live 42767982942 at 5bf153d) | T-167 |
| #365 | fakepwd only via ts-shims (partly: it works, gap is name/docs) | closed (ci 42767983253, live 42767982942 at 5bf153d) | T-168 |
| #366 | tunnel: no `--masque-endpoint`; v4 door first | closed | T-169 |
| #367 | provision retries DNS; verify has no pin path | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-170, T-171, T-172 |
| #368 | WARP identity of another uid refused | closed | T-173 |
| #369 | netns veth wire without /dev/net/tun | closed | T-174 |
| #370 | one TLS connection per DoH query | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-175 |
| #371 | TCP flow terminators (design limits, no present defect) | closed | T-176 |
| #372 | router datagram carrier | closed (ci 43459872995 at bf01ac6) | T-098, T-099, T-111 |
| #373 | router health: hysteresis, cooldown | closed (ci 43459872995 at bf01ac6) | T-096, T-097 |
| #374 | router probes via own member; resolver | closed (ci 43459872995 at bf01ac6) | T-097, T-098 |
| #375 | `route check`, group policies, direction split | open | T-177, T-178, T-179 |
| #376 | router control, typed selection, seccomp budget | closed (ci 43459872995 at bf01ac6) | T-096, T-180, T-181 |
| #377 | Transport trait addressing; gsr transport | closed | T-055, T-058 |
| #378 | doctor --agent checks index host, not file host | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-182 |
| #379 | http:// proxy default port 8080 vs 80 | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-183 |
| #380 | doctor relay row from PODNET_RELAY only (partly) | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-184 |
| #384 | net proxy handshake has no deadline (pod --serve too) | closed (ci 43382314031 at 9583ea3) | T-061, T-185 |
| #385 | net tailscale unix bind deletes a live path (frontdoor too) | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-186 |
| #386 | net proxy --forward dials direct (partly disputed) | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-187 |
| #387 | --forward prints before bind; bind failure only logged | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-188 |
| #388 | vnet switch: uncounted drops, unbounded queues | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-189, T-190 |
| #389 | net doctor listener table incomplete | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-191, T-192 |
| #390 | net proxy --map reaches the door only | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-193 |
| #391 | relay node: close drops the abort handle | closed (ci 42641451152, live 42641450984 at e784949) | T-194 |
| #392 | relay stop: wrong order | closed (ci 42641451152, live 42641450984 at e784949) | T-195 |
| #393 | relay node: no drain after local EOF | closed (ci 42641451152, live 42641450984 at e784949) | T-196 |
| #394 | relay pair file: mode and read checks | closed (ci 42641451152, live 42641450984 at e784949) | T-197 |
| #395 | docs say 6 seed hosts, client takes 3 | closed (ci 42638635724, live 42638635442 at dbfcfd6) | T-198 |
| #396 | relay connect --listen: no --detach, record, restart | closed (ci 42641451152, live 42641450984 at e784949) | T-199 |
| #397 | relay reverse road uses host 0 only | open | T-200 |
| #398 | WebSocketTunnel dials direct (latent, no caller) | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-201 |
| #399 | serve: root_404 never set | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-202 |
| #400 | share --json: no schema, no podnet key | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-203 |
| #401 | tunnel --config drops origin flags | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-204 |
| #402 | detach: nested start inherits the env (only --no-restart) | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-205 |
| #403 | agent status has no --json | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-206 |
| #404 | static: `+` decoded differently (one more divergence) | closed (ci 42710206231, live 42710206544 at 93bafc2) | T-207 |
| #405 | warp up --tun drops --replace and others | closed (ci 42641451152, live 42641450984 at e784949) | T-208 |
| #406 | warp status never names warp-tun | closed (ci 42641451152, live 42641450984 at e784949) | T-209 |
| #407 | warp: every type but "free" reads as WARP+ (partly) | open | T-210 |
| #408 | warp down removes the fish file without an owner check | closed (ci 42641451152, live 42641450984 at e784949) | T-211 |
| #409 | README says --l4 ignores rules; code applies them | closed (ci 42638635724, live 42638635442 at dbfcfd6) | T-212 |
| #410 | WireGuard endpoint: v6 and ports unread | closed (ci 42641451152, live 42641450984 at e784949) | T-213 |
| #411 | warp ssh-config: path not quoted | closed (ci 42641451152, live 42641450984 at e784949) | T-214 |
| #412 | warp proxy --dns: one task per datagram | closed (ci 42641451152, live 42641450984 at e784949) | T-215 |
| #413 | COORD-006: asks of podnet for pod-relay, pod-dns, pod-proxy and pod-services | open (a COORD issue between sessions; 2026-11-12: asks 2, 3 and 11 hold in `pod-relay` v0.30.0, T-162, T-083 done and T-163 partial; asks 1 and 4 wait, T-161 and T-200) | T-083, T-161, T-162, T-163, T-166, T-172, T-175, T-182, T-200 |
| #414 | COORD-007: podnet took v0.7.0 and the parts of pod-core; the asks of cfb; the view on pod-hold | closed (podcrates c229926 recorded it) | T-137, T-216 to T-222 |
| #416 | COORD-008: podnet took v0.8.0; the freeze of the datagram path of upstream.rs is recorded | closed (podcrates recorded it) | T-137 |
| #417 | COORD-009: podnet took v0.9.0; podnet-netns calls pod_proxy::udp, so the freeze of #42 can end | closed (podcrates recorded it) | T-137, T-224 |
| #418 | COORD-010: podnet took v0.10.0 | closed (podcrates recorded it) | T-137 |
| #419 | COORD-011: podnet took v0.11.0; podnet has a new crate podnet-core | closed (podcrates recorded the later takes) | T-137, T-047 |
| #420 | COORD-012: podnet took pod-ws of v0.13.0 | closed (podcrates recorded it) | T-137 |
| #421 | COORD-013: podnet took pod-quic of v0.14.0 | closed (podcrates recorded it) | T-137, T-228 |
| #422 | COORD-014: the podrunner entries and the time of pod-hold | closed (podvm closed its COORD-022, PodUtils/podvm#228) | T-229, T-230, T-231, T-232 |
| #423 | live: two runs in a row failed (the new steps of the file server and git) | closed | T-091, T-094 |
| #424 | COORD-015: podnet took pod-hold of v0.15.0; the answers to Q-a and Q-b of COORD-030 | closed (podcrates recorded it) | none |
| #425 | COORD-016: T-229 measured, T-230 has the hold object, T-231 item 1 done (to podrunner) | closed (the report was delivered; T-231 item 2 is in the record) | none |
| #426 | COORD-017: podnet took v0.17.0 and pod_hold::connect and pod_hold::packet | closed (podcrates recorded it: taken e4d4049) | none |
| #427 | COORD-018: the cut of pod-netstack (to podcrates) | closed (podnet took pod-netstack at d23ce4e) | none |
| #428 | pod --serve door: early reply and pipelined SOCKS5 upstream handshake (donor review) | closed | T-237 |
| #429 | listen::bind_private: the socket has the umask mode between bind and chmod (donor review) | closed (fixed in 3c6173f) | T-238 |
| #430 | install.sh and update: tag check, strict SHA256SUMS, install tests that can fail (donor review) | closed | T-239 |
| #431 | warp tun --default-route: resolver backup on disk, restore after a kill, route table test (donor review) | closed | T-240 |
| #432 | warp: registration under a lock, save before the enrolment step (donor review) | closed | T-241 |
| #433 | rules: a shadowed rule warns, a shadowed reject fails (donor review) | closed (T-242: the pod and the router in e7ec95c and dc5d221, warp proxy in the next commit) | T-242 |
| #434 | net tailscale: --tailnet guard and the TS_FORCE_NOISE_443 hint (donor review) | closed | T-243 |
| #435 | tunnel --inspect: capture, ring, redaction, replay (donor review) | open (T-244: its capture hook goes into pod-cloudflare, which podnet took) | T-244 |
| #436 | podnet receive: containment of the received entries (donor review) | closed (the rules of the Correction of T-105 are `src/transfer/land.rs`, d5c38c1; `cargo test --test transfer_land`: 7 passed) | T-105 |
| #437 | COORD-019: proposals for podcrates from the donor review (to podcrates) | closed (2026-11-23: the coordinator of podcrates recorded the six proposals as PodUtils/podcrates T-189 to T-194; podcrates owns them) | none |
| #438 | COORD-020: proposals for podssh from the donor review (to podssh) | closed (podssh recorded them in its record, PodUtils/podssh#188) | none |
| #439 | COORD-021: proposals for podrunner from the donor review, part 1 of 2 (to podrunner) | closed (the proposals were delivered; podrunner owns them) | none |
| #440 | COORD-022: proposals for podrunner from the donor review, part 2 of 2 (to podrunner) | closed (the proposals were delivered; podrunner owns them) | none |
| #441 | COORD-023: proposals for the stub repositories from the donor review, part 1 of 3 (to all) | closed (a COORD issue between sessions; no entry, as CLAUDE.md says) | none |
| #442 | COORD-024: proposals for the stub repositories from the donor review, part 2 of 3 (to all) | closed (a COORD issue between sessions; no entry, as CLAUDE.md says) | none |
| #443 | COORD-025: proposals for the stub repositories from the donor review, part 3 of 3 (to all) | closed (a COORD issue between sessions; no entry, as CLAUDE.md says) | none |
| #444 | COORD-026: pod-broker bind_unix has the umask window of bind_private (to podcrates) | closed (a COORD issue between sessions) | none |
| #445 | COORD-027: podnet took pod-netstack of v0.20.0 (to podcrates) | closed (podcrates recorded it: taken d23ce4e) | none |
| #446 | COORD-028: podnet changed three frozen files: register.rs, route.rs (fixes) and netns/tun.rs (the veth wire) (to podcrates) | closed (podcrates answered in its #68: no port; the wire stays in veth.rs, podcrates T-201) | none |
| #447 | COORD-029: the answer to podcrates #67 (pod-wg): wg.rs is held to fixes; the split (to podcrates) | closed (podcrates wrote taken 089e1c9) | none |
| #448 | COORD-030: podnet took pod-services-watch and the crates of v0.21.0 (to podcrates) | closed (podcrates recorded the take) | none |
| #449 | COORD-031: podnet freezes the files of pod-relay, pod-warp and pod-cloudflare at 7c5f87e; asks for T-028 and T-029 (to podcrates) | closed (podcrates recorded it, its #74) | none |
| #450 | COORD-032: the file-transfer of magic-wormhole has no owner; no hand-off for pod-session, pod-e2e, pod-identity (to podcrates) | closed (podcrates T-214 and T-215, its #74) | none |
| #451 | COORD-033: pod-hold: the veth wire is done, the netns router moves onto pod-netstack, a root CI step runs the tests (to podcrates) | closed (a COORD issue between sessions) | none |
| #452 | COORD-034: the answer to podcrates #74: the words of podnet's transit hints; the traits of the registry; the lint (to podcrates) | closed (podcrates took it into its T-214 and T-028, its #75) | none |
| #453 | COORD-035: a new shared crate pod-git: a git server with no installed git; the contract and the acceptance of podnet (to podcrates) | closed (podcrates opened its T-217 to T-220, its #75) | none |
| #454 | COORD-036: pod-netstack needs four APIs before the netns router can move onto it (podnet T-249) (to podcrates) | closed (podcrates v0.25.0 has them; taken at e64fb99) | none |
| #455 | COORD-037: the facts of the SSH forward-relay leg as data of pod-services (podnet T-166) (to podcrates) | closed (a COORD issue between sessions) | none |
| #456 | COORD-038: podnet took podcrates v0.25.0; the mapped loopback address of the seccomp hold is fixed (to podcrates, to podrunner) | closed (a COORD issue between sessions) | none |
| #457 | COORD-039: pod-netstack v0.25.0: three gaps that the netns router hits; podnet T-249 waits (to podcrates) | closed (a COORD issue between sessions) | none |
| #458 | COORD-040: podnet took pod-hold-veth of podcrates v0.26.0; the fork finding of doctor.rs is fixed (to podcrates) | closed (podcrates wrote taken f107766) | none |
| #459 | Docs: stale or misleading comments and records (net.rs, bug_report.rs, TODO files) | closed | T-258 |
| #460 | COORD-041: podnet took the SSH relay facts (T-166); a comment fix in the frozen src/tunnel.rs; the pinggy URL rule (to podcrates) | closed (podcrates recorded the take; T-166 done) | none |
| #461 | cfb asks: tracker for the 6 asks of cfb owned by podnet (from podcrates#35) | open | T-216, T-217, T-218, T-219, T-220, T-221, T-260 |
| #462 | COORD-042: a change in the frozen src/doctor.rs for the lanes of A-C-10 (T-260) (to podcrates) | closed (a COORD issue between sessions) | none |
| #463 | COORD-043: a weekly watch of the relay contract in pod-relay (podnet T-134) (to podcrates) | closed (a COORD issue between sessions) | none |
| #464 | COORD-044: five more gaps of pod-netstack for the guest-wire router (podnet T-176, T-248) (to podcrates) | closed (a COORD issue between sessions) | none |
| #465 | COORD-045: podnet took v0.27.0 at 28c0e3d (to podcrates) | closed (a COORD issue between sessions) | none |
| #466 | link: peer identity, session, carrier registry, and the link command (E0) | closed | T-261, T-262, T-263, T-264, T-265, T-266, T-267 |
| #467 | link: stream carriers and availability (E1) | closed (each entry done; the last is T-273, bfdc2ff) | T-268, T-269, T-270, T-271, T-272, T-273, T-274, T-275 |
| #468 | link: datagram reliability, ARQ, and simplex carriers (E2) | closed (each entry done; the last are T-278, bfdc2ff, and T-279, bcfae9e) | T-276, T-277, T-278, T-279, T-280 |
| #469 | link: DNS stream, ICMP, WireGuard and NAT traversal carriers (E3) | open | T-281, T-282, T-283, T-284, T-334 |
| #470 | link: mesh announcements, path-vector routing, forwarding and ACLs (E4) | closed (each entry done; the last are T-285, T-287, T-289 and T-335 on 2026-11-13) | T-285, T-286, T-287, T-288, T-289, T-290, T-335 |
| #471 | link: services at a peer (SOCKS5, HTTP, raw, forward, Shadowsocks, Trojan, exec, serve-at) (E5) | open | T-266, T-291, T-292, T-293, T-294, T-295, T-296, T-297 |
| #472 | link: record layer (AEAD, compression, padding, replay and resume) (E6) | closed: T-298 to T-301 done; ci 43491819082 on d53df25 | T-298, T-299, T-300, T-301 |
| #473 | link: direction split and chain composition (E7) | closed (each entry done; the last is T-302, 3623ec5) | T-302, T-303 |
| #474 | link: C ABI (E8) | closed (ci 43382314031 at 9583ea3) | T-304 |
| #475 | link: portable build subset (E9) | closed (not planned: the owner removed its entry 305 on 2026-11-16; the ruling "Platforms: Linux only" stands; a later ask for a platform is a new issue; `docs/decisions.md`, 2026-11-16, The portable build) | T-396 |
| #476 | link: CLI query keys, control plane, aliases, dump, components and build features (E10) | closed (ci 43382314031 at 9583ea3) | T-306, T-307, T-308, T-309, T-310, T-311 |
| #477 | COORD-046: podnet took v0.28.0 at b08e06f (to podcrates) | closed (a COORD issue between sessions) | none |
| #478 | COORD-047: podnet took v0.29.0 and pod-hold syn-hold at 98d8a71 (to podcrates) | closed (a COORD issue between sessions) | none |
| #479 | COORD-048: asks for the link workstream of podnet (GitHub #466 to #476) (to podcrates) | closed (a COORD issue between sessions) | none |
| #480 | COORD-049: podnet starts the link workstream: pod-link with the unix and memory carriers first (to podcrates) | closed (a COORD issue between sessions) | none |
| #481 | COORD-050: podnet took v0.30.0 and pod-relay at 83e0c93 (to podcrates) | closed (a COORD issue between sessions) | none |
| #482 | COORD-051: two gaps of pod-relay after the take (pair renewal, pair on a second host) and the edge budget (to podcrates) | closed (a COORD issue between sessions) | none |
| #483 | live: two runs in a row failed | closed | T-315 |
| #484 | [DEFERRED] Carry a session in NTPv4 extension fields, SMTP and IMAP | closed (consolidated onto `main` with `podcrates` `v0.82.0`; T-365 done) | T-365 |
| #485 | [DEFERRED] Carry a session through a TURN allocation, with RFC 6062 | closed (consolidated onto `main` with `podcrates` `v0.82.0`; T-366 done) | T-366 |
| #486 | [DEFERRED] Close four QUIC gaps: MASQUE enrolment, HTTP/3 shape, version, connection ID | open (T-367 partial: HTTP/3 shape, TUIC, Juicity, rotation done; MASQUE QUIC-1 to QUIC-5 remain) | T-367 |
| #489 | [DEFERRED] Register third-party services as roads, with a watch, a date and a count | open (deferred by the owner, 2026-11-12: T-368 blocked on outside endpoints) | T-368 |
| #491 | [DEFERRED] Switch after a real handshake, with a cover script and multiplexed cover | closed (consolidated onto `main` with `pod-cover` `v0.82.0`; T-369 done) | T-369 |
| #493 | [DEFERRED] Shape the record layer: padding profiles, entropy, first flight, pacing | closed (consolidated onto `main` with `pod-shape` `v0.82.0`; T-370 done) | T-370 |
| #494 | [DEFERRED] Shape the application: deferred responses, a generator, carriers in the payload | closed (consolidated onto `main`; T-371 done) | T-371 |
| #495 | [DEFERRED] Add a packet sidecar: desync search, disagreement probe, fake TCP, bonding | closed (consolidated onto `main` with `pod-sidecar` `v0.82.0`; T-372 done) | T-372 |
| #496 | [DEFERRED] Keep the session end to end: opaque destination and keys from the ends | closed (consolidated onto `main` with `pod-e2e` `v0.82.0`; T-373 done) | T-373 |
| #497 | [DEFERRED] Make the pool live, the budget per deployment, tokens expiring, hosts movable | open (deferred by the owner, 2026-11-12: T-374 blocked on relay server deployment) | T-374 |
| #498 | [DEFERRED] Resume a session across hours, and serve the full service surface at a peer | open (deferred by the owner, 2026-11-12: T-375 blocked on `pod-session` multi-hour resume) | T-375 |
| #499 | [DEFERRED] Carry a session with no network: serial, HF, spool, discovery, pairing | closed (consolidated onto `main` with `pod-offline` `v0.82.0`; T-376 done) | T-376 |
| #500 | [DEFERRED] State the host runtime: no open port, classified egress, stated limits | closed (consolidated onto `main`; T-377 done) | T-377 |
| #501 | [DEFERRED] Build the verification harness: classifier, exit codes, conformance matrix | closed (consolidated onto `main` with `pod-harness` `v0.82.0`; T-378 done) | T-378 |
| #502 | COORD-052: podnet took v0.31.0 at e046560; relay connect hangs on a direct runner after the take of pod-relay (to podcrates) | closed (a COORD issue between sessions) | none |
| #503 | net: add a Dev Tunnels network for TCP streams | closed | T-320 |
| #504 | doctor: resolve the ssh-relay probe through the configured proxy | closed | T-322 |
| #505 | ssh-relay: try the authentication methods the relay offers | closed | T-318 |
| #506 | ssh-relay: take the remote port from the service data | closed (T-319, commit 3e4c999) | T-319 |
| #507 | net: add a TURN network for UDP flows | open (T-321 partial: the UDP half is done; the TCP relay, the doctor row and an account for each network stay) | T-321 |
| #508 | COORD-053: podnet took v0.32.0 at fa6b5ba; pod-link is next (to podcrates) | closed (a COORD issue between sessions) | none |
| #509 | COORD-054: the mapped loopback address is now dialed direct (T-317); it comes in v0.4.0-beta.7 (to podrunner) | closed (a COORD issue between sessions; podrunner run 43337313388 passed) | none |
| #510 | COORD-055: asks from GitHub #503 to #507: pod-devtunnel, pod-turn, a remote-port fact, the Serveo URL rule (to podcrates) | closed (a COORD issue between sessions) | none |
| #511 | live: two runs in a row failed | closed | T-323 |
| #512 | COORD-056: podnet takes pod-identity and pod-session (T-261, T-263); three small asks (to podcrates) | closed (a COORD issue between sessions) | none |
| #513 | COORD-057: podnet starts the network carriers of the link: TCP with TLS first (T-268; your T-304, A7) (to podcrates) | closed (a COORD issue between sessions) | none |
| #514 | COORD-058: podnet took v0.33.0 at 797104f (to podcrates) | closed (a COORD issue between sessions) | none |
| #515 | COORD-059: podnet took v0.34.0 at ef8c901; the TCP carrier is in; the next carriers of pod-link (to podcrates) | closed (a COORD issue between sessions) | none |
| #516 | live: two runs in a row failed | closed (quick-tunnel edge flakes; the rerun 43333498755 passed) | T-364 |
| #517 | COORD-060: the answer to the asks N-1 to N-5 of podswarm (PodUtils/podcrates#125) (to all) | closed: each ask is in `v0.4.0-beta.10` and later | none |
| #518 | COORD-061: port rules in the rules reader of pod-warp: port:N=WAY and tcp:HOST:PORT=WAY (podnet T-328; ask N-2 of podswarm) (to podcrates) | closed (T-328 done, f5bab50) | none |
| #519 | COORD-062: podnet took v0.35.0 at 5562f21 (to podcrates) | closed (a COORD issue between sessions) | none |
| #520 | COORD-063: podnet took v0.36.0 at 775486f (to podcrates) | closed (a COORD issue between sessions) | none |
| #521 | COORD-064: what podnet waits for to close its issues: the datagram carriers of pod-link, pod-e2e, pod-wormhole, pod-warp's freeze and the others (to podcrates) | closed (answered by PodUtils/podcrates#133) | none |
| #522 | COORD-065: podnet took v0.37.0 and pod-warp at f6abdc3; src/warp/ holds fixes only (to podcrates) | closed (podcrates recorded the take) | none |
| #523 | live: two runs in a row failed | closed: live 43459893595 on bf01ac6 passed | T-332 |
| #524 | COORD-066: a fix in src/warp/quic.rs: WarpNet's datagrams bind [::], so QUIC to an IPv4 edge goes as IPv4-mapped and times out (to podcrates) | closed: live 43459893595 on bf01ac6 passed with the fix of pod-warp v0.39.0 | none |
| #525 | COORD-067: podnet took v0.38.0 (to podcrates) | closed (a COORD issue between sessions) | none |
| #526 | COORD-068: podnet took v0.39.0 at 306d80b; the one-family fix of WARP is dropped (to podcrates) | closed (podcrates recorded the take) | none |
| #527 | COORD-069: the answer to the question on KCP: the congestion window by default; the 20 % loss gate runs with nodelay (podnet T-276) (to podcrates) | closed: the kcp profile and the feature udp are in d474725; ci 43491819082 on d53df25 | none |
| #528 | [gh action] requested: ci, deny, secrets and live on b1e2b80 (Actions does not start on a push of the cloud session) | closed: ci 43445945197 and live 43445966561 failed (fixed in 2122bd4), deny and secrets passed | none |
| #529 | [gh action] requested: ci, deny, secrets and live on 7d73ec7 | closed: ci 43452891413 failed at restricted-host (fixed in the next commit), live 43452912977 failed at the file server (fixed in 098ff8d), deny and secrets passed | none |
| #530 | [gh action] requested: ci, deny, secrets and live on bf01ac6 | closed: each run passed (ci 43459872995, deny 43459879363, secrets 43459886257, live 43459893595) | none |
| #531 | [gh action] requested: ci, deny, secrets and live on 6f8e69e (T-012, T-095, T-087) | closed: each run passed (ci 43471936738, deny 43471944208, secrets 43471952954, live 43471960138) | none |
| #532 | COORD-070: podnet took v0.40.0 at 1c88531 (to podcrates) | closed (podcrates recorded the take) | none |
| #533 | COORD-071: two asks of the carriers of pod-link: a typed refusal of the key, and the TC rule of the dns carrier (to podcrates) | closed: v0.45.0 carries both asks; the exit 77 at each listener is tested in bfdc2ff (T-273, T-278) | none |
| #534 | [gh action] requested: ci, deny, secrets and live on d53df25 | closed: ci 43491819082, deny 43491826156 and secrets 43491833085 passed; live 43491840609 failed at the new router step (fixed in c02da54) | none |
| #535 | COORD-072: the asks of the transfer of podnet over pod-wormhole: the derivation of a key, a stand-in mailbox, the count of words, the TCP transit relay (to podcrates) | closed: v0.42.0 carries asks 1 to 4; ask 5 rides on their entry 214 | none |
| #536 | [gh action] requested: ci, deny, secrets and live on 0dd7663 | closed: deny 43499858663 and secrets 43499865056 passed; ci 43499850487 failed (the transfer tests have no loopback guard) and live 43499872018 failed at the router step (w stayed up) | none |
| #537 | live: two runs in a row failed (the router step, runs 43491840609 and 43499872018) | closed: live 43521809728 on 3061c7c passed | T-101 |
| #538 | COORD-073: pod-hold: a seccomp rule for a raw ICMP socket (podnet T-334) (to podcrates) | closed: v0.42.0 and v0.44.0 carry it (T-334, T-350) | none |
| #539 | [gh action] requested: ci, deny, secrets and live on c8a9510 | closed: ci 43506563209, deny 43506569632 and secrets 43506579272 passed; live 43506586024 failed at the router step (w stays up) | none |
| #540 | COORD-074: pod-link: dgram_tls does not call rekeyed, so a dns or icmp link stops at the wrap of the ARQ (podnet T-277) (to podcrates) | closed: v0.42.0 carries their entry 541 | none |
| #541 | [gh action] requested: ci, deny, secrets and live on 54f0809 | closed: ci 43515459301, deny 43515464237 and secrets 43515470028 passed; live 43515475108 failed at the router step (the WARP session took the relay leg) | none |
| #542 | [gh action] requested: ci, deny, secrets and live on 3061c7c | closed: each run passed (ci 43521791090, deny 43521797247, secrets 43521803788, live 43521809728) | none |
| #543 | [gh action] requested: the release of v0.4.0-beta.10 at 43c53a6 | closed: stopped at ci 43535662399 (restricted-host, a race in a test); deny, secrets and live passed; nothing published | T-336 |
| #544 | [gh action] requested: the release of v0.4.0-beta.10 at 59cf78c | closed: ci, deny, secrets and live passed; the dry run 43541138625 failed on the 32-bit targets (nfnl.rs); nothing published | T-336 |
| #545 | [gh action] requested: the release of v0.4.0-beta.10 at e99eba6 | closed: each run passed; the release v0.4.0-beta.10 is out (release-proofs 43549615088) | T-336 |
| #546 | COORD-075: podnet took v0.41.0 at 095955c (to podcrates) | closed (podcrates closed PodUtils/podcrates#143) | none |
| #547 | COORD-076: pod-wg: a responder sans I/O, and the preshared key (podnet T-125, T-337) (to podcrates) | closed: podcrates v0.43.0 carries both asks; T-125 and T-337 are done | none |
| #548 | [gh action] requested: ci, deny, secrets and live on 3d54f76 (T-117) | closed: each run passed on 036d23c (ci 43551002812, deny 43551006710, secrets 43551010275, live 43551013239) | none |
| #549 | [gh action] requested: ci, deny, secrets and live on 64b0c8b (T-120, T-248) | closed: each run passed on a93a37a (ci 43553742871, deny 43553746054, secrets 43553749542, live 43553752592) | none |
| #550 | COORD-077: podnet took v0.42.0 at 654c0f1; two notes on pod-hold and pod-services (to podcrates) | closed: v0.44.0 carries note 1 and note 2; podnet takes it at 6e9e9ee (T-350) | none |
| #551 | [gh action] requested: ci, deny, secrets and live on eddaa22 | closed: each run passed on c72c604 (ci 43560703081, deny 43560707016, secrets 43560710399, live 43560713372) | none |
| #552 | COORD-078: podnet took v0.43.0 at c8ccc40; pod-netstack: bind_udp with port 0 (to podcrates) | closed: v0.45.0 carries the ask; podnet binds port 0 in 2f8180c (T-117) | none |
| #553 | [gh action] requested: ci, deny, secrets and live on b3eaba6 (v0.43.0, T-337, T-125) | closed: each run passed on b92c100 (ci 43567820518, deny 43567823619, secrets 43567826340, live 43567829232) | none |
| #554 | [gh action] requested: ci, deny, secrets, live and fuzz on fbe4235 (T-339, T-340, T-316, T-106) | closed: the runs are on 9610316 (`main` moved before the block ran): ci 43590942248, deny 43590945244, secrets 43590947996 and live 43590951009 passed; fuzz 43590953876 failed with two crashes, fixed in 54637d9 (T-348) and 243eb48 (T-349) | T-348, T-349 |
| #555 | COORD-079: two crashes of podnet's fuzz run: a credential in the label of a unix proxy (pod-proxy), and a list past its segment (capnp of pod-cloudflare) (to podcrates) | closed (answered: PodUtils/podcrates#156 is closed; part 1 in podcrates v0.45.0, T-349; part 2 by the take of pod-cloudflare, T-423) | none |
| #558 | COORD-080: podnet took v0.44.0 at 6e9e9ee (to podcrates) | closed (podcrates closed PodUtils/podcrates#154) | none |
| #559 | [gh action] requested: ci, deny, secrets, live and fuzz on 79dfa13 (T-341 to T-350, v0.44.0) | closed: the runs are on 3ad5315 (`main` moved before the block ran): ci 43603454896, deny 43603458277 and fuzz 43603470175 passed; secrets 43603461673 and live 43603465996 failed, fixed in ce14ac4 | T-349 |
| #560 | COORD-081: podnet took v0.45.0 at e84ebdf (to podcrates) | closed (podcrates recorded the take) | none |
| #561 | [gh action] requested: ci, deny, secrets, live and fuzz on 7860081 (ce14ac4, v0.45.0) | closed: deny 43610669298, live 43610676971 and fuzz 43610680236 passed on 7860081; ci 43610665702 (clippy `err_expect` under `--features full`) and secrets 43610673639 (the test vector of T-349 in the allow line and in TODO/fixes.md) failed, fixed in 42943fa | T-273, T-278, T-349 |
| #562 | [gh action] requested: ci and secrets on the fix of #561 | closed: ci 43617081662 and secrets 43617085112 passed on 3b3b9be | none |
| #563 | [gh action] requested: the release of v0.4.0-beta.11 (checks, dry run, publish, release proofs) | closed: `v0.4.0-beta.11` is out on 6491f9d (release-proofs 43623174074) | T-351 |
| #564 | COORD-082: podnet took v0.46.0 at 27cdf19; E7-1: podnet confirms the call C493 (to podcrates) | closed (podcrates closed PodUtils/podcrates#159; the ask of the stand-in is done in v0.47.0, 556708a) | none |
| #565 | [gh action] requested: ci, deny, secrets, live and fuzz on the take of v0.46.0 (T-137, T-279, T-302) | closed: ci 43632069484, deny 43632072542, secrets 43632076272 and fuzz 43632083716 passed on b7c3e90; live 43632080404 failed: each WARP proxy download of speed.cloudflare.com gave 1 byte and no warning (no code of the WARP path changed since the pass of 43619120794); the step now prints the HTTP status and the start of a short body | none |
| #566 | [gh action] requested: live again (the WARP proxy download of #565) | closed: live 43638808808 passed on 5db015c | none |
| #567 | [gh action] requested: the release of v0.4.0-beta.12 (checks, dry run, publish, release proofs) | closed: `v0.4.0-beta.12` is out on b6e7959 (release-proofs 43645375545) | T-352 |
| #568 | COORD-083: podnet took v0.47.0 at 1d4d7f1; the stand-in of the S3 API at 556708a (to podcrates) | closed (podcrates closed PodUtils/podcrates#160) | none |
| #569 | [gh action] requested: ci, deny, secrets, live and fuzz on the take of v0.47.0 | closed: ci 43654435897, deny 43654439784, secrets 43654443103, live 43654446240 and fuzz 43654449404 passed on 6536905 | none |
| #570 | COORD-084: podnet took v0.48.0 at 862a6a9 (to podcrates) | closed (podcrates closed PodUtils/podcrates#162) | none |
| #571 | [gh action] requested: ci and deny on the take of v0.48.0 | closed: ci 43659845378 and deny 43659848188 passed on 34bb2cd | none |
| #572 | COORD-085: podnet took v0.49.0; a closed half can end the listener of a split session with "the carrier of the link ended" (T-353) (to podcrates) | closed: the take is recorded (PodUtils/podcrates#163); v0.51.0 carries the fix of T-353 | T-353 |
| #573 | [gh action] requested: ci and deny on the take of v0.49.0 | closed: ci 43664113413 and deny 43664116501 passed on 0bda088 | none |
| #574 | COORD-086: podnet took v0.50.0 (to podcrates) | closed (podcrates closed PodUtils/podcrates#164) | none |
| #575 | [gh action] requested: ci and deny on the take of v0.50.0 | closed: ci 43673745417 and deny 43673748869 passed on d072007 | none |
| #576 | [gh action] requested: ci, deny and secrets on T-354 (devtunnel:// with an access token or no token) | closed: ci 43705620683, deny 43705623914 and secrets 43705627029 passed on 08802d8 | T-354 |
| #577 | COORD-087: podnet took v0.51.0; the fix of #572 holds (to podcrates) | closed (podcrates closed PodUtils/podcrates#172) | none |
| #578 | [gh action] requested: ci and deny on the take of v0.51.0 | closed: ci 43708707139 and deny 43708710128 passed on f1327d6 | none |
| #579 | [gh action] requested: the release of v0.4.0-beta.13 (checks, dry run, publish, release proofs) | closed: `v0.4.0-beta.13` is out on 0a0f3e5 (release-proofs 43716868782) | T-355 |
| #580 | COORD-088: podnet took v0.52.0 (to podcrates) | closed (podcrates closed PodUtils/podcrates#176) | none |
| #581 | [gh action] requested: ci and deny on the take of v0.52.0 | closed: ci 43718529045 and deny 43718532371 passed on f6aaeca | none |
| #582 | COORD-089: the vnet shim for a .NET payload in a podvm unit: TCP to a unix socket (to podrunner) | closed (T-356 and T-357 done) | T-356, T-357 |
| #583 | [gh action] requested: ci, deny and secrets on T-356 and T-357 (the vnet shim) | closed: ci 43728239022, deny 43728242664 and secrets 43728245947 passed on 78a3911 (read from the runs; no comment came) | none |
| #584 | COORD-090: one shared crate for the help and the manual of each command line; podnet's src/manual as a source (to all) | closed (podcrates made pod-manual, podnet took it in T-359; podssh answered in PodUtils/podssh#267, podrunner in PodUtils/podrunner#181) | T-359 |
| #585 | COORD-091: podnet's answer to pod-lab and pod-manual: the host classes of podnet's users, the network profiles, the freeze of src/manual (to podcrates) | closed (2026-11-23: each ask to podcrates is done: pod-manual taken in T-359, the host classes and the network profiles in podcrates v0.55.0 to v0.60.0; the release gate stays in T-358) | none |
| #586 | COORD-092: podnet's answer to the rulings Issues and Installer: the map of issues to entries, the external reports, scripts/install.sh (to podcrates) | closed (T-360 done) | T-360 |
| #587 | COORD-093: podnet took v0.53.0; podnet runs the one watcher and the rule of strangers (to podcrates) | closed (podcrates recorded the take) | none |
| #588 | [gh action] requested: ci and deny on the take of v0.53.0 | closed: ci 43760786274 and deny 43760789619 passed on 34fe7e1 | none |
| #589 | COORD-094: podnet took v0.54.0 and moved its help and manual onto pod-manual; four gaps of the crate; a text fix of frozen src/warp/ (to podcrates) | closed (podcrates recorded the take; T-359 done) | T-359 |
| #590 | [gh action] requested: ci, deny and secrets on the move to pod-manual (T-359) and the documented commands (T-358, T-363) | closed: deny 43771977628 and secrets 43771980776 passed on aa8bae2; ci 43771973878 failed at the man page (groff -ww: cannot adjust line), fixed in 3aa1ee7 | none |
| #591 | [gh action] requested: ci, deny, secrets and the dry run of issue-sync on the takes of v0.56.0 and the fix of the man page | closed: ci 43774698575, deny 43774702318, secrets 43774705423 and the dry run of issue-sync 43774709310 passed on f2d96fc (the dry run: 196 status blocks, one close of #436, 76 notes of authors outside the team) | none |
| #592 | COORD-095: podnet's lane in the labs of pod-lab v0.56.0; two defects of podcrates crates (pod-link under Landlock, pod-hold under udp-denied); the six network profiles (to podcrates) | closed (2026-11-23: each ask to podcrates is done: the fixes of pod-link and pod-hold in podcrates v0.59.0, taken in T-386; T-380 and T-381 done; the release gate stays in T-358) | none |
| #593 | [gh action] requested: ci and deny on the lane of podnet and the fix of unix listeners under Landlock (T-358, T-379) | closed: deny 43779619505 passed on 31f357a; ci 43779615497 failed at shellcheck (SC2046 in scripts/lab-lane.sh), fixed in b2faf08 | none |
| #594 | [gh action] requested: ci and deny on the fix of shellcheck and the reason of pod doctor (T-358, T-381) | closed: ci 43782124302 and deny 43782129951 passed on 077a9d1 | none |
| #595 | [gh action] requested: ci on the consumer .NET HttpClient under the shim (T-358 step 3) | closed: ci 43784289289 passed on aa8ee52 | none |
| #596 | [gh action] requested: ci and deny on the consumer Python, the fix T-382 and the take of v0.58.0 (T-358, T-382, T-385) | closed: ci 43788961230 and deny 43788964639 passed on 2047070 | none |
| #597 | COORD-096: podnet took v0.58.0 to v0.68.0 (to podcrates) | closed (PodUtils/podcrates#200 to #219 are closed) | none |
| #598 | [gh action] requested: ci, deny and a dry run of release on the installer core, the row proxy-only and the take of v0.59.0 (T-360, T-358, T-386) | closed: deny 43796829153 and release 43796832964 passed on 1852fcc; ci 43796824990 failed there on two install-test cases (T-360, fixed in c24e0cf); ci 43820922609, deny 43820922608, secrets 43820922620 and live 43820922672 passed on 01487d8 | none |
| #599 | vnet: let a stock client bind a wildcard, lock a file and enumerate interfaces | closed (outside the team: verified, labelled `confirmed`; T-390 and T-391 in c3616be, T-392 in the commit of its entry) | T-390, T-391, T-392 |
| #600 | link: an exec service never tells the peer how the program ended | closed (outside the team: verified, labelled `confirmed`; T-403 in the commit of its entry) | T-403 |
| #601 | forward has no name table for a client that cannot edit its hosts file | closed (outside the team: verified, labelled `confirmed`; T-404 in the commit of its entry) | T-404 |
| #602 | router doors count bytes but cannot cap them | closed (outside the team: verified, labelled `confirmed`; T-405 in the commit of its entry) | T-405 |
| #603 | COORD-097: a fix in the frozen src/tunnel.rs: the quic+warp leg reconnects when its WARP session closes (T-402) (to podcrates) | closed (podcrates answered in PodUtils/podcrates#214: the import of pod-cloudflare reads the files at eb23c379 or later) | none |
| #604 | COORD-098: the relay keeps a lost node socket for minutes after a reset (409); an ask for pod-relay and the relay server (to podcrates) | open (a message to podcrates; T-406 waits for the answer) | none |
| #605 | COORD-099: the answer to PodUtils/podrunner#201: --loopback-rules holds each loopback port but the broker (since v0.4.0-beta.10) (to podrunner) | closed (podrunner closed PodUtils/podrunner#201 on 2026-11-16) | none |
| #606 | live: two runs in a row failed | closed (2026-11-23, the coordinator: T-407, T-408 and T-411 are done; T-406 stays with GitHub #604; live passed in 44713609259, 44900660966, 45019925467 and 45146226467) | T-407, T-408, T-411 |
| #607 | COORD-100: pod-services: localhost-run needs the fact remote-port = 80 (to podcrates) | closed (podcrates v0.67.0 has the fact, taken in 71924ec (T-416); podnet asks for the bind address localhost in 3d85179 (T-418); live 44031540675: localhost-run served the origin at the first try) | T-418 |
| #608 | COORD-101: pod-tls: Trust::Default does not read the CA file of the Context; a fix in the frozen src/ws.rs (to podcrates) | closed (2026-11-23: podcrates v0.70.0 reads the CA file in Trust::Default (PodUtils/podcrates@T-858); podnet took it in T-480 and T-506; T-421 done) | T-421 |
| #609 | podnet net proxy: a real target gets SOCKS5 success, then EOF | closed (outside the team: verified, not a defect: the reply is 0x01, general failure; T-433) | T-433 |
| #610 | podnet pod: a dead or refusing upstream proxy hangs the program instead of failing the connect | closed (outside the team: verified, labelled `confirmed`; T-430) | T-430 |
| #611 | podnet pod: a reject rule returns a successful connect() and then hangs | closed (outside the team: verified, labelled `confirmed`; T-431, T-430) | T-431 |
| #612 | podnet expose: --detach after -- is ignored and the command runs in the foreground | closed (outside the team: verified, labelled `confirmed`; T-435) | T-435 |
| #613 | podnet link dial: stdin EOF closes the whole stream instead of half-closing | closed (outside the team: verified, labelled `confirmed`; the fault was in serve, T-434) | T-434 |
| #614 | podnet git init: an existing bare repository is reported as "not a repository" | closed (outside the team: verified, labelled `confirmed`; T-424) | T-424 |
| #615 | podnet warp ssh-config: the printed GIT_SSH_COMMAND is over-escaped and not copy-pasteable | closed (outside the team: verified, labelled `confirmed`; T-425) | T-425 |
| #616 | CLI usage errors suggest "podnet update --check" instead of naming the syntax error | closed (outside the team: verified, labelled `confirmed`; T-436) | T-436 |
| #617 | One-shot commands print the "runs in the foreground" warning | closed (outside the team: verified, labelled `confirmed`; T-437) | T-437 |
| #618 | podnet serve --pin: the client unlock flow is not documented | closed (outside the team: verified, labelled `confirmed`; T-426) | T-426 |
| #619 | podnet route: clarify which state directory holds router-usage.json | closed (outside the team: verified, labelled `confirmed`; T-427) | T-427 |
| #620 | Release binary: all network link carriers are off (only unix and memory) | open (outside the team: verified, labelled `confirmed`; T-428 partial) | T-428 |
| #621 | podnet pod -x direct: a false "upstream :0" warning, and -x does not accept direct | closed (outside the team: verified, labelled `confirmed`; T-432) | T-432 |
| #622 | Minor output nits: duplicate headers, double spaces, missing schema, duplicated error | closed (outside the team: verified, labelled `confirmed`; T-429) | T-429 |
| #623 | podnet exit: --stdio is the only mode, so make it the default | closed (outside the team: verified, labelled `confirmed`; T-438) | T-438 |
| #624 | podnet autostart run: --dir is required, but the default is already known | closed (outside the team: verified, labelled `confirmed`; T-439) | T-439 |
| #625 | podnet forward --name: --alias-port is required, but the target port is already in the name | closed (outside the team: verified, labelled `confirmed`; T-440) | T-440 |
| #626 | podnet warp env: print a unix-socket proxy when the TCP listener is absent | closed (outside the team: verified, labelled `confirmed`; T-441) | T-441 |
| #627 | podnet warp scan: the hint names `podnet warp --masque-endpoint`, which is not a command | closed (outside the team: verified, labelled `confirmed`; T-442) | T-442 |
| #628 | podnet serve --pin: the ready document and the text output do not say how to unlock | closed (outside the team: verified, labelled `confirmed`; T-443) | T-443 |
| #629 | podnet serve --list --tar: the listing has no tar link | closed (outside the team: verified, labelled `confirmed`; T-444) | T-444 |
| #630 | podnet autostart: add --json to list and --manual to add on a host with no manager | closed (outside the team: verified, labelled `confirmed`; T-445) | T-445 |
| #631 | podnet relay node: add --json, or say that --detach prints the document | closed (outside the team: verified, labelled `confirmed`; T-446) | T-446 |
| #632 | podnet ssh-relay: add --run-for, and make -q quiet | closed (outside the team: verified, labelled `confirmed`; T-447) | T-447 |
| #633 | podnet serve: add a way to append a user to the users file | closed (outside the team: verified, labelled `confirmed`; T-448) | T-448 |
| #634 | podnet link: the address scheme does not select the carrier, and a unix: prefix is taken as a filename | closed (outside the team: verified, labelled `confirmed`; T-458) | T-458 |
| #635 | Error messages repeat the cause twice | closed (outside the team: verified, labelled `confirmed`; T-459) | T-459 |
| #636 | podnet serve --local --pin: a plain-HTTP listener sets a Secure cookie, so browser PIN unlock cannot persist | closed (outside the team: verified, labelled `confirmed`; T-460) | T-460 |
| #637 | podnet route: a door quota can be overshot by a full 64 KiB buffer per flow | closed (outside the team: verified, labelled `confirmed`; T-461) | T-461 |
| #638 | podnet forward --dns: the DNS door is printed as up before it binds, and a bind failure exits 1 after the success line | closed (outside the team: verified, labelled `confirmed`; T-462) | T-462 |
| #639 | podnet bench exits 0 when every request fails | closed (outside the team: verified, labelled `confirmed`; T-463) | T-463 |
| #640 | podnet autostart add exits 69 but still creates the entry | closed (outside the team: verified, labelled `confirmed`; T-464) | T-464 |
| #641 | Detached --json documents do not share a schema (url vs http / doors / forward) | closed (outside the team: verified, labelled `confirmed`; T-465) | T-465 |
| #642 | podnet --json is accepted by some commands and rejected by others | closed (outside the team: verified, labelled `confirmed`; T-466) | T-466 |
| #643 | podnet serve --check refuses --local, so a local-only file server plan cannot be validated | closed (outside the team: verified, labelled `confirmed`; T-467) | T-467 |
| #644 | Default log level prints an INFO line on stderr for status, which is documented to open no connection | closed (outside the team: verified, labelled `confirmed`; T-468) | T-468 |
| #645 | podnet-install.sh --dry-run prints the proxy line twice, and the default install directory can be noexec | closed (outside the team: verified, labelled `confirmed`; T-469, the core of podcrates `v0.70.0`, T-480) | T-469 |
| #646 | podnet expose --via switch rejects the http:// target syntax that expose itself documents | closed (outside the team: verified, labelled `confirmed`; T-470) | T-470 |
| #647 | podnet ssh-relay --no-restart help names --stop-after, --idle-secs and --run-for, which the command rejects | closed (outside the team: verified, labelled `confirmed`; T-471) | T-471 |
| #648 | podnet serve --list: the HTML listing omits the modified time and the upload limit | closed (outside the team: verified, labelled `confirmed`; T-472) | T-472 |
| #649 | COORD-102: the QA round of podnet: 40 issues in one hour; a gate that measured reach, not correctness; asks for podcrates and the siblings (to all) | closed (its subject is done: the gate of the QA round, T-450 to T-455, done on 2026-11-20; `docs/decisions.md`, The end of the release freeze) | T-450, T-451, T-452, T-453, T-454, T-455, T-456, T-457 |
| #650 | COORD-103: the installer prints the proxy line twice; the default directory can be noexec (PodUtils/podnet#645) (to podcrates) | closed (answered: podcrates PodUtils/podcrates@T-883 in `v0.70.0`; taken in T-480) | T-469 |
| #651 | COORD-104: the answer to PodUtils/podcrates#221 (COORD-131): podnet's copies of shared code, and the text of the rulings (to podcrates) | closed (2026-11-23: T-473 and T-494 are done; the take is written in #686, COORD-114) | T-473 |
| #652 | COORD-105: podnet's copies that need a change of podcrates (ask 3 of PodUtils/podcrates#221) (to podcrates) | closed (answered: podcrates' entries 896 to 901) | T-473 |
| #653 | COORD-106: pod-testkit: a refused address that no other test can take (to podcrates) | closed (answered: `pod_testkit::refused` in podcrates `v0.71.0`; taken in T-483, T-479) | T-479 |
| #654 | COORD-107: podnet takes the rulings of COORD-134 (podrunner first, Coordinator, schema ids) and waits for the asks of podrunner (to podcrates) | open (a COORD issue between sessions) | T-137 |
| #655 | COORD-108: podnet v0.4.0-beta.17 has the fixes of PodUtils/podrunner#213 and #142; please pin it and run #142 again (to podrunner) | closed (answered: PodUtils/podrunner#309, COORD-099: `user-gate.yml` 44393864747 passes `hold:podvm` on each lane with beta.17) | T-481 |
| #656 | COORD-109: podnet on ghcr.io: the form (2) since issue 160, an attestation and the digests from beta.18, and the digests of beta.17 now (to podcrates) | open (a COORD issue between sessions) | T-487 |
| #657 | COORD-110: pod-todo: an opt-in check that a done defect names its test (to podcrates) | closed (answered: podcrates `v0.79.0`, PodUtils/podcrates@T-979; taken in T-501, T-450) | T-450 |
| #658 | COORD-111: pod-cloudflare, pod-warp and pod-ws: an error line names one cause two or three times (to podcrates) | closed (answered: podcrates `v0.79.0`, PodUtils/podcrates@T-985, PodUtils/podcrates@T-986 and PodUtils/podcrates@T-987; taken in T-501, T-497) | T-497 |
| #659 | [gh action] requested: ci on the error rows of `pod` (T-496), `src/egress.rs` (T-498) and the test of T-433 (T-450) | closed: ci 44664720872 failed on 0dc34d0 in `restricted-host` and `no-loopback` (the new test `tests/pod_error_lines.rs`; fixed in 8f3a20c, T-496); the other 10 jobs passed | none |
| #660 | [gh action] requested: ci on the fix of the test of T-496 and the lane case `errors` (T-452) | closed: ci 44667637115 passed on 826475c (13 of 13 jobs, also restricted-host and no-loopback) | none |
| #661 | [gh action] requested: ci on T-499 (the resolver of the netns backend at port 53 of its loopback) | closed: ci 44671079409 passed on 1413960 | none |
| #662 | [gh action] requested: ci on the lane row `testers` (T-453) and the prompt of the QA pass (T-454) | closed: ci 44695405116 passed on a520510 | none |
| #663 | [gh action] requested: ci on the measure of the promises (T-455) and the flags of `pod` in the manual (T-500) | closed: ci 44701681199 failed on 1c7df7d (`test`, `features`, `restricted-host`, `no-loopback`: two manual tests; fixed in the next commit, T-500) | none |
| #664 | [gh action] requested: ci on the take of podcrates `v0.79.0` (T-501) and the fix of the manual tests (T-500) | closed: ci 44707736066 passed on 4559204 | none |
| #665 | [gh action] requested: live on the take of podcrates `v0.79.0` (T-501) | closed: live 44712072334 failed on ab0612d: HTTP 530 of a quick tunnel in the step "Throughput of the HTTP path" (T-488) | T-488 |
| #666 | [gh action] requested: live again on the take of podcrates `v0.79.0` (T-488: a flake or a fault of the take) | closed: live 44713609259 passed on 3f14a38 (the 530 of #665 was the flake of T-488, not the take) | T-488 |
| #667 | [gh action] requested: ci on the error rows of the lane (T-452) and a refused getsockname (T-502) | closed: ci 44737373446 passed on 79b8331 | none |
| #668 | [gh action] requested: gate (the full lane with the case `errors` and the row `testers`) before the next beta | closed: gate 44742237281 failed on c9e394a: the case `errors` in 7 labs (T-503: the loopback nameserver of the runner; the words of Rust for an unreachable network; a client limit of 10 s on an IPv6-only host); fixed in the next commit | T-503 |
| #669 | [gh action] requested: ci on the resolver at each loopback nameserver (T-503) and the error rows | closed: ci 44782773504 passed on 3836971 | T-503 |
| #670 | [gh action] requested: gate again on the resolver at each loopback nameserver (T-503) and the error rows | closed: gate 44782773447 passed on 3836971 (the full lane on the GitHub runner, with the case `errors` and the row `testers`) | T-503 |
| #671 | [gh action] requested: issue-sync (not a dry run) after `record-issues` 44752520313 found 4 differences | closed: issue-sync 44782981366 passed on 3836971 (GitHub agrees with the record) | none |
| #672 | [gh action] requested: issue-sync (not a dry run) after the record closed #649, #670 and #671 | closed: issue-sync 44787120939 passed on ab66048 | none |
| #673 | [gh action] requested: ci on one line for a refused namespace (T-482) | closed: ci 44799344725 passed on ac67710 (13 of 13 jobs) | T-482 |
| #674 | [gh action] requested: ci on the take of podcrates `v0.80.0` (T-504) | closed: ci 44803822897 passed on 058059f | T-504 |
| #675 | [gh action] requested: ci on the date code of `pod_core::clock` (T-473) | closed: ci 44846557642 passed on 0948be4 | T-473 |
| #676 | [gh action] requested: ci on the retry waits of `pod_core::backoff` (T-473) | closed: ci 44850063694 passed on 00ceb2f | T-473 |
| #677 | COORD-112: pod-core: one atomic replace of a file at a path (a temporary file, fsync, rename) (to podcrates) | closed (answered: podcrates `v0.81.0`, PodUtils/podcrates@T-1066; taken in T-505) | T-473 |
| #678 | [gh action] requested: ci on the take of podcrates `v0.81.0` and the replace of `pod_core::files` (T-505, T-473) | closed: ci 44871490037 passed on 9a9a0a9 | T-505 |
| #679 | [gh action] requested: issue-sync (not a dry run) on `1a4781eb` for #672 and #677 | closed: superseded by the `v0.82.0` consolidation push | none |
| #685 | COORD-113: pod-cloudflare: a refusal with retry: yes needs a time budget, and keeps its class through "no edge leg worked" (to podcrates) | closed (answered: podcrates `v0.82.1`, PodUtils/podcrates#257; taken in T-539, the typed refusal is the first path, T-513 the second) | none |
| #686 | COORD-114: podnet took pod-release (T-494) and the last shared-code copies of COORD-104 (T-473) (to podcrates) | closed (read: PodUtils/podcrates@T-966 and PodUtils/podcrates@T-1118; the index of a release is T-540) | none |
| #687 | [qa 0.4.0-beta.18] send: the timeout line gives the wrong time ("within its 1 minutes" for --timeout 5) | closed: T-520, T-531 and T-532 are done; T-531 in the commit of the entry, 2026-11-23 (tests/config_input.rs; CI after the push) | T-520, T-531, T-532 |
| #688 | live: two runs in a row failed | open (live 45186065944: HTTP 530 of a quick tunnel, T-488; live 45215638823: WARP's door and the relay, T-406; live 45517951377: WARP's door in "warp scan", T-406; live 45518282050: resets of `warp proxy` with no session, T-546) | T-406, T-488, T-546 |
| #689 | [qa 0.4.0-beta.18] connect: a failed listener bind exits 1; net proxy and forward exit 69 | closed: T-515 done in the commit of the entry, 2026-11-23 (tests/listener_bind.rs; CI after the push) | T-515 |
| #690 | [qa 0.4.0-beta.18] pod: with the seccomp backend, a missing program path exits 127 with no line | closed: T-516 done (T-542, the take of podcrates v0.83.0, ends T-516 and T-522; the test of each entry is in its Done section) | T-516 |
| #691 | [qa 0.4.0-beta.18] pod: SIGTERM, SIGINT or SIGHUP to podnet ends podnet and leaves the program running | closed: T-517 done in the commit of the entry, 2026-11-23 (tests/pod_signals.rs; CI after the push) | T-517 |
| #692 | [qa 0.4.0-beta.18] net doctor: no-loopback host reports "not needed"; a TCP loopback listener starts and cannot be reached | closed: T-524 is done (black-box QA pass of 0.4.0-beta.18, 2026-11-23); `250110a` | T-524 |
| #693 | [qa 0.4.0-beta.18] warp status --json: the document has no `schema` and no `podnet` key | closed: T-519 done (`2427cb7`; ci of the push of `2427cb7`) | T-519 |
| #694 | [qa 0.4.0-beta.18] pod doctor --json: the document has no `podnet` key | closed: T-519 done (`2427cb7`; ci of the push of `2427cb7`) | T-519 |
| #695 | [qa 0.4.0-beta.18] tunnel: a non-JSON answer of the quick-tunnel service gives a hint for another step (edge register) | closed: T-521 done (the commit of T-521 and T-522, "error lines name each cause once"; tests/error_words.rs); the crate parts are COORD asks (to podcrates) | T-521 |
| #696 | [qa 0.4.0-beta.18] tunnel --check, serve --check: the edge row prints one DNS error six times, as Rust debug text | closed: T-522 done (T-542, the take of podcrates v0.83.0, ends T-516 and T-522; the test of each entry is in its Done section) | T-522 |
| #697 | [qa 0.4.0-beta.18] connect: a dead proxy gives "the remote does not answer", repeats the cause three times, no next step | closed: T-521, T-522 done (T-542, the take of podcrates v0.83.0, ends T-516 and T-522; the test of each entry is in its Done section) | T-521, T-522 |
| #698 | [qa 0.4.0-beta.18] bench: a name that does not resolve gives one very long line and no next step | closed: T-521, T-522, T-530 done (T-542, the take of podcrates v0.83.0, ends T-516 and T-522; the test of each entry is in its Done section) | T-521, T-522, T-530 |
| #699 | [qa 0.4.0-beta.18] net shim: the documented `--out /tmp/podnet-vnet` example prints a working LD_PRELOAD line on a noexec /tmp | closed: T-525 is done (black-box QA pass of 0.4.0-beta.18, 2026-11-23); `250110a` | T-525 |
| #700 | [qa 0.4.0-beta.18] rules check: an unreadable or unparseable policy file exits 1; PODNET_POLICY gives 78 | closed: T-518 done in the commit of the entry, 2026-11-23 (tests/config_input.rs; CI after the push) | T-518 |
| #701 | [qa 0.4.0-beta.18] docs/host-runtime.md: the examples `podnet fetch` and `podnet init generate --node` do not run | closed: T-526 done (`2427cb7`; ci of the push of `2427cb7`) | T-526 |
| #702 | [qa 0.4.0-beta.18] pod --serve: a refused target gives SOCKS5 reply 0x01; docs/proxies.md says 0x04 for "refused" | closed: T-527 is done (black-box QA pass of 0.4.0-beta.18, 2026-11-23); `b2d47f3` | T-527 |
| #703 | [qa 0.4.0-beta.18] doctor: exit 1 when no leg works; `podnet man exit-status` says 69 for "no edge leg worked" | closed: T-528 done (`2427cb7`; ci of the push of `2427cb7`) | T-528 |
| #704 | [qa 0.4.0-beta.18] state directory: the warning names a path that does not exist, and `warp register` prints it three times | closed: T-529 is done (black-box QA pass of 0.4.0-beta.18, 2026-11-23); `250110a` | T-529 |
| #705 | [qa 0.4.0-beta.18] warp proxy: a listener that cannot bind prints the same cause on two error lines | closed: T-523 done in the commit of the entry, 2026-11-23 (tests/listener_bind.rs; CI after the push) | T-523 |
| #706 | [qa 0.4.0-beta.18] forward, net proxy, serve --local: a TCP bind denied by the host gives no hint to use a unix socket | closed: T-523 done in the commit of the entry, 2026-11-23 (tests/listener_bind.rs; CI after the push) | T-523 |
| #707 | COORD-115: the QA pass of podnet 0.4.0-beta.18: asks for pod-hold, pod-wormhole, pod-ws, pod-proxy, pod-dns, pod-tls, pod-warp and pod-cloudflare (to podcrates) | closed (answered: podcrates v0.83.0, PodUtils/podcrates#259, PodUtils/podcrates@T-1125 to PodUtils/podcrates@T-1134; taken in T-542) | none |
| #708 | COORD-116: podnet took podcrates v0.82.1; COORD-113 and COORD-114 closed; the release v0.4.0-beta.18 (to podcrates) | closed (read: PodUtils/podcrates#257 names the take, a0b727c; PodUtils/podcrates@T-1117 is the line of docs/tunnels.md, 4a4361a) | none |
| #709 | COORD-117: pod-core: one probe of a unix socket that a forked child can hold (socket_answers) for pod-control, pod-link and pod-warp (to podcrates) | closed (answered: podcrates v0.83.0, PodUtils/podcrates@T-1209; taken in T-542, podnet re-exports `pod_core::files::socket_answers`) | none |
| #710 | COORD-118: podnet took podcrates v0.83.0; COORD-115 and COORD-117 closed; runs.py and the watcher of v0.83.0 (to podcrates) | open (a COORD issue between sessions) | none |
| #711 | COORD-119: pod-core: one temp directory and one temp file with a random name (lesson 5 of PodUtils/podvm#567) (to podcrates) | open (a COORD issue between sessions; the ask of T-545) | none |
| #712 | COORD-120: pod-warp proxy: a client with no WARP session gets a reset, not a SOCKS5 or HTTP failure reply (to podcrates) | open (a COORD issue between sessions; the ask of T-546) | none |
| #713 | [qa 0.4.0-beta.19] pod doctor: says that the netns backend is ok where the netns backend cannot run (bind-denied) | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-547 |
| #714 | [qa 0.4.0-beta.19] logs: the log of a detached instance does not hold the lines before the ready | closed: T-548 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-548 |
| #715 | [qa 0.4.0-beta.19] warp proxy: a MASQUE door behind a TLS-inspecting middlebox gives a wrong next step (--ca-file) and blames WARP | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-549, T-550 |
| #716 | [qa 0.4.0-beta.19] tunnel --json: the "verified" event has no "schema" and no "podnet" key | closed: T-551 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-551 |
| #717 | [qa 0.4.0-beta.19] pod --serve: a listener that cannot bind gives a bare OS error and exit 3, not one line with the address and exit 69 | closed: T-552 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-552 |
| #718 | [qa 0.4.0-beta.19] doctor: the leg quic+warp is "ok" where a TLS middlebox answers at the MASQUE door, and warp then fails | closed: T-553 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-553 |
| #719 | [qa 0.4.0-beta.19] exit: a name with no address and a dial time limit get SOCKS5 0x01, not 0x04 | closed: T-554 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-554 |
| #720 | [qa 0.4.0-beta.19] send, receive --json: the events have no "schema" and no "podnet" key | closed: T-551 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-551 |
| #721 | [qa 0.4.0-beta.19] net proxy: a closed virtual port gets SOCKS5 0x01, not the documented code | closed: T-555 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-555 |
| #722 | [qa 0.4.0-beta.19] init enroll: a failed dial of the relay drops the cause and gives no next step | closed: T-556 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-556 |
| #723 | [qa 0.4.0-beta.19] init enroll: a relay URL that is not a relay prints the raw chunked HTML body in the error | closed: T-557 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-557 |
| #724 | [qa 0.4.0-beta.19] host probe --json: "schema" is a string and the "podnet" key is missing | closed: T-558 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-558 |
| #726 | COORD-121: pod-warp: a pin mismatch names the cause by the issuer of the door certificate, and says each cause once (to podcrates) | open (a COORD issue between sessions; the ask of T-550) | none |
| #729 | [qa 0.4.0-beta.19] stat --json: "schema" is a string and the "podnet" key is missing | closed: T-558 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test of each entry is in its Done section) | T-558 |
| #733 | [qa 0.4.0-beta.19] relay pair --json: the document has no "schema" and no "podnet" key | closed: T-551 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; `relay pair --json` is one of the six sites of T-551) | T-551 |
| #728 | [qa 0.4.0-beta.19] rules check: the documents do not give the fields of a policy rule | closed: T-559 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-559 |
| #730 | [qa 0.4.0-beta.19] ci check: a declaration with "schema": 1 is refused; the help and the documents do not give the form | closed: T-560 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-560 |
| #732 | [qa 0.4.0-beta.19] man --json SECTION: the section names "variables", "exits" and "table" of the document are refused | closed: T-561 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-561 |
| #734 | [qa 0.4.0-beta.19] autostart add: refuses ~/.local/bin/podnet as "a temporary directory" and tells the user to copy it to ~/.local/bin | closed: T-562 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-562 |
| #735 | [qa 0.4.0-beta.19] offline bundle build: the example of docs/offline.md (--file ./site:/site) does not run | closed: T-563 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-563 |
| #736 | [qa 0.4.0-beta.19] serve --local: on a host with no loopback address the listener is "ready" and --check passes, but no client can connect | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-564 |
| #742 | [qa 0.4.0-beta.19] net proxy: docs/virtual-network.md says that curl does not accept a unix-socket SOCKS proxy, but it does | closed: T-565 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-565 |
| #743 | [qa 0.4.0-beta.19] --log-format json: the final ERROR object has no "timestamp", the other objects have one | closed: T-566 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-566 |
| #747 | [qa 0.4.0-beta.19] route --config: a missing or bad file exits 64; tunnel --config and "man exit-status" give 78 for the same fault | closed: T-567 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-567 |
| #748 | [qa 0.4.0-beta.19] wg serve: on a host that refuses UDP, the error tells the user to give a port above 1023 or a unix socket | closed: T-568 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-568 |
| #750 | [qa 0.4.0-beta.19] serve --local, net proxy --listen: where getsockname is refused, the error is a bare "Not supported (os error 95)" and exit 1 | closed: T-569 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-569 |
| #725 | [qa 0.4.0-beta.19] pod: PODNET_POLICY does not refuse a destination (the program reaches a refused host) | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-572 |
| #727 | [qa 0.4.0-beta.19] bench, connect: a refusal of PODNET_POLICY gives the next step of a connect failure | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-573 |
| #731 | [qa 0.4.0-beta.19] warp status, rotate, delete, license: a missing identity file exits 78, not 66 (no-input) | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-574 |
| #737 | [qa 0.4.0-beta.19] bench, connect, tunnel, relay connect, forward: the line of a name that does not resolve is cut with "..." in the middle of a cause | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-575 |
| #738 | [qa 0.4.0-beta.19] warp up: the failure prints two lines of about 1500 characters each that repeat each cause several times | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-576 |
| #739 | [qa 0.4.0-beta.19] pod --report: "backend": "netns-kernel" is not a value of pod-report.schema.json | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-577 |
| #740 | [qa 0.4.0-beta.19] pod --backend netns (userns-off): the start failure gives no next step, and the report has no true reason | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-578 |
| #741 | [qa 0.4.0-beta.19] tunnel: behind a proxy, the start waits about 35 s in DNS over HTTPS with no line at the default level and no cause at -v | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-579 |
| #744 | [qa 0.4.0-beta.19] --proxy http+exec: a command that does not exist floods stderr with one sh line for each try and ends with a wrong cause | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-580 |
| #749 | [qa 0.4.0-beta.19] pod doctor: says that seccomp (and netns) are ok in no-proc and proc-masked, where the pod cannot start | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-581 |
| #751 | [qa 0.4.0-beta.19] update --check: a proxy that does not answer costs 60 s and the error gives no next step | open (black-box QA pass of 0.4.0-beta.19, 2026-11-25) | T-582 |
| #745 | [qa 0.4.0-beta.19] --egress, forward --network: an unknown spec is read as a proxy host:port, and the error does not give the known specs | closed: T-570 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-570 |
| #746 | [qa 0.4.0-beta.19] forward: a UDP forward and the DNS door over "direct" carry no datagram ("direct carries no UDP") | closed: T-571 done (black-box QA pass of 0.4.0-beta.19, 2026-11-25; the test is in its Done section) | T-571 |

#312 to #315, #318, #381 to #383, #415, #556, #557 and #680 to #684 are closed; they are pull requests.
