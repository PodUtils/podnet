# The link: `podnet link`

This page gives `podnet link`: a link between two installs of podnet over a carrier. A link holds many streams. Each end proves
its Ed25519 key to the other end before the link offers a service. An allow list says which peers may open streams.

## Commands

```sh
podnet link status --create                       # make the key of this install, print its fingerprint
podnet link listen /run/user/1000/app.sock --allow peers.txt --to unix:/run/user/1000/origin.sock
podnet link dial /run/user/1000/app.sock --peer FINGERPRINT-OF-THE-LISTENER      # stdin and stdout over one stream
podnet link dial /run/user/1000/app.sock --allow trusted.txt --check --json      # connect, check, print, exit 0
podnet link listen /run/user/1000/app.sock --allow peers.txt --detach --name app  # in the background
podnet link status                                # the fingerprint, and the link instances that run
podnet link close app                             # stop the instance; the listener removes its socket
podnet link carriers                              # each carrier, and whether this host can use it
podnet components                                 # each carrier and service of this build, with its feature
podnet link connect /run/user/1000/app.sock --peer FINGERPRINT --control /run/user/1000/link.ctl   # hold a link, run a control socket
podnet link status --control /run/user/1000/link.ctl --json                                         # ask that socket
podnet forward -L 8080=peer:FINGERPRINT/http      # a local port to a service of the peer (forward.md)
```

* `link listen ADDRESS --allow FILE` binds the address through the carrier. It prints the address (for the unix carrier: the
  path and its mode, 0600), the fingerprint of this install and the size of the allow list. `--service NAME=TARGET` (repeat it,
  16 at most) sets where the streams of the service NAME go; `--to TARGET` is `--service raw=TARGET`. A TARGET is `host:port` or
  `unix:/path`. `--service socks5` (no target) makes this end the server side of SOCKS5 (see "The service `socks5`"). `--service http` (no target) makes this end an HTTP proxy (see "The service `http`"). `--service NAME=exec:/ABS/PATH` runs a program for each stream (see "The service `exec`"). A stream for a service that the listener does not have is refused (the opener exits 69). A second
  listener at a live address exits 69. A regular file at the address exits 64 and stays. A stale socket file is replaced.
* `link dial ADDRESS` connects, proves the key of this install, and checks the key of the listener. Give `--peer FINGERPRINT`
  (repeat it for more than one) or `--allow FILE`: the dialer trusts no listener by default (else exit 64). Then it copies stdin
  to one stream of the service `--service NAME` (default `raw`), and the stream to stdout. The lines about the peer go to stderr. `--check` stops after the check and prints the
  fingerprint of the peer; with `--json` it prints one document.
* `link connect ADDRESS (--peer FINGERPRINT | --allow FILE)` connects, runs the hello, and holds the link until the link ends
  (exit 69) or a stop signal. It runs the control socket. The ready document has `carrier`, `address`, `fingerprint`, `peer` and
  `control`. It accepts `--detach`, `--name` and the other flags of the background record. `--service NAME=TARGET` (the forms of
  `link listen --service`: a raw target, `socks5`, `http`, `exec:/ABS/PATH` with `--exec-arg`, `--exec-env`, `--exec-user`) serves the
  streams that the peer opens. The grant of the peer in the `--allow` file (`services=`; a `--peer` key has every service) applies before
  any service: a service that it does not grant gets answer 6 and is counted in `refused_service`. A stream for a service that
  this end does not have gets answer 1. With no `--service`, each stream gets answer 1.
* `link status [NAME]` prints the fingerprint of this install and the link instances (`link listen --detach` or `--name`).
  `--create` makes the key when there is none; without it, `status` writes nothing. `--json` prints one document with `identity`
  and `links`. For an instance that runs a control socket, each row has `live` (the document of the verb `status`: the links, with
  the peer, the role, the streams and the counters), or `live: null` and `live_error` when the socket does not answer: the row is
  then the background record. `--control PATH [--token-file FILE]` asks one socket (the `control` of a ready document); a wrong
  token exits 77, and a socket that does not answer exits 69.
* `link close NAME` ends each link that the instance holds with the verb `close-link` of its control socket (one line `closed link
  ID  PEER` for each), then stops the instance (SIGTERM, then SIGKILL after `--timeout`, default 10 s). A socket that does not
  answer is one line on stderr, and the stop goes on. A name that is no link instance exits 64: use `podnet stop`.
* `link carriers [--json]` prints one line for each carrier: its framing, duplex, order, MTU, cost, address form and
  addressing mode, then `available` or `unavailable:` with the reason. The JSON has the same facts, `available` (bool) and
  `reason` (string or null).

`--detach`, `--name`, `--ready-timeout`, `--replace` and `--no-restart` of `link listen` and `link connect` are the flags of the background record
([background.md](background.md)). `podnet ps`, `logs`, `wait` and `stop` work on a link instance. The ready document has
`carrier`, `address`, `listen` (the same address), `mode`, `fingerprint`, `allow`, `to`, `services` (name to target) and `control`.

## Names for live links

`--name NAME` of `link listen` and `link connect` names the instance. For a link, the rules are:

* A name has 1 to 32 characters from `a-z 0-9 -`. Another name exits 64.
* A name is unique among the live instances. A second start with a name that a live instance holds exits 64 (also with
  `--detach`; `--replace` stops the old instance first). At most 64 link instances are live; the 65th exits 64.
* The name is in the background record, so `podnet ps` and `podnet link status NAME` show it.
* A name never replaces the fingerprint. The first dial still needs `--peer` or `--allow` (else exit 64).
* `link connect` binds its name to the fingerprint of the peer that it proved. The file `NAME.peer` in the run directory
  holds it, and it lives as long as the record: a clean stop removes both, so a name is not reused while its record exists.
  A dial under the same name to a peer with another fingerprint exits 77 and keeps the binding. A supervisor that starts
  the instance again checks the same binding.

## The service of a stream

The hello admits a peer for the whole link and binds no stream. The opener of a stream names the service in its first line,
`PNSVC1 NAME` (a name of 1 to 32 characters from `A-Z a-z 0-9 . _ -`), and the other end answers with one byte before any data:
0 open, 1 no such service, 2 the origin of the service did not answer. A byte other than 0 exits 69 at the opener. The default
service is `raw`.

### The service `raw`

The service `raw` copies the bytes of the stream to its target and back. It adds no framing: one byte in is one byte out. Half-close
goes in both directions. The end of the input of one side shuts down the write side of the other side, and the other direction stays
open until its own end. A stream of 4 MiB in each direction arrives unchanged (`tests/link_raw.rs`). The copy has no record size
of its own.

### The service `socks5`

`--service socks5` runs the server side of the SOCKS5 front door (`podnet_netns::door::Door`, the code of `podnet pod --serve`) on the
stream. The opener speaks SOCKS5 on its end, for example `podnet link dial ADDRESS --peer FP --service socks5`. The peer is on the
allow list before the stream opens, so a stranger gets exit 77 and no service runs. `CONNECT` goes from the host of the listener. A name
resolves there. A refused or failed `CONNECT` gets a SOCKS5 failure reply. A stream whose first byte is not 5 (HTTP `CONNECT`, for
example) ends with no answer. `socks5` takes no target: `--service socks5=TARGET` exits 64. There is no user and password: the link
admits the peer. `UDP ASSOCIATE` (command 3) gets reply 7, command not supported: a UDP socket of the peer would be
unreachable for the opener. `BIND` (command 2) listens on a TCP port of the host of the listener: on the loopback, or on the address
of `--socks-bind-ip IP` (a bad address exits 64). Reply 1 gives the address and the port. The first incoming connection (30 s at
most; else reply 6) ends the wait and the listener closes. Reply 2 gives its source, and its bytes pass on the same stream. RFC 1928
puts the incoming connection on the control connection, so the opener needs no second stream. The `BIND` address of the request is
not checked against the source of the incoming connection. UDP uses the command `0x83` of `pod-proxy` (UDP in the stream): after the greeting, the opener sends the
request `05 83 00 01 00 00 00 00 00 00` and gets `05 00 00 01 00 00 00 00 00 00`. Then each datagram in each direction is a frame on
the stream: a length of 2 bytes (big endian), then a SOCKS5 UDP datagram (RFC 1928, section 7). The datagrams leave from one UDP socket
of the host of the listener, and a name in a frame resolves there. The frame is in `docs/proxies.md`, "UDP in the stream". Without
`--listen`, `link dial` carries bytes only: an opener that is a plain SOCKS5 client writes the frames itself.

#### The local door (`link dial --service socks5 --listen ADDRESS`)

`podnet link dial ADDRESS --peer FP --service socks5 --listen 127.0.0.1:1080 [--resolve-at peer|local]` does not copy stdin and
stdout. It listens at `ADDRESS` (`HOST:PORT`, or `unix:/PATH`; no default) and serves the front door (`podnet_netns::door::Door`;
SOCKS5 and HTTP `CONNECT`) to local clients. The line `podnet: socks5 door  ADDRESS` is the first line of stdout. The door runs
until a stop signal (exit 0) or the end of the link (exit 69). `--listen` needs `--service socks5` (else exit 64) and conflicts with
`--check` and `--announce`. The code is `podnet::link_door`.

* `CONNECT`: the door opens one stream of the link for each request, speaks SOCKS5 to the service `socks5` of the peer, and copies the
  bytes. A refusal of the peer is a failure reply to the client (the code is coarse: the door maps the text of the failure).
* Names: `--resolve-at peer` (the default) sends the name in the request, and the peer resolves it. `--resolve-at local` resolves the
  name at the opener (the system resolver, and the pins of the global `--resolve`) and sends an address. The door tries each
  address in the order of the resolver, on a new stream for each, until the peer accepts one. The failure of the last address
  goes to the client. The flag is not called
  `--resolve`: that name is the global flag of the pins.
* `BIND`: the door opens a stream, sends the request, and copies both replies and the bytes of the incoming connection. The listener
  is at the peer (see above).
* `UDP ASSOCIATE`: the door opens a UDP socket for the client (the address of the reply) and one stream with the command `0x83` of the
  peer. Each datagram of the client is a frame on the stream, and each answer is a frame back. A name in a datagram resolves at the
  opener (the datagrams of a way take an address). The association ends when the client closes the control connection, after 300 s
  with no datagram, or when the link ends. The queue is 256 datagrams: a datagram for a full queue is dropped.
* The door checks no user: it serves each local client that can reach `ADDRESS`. Use a loopback address, or a unix socket.

Proof: `tests/link_door.rs`, `tests/link_socks5.rs`.
`link dial` exits at once when the far end ends the stream, also when stdin is still open.

### The service `http`

`--service http` runs an HTTP proxy on the stream (`podnet_netns::door::Door`, the same code as `podnet pod --serve`). The opener sends
one request for each stream, for example `podnet link dial ADDRESS --peer FP --service http`. Two framings work: `CONNECT NAME:PORT`
(the reply is `200`, then the bytes pass) and the absolute form `GET http://NAME/path HTTP/1.1` (the request goes on in origin form with
`Connection: close`; the reply and the body pass). The peer is on the allow list before the stream opens, so a stranger gets exit 77 and
no service runs.

* A target is the name of a service of the listener (`--service NAME=TARGET`; `--to` is `raw`). The port of the request is not used:
  the name stands for the target of the service. A host that is no such name, and each IP address, gets 403. The proxy does no DNS lookup.
* The request head is 8 KiB at most (else 431). A `CONNECT` with a body (`Content-Length` other than 0, or `Transfer-Encoding`) gets 400. A
  request that is neither `CONNECT` nor absolute form (`GET /x`) gets 400. A target that does not answer gets 502 or 504.
* There is no cache and no user and password. `http` takes no target: `--service http=TARGET` exits 64.

Proof: `tests/link_http.rs`.

### The service `exec`

`--service NAME=exec:/ABS/PATH` runs one program for each stream of the service NAME. The stdin and the stdout of the program are the
stream. The stderr of the program goes to the log (`tracing`, one record for each line), never to the stream. The peer is on the allow
list before the stream opens.

```sh
podnet link listen ADDRESS --allow FILE \
  --service tool=exec:/usr/local/bin/tool --exec-arg tool=--stdio --exec-arg tool=--quiet \
  --exec-env tool=LANG --exec-user tool=nobody
podnet link dial ADDRESS --peer FP --service tool
```

* The program is an absolute path to a file. A relative path, a `..` in the path, a missing file, and a program named `sh`, `bash`,
  `dash`, `zsh`, `ash`, `ksh`, `mksh`, `csh`, `tcsh`, `fish` or `busybox` exit 64. No shell runs: the arguments are an array.
* `--exec-arg NAME=ARG` adds one argument (repeat it for each argument, in order; 64 at most, 4096 bytes each). No word splitting or
  expansion takes place.
* The environment is empty. `--exec-env NAME=VAR` passes the variable VAR of the listener (repeat it; 32 at most). A variable that the
  listener does not have is left out. The working directory is `/`.
* `--exec-user NAME=USER` runs the program as USER, with no supplementary groups. It needs root: without root the listener exits 64.
* The flags `--exec-*` for a service that has no `exec:` target exit 64.
* At most 64 programs run at one time. A stream over the limit gets the answer 4 (the opener exits 69). A program that cannot start
  gets the answer 2 (the opener exits 69).
* When the program closes its stdout, the stream ends, and the program has 10 s to end (else it is killed). When the peer ends its side,
  the stdin of the program closes. A reset stream kills the program. A program that ends with a fault is logged.

The file server serves a tree at a peer with `podnet serve DIR --at peer:FP/SERVICE --at-listen ADDRESS` (see `docs/serve.md`, "Serve at a peer"), or, through a running `link connect`, with `podnet serve DIR --ingress peer:FP/SERVICE`.

Proof: `tests/service_exec.rs`. Not proven: `--exec-user` as root (the test runs without privilege and checks the refusal).

## The control socket

`link listen` and `link connect` run a control socket for the links they hold. It is a unix socket with the mode 0600, at
`--control PATH` (default `link-PID.ctl` in the run directory; a host with no run directory runs with none, and the ready
document says `control: null`). The token is in `PATH.token` (mode 0600): 32 random bytes in hex, new for each run. The instance
removes both files at its stop. The token is never in the ready document, `status`, `ps` or the log.

* One JSON object for each line, not HTTP. A request is `{"token":T,"verb":V,...}` of 4 KiB at most. The answer is
  `{"ok":true,...}` or `{"ok":false,"code":EXIT,"error":TEXT}` of 64 KiB at most. A client has one command at a time. 16
  clients are served at one time; the 17th gets `code` 69. There is no TCP listener and no HTTP endpoint.
* The user of the peer (`SO_PEERCRED`) must be the user of the instance, or root (the rule of the router). Another user gets 77.
  A wrong or missing token gets 77, and the connection closes.
* Verbs:
  * `status`: `fingerprint`, `pid` and `links`, a row for each held link: `id`, `carrier`, `address`, `role`
    (`listener` or `dialer`), `peer` (the fingerprint), `state` (`up` or `ended`), `streams`, `rtt_ms`, `opened` and `accepted`
    (the streams opened here and by the peer), `refused_service` and `refused_forward` (the streams that the allow list
    refused, T-290), `since`, and `stream_ids` (the streams that a task serves, 64 at most), and at the
    top `selected` (the pinned link, or `null`). It has no seed and no token.
  * `peer` (`"peer":FINGERPRINT`): `ok` when a link that is up goes to that peer; 77 when the links go to other peers; 69 when
    the instance holds no link that is up.
  * `open` (`"peer":FINGERPRINT,"service":NAME`): opens one stream, asks for the service, and on `ok` the connection carries the
    bytes of the stream. The network `peer:FINGERPRINT/SERVICE` uses it.
  * `accept` (`"peer":FINGERPRINT,"service":NAME,"target":"unix:/ABS/PATH"`): serve the streams for NAME that the peer opens on the
    unix socket at PATH, while the client keeps the connection (`podnet serve --ingress peer:FP/NAME` uses it). The instance must
    hold a link to the peer (69 or 77 as for `open`). A name that `--service` has, or that another client holds, exits 64. The
    grant of the peer applies to the name. No other target form is taken.
  * `close-link` (`"link":ID`): end the link with a GOAWAY after the frames that wait. A link that has ended is not an error.
  * `close-stream` (`"link":ID,"stream":ID`): reset one stream. The other streams and the link stay. The ids are the
    `stream_ids` of the row of `status` (64 ids at most for each link).
  * `select` (`"link":ID` or `"link":null`): pin the link that serves the streams opened from now on (the verb `open`, so
    `peer:FINGERPRINT/SERVICE`); `null` returns to the first link that is up. The streams that run stay where they are. The
    pin ends with its link. A link that has ended exits 69. `status` shows the pin as `selected`.
  * Each of the three answers the document of `status` after the change, with `verb` set to its name. An id that is missing,
    is no number or names no link or stream gets 64. They need the token and the user check, like every verb: a refused
    request changes nothing.
  * `reconnect` (`"link":ID`): reset the streams of the link (the ids are in `reset_streams`), end the link, and dial again
    with the same key, carrier, address, allow list and settings. A reconnect always makes a new session (the resume of
    `session` is not used by `link connect`), so the streams that ran end with a reset. The dial must reach the peer that the
    old link proved; another key exits 77, and a name keeps its fingerprint (`--name`). When the dial fails, no link is left
    and the instance exits 69.
  * `rotate` (`"link":ID`): dial a new link first (make before break), pin it for the streams that open from now on (a pin on
    another link stays), and end the old link when its streams end. No stream fails. The old link stays up while a stream
    runs on it. The same key check as `reconnect` applies, and a failed dial leaves the old link as it was.
  * `reconnect` and `rotate` answer the document of `status` after the change, with `link` (the id of the new link) and
    `replaced` (the id of the old link). An instance that listens (`link listen`) does not dial: both verbs exit 69 with
    `a listener does not dial`. `link reconnect NAME [--link ID]` and `link rotate NAME [--link ID]` call the verbs for each
    link that is up, and print one line `VERB link OLD -> NEW  PEER`.
  * `dump`: the state of the instance for a bug report. It has `build` (`version`, the optional `features` that this binary
    holds, and the `carriers` with `feature` and `built`), `fingerprint`, `pid`, `settings` (`selected` and the `limits`),
    `totals` (`links`, `up`, `streams`, `opened`, `accepted`), `links` (the row of `status`, and `stream_table`: one
    `{id, owner}` for each stream, `owner` being `local` or `peer`, and `streams_cut`), the last 20 `errors` (`at`, `text`; a text
    keeps 512 characters), `routes` and `cut`. With `--announce`, `routes` is the mesh view of the verb `mesh` (`built: true`, `node`, `limits`, `counters`, `truncated`, `nodes` with the edges; at most 96 KiB, and the rows of the stream tables set its nodes and edges when the dump is cut). Without `--announce`, `routes` is `{built: false, reason}`. It has no seed, no token
    and no secret of a peer: every string passes `report::mask` (credentials in URLs, `Authorization` values, runs of 32 or more
    hex characters, the token of the instance, the text of the key file) and `report::clean` (control characters). Its answer
    may be 256 KiB; every other answer is 64 KiB at most. A table that does not fit is cut from its last rows, and `streams_cut`
    (for a link) and `cut.stream_rows` and `cut.links` (at the top) count the rows that the dump cut; the counters stay whole.
    `podnet bug-report --include-link` adds the dump of each link instance that runs.
  * An unknown verb gets 64 and the list of the verbs.

`PODNET_LINK_CONTROL` names the one socket that `peer:FINGERPRINT/SERVICE` asks. Without it, the network asks the control socket
of each link instance in the run directory, and takes the first that holds a link to the peer.

## Link groups

A link group is a group of the router engine ([router.md](router.md)) whose members are
`peer:FINGERPRINT/SERVICE`. Put the members in the `networks` and the `groups` of a router file, or build
one in code with `podnet::link_group::LinkGroup`.

* A group has 1 to 16 members. A weight is 1 to 100. The probe interval is 1 to 60 s.
* The policies are `weighted` (default), `rr`, `minrtt` and `leastload`. `minrtt` moves its head only for
  `max(5 ms, 10%)` over three probe rounds.
* A flow takes one member when it opens. The stream stays on that member.
* The control probe asks the control socket whether the link to the peer is up. A member that fails it is
  down at once.
* The data probe opens one stream to the service of the member. A member that fails it is down after
  `down_after` failures in a row (default 4). A member that is down goes through probation when a probe
  passes.
* When each member is down, the flow is refused with the class `policy`. There is no direct fallback.

## Flags of the carriers

* `--carrier NAME` selects the carrier (default `unix`). An unknown name exits 64 and lists the names.
* `--allow-other-uid` (unix carrier) accepts a peer of another uid. Each end checks the uid of the other end with
  `SO_PEERCRED`, so give the flag at both ends. Without it, a peer of another uid exits 69.
* `--dns-server IP:PORT` and `--dns-zone ZONE` (dns carrier): the server of the queries of a dial, and the zone of a listener. With
  another carrier they exit 64.
* `--allow-raw icmp` allows a carrier that opens a raw socket. The `icmp` carrier without it exits 64 and names the flag.
  Another value exits 64.
* `--ring BYTES` (memory carrier) sets the ring of each direction: 65536 to 67108864 (64 KiB to 64 MiB). Default 1048576. A
  value out of the limits exits 64.

## Query keys of an address

The address of `link listen`, `link dial` and `link connect` can end in a query: `ADDRESS?key=value&key=value`. The
code is `src/link_query.rs`. The command checks the query first, before it opens a socket.

| Key | Value | Effect |
| --- | --- | --- |
| `carrier` | a carrier name | Takes the place of `--carrier`. |
| `tls` | `on`, `off` | T-269 adds it. |
| `retry` | 0 to 16 | T-263 adds it. |
| `lb` | `weighted`, `rr`, `minrtt`, `leastload` | T-288 adds it. |
| `compress` | `none`, `snappy` | Compresses the payload of each record with snappy before the seal (T-299). It needs `--seal` (else exit 64). |
| `pad` | `on`, `off` | Pads each record to a bucket (T-300). It needs `--seal` (else exit 64). |
| `mtu` | 64 to 65535, at most the unit of the carrier | The `udp` carrier: the largest datagram of KCP, 64 to 1200 bytes (T-276). Another stream carrier has no unit: exit 64. |
| `maxhops` | 1 to 8 | T-287 adds it. |
| `poll` | 1 to 60 (seconds) | The poll carriers: the hold of a long poll that the dialer asks for (default 25 s; T-271). Another carrier: exit 64. |
| `window` | 16384 to 8388608 (bytes) | T-262 adds it. |
| `name` | 1 to 32 characters of `a-z 0-9 -` | No effect in a query: give the name with `--name` (T-308). |
| `kcp-nodelay` | `0`, `1` | The `udp` carrier: `1` turns on the nodelay of KCP with a window of 1024 segments (T-276). Another carrier: exit 64. |

* A key with no effect now is parsed and checked. Then the command exits 64 and names the entry that adds it.
* Each key is given once. A repeated key exits 64.
* An unknown key exits 64 and lists the allowed keys. The message does not show the unknown key.
* `password`, `key`, `token` and `identity` are refused with or without a value (exit 64).
* No message shows a value.
* A URL has 1024 bytes at most and 16 keys at most. A value is printable ASCII with one level of percent-encoding.
  A decoded value with `%` exits 64.
* A path of the unix carrier that holds `?` cannot be used: the first `?` starts the query.

## Carriers

| Name | Address | What it is |
| --- | --- | --- |
| `unix` | a path (1 to 107 bytes) | A stream over `AF_UNIX`. The socket has the mode 0600. The uid of the peer is checked. It needs no network. |
| `memory` | a name | Two ends in one process, with a ring for each direction. A full ring blocks the writer. It is for tests and for programs that embed the link. |
| `tcp` | `KEY@HOST:PORT` to dial, `IP:PORT` to listen | TCP with TLS 1.3 and a pinned Ed25519 key (T-268). Build feature `tcp`. See "The TCP carrier". |
| `ws` | `KEY@wss://HOST[:PORT]/PATH` to dial, `IP:PORT` to listen | A WebSocket over TLS 1.3 and a pinned Ed25519 key (T-269). Build feature `ws`. See "The WebSocket carrier". |
| `h2` | `KEY@https://HOST[:PORT]/PATH` to dial, `IP:PORT` to listen | One POST of HTTP/2 over TLS 1.3 and a pinned Ed25519 key (T-270). Build feature `h2`. See "The HTTP/2 carrier". |
| `quic` | `KEY@HOST:PORT` to dial, `IP:PORT` (UDP) to listen | One stream of a QUIC connection with TLS 1.3 and a pinned Ed25519 key (T-273). Build feature `quic`. See "The QUIC carrier". |
| `udp` | `KEY@HOST:PORT` to dial, `IP:PORT` (UDP) to listen | KCP over UDP with TLS 1.3 and a pinned Ed25519 key (T-276). Build feature `udp`. See "The UDP carrier". |
| `poll` | `KEY@https://HOST[:PORT]/PATH` to dial, `IP:PORT` to listen | A long poll of HTTP/1.1 over TLS 1.3 and a pinned Ed25519 key (T-271). Build feature `poll`. See "The poll carriers". |
| `sse` | as `poll` | POSTs up, one answer of server-sent events down (T-272). Build feature `poll`. |
| `chunked` | as `poll` | POSTs up, one chunked answer down (T-272). Build feature `poll`. |
| `httppoll` | as `poll` | POSTs up, a GET poll down: the simplex poll, the highest overhead (T-280). Build feature `poll`. |
| `dns` | `KEY@ZONE` to dial (`--dns-server`), `IP:PORT` (UDP) to listen (`--dns-zone`) | Query names up and TXT answers down, an ARQ and TLS 1.3 with a pinned Ed25519 key above (T-278). Build feature `dns`. See "The DNS carrier". |
| `s3` | `BUCKET/PREFIX` | A simplex pair of objects in a bucket (T-279). Build feature `s3`. |
| `icmp` | `KEY@HOST` (IPv4) to dial, an IPv4 address with no port to listen | The data of ICMP echo messages on a raw socket, an ARQ and TLS 1.3 with a pinned Ed25519 key above (T-282). Build feature `icmp`; it needs `CAP_NET_RAW` and `--allow-raw icmp`. See "The ICMP carrier". |

The carriers `tcp`, `ws`, `h2`, `quic`, `udp`, `poll`, `sse`, `chunked`, `httppoll`, `dns`, `s3` and `icmp` sit behind a build feature ([build.md](build.md), Features). The default build has none of
them. `podnet link carriers` lists each one with `unavailable: not built: the feature NAME is off (build with --features NAME)`
(the JSON has `built: false`, `feature` and `reason`, and no facts that a carrier would give), and a command that names the
carrier exits 69 with that reason. In a build with the feature `s3` on, the row says that the code of the carrier is not in
this build, and names its entry (T-279).

`podnet components [--json]` lists each carrier and each service of this build (T-310). A row has the fields `name`, `kind`
(`carrier` or `service`), `framing`, `duplex`, `needs_privilege`, `available`, `reason`, `cost`, `feature`, `built`, `about` and
`address`. The names are stable, the list has at most 64 rows, and a row has at most 4 KiB in JSON. A row of a feature that this
build leaves out has `available: false`, `built: false` and the reason; its `framing` and `duplex` are `null`. The services are
`ss` and `trojan` (features `ss` and `trojan`). The JSON also has `features` (each optional feature and whether it is built),
`excluded` (each feature that is off, with its reason) and `portable`. `podnet man inventory` is the section of the manual for
each row, and `tests/cli/manual.rs` requires a section for each row that the build holds.

## The TCP carrier

Build with `--features tcp`. `pod-link` of PodUtils/podcrates does the work (`pod_link::tcp`); `podnet::link_tcp` is the glue.

* TLS 1.3 only. The certificate holds an Ed25519 key only. The ALPN is `podlink/1`. There is no resumption.
* Each end pins the key of the other end in the TLS handshake. The key is not proved by the hello: the hello (`podnet::hello`) does not run
  on this carrier. The streams of the link run inside the TLS session that the pinned keys made.
* A listener binds `IP:PORT` and takes a client only when its key is in the allow list (`--allow`, default deny). The list feeds
  `with_clients` of the carrier, and a change of the set applies at the next handshake. At most 64 handshakes run at once, each
  for at most 10 s, so a peer that sends nothing holds no other peer.
* A dialer gives `KEY@HOST:PORT`. `KEY` is the key of the listener: the display fingerprint that `link listen` prints, a `SHA256:`
  fingerprint, or the key in hex or base32. The dial goes through the chain of `--egress` elements (see "A chain before the carrier"), or, with none, through the way out of the process (`--proxy`). It ends in 20 s. The key
  that the listener proves must also be in the allow list of the dialer (`--peer` or `--allow`).
* A key that is not pinned exits 77, at the dialer and at the listener. The listener names the key that it refused on stderr and keeps
  its door open. A host that denies the socket (`EACCES`) exits 69: it is not a refused key.
* There is no plaintext mode. The carrier always runs TLS, also on a loopback bind. So the rule "a public bind with `--plaintext` exits
  68" has no case, and `link listen` has no `--plaintext` flag.

```sh
podnet link listen 0.0.0.0:7000 --carrier tcp --allow allowed.txt --to 127.0.0.1:22
podnet link dial SERVER-FINGERPRINT@host.example:7000 --carrier tcp --allow servers.txt
```

Each carrier reports `framing`, `duplex`, `ordered`, `mtu`, the cost and `available()`. A carrier that this host cannot use
gives its reason, and a command that needs it exits 69 with the reason. `podnet doctor` has a row `carriers` with the same
diagnosis. The addressing mode of each carrier is `peer`: the address names a peer, not a destination, so a consumer of
destinations does not take it.

## The WebSocket carrier

Build with `--features ws` (it holds `tcp`). `pod-link` of PodUtils/podcrates does the work (`pod_link::ws`, with the accept loop of
`pod_ws::serve`); `podnet::link_ws` is the glue. It has the same calls as `podnet::link_tcp`.

* A WebSocket (RFC 6455) runs over TLS 1.3 with the ALPN `http/1.1`. The bytes of the link go in binary messages. Each end pins the key
  of the other end in the TLS handshake, as the `tcp` carrier does. The hello does not run on this carrier.
* A listener binds `IP:PORT` and takes any path of the request. It takes a client only when its key is in the allow list (`--allow`,
  default deny). The listener sends the 101 after it checked the key, so a refused key is an error of the dial. The accept loop
  limits the time of each stage and the count of the connections that wait.
* A dialer gives `KEY@wss://HOST[:PORT]/PATH`. The request goes to `PATH` with the `Host` of the URL, so a front that routes by path
  or by name and does not end the TLS can carry it. The dial goes through the way out of the process (`--proxy`). The key that the
  listener proves must also be in the allow list of the dialer (`--peer` or `--allow`). `--egress` is for the `tcp` carrier only.
* A key that is not pinned exits 77 at the dialer. The accept loop of the listener cuts a connection that fails a stage and goes on, so the listener gets no error for it and keeps its door open. A host that denies the socket exits 69.
* The subprotocol is `podnet/1` (GitHub #467). The dial offers it; a listener refuses a dial without it (426) and names it in the 101.
  A text message ends the link (Close 1003). Each end sends a Ping each 20 s and ends the link when no frame comes within 10 s of it.

```sh
podnet link listen 0.0.0.0:7000 --carrier ws --allow allowed.txt --to 127.0.0.1:22
podnet link dial SERVER-FINGERPRINT@wss://host.example:7000/link --carrier ws --allow servers.txt
```

## The HTTP/2 carrier

Build with `--features h2` (it holds `tcp`). `pod-link` of PodUtils/podcrates does the work (`pod_link::h2`); `podnet::link_h2` is the
glue. It has the same calls as `podnet::link_tcp`.

* The dialer opens one POST of HTTP/2 over TLS 1.3 with the ALPN `h2`. The request body carries the bytes of the dialer and the
  response body carries the bytes of the listener. Both stay open for the life of the link. END_STREAM is the half close.
* Each end pins the key of the other end in the TLS handshake. The listener sends its SETTINGS after it checked the key of the dialer,
  so a refused key is an error of the dial. The hello does not run on this carrier.
* A listener binds `IP:PORT` and takes a client only when its key is in the allow list (`--allow`, default deny). At most 64
  handshakes run at once, each for at most 10 s.
* A dialer gives `KEY@https://HOST[:PORT]/PATH`. The dial goes through the way out of the process (`--proxy`). The key that the
  listener proves must also be in the allow list of the dialer. `--egress` is for the `tcp` carrier only.
* A key that is not pinned exits 77. A host that denies the socket exits 69.

```sh
podnet link listen 0.0.0.0:7000 --carrier h2 --allow allowed.txt --to 127.0.0.1:22
podnet link dial SERVER-FINGERPRINT@https://host.example:7000/link --carrier h2 --allow servers.txt
```

## The QUIC carrier

Build with `--features quic` (it holds `tcp`). `pod-link` of PodUtils/podcrates does the work (`pod_link::quic`, over `pod-quic`);
`podnet::link_quic` is the glue.

* One bidirectional stream of a QUIC connection carries the link. TLS 1.3 with the ALPN `podlink/1` (as the `tcp` carrier), no
  resumption, no 0-RTT. Each end pins the key of the other end in the TLS handshake. The hello does not run on this carrier.
* A listener binds `IP:PORT` on UDP and takes a client only when its key is in the allow list (`--allow`, default deny). After its
  handshake the listener opens the stream, so a refused key is an error of the dial. At most 64 handshakes run at once, each for at
  most 10 s.
* A dialer gives `KEY@HOST:PORT`. The dial sends from a UDP socket of the system: the way out of the process (`--proxy`) carries
  no datagrams, and `--egress` is for the `tcp` carrier only. The key that the listener proves must also be in the allow list of the
  dialer (`--peer` or `--allow`).
* The connection ends after 30 s of silence; each end sends a keepalive each 10 s.
* A key that is not pinned exits 77. A host that denies UDP sockets exits 69, and `podnet link carriers` gives the reason.

```sh
podnet link listen 0.0.0.0:7000 --carrier quic --allow allowed.txt --to 127.0.0.1:22
podnet link dial SERVER-FINGERPRINT@host.example:7000 --carrier quic --allow servers.txt
```

## The UDP carrier

Build with `--features udp` (it holds `tcp`). `pod-link` of PodUtils/podcrates does the work (`pod_link::udp`, KCP of the crate
`kcp`); `podnet::link_udp` is the glue.

* KCP runs in stream mode over UDP: datagrams of 1200 bytes at most, segments of 1 KiB (no IP fragmentation), a window of 256
  segments, the congestion window of KCP. TLS 1.3 with the ALPN `podlink/1` runs above KCP, and each end pins the key of the other
  end in the handshake. TLS seals each byte, so a datagram of a stranger can end a link but not change it.
* A listener binds `IP:PORT` on UDP and takes a client only when its key is in the allow list (`--allow`, default deny). A dialer
  gives `KEY@HOST:PORT` and sends from a UDP socket of the system: the way out of the process (`--proxy`) carries no datagrams, and
  `--egress` is for the `tcp` carrier only.
* The session ends after 20 s of silence of the peer.
* The query key `kcp-nodelay=1` turns on the nodelay of KCP (an RTO of 30 ms at least, no congestion window) with a window of 1024
  segments. It is not the default: it is unfair to the other flows of a path. It recovers faster from a high loss. Give it at
  both ends. The query key `mtu=N` sets the largest datagram (64 to 1200 bytes).
* A key that is not pinned exits 77. A host that denies UDP sockets exits 69: the carrier sends no datagram, and `podnet link
  carriers` gives the reason.

```sh
podnet link listen '0.0.0.0:7000?kcp-nodelay=1' --carrier udp --allow allowed.txt --to 127.0.0.1:22
podnet link dial 'SERVER-FINGERPRINT@host.example:7000?kcp-nodelay=1' --carrier udp --allow servers.txt
```

## The poll carriers

Build with `--features poll` (it holds `tcp`). `pod-link` of PodUtils/podcrates does the work (`pod_link::poll`); `podnet::link_poll`
is the glue. The link goes in requests of HTTP/1.1 over TLS 1.3 with the ALPN `http/1.1`, and each end pins the key of the other end in
the TLS handshake of each connection. No request lives as long as the link, so a proxy that ends idle connections, or a front of
HTTP/1.1 that does not end the TLS, can carry it.

| Carrier | Bytes down | Overhead | Entry |
| --- | --- | --- | --- |
| `poll` | A POST `PATH/down?since=N` that the listener holds up to the hold (default 25 s; the query key `poll=1..60`). | medium | T-271 |
| `sse` | One answer of `GET PATH/push?since=N` as server-sent events: base64, 8 KiB for each event, a `: ping` comment each 15 s. | high | T-272 |
| `chunked` | One chunked answer of `GET PATH/push?since=N`: the raw bytes. | medium | T-272 |
| `httppoll` | A `GET PATH/out?since=N` at once (204 when the listener has nothing), again after a backoff of 100 ms to 5 s. 64 KiB for each request. | highest | T-280 |

* The dialer opens a session with `POST PATH/open?s=ID` (201). The answer comes after the listener checked the key of the dialer, so
  a refused key is an error of the dial. The bytes up go in `POST PATH/up?s=ID&seq=N&ack=M` (202 and `Pod-Ack`), one in flight, at
  most 256 KiB (64 KiB for `httppoll`). The offsets make a request again after a lost answer give no byte twice.
* A listener binds `IP:PORT` and serves each form, so one listener takes a dialer of each carrier name. It takes a client only when
  its key is in the allow list (`--allow`, default deny). It holds at most 256 sessions and 64 MiB down for its sessions; past that it
  answers 429. The third 429 in a row ends a dial. A session with no request for 60 s ends.
* A listener that refuses a push answers 406, and a dialer of `sse` or `chunked` takes the long poll.
* A dialer gives `KEY@https://HOST[:PORT]/PATH`; PATH has no query. The dial goes through the way out of the process (`--proxy`).
  A lane that loses its connection connects again with a backoff of 100 ms to 5 s; a lane that fails for 60 s ends the link.
* `podnet link carriers --json` gives the overhead class of each carrier (`overhead`: `low`, `medium`, `high` or `highest`).
* A key that is not pinned exits 77. A host that denies the socket exits 69.

```sh
podnet link listen 0.0.0.0:7000 --carrier poll --allow allowed.txt --to 127.0.0.1:22
podnet link dial 'SERVER-FINGERPRINT@https://host.example:7000/link?poll=50' --carrier poll --allow servers.txt
podnet link dial SERVER-FINGERPRINT@https://host.example:7000/link --carrier sse --allow servers.txt
```

## The DNS carrier

Build with `--features dns` (it holds `tcp`). `pod-link` of PodUtils/podcrates does the work (`pod_link::dns`, with the TXT codec of
`pod-dns`); `podnet::link_dns` is the glue. It is a carrier of the last resort for a host whose only way out is DNS.

* The dialer sends each datagram up in base32 labels of a query name under the zone (253 bytes at most, labels of 63 bytes), and reads
  the datagrams down from the TXT records of the answer (1232 bytes with EDNS0). It starts at most 20 queries a second, 8 in flight,
  and polls with a backoff of 5 ms to 2 s when it has nothing to send, so the listener can send its bytes down.
* The queries lose, reorder and repeat datagrams, so the selective-repeat ARQ of `pod-link` runs over them, and TLS 1.3 with the ALPN
  `podlink/1` over the ARQ. Each end pins the key of the other end in the TLS handshake; a query of a stranger can end a link but not
  change it.
* A listener is the authoritative server of the zone `--dns-zone ZONE` on UDP `IP:PORT` (a listener with no zone exits 64). Delegate
  the zone to it (an `NS` record) so that a resolver sends it the queries. It takes a client only when its key is in the allow list.
* A dialer gives `KEY@ZONE` and sends its queries to `--dns-server IP:PORT` (a resolver or the listener itself), else to the first
  `nameserver` of `/etc/resolv.conf` at port 53. With neither, the dial exits 64.
* `--dns-server` and `--dns-zone` with another carrier exit 64. The overhead class is `highest`.
* A key that is not pinned exits 77. A host that denies UDP sockets exits 69.

```sh
podnet link listen 0.0.0.0:53 --carrier dns --dns-zone t.example.com --allow allowed.txt --to 127.0.0.1:22
podnet link dial SERVER-FINGERPRINT@t.example.com --carrier dns --allow servers.txt
```

## The ICMP carrier

Build with `--features icmp` (it holds `tcp`). `pod-link` of PodUtils/podcrates does the work (`pod_link::icmp`);
`podnet::link_icmp` is the glue. It is a carrier for a host whose only way out is ICMP echo.

* Each datagram rides in the data of one ICMP echo message: the dialer sends echo requests, the listener sends echo replies. The
  payload is 1400 bytes. A session id in the data tells one link from another on one socket, and a mark tells the bytes of the
  dialer from the bytes of the listener. A message of another session, and the reply of the kernel to a request, are dropped.
* The echoes lose, reorder and repeat datagrams, so the selective-repeat ARQ of `pod-link` runs over them, and TLS 1.3 with the
  ALPN `podlink/1` over the ARQ. Each end pins the key of the other end in the TLS handshake; an echo of a stranger can end a link
  but not change it. The overhead class is `high`.
* The socket is a raw ICMPv4 socket (`SOCK_RAW`). It needs `CAP_NET_RAW` or uid 0; the binary is not setuid. A datagram ICMP
  socket (`ping_group_range`) cannot send an echo reply, so the carrier does not use one. IPv4 only.
* Give `--allow-raw icmp` at each end. Without it the command exits 64 before a socket opens.
* The checks come in this order: the build feature (exit 69), `--allow-raw icmp` (exit 64), the address (exit 64), the raw socket
  (exit 69, with the reason `a raw ICMP socket needs CAP_NET_RAW or uid 0`), the key (exit 77).
* A listener takes an IPv4 address with no port (`0.0.0.0`, `127.0.0.1`); a port exits 64. Each listener on one address takes
  each echo request to that address: run one listener for each address. A dialer gives `KEY@HOST`.
* The kernel of the listener host answers each echo request too. Set `net.ipv4.icmp_echo_ignore_all=1` there to stop the second
  reply; the dialer drops it in each case.
* `podnet doctor` (the `carriers` rows) and `podnet link carriers` say whether a raw ICMP socket opens on this host.

```sh
podnet link listen 0.0.0.0 --carrier icmp --allow-raw icmp --allow allowed.txt --to 127.0.0.1:22
podnet link dial SERVER-FINGERPRINT@203.0.113.7 --carrier icmp --allow-raw icmp --allow servers.txt
```

## The ARQ under a link

`pod_link::arq` of PodUtils/podcrates is the selective-repeat ARQ: it makes one ordered byte stream of datagrams that a carrier can
lose, reorder or repeat. The carriers `dns` and `icmp` run it inside `pod-link`, under their TLS. The `udp` carrier runs KCP, its
own ARQ, and not this one. `podnet::link_arq` is the glue for a carrier of datagrams that has no ARQ (a
`pod_link::datagram::Datagrams` of a program that embeds podnet). It has no command. (T-277)

* A write of the link is one record. The ARQ cuts a record into fragments of 1 KiB at most. Each fragment has a 32-bit sequence, a
  16-bit index and the count of the record. Each datagram fits the unit of the carrier.
* An `ACK` has a bitmap of 256 frames. At most 256 frames are in flight. The ARQ sends each lost fragment again by itself. The RTO
  is 200 ms to 8 s. A frame gets 8 retries.
* The ARQ is below the link and knows no stream. A stream that ends or that its peer resets closes alone; the link and its other
  streams go on. When a frame of the link gets no acknowledgement after its 8 retries, the carrier is gone: the ARQ ends with
  `arq-timeout`, and the link ends with it (exit 124). The keepalive of the link (30 s) can end the link first.
* The sequence does not wrap to 0 with no rekey. The ARQ asks for a rekey before the wrap, and new frames wait at the wrap until
  the rekey is done. `Rekey::Above` lets the sequence wrap: the TLS of a carrier and the record layer of a sealed stream seal each
  byte with their own nonces, and the sequence of the ARQ is not one of them. `Rekey::With(hook)` runs the hook first; a hook that
  fails keeps the frames at the wrap, and the link ends.
* `podnet::link_arq::arq_fault` gives an error of the ARQ its exit code: a setting out of its limits 64, `arq-timeout` 124, a
  broken protocol 65, a failed carrier 69.

## A chain before the carrier

`podnet link dial` and `podnet link connect` take each `--egress SPEC` as one element of a chain before the `tcp` carrier. Only the
`tcp` carrier opens a network connection: a chain with another carrier exits 64. A `--egress` beside `--proxy`, `--no-proxy`,
`--transport` or `--relay` exits 64.

* The elements apply in order. The first element is the first hop from this host. The last element reaches the carrier address.
  There is no `direct` at the end: give `direct` as the first element if you want it.
* An element is `direct` (first only), a proxy (`socks5://`, `socks5h://`, `http://`; a proxy on a unix socket is first only), or
  `peer:FINGERPRINT/SERVICE` (first only). A `peer:` stream is the connection to the next element or to the carrier address, so
  the service of the peer must lead there ([egress.md](egress.md)). Each other network of the registry exits 64.
* The chain is a `Chain` of `podnet::network`: each element comes up over the way of the element below it.
* At most 8 elements (else exit 64) and 10 s for each element, to come up and to dial. A duplicate element, two elements at one place
  (`socks5://H:1` and `socks5h://H:1`), or an element at the place of the carrier address (a cycle) exits 78.
* A chain that cannot be built (a name that does not resolve, a unix socket that is not there, no running link for a `peer:` element)
  exits 69 and names the element with its number. It exits before the carrier opens. The line shows no password.
* `--check` resolves each element and dials it once, then connects and checks the peer, prints the result and ends. The dial of
  element `n` goes through elements 1 to `n` to the place of element `n + 1` (the last one: to the carrier address) and ends at once.
  Each line is `podnet: element N of M  SPEC  resolved ADDRESS  dialed in T ms`; with `--json` the document has `elements`. A
  failed element exits 69 and is named. `link connect --check` holds no link, starts no control socket, takes no name and leaves
  no listener.

```sh
podnet link dial KEY@host.example:7000 --carrier tcp --allow servers.txt \
  --egress socks5://127.0.0.1:1080 --egress socks5://hop2.example:1080 --check
```

## Identity and allow list

* The key of an install is one Ed25519 key in `<state directory>/link-identity.key`, made with the mode 0600 on first use. A
  key file with a wider mode exits 71. A missing key, where a command needs an existing one, exits 66. The seed is never
  printed, never logged and never in JSON.
* The fingerprint is base32 in five groups with a checksum. `podnet link status` prints it. The OpenSSH form (`SHA256:...`) and
  a key in hex are also accepted in a file.
* An allow list is a text file: one peer on each line. A word after the peer is a comment, and a line that starts with `#` is a
  comment. The list holds 4096 entries at most (else exit 78). It is default deny: a list with no entry admits no peer. The file
  must belong to this user and must not be writable by others (else exit 71). A missing file exits 66.
* A peer line can have a grant (per-hop access control, T-290): `KEY services=raw,echo forward=yes`. `services=` is a list
  of 1 to 64 service names, or `*` for every service. `forward=yes` lets the peer have this node relay a stream to another
  node (the service `fwd`); `forward=no` does not. A peer that is not in the list gets no grant at all. The first line that
  admits a key is its grant.
  * A bare key, with no option (it can have comment words), keeps the old behaviour: every service, and forwarding. No file
    breaks.
  * A key with an option is restricted: what is not named is denied. `services=raw` alone allows `raw` and no forwarding.
    `forward=yes` alone allows forwarding and no service of this node. `services=*` with no `forward` allows every service and
    no forwarding. A word after the key that does not start with `services=` or `forward=` is still a comment, and a word that
    starts with `#` ends the options.
  * A bad service name, an empty list, more than 64 services for one peer, or a `forward=` value that is not `yes` or `no`
    exits 78.
* One decision is made for each `OPEN`, at the listener (and at the dialer for a stream that its listener opens): the grant of
  the peer that opened the stream on this link. A forwarded stream is a service request at each hop, so each hop decides on
  its own neighbour: the relay checks `forward`, and the last node checks the service for the node that sent the stream. A
  stream that is refused gets the answer byte 6 (`FORBIDDEN`), the opener exits 77 (a `--node` stream exits 69 and names the
  grant), the refusal is counted in `refused_service` or `refused_forward` of the row of the link in `link status`, and the
  link stays up. The streams of `--announce` are not a service: an admitted peer always sends them.

## The hello

The first stream of a link carries the hello. It runs before the link offers any service.

1. The dialer sends the tag, its public key and a nonce.
2. The listener sends its public key, a nonce and its signature over the transcript. The dialer checks the signature and checks
   the key against its list. A key that the list does not hold exits 77 at the dialer.
3. The dialer sends its signature over the transcript.
4. The listener checks the signature and its list, and sends one byte: admitted or refused. A peer that the list does not hold
   exits 77 at the dialer too.

The transcript holds both keys, both nonces and the role of the signer. A dialer that names a key but cannot sign with it is
refused. The hello ends in 10 s (else exit 124). The listener names each peer that it refuses on stderr (and in the log of an
instance).

The hello proves the keys of the two ends. It makes no key for the link and does not protect the streams after it. It is for
the unix and memory carriers, whose channel the kernel keeps on this host. A carrier that crosses a network does not rely on
it: the `tcp` carrier pins the key of the peer in its TLS handshake (T-268), and the other network carriers take the end-to-end
handshake of `pod-e2e` (T-298, see "Sealed streams").

## The route announcement

`podnet::mesh_announce` is the signed route announcement of the mesh (T-285). `--announce` on `link listen` and `link dial`
sends it over a link (`podnet::link_announce`, see "Announcements on a link" below).

* The functions do no I/O. The caller gives the time in milliseconds.
* The signed body has a fixed field order, all integers big-endian: version (2), origin key (32), `seq` (8), time of issue
  (8), expiry (8), neighbour count (2) and each neighbour (key 32, cost 4), service count (2) and each service (length 1,
  UTF-8 name). Then the signature (64). Then the path: a length n (1 to 16), and one hop for each node of the path, the
  origin first: its key (32), the cost of its link to the node before it (4; 0 for the origin), the key of the next node (32:
  the node that it sent the announcement to) and the signature of its hop (64).
* The encoding is canonical. The neighbours are in strictly ascending key order. The services are in strictly ascending byte
  order. The decoder refuses any other order and any byte after the end.
* The origin signs the label `podnet-mesh-announce-v2\0` and the body with its identity key, then the hop of the origin
  for one neighbour (`encode(identity, body, to)`). A forwarder adds its hop (`append_hop(wire, identity, cost, to)`): it
  signs the label `podnet-mesh-hop-v2\0`, the signature before it (the signature of the body for the hop of the origin), its
  key, the cost of its link to the node that sent it the announcement and the key of `to`. Each neighbour gets its own copy.
  The signatures form a chain. A hop cannot be dropped from the middle, a key cannot be changed and a cost cannot be lowered:
  the next signature covers the one before it, and `verify` checks the whole chain (`BadHopSignature`). Each hop names the
  next node of the path, and the last hop names the receiver (`BadNextHop`, `NotForMe`). Thus a node cannot cut the hops
  before its own and claim to be next to the origin: the hop of the origin names the first neighbour (T-346). The session
  with the sender authenticates the last node of the path.
* Bounds: 64 neighbours, 16 services, a service name of 1 to 64 bytes, 16 nodes in the path, 8 KiB for the body. An interval
  is 10 s, the life is at most 3 intervals (30 s), the clock skew is 120 s in each direction.
* `verify` drops an announcement for the first failed rule: shape and bounds (a version other than 2 is `BadVersion`),
  path (not from the origin, a node two times, this node), a hop that names another next node or a cost of the origin
  (`BadNextHop`), a last hop that names another node than this node (`NotForMe`), a time of issue more than 120 s ahead or
  behind, a bad lifetime, an expired announcement, a bad signature, a bad hop signature.
* `Receiver::accept` is `verify` and the seen-`seq` rule. It keeps the newest `seq` of each origin for 4096 origins. An entry
  is removed when its time of issue is more than 120 s old (a replay then fails the skew rule). At the limit, the entry with
  the oldest time of issue is removed.
* Only an announcement with the path `[origin]` is a neighbour claim (`Verified::neighbour_claims`). A multi-hop announcement
  tells reachability. `Verified::route` gives the announcement for `mesh_table`, with the sum of the signed hop costs
  (`Verified::path_cost`) as the cost from the sender to the origin.

## Announcements on a link

`link listen --announce`, `link dial --announce` (T-285) and `link connect --announce` (T-287) turn the route announcements on
for each link of the command. The
peer must run the flag too: a peer without it answers "no such service" to the stream, and the other end logs it and sends
nothing.

* Each end opens one stream of the service `announce` to the other end and sends its announcement on it at once, then each 10 s
  with a jitter of 10% either way. The announcement lists the peers of the links that are up (direct neighbours, each with the
  cost of its link) and the services of this end (the listener: the names of `--service` and `--to`; `link connect`: the names
  of `--service`; `link dial`: none). `link connect --announce-weight COST` sets the cost of its link, as on the other commands.
* A frame is a big-endian `u16` length and the wire bytes of the announcement. A length above 10369 ends the announce stream.
  The link stays up.
* Each announce stream has a bucket of signature checks (T-347): 2048 each second, 8192 at most
  (`Mesh::set_check_rate`). A frame costs one check for the body and one for each hop (17 for a path of 16 nodes:
  about 0.9 ms of one core in a release build, measured on 2026-11-14). A frame over the bucket is dropped before a
  check and counted (`limited` and `dropped` of the verb `routes`); the log gets one line for each stream in each 10 s.
  The checks run outside the lock of the route table, so a flood on one stream does not stop the others.
* The receiver takes an announcement whose path ends at the peer of the link (T-287; a path `[origin]` is a direct
  announcement). Any other announcement is dropped (`not from the peer itself`): this stops a peer from sending the
  announcement of a node that it does not speak for. Then it runs the rules of the announcement (the chain of hop signatures
  included) and the seen-`seq` rule, and gives the announcement to the route table. A dropped announcement changes no route,
  keeps no `seq`, and the link stays up. The listener and the dialer log it. An announcement of another version of the wire
  gets one log line for each stream, which names the version; the next ones are dropped with no line.
* A new announcement is forwarded: the node adds its hop (its key, the cost of its link to the peer, the key of the
  neighbour, its signature over the signature before it) and sends it on the `announce` stream of each other neighbour that
  is not in the path. Each neighbour gets a hop that names it. It forwards only
  a path that stays within the hop limit (8 nodes; `Mesh::set_hop_limit`, 16 at most). The forward queue of a link holds 64
  frames: a full queue drops the frame, and the origin sends again. Only the newest `seq` of an origin is forwarded.
* Each link of an origin gets its own `seq`, so one round of the origin comes on two paths with two `seq` (T-335). An
  announcement whose `seq` is not the newest of its origin goes to the route table only: the table takes it when its next hop
  has no newer route of that origin. It is not forwarded, and it does not change the status view. So in a diamond (A to D
  through B and through C) A keeps the two next hops after one round. When the link to the first next hop ends, the next
  stream takes the other one at once, with no new round. A replay from the same next hop is dropped (`the seq is not new`).
  The cost of a route is the cost of the link to the sender plus the signed costs of the hops.
* The first `seq` of a run is the time in ms, so a restart does not use a `seq` again.
* The cost of a link is `--announce-weight COST` (1 or more), else the RTT of the link in ms (at least 1), else 10.
* The end of a link ends the direct route of its peer at once.
* The name `announce` cannot be a `--service` of a listener that runs `--announce` (exit 64).
* The control socket of a listener has the verb `routes`: `{"verb":"routes","token":...}` gives `node`, `neighbours`,
  `received`, `dropped`, `evictions` and `routes`, the best route of each origin (`origin`, `next_hop`, `cost`, `hops`, `seq`,
  `expires_ms`, `direct`). Without `--announce` the verb exits 69. The dialer has no control socket and shows no routes.
  The control socket of `link connect --announce` has the verb `routes` too. `routes` also shows `forwarded`, the count of
  forwarded frames,
  and `limited`, the count of the frames that the bucket of a stream dropped.

## The status view of a mesh

`podnet link mesh --control unix:PATH [--json | --metrics] [--token-file FILE] [--max-nodes N] [--max-edges N]` shows the local
view of a mesh (T-289, `link_announce/status.rs`). It asks the verb `mesh` of the control socket of a listener that runs
`--announce`. The view is local: the socket is a unix socket of mode 0600, and no network endpoint serves the view. A
listener without `--announce` exits 69. A wrong token exits 77. `--control` without `unix:` exits 64.

* The default output is one line for each node and each edge. `--json` prints the document. `--metrics` prints the counters
  as Prometheus text (`podnet_mesh_nodes`, `podnet_mesh_edges`, `podnet_mesh_neighbours`,
  `podnet_mesh_announcements_received_total`, `podnet_mesh_announcements_dropped_total`,
  `podnet_mesh_announcements_forwarded_total`, `podnet_mesh_route_evictions_total`, `podnet_mesh_truncated_nodes`,
  `podnet_mesh_truncated_edges`, `podnet_mesh_truncated_bytes_nodes`).
* A node has `fingerprint`, `name`, `self`, `seq`, `hops`, `cost`, `next_hop`, `state`, `services`, `age_ms` and `edges`.
  This node is first (`self` true, hops 0); the other nodes follow in the order of their fingerprints. `seq`, `hops` and
  `cost` are those of the best route. `state` is `up`, `held` (the announcement expired and the route is held) or `down`
  (the session of the next hop closed). `age_ms` is the time since the last accepted announcement of the node. `name` is a
  local name (`Mesh::set_name`), else `null`: the announcement carries no name.
* An edge has `peer`, `cost`, `last_probe_ms` and `state`. The edges of this node are its links (`up`; `last_probe_ms` is the
  time since the last announcement of the peer, `null` if none came). The edges of another node are the neighbours that it
  signed in its last announcement (`claimed`). A claim is no proof of adjacency.
* The limits are 4096 nodes, 64 edges for each node and a document of 256 KiB (262143 bytes). `max_nodes` and `max_edges` of the
  request, or `--max-nodes` and `--max-edges`, can only lower them. A cut keeps the first nodes and the first edges. `truncated`
  has the count of what was cut: `nodes`, `edges` (the edges of cut nodes too) and `bytes` (nodes cut to fit the size).
* `counters` has the totals before the cut: `nodes`, `edges`, `neighbours`, `announcements_received`,
  `announcements_dropped`, `announcements_forwarded`, `route_evictions` and the three `truncated_*` counts.

## Forwarding a stream

A stream for a node that is not a neighbour goes through the mesh (T-287, `podnet::link_forward`). `link dial --announce --node
FINGERPRINT [--hops N] --service NAME` opens the stream: the dialer waits up to 20 s for a route to the node (the fingerprint
as `routes` shows it), then sends the stream to the next hop of the best route. A stream to a direct neighbour is the plain
request. Exit 69 when no route comes, the node has no such service, or the path is longer than `--hops`.

* The opener sends `PNSVC1 fwd`, then the header line `PNFWD1 DEST HOPS SERVICE VISITED`: the key of the destination in hex, the
  links that the stream may still cross, the service, and the keys of the nodes passed (hex, with commas).
* A node that gets it: is the destination (it serves the service as if the request came direct), or has `HOPS` 0 (answer 3, hop
  limit), or is in `VISITED` (answer 5, no route: the loop rule), or tries the usable routes to `DEST`, best first, skipping a
  next hop that is in `VISITED`. For each route it opens a stream to the next hop with `HOPS - 1` and itself in `VISITED`, and
  waits 30 s for the answer. A stream that cannot be opened, the answer 4 (the hop has no stream to give), the answer 5 and the
  timeout make it try the next route. Any other answer goes back to the previous hop. With the answer 0 it answers 0 and
  copies the bytes both ways. The credit windows of the links give back-pressure hop by hop.
* The answers of a service request: 0 open, 1 no such service, 2 the origin did not answer, 3 hop limit used up, 4 no stream to
  give (resource limit), 5 no route.
* 8 links by default, 16 at most (`--hops`). A route longer than the hop limit of a node is not used.
* The listener runs this when it has `--announce`. The name `fwd` cannot be a `--service` then (exit 64). A dialer with
  `--announce` relays a stream that its peer sends, but it has no service of its own.
* With `--seal` at both ends, the hops see sealed records only (see "Sealed streams").
* A neighbour cannot cut the hops before its own and claim a direct link to an origin (T-346): each hop names its next
  node. A hop of the path still sees and can drop the streams that cross it: use `--seal` through hops, so that a hop sees
  sealed records only.
* `link connect --announce` is a node that dials and serves: it announces its `--service` names, and a stream that its peer
  forwards for it reaches them. So three processes make a chain: B runs `link listen --announce`, C runs `link connect --announce`
  to B with a service, and A runs `link dial --announce --node C` to B. The names `announce` and `fwd` cannot be a `--service` of
  `link connect --announce` (exit 64).

```sh
podnet link listen 0.0.0.0:7000 --carrier tcp --allow nodes.txt --announce                                   # B
podnet link connect 'B-FINGERPRINT@b.example:7000' --carrier tcp --allow nodes.txt --announce --seal --service ssh=127.0.0.1:22   # C
podnet link dial 'B-FINGERPRINT@b.example:7000' --carrier tcp --peer B-FINGERPRINT --announce --seal --node C-FINGERPRINT --service ssh   # A
```

## Sealed streams

`--seal` of `link listen`, `link connect` and `link dial` seals each stream of a service end to end (T-298, T-299, T-300,
T-301; `podnet::link_record`, over `pod-e2e` of podcrates). Give it at both ends. A listener with `--seal` serves sealed
streams only. A dialer with `--seal` opens sealed streams only. A dialer with `--seal` and a listener without it fail (exit
non-zero, no data).

* The handshake runs on the stream, after the answer OPEN of the service request. It is Noise XX with the Ed25519 key of each
  end. The end that serves the stream must hold the key of the opener in its allow list (else exit 77). The opener checks the
  key of the end that serves: the listener of the link, or the node of `--node` (else exit 77). A hop of a mesh that forwards
  the stream copies the bytes of the handshake and of the records. It holds no key.
* The two ends agree on a suite: `chacha20-poly1305` (default) or `aes-256-gcm`. A suite that the two ends do not share exits
  78. The keys come from the exporter of the handshake, not from the TLS exporter of a hop.
* A record has a header of 24 bytes (`version`, `type`, `flags`, `stream`, `counter`, `length`, `pad`, `reserved`), a body and
  a tag of 16 bytes. The header is the associated data. The type `1` carries bytes. The type `2` (FIN) ends one direction. A
  stream that ends with no FIN is a cut: the reader gets an error (`UnexpectedEof`), not an end.
* The counter of each direction starts at 0. The key changes after 2^31 records, 1 GiB or 1 hour. The new key is in use before
  the old key is dropped.
* The receiver keeps a replay window of 1024 counters. A record at or below the window, or a repeat inside it, is dropped and
  counted. A record that fails its check (a changed header, `pad` or body) ends the stream with an error.
* `pad=on` (query key) pads each body to 0, 128, 512, 1024 or 2048 bytes, and above that to the next multiple of 2048. The
  receiver removes the padding. `pad` is in the associated data.
* `compress=snappy` (query key) compresses the payload before the seal (flag bit `0x0002`). A payload under 256 bytes, and a
  payload that does not get smaller, go with no flag. The receiver refuses a compressed payload that says more than 8 MiB or
  more than 64 times its own size: the stream ends with `decompress-limit`. The receiver reads compressed records at all
  times. Each end sets `pad` and `compress` for its own direction.
* The RESUME record (library: `Sealed::resume`): the peer sends again the payloads that the sender of the RESUME did not get
  (it keeps up to 1 MiB of sent payloads). A RESUME is used once, within 10 minutes of the last record. A copy of a RESUME is
  refused and counted.
* One resume window: `link_record::SETTINGS` sets `resume_window` of `pod-e2e` to `session::RESUME_WINDOW` (10 minutes, the
  `resume_deadline` of `pod-session`). The default `Options` of the layer use it.
* How a sealed stream resumes after the loss of its carrier:
  * Over a link of a session (`podnet::session`): the session sends again the lost bytes from offsets. The sealed stream sees
    no loss and sends no RESUME record. The stream goes on with no lost or repeated byte.
  * Over a carrier with no session below it (library: `Options::carry_over` and `Sealed::rebind`): after a cut, the stream
    waits for a new carrier for the window. Each end gives its end of the new carrier to `rebind`. Each end sends its RESUME
    record first on the new carrier, and sends no new record until the RESUME of the peer comes. Then it sends again the
    records from the counter that the RESUME names.
* Three failed RESUME records, a RESUME after the window, or no new carrier in the window end the stream with
  `ResumeExpired`. `link dial` then exits 69 (`link_record::ended`).
* Not done: the local door of `link dial --listen` and the control socket do not seal. `link dial --listen` with `--seal`
  exits 64. The CLI runs no session below a link and does not call `rebind`: a lost carrier ends a sealed stream of the CLI.

## The route table

`podnet::mesh_table` is the path-vector route table of the mesh (T-286). `--announce` fills it from the announcements of the direct peers (verb `routes`). The forwarding (T-287) uses it; the status view
(T-289) will use it.

* The table does no I/O. The caller gives the time in milliseconds to each call, and checks the signature of an announcement
  before `RouteTable::learn`.
* The table is keyed by the origin (a `PublicKey`). An origin keeps one route for each next hop, 8 at most.
* The cost of a route is the cost of the link to the sender plus the cost in the announcement. The path lists the origin first
  and the sender last. A direct neighbour sends the path `[origin]`.
* Selection (`best`, `ranked`): a fresh route before a held route, then the lowest cost, then the fewest hops, then the newest
  `seq`.
* A path that holds this node or a node two times is refused (`Loop`). A path of more than 16 nodes is refused. An expired
  announcement is refused. A `seq` that is not above the `seq` of the same next hop is refused (`StaleSeq`). Another next hop
  can give an older `seq` of the origin; the expiry of the announcement bounds the age of that route.
* A direct route ends at its expiry. A multi-hop route is held for one expiry interval (30 s) after its expiry, and it ends
  then. `session_closed` ends the direct route of that peer at once. The multi-hop routes through that peer are not used from
  then on, and they end after one expiry interval.
* The table holds 4096 origins. At 90% (3686) each new origin evicts the worst origin first: expired or held, then the highest
  cost, then the most hops, then the oldest `seq`. A direct route is not evicted: a table that holds direct routes only refuses
  the new origin (`Full`). `evictions()` counts the evictions.

## The C library

`include/podnet.h` is the C interface of the link (T-304). `crates/podnet-capi` builds it as a shared library and a static
library ([build.md](build.md), "The C library"). Linux only.

| Function | What it does |
| --- | --- |
| `podnet_init(config_json, err)` | Starts the library. `config_json` is NULL, empty, or `{"threads": N}` (1 to 64, default 2). It is idempotent: a second call returns 0 and changes nothing. The other functions start the library with the defaults when no call did. |
| `podnet_free()` | Stops the library. It returns `PODNET_E_STATE` while a link or a stream is open. |
| `podnet_identity_fingerprint(dir, buf, len)` | Writes the fingerprint of the install key in `dir` (made when missing). A peer gives it as `peer_fp`. |
| `podnet_link_new(identity_path, peer_fp, out)` | Makes an end that has the key of `identity_path` and admits only the key of `peer_fp`. |
| `podnet_link_pair(dialer, listener)` | Joins two ends over a memory carrier (1 MiB ring in each direction) and runs the hello of each end. A key that is not the `peer_fp` of the other end returns `PODNET_E_PERMISSION`. |
| `podnet_link_open(l, service, out)` | Opens a stream to a service and waits for the answer. |
| `podnet_link_accept(l, out)` | Waits for a stream of the peer, reads its service name and answers open. |
| `podnet_stream_service(s)` | The service name of a stream. |
| `podnet_stream_read(s, buf, len)` | Blocks. Returns the count (1 to `len`), 0 at the end of the stream, or a negative code. |
| `podnet_stream_write(s, buf, len)` | Blocks until the link took all `len` bytes. Returns `len`. |
| `podnet_stream_close(s)` / `podnet_link_close(l)` | End the stream (the peer reads the end of the stream) or the link, and free the handle. |
| `podnet_last_error()` / `podnet_version()` | The text of the last failure of the thread, and the version. |

* A function returns 0 or a count, or a negative code: `PODNET_E_ARG` -1, `PODNET_E_UNAVAILABLE` -2 (exit 69),
  `PODNET_E_PERMISSION` -3 (exit 77), `PODNET_E_IO` -4, `PODNET_E_STATE` -5, `PODNET_E_LIMIT` -6, `PODNET_E_PANIC` -99.
* `podnet_last_error()` is thread-local, UTF-8 and at most 64 KiB. It stays valid until the next call on the thread. Each
  call but `podnet_last_error()` and `podnet_version()` clears it.
* Handles are opaque. The caller owns each buffer; the library never frees a caller pointer. No Rust panic crosses the
  interface: each entry point catches it and returns `PODNET_E_PANIC`. The library calls no function of the caller.
* A call takes at most 1 MiB (`PODNET_MAX_IO`); more returns `PODNET_E_LIMIT`. Use one thread for each direction of a
  stream, and do not close a handle while another thread uses it.
* Only the memory carrier is in the interface now. Two ends of one process meet with `podnet_link_pair`. A network carrier
  has no C function in this version.
* The tests are `crates/podnet-capi/tests/c_abi.rs`, `c_abi.c` and `c_abi.py`: a memory link, a service `echo`, 1 MiB, the
  digest of the bytes that came back.

## Breaking

* 2026-11-14, T-346: the route announcement is version 2. Each hop names its next node, and the origin signs one hop for
  each neighbour. A node of version 2 refuses an announcement of version 1 (podnet 0.4.0-beta.10 and older) and logs one line
  for each stream that names the version: a mesh of mixed versions learns no route across the change. Update each node of a
  mesh. The library calls changed: `mesh_announce::encode` and `append_hop` take the key of the next node, and
  `link_announce::Mesh::make` takes the key of the neighbour.

## Exit codes

| Code | Case |
| --- | --- |
| 64 | A bad flag or value: an unknown carrier, a ring out of its limits, the `icmp` carrier with no `--allow-raw icmp`, a regular file at the address, no `--peer` or `--allow`; a chain of more than 8 elements, a network that is not an element, a chain with a carrier that is not `tcp`. |
| 66 | The allow list file is missing. |
| 69 | The address is held by a live listener, no listener answers, the host refuses the carrier (the `icmp` carrier with no `CAP_NET_RAW`), or a peer has another uid; an element of a chain that cannot be built or dialed; with `--seal`, `ResumeExpired` (three failed RESUME records). |
| 71 | The key file or the allow list file has a mode that others can use. |
| 77 | A peer is not in the allow list, or it did not prove its key. On `tcp`: a key that the handshake refused. With `--seal`: the end of a stream is not the expected key, or its key is not in the allow list. |
| 78 | The allow list has a line that is no peer, or more than 4096 entries; a duplicate element or a cycle in a chain; with `--seal`, a suite that the two ends do not share. |
| 124 | The hello did not end in 10 s. |

## Library

`podnet::link` opens a link over a carrier (`dial`, `listen`, `Door::accept`, `Door::accept_peer`, `unix_pair`, `memory_pair`). `podnet::session` runs
a link over a session that resumes after the loss of its carrier, in 10 minutes. `podnet::carriers` is the registry.
`podnet::identity` is the key and the allow list. `podnet::hello` is the hello. `podnet::mesh_announce` is the signed announcement. `podnet::mesh_table` is the route table of the mesh. `podnet::link_announce` sends and takes the announcements on a link. `podnet::link_arq` runs a link over the ARQ of `pod-link` over any lossy carrier of datagrams. `podnet::link_forward` forwards a stream through the mesh. `podnet::link_chain` checks and brings up a chain of `--egress` elements (`Plan::parse`, `open`, `check`). `podnet::link_tcp` (feature `tcp`) makes the TCP carrier from the key and the allow list (`carrier`, `carrier_via` over a chain),
listens (`listen`) and dials (`dial`, which gives the key that the listener proved). `podnet::link_ws` (feature `ws`) and `podnet::link_h2` (feature `h2`) have the same calls for the WebSocket and the HTTP/2 carrier. `podnet::link_keyed` opens the other carriers that pin the keys by name (`dial`, `listen`, `Tune`): `podnet::link_quic` (feature `quic`) `podnet::link_udp` (feature `udp`) `podnet::link_poll` (feature `poll`), `podnet::link_dns` (feature `dns`) and `podnet::link_icmp` (feature `icmp`).
The streams, the windows and the keepalive are the work of `pod-link` of PodUtils/podcrates. `crates/podnet-capi` is the C interface of the link (see The C library).
