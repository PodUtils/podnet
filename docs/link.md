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
```

* `link listen ADDRESS --allow FILE` binds the address through the carrier. It prints the address (for the unix carrier: the
  path and its mode, 0600), the fingerprint of this install and the size of the allow list. `--to TARGET` (`host:port` or
  `unix:/path`) sets where each stream of an admitted peer goes. Without it, the listener closes each stream at once. A second
  listener at a live address exits 69. A regular file at the address exits 64 and stays. A stale socket file is replaced.
* `link dial ADDRESS` connects, proves the key of this install, and checks the key of the listener. Give `--peer FINGERPRINT`
  (repeat it for more than one) or `--allow FILE`: the dialer trusts no listener by default (else exit 64). Then it copies stdin
  to one stream, and the stream to stdout. The lines about the peer go to stderr. `--check` stops after the check and prints the
  fingerprint of the peer; with `--json` it prints one document.
* `link status [NAME]` prints the fingerprint of this install and the link instances (`link listen --detach` or `--name`).
  `--create` makes the key when there is none; without it, `status` writes nothing. `--json` prints one document with `identity`
  and `links`.
* `link close NAME` stops a link instance (SIGTERM, then SIGKILL after `--timeout`, default 10 s). A name that is no link
  instance exits 64: use `podnet stop`.
* `link carriers [--json]` prints one line for each carrier: its framing, duplex, order, MTU, cost, address form and
  addressing mode, then `available` or `unavailable:` with the reason. The JSON has the same facts, `available` (bool) and
  `reason` (string or null).

`--detach`, `--name`, `--ready-timeout`, `--replace` and `--no-restart` of `link listen` are the flags of the background record
([background.md](background.md)). `podnet ps`, `logs`, `wait` and `stop` work on a link instance. The ready document has
`carrier`, `address`, `listen` (the same address), `mode`, `fingerprint`, `allow` and `to`.

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
it: it takes the end-to-end handshake of `pod-e2e` (T-298) or a TLS handshake that pins the key of the peer (T-268).

## Exit codes

| Code | Case |
| --- | --- |
| 64 | A bad flag or value: an unknown carrier, a ring out of its limits, a regular file at the address, no `--peer` or `--allow`. |
| 66 | The allow list file is missing. |
| 69 | The address is held by a live listener, no listener answers, the host refuses the carrier, or a peer has another uid. |
| 71 | The key file or the allow list file has a mode that others can use. |
| 77 | A peer is not in the allow list, or it did not prove its key. |
| 78 | The allow list has a line that is no peer, or more than 4096 entries. |
| 124 | The hello did not end in 10 s. |

## Library

`podnet::link` opens a link over a carrier (`dial`, `listen`, `Door::accept`, `unix_pair`, `memory_pair`). `podnet::session` runs
a link over a session that resumes after the loss of its carrier, in 10 minutes. `podnet::carriers` is the registry.
`podnet::identity` is the key and the allow list. `podnet::hello` is the hello.
The streams, the windows and the keepalive are the work of `pod-link` of PodUtils/podcrates.
