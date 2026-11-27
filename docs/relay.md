# Relays: `podnet relay`, `podnet derp` and `podnet ssh-relay`

This page gives the reverse relay (`podnet relay`), the DERP road (`podnet derp`) and the SSH forward relay (`podnet ssh-relay`).

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
  at most 72 h. The file is `relay/pair-NAME.json` in the state directory,
  with mode 0600. `--json` prints the name, the connect token and the end
  time. An old file `relay/NAME.json` is not read: make a new pair.
* `podnet relay node [NAME] --to TARGET` takes the pair with the most time left
  when no name is given. `TARGET` is `host:port` or `unix:/path`. The node
  answers each `open` with `ready` after its local connection is ready, or
  with `reject` and the cause. A lost node socket is opened again at once
  after a socket that served, else after a backoff with jitter (`pod-relay`
  of PodUtils/podcrates). The node warns 24 h, 6 h and 1 h before the pair
  expires. A node with no pair name takes the next stored pair at the
  expiry (a pair that `podnet relay pair` made since), with no new start. Its
  operators need the name and the connect token of the next pair. When the
  relay ends the pair (it expired with no next pair, or `podnet relay stop`
  ran), or refuses the node token (a 401 or 403 at the upgrade of the node
  socket), the node exits 78 and is not started again. A Close 1008 that
  names the token does not stop the node: it opens the socket again. When
  another node holds the name for 30 s, the node exits 69. When the relay
  still holds the lost socket of this node (409) for 60 s, the node stops
  on that route too (T-406): with a pair directory it takes the next stored
  pair, else it exits 69 and says to make a new pair or to use another relay.
* `podnet relay node --json` prints one JSON line on stdout when the node is
  connected to the relay: `event` (`ready`), `pair`, `relay`, `target`,
  `expires` and `expires_in`. The node stays in the foreground. `--detach`
  prints the same document and returns.
* `podnet relay node --run-for SECONDS` stops the node after that time, with
  exit 0 (a planned end: the supervisor of `--detach` does not start it
  again).
* `podnet relay connect NAME` waits for `ready`, then copies stdin and stdout.
  A name that is not a valid pair name exits 64.
  A refused session exits 69 with the cause from the node. A refused token
  exits 78. After the end of stdin, the session ends when the node closes,
  or 15 s (the idle time of the relay) after the last data. A close of the
  relay other than 1000 (for example 1009 at its byte limit) or a lost link
  exits 69, also after the end of stdin: the output can be cut.
* A proxy that does not pass a WebSocket upgrade answers it with its own
  page. `relay connect` then says that a hop on the way refused the upgrade,
  not the relay, and exits 69: the token is not the cause. Each relay
  connection takes the roots of `--ca-file` or `SSL_CERT_FILE`, as the other
  clients of podnet do ([proxies.md](proxies.md)).
* The relay authenticates tokens, not persons. Give the connect token only
  over a channel that you trust. Check the host key of SSH as usual.
* The relay sets its own limits; podnet does not enforce them. Measured on
  `tcp.ssh.relay.ajam.dev`: 64 MiB and 12 h for each session, and it can end
  a session after 180 s with no payload. In the plain mode, open a new
  session after a limit. With `--resume` on both sides, the session goes on
  past the limits (next section).

## Sessions past the limits of the relay: `--resume`

`relay node --resume` and `relay connect --resume` run the resumable layer
(`pod-session` of PodUtils/podcrates) in each session. A session then goes
on past the byte and time limits of the relay, and after the loss of a leg.

```sh
podnet relay node --to 127.0.0.1:22 --resume --detach
ssh -o ProxyCommand='podnet relay connect NAME --resume' USER@NAME
```

* A leg is one relay session: one operator socket to `/v1/connect/NAME`.
  The first leg starts the session of the layer. Before a leg carries 48 MiB
  (both ways) or becomes 11 h old, `relay connect` opens `/v1/connect/NAME`
  again, and the session moves to the new leg. The old leg stops at a record
  boundary. With `-v`, podnet prints each move.
* After the loss of a leg (a close of the relay, a lost link), `relay connect`
  opens a new leg with a backoff. Each side sends again from the offset that
  the other side received. The node keeps a session with no leg for 10
  minutes, and the connection to the target stays open. The replay memory of
  a node is at most 64 MiB for all its sessions.
* The end of the input of `relay connect` ends one direction only: the answer
  of the target still comes.
* A session cannot go past the end of its pair. When `relay connect` reads
  the pair from the local pair file, it warns 24 h, 6 h and 1 h before the
  expiry. When the relay refuses the connect token for a new leg (the pair
  expired or was stopped), the session ends with exit 78.
* Both sides must use `--resume`:
  * A node with `--resume` and an operator with no `--resume`: the operator
    exits 78, with the words "the node NAME runs the resumable layer". An
    operator of podnet before T-087, and other clients of the reverse road,
    get the bytes of the layer and fail. Give `--resume` to the node only
    when each operator uses it.
  * An operator with `--resume` and a node with no `--resume`: the session
    runs in the plain mode, with a warning, and it ends at the limits of the
    relay. A target that sends first (sshd) starts at once. A target that
    waits for the client (HTTP) gets the first bytes after 30 s.
* `serve --ingress relay`, `git serve --ingress relay` and `expose --via relay`
  run their nodes in the plain mode.

## Sealed sessions: `--seal`

`relay node --resume --seal --allow FILE` and `relay connect --resume --seal`
run an end-to-end channel above the resumable layer (T-087;
`podnet::relay_reverse::Seal`, over `pod-e2e` and `pod-relay` of
PodUtils/podcrates). The relay then sees sealed messages only. It is off by
default.

```sh
# On the host: the fingerprint of the node key.
podnet link status --create
podnet relay node --to 127.0.0.1:22 --resume --seal --allow operators.allow --detach
# On the computer of the operator:
ssh -o ProxyCommand='podnet relay connect NAME --resume --seal --peer NODE-FINGERPRINT' USER@NAME
```

* The key of each end is the key of the install: the key of `podnet link`
  (`<state directory>/link-identity.key`, made on first use). `podnet link
  status` prints its fingerprint.
* The handshake is Noise XX with the Ed25519 key of each end. The operator
  checks the key of the node against `--peer FINGERPRINT` (repeat it for more
  than one) or `--allow FILE` before it proves its own key. The node checks
  the key of the operator against its `--allow FILE`. A file has the form of
  `link listen --allow` (one key on each line, default deny).
* A key that the other end does not allow exits 77 at the operator. The node
  dials the target only after it lets the operator in: a refused operator
  costs no connection to the target and no replay memory of the layer. The
  node writes each refusal to its log.
* The channel is above the layer. A move to a new leg and a resume after a
  lost leg carry the same channel on.
* `--seal` needs `--resume`. The node needs `--allow`. The operator needs
  `--peer` or `--allow`. Else exit 64.
* Both sides must use `--seal`:
  * A sealed operator and a node with no `--seal`: the handshake fails, and
    the operator exits 69 with the words "the channel's handshake failed".
  * A node with `--seal` and an operator with no `--seal`: the operator gets
    the bytes of the handshake of the node (not the bytes of the target), and
    the target gets no connection. The session ends with exit 0. Give
    `--seal` to the node only when each operator uses it.
* The connect token still decides who reaches the node at the relay. The keys
  decide who reaches the target.

## Meet at a DERP server: `podnet derp`

Two hosts with outbound HTTPS only can meet at a DERP server: a server of
Tailscale's fleets (no account) or a `derper` of your own. DERP runs in a
WebSocket at `wss://HOST/derp` on port 443, through the way out of podnet
(`--proxy`, `--resolve`, `--ca-file`). `pod-derp` of PodUtils/podcrates does
the work (T-128). No pair and no token are necessary: the ticket of the node
is its address, and the keys decide who comes in.

```sh
# On the host: the fingerprint of the key of the client.
podnet derp node --to 127.0.0.1:22 --peer CLIENT-FINGERPRINT --detach
# On the computer of the client:
ssh -o ProxyCommand='podnet derp connect TICKET' USER@HOST
```

* `podnet derp node --to TARGET` goes online at the first server of the list
  that answers its probe and its DERP handshake, and prints its ticket
  (`derp:` and one word of base32): the key of the node, its key on the
  server, and the server. `TARGET` is `host:port` or `unix:/path`. The key on
  the server comes from the key of the install, so a new start at the same
  server gives the same ticket.
* The servers: `--server HOST[:PORT][,HOST[:PORT]...]` (port 443 by default),
  else the fleet of the data of `pod-services`: the four servers for tailcat,
  then Tailscale's main fleet. `--main-fleet no` keeps the four servers alone.
  podnet holds no host of a DERP server.
* When the home of the node goes away (three failed connections in a row),
  the node moves to the next server of the list and prints its new ticket. A
  client of the old ticket and of the same list tries the other servers of the
  list after the server of the ticket, and finds the node there.
* `podnet derp connect TICKET` copies stdin and stdout to the node. Give it the
  same `--server` list as the node (or none).
* Each session runs the resumable layer (`pod-session`), with the end-to-end
  channel of `pod-e2e` above it (Noise XX with the key of each install, as
  `relay --seal`). The server sees the keys, the sizes and the times only.
  After the loss of a link, the client opens a new link, and the session goes
  on from the offsets of each end. The node keeps a session with no link for
  10 minutes; its replay memory is at most 64 MiB for all its sessions.
* The node admits the keys of `--peer FINGERPRINT` (repeat it) or `--allow
  FILE` (the form of `link listen --allow`); give one of them, else exit 64.
  The client checks that the node proves the key of the ticket. A key that an
  end refuses exits 77 at the client, and the node does not dial the target.
* The end of the input of `derp connect` ends one direction only: the answer of
  the target still comes.
* `--json` prints the ready document of the node as one JSON line: `ticket`,
  `server`, `fingerprint` and `target`. `--detach` prints the same document.
  `--run-for SECONDS` stops the node after that time, with exit 0.
* Exit codes: 64 for a bad value (a ticket, `--server`, `--main-fleet`,
  `--to`), 69 when no server takes the node, when the client reaches no node,
  when the target does not answer, or when the session ends with an error,
  77 for a key that an end refuses, 78 when the two ends share no suite.
* `podnet send --derp` offers a node of the run at a DERP server as a way of the
  transit to a podnet receiver ([transfer.md](transfer.md), A DERP server).
* `podnet doctor` has the row `derp`: the first server of the list that
  answers derper's probe through the way out, and its round trip, or the
  reason why none does. `podnet doctor --derp-server LIST` probes your own
  servers ([doctor.md](doctor.md)).
* The step "The DERP road through a derper on the runner" of
  `.github/workflows/live.yml` runs a node and a client through a `derper` of a
  pinned version (v1.104.4) on the runner, then the row `derp` of the doctor
  and a transfer of 8 MiB by `send --derp` through the same `derper`.

## Hosts and limits

* A list in `--relay` (comma-separated) is a preference of order. A host that
  fails with an error that another host can repair, or that says nothing while
  a host after it opens, is tried last for 10 minutes. A host that opens is
  tried first for the same time. One failure moves a host. No host is left
  out. The record is a private file in the state directory
  (`pod-relay` of PodUtils/podcrates).
* The budget of a stream through the relay (the edge leg of a tunnel and the
  MASQUE leg of WARP) follows the `limits` of the pool document of the relay
  (`max_session_bytes`, `max_session_ms`): 4 MiB under the byte cap and 30
  minutes under the time cap, never more than 60 MiB and 11.5 h. A relay with
  no `limits` keeps these values. The HTTP/2 edge leg still counts the
  constants (T-163).
* A pair is made on the control host of the relay only. The node and the
  operator use the first host of the list (T-200).

## Other uses of the relay

* `podnet serve --ingress relay --pair NAME` and `podnet git serve --ingress relay --pair NAME` put the
  file server and the git SSH server on the node of a pair ([serve.md](serve.md)).
* `podnet expose TARGET --via relay` starts a node for a local service ([forward.md](forward.md)).
* `podnet send --relay-pair NAME` runs the node of a stored pair for the time of a transfer: the last way of the
  transit to a podnet receiver, with the resumable layer ([transfer.md](transfer.md), The podnet relay).
* `--egress relay` (or `--transport relay`) sends the edge connection of a tunnel in a WebSocket to the
  relay ([tunnels.md](tunnels.md), "Edge protocol and transport"). `--egress relay` is also a network of
  `podnet forward` and of a router ([egress.md](egress.md)).
* `podnet warp nc --via-relay HOST PORT` and the relay path of the MASQUE connection ([warp.md](warp.md))
  use the same relay.
* `podnet relay connect` and `podnet relay node` take `--detach` ([background.md](background.md)).
* A program that uses the library gives a node a handler in place of a target:
  `relay_reverse::channel(depth)` returns a handler and a receiver. Each
  session that the relay opens is one end of a pipe; the receiver gives the
  other end with the session id. `relay_reverse::run_node_with` runs the node
  over the handler. No socket file is made, and a closed receiver refuses a
  session.
* `podnet doctor` tests the TCP and TLS path to the relay ([doctor.md](doctor.md)).

## SSH forward relay: `podnet ssh-relay`

Some hosted relays give a public URL for a unix socket through an SSH remote forward on port 443 (the
server prints the URL in its banner). `podnet ssh-relay` is the client for them. It publishes a unix
socket and keeps the session: when the relay ends a running session, podnet connects again with a backoff
and prints the new URL. If the first start fails before a session comes up, podnet prints the cause and
exits with no retry: exit 68 when the relay name does not resolve, or exit 69 when the relay or proxy
cannot be reached or does not answer in time.

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
* `--user USER` replaces the SSH user of the facts. podnet tries the methods in this order: a key made for
  the run (kept in memory only), no key (this also reads the list of methods of the relay), and an empty
  answer to keyboard-interactive (up to four rounds), when the relay offers it. It stops at the first
  success. It sends no password. When the relay takes none, the error names the methods that the relay
  offers. podnet checks RSA host keys (the relay Pinggy presents one) and keeps no known-hosts file.
* The banner of the relay can hold more than one URL, and a URL can cross two data frames. podnet reads the
  whole banner, then picks the URL. `--url-match TEXT` selects the tunnel URL: it holds this text (the
  tunnel domain of the relay). `--url-skip TEXT` removes a URL: it holds this text. Both are repeatable,
  and each replaces its list of the facts. The default skip list holds the dashboards of all known relays.
* `--remote-port PORT` (1 to 65535) is the port that the relay opens for the forward. A relay can refuse
  port 0 (the Sandhole demo needs 80). The service data of such a relay has the fact `remote-port`
  (`forward::Relay::remote_port`), and `--service sandhole` asks for port 80 from it. The flag replaces the
  fact. A relay with no fact gets port 0, and the relay chooses. `--server` with no `--service` has no
  fact. `--json` and the ready document show the port of the forward.
* `--socket PATH` is the unix socket of the origin. The relay opens one channel for each visitor, and
  podnet connects it to this socket. The origin must listen on the socket before the first visitor comes.
  The default is `origin.sock` in the private run directory of this user.
* `--connect-proxy HOST:PORT` reaches the relay through an HTTP CONNECT proxy. The default is a direct
  connection. `--proxy`, `--egress` and `--resolve` apply as for the other commands ([proxies.md](proxies.md)).
* `--json` prints each URL as one JSON line: `{"event": "ready" or "reconnected", "url", "remote_port",
  "relay"}`.
* The banner of the relay goes to stderr, with the prefix `[relay]`. `-q` hides it. The line of the URL on
  stdout stays.
* `--run-for SECONDS` stops the command after that time, with exit 0.
* `--detach`, `--name` and the other options of [background.md](background.md) work. The ready document
  has the URL. `podnet ps` shows the new URL after each reconnect.
* `podnet doctor --ssh-relay HOST:PORT --ssh-user USER [--connect-proxy HOST:PORT]` tests the relay with no
  session: the TCP connect (through the proxy of the process, or the given HTTP CONNECT proxy), the SSH banner
  and the login of the user. With no flag, it reads the relay of a running
  `ssh-relay` instance ([doctor.md](doctor.md)).
