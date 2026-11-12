# Stand-ins for the Cloudflare Tunnel edge and the quick-tunnel service

Date: 2026-11-12. Scope: research (sections 1 to 8). Section 9 gives what
podnet built from it (T-316).

## 1. Question

Is there an open-source, self-hostable stand-in for the Cloudflare Tunnel
edge and for `api.trycloudflare.com`? podnet CI could run it locally. Today
`live.yml` calls Cloudflare and gets HTTP 429 (error 1015)
(`docs/protocol.md`, section "Quick tunnels", at podnet commit
e0465600ac77c7e12ac372658016d96f5eff9a2a).

## 2. Answer

* No complete, standalone, open-source stand-in exists. No project ships
  an edge server binary or a service that cloudflared or another client can
  use. None implements visitor routing from the Internet.
* One partial stand-in exists as test code: the `fake_edge` module of
  `flolep2607/cctop` (section 4.0). It is a Rust edge for QUIC and HTTP/2
  that answers `registerConnection`, pushes configuration, and opens
  visitor streams. It has a fake quick-tunnel API in its tests. It is
  `#[cfg(test)]` code inside one crate, licensed MIT OR Apache-2.0. podnet
  can port it. This is the best base.
* The upstream cloudflared repository has fake edges in Go `_test.go` files
  only (section 4.1). The capnp schema and the Go server-side RPC types are
  Apache-2.0.
* Section 6 lists what podnet must still build.

## 3. open-compute (first request)

Source: https://github.com/elliothux/open-compute at commit
bca30aa3c6da51e9e3520727e41b59c2dd379cf9 (2026-11-08). Apache-2.0
(`LICENSE`).

* It is a self-hosted platform that is compatible with Cloudflare Workers
  (`README.md`: "A self-hosted, Cloudflare Workers-compatible platform for
  a single machine"). It runs a fork of workerd.
* The README table lists what it supports: Workers, KV, R2, D1, Durable
  Objects, Queues, Cron, Workflows, the `/client/v4` API. It lists no
  Tunnel product.
* A search of the whole tree for `tunnelrpc`, `registerConnection`,
  `trycloudflare`, `cloudflared` and `7844` finds no source file that uses
  them. The few hits are certificate serial numbers in
  `test/fixtures/python-runtime/upload.multipart` and the word
  `cloudflared` in no code path.
* `packages/runtime/src/sockets/tunnel.ts` is a private hop between its own
  processes (its header: "carried only across open-compute's private tunnel
  hops"). It is not the Cloudflare Tunnel protocol.
* The `capnp` files (`packages/runtime/config.capnp`,
  `test/runtime/fixtures/*.capnp`) are workerd configuration. They are not
  `tunnelrpc.capnp`.
* `test/conformance/differential/` calls the real Cloudflare API to compare
  behavior. It does not stand in for Cloudflare.

Result: open-compute is a Workers-like runtime only. It has no edge
protocol, no registration RPC, and no quick-tunnel API. It is not usable
for this need.

## 4. Other candidates

### 4.0 flolep2607/cctop, crate `crates/tunnel` (best partial)

Source: https://github.com/flolep2607/cctop at commit
a29a1bd27981bb9bec3fd1374b516e1c7fbb5750 (pushed 2026-11-09). Crate
`cctop-tunnel`, `license = "MIT OR Apache-2.0"` (`crates/tunnel/Cargo.toml`).
Rust. The crate is a native Cloudflare quick-tunnel and named-tunnel client.
Files read through the forge service, path
`crates/tunnel/src/cloudflare/fake_edge/`:

* `mod.rs` (header comment): "A fake argotunnel edge, for tests that must
  not reach Cloudflare." It is a QUIC server on loopback (quinn), with a
  self-signed certificate for `quic.cftunnel.com` and `h2.cftunnel.com`
  (`cert.pem`, `key.pem` in the same directory). It:
  * answers `registerConnection` and records the `Registration`
    (account tag, tunnel id, secret, connection index) with `Answer::Accept`
    or `Answer::Refuse` (the non-retryable error of a deleted tunnel);
  * pushes configuration (`push_config`, the `ConfigurationManager` RPC);
  * opens a request stream for a visitor request (`get(conn, host, path)`),
    as the edge does.
* `http2.rs`: `FakeH2Edge`, the HTTP/2 twin over TLS on TCP, with `fetch`,
  `fetch_with_body` and `push_config`.
* `Seam` replaces edge discovery and the trust roots. It exists only under
  `cfg(test)` (`cloudflare/mod.rs`: `#[cfg(test)] mod fake_edge;`). The
  authors did this on purpose so that no release build can trust a fake
  edge.
* The quick-tunnel API is faked in `api.rs` (`tests::fake_api`, a
  `TcpListener` on loopback that returns a canned body). The test
  `a_quick_tunnel_takes_its_credentials_from_the_api_and_any_host` runs
  the whole quick path against both fakes.

Limits: the code is `pub(crate)` and `cfg(test)` and depends on the crate's
types (`EdgeAddr`, `Routes`). It is not a binary. It has no Host-based
router from a public listener. It does not do datagram sessions. Its
protocol knowledge is only as current as cctop (see its `CHANGELOG.md`).
Use: port the module into a podnet test crate. Keep the MIT or Apache-2.0
notice when you copy text (the 0BSD rule in `CLAUDE.md` covers other
projects only).

### 4.1 cloudflare/cloudflared (upstream)

Source: https://github.com/cloudflare/cloudflared at commit
b0297f0b24c60a4cef743b7b878e2da3798cb839 (2026-11-12, `RELEASE_NOTES`
2026.11.0). Apache-2.0. Go.

* It has no edge server and no mock-edge binary. `ls` of the repo root shows
  the client packages (`connection/`, `tunnelrpc/`, `quic/`, `supervisor/`,
  `edgediscovery/`, `quicktunnelauth/`).
* It holds the server-side half of the RPC as a Go type.
  `tunnelrpc/registration_server.go` has `RegistrationServer.Serve`, which
  serves the capnp RPC on a stream. `tunnelrpc/pogs/registration_server.go`
  has `RegistrationServer_ServerToClient` and `RegisterConnection`
  server-side handling. The edge uses this code in production. The same
  code is a base for a mock edge in Go.
* Its tests use small in-process fake edges:
  * `connection/quic_connection_test.go` (`newTestQUICConnection`,
    `quicServer`) opens a local QUIC listener, accepts the connection from
    the tunnel, opens a stream, writes a connect request and checks the
    answer.
  * `connection/http2_test.go` (`TestServeControlStream`) makes an HTTP/2
    client connection to the tunnel and sends the control-stream upgrade
    request (`InternalUpgradeHeader`). `mockNamedTunnelRPCClient` and
    `mockRPCClientFactory` replace the RPC.
  * `tunnelrpc/pogs/registration_server_test.go` runs a capnp
    `RegistrationServer` and calls `RegisterConnection` against it.
  These are test fixtures in `_test.go` files. They are not a runnable
  server and they do not route visitor requests.
* `component-tests/test_quicktunnels.py` (Python, pytest) runs real
  cloudflared against the real edge. It needs the Internet.
* The client has a hidden `--edge` flag (`cmd/cloudflared/flags/flags.go:94`,
  `cmd/cloudflared/tunnel/cmd.go:669`) and a hidden `--quick-service` flag
  (`cmd/cloudflared/tunnel/cmd.go:867`). A stand-in can be pointed at.
  The `POST {quick-service}/tunnel` request is in
  `cmd/cloudflared/tunnel/quick_tunnel.go:145`.
* The edge TLS server names are `h2.cftunnel.com` and `quic.cftunnel.com`
  (`connection/protocol.go:11-15`). The SRV name is
  `_v2-origintunneld._tcp.argotunnel.com`
  (`edgediscovery/allregions/discovery.go`).
* The schema is `tunnelrpc/proto/tunnelrpc.capnp`. `RegistrationServer` has id
  `0xf71695ec7fe85497`. `TunnelAuth`, `ConnectionOptions`,
  `ConnectionDetails` and `SessionManager` are in the same file.
* `quicktunnelauth/` (`assertion.go`) is the new "protected quick tunnel"
  logic. It verifies a JWT from `https://login.trycloudflare.com`. It is the
  client side only.

### 4.2 cloudflare/workerd

Source: https://github.com/cloudflare/workerd at commit
9d5481ab0999210790528b805b3114b2e8f4f27b (2026-11-12). Apache-2.0.
C++, Rust.

* A search of file names for `tunnel`, `argo`, `cloudflared` finds only
  `src/rust/jsg/tunnel.rs` and `src/rust/jsg-test/tests/tunneled_exception.rs`.
  These are about exceptions that cross the JS/C++ boundary. They are not
  network tunnels.
* No source file in the tree speaks `tunnelrpc`. workerd has no tunnel edge.

### 4.3 Wrangler and Miniflare

* `wrangler dev --tunnel` and `wrangler tunnel quick-start` download and
  run the real cloudflared client. They call the real
  `api.trycloudflare.com` and the real edge. Source:
  https://developers.cloudflare.com/workers/development-testing/local-dev-tunnels/
  and https://developers.cloudflare.com/changelog/2026-05-18-local-dev-tunnels/
* I found no tunnel feature in Miniflare. Miniflare does not stand in for
  the edge.

### 4.4 Rust client libraries (no server)

All hold the capnp schema and a client. None runs an edge.

| Project | License | Last commit read | Notes |
|---|---|---|---|
| https://github.com/apersomany/libcfd (fe57dd36) | none found in the tree | 2026-10-10 | Library port of cloudflared. Quick and named tunnels, QUIC and HTTP/2, v0.3.0. `crates/libcfd-rpc/tests/common/mod.rs` has a scripted "mock edge" that answers framed RPC messages (`serve_mock`, `serve_mock_register_error`, and others) on an in-memory stream. It tests the RPC client only. `research/*.md` documents the protocol in detail. |
| https://github.com/networks-rs/cloudflared-rs (1bbd9517) | Apache-2.0 | 2026-08-14 | Rewrite of the whole cloudflared CLI. `crates/cloudflared-tunnelrpc/src/capnp_registration.rs` has a `RecordingRegistrationServer` in a unit test. Client first. |
| https://github.com/lordmacu/cloudflare-quick-tunnel-rs (0473d5da) | Apache-2.0 (`LICENSE-APACHE`) | 2026-05-14 | Quick-tunnel client. Its tests (`tests/live_*.rs`) need the real Cloudflare (`CFQT_LIVE_TESTS=1`). |
| https://github.com/devsnek/cf-tunnel-rs (6939d685) | MIT | 2023-12-14 | Old client. `src/rpc.rs` is the registration client. |
| https://github.com/ratazzi/coulson (c3141441) | MIT | 2026-10-19 | App with an embedded tunnel client (`crates/coulson/src/tunnel/rpc.rs`). |
| https://github.com/hop-os/hop (4571448d) | Commons Clause (not open source) | 2026-11-10 | `apps/cloudflared-lean/src/register.rs`. Client. |
| https://github.com/PodUtils/podssh | 0BSD (project rule) | n/a | `crates/podssh-relay/src/quick/register.rs`. Client. |

More clients that the code search found (not read in full):
https://github.com/max-lt/cf-tunnel-rs (Apache-2.0, 2026-11-02, has a
`PROTOCOL.md`), https://github.com/SagerNet/sing-cloudflared (Go,
reimplementation for sing-box, license NOASSERTION, 2026-11-09),
https://github.com/frxbg/esp-cf-tunnel (MIT, C for ESP-IDF, has
`tools/rpc_reference`), https://github.com/Ujang360/cfdrs (Apache-2.0),
https://github.com/ztwww2222/cftun (Go, no license, `server/` directory is a
modified client). Forks of cloudflared (for example khulnasoft/netscale,
mewisme/cf-tunnel) copy `tunnelrpc/`. A forge-service code search for
`registerConnection tunnelrpc` (159 hits), `RegistrationServer tunnelrpc
server` (130 hits), `h2.cftunnel.com` (138) and `quic.cftunnel.com` (125)
shows only clients, forks, documents and block lists, and the cctop
`fake_edge` module. It shows no edge server project.

### 4.5 Look-alike projects (not compatible)

These use their own client and server protocol. cloudflared cannot talk to
them. They cannot run podnet's Cloudflare adapter.

| Project | Notes |
|---|---|
| frp (https://github.com/fatedier/frp) | Own protocol. Fits podnet's own relay tests, not the Cloudflare adapter. |
| rathole, bore, chisel | Own protocol. Same limit. |
| Pangolin and its agent Newt (AGPL-3.0) | Own agent. Not cloudflared. https://blog.elest.io/pangolin-free-open-source-alternative-to-cloudflare-tunnels/ |
| Octelium | Zero-trust platform with its own client. https://octelium.com/solutions/cloudflare-tunnel-alternative-open-source |
| TunnelFlow | Management UI for the real Cloudflare Tunnel API. No edge. |
| unjs/untun, akazwz/hostc | Wrappers of cloudflared, or own service. No edge protocol. |

Web searches for "tunnelrpc server", "origintunneld", "self-hosted
cloudflared edge" and "fake edge" found no project that implements the
edge. Search results list only the client and the look-alike projects
above.

## 5. Comparison

| Candidate | Edge legs (H2, QUIC, RPC, visitor routing) | Quick-tunnel API | Reuse for a mock edge |
|---|---|---|---|
| open-compute | none | none | none |
| workerd | none | none | none |
| Wrangler, Miniflare | none (run real cloudflared) | none | none |
| cctop `fake_edge` (Rust, MIT OR Apache-2.0) | yes, test-only: QUIC and H2 listeners, registration, config push, visitor streams; no Host router | fake API in tests (canned body) | best base; port into a podnet test crate |
| cloudflared tests (Go) | partial, in `_test.go`: QUIC listener, H2 client, capnp server | none | Go types and test helpers; Apache-2.0 |
| libcfd mock edge (Rust) | RPC framing only, in memory | none | message-level mock; license unknown |
| cloudflared-rs, quick-tunnel-rs, cf-tunnel-rs, coulson | client only | client only | schema and capnp code to copy |
| frp, rathole, bore, chisel, Pangolin | not compatible | none | none |

## 6. What podnet must build

podnet already has the client. It can build a test edge from its own pieces.
Evidence of existing parts:

* `src/cloudflare/rpc.rs:519` has `fake_edge` for an in-memory stream. It
  answers the bootstrap and `registerConnection`. It is a unit-test fixture
  and has no listener.
* `src/tunnel.rs:555` and `:785`: `PODNET_EDGE` replaces edge discovery with
  a `host:port` list.
* `--service-url` replaces the provision endpoint (`docs/protocol.md`
  section 1).
* `src/cloudflare/mod.rs` embeds the Cloudflare edge roots. I found no
  override for the roots.

Start from the cctop `fake_edge` module (section 4.0) and the existing
`fake_edge` of `src/cloudflare/rpc.rs`. Work items for a test edge
(`podnet-fakeedge`, a dev-only binary or a test crate):

1. Provision API. `POST /tunnel` returns
   `{"success":true,"result":{"id","name","hostname","account_tag","secret"},"errors":[]}`
   (shape in `docs/protocol.md` section 1; cloudflared reads it in
   `quick_tunnel.go`). Add a `429` mode with `Retry-After` to test the
   rate-limit path on purpose. About 50 lines.
2. TLS. The edge needs a certificate for `h2.cftunnel.com` and
   `quic.cftunnel.com` from a test CA. podnet must trust that CA. Section 9
   shows that `SSL_CERT_FILE` already does this.
3. HTTP/2 leg. Listen on TCP. Accept TLS with no ALPN. Act as an HTTP/2
   client on the accepted connection. Send the control-stream request, run
   the capnp RPC (answer `registerConnection` with a `ConnectionDetails`
   that has a UUID and a location name), then send visitor requests as
   further streams. This is the reverse of cloudflared's own H2 test client
   (`connection/http2_test.go`).
4. QUIC leg. Listen on UDP with ALPN `argotunnel`. Accept the first
   client-initiated stream for the capnp RPC (no signature bytes, see
   `libcfd/research/quic.md:58` at libcfd fe57dd36). Open one server-initiated
   stream for each visitor request with the connect-request framing of
   `tunnelrpc/quic/request_server_stream.go`. `quic_connection_test.go`
   (`quicServer`) shows the byte sequence.
5. Visitor routing. Run an HTTP listener that maps `Host:
   <hostname>.trycloudflare.com` to a registered connection and forwards the
   request down a stream. Map the response back. Needed for end-to-end
   checks; the registration alone does not need it.
6. Optional later: datagram sessions (UDP/ICMP), the `SessionManager` and
   `ConfigurationManager` RPCs, the SRV discovery answer.

Do not copy a client crate whose license is unknown (libcfd) or
Commons Clause (hop). The cloudflared Go code and schema are Apache-2.0 and
podnet already uses its schema under that license
(`src/cloudflare/cloudflared-pin.txt`).

## 7. Limits of the stand-in

A local edge tests podnet against a copy of the protocol that podnet's
authors wrote. It cannot show that the real edge accepts a change. Keep a
small number of live runs (one per release, not one per job) to catch
drift. The real edge also adds behaviors that a mock lacks: header rewrite,
HTTP/2 flow-control limits, the 200 in-flight cap, and protected quick
tunnels (`quicktunnelauth/`).

## 8. Search method

* Read: open-compute, cloudflared, workerd, libcfd, cloudflared-rs,
  cf-tunnel-rs, cloudflare-quick-tunnel-rs, coulson, hop. Each was cloned
  with `--depth 1` into `/home/user/research-cfedge` (deleted after this
  note).
* After the owner rule (use the pkgforge services first), code search and
  file reads went through `https://forge.web.pkgforge.dev/github/...`. The
  first pass used WebSearch and `git clone --depth 1` (before the rule
  reached this session). The cctop finding comes from the forge service.
* GitHub repository search for: argotunnel, tunnelrpc, cloudflared server
  or edge, trycloudflare, origintunneld, mock edge. GitHub code search for
  `registerConnection tunnelrpc`.
* Web search (several queries) for self-hosted Cloudflare Tunnel servers,
  edge reimplementations, workerd tunnel code and wrangler/miniflare tunnel
  features.
* Not checked: private repositories, and projects with no GitHub presence.
  A hidden project may exist. The search found none.

## 9. What podnet built (T-316, 2026-11-12)

The stand-in is test code in `tests/standin/`. The test file is
`tests/quick_tunnel_standin.rs`. Run it with
`cargo test --test quick_tunnel_standin`.

| Part | File | What it does |
|---|---|---|
| Provision API | `tests/standin/api.rs` | `POST /tunnel` on the loopback. The body has the shape of the real service. A script of answers gives a 429 with `Retry-After`, or another status. It keeps each tunnel that it issues. |
| Edge, HTTP/2 | `tests/standin/edge.rs` | TLS on TCP with no ALPN, the edge as the HTTP/2 client, the control stream, `registerConnection` (accept or refuse), `unregisterConnection`, one stream for each visitor request. It checks the account tag and the secret against the tunnel that the API issued. |
| Front | `tests/standin/front.rs` | An HTTP/1.1 server on the loopback. It maps `Host` to the tunnel. It answers 530 with error 1033 for an unknown host. |
| CA | `tests/standin/ca.rs` | A test CA made at run time (rcgen). |
| Proxy | `tests/standin/proxy.rs` | A CONNECT proxy that keeps the authorities. |

Hooks in podnet: none. The service is `--service-url`. The edge is
`PODNET_EDGE`. The test CA is `SSL_CERT_FILE`: podnet uses that file as its
only platform roots and adds the roots of the edge
(`crates/podnet-core/src/trust.rs`). The code of `src/cloudflare/` is not
changed. A test shows that the edge certificate is refused without the file.

The code is written from the client of podnet and the schema of
cloudflared. No code of `flolep2607/cctop` is copied.

Not built: the QUIC leg (UDP, ALPN `argotunnel`), datagram sessions, the
configuration push, the verify of the public URL (the stand-in name is not in
the DNS, and the verify uses port 443), and the WARP and relay legs. `live`
keeps one real quick tunnel for each leg.
