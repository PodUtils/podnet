# Guest-wire router on pod-netstack: the constraints

> Measured on 2026-11-12 with `pod-netstack` v0.27.0 (smoltcp 0.14.0) as `Cargo.lock` holds it. Record entry:
> T-176. Router entry: T-248. Tests: `crates/podnet-vnet/tests/guest_wire.rs` (12 tests, with the helpers in
> `crates/podnet-vnet/tests/wire/mod.rs`). Run: `cargo test -p podnet-vnet --test guest_wire`.

## 1. Set-up

- The router is a `NetStack::over_link` with `StackConfig::any_ip`. It uses `listen_any` and `bind_udp_any`.
- The guest is a second stack with the subnet `0.0.0.0/0` and the address `10.66.0.4`.
- A pump joins the two link queues. The pump of the guest can hold each SYN (the `HeldSyn` of the netns
  backend, on the wire). It counts the RST segments in each direction.
- The default settings: MTU 1280, buffers of 64 KiB, `max_connections` 4096, `linger` 60 s, `udp_packets` 64, `udp_payload` 64 KiB.

## 2. Constraints

Each row gives the rule for a router, the evidence, and the test.

### C1. A tuple that the router holds still

| Case | Measure | Rule |
| --- | --- | --- |
| Both sides closed, the guest uses the tuple again at once | The handshake ends in under 500 ms, 3 rounds. The stack reaps `TimeWait` and `Closing` as closed (`close_and_check`, `service.rs`), so the 10 s `TimeWait` delay of the prototype does not exist here. | None. Do not add a `TimeWait` timer. |
| The guest forgot the flow (a reboot) and uses the same address and port | The old socket answers the SYN with an ACK. The guest answers with a RST. The guest sends the SYN again 1.0 s after the first, and the handshake of the new flow ends. Total: 1.0 s to 3 s. The RST of the guest reaches the old socket: the router app of the old flow reads `ConnectionReset` (v0.27.0; v0.26.0 gave an end of file). | A new SYN for a stale tuple needs one SYN retransmit of the guest. The router must not drop the SYN of a tuple that it holds. Treat `ConnectionReset` and a read of 0 bytes as the end of a flow that the guest may have reset. |

Tests: `guest_wire_closed_tuple_is_free_at_once`, `guest_wire_stale_tuple_is_released_by_a_new_syn`.

### C2. The dial and the handshake

| Fact | Measure | Rule |
| --- | --- | --- |
| The stack ends the handshake before the app sees the flow | The guest has `established` with no `accept` call. | The stack alone cannot dial first. `Admit` is asked at bind, connect and UDP send, never at an inbound SYN. |
| A hold of the SYN on the wire works | The guest has no handshake while the pump holds its SYN. The guest sends the SYN again at 1 s (2 held copies at 1.2 s). After the release, the handshake ends in under 500 ms, and the router makes one flow. | Dial at the wire: parse the SYN (`pod_hold::packet`), hold it, key the hold by the tuple (a copy is a retransmit), dial, then release the SYN into `link_in`. The dial time limit must be under the connect time of the guest (Linux: 6 retransmits, about 127 s). A failed dial answers the SYN with a RST (`rst_reply_for_syn` of `pod_hold::packet`): the guest sees `ConnectionRefused`. This reply is not proven here. |
| No flow is lost after the handshake | 60 SYNs, no `accept`: 60 handshakes end, 60 flows reach the accept queue, 0 are counted in `NetStats::dropped`, 0 RST leave. `StackConfig::accept_pool` and the queue of 32 flows are gone (v0.27.0). | Call `accept` in a task that never waits for a dial. Keep the dial in a spawned task. |
| `max_connections` limits the flows | With `max_connections` 16 and 40 SYNs: 16 handshakes end, 16 flows reach `accept`, the other SYNs are dropped and counted in `NetStats::dropped`, 0 RST leave. The guest keeps the flow in `SynSent` and sends the SYN again. The limit counts the open and the half-open connections (v0.27.0). | Set `max_connections` for the load of the router. A SYN past the limit waits for a free slot; it gets no refusal. |
| `TcpStream::abort` sends its RST | The router accepts and aborts. The guest reads `ConnectionReset`. One RST crosses the wire down (v0.27.0; v0.26.0 sent none). | A dial that fails after the handshake resets the guest with `abort`. A `shutdown` sends a FIN instead. |
| A drop of the stream sends the queue | After `write_all` of 1 MB and `drop`, the guest reads 1 MB, then the end of the stream. `shutdown` with a held handle delivers every byte too. After `StackConfig::linger` (60 s) with no progress, the stack sends a RST (v0.27.0; closes gap 2 of PodUtils/podnet#457). | A drop is safe with data in flight. |

Tests: `guest_wire_handshake_ends_before_the_app_accepts`, `guest_wire_held_syn_dials_before_the_handshake`,
`guest_wire_a_burst_of_flows_all_reach_accept`, `guest_wire_max_connections_drops_a_syn_with_no_reset`,
`guest_wire_abort_after_the_handshake_resets_the_guest`, `guest_wire_a_drop_delivers_the_tail_like_a_held_shutdown`.

### C3. The UDP socket and the wire

| Fact | Measure | Rule |
| --- | --- | --- |
| Bounded queue | A burst of 500 datagrams with no reader: 113 reach the app, in order. The two bounds are the channel to the app (64) and the smoltcp buffer (`udp_packets`, 64); the sum is not exact. The rest is lost. | The router keeps its own bounded queue and reads the socket at once. |
| No count of the loss | After the burst: `NetStats::dropped` 0, `rx_dropped` 0, 500 packets in `rx_packets`. `service_udp` takes a datagram from smoltcp (`recv_slice`), and loses it when `try_send` to the app fails. | Count in the router. The stack gives no signal. |
| No stop-reading signal | `PacketQueue::is_full()` is a poll. A push to a full queue drops and counts the packet (`rx_dropped` of the link queue, 4096 packets). No wait for room. No socket-level signal reaches the pump. | The pump polls `is_full()` before it reads the wire, and stops reading the TUN while the queue is full. |
| Datagram size | A datagram over `mtu - 28` bytes is lost, up and down: 1252 passes and 1253 fails at MTU 1280; 1472 passes and 1473 fails at MTU 1500. `send_to` returns `Ok`. The guest stack counts the datagram as sent. No counter shows the loss. The sizes 4000 and 65507 fail the same way. | Set the MTU of the router to the MTU of the guest. A guest that sends a larger datagram needs IPv4 fragmentation (below). |
| IPv4 fragmentation | With the features `proto-ipv4-fragmentation`, `reassembly-buffer-size-65536` and `fragmentation-buffer-size-65536` on the `smoltcp` dependency of `podnet-vnet` (Cargo feature unification, no change of podcrates), the sizes 1253, 4000 and 65507 all pass, up and down, at MTU 1280. Measured once by hand; not in the repository. | Decided in T-248 (2026-11-12): the feature `ipv4-fragmentation` of `pod-netstack` (v0.28.0) is on, in the root crate and in `podnet-vnet`. It applies to each interface of the binary (WARP too) and costs a buffer of 64 KiB for each interface; smoltcp joins one datagram at a time. Test: `guest_wire_udp_over_the_mtu_is_carried_with_fragmentation`; `guest_wire_udp_carries_a_datagram_over_the_mtu` in the router. |
| Buffer of 65535 bytes | `udp_payload` is 64 KiB (65536), so one datagram of 65535 bytes fits an empty buffer; the metadata ring is separate (`udp_packets`). With fragmentation on, 65507 passed alone (one datagram in the buffer). | Keep `udp_payload` at 65536 or more. The oversized-datagram counter must be the router's. |
| Partial write of a stream | A TCP socket buffer of 4 KiB takes 2 000 000 bytes with a reader that sleeps: the bytes arrive equal. `Conn::pending` keeps the offset. | None for TCP. The record stream of the UDP flow (`udpflow.rs`) keeps its test `a_partial_stream_write_keeps_the_record_stream_aligned`. |

Tests: `guest_wire_stack_udp_queue_is_bounded_and_silent`, `guest_wire_udp_over_the_mtu_is_carried_with_fragmentation`,
`guest_wire_partial_write_keeps_the_byte_order`, `guest_wire_link_queue_says_full_and_has_no_wait_for_room`.

## 3. Asks for pod-netstack

`pod-netstack` v0.27.0 closed the three gaps of PodUtils/podnet#457 (the close of a `TcpListener`, no loss of
queued bytes at the drop of a `TcpStream`, no RST from the accept pool for a burst of SYNs). It also closed
asks 1 and 2 of PodUtils/podnet#464. Asks 3 to 5 stay open.

Closed in v0.27.0:

1. `TcpStream::abort` sends a RST. Test: `guest_wire_abort_after_the_handshake_resets_the_guest`.
2. A flow does not get lost after the handshake: `accept_pool` and the accept queue are gone, and a SYN past
   `max_connections` is dropped before the handshake, so the guest sends it again. The ask for a RST and for the
   setting `accept_queue` has no object now. Tests: `guest_wire_a_burst_of_flows_all_reach_accept`,
   `guest_wire_max_connections_drops_a_syn_with_no_reset`.

Open (PodUtils/podnet#464):

3. UDP counters in `NetStats`: a datagram lost at the full channel to the app, a datagram lost at a full
   smoltcp send buffer, and a datagram that the send path cannot carry (over the MTU).
4. `UdpSocket::send_to` returns an error (`InvalidInput`) for a datagram over `mtu - 28` when the stack has no
   fragmentation, or over `udp_payload`.
5. `service_udp` does not take a datagram from the smoltcp socket (`peek_slice`, then `recv_slice`) when the
   channel to the app is full: smoltcp then applies its own bound, and no datagram is lost in the reactor.

## 4. Not proven

- A failed dial that answers a held SYN with `rst_reply_for_syn`.
- A guest with a real kernel TCP (the guest here is `pod-netstack`: its SYN retransmit of 1 s and its
  challenge-ACK answer are those of smoltcp, and a Linux guest uses 1 s and the same RST rule).
- Load: the numbers of C3 hold for the default settings and one flow.
