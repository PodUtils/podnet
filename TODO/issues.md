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
| #161 | Publish the library and the CLI on crates.io (deferred) | open (deferred: publish nothing now) | T-005 |
| #222 | crates.io: the workspace version is a prerelease, and the beta requirement cannot match a stable pod-netns | open (deferred: the first stable release) | T-006 |
| #224 | crates.io: the crate name cfrs is registered to another project | closed (the owner chose and reserved `podnet`) | T-007 |
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
| #425 | COORD-016: T-229 measured, T-230 has the hold object, T-231 item 1 done (to podrunner) | open (a COORD issue between sessions) | none |
| #426 | COORD-017: podnet took v0.17.0 and pod_hold::connect and pod_hold::packet | closed (podcrates recorded it: taken e4d4049) | none |
| #427 | COORD-018: the cut of pod-netstack (to podcrates) | closed (podnet took pod-netstack at d23ce4e) | none |
| #428 | pod --serve door: early reply and pipelined SOCKS5 upstream handshake (donor review) | closed | T-237 |
| #429 | listen::bind_private: the socket has the umask mode between bind and chmod (donor review) | closed (fixed in 3c6173f) | T-238 |
| #430 | install.sh and update: tag check, strict SHA256SUMS, install tests that can fail (donor review) | closed | T-239 |
| #431 | warp tun --default-route: resolver backup on disk, restore after a kill, route table test (donor review) | closed | T-240 |
| #432 | warp: registration under a lock, save before the enrolment step (donor review) | closed | T-241 |
| #433 | rules: a shadowed rule warns, a shadowed reject fails (donor review) | open | T-242 |
| #434 | net tailscale: --tailnet guard and the TS_FORCE_NOISE_443 hint (donor review) | closed | T-243 |
| #435 | tunnel --inspect: capture, ring, redaction, replay (donor review) | open (T-244 is blocked on pod-cloudflare) | T-244 |
| #436 | podnet receive: containment of the received entries (donor review) | open (a correction of T-105) | T-105 |
| #437 | COORD-019: proposals for podcrates from the donor review (to podcrates) | open (a COORD issue between sessions) | none |
| #438 | COORD-020: proposals for podssh from the donor review (to podssh) | closed (podssh recorded them in its record, PodUtils/podssh#188) | none |
| #439 | COORD-021: proposals for podrunner from the donor review, part 1 of 2 (to podrunner) | open (a COORD issue between sessions) | none |
| #440 | COORD-022: proposals for podrunner from the donor review, part 2 of 2 (to podrunner) | open (a COORD issue between sessions) | none |
| #441 | COORD-023: proposals for the stub repositories from the donor review, part 1 of 3 (to all) | closed (a COORD issue between sessions; no entry, as CLAUDE.md says) | none |
| #442 | COORD-024: proposals for the stub repositories from the donor review, part 2 of 3 (to all) | closed (a COORD issue between sessions; no entry, as CLAUDE.md says) | none |
| #443 | COORD-025: proposals for the stub repositories from the donor review, part 3 of 3 (to all) | closed (a COORD issue between sessions; no entry, as CLAUDE.md says) | none |
| #444 | COORD-026: pod-broker bind_unix has the umask window of bind_private (to podcrates) | closed (a COORD issue between sessions) | none |
| #445 | COORD-027: podnet took pod-netstack of v0.20.0 (to podcrates) | closed (podcrates recorded it: taken d23ce4e) | none |
| #446 | COORD-028: podnet changed three frozen files: register.rs, route.rs (fixes) and netns/tun.rs (the veth wire) (to podcrates) | closed (podcrates answered in its #68: no port; the wire stays in veth.rs, podcrates T-201) | none |
| #447 | COORD-029: the answer to podcrates #67 (pod-wg): wg.rs is held to fixes; the split (to podcrates) | closed (podcrates wrote taken 089e1c9) | none |
| #448 | COORD-030: podnet took pod-services-watch and the crates of v0.21.0 (to podcrates) | open (a COORD issue between sessions) | none |
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
| #460 | COORD-041: podnet took the SSH relay facts (T-166); a comment fix in the frozen src/tunnel.rs; the pinggy URL rule (to podcrates) | open (a COORD issue between sessions) | none |
| #461 | cfb asks: tracker for the 6 asks of cfb owned by podnet (from podcrates#35) | open | T-216, T-217, T-218, T-219, T-220, T-221, T-260 |
| #462 | COORD-042: a change in the frozen src/doctor.rs for the lanes of A-C-10 (T-260) (to podcrates) | closed (a COORD issue between sessions) | none |
| #463 | COORD-043: a weekly watch of the relay contract in pod-relay (podnet T-134) (to podcrates) | closed (a COORD issue between sessions) | none |
| #464 | COORD-044: five more gaps of pod-netstack for the guest-wire router (podnet T-176, T-248) (to podcrates) | closed (a COORD issue between sessions) | none |
| #465 | COORD-045: podnet took v0.27.0 at 28c0e3d (to podcrates) | closed (a COORD issue between sessions) | none |
| #466 | link: peer identity, session, carrier registry, and the link command (E0) | closed | T-261, T-262, T-263, T-264, T-265, T-266, T-267 |
| #467 | link: stream carriers and availability (E1) | open | T-268, T-269, T-270, T-271, T-272, T-273, T-274, T-275 |
| #468 | link: datagram reliability, ARQ, and simplex carriers (E2) | open | T-276, T-277, T-278, T-279, T-280 |
| #469 | link: DNS stream, ICMP, WireGuard and NAT traversal carriers (E3) | open | T-281, T-282, T-283, T-284, T-334 |
| #470 | link: mesh announcements, path-vector routing, forwarding and ACLs (E4) | open | T-285, T-286, T-287, T-288, T-289, T-290, T-335 |
| #471 | link: services at a peer (SOCKS5, HTTP, raw, forward, Shadowsocks, Trojan, exec, serve-at) (E5) | open | T-266, T-291, T-292, T-293, T-294, T-295, T-296, T-297 |
| #472 | link: record layer (AEAD, compression, padding, replay and resume) (E6) | closed: T-298 to T-301 done; ci 43491819082 on d53df25 | T-298, T-299, T-300, T-301 |
| #473 | link: direction split and chain composition (E7) | open | T-302, T-303 |
| #474 | link: C ABI (E8) | closed (ci 43382314031 at 9583ea3) | T-304 |
| #475 | link: portable build subset (E9) | closed (not planned: the owner, 2026-11-12, Linux only) | T-305 |
| #476 | link: CLI query keys, control plane, aliases, dump, components and build features (E10) | closed (ci 43382314031 at 9583ea3) | T-306, T-307, T-308, T-309, T-310, T-311 |
| #477 | COORD-046: podnet took v0.28.0 at b08e06f (to podcrates) | closed (a COORD issue between sessions) | none |
| #478 | COORD-047: podnet took v0.29.0 and pod-hold syn-hold at 98d8a71 (to podcrates) | closed (a COORD issue between sessions) | none |
| #479 | COORD-048: asks for the link workstream of podnet (GitHub #466 to #476) (to podcrates) | closed (a COORD issue between sessions) | none |
| #480 | COORD-049: podnet starts the link workstream: pod-link with the unix and memory carriers first (to podcrates) | closed (a COORD issue between sessions) | none |
| #481 | COORD-050: podnet took v0.30.0 and pod-relay at 83e0c93 (to podcrates) | closed (a COORD issue between sessions) | none |
| #482 | COORD-051: two gaps of pod-relay after the take (pair renewal, pair on a second host) and the edge budget (to podcrates) | closed (a COORD issue between sessions) | none |
| #483 | live: two runs in a row failed | closed | T-315 |
| #484 | [DEFERRED] Carry a session in NTPv4 extension fields, SMTP and IMAP | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #485 | [DEFERRED] Carry a session through a TURN allocation, with RFC 6062 | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #486 | [DEFERRED] Close four QUIC gaps: MASQUE enrolment, HTTP/3 shape, version, connection ID | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #489 | [DEFERRED] Register third-party services as roads, with a watch, a date and a count | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #491 | [DEFERRED] Switch after a real handshake, with a cover script and multiplexed cover | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #493 | [DEFERRED] Shape the record layer: padding profiles, entropy, first flight, pacing | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #494 | [DEFERRED] Shape the application: deferred responses, a generator, carriers in the payload | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #495 | [DEFERRED] Add a packet sidecar: desync search, disagreement probe, fake TCP, bonding | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #496 | [DEFERRED] Keep the session end to end: opaque destination and keys from the ends | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #497 | [DEFERRED] Make the pool live, the budget per deployment, tokens expiring, hosts movable | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #498 | [DEFERRED] Resume a session across hours, and serve the full service surface at a peer | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #499 | [DEFERRED] Carry a session with no network: serial, HF, spool, discovery, pairing | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #500 | [DEFERRED] State the host runtime: no open port, classified egress, stated limits | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
| #501 | [DEFERRED] Build the verification harness: classifier, exit codes, conformance matrix | open (deferred by the owner, 2026-11-12: the entries wait for the owner) | none |
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
| #516 | live: two runs in a row failed | closed (quick-tunnel edge flakes; the rerun 43333498755 passed) | none |
| #517 | COORD-060: the answer to the asks N-1 to N-5 of podswarm (PodUtils/podcrates#125) (to all) | open (a COORD issue between sessions) | none |
| #518 | COORD-061: port rules in the rules reader of pod-warp: port:N=WAY and tcp:HOST:PORT=WAY (podnet T-328; ask N-2 of podswarm) (to podcrates) | closed (T-328 done, f5bab50) | none |
| #519 | COORD-062: podnet took v0.35.0 at 5562f21 (to podcrates) | closed (a COORD issue between sessions) | none |
| #520 | COORD-063: podnet took v0.36.0 at 775486f (to podcrates) | closed (a COORD issue between sessions) | none |
| #521 | COORD-064: what podnet waits for to close its issues: the datagram carriers of pod-link, pod-e2e, pod-wormhole, pod-warp's freeze and the others (to podcrates) | closed (answered by PodUtils/podcrates#133) | none |
| #522 | COORD-065: podnet took v0.37.0 and pod-warp at f6abdc3; src/warp/ holds fixes only (to podcrates) | open (a COORD issue between sessions) | none |
| #523 | live: two runs in a row failed | closed: live 43459893595 on bf01ac6 passed | T-332 |
| #524 | COORD-066: a fix in src/warp/quic.rs: WarpNet's datagrams bind [::], so QUIC to an IPv4 edge goes as IPv4-mapped and times out (to podcrates) | closed: live 43459893595 on bf01ac6 passed with the fix of pod-warp v0.39.0 | none |
| #525 | COORD-067: podnet took v0.38.0 (to podcrates) | closed (a COORD issue between sessions) | none |
| #526 | COORD-068: podnet took v0.39.0 at 306d80b; the one-family fix of WARP is dropped (to podcrates) | open (a COORD issue between sessions) | none |
| #527 | COORD-069: the answer to the question on KCP: the congestion window by default; the 20 % loss gate runs with nodelay (podnet T-276) (to podcrates) | closed: the kcp profile and the feature udp are in d474725; ci 43491819082 on d53df25 | none |
| #528 | [gh action] requested: ci, deny, secrets and live on b1e2b80 (Actions does not start on a push of the cloud session) | closed: ci 43445945197 and live 43445966561 failed (fixed in 2122bd4), deny and secrets passed | none |
| #529 | [gh action] requested: ci, deny, secrets and live on 7d73ec7 | closed: ci 43452891413 failed at restricted-host (fixed in the next commit), live 43452912977 failed at the file server (fixed in 098ff8d), deny and secrets passed | none |
| #530 | [gh action] requested: ci, deny, secrets and live on bf01ac6 | closed: each run passed (ci 43459872995, deny 43459879363, secrets 43459886257, live 43459893595) | none |
| #531 | [gh action] requested: ci, deny, secrets and live on 6f8e69e (T-012, T-095, T-087) | closed: each run passed (ci 43471936738, deny 43471944208, secrets 43471952954, live 43471960138) | none |
| #532 | COORD-070: podnet took v0.40.0 at 1c88531 (to podcrates) | open (a COORD issue between sessions) | none |
| #533 | COORD-071: two asks of the carriers of pod-link: a typed refusal of the key, and the TC rule of the dns carrier (to podcrates) | open (a COORD issue between sessions) | none |
| #534 | [gh action] requested: ci, deny, secrets and live on d53df25 | closed: ci 43491819082, deny 43491826156 and secrets 43491833085 passed; live 43491840609 failed at the new router step (fixed in c02da54) | none |
| #535 | COORD-072: the asks of the transfer of podnet over pod-wormhole: the derivation of a key, a stand-in mailbox, the count of words, the TCP transit relay (to podcrates) | open (a COORD issue between sessions) | none |
| #536 | [gh action] requested: ci, deny, secrets and live on 0dd7663 | closed: deny 43499858663 and secrets 43499865056 passed; ci 43499850487 failed (the transfer tests have no loopback guard) and live 43499872018 failed at the router step (w stayed up) | none |
| #537 | live: two runs in a row failed (the router step, runs 43491840609 and 43499872018) | closed: live 43521809728 on 3061c7c passed | T-101 |
| #538 | COORD-073: pod-hold: a seccomp rule for a raw ICMP socket (podnet T-334) (to podcrates) | open (a COORD issue between sessions) | none |
| #539 | [gh action] requested: ci, deny, secrets and live on c8a9510 | closed: ci 43506563209, deny 43506569632 and secrets 43506579272 passed; live 43506586024 failed at the router step (w stays up) | none |
| #540 | COORD-074: pod-link: dgram_tls does not call rekeyed, so a dns or icmp link stops at the wrap of the ARQ (podnet T-277) (to podcrates) | open (a COORD issue between sessions) | none |
| #541 | [gh action] requested: ci, deny, secrets and live on 54f0809 | closed: ci 43515459301, deny 43515464237 and secrets 43515470028 passed; live 43515475108 failed at the router step (the WARP session took the relay leg) | none |
| #542 | [gh action] requested: ci, deny, secrets and live on 3061c7c | closed: each run passed (ci 43521791090, deny 43521797247, secrets 43521803788, live 43521809728) | none |
| #543 | [gh action] requested: the release of v0.4.0-beta.10 at 43c53a6 | closed: stopped at ci 43535662399 (restricted-host, a race in a test); deny, secrets and live passed; nothing published | T-336 |
| #544 | [gh action] requested: the release of v0.4.0-beta.10 at 59cf78c | closed: ci, deny, secrets and live passed; the dry run 43541138625 failed on the 32-bit targets (nfnl.rs); nothing published | T-336 |
| #545 | [gh action] requested: the release of v0.4.0-beta.10 at e99eba6 | closed: each run passed; the release v0.4.0-beta.10 is out (release-proofs 43549615088) | T-336 |
| #546 | COORD-075: podnet took v0.41.0 at 095955c (to podcrates) | closed (podcrates closed PodUtils/podcrates#143) | none |
| #547 | COORD-076: pod-wg: a responder sans I/O, and the preshared key (podnet T-125, T-337) (to podcrates) | open (a COORD issue between sessions; answered by PodUtils/podcrates#144) | none |
| #548 | [gh action] requested: ci, deny, secrets and live on 3d54f76 (T-117) | closed: each run passed on 036d23c (ci 43551002812, deny 43551006710, secrets 43551010275, live 43551013239) | none |

#312 to #315, #318, #381 to #383 and #415 are closed; they are pull requests.
