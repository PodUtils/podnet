# The iroh road, and a relay and mailbox of podnet's own

> Research of T-130. Date of reading: 2026-11-11. Nothing was run against a service. No account was
> used. The build trial made only a lock file and a dependency tree. No binary was built, so no
> binary size was measured. Each fact has a mark.

Marks:

* **read**: the fact is in a document or in source code. The source is a URL, read on 2026-11-11.
  "Registry copy" means the crate source that cargo downloaded to the local registry. It is the same
  text as the file at the docs.rs URL of the same version.
* **measured**: a command gave the number on 2026-11-11. The command is in the text.
* **inferred**: the fact follows from read or measured facts. It was not run.

## 0. Result

| Question | Answer | Mark |
| --- | --- | --- |
| Does the iroh road need aws-lc (C)? | No. The feature `tls-ring` of `iroh` and `iroh-relay` uses ring. `tls-ring` is a default feature. The lock of podssh links aws-lc because podssh chose `tls-aws-lc-rs`. | read, measured |
| What does the iroh road add to the lock of podnet? | 63 crates in the normal graph (Linux), with `default-features = false` and `tls-ring`. No `aws-lc-sys`. | measured |
| Is Rust 1.91 a limit? | No. The `rust-version` of `iroh` 1.5.0 is 1.91 and podnet has 1.99. | read |
| Can the iroh road work with a proxy-only egress? | Only through a relay. The relay client does an HTTP `CONNECT` to the relay name and a WebSocket upgrade. Direct paths need UDP. | read, inferred |
| Is the iroh road worth its cost now? | Not now. It adds a second relay system. See section 2.5. | inferred |
| Port of the relay role of podssh (about 14,000 lines)? | Do not port it whole. Take it as `pod-relay-serve` in parts. The mailbox first (629 lines). | inferred |

The recommendation is in section 5. The questions for the owner are in section 6. The proposed
entries are in section 7.

## 1. The target (what a road of podnet must fit)

All facts of this section are **read** from the repository.

* `docs/target-environment.md`: the hosts have no `CAP_NET_ADMIN`. Egress is often one HTTP `CONNECT`
  proxy that allows TCP 443 to named hosts. `CONNECT` to an IP address can be refused. UDP can be
  blocked. `bind` can work while `listen` is refused.
* The WebSocket relay of podnet (`src/ws_relay.rs`, `src/relay_pool.rs`, `src/relay_reverse.rs`,
  2,514 lines in total with the reverse road) is a client of the relay of `tcp.ssh.relay.ajam.dev`. It
  carries TCP only, through the egress proxy. It has no server.
* `crates/podnet-serve/src/road.rs` and `src/roads.rs`: a road publishes a router to visitors. The
  rows are `local`, `tunnel`, `relay` and `vnet`. A road that needs a carrier serves the router on a
  private unix socket and points the carrier at it.
* `crates/podnet-serve/src/relay.rs` (671 lines) is not a relay server. It is an SSH forward leg
  (`russh`) to a public SSH relay (`pinggy`).
* `Cargo.toml` of podnet: `rust-version = "1.99"`. Crypto is ring (`rustls`, `rcgen`, `russh` with
  `ring`). The lock has no `aws-lc-sys`. A new road must keep this.
* `docs/decisions.md`, 2026-11-05: file transfer speaks magic-wormhole. The public mailbox and transit
  servers are the default. The podnet relay is one more carrier. `podnet-relay` merges into
  `pod-relay`. The relay role is `pod-relay-serve` and the iroh road is `pod-iroh` (wave 4 of
  `PodUtils/podcrates`).
* `TODO/PROGRESS.md`: T-126 to T-130 have no ruling. Each asks the owner before its work starts.
* No file of the repository holds a mailbox server or a mailbox client yet (`grep -rl mailbox
  --include=*.rs` gives no file). The transfer commands wait for the takes of `pod-wormhole` and
  `pod-transit` (T-102, T-103).

## 2. The iroh road

### 2.1 What iroh is

* **read**: iroh opens QUIC connections between endpoints that are named by a public key. It tries
  hole punching first. If that fails, it falls back to relay servers. Source:
  <https://raw.githubusercontent.com/n0-computer/iroh/main/README.md>, 2026-11-11.
* **read**: the licence is Apache-2.0 or MIT. Same source. This fits 0BSD code and `deny.toml`
  (inferred: not run through `cargo deny`).
* **read**: the traffic of a relay is encrypted QUIC between the two endpoints. The relay "temporarily
  routes encrypted traffic until a direct, P2P connection is feasible". Source:
  <https://raw.githubusercontent.com/n0-computer/iroh/main/iroh-relay/README.md>, 2026-11-11.

### 2.2 Build: aws-lc, ring, size of the graph (Approach 1)

* **read**: `iroh` 1.5.0 on the `main` branch has `rust-version = "1.91"` and
  `default = ["metrics", "fast-apple-datapath", "portmapper", "tls-ring"]`. `tls-ring` turns on
  `noq/ring`, `iroh-relay/tls-ring` and `iroh-dns/tls-ring`. `tls-aws-lc-rs` is not a default.
  Source: <https://raw.githubusercontent.com/n0-computer/iroh/main/iroh/Cargo.toml>, 2026-11-11.
* **read**: `iroh-relay` has `default = ["metrics", "tls-ring"]` and the features `server`,
  `tls-aws-lc-rs`, `platform-verifier`. Source:
  <https://raw.githubusercontent.com/n0-computer/iroh/main/iroh-relay/Cargo.toml>, 2026-11-11.
* **measured**: a scratch crate (outside the repository) depended on `iroh = "1"` with
  `default-features = false, features = ["tls-ring"]`. `cargo generate-lockfile` chose `iroh` 1.4.1
  (the released version; 1.5.0 is not on crates.io yet, because it needs a prerelease dependency:
  cargo refused `=1.5.0`). The lock had 371 packages for all targets. `cargo tree -e normal -i
  aws-lc-sys` found no package. `ring` 0.17.14 came in through `noq-proto`.
* **measured**: the same crate with `features = ["tls-aws-lc-rs"]` links `aws-lc-sys` 0.47.0 (C). So
  the premise "the iroh road links aws-lc" is true only for the choice of podssh.
* **measured**: the names in the normal graph of the ring variant that are not in the `Cargo.lock` of
  podnet (414 packages): 63 crates, not counting the scratch crate. Among them: `iroh`, `iroh-base`,
  `iroh-relay`, `iroh-dns`, `iroh-metrics`, `n0-future`, `n0-error`, `n0-watcher`, `netwatch`,
  `netdev`, `netlink-*` (4), `reqwest`, `hyper-rustls`, `rustls-platform-verifier`,
  `rustls-native-certs`, `tokio-websockets`, `postcard`, `blake3`, `strum`, `lru`, `slotmap`,
  `zerocopy`, `simple-dns`. The QUIC stack is `noq` 1.4.0, which podnet already has through
  `pod-quic`. The "about 520 packages" of the premise is the lock of the whole podssh workspace, not
  the cost of the iroh road.
* **measured**: with the default features of `iroh` (adds `portmapper`), the lock has 371 packages too
  and 2 crates named `ring` in the tree. With `tls-ring` only, one `ring`.
* **not measured**: the size of a release binary. This needs a program that uses `Endpoint`. A binary
  of `fn main(){}` would drop all of iroh. The first proposed entry (E1) measures it.
* **inferred**: `ed25519-dalek` 3 (a major version that podnet does not have) comes with iroh. The
  lock of podnet then holds two major versions of that crate. This is a size cost, not a conflict.

### 2.3 Egress: what works with proxy-only hosts (Approach 1, the road)

* **read** (registry copy of `iroh-relay` 1.4.1, `src/client/tls.rs`, equal to
  <https://docs.rs/crate/iroh-relay/1.4.1/source/src/client/tls.rs>): the relay client has
  `proxy_url`. With a proxy, it dials the proxy, optionally with TLS, and sends `CONNECT HOST:PORT`
  with the host name of the relay (not an IP address), `Proxy-Connection: Keep-Alive` and a
  `Proxy-Authorization` header when the proxy URL has a user name. Then it does TLS and the WebSocket
  upgrade to the path `/relay` (`src/http.rs`: `RELAY_PATH = "/relay"`) with a
  `Sec-WebSocket-Protocol` header. The proxy resolves the relay name. This is the same shape as the
  other roads of podnet that work in the measured sandbox.
* **read** (registry copy of `iroh` 1.4.1, `src/endpoint.rs`): the builder has `proxy_url(Url)` and
  `proxy_from_env()`. The second reads `HTTP_PROXY`, `http_proxy`, `HTTPS_PROXY`, `https_proxy` in
  this order. Source of the release notes on proxies:
  <https://newreleases.io/project/github/n0-computer/iroh/release/v1.4.0>, 2026-11-11 (search result:
  relay and pkarr traffic go through the configured proxy).
* **inferred**: only an `http://` or `https://` proxy URL is handled by this code. The networks of
  podnet that are SOCKS5, unix socket or exec specs (`src/networks.rs`) cannot be given to iroh as
  they are. podnet needs a local `CONNECT` shim, or iroh needs a custom transport. The builder has an
  example `custom-transport.rs` in the registry copy (not read in detail). Not verified.
* **inferred**: the QUIC socket of iroh uses UDP. With UDP blocked, no direct path forms, and iroh
  runs over the relay only. The relay carries each QUIC datagram in a WebSocket frame. The path is
  QUIC over WebSocket over TCP 443: a double congestion control, with the head-of-line delay of TCP.
  This is not measured.
* **inferred**: `Endpoint::bind` opens UDP sockets and `netwatch` reads the network state with
  netlink (the `netlink-*` crates). A host that forbids UDP `bind` or `AF_NETLINK` can fail at start.
  `docs/target-environment.md` has no row for UDP `bind` or netlink. Not measured. E1 measures it on
  the sandbox of the lead.
* **read**: the first-class use of the relay is the ID of a relay in the address of an endpoint. A
  peer needs the relay URL and the key. A ticket can carry both. Source:
  `examples/connect.rs` in the registry copy of `iroh` 1.4.1 (args `--endpoint-id`, `--relay-url`).

### 2.4 The relays that exist

* **read**: the public relays of n0 are free. They are "meant for development and hobby use". The
  limits are per connection, by a token bucket on the receive side, and the numbers are not
  published. The relay pauses the read, so the client sees back pressure. Source:
  <https://docs.iroh.computer/relays/rate-limiting>, 2026-11-11.
* **read**: shared relays (5 MB/s, API key) and dedicated relays are paid services of n0. Source:
  <https://docs.iroh.computer/iroh-services/relays/shared> (search result summary), 2026-11-11.
  podnet does not use paid services (inferred from the rule "measure nothing that needs an account").
* **read**: a self-hosted relay is `iroh-relay` with the feature `server`: HTTP on one port, HTTPS
  with ACME or files, and an optional QUIC address discovery port (default 7842 in
  `src/defaults.rs` of the registry copy; the README of the `main` branch says 7824 in a local
  example, so the two sources disagree). The access control is an allow or deny list by endpoint ID, a
  shared token, or an HTTP call-out. Source:
  <https://raw.githubusercontent.com/n0-computer/iroh/main/iroh-relay/README.md>, 2026-11-11.
* **measured**: `iroh-relay` with `server` and `tls-ring` adds 55 crates to the lock of podnet
  (names in the normal graph that are not in the lock), and has no `aws-lc`. Among them:
  `tokio-rustls-acme`, `dashmap`, `toml`, `rustls-cert-file-reader`, `rustls-cert-reloadable-resolver`.
  The sources of `iroh-relay` 1.4.1 hold about 12,500 lines in all (client and server, counted in the
  registry copy).
* **inferred**: a visitor of a hobby relay of n0 is a host name that the proxy allow list may not
  hold. The relay host name of n0 was not read. A host that allows only the hosts of podnet needs
  the relay of podnet or a host that the user adds.

### 2.5 Assessment

* **inferred**: where UDP works, podnet has other roads (QUIC edge, WARP, WireGuard). The gain of
  iroh there is hole punching between two hosts that have no public address. This is the use that
  file transfer and pods need.
* **inferred**: where only TCP 443 works, iroh is a second relay system. It has its own relay
  protocol, its own key identity (Ed25519, as `pod-identity`), and its own relay servers. The relay of
  podnet already gives TCP 443 reach, the reverse road, and (after T-137) the resumable layer and the
  end-to-end channel of `pod-session` and `pod-e2e`. Those do the job of iroh over a relay.
* **inferred**: the cost is 63 crates, a second toolchain of QUIC options, and a second key and relay
  list for the user. The gain is one: key-to-key links with hole punching where UDP is open.
* **inferred**: `pod-iroh` is in wave 4 of podcrates. Code that a crate of podcrates will hold is not
  developed in podnet (`CLAUDE.md`, "Order"). So podnet takes the road when `pod-iroh` is released.

## 3. The relay role and the mailbox

### 3.1 What the relay role of podssh does

All facts are **read** from <https://raw.githubusercontent.com/Azathothas/podssh/main/docs/relay.md>
(2026-11-11; the first 100,000 characters of 107,339) and from `docs/research/podnet/podssh-copy.md`.

* Forward road: `/connect/<host>/<port>` carries raw TCP over a WebSocket. A policy checks the
  targets. `--routes` can allow private ranges.
* Token mint: `/v1/mint`. `--mint` chooses who gets a token: anyone, the keys of a list, or the members
  of a net.
* Reverse road: pairs, node and operator sockets, renewal, halt, spares.
* Mailbox: `/v1/mailbox`, a meeting point for short codes with the magic-wormhole protocol.
* Resumable layer: `/v1/layer/`, which keeps a session alive when the client link drops.
* Egress: direct, or through the proxy of the host. Roads: cleartext, HTTP/1.1, HTTP/2 (RFC 8441
  extended `CONNECT`), QUIC (ALPN `podssh-relay/1`). A mode behind a Cloudflare quick tunnel. Probes
  `/v1/probe/drip` and `/v1/probe/anything`.
* Own HTTP stack: TLS 1.3 with a P-256 key in pure Rust, reload on SIGHUP, ACME.
* Limits (examples): session 12 h and 64 MiB each way, 180 s idle; frame 262,144 bytes; 120
  requests per minute per address on mint and pair; 256 connections per address; 4,096 pairs; 4,096
  nameplates and 4,096 mailboxes; 256 MiB resume budget.
* Size: `serve/` 11,793 lines, `serve/mailbox/` 629 lines, `podssh-cli/src/relay_serve/` 1,730
  lines (`podssh-copy.md`, table of crates). Total about 14,150.

### 3.2 The mailbox server protocol

* **read**: each client keeps a WebSocket connection. Messages are JSON, sent as binary frames. The
  messages are `bind`, `list`, `allocate`, `claim`, `release`, `open`, `add`, `close`, with replies
  `allocated`, `claimed`, `released`, `message`, `nameplates`, `error`. A nameplate is a short
  number. A mailbox holds the messages of two sides. The server keeps them in a database, so a
  restart does not lose them. The protocol gives no limits on size or rate. Source:
  <https://raw.githubusercontent.com/magic-wormhole/magic-wormhole-mailbox-server/master/docs/server-protocol.md>,
  2026-11-11. The page does not name a licence.
* **read**: the mailbox server of podssh is 629 lines and the mailbox client is 324 lines
  (`podssh-copy.md`, section on `code/`).
* **inferred**: a mailbox server is a small state machine over WebSocket, with a table of nameplates
  and a table of mailboxes. It needs no TLS stack of its own when it runs behind the tunnel or the
  relay. It can use `axum` with the `ws` feature (axum 0.8 is in podnet already; the `ws` feature adds
  `tokio-tungstenite`, which podnet has: lock version 0.30.0). This was not built.
* **inferred**: the use for podnet is a mailbox that the user hosts when the public mailbox hosts are
  not on the proxy allow list. `podnet send` and `receive` then run with `--mailbox URL` against the
  mailbox of a second podnet. The host of the user is in the allow list by assumption of the user.

### 3.3 The port onto the HTTP stack of podnet (Approach 2)

Nothing was ported. The sources of podssh were not copied or built here. The estimate:

| Part | Lines in podssh | Fit in podnet | Mark |
| --- | --- | --- | --- |
| Mailbox server | 629 | `axum` WebSocket route. Small. Needs the protocol above and a store (memory, or a file) | inferred |
| Forward road `/connect`, mint, policy | not listed apart | `axum` route plus `pod-ws` (podnet has `pod-ws`, `src/ws.rs` 456 lines). Token logic exists as the client of `pod-relay` | inferred |
| Reverse road, pairs, node sockets | not listed apart | The client is `src/relay_reverse.rs` (891 lines). The server side is new | inferred |
| Resumable layer `/v1/layer/` | part of `session/` 7,468 | Comes with `pod-session` (T-137) | inferred |
| Own TLS 1.3 stack | part of 11,793 | Do not port. podnet has `pod-tls` and `rustls` with ring | inferred |
| HTTP/2 extended `CONNECT`, QUIC road, cleartext road | part of 11,793 | Later. `pod-quic` exists. The others add little for the target | inferred |
| ACME, probes, quick tunnel mode | part of 11,793 | Later (`pod-acme` is wave 4) | inferred |

The own TLS stack, and the roads other than WebSocket, are most of the 14,000 lines (inferred: the
split is not in `podssh-copy.md`; the file `serve/` has sub-parts that were not counted here). A port
of the mailbox and the forward road with axum and the existing TLS crates is much smaller than the
whole. The size was not measured. E2 and E3 measure it as part of the work.

### 3.4 Where the code belongs

* **read**: the owner ruled that shared code lives in `PodUtils/podcrates` (`docs/decisions.md`,
  2026-11-05). The relay role is the planned crate `pod-relay-serve`. The name `pod-mailbox` is held.
* **inferred**: the mailbox server is a separate crate or a module of `pod-relay-serve`. podnet does
  not develop it. podnet takes it by a tag. The `podnet relay serve` command and its flags
  (`--mint`, `--mailbox`, limits) are new code in podnet. So the work in podnet is the command and the
  tests, after the take.

## 4. Options

| Option | What | Cost | Gain |
| --- | --- | --- | --- |
| A | Nothing now. Keep the relay client and the public mailbox | none | none; the host with a blocked public mailbox cannot use `send` |
| B | Mailbox server first (`podnet relay serve --mailbox`) | small; axum route, store, tests; takes `pod-mailbox` or the module of `pod-relay-serve` | a self-hosted rendezvous for transfer |
| C | B plus the forward road and the mint (a full relay of podnet's own) | medium; takes the crate `pod-relay-serve` in parts | a user can run the relay of `tcp.ssh.relay.ajam.dev` on a host of the user |
| D | The iroh road as a network row (`iroh:...`), behind a feature | 63 crates, a CONNECT shim for non-HTTP proxy specs, a second relay list | hole punching between hosts with open UDP |
| E | D plus the iroh relay server (`iroh-relay` with `server`) | 55 more crates | a self-hosted iroh relay |

## 5. Recommendation

1. Take B, then C, in the order of the need of transfer. The mailbox is small and it removes a
   dependency on a public service that a proxy can block. Do it through `pod-relay-serve` (or
   `pod-mailbox`) after podcrates releases it, as the rulings of 2026-11-05 say.
2. Do not port the whole relay role. Do not port its own TLS stack or its QUIC, HTTP/2 and cleartext
   roads. Use `pod-tls` and `axum`. Take the other roads only when a measured host needs them.
3. Do not take the iroh road now (option D). Wait for `pod-iroh`. Before that, run the spike E1 to
   measure the binary size, the start on the sandbox (UDP `bind`, netlink), and the path through a
   `CONNECT` proxy to a local `iroh-relay` on the loopback. The spike needs no account.
4. If the owner takes D, make it a cargo feature that is off in the default build, with
   `default-features = false` and `tls-ring`. Never use `tls-aws-lc-rs` (`deny.toml` and the
   static musl build keep C out).
5. Do not take E. The relay of podnet (C) does the job for the hosts that need a relay.

## 6. Questions for the owner

1. Mailbox server (option B): accept as a part of `podnet relay serve`, taken from `pod-relay-serve`
   or `pod-mailbox`? Recommendation: yes.
2. Full relay role (option C): accept in parts (forward road and mint first), with no own TLS stack?
   Recommendation: yes, after B.
3. iroh road (option D): defer until `pod-iroh` exists and until the spike E1 gives the binary size?
   Recommendation: yes, defer.
4. Self-hosted iroh relay (option E): decline? Recommendation: decline.

When the owner does not answer, the call is: B and C go on after the takes of T-137; D waits;
E is declined (`CLAUDE.md`, "Decisions").

## 7. Proposed entries

The lead opens these. Each needs the ruling of the owner on the question that is named.

| Id | Title | Depends on | Effort | Prove |
| --- | --- | --- | --- | --- |
| E1 | Spike: an iroh `Endpoint` (ring) in a scratch binary: release size, start on the sandbox, and the path through a `CONNECT` proxy to a local `iroh-relay` | question 3 | M | A test-only report in `docs/research/podnet/`; no code in the workspace |
| E2 | `podnet relay serve --mailbox`: the magic-wormhole mailbox server on `axum`, with an interop test against the Python `magic-wormhole` on the loopback | question 1; T-137 take of `pod-relay-serve` or `pod-mailbox`; T-102 | M | `cargo test` of the mailbox; `scripts/wormhole-peer.py` pattern |
| E3 | `podnet relay serve`: forward road, mint and policy on the take of `pod-relay-serve`, with the existing relay client as the test peer | question 2; E2 | L | `cargo test`; a loopback run of `podnet relay connect` against it |
| E4 | `podnet send --mailbox URL` and `receive --mailbox URL` use the mailbox of E2 | E2; T-102, T-103 | S | A loopback transfer test |
| E5 | The iroh road as a network row behind a feature, with a `CONNECT` shim for SOCKS5 and unix-socket specs | question 3; E1; `pod-iroh` release | L | `cargo test --features iroh`; `cargo deny check` with no `aws-lc` |

## 8. Not verified

* The size of a release binary with iroh (needs E1).
* The behaviour of `Endpoint::bind` on a host that forbids UDP `bind` or netlink.
* The speed of QUIC over the WebSocket relay of iroh through a proxy.
* The lines of the relay role of podssh by part (the `serve/` sub-parts were not counted).
* The licence of the mailbox server protocol document, and the host names of the n0 relays.
* Whether `cargo deny` accepts the 63 crates (not run).
* The last 7,339 characters of `docs/relay.md` of podssh (not read).

## 9. Sources

| URL | Read |
| --- | --- |
| <https://raw.githubusercontent.com/n0-computer/iroh/main/iroh/Cargo.toml> | 2026-11-11 |
| <https://raw.githubusercontent.com/n0-computer/iroh/main/iroh-relay/Cargo.toml> | 2026-11-11 |
| <https://raw.githubusercontent.com/n0-computer/iroh/main/iroh-relay/README.md> | 2026-11-11 |
| <https://raw.githubusercontent.com/n0-computer/iroh/main/README.md> | 2026-11-11 |
| <https://docs.rs/crate/iroh-relay/1.4.1/source/src/client/tls.rs> (read as the registry copy) | 2026-11-11 |
| <https://docs.rs/crate/iroh-relay/0.98.0/source/src/client.rs> | 2026-11-11 |
| <https://newreleases.io/project/github/n0-computer/iroh/release/v1.4.0> (search result) | 2026-11-11 |
| <https://docs.iroh.computer/relays/rate-limiting> | 2026-11-11 |
| <https://docs.iroh.computer/iroh-services/relays/shared> (search result) | 2026-11-11 |
| <https://raw.githubusercontent.com/Azathothas/podssh/main/docs/relay.md> | 2026-11-11 |
| <https://raw.githubusercontent.com/magic-wormhole/magic-wormhole-mailbox-server/master/docs/server-protocol.md> | 2026-11-11 |
