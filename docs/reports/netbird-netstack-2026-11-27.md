# NetBird in netstack mode on a proxy-only host

> Measurement of T-247. Date: 2026-11-27. Host: Ubuntu 24.04 x86_64, non-root user, Docker 29.10.0.
> Client: NetBird `v0.84.0` (`linux/amd64`). Server: self-hosted `ghcr.io/netbirdio/netbird-server:latest`
> (`v0.84.0`, combined management, signal and WebSocket relay, STUN disabled) on a private Docker bridge
> (`172.29.0.10:18080`, relay `rel://172.29.0.10:18080`). podnet: `v0.4.0-beta.21` (`af1f172`).

## 1. Result

| Question | Measured answer |
| --- | --- |
| Does `netbird up -F` in netstack mode run with no root and no TUN device? | Yes. Both peers joined with `NB_USE_NETSTACK_MODE=true` as an unprivileged user (`uid 1001`, no `CAP_NET_ADMIN`, no `/dev/net/tun`) and opened a SOCKS5 listener on `127.0.0.1:NB_SOCKS5_LISTENER_PORT`. |
| Does `netbird up -F` pass an HTTP `CONNECT` proxy through `HTTP_PROXY` / `HTTPS_PROXY`? | **No.** In an isolated network namespace with only `lo` and no external route, `netbird up -F` sent zero requests to the proxy. Its gRPC client dials with `nbnet.NewDialer().DialContext`, which ignores `HTTP_PROXY` and `HTTPS_PROXY` and fails with `connect: network is unreachable`. |
| Does `podnet pod -x http://PROXY` carry `netbird up -F` on a proxy-only host? | **Yes.** Inside `podnet pod -x http://127.0.0.1:18134` (a `CONNECT`-only proxy that allowed only `172.29.0.10:18080` and denied all other targets with 403), `netbird` registered over gRPC, opened the WebSocket relay (`rel://172.29.0.10:18080`, racing QUIC which `podnet pod` dropped because UDP needs SOCKS5, and winning via `ws`), and established a relayed WireGuard session to Peer A. |
| Does `podnet`'s `netbird:[PORT]` road carry TCP to a peer? | **Yes.** `podnet net doctor` reported `ok`, `podnet forward --network netbird:11082` read Peer A's TCP banner 3/3 times in 18.4 to 19.7 ms, and `podnet --egress netbird:11082 pod` read it in 30.8 ms. |
| Does NetBird netstack resolve peer names? | **No.** Sending `peer-a.netbird.selfhosted:22022` directly to NetBird's SOCKS5 listener (`curl --socks5-hostname`) timed out after 5 s. `podnet`'s `netbird:` road refused the name at once with one line and no dial. |

## 2. Setup

1. **Control plane and relay.** One self-hosted `netbird-server` container on an isolated Docker bridge
   (`172.29.0.10:18080`, `stunPorts: []`, `NB_SETUP_PAT_ENABLED=true`). Initial setup via `POST /api/setup`
   created a local admin user and PAT; `POST /api/setup-keys` minted a reusable setup key.
2. **Peer A (remote peer, no root).** Plain non-root user on the host running:

```sh
NB_USE_NETSTACK_MODE=true NB_SOCKS5_LISTENER_PORT=11081 \
  netbird up -F --management-url http://172.29.0.10:18080 --setup-key "$KEY" \
  --hostname peer-a --allow-server-ssh --disable-ssh-auth \
  --disable-dns --disable-firewall
```

Peer A received `100.120.28.90/16`, connected to the WebSocket relay `rel://172.29.0.10:18080`, and (after
enabling `ssh_enabled: true` via `PUT /api/peers/{id}`) listened inside its netstack on `100.120.28.90:22022`.

3. **Peer B (proxy-only host, no root).** Tested in two configurations with zero direct IP route and zero UDP
   egress to `172.29.0.10:18080`:
   - **Bare `HTTP_PROXY` / `HTTPS_PROXY`:** `unshare -Urn` (only `lo`, empty routing table, direct `curl` to
     `172.29.0.10:18080` failed in 0 ms). A TCP-to-Unix bridge forwarded `127.0.0.1:18128` to an HTTP proxy
     on the host (`curl -x http://127.0.0.1:18128 http://172.29.0.10:18080/api/instance` returned
     `{"setup_required":false}`).
   - **Wrapped in `podnet pod`:** `podnet pod -x http://127.0.0.1:18134 -- ...` where `127.0.0.1:18134` is a
     `CONNECT`-only HTTP proxy that accepts `CONNECT 172.29.0.10:18080` and returns `403 Forbidden` for every
     other host or non-`CONNECT` method.

## 3. Observations

### 3.1 Bare `HTTP_PROXY` / `HTTPS_PROXY` fails

With `HTTP_PROXY`, `HTTPS_PROXY`, `http_proxy` and `https_proxy` all set to `http://127.0.0.1:18128` and
`NO_PROXY=""`, `netbird up -F` logged:

```text
grpc: addrConn.createTransport failed to connect to {Addr: "172.29.0.10:18080", ...}.
Err: connection error: desc = "transport: Error while dialing: nbnet.NewDialer().DialContext: dial tcp 172.29.0.10:18080: connect: network is unreachable"
```

After 30 s, `netbird up -F` exited with `foreground login failed: failed to create auth client: create connection: dial context: context deadline exceeded`. The proxy log showed zero requests from `netbird`.

### 3.2 `podnet pod -x http://PROXY` succeeds end to end

Inside `podnet pod -x http://127.0.0.1:18134`:

1. The `CONNECT`-only proxy log recorded `6 REQ: CONNECT 172.29.0.10:18080 HTTP/1.1` (management gRPC, signal
   gRPC and WebSocket relay). Cloud metadata probes (`169.254.169.254:80` and `100.100.100.200:80`) were
   denied with 403 and did not block startup.
2. `podnet pod` dropped NetBird's UDP relay race (`udp 172.29.0.10:18080: UDP needs a SOCKS5 proxy`), and
   NetBird's `race_dialer` completed immediately over WebSocket (`successfully dialed via: ws`).
3. `NB_SOCKS5_LISTENER_PORT=11082 podnet net doctor` reported:

```text
NetBird (SOCKS5 listener)     ok       127.0.0.1:11082 answers the SOCKS5 greeting (TCP, peers by IP address)
```

4. Peer B (`100.120.49.63`) dialed Peer A (`100.120.28.90:22022`) over the relayed WireGuard tunnel:
   - `podnet forward --network netbird:11082 -L 127.0.0.1:19822=100.120.28.90:22022`: read
     `SSH-2.0-NetBird-SSH-Server-0.84.0\r\n` on 3 of 3 tries (19.6 ms, 18.4 ms, 19.7 ms).
   - `podnet --egress netbird:11082 pod -- ...`: read `SSH-2.0-NetBird-SSH-Server-0.84.0\r\n` in 30.8 ms.
   - Peer A logged `SSH connection from NetBird peer 100.120.49.63:<port> allowed` for all four connections.
5. Name targets:
   - `podnet forward --network netbird:11082 -L 127.0.0.1:19889=peer-a.netbird.selfhosted:18888` refused with:
     `peer-a.netbird.selfhosted: NetBird netstack mode has no DNS and takes the IP address of a peer only`.
   - `curl --socks5-hostname 127.0.0.1:11082 http://peer-a.netbird.selfhosted:22022/` timed out after 5 s.

### 3.3 Two limits found during the run

1. **Lazy connection warmup on first dial.** NetBird `v0.84.0` enables lazy peer connections by default. The
   first TCP dial after startup activates the peer over Signal and brings up the relayed WireGuard session
   (~1 s in this local test); a client that connects before the WireGuard handshake finishes can get one
   reset and succeed on the next try.
2. **Syntax defect in `podnet pod` for `netbird:PORT`.** `podnet --egress netbird:11082 pod -- CMD` expanded
   `netbird:11082` to `-x 127.0.0.1:11082` and succeeded, while `podnet pod -x netbird:11082` (shown in
   `docs/networks.md`) and `podnet pod --egress netbird:11082` (shown in GitHub #89) passed `netbird:11082`
   unexpanded as a proxy hostname because `src/cli/way.rs` stopped scanning `--egress` at the word `pod`.
   Fixed alongside this measurement.
