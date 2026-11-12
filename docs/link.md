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
  `unix:/path`. A stream for a service that the listener does not have is refused (the opener exits 69). A second
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

## Flags of the carriers

* `--carrier NAME` selects the carrier (default `unix`). An unknown name exits 64 and lists the names.
* `--allow-other-uid` (unix carrier) accepts a peer of another uid. Each end checks the uid of the other end with
  `SO_PEERCRED`, so give the flag at both ends. Without it, a peer of another uid exits 69.
* `--ring BYTES` (memory carrier) sets the ring of each direction: 65536 to 67108864 (64 KiB to 64 MiB). Default 1048576. A
  value out of the limits exits 64.

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
  fingerprint, or the key in hex or base32. The dial goes through the way out of the process (`--proxy`). It ends in 20 s. The key
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

## Exit codes

| Code | Case |
| --- | --- |
| 64 | A bad flag or value: an unknown carrier, a ring out of its limits, a regular file at the address, no `--peer` or `--allow`. |
| 66 | The allow list file is missing. |
| 69 | The address is held by a live listener, no listener answers, the host refuses the carrier, or a peer has another uid. |
| 71 | The key file or the allow list file has a mode that others can use. |
| 77 | A peer is not in the allow list, or it did not prove its key. On `tcp`: a key that the handshake refused. |
| 78 | The allow list has a line that is no peer, or more than 4096 entries. |
| 124 | The hello did not end in 10 s. |

## Library

`podnet::link` opens a link over a carrier (`dial`, `listen`, `Door::accept`, `Door::accept_peer`, `unix_pair`, `memory_pair`). `podnet::session` runs
a link over a session that resumes after the loss of its carrier, in 10 minutes. `podnet::carriers` is the registry.
`podnet::identity` is the key and the allow list. `podnet::hello` is the hello. `podnet::link_tcp` (feature `tcp`) makes the TCP carrier from the key and the allow list (`carrier`),
listens (`listen`) and dials (`dial`, which gives the key that the listener proved).
The streams, the windows and the keepalive are the work of `pod-link` of PodUtils/podcrates.
