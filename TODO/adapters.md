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
`VnetDatagrams` (`crates/podnet-vnet/src/stack/datagrams.rs`). The upstream chain and the tailnet carry TCP only:
`datagrams` gives the default error. The WARP dialer is T-057.

## Done

2026-11-10, commit 03a3b49. `src/dialers/` holds `Direct`, `proxy`, `process` and `RelayDialer`;
`crates/podnet-vnet/src/dialer.rs` holds `VnetDialer`, `UpstreamChain` and `Tailnet`, and
`crates/podnet-vnet/src/stack/datagrams.rs` holds `VnetDatagrams`. No command uses them yet (T-060, T-061).
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
**Status:** open

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

# T-059: The registry of networks, the parser of network specs, and the rows of `doctor`

**Source:** `docs/research/podnet/architecture.md`, sections 3.2 (the two proxy parsers) and 4.3
(the registry).
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-060: `--via SPEC` on each command, with each old network flag as an alias

**Source:** `docs/research/podnet/architecture.md`, sections 4.3 and 6 (question 7).
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-061: One front door over a dialer for `warp proxy`, `pod --serve`, `net proxy` and `net tailscale`

**Source:** `docs/research/podnet/capabilities.md`, sections 1.3 and 2.4; GitHub #384.
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** open

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

# T-062: The service and ingress traits, with wrappers of the Cloudflare tunnel, the local listener, the relay node and the virtual network

**Source:** `docs/research/podnet/architecture.md`, section 4.3 (ingress and services).
**Category:** feature
**Milestone:** M4
**Priority:** P2
**Effort:** M
**Status:** open

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
