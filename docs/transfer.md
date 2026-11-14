# File transfer by code: `podnet send` and `podnet receive`

This page gives `podnet send` and `podnet receive`: a file, a directory or a
text from one host to another with a short code, as magic-wormhole does. The
code meets at a public mailbox. The bytes go direct between the hosts, or
through a transit relay. Between two podnet ends, the podnet relay and a
piping server are the last ways. Each byte is sealed end to end: the
mailbox, the relays and the piping server see no plain text.

## Send and receive

```sh
# On the first host:
podnet send ./report.pdf          # prints: Wormhole code is: 7-crossover-clockwork-adroitness
podnet send ./photos              # a directory goes as a zip
podnet send --text "the words"    # a text goes in the wormhole: no transit

# On the second host:
podnet receive                    # asks for the code, shows the offer, asks y/N
podnet receive --out ~/Downloads  # where the file or the directory lands
echo 7-crossover-clockwork-adroitness | podnet receive --yes   # a script
```

* `podnet receive` reads the code from the terminal, or one line of stdin.
  It takes no code on the command line: each user of the host sees the
  command line of each process.
* The code is a number and three words of the PGP word list.
  `podnet send --words N` gives N words (2 to 8, as magic-wormhole's
  `--code-length`; another count exits 64). Each word adds 8 bits: with 2
  words, a guess has one chance in 65536. A code is good
  for one exchange: a wrong code, a guess or a success spends it. A wrong
  code fails at the two ends (exit 77): send again for a new code.
* `--yes` accepts a file or a directory with no question. When stdin is
  not a terminal, `--yes` is necessary (else exit 64). A text needs no
  answer.
* `--json` prints one JSON object for each event on stdout: the sender
  prints `{"event": "code", "code", "mailbox"}` first, then
  `{"event": "sent", "bytes", "sha256", "via"}`; the receiver prints
  `{"event": "received", ...}`.
* `podnet send --timeout SECS` is the wait for a receiver and for its
  answer (default 1800).

## The meeting: mailboxes

The mailboxes are the services of the kind `mailbox` of `pod-services`
(PodUtils/podcrates), in their order: the public mailbox of magic-wormhole,
then the mailbox of Least Authority (Winden). `--mailbox URL` (more than
one is permitted) or `PODNET_MAILBOX` (URLs separated by commas) replaces
the list.

* The sender takes the first mailbox that answers.
* The receiver looks for the code at each mailbox in order: a mailbox that
  lists the nameplate of the code has the sender; one that lists other
  nameplates does not; at one that lists none, the receiver waits 10 s for
  the sender's first message, then goes to the next mailbox.
* The code carries no mark of its mailbox, so a code of magic-wormhole
  works as it is. Known limit: magic-wormhole issue #754 (open, 2026-11-04)
  says that two clients of the public mailbox can reach two instances of
  it, which do not share their mailboxes. Then the receiver finds nobody
  (exit 69). Give the two ends the same `--mailbox`, for example the
  second mailbox of the list.

The mailbox, SPAKE2 and the sealed phases are the crate `pod-wormhole` of
PodUtils/podcrates. The AppID is `lothar.com/wormhole/text-or-file-xfer`,
the AppID of magic-wormhole's file transfer.

## The transit: direct or through a relay

After the offer and its answer, the two ends connect for the bytes:

* Each end listens on a free port of each address of the host (not the
  loopback), and sends these addresses as hints. `--listen ADDR` listens on
  one address only, and sends that address (also a loopback address).
  `--no-listen` does not listen. A host that refuses a bind of `AF_INET`
  has no listener: the other end or a relay carries the transfer.
* Each end dials each hint of the other end through the way out of podnet
  (`--proxy`, `--egress`, `NO_PROXY`).
* After 2 s (at once when no end has a direct hint) each end also dials
  each transit relay of the two ends. The relays of this end are the
  services of the kind `transit` of `pod-services`, in their order (Least
  Authority's WebSocket relay on port 443, then the public TCP relay of
  magic-wormhole on port 4001, `magic-wormhole-transit`), or
  `--transit-relay URL` (`tcp:HOST:PORT` or `tcp://HOST:PORT`, `ws://...`,
  `wss://...`; `none` for no relay of this end).
* The sender takes the first connection that passes the handshake, and the
  bytes go in records sealed with NaCl's SecretBox under the transit key.
* The receiver sends the SHA-256 of the bytes at the end. The sender
  compares it with its own.

A host whose only way out is a proxy on TCP 443 reaches the WebSocket relay
of Least Authority. The TCP relay on port 4001 is the default relay of the
Python magic-wormhole: a host where port 4001 is open meets a Python end
there.

## The podnet relay: a podnet extension

Two hosts that reach only TCP 443 through a proxy can fail to reach a
transit relay. Then the podnet relay (the reverse road, [relay.md](relay.md))
carries the transit. This way is a podnet extension: it works from podnet
to podnet only.

```sh
# On the first host (a pair of `podnet relay pair`; no node of it runs):
podnet send --relay-pair NAME report.pdf
# On the second host:
podnet receive
```

* `podnet send --relay-pair NAME` runs the node of the stored pair `NAME`
  (`podnet relay pair`) for the time of the transfer. Do not run another
  node of the pair at the same time: the relay gives the name to one node.
  A pair that is not stored stops the sender before its offer, with the
  error of `podnet relay node`, and the receiver gets the error.
* The sender adds one hint of its own kind, `podnet-relay-v1`, with the URL
  of the relay and the name of the pair. It puts the connect token of the
  pair under the key `podnet-relay-token` of its `transit` message. The
  wormhole seals this message, so only the receiver of the code reads the
  token.
* A client of magic-wormhole ignores a hint of a kind that it does not know,
  and a key of the `transit` message that it does not know. podnet sends a
  file or a directory to podnet only, so the hint goes to podnet only.
* The receiver dials the node as an operator of the reverse road. It is the
  last way: it starts 4 s after the other hints when a direct hint or a
  transit relay exists, else at once. A node that is not online is
  dialed again each second, until the end of the wait of the transit.
* Each session runs the resumable layer of `relay node --resume` and
  `relay connect --resume`: the session moves to a new leg before the byte
  cap of the relay (48 MiB of a leg), and resumes after the loss of a leg.
  A transfer thus goes past the 64 MiB of one relay session.
* On the stream, the two ends run the handshake of the transit and the
  records that the transit key seals, as on a direct connection. The relay
  sees no byte of the file. The line of the sender and of the receiver says
  `podnet relay NAME`.
* After the ack, the sender waits at most 2.5 s for the end of the receiver
  before its node stops. The receiver waits at most 15 s for the end of the
  session.

## A piping server: a podnet extension

When no direct hint, no transit relay and no podnet relay works, a piping
server carries the transit. A piping server gives the body of a request
`PUT /PATH` to the one request `GET /PATH` on the same path, on HTTPS port
443. This way is a podnet extension: it works from podnet to podnet only.

```sh
# The servers of pod-services (the default):
podnet send report.pdf
# A server of your own, at the two ends:
podnet send --piping https://piping.example report.pdf
PODNET_PIPING=https://piping.example podnet receive
# No piping server:
podnet send --piping none report.pdf
```

* The servers of an end are the services of the kind `piping` of
  `pod-services`, in their order. `--piping URL` (repeat it for a list) or
  `PODNET_PIPING` (URLs separated by commas) gives other servers.
  `--piping none` gives no piping server.
* Each end adds one hint of the kind `piping-v1` (`url`) for each of its
  servers. The two ends use the first server of the sender that the
  receiver names too. When no server is in the two lists, the transfer
  does not use a piping server. A client of magic-wormhole ignores the
  hint.
* It is the last way: it starts 6 s after the other hints when another way
  exists (a direct hint, a transit relay or the podnet relay), else at
  once.
* The base path is the first 32 hex digits of HKDF-SHA256 of the transit
  key with the info `podnet piping path v1`. Only the two ends know it.
* The bytes go in numbered pieces of at most 256 KiB, not in one stream: a
  server or a proxy can give the body of a request only at its end. The
  bytes of the sender go to `/BASE/s/N`, the bytes of the receiver to
  `/BASE/r/N`, with N from 0. An empty piece ends the bytes of an end.
* Each request goes through the way out of podnet. A request that times
  out (60 s) goes again. Five other failures in a row stop that
  direction, and the transfer fails.
* The piping server sees only the sealed records: the handshakes of the
  transit and the records that the transit key seals. It sees no byte of
  the file, no name and no code. It sees the size and the time of each
  piece, and the addresses of the two ends. The line of the sender and of
  the receiver says `piping URL`.

## What the receiver writes

The receiver checks each name before it writes a byte:

* Each name goes through `pod_extract::safety::components`: no absolute
  path, no `..`, no NUL. Also refused: a backslash in a component, a
  component over 255 bytes, a path over 4096 bytes, two entries with the
  same name after ASCII case folding.
* Only regular files and directories: a symbolic link, a hard link, a
  device, a fifo or a socket in a zip is refused.
* The file or the directory of the transfer must not exist under `--out`.
  The receiver never overwrites.
* Each file is made new (`O_CREAT|O_EXCL|O_NOFOLLOW`) with the mode
  `0666 & ~umask`: never an executable or a set-id bit.
* `--max-files N` (default 100000) and `--max-bytes N` (default 64 GiB)
  cap a transfer. The receiver checks them from the offer and the zip
  directory before it writes, and again while it writes, as a header can
  lie.
* The offer that the person accepts is a cap too: a zip with more bytes
  or more entries than its offer said is refused (T-341).
* A refusal ends the whole transfer with exit 65, and no file of it stays
  on the disk. When a write fails, the receiver removes each file and
  directory that it made, and no other.

A directory goes as a zip with stored entries. The receiver reads stored
and deflated entries, as `wormhole send` makes them. There is no zip64: an
entry of 4 GiB or more, or more than 65535 entries, is refused.

## With magic-wormhole

A text, a file and a directory go to and from each client of
magic-wormhole. The transit key is the key of magic-wormhole:
`derive_key` of the key of the wormhole (`pod-wormhole`) for the purpose
`lothar.com/wormhole/text-or-file-xfer/transit-key`, 32 bytes. Each podnet
end says `{"podnet": {"transit-key": 2}}` in its `version`. podnet
0.4.0-beta.10 and older (`"transit-key": 1`) drew a random transit key:
a file between such a podnet and this one stops with a line that says
"update podnet". A text works.

The test `tests/wormhole_interop.rs` (T-106, T-103) runs a text, a file
and a directory both ways with the Python magic-wormhole 0.24.0 at the
mailbox server of magic-wormhole (`twist wormhole-mailbox`, 0.8.0) on the
loopback. For a file and a directory, the two ends have the stand-in TCP
transit relay of the tests (`--transit-relay`, `--transit-helper`), and
the Python end listens for a direct connection too. A wrong code exits 77
at podnet, and `wormhole send` fails too. With no `wormhole` program on
the PATH, each test prints a skip line. With `wormhole` and no `twist`,
the tests use the stand-in mailbox of the tests. CI (`ci.yml`, job
`test`) installs the two packages in a venv and runs the test.

```sh
python3 -m venv /tmp/wormhole
/tmp/wormhole/bin/pip install magic-wormhole==0.24.0 magic-wormhole-mailbox-server==0.8.0
PATH=/tmp/wormhole/bin:$PATH cargo test --test wormhole_interop
```

## Exit codes

| Code | When |
| --- | --- |
| 0 | The transfer completed. |
| 1 | The other end refused the offer, or broke the protocol. |
| 64 | A bad command line, a bad code, or a file offer with no terminal and no `--yes`. |
| 65 | The receiver refused the input: a name, a kind, a name that exists, a cap, a SHA-256 that differs. |
| 66 | The path to send, or `--out`, does not exist. |
| 69 | No mailbox answered, no sender waits at the code, or no way of the transit worked. |
| 77 | The code did not match at the other end: the code is spent. |
| 124 | No receiver took the code within `--timeout`. |
