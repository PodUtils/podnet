# Limits of the packet carriers

The packet sidecar (GitHub #495) holds the packet path of `podnet sidecar`.
It is behind the build feature `sidecar`, which is off by default. A default
build holds no packet path, no raw socket and no TUN device (LIM-2).

Each limit below is a condition that defeats the behaviour, and the check
that detects the defeat. A limit with no detection is a claim, so the crate
also fails a test when a split behaviour has no limit on this page.

## The limits

| Behaviour | Condition that triggers the limit | Check that detects it |
| --- | --- | --- |
| split-at-an-offset | a receiver that reassembles the IP fragments, or a TCP stack that reassembles the byte stream in order | Simulator::split with Policy::strict returns SplitError::Reassembled, and the appended bytes arrive whole |
| disorder | a receiver that reassembles the byte stream and tolerates a gap, as every TCP stack does | Simulator::split with Policy::strict returns SplitError::Reassembled, and the delivered stream is unchanged |
| fake-ttl | a path that honours the TTL, decrements it and drops the segment at zero | Simulator::split with Policy::strict returns SplitError::TimeExceeded |
| tamper-a-checksum | a receiver that requires a valid TCP checksum, or a middlebox that repairs the checksum | Simulator::split with Policy::strict returns SplitError::ChecksumRejected; the endpoint also counts a bad checksum |
| pad-the-first-segment | a receiver that reassembles the stream and compares it with the original | Simulator::split with Policy::strict returns SplitError::Reassembled; the padding is visible in SplitRun::padding |
| fake-tcp | a path that refuses UDP by protocol number, or repairs the checksum of a frame | Simulator::udp_echo returns a refusal, and FakeTcp::recv returns FakeTcpError::BadChecksum when a frame is changed |
| raw-socket-probe | a host with no CAP_NET_RAW and no uid 0 | probe::raw_capability returns a Refusal that names the error, and Probe reports Verdict::Unknown |
| silent-probe | a path that drops every unusual segment and answers nothing | the verdict is Verdict::Agrees with an empty response, so silence is not evidence of a disagreement |
| bond-one-flow | a session that is one flow, so the per-flow scheduler puts it on one link | Bond::estimate of the flow is 10 of a sum of 11 on links at 10:1, which is 91 percent of the sum |
| bond-move | a link that fails, which moves each of its flows once | Bond::moves counts each move, and a flow on a live link does not move |

## The commands and their limits

`podnet sidecar probe --path raw` prints `could not determine` when the host
refuses a raw socket. That is the limit `raw-socket-probe`, and the verdict
is never a pass.

`podnet sidecar probe --path silent` prints `could not determine` too. The
row above states the crate's reading of silence, where `Probe::run` returns
`Verdict::Agrees` because a silent path is not evidence of interference.
The command maps that to `Verdict::Unknown`, because no measurement is not
a pass (PROBE-2).

`podnet sidecar split` prints each split behaviour with its defeat and its
limit. The row above is the whole behaviour, and the strict policy is the
check.

`podnet sidecar faketcp` prints a named skip when the host refuses a raw
socket. The simulated middlebox is local, so the echo itself needs no
privilege. Fake TCP is the last resort of the library: a real TCP carrier
with message framing is preferred. Its costs are one extra round trip for
the first frame, a TTL that a path sees and can act on, and a checksum that
a middlebox may repair.

`podnet sidecar bond` prints the flow, the sum and the number of moves. A
flow on a live link does not move, and 10 of 11 is within 20 percent of the
sum.

## Related

- `docs/link.md` describes the link carriers.
- `docs/tunnels.md` describes the Cloudflare tunnel.
- `docs/e2e.md` describes the session end to end of GitHub #496.
