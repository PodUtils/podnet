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
  `unix:/path`. `--service socks5` (no target) makes this end the server side of SOCKS5 (see "The service `socks5`"). A stream for a service that the listener does not have is refused (the opener exits 69). A second
  listener at a live address exits 69. A regular file at the address exits 64 and stays. A stale socket file is replaced.
* `link dial ADDRESS` connects, proves the key of this install, and checks the key of the listener. Give `--peer FINGERPRINT`
  (repeat it for more than one) or `--allow FILE`: the dialer trusts no listener by default (else exit 64). Then it copies stdin
  to one stream of the service `--service NAME` (default `raw`), and the stream to stdout. The lines about the peer go to stderr. `--check` stops after the check and prints the
  fingerprint of the peer; with `--json` it prints one document.
* `link connect ADDRESS (--peer FINGERPRINT | --allow FILE)` connects, runs the hello, and holds the link until the link ends
  (exit 69) or a stop signal. It runs the control socket. The ready document has `carrier`, `address`, `fingerprint`, `peer` and
  `control`. It accepts `--detach`, `--name` and the other flags of the background record. This end offers no service: it closes
  each stream that the peer opens.
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
admits the peer. Not built (T-291 is partial): `BIND` and `UDP ASSOCIATE` (both get reply 7, command not supported), and `--resolve local`.
`link dial` exits at once when the far end ends the stream, also when stdin is still open.

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
    (the streams opened here and by the peer), `since`, and `stream_ids` (the streams that a task serves, 64 at most), and at the
    top `selected` (the pinned link, or `null`). It has no seed and no token.
  * `peer` (`"peer":FINGERPRINT`): `ok` when a link that is up goes to that peer; 77 when the links go to other peers; 69 when
    the instance holds no link that is up.
  * `open` (`"peer":FINGERPRINT,"service":NAME`): opens one stream, asks for the service, and on `ok` the connection carries the
    bytes of the stream. The network `peer:FINGERPRINT/SERVICE` uses it.
  * `close-link` (`"link":ID`): end the link with a GOAWAY after the frames that wait. A link that has ended is not an error.
  * `close-stream` (`"link":ID,"stream":ID`): reset one stream. The other streams and the link stay. The ids are the
    `stream_ids` of the row of `status` (64 ids at most for each link).
  * `select` (`"link":ID` or `"link":null`): pin the link that serves the streams opened from now on (the verb `open`, so
    `peer:FINGERPRINT/SERVICE`); `null` returns to the first link that is up. The streams that run stay where they are. The
    pin ends with its link. A link that has ended exits 69. `status` shows the pin as `selected`.
  * Each of the three answers the document of `status` after the change, with `verb` set to its name. An id that is missing,
    is no number or names no link or stream gets 64. They need the token and the user check, like every verb: a refused
    request changes nothing.
  * `rotate` and `reconnect` need a carrier that the instance dials again. An instance holds links and not the way to make
    them, so both get 69 (`not built`).
  * `dump`: the state of the instance for a bug report. It has `build` (`version`, the optional `features` that this binary
    holds, and the `carriers` with `feature` and `built`), `fingerprint`, `pid`, `settings` (`selected` and the `limits`),
    `totals` (`links`, `up`, `streams`, `opened`, `accepted`), `links` (the row of `status`, and `stream_table`: one
    `{id, owner}` for each stream, `owner` being `local` or `peer`, and `streams_cut`), the last 20 `errors` (`at`, `text`; a text
    keeps 512 characters), `routes` (`built: false`: the route table waits for the mesh) and `cut`. It has no seed, no token
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
| `compress` | `none`, `snappy` | T-299 adds it. |
| `pad` | `on`, `off` | T-300 adds it. |
| `mtu` | 64 to 65535, at most the unit of the carrier | T-276 adds it. A stream carrier has no unit: exit 64. |
| `maxhops` | 1 to 8 | T-287 adds it. |
| `poll` | 1 to 60 (seconds) | T-271 adds it. |
| `window` | 16384 to 8388608 (bytes) | T-262 adds it. |
| `name` | 1 to 32 characters of `a-z 0-9 -` | No effect in a query: give the name with `--name` (T-308). |

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
| `quic` | `HOST:PORT` | QUIC streams (T-273). Build feature `quic`. |
| `s3` | `BUCKET/PREFIX` | A simplex pair of objects in a bucket (T-279). Build feature `s3`. |
| `icmp` | `HOST` | ICMP echo, a datagram carrier (T-282). Build feature `icmp`; it needs a privilege. |

The carriers `tcp`, `quic`, `s3` and `icmp` sit behind a build feature ([build.md](build.md), Features). The default build has none of
them. `podnet link carriers` lists each one with `unavailable: not built: the feature NAME is off (build with --features NAME)`
(the JSON has `built: false`, `feature` and `reason`, and no facts that a carrier would give), and a command that names the
carrier exits 69 with that reason. In a build with the feature on, the row says that the code of the carrier is not in this
build, and names its entry (T-273, T-279 or T-282).

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
handshake of `pod-e2e` (T-298).

## The route announcement

`podnet::mesh_announce` is the signed route announcement of the mesh (T-285). `--announce` on `link listen` and `link dial`
sends it over a link (`podnet::link_announce`, see "Announcements on a link" below).

* The functions do no I/O. The caller gives the time in milliseconds.
* The signed body has a fixed field order, all integers big-endian: version (1), origin key (32), `seq` (8), time of issue
  (8), expiry (8), neighbour count (2) and each neighbour (key 32, cost 4), service count (2) and each service (length 1,
  UTF-8 name). Then the signature (64). Then the path: a length n (1 to 16), the origin key (32), and for each of the n - 1
  forwarders its key (32), the cost of its link (4) and the signature of its hop (64).
* The encoding is canonical. The neighbours are in strictly ascending key order. The services are in strictly ascending byte
  order. The decoder refuses any other order and any byte after the end.
* The origin signs the label `podnet-mesh-announce-v1\0` and the body with its identity key. A forwarder adds its hop
  (`append_hop(wire, identity, cost)`): it signs the label `podnet-mesh-hop-v1\0`, the signature before it (the origin
  signature for the first hop), its key and the cost of its link to the node that sent it the announcement. The signatures
  form a chain. A hop cannot be dropped from the middle, a key cannot be changed and a cost cannot be lowered: the next
  signature covers the one before it, and `verify` checks the whole chain (`BadHopSignature`). A forwarder vouches only for
  itself: it can cut the chain at its own hop and claim to be next to the origin. The session with the sender authenticates
  the last node of the path.
* Bounds: 64 neighbours, 16 services, a service name of 1 to 64 bytes, 16 nodes in the path, 8 KiB for the body. An interval
  is 10 s, the life is at most 3 intervals (30 s), the clock skew is 120 s in each direction.
* `verify` drops an announcement for the first failed rule: shape and bounds, path (not from the origin, a node two times,
  this node), a time of issue more than 120 s ahead or behind, a bad lifetime, an expired announcement, a bad signature, a
  bad hop signature.
* `Receiver::accept` is `verify` and the seen-`seq` rule. It keeps the newest `seq` of each origin for 4096 origins. An entry
  is removed when its time of issue is more than 120 s old (a replay then fails the skew rule). At the limit, the entry with
  the oldest time of issue is removed.
* Only an announcement with the path `[origin]` is a neighbour claim (`Verified::neighbour_claims`). A multi-hop announcement
  tells reachability. `Verified::route` gives the announcement for `mesh_table`, with the sum of the signed hop costs
  (`Verified::path_cost`) as the cost from the sender to the origin.

## Announcements on a link

`link listen --announce` and `link dial --announce` (T-285) turn the route announcements on for each link of the command. The
peer must run the flag too: a peer without it answers "no such service" to the stream, and the other end logs it and sends
nothing.

* Each end opens one stream of the service `announce` to the other end and sends its announcement on it at once, then each 10 s
  with a jitter of 10% either way. The announcement lists the peers of the links that are up (direct neighbours, each with the
  cost of its link) and the services of this end (the listener: the names of `--service` and `--to`; the dialer: none).
* A frame is a big-endian `u16` length and the wire bytes of the announcement. A length above 9789 ends the announce stream.
  The link stays up.
* The receiver takes an announcement whose path ends at the peer of the link (T-287; a path `[origin]` is a direct
  announcement). Any other announcement is dropped (`not from the peer itself`): this stops a peer from sending the
  announcement of a node that it does not speak for. Then it runs the rules of the announcement (the chain of hop signatures
  included) and the seen-`seq` rule, and gives the announcement to the route table. A dropped announcement changes no route,
  keeps no `seq`, and the link stays up. The listener and the dialer log it.
* A new announcement is forwarded: the node adds its hop (its key, the cost of its link to the peer, its signature over the
  signature before it) and sends it on the `announce` stream of each other neighbour that is not in the path. It forwards only
  a path that stays within the hop limit (8 nodes; `Mesh::set_hop_limit`, 16 at most). The forward queue of a link holds 64
  frames: a full queue drops the frame, and the origin sends again. The route table keeps one `seq` for each origin: the
  first announcement of a `seq` that arrives sets the route of that round, a longer path of the same `seq` is dropped.
  The cost of a route is the cost of the link to the sender plus the signed costs of the hops.
* The first `seq` of a run is the time in ms, so a restart does not use a `seq` again.
* The cost of a link is `--announce-weight COST` (1 or more), else the RTT of the link in ms (at least 1), else 10.
* The end of a link ends the direct route of its peer at once.
* The name `announce` cannot be a `--service` of a listener that runs `--announce` (exit 64).
* The control socket of a listener has the verb `routes`: `{"verb":"routes","token":...}` gives `node`, `neighbours`,
  `received`, `dropped`, `evictions` and `routes`, the best route of each origin (`origin`, `next_hop`, `cost`, `hops`, `seq`,
  `expires_ms`, `direct`). Without `--announce` the verb exits 69. The dialer has no control socket and shows no routes.
  `link connect` has no `--announce` (T-285 left it out). `routes` also shows `forwarded`, the count of forwarded frames.

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
* Not done: the bytes are not encrypted end to end (T-298), so each hop sees them. The CLI has no node that both listens and
  dials, so a chain of three podnet processes needs a library caller (`link_forward::open`).

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
  announcement is refused. A `seq` that is not above the `seq` of the same next hop, or is below the newest `seq` of the origin,
  is refused (`StaleSeq`).
* A direct route ends at its expiry. A multi-hop route is held for one expiry interval (30 s) after its expiry, and it ends
  then. `session_closed` ends the direct route of that peer at once. The multi-hop routes through that peer are not used from
  then on, and they end after one expiry interval.
* The table holds 4096 origins. At 90% (3686) each new origin evicts the worst origin first: expired or held, then the highest
  cost, then the most hops, then the oldest `seq`. A direct route is not evicted: a table that holds direct routes only refuses
  the new origin (`Full`). `evictions()` counts the evictions.

## Exit codes

| Code | Case |
| --- | --- |
| 64 | A bad flag or value: an unknown carrier, a ring out of its limits, a regular file at the address, no `--peer` or `--allow`; a chain of more than 8 elements, a network that is not an element, a chain with a carrier that is not `tcp`. |
| 66 | The allow list file is missing. |
| 69 | The address is held by a live listener, no listener answers, the host refuses the carrier, or a peer has another uid; an element of a chain that cannot be built or dialed. |
| 71 | The key file or the allow list file has a mode that others can use. |
| 77 | A peer is not in the allow list, or it did not prove its key. On `tcp`: a key that the handshake refused. |
| 78 | The allow list has a line that is no peer, or more than 4096 entries; a duplicate element or a cycle in a chain. |
| 124 | The hello did not end in 10 s. |

## Library

`podnet::link` opens a link over a carrier (`dial`, `listen`, `Door::accept`, `Door::accept_peer`, `unix_pair`, `memory_pair`). `podnet::session` runs
a link over a session that resumes after the loss of its carrier, in 10 minutes. `podnet::carriers` is the registry.
`podnet::identity` is the key and the allow list. `podnet::hello` is the hello. `podnet::mesh_announce` is the signed announcement. `podnet::mesh_table` is the route table of the mesh. `podnet::link_announce` sends and takes the announcements on a link. `podnet::link_forward` forwards a stream through the mesh. `podnet::link_chain` checks and brings up a chain of `--egress` elements (`Plan::parse`, `open`, `check`). `podnet::link_tcp` (feature `tcp`) makes the TCP carrier from the key and the allow list (`carrier`, `carrier_via` over a chain),
listens (`listen`) and dials (`dial`, which gives the key that the listener proved).
The streams, the windows and the keepalive are the work of `pod-link` of PodUtils/podcrates.
