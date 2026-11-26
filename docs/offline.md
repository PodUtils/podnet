# Offline: `podnet offline`, and the carriers `serial` and `hf`

A host can have no network for a while: a field laptop, a radio link, a machine in a box. This page gives the commands that
carry work across that gap, and the two carriers that run a link over a serial device. No command of this page opens a
socket on the network, and none of them needs one. Each command reads and writes files in a directory, and the two
carriers speak over a device such as `/dev/ttyUSB0`.

```sh
podnet offline spool add ./spool --file report.txt        # one entry, with a sequence, a checksum and the sender's key
podnet offline spool take ./spool --out report.txt        # the lowest sequence: write the body, then remove the entry
podnet offline discover announce --name field --address /dev/ttyUSB0 --port 22 --to 127.0.0.1:5353
podnet offline discover listen --timeout 10 --json        # the peers whose signature verified
podnet offline bundle build --out ./bundle --name field --file ./site:site   # each file of ./site at site/PATH
podnet offline bundle build --out ./delta --name field --base ./old --file ./site:site
podnet offline bundle apply --base ./old --delta ./delta
podnet offline capture merge ./a.cap ./b.cap --out all.cap --secret-file secrets.txt
podnet offline pair offer --out offer.txt > code.txt      # eight words in code.txt, and a signed offer
podnet offline pair accept --offer offer.txt --used ./used < code.txt   # the eight words on stdin
podnet offline replica merge ./a.log ./b.log --out merged.log --json
```

The exit codes are those of `podnet man exit-status`: 64 for a bad flag or a bad address, 65 for data that does not verify
or a delta from another base, 66 for a missing file or an empty spool, 69 for a carrier or a device that the host cannot
use, 71 for another failure of the file system, and 77 for a pairing code that was already used. `podnet offline spool
check` exits 65 and names each fault at its file and offset, or 0 and prints `ok: no fault`.

## The spool

`spool add DIR [--file FILE] [--sequence N] [--key DIR]` writes one file in `DIR`, with the body, the sequence, the digest
and the sender's key. The body comes from `--file`, else from stdin. The sequence defaults to one above the highest entry.
The command prints `SEQUENCE DIGEST LENGTH PATH`.

`spool list DIR [--json]` prints each entry that verifies, in the order of its sequence. A line is `SEQUENCE SENDER
LENGTH DIGEST PATH`; the JSON document has one object for each entry with `sequence`, `sender`, `length`, `checksum` and
`file`.

`spool take DIR [--out FILE]` takes the entry with the lowest sequence, writes its body to `--out` (else stdout), and
removes the file only after the write. A process that is killed before the write keeps the entry, so a later take gives
the body again; the test `a_taken_entry_survives_a_kill_before_its_consume` kills a real process between the receive and
the consume and reads the entry afterwards. A process that dies between the write and the removal leaves the entry, so a
second take writes the body a second time. The spool is at least once, not exactly once: a body that must apply once
needs an idempotent apply, with the digest of `spool list` as its key.

`spool check DIR` reads each file and prints `FILE OFFSET WHY` for each fault, then exits 65; a spool with no fault prints
`ok: no fault` and exits 0. An entry with a bad checksum, a bad header or a bad length is a fault, and it is not listed by
`spool list`.

## Discovery

`discover announce --name NAME --address ADDRESS --port PORT [--to IP:PORT] [--repeat N] [--key DIR]` signs one TXT
record with the key of this install and sends it to the mDNS group `224.0.0.251:5353`, or to `--to`. The record holds the
name, the address, the port and the key. `--repeat` sends it more than once, because a datagram is lost.

`discover listen [--bind IP:PORT] [--timeout SECONDS] [--json]` hears the announcements for the time (5 s by default),
verifies each signature, and prints `NAME ADDRESS:PORT KEY` for each peer that verifies. The JSON document has `peers`,
`records` (each packet that carried a record) and `ignored` (each record whose signature did not verify). A wildcard bind
joins the group. A record whose name, address, port or key was changed after signing does not verify: it is counted in
`ignored` and is not a peer. The test `a_listener_takes_the_signed_record_and_ignores_a_changed_one` sends one good
record and one record with a changed port, and reads one peer and at least one ignored record.

The record has a TTL of 120 s. The command sends and hears one packet at a time; it does not answer a query, and it keeps
no cache of the peers after the command ends.

## Bundles and deltas

`bundle build --out DIR --name NAME --file SRC:DEST [--file ...] [--key DIR]` writes a bundle: each `DEST` under `DIR`,
with a signed manifest of each file and its digest. `DEST` is relative and holds no `..`. It prints `bundle NAME ID files
N`.

`bundle build` with `--base DIR` reads the bundle of the previous release and writes a delta instead: the files to add,
the files to change and the files to delete, with the id of the base in its manifest. It prints `delta NAME ID base ID add
N change N delete N`. A delta is smaller than the bundle that it updates when most files did not change; the test
`a_delta_applies_on_the_host_of_its_base_and_carries_only_the_change` changes one file of two and asserts that the delta
holds the changed file and not the unchanged one, and that the delta is smaller than the base.

`bundle apply --base DIR --delta DIR` applies the delta to the base bundle and prints `applied NAME ID files N`. A delta
whose base id is not the id of `--base` is refused, and exits 65; it is not applied in part. `bundle verify DIR` checks the
signature of the manifest and the digest of each file, and prints `bundle NAME ID files N key KEY`.

## Capture

A capture file starts with the line `pod-offline capture 1`. Each record is one line `<nanoseconds> <in|out> <0|1>
<hex>`. The third field is 1 when a secret was replaced by `<redacted>` before the line was written.

`capture merge --out FILE FILE... [--secret SECRET] [--secret-file FILE]` reads the files in the order of the list and
writes one file. The order is the timestamp, then the place of the file in the list, then the line, so the same inputs give
the same output bytes on each run. Each secret is replaced again at write time, and the count of records with the marker
is the count that the command prints: `merged N record(s) into FILE, M marked redacted`. A secret never reaches the merged
file, and a secret given as a value on the command line is visible in the process list: use `--secret-file` for one that
matters.

## Pairing

`pair offer [--out FILE] [--spool DIR] [--key DIR]` makes an eight-word code and writes a signed offer with the key of
this install. With `--out FILE` the offer goes to that file, and with `--spool DIR` it goes into the spool for a peer that
comes later; the eight words go to stdout in each case. With neither, the offer goes to stdout and the words go to stderr.
The words are joined by `-`, as in `acorn-amber-anchor-apple-arrow-aspen-atlas-autumn`.

`pair accept (--offer FILE | --spool DIR) --used DIR [--out FILE] [--key DIR]` takes an offer, checks its signature, uses
the code once, and writes the record that binds the key of the peer to the key of this install. The code comes from
stdin:

```sh
podnet offline pair offer --out offer.txt > code.txt
podnet offline pair accept --offer offer.txt --used ./used < code.txt
```

A code is a file in `--used`; a code that is already there or an offer past its expiry exits 77 before any record is written. The second use of a code
is refused by the test `a_code_pairs_the_two_keys_once_through_a_file`. An offer in a spool is taken from the spool after
the accept, and a second accept from the same spool exits 66.

The code carries 48 bits of entropy hashed with PBKDF2-HMAC-SHA256 (`100_000` iterations) and is not a secret channel:
the offer is signed by the key of the offerer, so a changed offer does not verify.

## Replication

A replica log is a text file, one record on each line. A record names a key, a value, a vector clock and the key of its
actor. `replica merge --out FILE LOG... [--json] [--key DIR]` copies each record that the output does not hold, and never
removes or rewrites a record. For each key the state is the record that no other record dominates. Two records are
concurrent when neither clock covers the other; then the greater own counter wins, and the greater hex of the actor key
breaks an equal counter. The rule gives the same winner on each replica, whatever the order of the logs.

The text form prints `records N`, then one line `KEY HEX` for each key of the state, then one line `conflict KEY
winner=HASH losers=HASH,... winner_key=HEX` for each conflict. The JSON document has `records`, `state` (key to hex value)
and `conflicts` (key, `winner`, `winner_hash`, `losers`, `loser_hashes`). The hashes are SHA-256 of the value; they are
what a person compares on the two hosts. The test `two_replicas_diverge_and_converge_with_both_hashes` writes one key on each
replica, merges in both orders, and reads the same bytes from each order and the two hashes in each conflict.

## The `serial` carrier

The address is `serial:PATH[,BAUD]`, and the carrier name is `serial`:

```sh
podnet link listen serial:/dev/ttyUSB0,1200 --carrier serial --allow peers.txt --service raw=exec:/usr/bin/cat
podnet link dial serial:/dev/ttyUSB1,1200 --carrier serial --peer FINGERPRINT
```

The address in the flag is the same address: `--carrier serial:PATH[,BAUD]` takes the address in the flag, so the
positional address is then unused, and `podnet link carriers` shows the form. `BAUD` is one of the speeds that the
kernel names, 300 to 4,000,000; the default is 115200. The open is raw 8N1 with no flow control, `O_NOCTTY` and
`TIOCEXCL`, and the settings of the device are put back when the line closes. A second listener on a line that is held
exits 69.

The carrier sends AX.25 frames of 256 bytes at most in KISS framing: one control byte, one sequence and 255 bytes of
payload. A payload above the maximum is refused with the name `frame-too-large`; the carrier never splits it in silence.
The window is 4 frames (1 to 7). The retransmission timeout is two frame times at the baud rate, and a frame of 256 bytes
takes 2560 bits on an 8N1 line: at 1200 baud one frame takes 2.133 s and the timeout is 4.267 s. The rate of the carrier is
`BAUD / 10` bytes for each second, so 120 B/s at 1200 baud. There is no fixed floor under the timeout.

The path is point-to-point. Both ends open a device, the hello of the link proves the keys above it, and the line carries
one peer for as long as it is held; a second accept waits, because a second peer cannot come while the line is held. The
carrier does not pace its writes: the device runs at its speed. A pseudo-terminal ignores the speed, so a test that needs
1200 baud paces the wire itself; the test `an_echo_over_a_1200_baud_wire_comes_back_inside_three_seconds` does that, and
the echo of the service came back in 1.67 s on this host, against the limit of 3 s. The service is `/bin/cat`, because
a shell is refused as a service program by name.

It cannot do: address a station by an AX.25 callsign (there is no address field), share one line among three or more
ends, carry a frame above 256 bytes, or report a byte rate above the line speed.

## The `hf` carrier

The address is `hf:PATH[,BPS]`, and the carrier name is `hf`:

```sh
podnet link dial hf:/dev/ttyUSB0,1200 --carrier hf --peer FINGERPRINT
```

`BPS` is 300 to 3000 bit/s, and 1200 bit/s without one. The modem (ARDOP or VARA) is on a serial device at 115200 baud.
The speed is set on the modem when the session opens. The link is half duplex: a reception holds each transmission, and a
busy modem makes the attempt fail with the name `busy`, so the caller sends again when the channel is idle. The modem owns
the retransmission: the session ARQ is the deferring one (`Arq::deferring`), which sends each frame once and retransmits
nothing, so one chain is counted, not two. The rate of the carrier is `BPS / 8` bytes for each second, 150 B/s at
1200 bit/s.

The modem is driven by one blocking thread. A modem that does not answer the speed command blocks that thread, and the
link does not recover by itself; the operator ends the process, checks the modem and starts again.

It cannot do: a link while the modem is busy, a full-duplex exchange, a byte rate above 3000 bit/s, or a second peer on
one modem.
