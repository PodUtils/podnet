This file holds the entries of milestone M4: one interface for the networks and for the
services, a registry of the networks, and one form of a network spec on the command line.
`docs/research/podnet/architecture.md`, section 4.3, gives the types. Each step first wraps an
API that exists, with no change of behaviour. Then the commands use the interface. Each old flag
stays (the owner's ruling of 2026-11-05 on regressions, `docs/decisions.md`).

# T-055: The network types in `podnet-core`: a target, a dialer, datagrams, a network and its link

**Source:** `docs/research/podnet/architecture.md`, section 4.3;
`docs/research/podnet/capabilities.md`, sections 2.2 and 6 (enabler 4); GitHub #377.
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Five dial functions exist with no common interface: `egress::dial` (`src/egress.rs`), the WARP
dial of `src/warp/proxy.rs`, `Relay::connect` (`src/ws_relay.rs`), the tailnet dialer of
`crates/podnet-netns/src/tailscale.rs` and the stack of `crates/podnet-vnet/src/stack.rs`. Each front door and
each command calls one of them by name. A router, a group or a new overlay needs one interface.

## Premise

- `docs/research/podnet/architecture.md`, section 4.3, lists for each network what it gives:
  egress, ingress, an export, or a mix.
- `WarpUdpSocket` wraps the datagrams of the WARP stack only, for quinn (`src/warp/quic.rs`).
- `futures` is a dependency (`Cargo.toml`). Boxed futures need no new crate.

## Approach

1. Add a network module to the core with these types: `Target` (a name and a port, or an
   address); `Dialer` (`label`, `tcp`, `udp`, `names_inside`); `Datagrams` (a poll to send and a
   poll to receive, so that a quinn socket can wrap it); `Network` (`scheme`, `caps`, `up`,
   `checks`); `Link` (a dialer, the exports, a status document and a close function); `Caps`.
2. Write the documentation of each type. Add unit tests with a fake dialer.
3. Declare the addressing mode in the types (GitHub #377). A `Target` is a destination (a host and a
   port) or a peer (a name or a shared secret, with no port). Each `Dialer` states which one it takes.
   The combinators `Chain`, `Rules` and `Race` (T-058) refuse a peer dialer where the consumer dials a
   destination, at startup, with one error line. Add a test with a fake peer dialer in a destination
   chain. The issue keeps the sketch `Addressing { Destination, Peer }` and a measured keepalive of 5 s
   through a CONNECT proxy. Add no step to the old `src/transport/mod.rs`: it has no consumer, and M4
   replaces it.

## Decision

The lead, 2026-11-09: no entry for a transport of the gsr network (a third-party relay with a shared secret, TLS-SRP, a new service of another party). Only the addressing contract above goes into this entry. A protocol crate for gsr is a COORD issue (to podcrates), when the owner wants it. The owner can rule otherwise (`docs/decisions.md`).

## Prove

```sh
cargo test -p podnet-core -- network::
cargo doc -p podnet-core --no-deps
```

## Start condition

T-053 is done.

## Correction

2026-11-05, `src/egress.rs`, `src/ws_relay.rs` and `src/warp/` move to `pod-proxy`, `pod-relay` and `pod-warp`
(T-137). A shared crate takes a stream or dialer trait (T-137, Premise). Before step 1, find which shared crate
holds the dialer trait and use it. Do not define a second trait in `podnet-core`.

2026-11-10, the start condition: the types of this entry need only `crates/podnet-core`, which exists (T-046,
T-047). The work starts before T-053. T-053 later gives the types again through the facade.

2026-11-10, the shared crate: `pod-core` of PodUtils/podcrates (tag v0.14.0) holds `Target`, the dialer trait
`Dial` (`label`, `dial`, `datagrams`, `names_inside`), `Datagrams` (the shape of the UDP socket of quinn),
`BoxFuture` and `BoxedIo`. `crates/podnet-core/src/network.rs` gives them again and defines no second copy. The
`Dialer` of the Approach is `pod_core::Dial`, and `udp` is `Dial::datagrams`. A peer target is a
`pod_core::Target` with port 0 (`network::peer`), not a new target type. The addressing mode (GitHub #377) is a
`Way`: a `Dial` and its `Addressing`. `Addressing::admit` is the check that `Chain`, `Rules` and `Race`
(T-058) call at startup.

## Done

2026-11-10, commit d9ac293. `crates/podnet-core/src/network.rs` gives `Target`, `Dial`, `Datagrams`,
`BoxFuture`, `BoxedIo`, `Connected`, `Failure`, `send_to` and `recv_from` of `pod-core` again, and adds
`Addressing`, `peer`, `Way`, `Caps`, `Ctx`, `Export`, `Check`, `Link` and the trait `Network`. `src/lib.rs`
gives the module as `podnet::network`. No command uses the types yet (T-060, T-061).
`cargo test -p podnet-core -- network::`: 8 passed, among them
`network::tests::a_peer_dialer_in_a_destination_chain_is_refused` (the fake peer dialer of GitHub #377 in a
destination chain gives one error line of class `Usage`), `a_fake_dialer_carries_bytes_and_gives_no_udp` and
`a_network_comes_up_and_its_link_closes`. `cargo doc -p podnet-core --no-deps`: no warning.
`sh scripts/agent/check.sh`: each check 0 (1200 tests passed). CI: at the merge.

# T-056: Dialers for direct, the egress proxy, the relay, the pod upstream chains, the tailnet front door and the virtual network

**Source:** `docs/research/podnet/architecture.md`, sections 4.3 and 4.6, step F.
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The dial functions of T-055 have no `Dialer` form.

## Premise

- Direct and the egress proxy: `egress::dial`, `dial_direct` and `Proxy::connect`
  (`src/egress.rs`).
- The relay: `Relay::connect` reaches a public IPv4 TCP address only (`src/ws_relay.rs`).
- The pod upstream chains and the tailnet: `crates/podnet-netns/src/upstream.rs` and
  `crates/podnet-netns/src/tailscale.rs`.
- The virtual network: `NetStack` (`crates/podnet-vnet/src/stack.rs`).

## Approach

1. Write one wrapper for each: direct, proxy, relay (TCP only; `udp` gives the default error),
   upstream chain, tailnet, virtual network.
2. Each wrapper calls the function that exists. Change no behaviour.
3. Test each wrapper on the loopback against the fakes of the tests that exist: a fake SOCKS5
   proxy, the fake relay of `tests/relay_reverse.rs`, a pair of stacks.

## Prove

```sh
cargo test --workspace -- dialer_
```

## Start condition

T-055 is done.

## Correction

2026-11-05, The dial functions of the egress proxy and of the relay are in `pod-proxy` and `pod-relay` after the
takes. The wrappers use the paths of the shared crates.

2026-11-10, the start condition: T-055 is done (d9ac293). `pod-relay` is not taken yet, so the relay wrapper
calls `Relay::connect` of `src/ws_relay.rs`. The frozen files (`src/egress.rs`, `src/ws_relay.rs`) do not change:
the wrappers are new code that calls their public functions. Each wrapper is a `pod_core::Dial`.

2026-11-10, what the wrappers reuse: `pod-proxy` (through `src/egress.rs`) holds the proxy dialer.
`egress::Egress` is a `Dial` already, so `dialers::proxy` and `dialers::process` give `egress::egress` and add no
type. `dialers::Direct` calls `egress::dial_direct`, and its UDP is the `SystemDatagrams` of `pod-core` on an IPv4
socket (that type sends to the address as it is: one family for each socket). `dialers::RelayDialer` calls
`Relay::connect`, and resolves a name on this host with a `pod_core::Resolve` (the resolver chain of the process
by default) to its IPv4 addresses. The wrappers of the upstream chain (`connect_upstream_async`), of the tailnet
(`tailscale::Dialer`) and of the virtual network (`NetStack`) are in `crates/podnet-vnet/src/dialer.rs`: that crate
uses `podnet-core` and `podnet-netns`, so no dependency is new. The UDP of the virtual network is
`VnetDatagrams` (`UdpDatagrams` of `PodUtils/podcrates:crates/pod-netstack/src/stack/datagrams.rs`). The upstream chain and the tailnet carry TCP only:
`datagrams` gives the default error. The WARP dialer is T-057.

## Done

2026-11-10, commit 03a3b49. `src/dialers/` holds `Direct`, `proxy`, `process` and `RelayDialer`;
`crates/podnet-vnet/src/dialer.rs` holds `VnetDialer`, `UpstreamChain` and `Tailnet`, and
`PodUtils/podcrates:crates/pod-netstack/src/stack/datagrams.rs` holds `VnetDatagrams` (as `UdpDatagrams`). No command uses them yet (T-060, T-061).
`cargo test --workspace -- dialer_`: 10 passed:
`dialers::tests::dialer_direct_carries_tcp_and_udp_on_the_loopback`,
`dialer_direct_gives_a_refusal_at_the_stage_connect`, `dialer_proxy_sends_the_name_to_a_socks5h_proxy` (a fake
SOCKS5 proxy on a Unix socket), `dialer_relay_carries_tcp_to_an_address_and_to_a_name` (a fake relay of
`/connect/IP/PORT`); `dialer::tests::dialer_vnet_carries_tcp_and_udp_between_two_stacks` (a pair of stacks on a
switch), `dialer_vnet_refuses_a_name_that_is_not_in_its_table`,
`dialer_upstream_chain_sends_the_name_to_a_socks5_hop`, `dialer_upstream_chain_needs_a_hop_and_hides_the_password`,
`dialer_tailnet_dials_through_the_local_api` and `dialer_tailnet_gives_the_refusal_of_the_daemon` (a fake LocalAPI
of `tailscaled`). The tests of the loopback call `need_loopback!`. `sh scripts/agent/check.sh`: each check 0
(1210 tests passed). `unshare -n -- cargo test --offline --workspace --no-fail-fast`: exit 0.
`sh scripts/restricted-host.sh --no-fail-fast`: exit 0 (113 tests skipped). CI: at the merge.

# T-057: A dialer and datagrams for WARP

**Source:** `docs/research/podnet/architecture.md`, section 4.3.
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

The WARP stack gives TCP and UDP (`WarpStack::tcp_connect`, `UdpConn` in `src/warp/ip.rs`), but
only the WARP commands and the quic+warp leg use them.

## Premise

- `src/warp/quic.rs` wraps the UDP of the WARP stack as a quinn socket for the quic+warp leg.
- The stand-ins of T-012 give a loopback edge.

## Approach

1. Write a `Dialer` over a live WARP stack, with `tcp` and `udp`.
2. Make the quinn socket of `src/warp/quic.rs` wrap any `Datagrams`, and use it for the quic+warp
   leg.
3. Test both through the edge stand-in.

## Prove

```sh
cargo test --workspace -- warp_dialer_
gh workflow run live.yml
```

## Start condition

T-055 and T-012 are done.

## Correction

2026-11-05, There is no crate `podnet-warp`. The WARP adapter is `pod-warp` (T-050). The Dialer belongs to the
adapter that holds the stack (`pod-warp`): do this work after the take, or as a change in PodUtils/podcrates.

# T-058: The combinators `Chain`, `Rules`, `Race` and `Failover`

**Source:** `docs/research/podnet/architecture.md`, section 4.3 (the combinators); GitHub #377.
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Each way to combine networks exists once, inside one command: MASQUE over the relay over a
proxy (`warp::Dial`, `MasqueOptions`), the rules of `warp proxy -r` (`src/warp/rules.rs`) and of
`pod -r`, `NO_PROXY`, and the staggered race of the tunnel legs (`crates/podnet-core/src/race.rs`).

## Premise

- `crates/podnet-core/src/race.rs` races staggered attempts and closes the losers.
- The pod rules use `RuleMatch`, which `src/warp/rules.rs` shares.

## Approach

1. `Chain`: an outer dialer carries an inner one.
2. `Rules`: a dialer for each target, first match wins, with `RuleMatch`.
3. `Race`: staggered attempts over several dialers, with `crates/podnet-core/src/race.rs`.
4. `Failover`: the next dialer after an error.
5. Unit tests with fake dialers: the order, the stagger, the first rule, the failover.
3. The combinators read the addressing mode of T-055 (GitHub #377): `Chain`, `Rules` and `Race`
   refuse a peer dialer in a destination chain, at startup, with one error line.

## Prove

```sh
cargo test -p podnet-core -- network::chain network::rules network::race network::failover
```

## Start condition

T-056 and T-057 are done.

## Correction

2026-11-11, the call of 2026-11-10 (`docs/decisions.md`, Start conditions): the combinators need
only `podnet-core` and no frozen file, so they start before T-057 (the dialer of WARP). The match
of `Rules` is the trait `Matcher` of `podnet-core`: `podnet-core` uses no other crate of the
workspace, so the adapter of `RuleMatch` (`PodRule`) is in `crates/podnet-vnet/src/dialer.rs`.

## Done

2026-11-11, commit 8d85f9f. `crates/podnet-core/src/network/`: `chain.rs` (`Chain`: each network
comes up over the way of the network below it; a failure closes each layer that came up, the
highest first; a layer below another must give a way that dials a destination), `rules.rs`
(`Rules`, `Rule`, `Matcher`, `matcher`, `Choice`: the first rule that matches wins; a rule or a
default with no way refuses with the class `policy`; UDP takes the default way), `race.rs`
(`Race`: staggered attempts with `race::race`, the losers closed) and `failover.rs` (`Failover`:
the next way after an error). `Rules`, `Race` and `Failover` refuse a peer way with one error
line (`Addressing::admit`). `PodRule` gives a rule of `pod -r` to `Rules`. Tests: 25 in
`podnet-core` (`network::`), 2 in `podnet-vnet` (`pod_rule`, `rules_combinator`).

# T-059: The registry of networks, the parser of network specs, and the rows of `doctor`

**Source:** `docs/research/podnet/architecture.md`, sections 3.2 (the two proxy parsers) and 4.3
(the registry).
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Two parsers read a proxy URL with different schemes: `egress::Proxy::parse` (http, socks5,
socks5h, unix, +exec) and the upstream parser of the pod (socks5, http, unix, tailscale). No
table names the networks of a build.

## Premise

- `src/egress.rs` and `crates/podnet-netns/src/upstream.rs` hold the two parsers.
- `docs/research/podnet/architecture.md`, section 4.3: a static table in the binary needs no
  dependency (no `inventory`, no `linkme`) and keeps a static musl build.

## Approach

1. Add a function in the binary that returns the table of networks: one row for each enabled
   crate, with the cargo features `warp`, `cloudflare`, `relay` and `vnet`.
2. Parse specs in the form of `pod -x`: `direct`, `socks5h://...`, `http+exec:CMD`, `warp`,
   `warp:identity=PATH`, `relay:HOST`, `tailscale:/PATH`. Keep each form of both old parsers.
3. Make `doctor` read its rows from `Network::checks`.
4. Extend the fuzz target `proxy_url` to the specs.

## Prove

```sh
cargo test --workspace -- network_spec_
cargo check --manifest-path fuzz/Cargo.toml --bins
```

## Start condition

T-056 and T-057 are done.

## Correction

2026-11-11, the call of 2026-11-10 (`docs/decisions.md`, Start conditions): the registry and the parser
need no frozen file, so they start before T-057. Three parts differ from the Approach:

- The table has one row for each network that has a `Network` now: `direct`, `proxy`, `relay` and
  `tailscale`. The spec `warp` is named and refused with the class `unsupported`; its row comes with
  T-057. The cargo features `warp`, `cloudflare`, `relay` and `vnet` do not exist yet: the WARP and
  Cloudflare adapters move to `PodUtils/podcrates` (T-050, T-052), and the features come with the facade
  (T-053).
- A spec that both old parsers take goes to the proxy of the process (`pod-proxy`: Digest, `socks5h`,
  the commands). A spec with no scheme (`HOST:PORT`) is a SOCKS5 hop, as `pod -x` reads it (the proxy of
  the process reads it as HTTP).
- `doctor` reads the row of the relay from `Network::checks`. The rows of the edge, the WARP API and
  MASQUE become checks of their networks with T-057 and T-052.

## Done

2026-11-11, commit 851a7ea. `src/networks.rs`: `networks()` (the table: scheme, forms, about, caps),
`parse()` (one usage line with no password on an error) and `SpecNetwork` (a `Network`: `up` gives its way
and refuses a way below it with the class `unsupported`; `checks` names the proxy on TCP and the relay).
`Check` has `tls`; `Caps` has the const `with`. `src/doctor/targets.rs` builds the relay row from the
check of the relay network. The fuzz target `proxy_url` parses the text as a spec too. Tests:
`cargo test -p podnet --lib -- network_spec_` (7), the doctor tests, `cargo check --manifest-path
fuzz/Cargo.toml --bins`.


# T-060: `--via SPEC` on each command, with each old network flag as an alias

**Source:** `docs/research/podnet/architecture.md`, sections 4.3 and 6 (question 7).
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Each command has its own network flags: `--proxy`, `--no-proxy`, `tunnel --transport`,
`warp --transport`, `warp --carry`, `pod -x` and `pod -r`. A user learns each one, and a new
network needs a new flag on each command.

## Premise

- The global flags are in `src/main.rs` (after M2, in the CLI module).
- The pod parses its own flags (`crates/podnet-netns/src/cli.rs`).

## Approach

1. Add a global `--via SPEC`, repeatable for a chain.
2. Map each old flag to a spec. Keep each old flag with its behaviour and its help.
3. Make `pod -x` and `pod -r` accept each spec of the registry.
4. Name `--via` in the manual and the help.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, Regressions): no feature or capability of the code is
lost. Each old flag stays.

## Prove

```sh
cargo test --workspace -- via_alias_
cargo test --test cli -- the_man_page_names_each_flag
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
```

For each old flag, a unit test asserts that it builds the same chain as its spec.

## Start condition

T-059 is done.

## Correction

2026-11-11, the owner: "whichever is better, we are beta and must make right decisions even if breaking,
that allow us to extend features later" (the question of the name; `serve` and `git serve` had `--via
tunnel|relay|none` for the road to the visitors). The call of the session: the way out is `--egress SPEC`,
and the road to the visitors is `--ingress tunnel|relay|none`. Two names, one for each direction, and each
can take more specs of the registry later. The Prove filter is `egress_alias_` in place of `via_alias_`.

The alias is a rewrite of the arguments before the parse (`src/cli/way.rs`): each old flag keeps its
code, its help and its checks. `--egress` takes one spec outside `podnet pod`: the egress of the process
is one `pod-proxy` dialer (`src/egress.rs`), so a chain, a hop of the pod or the tailnet stays with
`podnet pod`. `pod -x` and `pod -r` take the specs of the pod hops of the registry; `+exec` and the relay
are not pod hops (a usage line says so). No parity snapshot was run: the change adds flags and renames
one (`--via` of `serve` and `git serve`), and the tests of both commands pass with the new name.

## Done

2026-11-11, commit 6784d84. `--egress SPEC` (global, repeatable for `podnet pod`): `direct` is
`--no-proxy`, a proxy URL is `--proxy`, `relay` and `relay:URL` are `--transport relay` and `--relay`; with
`podnet pod`, each `--egress` is one `-x` hop in its order, and the pod parser takes `--egress` as `-x`. The
pod parser takes `socks5h+unix:/PATH` and `http+unix:/PATH`. Breaking: `serve --via` and `git serve --via`
are `--ingress`; podnet gives a hint for `--via`. Tests: `cargo test -p podnet --bin podnet egress_alias`
(6: each alias parses as its old flag through clap; the refusals; the hint),
`cargo test -p podnet-netns --lib egress_alias`, the tests of `serve`, `git serve` and the relay with
`--ingress`, `the_man_page_names_each_flag`. The notes of `v0.4.0-beta.3` list the rename in `## Breaking`.

# T-061: One front door over a dialer for `warp proxy`, `pod --serve`, `net proxy` and `net tailscale`

**Source:** `docs/research/podnet/capabilities.md`, sections 1.3 and 2.4; GitHub #384.
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** partial

## Problem

Four front doors share one parser of SOCKS5 and HTTP CONNECT (`crates/podnet-netns/src/socks.rs`),
but each has its own accept loop and its own dial side: `src/warp/proxy.rs`,
`crates/podnet-netns/src/frontdoor.rs`, `crates/podnet-vnet/src/proxy.rs` and `crates/podnet-netns/src/tailscale.rs`.
The router of M6 needs a door over any network.

## Premise

- `docs/research/podnet/capabilities.md`, section 1.3, lists the extras of each door: UDP
  ASSOCIATE, `--dns`, rules with a reload, proxy authentication, `--l4`, `--map`, `--import` and
  `--control`.

## Approach

1. Write one door that takes a dialer: SOCKS5, HTTP CONNECT, UDP ASSOCIATE and the
   authentication.
2. Make the four commands build their dialer and call the door. Keep their extras as options of
   the door or of the command.
3. The one door has one deadline for the request, and one cap or counter of live client tasks (GitHub #384).
   The deadline covers the first byte, the SOCKS5 request and the CONNECT head (30 s as `REQUEST_LIMIT`
   in `src/warp/proxy.rs:586`). T-185 fixes the defect first in two doors and names the constant.

## Prove

```sh
cargo test --workspace -- front_door_
gh workflow run live.yml
```

The tests of the four doors that exist pass. A new test runs the door over a fake dialer.

## Start condition

T-058 is done.

## Correction

2026-11-11: `src/warp/proxy.rs` is frozen for `pod-warp` (T-050), so the WARP door keeps its code until the
take. Done so far (5c275ed): `door.rs` of `podnet-vnet` (now `crates/podnet-netns/src/door.rs`), `Door` over any `Dial`: SOCKS5 and HTTP CONNECT
(and an `http://` request in absolute form, through `dial_http`) on one listener, one deadline for the
request of a client (`REQUEST_LIMIT`, 30 s), a cap of the live clients with a count of the clients that
it turned away (GitHub #384), and users for SOCKS5 (RFC 1929) and HTTP (`Proxy-Authorization: Basic`).
Tests: `cargo test -p podnet-vnet --lib front_door` (5). Left: UDP ASSOCIATE through `Dial::datagrams`
(the door answers "command not supported" now), the move of `pod --serve`, `net proxy` and `net tailscale`
onto the door with their extras (`--dns`, the rules and their reload, `--map`, `--import`, `--control`),
and the WARP door at the take of `pod-warp`.

2026-11-12: only the WARP door is left (`src/warp/proxy.rs`, the take of `pod-warp`, T-050). Do not touch
`src/warp/` before that take. UDP ASSOCIATE through `Dial::datagrams` is done: T-098 added it
(`door/udp.rs`, commit d80e561 of the branch of T-098). The three other commands run on the one door.
`Door` moved from `podnet-vnet` to `crates/podnet-netns/src/door.rs`, because `pod --serve` and the
tailnet door are in `podnet-netns`, and that crate does not use `podnet-vnet`. `podnet_vnet::door` is the
same module. The door has these options for the extras of the commands: `with_default_port` (`--port`),
`with_flow_limit` (300 s), `connect_only` (405 for a plain `http://` request), `with_extension` (the
SOCKS5 commands that the dialer cannot carry: the UDP ASSOCIATE and the UDP in the stream of the pod),
and `on_error` (the `-v` line of the pod). The reply follows the class of the failure of the dial.

## Done

2026-11-12, five commits on the branch `worktree-agent-ae070f705e8d9f4a0` (the ids can change at the
merge): 83c9042 (the door moves to `podnet-netns`), 3ff567a (the options of the door), 5e45e88 (`net
proxy`), 3cf0da0 (`net tailscale`), 294d2df (`pod --serve`). Result: `net proxy`, `net tailscale` and
`pod --serve` serve their clients with `Door`. Each keeps its extras:
`net proxy` keeps `--map`, `--map-file`, `--forward`, `--switch`, `--import`, `--control` and `--port` (the
command and the stack read them as before); the 5 s wait for the handshake of a virtual address, the 502
and 504 answers with a body, and the 405 for a plain `http://` request. `net tailscale` keeps the guard of
the tailnet (T-243: SOCKS5 `0x02`, HTTP 403), the 10 s limit of the dial (issue 181), the notes of T-081
and the exit codes of the command. `pod --serve` keeps the `-r` rules and their refusals, `-v`, `--flow-log`
(the dial line and the close line with `bytes_out` and `bytes_in`), UDP ASSOCIATE and UDP in the stream
through the first upstream, and the output line `serving ADDR (N upstream)`. The three doors have the cap of
1024 live clients and the deadline of 30 s for the request (GitHub #384). A change of behaviour: BIND is
`0x07` on `pod --serve` (it was a CONNECT), and an HTTP failure answer of `pod --serve` has a body.
Tests: `cargo test -p podnet-netns --lib` (door: 7 new, `front_door_*`; `frontdoor::tests::the_pod_door_*`,
2); `cargo test -p podnet-vnet --lib proxy` (6 old tests unchanged, 1 new:
`the_proxy_is_the_door_over_the_stack`); `cargo test -p podnet-netns --lib tailscale` (11 old tests
unchanged, 1 new: `the_tailnet_door_gives_port_0_the_port_option_and_takes_connect_only`);
`cargo test --test pod_usage --test pod_seccomp --test vnet --test vnet_proxy_forward` and
`cargo test --test cli` pass unchanged. Areas of `docs/research/podnet/parity-checklist.md` ticked: `net
proxy`, `net tailscale` and `pod --serve`, `-x`, `-r` (section 4). Not proven: a live `tailscaled`, and the
`live.yml` run; UDP ASSOCIATE on `net proxy` and `net tailscale` (they never had it: the answer is `0x07`);
`--control` and `--import` of `net proxy` (the command code did not change; no test of them ran here); the
WARP door (T-050).

# T-062: The service and ingress traits, with wrappers of the Cloudflare tunnel, the local listener, the relay node and the virtual network

**Source:** `docs/research/podnet/architecture.md`, section 4.3 (ingress and services).
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

A service (static files, share, forward, hello, status, metrics) reaches users through the
tunnel, a local listener, a relay node or a virtual network, each with its own code. A new
service (files, git, router) needs one boundary.

## Premise

- `crates/podnet-serve/src/service.rs` has the enum `Built` of the built-in services. `crates/podnet-serve/src/ingress.rs` matches the
  rules of host and path.
- `src/proxy.rs` dispatches an axum router behind the tunnel and streams the request body.

## Approach

1. In `podnet-serve`, add `Service` (`kind`, `http`, `stream`), `Services` (the ordered rules of
   host and path), `Ingress` (`publish`) and `Published` (`url`, `ready`, `shutdown`).
2. Write the wrappers: the Cloudflare tunnel (over `TunnelBuilder`), the local listener (TCP or
   `unix:`), the relay node and a listener of the virtual network.
3. Keep `Built` as the implementation of the built-in services.
4. Add a second table in the binary for the ingress networks.

## Prove

```sh
cargo test --workspace -- ingress_
gh workflow run live.yml
```

A test serves a table of services through the local ingress on a unix socket.

## Start condition

T-055 is done.

## Correction

2026-11-11: the names differ from the Approach. `podnet-serve` has `Ingress` (the rules of the tunnel) and
`Service` (the enum of an origin) already, so the trait is `Road` (`name`, `publish`), the result is
`Published` (`url`, `ready`, `run`; a drop ends the road), and a service is an `axum::Router`.
`Services` holds the ordered rules of host and path over routers. A stream service (TCP) comes with the
doors of the router (T-096 to T-099). The roads of the tunnel, the relay and the virtual network are in the
root crate (`src/roads.rs`): `podnet-serve` uses no other crate of the workspace but `podnet-core`. Each
of them serves the router on a private unix socket of the run directory and points its carrier at it
(`Service::Unix`, `Target::Unix`, a copy from a vnet listener). `serve` and `git serve` keep their code;
their move onto the roads, and `--ingress SPEC` with the forms of `roads()`, come with T-084.

## Done

2026-11-11, commit cad2b33. `crates/podnet-serve/src/road.rs`: `Road`, `Published`, `Services`, `Rule`,
`Local` (TCP, or `unix:PATH` with mode 0600, `claim_socket_path` and `bind_fault`) and `private_socket`.
`src/roads.rs`: `Tunnel` (over `Tunnel::builder`), `Relay` (`run_node` of a pair), `Vnet` (a listener of a
`NetStack`), `roads()` (the table) and `parse()` (`local:ADDR`, `tunnel`, `relay`, `relay:PAIR`). Tests:
`cargo test -p podnet-serve --lib ingress_` (4: the table of services; the local road serves a table on a
unix socket, refuses a live socket and removes its socket; the TCP port; the private socket),
`cargo test -p podnet --lib ingress_` (4: the vnet road serves a table to a second stack on a switch; the
table and the parse of the roads). Not proven here: the tunnel and relay roads need the edge and the relay
(`live.yml`; this sandbox cannot reach them).


# T-237: Front door: an opt-in `--early-reply` answers the client before the upstream connect and pipelines the SOCKS5 upstream handshake (GitHub #428)

**Source:** GitHub #428 (https://github.com/PodUtils/podnet/issues/428); donor review, satmux.md block 1
(Th0rgal/satmux at 2922ebb, MIT; the block is a clean-room description)
**Category:** feature
**Milestone:** M4
**Priority:** P3
**Effort:** M
**Status:** done

## Problem

Add an opt-in `--early-reply` mode to the door of `podnet pod --serve`: answer the client after the
local rule check, send the SOCKS5 greeting and request to the upstream in one write, and hold the
first flight of the client (bounded) until the upstream says yes. Today the door answers its client only
after the whole upstream chain is connected, and its SOCKS5 upstream client waits for two replies in
turn. On a high-latency upstream, three round trips (TCP connect, method reply, CONNECT reply) come
before the first byte of the client. The symptom to fix is the time to the first byte on each new
connection, not the bandwidth.

## Premise

- The door answers after the dial. `crates/podnet-netns/src/frontdoor.rs:121-138` dials with
  `connect_upstream_async`, then replies with `socks5_reply` at `crates/podnet-netns/src/frontdoor.rs:138`.
  The HTTP CONNECT arm does the same at `crates/podnet-netns/src/frontdoor.rs:165-175`.
- The upstream client waits for two replies in turn: `crates/podnet-netns/src/upstream.rs:303-318`
  (`socks5_handshake`: the greeting, then `read_exact` of the method reply) and
  `crates/podnet-netns/src/upstream.rs:380-413` (`socks5_connect`: the request, then `read_exact` of the
  reply head). A hop with `proxy.user` offers the methods `[0x00, 0x02]`, a hop without it `[0x00]`
  (`crates/podnet-netns/src/upstream.rs:309-313`).
- `crates/podnet-netns/src/door.rs:244` and `:270` replies after the dial too.
- The door of podnet reads a pipelined client: `crates/podnet-netns/src/socks.rs:61` (`socks5_read_request`)
  reads the greeting, the methods and the request with sequential `read_exact`.
- `src/warp/proxy.rs` has its own dial (the replies at `src/warp/proxy.rs:657-733`). That door is frozen
  for `pod-warp` (T-061, Correction). Leave it out.
- T-061 (partial) plans one door over a dialer. It says nothing about the order of the reply. This entry
  stays apart, so that T-061 stays a refactor. The option belongs in the one door of T-061.
- GitHub #384 (T-185) is about the deadline of the handshake: a different thing.
- Not measured: any number on a real satellite or cellular link; that third-party SOCKS5 servers accept a
  pipelined greeting; the timings of the donor (they are the author's).

## Approach

1. Add the flag `--early-reply`, default off, on the door.
2. Tier A, retry-safe: write the greeting and the CONNECT request in one write, then read the two replies.
   No byte of the client has left the host, so a failed attempt can go to the next route. Use it only for
   a hop with no credentials (`proxy.user` is `None`), because a user and password hop needs the method
   reply before the next bytes.
3. Tier B, optimistic data: also reply to the client at once, and forward its first flight right behind
   the request. Hold at most 64 KiB for each client (TCP backpressure stops the read when the buffer is
   full). A half-close of the client before the dial ends goes to the upstream after the held bytes.
4. The rule check stays before the reply. A rejected target gets its real answer (`0x02` or `403`), never
   an early one.
5. An upstream that refuses (a reply code other than `0x00`): the client sees a closed connection. Add a
   `refused` event with the target and the upstream code to the flow log
   (`crates/podnet-netns/src/flowlog.rs`), so that the lost diagnosis is recoverable.
6. The deadline `within_timeout("front door connect", ...)` still applies, now after the reply.
7. Put the option in the one door of T-061, so that it exists once.

## Decision

A default-off flag keeps the precise error codes that automated callers read. The option does not apply to
`podssh -D`: an SSH channel cannot carry data before its open is confirmed. It does not cover TLS early
data or UDP.

## Prove

```sh
cargo test -p podnet-netns --lib -- early_reply_cuts_the_round_trips early_reply_holds_at_most_64_kib
cargo test -p podnet-netns --lib -- early_reply_keeps_the_refusal_of_a_rule early_reply_is_off_by_default
```

The first test uses a fake SOCKS5 upstream that delays each direction by D. With the flag off, the first
echoed byte arrives after about 3 round trips. With the flag on, it arrives after about 1. Assert the
ratio with a margin, not a fixed time. The other cases: the upstream answers with code 5 (flag off: the
client gets `0x05`; flag on: the client sees EOF and the log names the target and the code 5); a client
that half-closes before the connect; a client that sends 1 MiB before the connect ends; an upstream with
a user and a password (the serial path runs, the flag changes nothing).

## Start condition

T-061 has moved `pod --serve` onto `Door`. Tier A (one write of the greeting and the request, only for a
hop with no credentials) can land earlier in `crates/podnet-netns/src/upstream.rs`, because it is
retry-safe.

## Correction

2026-11-12: the start condition holds. T-061 has moved `pod --serve` onto `Door` (branch commits 83c9042 to
294d2df). The code of the Premise has moved. The door is `crates/podnet-netns/src/door.rs` (it was
in `podnet-vnet`). The dial of `pod --serve` is `PodDial` in
`crates/podnet-netns/src/frontdoor.rs`. It replies after the dial in `Door::handle`
(`socks5_reply`, `http_connect_ok`), and `serve_one` is gone. The place for the flag is `Door` (an option
beside `with_default_port`) with a change of the reply order in `handle`, and the upstream client in
`crates/podnet-netns/src/upstream.rs` (`socks5_handshake`, `socks5_connect`). This correction does
not do the entry.

## Done

2026-11-12, commit 40b573c. `podnet pod --serve --early-reply` (default off) answers a SOCKS5 or HTTP CONNECT
before the dial. `Door::with_early_reply` (`crates/podnet-netns/src/door.rs`) takes the local rule check
`Admit`; `PodDial::admits` in `crates/podnet-netns/src/frontdoor.rs` gives it from the rules. A target that
the rules refuse takes the old path: the real answer (`0x02`, `403`). Other targets get the success reply,
then the door dials and holds at most `EARLY_HOLD` (64 KiB) of the client. A half-close of the client goes to
the upstream after the held bytes. A failed dial closes the connection. The upstream handshake has the
orders `Handshake::{Serial, Pipelined, Optimistic}` (`crates/podnet-netns/src/upstream/early.rs`): tier A
writes the greeting and the request in one write (a hop without a user; the hops in front of the last hop);
tier B returns the last hop before its two replies (`Unconfirmed`), so that the bytes of the client follow the
request. A hop with a user, and an HTTP hop, stay serial. The wait for the replies has the 30 s limit.
`flowlog::refused` writes the line `refused` (target, upstream code), with or without the option. The
other doors (`net proxy`, `net tailscale`, `route`) and `src/warp/` do not get the flag. `docs/proxies.md`,
"Early reply", states what the client sees after a failed early reply. The help, the README of
`podnet-netns` and the llms files are updated.
Tests (each fails with the change disabled, shown once: 4 of 6 door tests and 2 of 4 pod door tests failed;
`early_reply_is_off_by_default` and `early_reply_keeps_the_refusal_of_a_rule` guard the old behaviour and pass
both ways):
`door::early_tests::` `early_reply_is_off_by_default`, `early_reply_answers_before_the_dial_and_forwards_the_held_bytes`,
`early_reply_keeps_the_refusal_of_a_rule`, `early_reply_closes_the_connection_when_the_dial_fails`,
`early_reply_sends_a_half_close_after_the_held_bytes`, `early_reply_holds_at_most_64_kib` (1 MiB sent, the
writer stops near 64 KiB plus the stream buffer, all 1 MiB arrives in order);
`frontdoor::early_tests::` `early_reply_cuts_the_round_trips` (a delay of 100 ms each way: 607 ms off, about
200 ms on), `early_reply_shows_a_refusal_as_a_closed_connection` (code 5), `early_reply_keeps_the_pod_rule_refusal`,
`early_reply_runs_the_serial_path_for_a_user_hop`; `upstream::early::tests::` 6 tests (one write of the
greeting and the request, the user hop stays serial, the data behind the request, the refusal code, the
time limit, the typed refusal); `flowlog::tests::the_refused_line_names_the_target_and_the_code`;
`tests/pod_early_reply.rs` 3 tests (the `podnet` binary with `--flow-log`: the client gets `0x00` and EOF, the
log names `refused.example:443` and code 5; without the option the client gets an error code; the flag needs
`--serve`). `sh scripts/agent/check.sh`: each check 0. `cargo test --workspace --no-fail-fast`: exit 0,
1493 passed, 0 failed, 2 ignored, in 55 test binaries.
Not proven: any number on a real satellite or cellular link (the delay is a loopback forwarder); that a
third-party SOCKS5 server accepts a pipelined greeting and request; the optimistic data behind the request on
a server that closes on unexpected bytes. The entry said that, without the option, an upstream code 5 gives
`0x05`; the door gives `0x01` (the failure reply by the words of the text), and the test asserts "not 0".
