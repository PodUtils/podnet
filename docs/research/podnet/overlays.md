# NetBird, Firezone and ZeroTier as networks of podnet

> Research of T-118. Date of reading: 2026-11-11. Nothing was run: no account, no daemon, no
> network measurement. Each fact has a mark.

Marks:

* **read**: the fact is in a document. The source is a URL, read on 2026-11-11.
* **inferred**: the fact follows from read facts or from the code of podnet. It was not run.

## 0. Result

| Overlay | Client with no root and no TUN | Network of podnet | Next entry |
| --- | --- | --- | --- |
| NetBird | Yes: netstack mode with a SOCKS5 listener (TCP; peers by IP address only) | `socks5://127.0.0.1:PORT` today; a sidecar spec is possible | Open E1 and E2 (section 5) |
| Firezone | No path found: the headless client opens `/dev/net/tun`, which needs `CAP_NET_ADMIN` | None | None. Reasons in section 3.5 |
| ZeroTier | Only as a library (libzt, BSL 1.1); the daemon needs a TUN | None now | None. Reasons in section 4.5 |

## 1. The target (what a network of podnet must fit)

All facts of this section are **read** from the repository.

* `docs/target-environment.md`: the hosts have no `CAP_NET_ADMIN`. Egress can be one HTTP `CONNECT`
  proxy. `CONNECT` to an IP address can be refused. UDP can be blocked. `listen(2)` on `AF_INET` and
  `AF_UNIX` is allowed on the measured sandbox. Other hosts forbid TCP binds.
* `src/networks.rs`: a spec names a network. The rows are `direct`, `proxy`, `relay` and `tailscale`.
  The `proxy` row takes `socks5://`, `socks5h://`, `http://`, `SCHEME+unix:/PATH` and
  `SCHEME+exec:COMMAND`. The `tailscale` row takes `tailscale:/PATH/tailscaled.sock`. Both give
  `Caps::EGRESS_TCP` only.
* `crates/podnet-netns/src/tailscale.rs`: for Tailscale, the door speaks to the LocalAPI of
  `tailscaled --tun=userspace-networking` over a unix socket. The reason: the SOCKS5 listener of the
  daemon is an `AF_INET` listener, which a restricted host can forbid.

Inferred: a daemon that gives an `AF_INET` SOCKS5 listener on `127.0.0.1` fits the `proxy` row with no
code. A daemon that gives only a library or a TUN does not.

## 2. NetBird

### 2.1 Client with no root and no TUN

* **read**: netstack mode runs the WireGuard stack in userspace with the gVisor netstack package. It
  starts with `NB_USE_NETSTACK_MODE=true`. `NB_SOCKS5_LISTENER_PORT` sets the SOCKS5 port (default 1080).
  The local example is `export NB_USE_NETSTACK_MODE=true; export NB_SOCKS5_LISTENER_PORT=30000;
  netbird up -F`. Source: <https://docs.netbird.io/use-cases/cloud/netbird-on-faas>, 2026-11-11.
* **read**: the SOCKS5 listener binds `127.0.0.1` by default. `NB_SOCKS5_LISTENER_ADDRESS=0.0.0.0` opens
  it to other containers. The listener has no authentication. Same source.
* **read**: the DNS feature is not supported in this mode. Peers are reached by IP address only. Same
  source.
* **read**: the Docker example passes the key with `NB_SETUP_KEY`. Same source.
* **read**: a user reports that UDP started by a local application does not enter the tunnel in
  netstack mode (RTSP over UDP fails, TCP works). The maintainers gave no technical answer and moved it
  to a discussion. Source: <https://github.com/netbirdio/netbird/issues/4929>, 2026-11-11.
  Inferred: UDP egress is not available through the listener. The network gives `EGRESS_TCP` only.
* **read**: the Go package documents a variable `NB_NETSTACK_SKIP_PROXY` (its meaning was not read).
  Source: pkg.go.dev, package `github.com/netbirdio/netbird/client/iface/netstack` (search result,
  2026-11-11).
* Not verified: whether `netbird up -F` needs root, and what the listener does with a name target
  (`socks5h`). Both need a run. Inferred: no interface is made, so no root is needed.

### 2.2 Ports and the account

* **read** (NetBird Cloud, outbound): management `api.netbird.io` TCP 443. Signal `signal.netbird.io`
  TCP 443. Relay over WebSocket `relay.netbird.io` and `*.relay.netbird.io` TCP 443 (clients 0.36.0 and
  later). Relay over QUIC UDP 443 (optional; the WebSocket is the fallback). STUN UDP 80, 443, 3478 and
  5555. WireGuard UDP 51820 is the default port of a peer and is not needed outbound. No inbound port is
  needed. Source: <https://docs.netbird.io/about-netbird/ports-and-firewalls>, 2026-11-11.
* Inferred: with UDP blocked, the client reaches its peers only through the WebSocket relay on TCP 443.
  The control plane and the relay then use TCP 443 only. Whether the gRPC and WebSocket legs pass an
  HTTP `CONNECT` proxy (`HTTPS_PROXY`) is **not found** in the documents. It needs a run on a proxy-only
  host.
* **read**: the account is a NetBird Cloud account or a self-hosted one. A client joins with a setup key
  (`NB_SETUP_KEY`). Source: same as 2.1. Single sign-on is also possible (inferred; not read on a page).

### 2.3 License and control server

* **read**: the repository is BSD-3-Clause, except `management/`, `signal/` and `relay/`, which are
  AGPLv3. Source: <https://github.com/netbirdio/netbird>, 2026-11-11. Inferred: a sidecar can run next
  to podnet with no copyleft duty on podnet, because podnet only talks to it over a socket.
* **read**: the control server can be self-hosted. The quickstart makes a Docker Compose set on one VM
  (1 CPU, 2 GB, a public domain): a combined server (management, signal, relay, STUN), a dashboard,
  Traefik with Let's Encrypt, and an embedded Dex. Public ports: TCP 443, UDP 3478. TCP 80 must be free.
  Source: <https://docs.netbird.io/selfhosted/selfhosted-quickstart>, 2026-11-11.
* **read** (community, not the official page): older layouts list management gRPC TCP 33073, API TCP
  33080 and signal gRPC TCP 10000. Source: guides in a search result, 2026-11-11. Unconfirmed.
* **read**: the protocol is WireGuard between peers, with ICE and STUN for NAT traversal, a signal
  service for the exchange of candidates, and a relay as fallback. Source: ports page and repository
  page above.

### 2.4 Network spec for podnet

Inferred for both forms. Nothing was run.

1. **No code.** The user runs `NB_USE_NETSTACK_MODE=true NB_SOCKS5_LISTENER_PORT=1080 NB_SETUP_KEY=...
   netbird up -F` and gives `-x socks5://127.0.0.1:1080` (the `proxy` row). Peers are addressed by their
   NetBird IP address. Name targets fail (no DNS).
2. **A spec `netbird:[PORT]`.** It is sugar for `socks5://127.0.0.1:PORT` (default 1080). It gives
   `Caps::EGRESS_TCP` only. It refuses a name target with one clear line. It adds a `podnet net doctor`
   row (the port is open; the SOCKS5 greeting answers). It needs no new dependency. It does not start
   the daemon. A form that starts `netbird up -F` as a child (`SCHEME+exec:` exists in the `proxy` row)
   is a later choice.

Limits to state in the docs: TCP only; IP targets only; the listener has no authentication, so keep it
on `127.0.0.1`.

### 2.5 Verdict

Pass. Open the entries E1 and E2 (section 5). A live proof needs an account and a run. Its entry is E3.
E3 stays blocked until a key exists.

## 3. Firezone

### 3.1 Client with no root and no TUN

* **read**: the headless client for Linux: "If you're running as an unprivileged user, you'll need the
  `CAP_NET_ADMIN` capability to open `/dev/net/tun`." The binary must run as root to change the DNS.
  With no DNS it needs only `CAP_NET_ADMIN`. Source:
  <https://github.com/firezone/firezone/blob/main/rust/headless-client/README.md>, 2026-11-11.
* **read**: the CLI reference lists the flags and the variables (`FIREZONE_TOKEN`, `FIREZONE_API_URL`,
  `--dns-control`, and others). It names no proxy mode, no SOCKS5 and no userspace mode. Source:
  <https://www.firezone.dev/kb/reference/cli/headless-linux>, 2026-11-11. The page says nothing about a
  TUN. A page that does not name a mode is not proof that the mode does not exist.
* Inferred: no path for a client with no TUN is in the documents. A host with no `CAP_NET_ADMIN` cannot
  run it. A pod of podnet with a user namespace holds `CAP_NET_ADMIN` inside its own network namespace
  (the netns backend of `podnet-netns`). The client could open a TUN there. That needs a run. It is not
  a netstack mode, and it is the concern of the pod backends, not of a network spec.

### 3.2 Ports and the account

* **read**: portal: outbound HTTPS and WebSocket on 443 (`app.firezone.dev`, `api.firezone.dev`,
  `flow-api.firezone.dev`). Relay: STUN on UDP 3478; TURN data on UDP 49152-65535. Gateway and client:
  WireGuard over UDP, direct or through a relay. No inbound port in normal use. Source:
  <https://www.firezone.dev/kb/reference/ports>, 2026-11-11.
* **read**: the relay listens on 3478 and the port is not configurable. Source: the README of
  `rust/relay` in the repository (search result, 2026-11-11).
* Inferred: the data path is UDP only (WireGuard direct, or TURN). With UDP blocked, no data path is in
  the documents. This matches the premise of the entry (TURN on UDP).
* **read**: a token is a service-account token (`FIREZONE_TOKEN`) or a user token from `sign-in`.
  Source: CLI reference above. Both need an account in a portal.

### 3.3 License and control server

* **read**: most of the repository is Apache-2.0. `elixir/` (the portal, the control plane) is under the
  Elastic License 2.0. Source: <https://github.com/firezone/firezone>, 2026-11-11.
* **read**: the licenses allow self-hosting. Production self-hosting is not supported. It is for
  development or evaluation. Published clients are guaranteed only against the managed service. Same
  source.
* **read**: the parts are the portal (control plane), the gateway (WireGuard server near the resources,
  `rust/gateway`), the clients (`rust/`) and the relay (`rust/relay`, STUN and TURN). Same source.

### 3.4 Network spec for podnet

None. A spec needs a SOCKS5 listener or a local dial API, and the client has neither in the documents
(inferred).

### 3.5 Verdict

Do not open an entry. Reasons: (a) the client needs `CAP_NET_ADMIN` for the TUN (read); (b) the data
path is UDP (read; inferred for the case of blocked UDP); (c) the control plane is a managed service and
production self-hosting is not supported (read); (d) the portal license is ELv2 (read). Open an entry
only if Firezone ships a userspace client or a proxy mode. Check again when its release notes name one.

## 4. ZeroTier

### 4.1 Client with no root and no TUN

* **read**: ZeroTier Sockets (libzt) is an SDK for "encrypted P2P connections for your app or service",
  with C/C++, Rust, Python, C# and Java bindings. Source: <https://docs.zerotier.com/sockets> and
  <https://github.com/zerotier/libzt>, 2026-11-11. The pages do not state "no root" or "no TUN".
* **read** (search result, secondary): the SDK runs its own network stack in the application (lwIP), so
  it needs "no drivers, no root, and no host configuration". A user-space service build exists. Source:
  forks of libzt in search results, 2026-11-11. Not from the official page.
* **read**: the service `zerotier-one` in a container needs `--device=/dev/net/tun --cap-add=NET_ADMIN`
  (search result that quotes ZeroTier, 2026-11-11). Inferred: the stock daemon needs a TUN.
* **read**: a community tool, Zerotier-port-forward, forwards ports over libzt with no TUN
  (<https://www.github.com/neoncloud/Zerotier-port-forward>, search result, 2026-11-11). It shows that a
  forwarder over libzt works. It is not a SOCKS5 server.
* Not found: a SOCKS5 or HTTP proxy listener in `zerotier-one` or in libzt. Inferred: none exists. A
  SOCKS5 front door would be new code over libzt.
* **read**: the local service API listens on `http://localhost:9993`. Source:
  <https://docs.zerotier.com/self-hosting/network-controllers/>, 2026-11-11. It manages networks. A
  dial call is not found (inferred: none).

### 4.2 Ports and the account

* **read**: UDP 9993 is the port to allow on a local firewall. Source:
  <https://docs.zerotier.com/routertips/>, 2026-11-11 (search result).
* **read**: ZeroTier Inc. runs a TCP relay that the agent falls back to when UDP fails. It is slow. A
  self-hosted relay uses `tcpFallbackRelay` in `local.conf` (form `IP/port`, for example `1.2.3.4/443`)
  and `pylon`. Source: <https://docs.zerotier.com/relay/>, 2026-11-11. A forum post says the fallback
  can take minutes to start (anecdote, search result).
* Not verified: that the fallback relay passes an HTTP `CONNECT` proxy. Inferred: it does not, because
  the agent speaks its own protocol to the relay. It needs a run.
* **read**: a network is made in ZeroTier Central or on a self-hosted controller. A node joins a network
  by its network id, and the controller authorizes the node. Source: controller page above (in part).
  The Central side is inferred.

### 4.3 License and control server

* **read**: the agent code (`node/`, `osdep/`, `service/`) is MPL-2.0. Source:
  <https://github.com/zerotier/ZeroTierOne/blob/dev/LICENSE.txt> and
  <https://github.com/zerotier/ZeroTierOne/blob/dev/nonfree/LICENSE.md>, 2026-11-11.
* **read**: the controller and the related components in `nonfree/` have the "ZeroTier
  SOURCE-AVAILABLE LICENSE" 1.0 (2025). It allows Non-Commercial Use only. Commercial use (for-profit,
  government, non-profit, business production) needs a commercial license. Same source.
* **read**: libzt: the project page says BSL 1.1. It is free for internal business use, academic use and
  non-commercial use. Some commercial uses (closed-source apps and devices, SaaS controllers) need a
  license. Source: <https://github.com/zerotier/libzt>, 2026-11-11. Read the license file of the exact
  tag before any use.
* **read**: a controller can be self-hosted ("Controllers are nodes!"). State is `identity.secret` and
  `controller.d`. Source: controller page above. The planet (root servers) stays with ZeroTier Inc.
  unless a custom planet is built (inferred; not read).

### 4.4 Network spec for podnet

None now. A spec `zerotier:NETWORK_ID` needs a libzt adapter in podnet: an FFI C library with lwIP under
BSL 1.1, linked into a static musl binary. That conflicts with the 0BSD license of podnet (the binary
carries BSL code) and with the rule of no dependency for a protocol (CLAUDE.md, Project). Inferred.

### 4.5 Verdict

Do not open an adapter entry. Reasons: (a) UDP 9993 is the data path, and the TCP relay is slow and
hosted (read); (b) the only no-TUN path is libzt, under BSL 1.1 with FFI (read); (c) the controller
license is non-commercial only (read). Open an entry only if the owner accepts BSL code in a build, or
if a maintained SOCKS5 sidecar of libzt appears. A user can run their own libzt SOCKS5 tool and give it
as `-x socks5://...` (inferred; the `proxy` row).

## 5. Proposed entries (the lead opens them)

| Id | Title | Area | Size | Note |
| --- | --- | --- | --- | --- |
| E1 | `netbird:[PORT]` network: sugar for the netstack SOCKS5 listener, TCP and IP targets only | `src/networks.rs`, doctor | S | A registry row; one error line for a name target; a unit test of `parse`. No account. |
| E2 | Document NetBird netstack mode: the variables, the limits (no DNS, TCP only, no authentication) and a `podnet net doctor` row for the listener | `README.md`, `docs/target-environment.md` | XS | Marks as in section 2. |
| E3 | Prove NetBird netstack mode on a proxy-only host: does the client reach management and the WebSocket relay through `HTTPS_PROXY`; does `netbird up -F` run with no root | proof run | M | Needs a NetBird account or a self-hosted server (T-114 harness idea). Blocked until a key exists. |

No entry for Firezone or ZeroTier. Recheck triggers: a Firezone userspace or proxy client; a ZeroTier
SOCKS5 sidecar or a license change of libzt.

## 6. Commands that ran

* `grep -n "^# T-118:" TODO/*.md` (the entry is in `TODO/overlays.md`).
* Web reads on 2026-11-11 (the URLs above). No account was made. No daemon was started. No network
  measurement was made.
