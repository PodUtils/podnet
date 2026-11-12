# Relays: `podnet relay` and `podnet ssh-relay`

This page gives the reverse relay (`podnet relay`) and the SSH forward relay (`podnet ssh-relay`).

## Inbound access through the relay: `podnet relay`

A host with outbound HTTPS only cannot take inbound connections (SSH, a
debugger, a database client). A quick tunnel carries HTTP and WebSocket
only. The reverse mode of the WebSocket relay (`--relay`, default
`https://tcp.ssh.relay.ajam.dev`) carries raw TCP bytes to a local service.

```sh
# On the host:
podnet relay pair                                   # name, connect token; the node token stays in a 0600 file
podnet relay node --to 127.0.0.1:22 --detach        # hold the node socket; one local connection for each session

# On the computer of the operator:
export PODNET_RELAY_CONNECT_TOKEN=<connect token>
ssh -o ProxyCommand='podnet relay connect NAME' USER@NAME
podnet relay connect NAME --listen tcp://127.0.0.1:2222   # or a local port for each client

# On the host, at the end:
podnet relay stop NAME
```

* `podnet relay pair` asks the relay for a pair (`POST /v1/pair`). A pair lasts
  at most 72 h. The file is `relay/NAME.json` in the state directory, with
  mode 0600. `--json` prints the name, the connect token and the end time.
* `podnet relay node [NAME] --to TARGET` takes the pair with the most time left
  when no name is given. `TARGET` is `host:port` or `unix:/path`. The node
  answers each `open` with `ready` after its local connection is ready, or
  with `reject` and the cause. A lost node socket is opened again after 1 s
  to 30 s, with jitter. The node warns 24 h, 6 h and 1 h before the pair
  expires. A node with no pair name takes the next stored pair at the
  expiry (a pair that `podnet relay pair` made since), with no new start. Its
  operators need the name and the connect token of the next pair. When the
  relay ends the pair (it expired with no next pair, or `podnet relay stop`
  ran), the node exits 78 and is not started again.
* `podnet relay connect NAME` waits for `ready`, then copies stdin and stdout.
  A refused session exits 69 with the cause from the node. A refused token
  exits 78. After the end of stdin, the session ends when the node closes,
  or 15 s (the idle time of the relay) after the last data. A close of the
  relay other than 1000 (for example 1009 at its byte limit) or a lost link
  exits 69, also after the end of stdin: the output can be cut.
* The relay authenticates tokens, not persons. Give the connect token only
  over a channel that you trust. Check the host key of SSH as usual.
* The relay sets its own limits; podnet does not enforce them. Measured on
  `tcp.ssh.relay.ajam.dev`: 64 MiB and 12 h for each session, and it can end
  a session after 180 s with no payload. Open a new session after a limit.

## Other uses of the relay

* `podnet serve --ingress relay --pair NAME` and `podnet git serve --ingress relay --pair NAME` put the
  file server and the git SSH server on the node of a pair ([serve.md](serve.md)).
* `podnet expose TARGET --via relay` starts a node for a local service ([forward.md](forward.md)).
* `--egress relay` (or `--transport relay`) sends the edge connection of a tunnel in a WebSocket to the
  relay ([tunnels.md](tunnels.md), "Edge protocol and transport"). `--egress relay` is also a network of
  `podnet forward` and of a router ([egress.md](egress.md)).
* `podnet warp nc --via-relay HOST PORT` and the relay path of the MASQUE connection ([warp.md](warp.md))
  use the same relay.
* `podnet relay connect` and `podnet relay node` take `--detach` ([background.md](background.md)).
* `podnet doctor` tests the TCP and TLS path to the relay ([doctor.md](doctor.md)).

## SSH forward relay: `podnet ssh-relay`

Some hosted relays give a public URL for a unix socket through an SSH remote forward on port 443 (the
server prints the URL in its banner). `podnet ssh-relay` is the client for them. It publishes a unix
socket and keeps the session: when the relay ends the session, podnet connects again with a backoff and
prints the new URL.

```sh
podnet ssh-relay --socket /run/user/1000/origin.sock --detach
podnet ssh-relay --service serveo --socket /run/user/1000/origin.sock --detach
podnet ssh-relay --server relay.example.net:443 --user podnet \
  --url-skip dashboard.example.net --socket /run/user/1000/origin.sock --detach
```

* `--service NAME` selects a known relay. Its endpoints, its SSH user and its URL rule are data of
  `pod-services` (`pod_services::forward`). The default is `pinggy`. An unknown name exits 64 and lists the
  known relays. podnet holds no relay host.
* `--server HOST:PORT` replaces the endpoints of the relay. Use a port that the egress allows (443 for most
  proxies). With `--server` and no `--service`, the relay is not a known one: give `--user` and a URL rule
  (else exit 64). Without `--server`, podnet tries the plain SSH endpoints of the relay in order, and goes
  to the next endpoint on a transport error only.
* `--user USER` replaces the SSH user of the facts. The relay takes an ephemeral key.
* The banner of the relay can hold more than one URL, and a URL can cross two data frames. podnet reads the
  whole banner, then picks the URL. `--url-match TEXT` selects the tunnel URL: it holds this text (the
  tunnel domain of the relay). `--url-skip TEXT` removes a URL: it holds this text. Both are repeatable,
  and each replaces its list of the facts. The default skip list holds the dashboards of all known relays.
* `--socket PATH` is the unix socket of the origin. The relay opens one channel for each visitor, and
  podnet connects it to this socket. The origin must listen on the socket before the first visitor comes.
  The default is `origin.sock` in the private run directory of this user.
* `--connect-proxy HOST:PORT` reaches the relay through an HTTP CONNECT proxy. The default is a direct
  connection. `--proxy`, `--egress` and `--resolve` apply as for the other commands ([proxies.md](proxies.md)).
* `--json` prints each URL as one JSON line: `{"event": "ready" or "reconnected", "url", "remote_port",
  "relay"}`.
* `--detach`, `--name` and the other options of [background.md](background.md) work. The ready document
  has the URL. `podnet ps` shows the new URL after each reconnect.
* `podnet doctor --ssh-relay HOST:PORT --ssh-user USER` tests the relay with no session: the TCP connect,
  the SSH banner and the authentication of the user. With no flag, it reads the relay of a running
  `ssh-relay` instance ([doctor.md](doctor.md)).
