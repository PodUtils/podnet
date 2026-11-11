# Hosted relays and mailboxes for podnet

A study of 2026-11-11, for the owner's ruling of that day (`docs/decisions.md`, Relays and
mailboxes): use each service that someone else already hosts, free and with no account, when it
fits the purpose. Marks: **read** (a document or the source), **probed** (a command against the
live service from a sandbox whose proxy allows only TCP 443 to some names), **inferred**. Each URL
was read on 2026-11-11.

podnet names no host of another party in its own code. Each service below is a fact of
`pod-services` of `PodUtils/podcrates`.

## The table

| Service | Role | Transport | TCP 443 | Account | Terms for a default | Limits | Server licence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `relay.magic-wormhole.io` | mailbox | WSS `/v1` on 443; WS on 4000 | yes (probed: HTTP/2 200, `Magic Wormhole Mailbox 0.8.0`; the default of the Python client since 2026-05-13, read) | no | "a personal server ... if it gets too expensive, I'll shut it down" (read) | none published | MIT |
| `transit.magic-wormhole.io` | transit relay (TCP) | `tcp:...:4001` | no (probed: 443 resets) | no | "please don't abuse" (read) | none published | MIT |
| `mailbox.winden.app`, `relay.winden.app` | mailbox, WS transit | WSS on 443 | yes (probed) | no | no terms found; Winden is a beta (read) | unknown | open source (not read) |
| `*.relay.n0.iroh.link` (4 hosts) | iroh relay | HTTPS 443 `/relay` | yes (probed) | no | "development and hobby use", "do not rely on it for production" (read) | a token bucket, numbers not published | MIT or Apache-2.0 |
| `ppng.io`, `piping.onrender.com` (piping-server) | stream relay (HTTP GET and PUT on a path) | HTTPS 443 | yes (probed end to end through the proxy; version 1.12.9) | no | no terms found | none documented; no storage | MIT |
| Nostr relays (`relay.damus.io`, `nos.lol`, `relay.primal.net`) | small messages | WSS 443 | yes (probed NIP-11 over HTTPS; no WS session run) | no | per relay; no terms found | 128 KiB to 1 MB a message | strfry |
| `ntfy.sh` | small messages | HTTPS, WSS 443 | yes (probed `/v1/health`) | no | IP bans for abuse (read) | 4,096 bytes a message, 250 a day | Apache-2.0 / GPL-2.0 |
| `free.pinggy.io` | reverse tunnel (HTTP, TCP, TLS) | SSH on 443 (read) | yes | no for the free plan | terms page not found | 60 minutes a tunnel | closed |
| Syncthing relay pool | stream relay | TCP 22067 (91 of 100), 443 (3) | poor; IP addresses only (probed) | no | for Syncthing | none published | MPL-2.0 |
| croc relay | stream relay | TCP 9009-9013 | no (read) | no | none found | none | MIT |
| bore.pub | reverse tunnel | TCP 7835 | no (read) | no | none found | none | MIT |
| ngrok free, Metered TURN | tunnel, TURN | 443 | yes | yes | n/a | 1 GB a month; 20 GB a month | closed |
| Tailscale DERP | relay | HTTPS 443 | yes | in effect (tailnet devices) | no (inferred) | n/a | BSD-3-Clause |
| libp2p circuit relays | stream relay | varies | varies | no | no policy found | 2 minutes, 128 KiB a connection (read) | MIT or Apache-2.0 |

Not checked: localhost.run and serveo (SSH ports not documented as 443), patchbay.pub (probed: a
certificate name mismatch, then HTTP 503).

## The result

1. **The mailbox of `podnet send` and `receive`:** `wss://relay.magic-wormhole.io/v1` on 443 as
   the default, the Winden mailbox as the second. One mailbox serves both peers: the sender picks
   it, and the code carries the choice. Risk: one person's server. podnet's own mailbox server
   (the owner's ruling) is the fallback. Open question: magic-wormhole issue #754 ("different
   mailboxes for the same nameplate across networks"); its body was not read.
2. **A transit carrier of last resort:** a piping-server relay (an HTTPS byte stream on 443, keyed
   by the transit key, end-to-end encrypted by the transit layer). Risk: no terms, unknown
   operators, a proxy that buffers a long GET or PUT.
3. **The iroh road:** the n0 relays, flagged "hobby use" in `pod-services`, after `pod-iroh`.
4. **A reverse tunnel on 443:** Pinggy free (SSH on 443, 60 minutes a tunnel, a reconnect loop).
   Its terms are not read.
5. **Not suitable:** croc and Syncthing (not 443, or IP addresses that a CONNECT proxy refuses),
   bore.pub (7835), ngrok and Metered (an account), DERP (tailnet only), libp2p public relays (no
   policy, tight limits), and `transit.magic-wormhole.io:4001` on a proxy-only host.
6. **Not opened:** a mailbox over Nostr or ntfy (no terms, no WS session proven, no delivery
   guarantee).

## Sources

- https://github.com/magic-wormhole/magic-wormhole (`src/wormhole/cli/public_relay.py`)
- https://github.com/magic-wormhole/magic-wormhole.rs (`src/transit.rs`)
- https://github.com/magic-wormhole/magic-wormhole-transit-relay
- https://magic-wormhole.readthedocs.io/en/latest/welcome.html
- https://github.com/LeastAuthority/winden
- https://docs.iroh.computer/relays/rate-limiting
- https://github.com/nwtgck/piping-server, https://ppng.io/help
- https://github.com/nostr-protocol/nips (`01.md`)
- https://github.com/binwiederhier/ntfy (`docs/publish.md`)
- https://docs.syncthing.net/users/strelaysrv.html, https://relays.syncthing.net/endpoint
- https://github.com/tailscale/tailscale (`cmd/derper/derper.go`)
- https://github.com/schollz/croc, https://github.com/ekzhang/bore
- https://pinggy.io/, https://localhost.run/docs/forever-free/, https://serveo.net/
- https://ngrok.com/docs/pricing-limits/free-plan-limits/
- https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/trycloudflare/

The Metered and libp2p rows rest on search summaries, not primary pages.
