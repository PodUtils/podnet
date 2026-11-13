# podnet

`podnet` is the Swiss army knife of networking for restricted hosts. It is one static binary for Linux
that brings its own network to a host that has no `CAP_NET_ADMIN`, no TUN device, no UDP, no DNS
resolver, or only a proxy to go out. With it you can publish a local service, reach out to the internet,
run a program through a proxy, join a tailnet, route flows over many ways out, and check which of these
work on the host. It needs no root and no other program.

* Publish: Cloudflare quick tunnels and named tunnels (the work of `cloudflared`), a file server, file
  transfer with a PIN, git hosting, and inbound TCP through a relay.
* Reach out: the Cloudflare WARP client with no root (`podnet warp`), proxy chains, Tailscale and NetBird
  with no TUN, DNS over HTTPS, and TCP and UDP forwards.
* Contain: `podnet pod` runs any program (also a static binary) with all its traffic sent through a
  proxy. `podnet net` gives a program IP addresses on a host that refuses `AF_INET` binds.
* Route: `podnet route` is a router of networks, groups, policies, health probes and doors, with a
  kernel mode.
* Check: `podnet doctor` tests the path to the edge, `podnet pod doctor` and `podnet net doctor` test the
  host, and `podnet bench` measures a way out.

The license is 0BSD. podnet is for Linux only. The binary has no run-time dependency on the C library.

## Install

Each [release](https://github.com/PodUtils/podnet/releases) has static binaries for `x86_64`,
`aarch64`, `armv7`, `riscv64gc`, `i686`, `arm` (ARMv6), `loongarch64` and `powerpc64le`. The install
script downloads the newest one for this machine, checks its SHA-256 (and the cosign signature when
`cosign` is present), runs it, and then puts it in place:

```sh
s=$(curl -fsSL https://raw.githubusercontent.com/PodUtils/podnet/main/scripts/install.sh) && sh -c "$s"
```

Do not use `curl URL | sh` in a script: a pipe has the exit status of `sh`, and `sh` exits 0 when `curl`
fails. There is also a container image, `ghcr.io/podutils/podnet:beta` (`FROM scratch`).

Verify a download by hand:

```sh
sha256sum -c --ignore-missing SHA256SUMS
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/vX.Y.Z \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
```

Update in place. `podnet update` checks the cosign certificate and signature over `SHA256SUMS`, the
SHA-256 of the binary, and that the new binary runs; `--rollback` puts the old binary back:

```sh
podnet update --check
podnet update
```

[docs/install.md](https://github.com/PodUtils/podnet/blob/main/docs/install.md) gives the install script
options, the release files, the provenance check, the notebook cell for hosts that lose their disk, the
image, `podnet update` and the rebuild of a release.

## Quick start

```sh
podnet doctor                                   # which ways out work from this host
podnet tunnel --url http://127.0.0.1:8080       # a public URL (quick tunnel, no account)
podnet serve ./public                           # serve a directory on a public URL
podnet share --file ./report.pdf --password     # send one file; the visitor needs a PIN
podnet warp up && eval "$(podnet warp env)"     # the internet through WARP, with no root
podnet pod -x socks5://127.0.0.1:1080 -- curl https://example.com   # any program through a proxy
podnet agent up --expose 3000                   # a sandbox: test, start WARP if needed, expose a port
podnet man                                      # the manual
```

## What podnet does

| Area | Commands | Function | Document |
| --- | --- | --- | --- |
| Tunnels | `tunnel`, `demo`, `qr` | Quick tunnels and named tunnels to a local origin (HTTP, unix socket, TCP). | [tunnels](https://github.com/PodUtils/podnet/blob/main/docs/tunnels.md) |
| Raw streams | `tunnel --forward`, `connect` | SSH, databases and unix sockets through a quick tunnel. | [tunnels](https://github.com/PodUtils/podnet/blob/main/docs/tunnels.md) |
| File server, transfer, git | `serve`, `share`, `git` | Serve a directory, receive or send files with a PIN, host git repositories. | [serve](https://github.com/PodUtils/podnet/blob/main/docs/serve.md) |
| Publish by one word | `expose` | Publish a local service by a tunnel, the relay or a virtual address. | [forward](https://github.com/PodUtils/podnet/blob/main/docs/forward.md) |
| WARP | `warp register`, `proxy`, `forward`, `run`, `up`, `env`, `exec`, `shell`, `nc`, `tun`, `export`, `scan` | The Cloudflare WARP client on TCP 443, with no root and no TUN. | [warp](https://github.com/PodUtils/podnet/blob/main/docs/warp.md) |
| Pods | `pod`, `pod doctor`, `pod --report`, `pod --backend netns\|netns-kernel\|seccomp` | Run a program with all its traffic sent through a proxy chain, with rules, audit and a JSON report. | [pods](https://github.com/PodUtils/podnet/blob/main/docs/pods.md) |
| Virtual networks | `net proxy`, `switch`, `shim`, `doctor`, `doctor --network` | IP addresses for programs on a host that refuses `AF_INET` binds. | [networks](https://github.com/PodUtils/podnet/blob/main/docs/networks.md) |
| Overlays | `net tailscale`, `net socksify`, `netbird:`, `wg:` | Tailscale, NetBird and WireGuard on a host with no TUN. | [networks](https://github.com/PodUtils/podnet/blob/main/docs/networks.md) |
| Router | `route --config`, `--check --to`, `--status`, `--control`, `--export` | Networks, groups, policies, health, doors, DNS door, kernel mode, WARP members (the egress check, renewal), and changes of a running router (`--control`: select, rotate, add or remove a door or a network). | [router](https://github.com/PodUtils/podnet/blob/main/docs/router.md) |
| Forwards | `forward -L`, `forward --dns`, `forward --network warp` | TCP and UDP forwards and a DNS door through a network. | [forward](https://github.com/PodUtils/podnet/blob/main/docs/forward.md) |
| Exit on stdio | `exit --stdio` | One SOCKS5 session (CONNECT, and UDP in frames) on stdin and stdout, with no listener, for a pipe such as ssh. | [proxies](https://github.com/PodUtils/podnet/blob/main/docs/proxies.md) |
| Relays | `relay pair`, `node`, `connect`, `--resume`, `ssh-relay` | Inbound TCP to a host with outbound HTTPS only, and sessions past the limits of the relay (`--resume`); an SSH forward relay. | [relay](https://github.com/PodUtils/podnet/blob/main/docs/relay.md) |
| Link | `link listen`, `dial`, `connect`, `status`, `mesh`, `close`, `carriers`, `peer:FP/SERVICE` | A link between two installs over a carrier (unix socket, memory; TCP with TLS, WebSocket, HTTP/2, QUIC, UDP with KCP, HTTP/1.1 polls, DNS, S3 and ICMP behind build features): each end proves its Ed25519 key, and an allow list (default deny) admits the peers. Services at a peer: `raw`, `socks5`, `http`. A local SOCKS5 door to a peer (`link dial --service socks5 --listen`, `--resolve-at peer\|local`). Names for live links, query keys, chains of `--egress` before the carrier, link groups, a mesh (`--announce`, `link mesh`) and a control socket. `forward -L PORT=peer:FP/SERVICE` reaches a service of a peer. A C interface (`include/podnet.h`, a shared and a static library from `crates/podnet-capi`). | [link](https://github.com/PodUtils/podnet/blob/main/docs/link.md) |
| Networks and `--egress` | `--egress`, `--proxy`, `--resolve` | One spec for each way out: direct, proxy URL, relay, `tailscale:`, `netbird:`, `wg:`, `devtunnel:` (a Dev Tunnels relay), `turn:` (UDP), `unix:`, `warp` and `warp:identity=FILE` (TCP and UDP through a WARP session, for `route` and `forward --network`). | [egress](https://github.com/PodUtils/podnet/blob/main/docs/egress.md), [proxies](https://github.com/PodUtils/podnet/blob/main/docs/proxies.md) |
| Background | `--detach`, `ps`, `logs`, `wait`, `stop`, `autostart` | Run a command in the background, restart it, start it at boot. | [background](https://github.com/PodUtils/podnet/blob/main/docs/background.md) |
| Checks | `doctor`, `doctor --agent`, `status`, `ports`, `components`, `bench`, `bug-report`, `agent up` | Test the host, the path to the edge and a way out. | [doctor](https://github.com/PodUtils/podnet/blob/main/docs/doctor.md) |
| Update | `update` | Replace the binary after a check of the signature. | [install](https://github.com/PodUtils/podnet/blob/main/docs/install.md) |
| Manual | `man`, `--help --json`, `completions` | The manual, JSON help, the llms files, exit codes. | [cli](https://github.com/PodUtils/podnet/blob/main/docs/cli.md) |

## Limits that podnet handles

| Limit on the host | Part of podnet |
| --- | --- |
| No UDP to the edge | The HTTP/2 edge leg on TCP 7844. |
| Only TCP 443 out | The QUIC edge leg inside WARP (MASQUE), or the WebSocket relay. |
| No DNS resolver | DNS over HTTPS by IP address, then a static edge list. |
| Egress through a proxy only | HTTP `CONNECT`, SOCKS5, Digest, unix-socket and command proxies, and `NO_PROXY`. |
| No root, no TUN, no network namespace | `podnet pod` (seccomp backend), `podnet warp run`. |
| No `AF_INET` bind | `podnet net` (shim and userspace stack), unix-socket origins. |

One limit stays: a proxy that terminates TLS cannot carry the edge connection or the MASQUE connection,
because both need the Cloudflare certificate chain. A quick tunnel is for tests and development: it has
no uptime guarantee, at most 200 requests in flight (more get `429`), and no Server-Sent Events
([limits](https://github.com/PodUtils/podnet/blob/main/docs/protocol.md#7-limits-of-quick-tunnels)).
[docs/target-environment.md](https://github.com/PodUtils/podnet/blob/main/docs/target-environment.md) gives
the measured hosts. The workflow `.github/workflows/live.yml` runs `podnet demo` with each edge leg on a
GitHub runner on each push and each day.

## Documents

* [docs/install.md](https://github.com/PodUtils/podnet/blob/main/docs/install.md): install, verify, update, rebuild a release.
* [docs/tunnels.md](https://github.com/PodUtils/podnet/blob/main/docs/tunnels.md): origins, the configuration file, named tunnels, the edge protocol and transport, raw streams.
* [docs/serve.md](https://github.com/PodUtils/podnet/blob/main/docs/serve.md): the file server, share, git hosting.
* [docs/warp.md](https://github.com/PodUtils/podnet/blob/main/docs/warp.md): the WARP client.
* [docs/pods.md](https://github.com/PodUtils/podnet/blob/main/docs/pods.md): `podnet pod`.
* [docs/networks.md](https://github.com/PodUtils/podnet/blob/main/docs/networks.md): `podnet net`, Tailscale, NetBird, the passwd shim.
* [docs/router.md](https://github.com/PodUtils/podnet/blob/main/docs/router.md): `podnet route`.
* [docs/forward.md](https://github.com/PodUtils/podnet/blob/main/docs/forward.md): `forward`, the DNS door, `expose`.
* [docs/relay.md](https://github.com/PodUtils/podnet/blob/main/docs/relay.md): `relay` and `ssh-relay`.
* [docs/link.md](https://github.com/PodUtils/podnet/blob/main/docs/link.md): `link` (the key of an install, the allow list, the carriers, the services, the mesh).
* [docs/egress.md](https://github.com/PodUtils/podnet/blob/main/docs/egress.md): network specs and `--egress`.
* [docs/proxies.md](https://github.com/PodUtils/podnet/blob/main/docs/proxies.md): proxies, DNS, TLS roots.
* [docs/background.md](https://github.com/PodUtils/podnet/blob/main/docs/background.md): `--detach`, `ps`, autostart.
* [docs/doctor.md](https://github.com/PodUtils/podnet/blob/main/docs/doctor.md): `doctor`, `bench`, `status`, `components`, `agent`.
* [docs/cli.md](https://github.com/PodUtils/podnet/blob/main/docs/cli.md): the manual, JSON help, llms files, exit codes.
* [docs/library.md](https://github.com/PodUtils/podnet/blob/main/docs/library.md) and [docs/build.md](https://github.com/PodUtils/podnet/blob/main/docs/build.md): the Rust library, build and fuzz.
* [docs/protocol.md](https://github.com/PodUtils/podnet/blob/main/docs/protocol.md): the edge protocol and the WARP protocol.
* [docs/virtual-network.md](https://github.com/PodUtils/podnet/blob/main/docs/virtual-network.md): the shim, the stack and the switch.
* [docs/target-environment.md](https://github.com/PodUtils/podnet/blob/main/docs/target-environment.md): the measured hosts and the rules of podnet for them.
* [crates/podnet-netns/README.md](https://github.com/PodUtils/podnet/blob/main/crates/podnet-netns/README.md): the pod backends.
* [SECURITY.md](https://github.com/PodUtils/podnet/blob/main/SECURITY.md): how to report a vulnerability, what Cloudflare can see, what listens, where secrets are kept.
* [docs/development.md](https://github.com/PodUtils/podnet/blob/main/docs/development.md) and [docs/maintenance.md](https://github.com/PodUtils/podnet/blob/main/docs/maintenance.md): the rules and the procedures for contributors.

This README describes `main`. The notes in `docs/releases/` say what a tag has. `llms.txt` and
`llms-full.txt` give the manual and the documents as plain text for agents.

## License

0BSD. See [`LICENSE`](https://github.com/PodUtils/podnet/blob/main/LICENSE). See
[`THIRD_PARTY_NOTICES.md`](https://github.com/PodUtils/podnet/blob/main/THIRD_PARTY_NOTICES.md) for the licenses of the
included third-party files.

---

Former name: podnet was `cfrs` before v0.4.0-beta.1. The binary, the variables and the signing identity have new names.
