# The direction split of a TCP flow (T-179, GitHub #375)

This document is the design of the TCP part of the direction split of a router group. The UDP part is built
(`splits:` of a router file, [router.md](../../router.md)). The owner ruled the split an option of a group, off by
default (`docs/decisions.md`, 2026-11-11, Direction split).

## The problem

One TCP connection carries both directions. To put the request bytes on one uplink and the response bytes on
another, a flow needs two connections, and a far end that joins them into one byte stream. In 2026-11-11 podnet had
no such far end: the relay and the proxies end one connection.

## The far end

The split session of the link (T-302, [link.md](../../link.md), "A split session") is that far end. `pod_link::split`
joins an up carrier and a down carrier into one session: the bytes of the dialer go up on one half, the bytes of the
listener come down on the other, and the credits of each direction ride on the other half. A lost half ends the
session at each end (the ruling of 2026-11-14, "The split session: a lost half ends the session"). The streams of the
link run above the session, so one session carries many TCP flows.

## The design

1. **The far host** runs `podnet link listen 'UP|DOWN' --up-carrier A --down-carrier B --allow FILE` with a service
   that forwards each stream to its target (the services of a peer, `link_service`). It is the exit of each flow: the
   target sees the address of the far host. This is the "exit table": the link keeps it, one stream for each flow.
2. **The router** gets a value `tcp` of `splits:` (today it is refused with `split::TCP_REFUSAL`):
   `NAME: up=MEMBER down=MEMBER tcp=FINGERPRINT@UP|DOWN`. The router dials one split session to the far host. The
   up half goes through the way of the member `up`, and the down half through the way of the member `down`.
3. **The way of each half.** The carriers `udp`, `dns` and `quic` of `pod-link` take a way of their own
   (`with_way`). podnet's `tcp` carrier takes the egress chain. `podnet::link_split::half` gets the way of a member as
   an argument and gives it to the carrier of the half. A carrier with no way (a half that cannot take a member) is
   refused when the router file is read.
4. **A TCP flow of the group** opens one stream of the session (`peer:FINGERPRINT/SERVICE` with the target in the
   request of the stream), not a connection of a member. The flow id is the id of the stream. The direction bit is
   the half: the session puts the bytes of the flow on the halves by their direction. The cleanup is the end of the
   stream (FIN or RST of either side), which the link already does.
5. **The fallback.** A session that lost a half ends (no promotion). The router then sends each new TCP flow of the
   group by the pick of the policy, as with no split, counts the fallback, and dials a new session after a back-off.
   A flow that was open on the lost session ends; the client sees a reset. The resumable layer (`pod-session`) can
   keep a flow over a new session; that is a later step.
6. **The status.** The key `split` of the group gets `mode: tcp`, the session (up, down, the far fingerprint), the
   open streams, and the fallbacks.
7. **The errors.** A `tcp` value with no far address, a far key that the allow list does not hold, and a half whose
   carrier takes no way each exit 78 at the read of the router file, with the key `splits.NAME`.

## The limits

- The far host must run podnet. A split to a host that does not run podnet is not possible: the joining is the job
  of the far end.
- The slower half sets the throughput of its direction, and the credits of each direction ride on the other half:
  a slow up half delays the credits of the down direction too.
- A flow of a split group pays the overhead of the link (the hello, the frames of a stream) on top of each carrier.

## The work

The entry T-611 builds it, with the start condition that `podnet::link_split::half` takes a way for each half.
Until then a TCP flow of a split group keeps one member, as the pick gives it.
