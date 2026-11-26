# The session end to end

This page describes `podnet tunnel --e2e` and `podnet tunnel --e2e-inspect`
(GitHub #496). The session runs a Noise XX handshake inside the carrier
before any application byte. The relay forwards the two streams and cannot
open the handshake, the destination or a record.

The commands in this page run two local ends. They do not contact a live
relay.

## The two ends

The near end connects to two addresses of the relay. The far end listens on
two addresses of the relay. The data stream carries the handshake and the
records. The control stream carries one sealed destination.

```sh
# The far end, where the destination is.
podnet tunnel --e2e \
  --listen 127.0.0.1:9001 --control-listen 127.0.0.1:9002 \
  --pair relay-a/pair-1 \
  --destination db.internal:5432 --service postgres \
  --peer 6f1c...c2

# The near end, which supplies the application on stdin and stdout.
podnet tunnel --e2e \
  --connect 127.0.0.1:9001 --control 127.0.0.1:9002 \
  --pair relay-a/pair-1 \
  --destination db.internal:5432 --service postgres \
  --peer 91ab...7d
```

`--pair` is the pair that the relay advertises, as `RELAY/PAIR`. Both ends
bind the handshake to that pair, so a session for another pair fails the
handshake.

`--peer` admits one Ed25519 key and may repeat. An empty allow list admits
no peer. The handshake proves the peer key before the session continues.

## The order on the wire

1. The near end starts the Noise XX handshake on the data stream.
2. Both ends bind the channel to the advertised pair.
3. Both ends agree on the record suite.
4. The near end seals the destination in a `pod_e2e::Destination` envelope
   and writes it on the control stream. The far end opens it with the same
   channel, so the envelope is bound to the pair.
5. Each end seals application bytes in records on the data stream.

The destination envelope is fixed at 301 bytes. The relay routes the
prelude by its 16-byte identifier. It holds no key that opens the envelope.
The far end learns the destination only after the handshake.

## The record layer (KEY-1 to KEY-4)

The record keys come from the end-to-end handshake exporter. They never
come from a TLS exporter of a hop. A relay that terminates TLS still holds
no record key.

The header is KEY-2. It is exactly 24 bytes, and the tag is 16 bytes:

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 1 | version, 1 |
| 1 | 1 | kind, 0 for data and 1 for the end of a direction |
| 2 | 2 | flags, big endian; bit 0 is the parity of the key's epoch |
| 4 | 4 | stream, big endian |
| 8 | 8 | counter, big endian |
| 16 | 4 | body length, the payload and the padding, big endian |
| 20 | 2 | padding, the bytes at the end of the body, big endian |
| 22 | 2 | reserved, zero |

The nonce is the 4-byte direction prefix and the 64-bit counter: 12 bytes
in all. The header and the body go in as associated data and plaintext, so
a changed header fails the tag.

The default suite is `chacha20-poly1305`. `--suite aes-256-gcm` selects the
other suite, and the flag may repeat to offer both. The suite is agreed
before the first record. A mismatch exits 78 before any record is sent.

The layer rekeys at 2^31 records or 1 GiB, whichever comes first, and it
also rekeys after one hour. It checks the ceiling before it seals a record.
When the bytes already sent reach the ceiling, it takes the new key before
that record, so the first record that starts at or above the ceiling uses
the new epoch. A record that crosses the ceiling from below finishes the
old epoch. The 1 GiB test ends a record exactly at the ceiling and sends
the next one with the new key.

## What each hop learns

`podnet tunnel --e2e-inspect` prints this table:

| Hop | Client address | Relay pair | Volume | Destination | Service | Peer identity |
| --- | --- | --- | --- | --- | --- | --- |
| first hop | the client address | yes | yes | - | - | - |
| relay | - | yes | yes | - | - | - |
| far hop | - | yes | yes | host:port | the service name | the peer key |

No hop sees the plaintext. No hop sees a record key. The first hop learns
the client address and the pair that it forwards to, because it must route
the streams. The relay learns its pair and the volume, and nothing else.
The far hop learns the destination, the service and the peer identity,
because it dials the destination.

## Inspect

```sh
podnet tunnel --e2e-inspect --pair relay-a/pair-1 \
  --destination db.internal:5432 --service postgres \
  --peer 91ab...7d
```

The command runs one handshake in memory and opens no socket. It prints the
hop table above, then the exchange as the relay sees it:

```text
tunnel --inspect, relay:
  prelude id: 00112233445566778899aabbccddeeff
  prelude: 301 bytes, sha256 ...
  wire: 45 bytes, sha256 ...
```

The crate `pod-e2e` writes the heading of the relay view, and it keeps the
old flag name `tunnel --inspect`. Since T-244, `tunnel --inspect` starts the
inspector of the HTTP exchanges ([tunnels](tunnels.md#the-inspector)).

The relay view holds the prelude identifier, the prelude size and digest,
and the size and digest of the first record. It holds no destination, no
service name and no key.

Without `--pair` and `--destination`, `--e2e-inspect` uses `relay/pair` and
`example.invalid:0`, so the table renders without an end to connect.

## The compatibility path

A relay that expects the `/connect/<host>/<port>` form can receive the
destination in clear on the control stream. This path is off by default.

```sh
podnet tunnel --e2e --compat-destination ...
```

The near end prints:

```text
warning: --compat-destination sends /connect/HOST/PORT in clear: every hop learns the destination
```

The far end refuses a clear destination unless it also has
`--compat-destination`, with exit 78. With the flag, it prints its own
warning. Every hop of the control stream learns the destination and the
service in this mode. The sealed envelope is the path for a session that
does not need the old form.

## Tokens

`--e2e-token TOKEN` with `--e2e-token-seed-file FILE` checks a token for one
relay and one service. The file holds the 64 hex digits of the 32-byte seed
on its first line. A token for relay A is refused at relay B, and a token
for one service is refused for another. A refusal is a permission fault.

## Tests

`tests/e2e.rs` proves the properties of this page:

* `the_relay_dump_holds_no_plaintext_destination_service_or_key`
* `a_splice_between_two_sessions_fails`
* `one_gib_loopback_transfer_rekeys_and_completes`
* `a_token_does_not_cross_a_relay_or_a_service`
* `a_suite_mismatch_exits_78_before_a_record`
* `the_record_header_is_key2`
* `the_clear_destination_is_refused_without_the_flag`
* `the_clear_destination_prints_a_leak_warning`
* `inspect_prints_the_hop_table_and_the_relay_view`

The 1 GiB test runs two ends over a loopback TCP socket. It sends one
record past 1 GiB, asserts that the far end moved to epoch 1, and prints the
duration. The test measures one record past the ceiling, because the layer
checks the ceiling before it seals.

## Related

- `docs/tunnels.md` describes the Cloudflare tunnel and its origins.
- `docs/limits-carriers.md` describes the limits of the packet sidecar.
- `docs/relay.md` describes the relay.
