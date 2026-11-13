# Tunnels

This page gives the Cloudflare tunnels of `podnet`: origins, the configuration file, named
tunnels, the edge protocol, the credential in front of a tunnel and raw streams.


A quick tunnel (`podnet tunnel` with no account, a name under `*.trycloudflare.com`) is for tests and
development: no uptime guarantee, at most 200 requests in flight (more get `429`), no Server-Sent Events,
and the Cloudflare Online Services Terms ([limits and terms](protocol.md#7-limits-of-quick-tunnels)).
A named tunnel uses a Cloudflare account.

## Origins

```sh
podnet tunnel --url http://127.0.0.1:8080   # HTTP or HTTPS origin
podnet tunnel --port 8080                   # the same as http://127.0.0.1:8080
podnet tunnel --unix /run/app.sock          # HTTP origin on a unix socket
podnet tunnel --hello-world                 # built-in test origin
podnet tunnel --config ./config.yml         # cloudflared ingress rules
podnet tunnel --token "$T" --tcp 10.0.0.5:5432  # raw TCP origin (named tunnel)
```

A raw TCP origin (`--tcp host:port` or `tcp://host:port`) carries TCP and
WebSocket streams of a named tunnel (WARP routing, `cloudflared access
tcp`). A quick tunnel receives HTTP: podnet answers each HTTP request to a
raw TCP origin with 501, and does not verify the public URL. For TCP
through a quick tunnel, use `--forward` and `podnet connect` (see "Raw streams
through a quick tunnel" below).

The service strings are the same as in cloudflared:

```text
http://host:port     https://host:port     unix:/path/to.sock
unix+tls:/path.sock  tcp://host:port       http_status:404
hello_world          metrics               static:/srv/www
spa:/srv/www
```

* `static:` serves a directory.
* `spa:` serves a directory and sends `index.html` for unknown paths.
* `metrics` serves the Prometheus metrics of the tunnel.
  `podnet_decisions_total{decision,outcome}` counts the path decisions of
  the process: `dns` (`ip`, `pin`, `system`, `doh`, `fail`), `egress`
  (`direct`, `none`, `no_proxy`, `via`, `http`, `socks5`, `unix`, `exec`),
  `leg` (the leg that won), `masque` (`direct`, `relay`, `relay_first`,
  `no_relay`), `dispatch` (the requests of `warp proxy`) and `rule`. The
  health line of `warp proxy` has the same counters.
* `--http-host-header NAME` sets the `Host` header for the origin.
* `--no-tls-verify` does not verify the TLS certificate of an HTTPS origin.
* `--no-verify` (`tunnel`, `serve`, `share`, `demo`) does not fetch the public URL after the registration. The
  fetch proves that the URL answers. The flag does not change the check of an origin certificate: `--no-tls-verify`
  does that. Use `--no-verify` on a host that resolves a new `*.trycloudflare.com` name late.

All built-in origins run in the process. They do not open a TCP listener.

## Configuration file

`podnet tunnel --config` reads a cloudflared YAML file:

```yaml
tunnel: my-tunnel                      # the name or the UUID of the tunnel
credentials-file: /etc/podnet/creds.json
ha-connections: 2
protocol: auto
ingress:
  - hostname: app.example.com
    path: /api
    service: http://127.0.0.1:8080
    originRequest:
      noTLSVerify: true
      connectTimeout: 5s
  - service: http_status:404
```

* The origin is in the `ingress:` rules. With `--config`, the origin flags (`--url`, `--unix`, `--port`, `--tcp`,
  `--forward`, `--hello-world`, `--no-tls-verify`, `--http-host-header`, `--strip-accept-encoding`) are a usage
  error (exit 64): they would have no effect.
* The last rule must match all requests (no `hostname`, no `path`). podnet
  refuses a file where it does not.
* `path:` is a regular expression, as in cloudflared (the syntax of the
  Rust `regex` crate, which is close to Go RE2). It matches when it
  matches a part of the request path: `path: /api` also matches `/x/api`
  and `/apix`. Write `path: ^/api/` for the paths that start with `/api/`,
  and `path: ^/api$` for one path. A `.` matches each character: write
  `\.` for a dot. The query (from `?`) is not part of the path. The match
  reads the decoded path, as cloudflared does: `/a%20b` matches `^/a b$`
  (a `+` stays a `+`). A `path:` that is not a valid regular expression
  is an error at load (exit 78), with the file and the rule number.
* With no `ingress:`, `url:` gives one rule for each request (the same
  services as `service:`), and `hello-world: true` gives the hello-world
  page. `url:` or `hello-world:` with `ingress:` is an error.
* A file with no rule (no `ingress:`, `url:` or `hello-world:`) fails
  `--check` (exit 78) and the run.
* With `credentials-file:`, `tunnel:` is the name or the UUID of the tunnel,
  as in cloudflared. A UUID must be the `TunnelID` of the file. With
  `tunnel:` only, it is a tunnel token.
* `token:` is the tunnel token of a named tunnel. `token:` with
  `credentials-file:` is an error. With `token:`, a `tunnel:` must be the
  tunnel id of the token.
* With no `tunnel:`, no `credentials-file:` and no `token:`, podnet starts a
  quick tunnel. `--token`, `TUNNEL_TOKEN` and `--credentials-file` win over
  the file.
* podnet ignores the cloudflared keys that it does not use (`metrics`,
  `loglevel`, `warp-routing`, `tcpKeepAlive`, ...). Another key is an error
  (exit 78), with the nearest key. podnet checks the keys at the top level, in
  each ingress rule and in each `originRequest`.
* The top-level `originRequest` is the default of each rule. A key of the
  rule wins.
* `protocol:` and `ha-connections:` of the file are used. A flag
  (`--protocol`, `--ha-connections`) wins over the file.
* `--token` and `--credentials-file` on one command line are a usage error
  (exit 64). With the token from `TUNNEL_TOKEN`, `--credentials-file` wins,
  and a warning says so. A bad token or credentials file exits 78.
* A UTF-8 BOM at the start of the file is skipped. A list, an empty file, a
  rule with no `service`, a time with no unit (`connectTimeout: 5`) and a
  value of the wrong type each have their own message.

podnet reads the YAML with its own reader (`crates/podnet-core/src/yaml.rs`). The reader accepts
block mappings, block sequences, plain and quoted scalars on one line, and
comments. It refuses anchors, aliases, tags, block scalars (`|`, `>`), flow
collections that are not empty, tabs in the indentation, duplicate keys and
a document that nests deeper than 64 levels, and gives the line number.

## Named tunnels

A named tunnel uses a Cloudflare account and a hostname from the dashboard.

```sh
TUNNEL_TOKEN=<token> podnet tunnel --url http://127.0.0.1:8080
podnet tunnel --token-file /run/secrets/tunnel-token --url http://127.0.0.1:8080
podnet tunnel --credentials-file ~/.cloudflared/<id>.json --url http://127.0.0.1:8080
```

`--token <token>` also works. Other users of the host can read a command
line, so `TUNNEL_TOKEN` or a file is safer. `--token-file PATH` (or
`TUNNEL_TOKEN_FILE`) reads the token from a file; white space at the ends
is removed. `podnet relay connect --token-file PATH` reads a connect token,
and `podnet warp license -` reads the license key from stdin.

A command line of cloudflared works for a tunnel: `podnet tunnel run --token
T` is `podnet tunnel --token T`, `--no-autoupdate` is ignored, and
`--loglevel debug|info|warn|error` becomes `-v`, nothing or `-q`. Another
flag of cloudflared (`--logfile`, `--pidfile`, `--metrics`, ...) is a usage
error with the step to take in podnet.

## Edge protocol and transport

`--protocol` selects the protocol to the edge. `--transport` selects the
path.

| Leg | Socket on the host | Use it when |
| --- | --- | --- |
| `quic` | UDP 7844 | UDP to the edge is open. This is the fastest leg. |
| `http2` | TCP 7844, directly or through the proxy | UDP is blocked and TCP 7844 is open. |
| `quic+warp` | TCP 443 only (MASQUE) | Only TCP 443 is open. |
| `http2+relay` | TCP 443 to a WebSocket relay | Only TCP 443 is open, and WARP does not work (the proxy terminates TLS or takes names only). Only with `--transport relay`. |

With the default values (`--protocol auto --transport auto`), `podnet` tries the
legs in the order of the table, in a staggered race: the next leg starts when
the leg before has not registered in 2 seconds, or at once when it fails, and
the first leg that registers stays (`docs/protocol.md` section 3.3).
`--leg-failover serial` waits for each failure. The time limit of an attempt
is the budget of its leg: 130 seconds for `quic`, 280 seconds for `http2`
and `http2+relay` (`docs/protocol.md` section 3.3). Other values:

| `--protocol` | `--transport` | Legs |
| --- | --- | --- |
| `auto` | `auto` | `quic`, `http2`, `quic+warp` |
| `quic` | `auto` | `quic`, `quic+warp` |
| `http2` | `auto` or `direct` | `http2` |
| `auto` | `direct` | `quic`, `http2` |
| `quic` | `direct` | `quic` |
| `auto` or `quic` | `warp` | `quic+warp` |
| `auto` or `http2` | `relay` | `http2+relay` |

`h2` is a name of `http2`. With a proxy, `auto` drops `quic` (see [proxies.md](proxies.md)); an explicit `--protocol quic` keeps it, and a warning says that
QUIC goes around the proxy.

`--transport relay` sends the HTTP/2 leg in a WebSocket to a
WebSocket-to-TCP relay on TCP 443 (`--relay URL`, or `PODNET_RELAY`; the
default is `https://tcp.ssh.relay.ajam.dev`). The relay opens TCP to the
edge on port 7844. The TLS to the edge is in the WebSocket, so the relay
does not see the tunnel. `PODNET_RELAY_TOKEN` gives a relay token; without
it, `podnet` mints one and keeps it in the state directory. When the relay
host does not answer, `podnet` tries other hosts of the same deployment (a
comma list in `--relay`, else the pool of the relay). The relay carries IPv4 TCP only, and it ends a
session after 64 MiB or 12 hours. Before that (at 60 MiB or 11.5 hours),
the tunnel connects a replacement edge connection, and the old one
finishes its open streams (`docs/protocol.md` section 3.4). A WARP
(MASQUE) session through the relay is replaced at the same limits: a new
session takes the new connections, and the open connections go on in the
old session for 90 s.

`--warp-identity PATH` keeps the WARP device in a file. Without it, each run
registers a new anonymous device. `PODNET_EDGE=ip:port,...` replaces edge
discovery.

`--masque-endpoint IP:PORT` names the WARP MASQUE front door of
`--transport warp`, in place of the doors of the device record (as in
`podnet warp`). Without it, a host with no IPv4 default route tries the IPv6
door of the record first.

`PODNET_FOREIGN_IDENTITY=allow` lets `--warp-identity PATH` name a file that
another user owns (root reads the device of a service user, and no second
device is registered). podnet checks the mode of the file (0600), logs a
warning, and writes nothing next to the file. The variable has no effect on
the default file of the state directory. Without it, a file of another user
exits 78 and the error names the variable.


## A credential in front of a tunnel

```sh
umask 077; printf '%s\n' 'a long password' > ~/.podnet-pass
podnet tunnel --port 3000 --auth-user me --auth-password-file ~/.podnet-pass
curl -u me:'a long password' https://<random>.trycloudflare.com/
```

* `--auth-user USER` and `--auth-password-file FILE` (`PODNET_AUTH_USER`,
  `PODNET_AUTH_PASSWORD_FILE`) work with `tunnel`, `serve`, `share` and
  `demo`. Each request must give the user and the password (HTTP Basic).
  Else podnet answers 401 with `WWW-Authenticate: Basic realm="podnet"`, and
  the request reaches no origin. The origin does not get the
  `Authorization` header.
* The file has the password on its first line and mode 0600; else exit 78.
  The password is not on the command line.
* The check covers the HTTP origins and the `--forward` endpoint:
  `podnet connect --auth-user USER --auth-password-file FILE` sends the
  credential in the WebSocket upgrade. A raw `--tcp` origin carries no HTTP
  header: `--auth-user` with `--tcp` is a usage error (exit 64).
* The verify of the public URL takes the 401 as the normal answer.
* Basic goes in the TLS to the edge of Cloudflare: Cloudflare can see it.
  The credential does not hide the URL.

## Raw streams through a quick tunnel

A quick tunnel carries HTTP and WebSocket. `--forward` puts a WebSocket
endpoint at `/__podnet/ws`. `podnet connect` changes local sockets into
connections through that endpoint. Thus SSH, databases and unix sockets can
go through a quick tunnel.

```sh
# Server: expose a unix socket (or tcp://host:port).
podnet tunnel --forward unix:/run/app.sock

# Client: local port 8080 goes to that unix socket.
podnet connect wss://<random>.trycloudflare.com/__podnet/ws -L tcp://127.0.0.1:8080
```

The tunnel prints the endpoint (`podnet: forward  wss://...`). The ready
event of `--json`, the ready document of `--detach` and `podnet ps --json`
have it as `forward` (and `forward_path`); `podnet ps` shows it on a second
line.

`-L` accepts `tcp://[bind:]port` and `unix:///path` and is repeatable. Each
local connection gets its own WebSocket. `--forward` and an origin can be used
together: the origin gets all paths other than `/__podnet/ws`.

`podnet connect` dials as the other commands do: `--proxy` (or HTTPS_PROXY and
NO_PROXY), `--resolve`, the system resolver and DoH, and the roots of
`--ca-file`. At start, it opens one WebSocket to the remote and exits 69 when
the remote does not answer; `--no-check` starts without this check. A unix
socket that another process serves is refused (exit 69); `--replace` takes
it. At the end, podnet removes the socket files that it made. `--detach` and
`--name` work as for `tunnel` (see [background.md](background.md)); the ready
document has `remote`, `listen` and `listeners`. At `-v`, each WebSocket
logs the time of its TCP, TLS and upgrade steps.

`podnet connect URL --stdio` carries one stream on stdin and stdout, with no
listener. Use it as an SSH `ProxyCommand`:

```sh
ssh -o ProxyCommand='podnet connect wss://<random>.trycloudflare.com/__podnet/ws --stdio' user@host
```

The end of stdin closes the WebSocket. The close of the remote ends the
command. `--stdio` takes `--auth-user` and `--auth-password-file`. It exits
69 when the remote does not answer. It cannot be used with `-L` or
`--detach`.

## Other flags of `podnet tunnel`

* `--ha-connections N` (1 to 8; default 2, else `ha-connections` of `--config`): the number of edge
  connections. When the edge loses one, the others carry the visitors. The edge chooses the connection of
  each request, and one request uses one connection, so more connections do not make one transfer faster.
* `--qr` prints the public URL as a QR code. `--json` prints JSON lines in place of text lines (see
  [cli.md](cli.md)). `--run-for SECONDS` stops the tunnel after that time.
* `--service-url URL` changes the provisioning endpoint of the quick tunnel (default
  `https://api.trycloudflare.com`), and `--user-agent TEXT` sets the `User-Agent` that podnet sends when it
  provisions (`docs/protocol.md` section 1).
* `--check` checks the plan and stops (see [cli.md](cli.md)).

## Other commands

```sh
podnet ports            # list the listening TCP ports (Linux)
podnet ports --probe    # only the ports that accept a connection
podnet qr <URL>         # print a QR code
```

`podnet expose TARGET` publishes a local service by one word ([forward.md](forward.md)).
`podnet ssh-relay` publishes a unix socket through an SSH relay ([relay.md](relay.md)).
