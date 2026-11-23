use super::*;

const KEY: UdpKey = (
    Ipv4Addr::new(10, 66, 255, 255),
    40000,
    Ipv4Addr::new(93, 184, 216, 34),
    9999,
);

#[test]
fn netns_udp_flow_is_reaped() {
    let start = Instant::now();
    let mut table = FlowTable::new(Duration::from_secs(1), 2);
    table.insert(KEY, "a", start);
    assert!(table.reap(start + Duration::from_millis(500)).is_empty());
    // A datagram keeps the flow.
    assert!(table
        .touch(&KEY, start + Duration::from_millis(900))
        .is_some());
    assert!(table.reap(start + Duration::from_millis(1500)).is_empty());
    assert_eq!(table.reap(start + Duration::from_millis(1900)), vec!["a"]);
    assert_eq!(table.len(), 0);

    // At the cap, a new flow ends the flow that was idle longest.
    let other = |port| (KEY.0, port, KEY.2, KEY.3);
    table.insert(other(1), "one", start);
    table.insert(other(2), "two", start + Duration::from_millis(10));
    table.touch(&other(1), start + Duration::from_millis(20));
    let evicted = table.insert(other(3), "three", start + Duration::from_millis(30));
    assert_eq!(evicted, Some("two"));
    assert_eq!(table.len(), 2);
}

#[tokio::test]
async fn an_empty_datagram_survives() {
    // The frame code of the stream.
    let wrapped = pod_proxy::udp::wrap(b"", IpAddr::V4(KEY.2), KEY.3);
    let mut wire = Vec::new();
    pod_proxy::udp::write_frame(&mut wire, &wrapped)
        .await
        .unwrap();
    assert_eq!(wire.len(), 2 + 10);
    let mut read = wire.as_slice();
    let datagram = pod_proxy::udp::read_frame(&mut read)
        .await
        .unwrap()
        .unwrap();
    let (_, off) = crate::inbox::udp_source(&datagram).unwrap();
    assert_eq!(off, datagram.len(), "the payload is empty");
    // The packet of the reply, and the parse of the same packet.
    let packets = reply_packets(KEY, &datagram[off..], TUN_MTU, 1);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].len(), 28);
    let swapped = (KEY.2, KEY.3, KEY.0, KEY.1);
    let (key, payload) = parse_udp(&packets[0]).unwrap();
    assert_eq!(key, swapped);
    assert!(payload.is_empty());
}

#[test]
fn a_large_reply_becomes_fragments() {
    let payload: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
    let packets = reply_packets(KEY, &payload, TUN_MTU, 7);
    assert_eq!(packets.len(), 3);
    let mut joined = Vec::new();
    for (i, pkt) in packets.iter().enumerate() {
        assert!(pkt.len() <= TUN_MTU);
        assert_eq!(ipv4_checksum(&pkt[..20]), 0);
        let flags = u16::from_be_bytes([pkt[6], pkt[7]]);
        assert_eq!(flags & 0x2000 != 0, i < 2, "more fragments");
        assert_eq!(usize::from(flags & 0x1fff) * 8, joined.len());
        joined.extend_from_slice(&pkt[20..]);
    }
    assert_eq!(joined.len(), 8 + payload.len());
    assert_eq!(&joined[8..], payload.as_slice());
}

// GitHub #372: a stream that takes 5 bytes for each write does not cut a
// frame, and the datagrams come back in their order.
#[tokio::test]
async fn a_partial_stream_write_keeps_the_record_stream_aligned() {
    let (near, far) = tokio::io::duplex(5);
    let (queue, datagrams) = tokio::sync::mpsc::channel(UDP_QUEUE);
    let (replies_tx, mut replies) = tokio::sync::mpsc::channel(UDP_REPLIES);
    let ends = Ends {
        key: KEY,
        name: Some("echo.example".into()),
        replies: replies_tx,
    };
    let flow = tokio::spawn(carry_in_stream(Box::new(near), datagrams, ends));
    // The proxy: read each frame and send it back.
    tokio::spawn(async move {
        let (mut read, mut write) = tokio::io::split(far);
        while let Ok(Some(datagram)) = pod_proxy::udp::read_frame(&mut read).await {
            assert_eq!(&datagram[..5], &[0, 0, 0, 3, 12], "a name header");
            pod_proxy::udp::write_frame(&mut write, &datagram)
                .await
                .unwrap();
        }
    });
    let count = 200usize;
    let sender = tokio::spawn(async move {
        for i in 0..count {
            let payload = vec![i as u8; 1 + i * 7 % 1200];
            queue.send(payload).await.unwrap();
        }
        queue
    });
    for i in 0..count {
        let (key, payload) = replies.recv().await.unwrap();
        assert_eq!(key, KEY);
        assert_eq!(payload, vec![i as u8; 1 + i * 7 % 1200], "datagram {i}");
    }
    drop(sender.await.unwrap());
    let _ = flow.await;
}

/// The IPv4 fragments of one UDP datagram, in order, for `mtu`-sized packets.
fn fragments_of(id: u16, key: UdpKey, payload: &[u8], mtu: usize) -> Vec<Vec<u8>> {
    let (src, sport, dst, dport) = key;
    let mut udp = Vec::with_capacity(8 + payload.len());
    udp.extend_from_slice(&sport.to_be_bytes());
    udp.extend_from_slice(&dport.to_be_bytes());
    udp.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(payload);
    let room = (mtu - 20) & !7;
    let mut packets = Vec::new();
    let mut offset = 0;
    loop {
        let end = (offset + room).min(udp.len());
        let more = end < udp.len();
        let total = 20 + end - offset;
        let flags = (u16::from(more) << 13) | (offset / 8) as u16;
        let mut pkt = Vec::with_capacity(total);
        pkt.extend_from_slice(&[0x45, 0]);
        pkt.extend_from_slice(&(total as u16).to_be_bytes());
        pkt.extend_from_slice(&id.to_be_bytes());
        pkt.extend_from_slice(&flags.to_be_bytes());
        pkt.extend_from_slice(&[64, libc::IPPROTO_UDP as u8, 0, 0]);
        pkt.extend_from_slice(&src.octets());
        pkt.extend_from_slice(&dst.octets());
        pkt.extend_from_slice(&udp[offset..end]);
        packets.push(pkt);
        if !more {
            return packets;
        }
        offset = end;
    }
}

#[test]
fn a_fragmented_datagram_is_reassembled() {
    let payload: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    let packets = fragments_of(7, KEY, &payload, TUN_MTU);
    assert_eq!(packets.len(), 3);
    let now = Instant::now();
    let mut table = Fragments::new();
    let mut done = Vec::new();
    for pkt in &packets {
        if let Fragment::Datagram(key, data) = table.push(pkt, now) {
            done.push((key, data));
        }
    }
    assert_eq!(done.len(), 1, "one datagram, after the last fragment");
    assert_eq!(done[0].0, KEY);
    assert_eq!(done[0].1, payload);
    assert_eq!(table.len(), 0);
}

#[test]
fn fragments_out_of_order_and_a_duplicate_still_join() {
    let payload: Vec<u8> = (0..4000u32).map(|i| (i % 241) as u8).collect();
    let packets = fragments_of(9, KEY, &payload, TUN_MTU);
    assert_eq!(packets.len(), 3);
    let now = Instant::now();
    let mut table = Fragments::new();
    assert!(matches!(table.push(&packets[2], now), Fragment::Pending));
    assert!(matches!(table.push(&packets[0], now), Fragment::Pending));
    assert!(matches!(table.push(&packets[0], now), Fragment::Pending));
    match table.push(&packets[1], now) {
        Fragment::Datagram(key, data) => {
            assert_eq!(key, KEY);
            assert_eq!(data, payload);
        }
        _ => panic!("the last missing fragment must complete the datagram"),
    }
    assert_eq!(table.len(), 0);
}

#[test]
fn a_gap_keeps_the_datagram_pending() {
    let payload: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
    let packets = fragments_of(11, KEY, &payload, TUN_MTU);
    let now = Instant::now();
    let mut table = Fragments::new();
    assert!(matches!(table.push(&packets[0], now), Fragment::Pending));
    assert!(matches!(table.push(&packets[2], now), Fragment::Pending));
    assert_eq!(table.len(), 1, "the set waits for the middle fragment");
    match table.push(&packets[1], now) {
        Fragment::Datagram(_, data) => assert_eq!(data, payload),
        _ => panic!("the middle fragment completes the datagram"),
    }
}

#[test]
fn an_overlapping_fragment_drops_the_set() {
    let payload: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
    let packets = fragments_of(13, KEY, &payload, TUN_MTU);
    let now = Instant::now();
    let mut table = Fragments::new();
    assert!(matches!(table.push(&packets[0], now), Fragment::Pending));
    // A second fragment that starts inside the first one, with other bytes.
    let mut overlap = packets[1].clone();
    let bits = u16::from_be_bytes([overlap[6], overlap[7]]);
    let flags = (bits & 0x2000) | 1; // offset 8 bytes, inside the first
    overlap[6..8].copy_from_slice(&flags.to_be_bytes());
    assert!(matches!(table.push(&overlap, now), Fragment::Dropped(_)));
    assert_eq!(table.len(), 0);
}

#[test]
fn a_fragment_set_expires() {
    let payload: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
    let packets = fragments_of(15, KEY, &payload, TUN_MTU);
    let start = Instant::now();
    let mut table = Fragments::new();
    assert!(matches!(table.push(&packets[0], start), Fragment::Pending));
    table.reap(start + FRAGMENT_IDLE - Duration::from_millis(1));
    assert_eq!(table.len(), 1);
    table.reap(start + FRAGMENT_IDLE);
    assert_eq!(table.len(), 0);
}

#[test]
fn a_whole_datagram_is_not_a_fragment() {
    let payload = b"hello";
    let packets = fragments_of(17, KEY, payload, TUN_MTU);
    assert_eq!(packets.len(), 1);
    assert!(matches!(
        Fragments::new().push(&packets[0], Instant::now()),
        Fragment::Whole
    ));
}

/// One IPv4 fragment of a UDP datagram, with the given offset and length.
fn raw_fragment(
    id: u16,
    key: UdpKey,
    offset: usize,
    more: bool,
    data: &[u8],
    dst: Ipv4Addr,
) -> Vec<u8> {
    let (src, sport, _, dport) = key;
    let total = 20 + data.len();
    let flags = (u16::from(more) << 13) | (offset / 8) as u16;
    let mut pkt = Vec::with_capacity(total);
    pkt.extend_from_slice(&[0x45, 0]);
    pkt.extend_from_slice(&(total as u16).to_be_bytes());
    pkt.extend_from_slice(&id.to_be_bytes());
    pkt.extend_from_slice(&flags.to_be_bytes());
    pkt.extend_from_slice(&[64, libc::IPPROTO_UDP as u8, 0, 0]);
    pkt.extend_from_slice(&src.octets());
    pkt.extend_from_slice(&dst.octets());
    pkt.extend_from_slice(data);
    if data.len() >= 8 {
        pkt.extend_from_slice(&sport.to_be_bytes());
        pkt.extend_from_slice(&dport.to_be_bytes());
    }
    pkt
}

#[test]
fn an_empty_fragment_is_dropped_and_stores_nothing() {
    let now = Instant::now();
    let mut table = Fragments::new();
    let empty = raw_fragment(21, KEY, 0, true, &[], KEY.2);
    assert!(matches!(table.push(&empty, now), Fragment::Dropped(_)));
    assert_eq!(table.len(), 0);
    // A run of them cannot grow the set either.
    for _ in 0..MAX_PIECES + 8 {
        assert!(matches!(table.push(&empty, now), Fragment::Dropped(_)));
    }
    assert_eq!(table.len(), 0);
}

#[test]
fn a_fragment_beyond_the_recorded_length_drops_the_set() {
    let now = Instant::now();
    let mut table = Fragments::new();
    let first = raw_fragment(23, KEY, 0, true, &[1u8; 96], KEY.2);
    assert!(matches!(table.push(&first, now), Fragment::Pending));
    let last = raw_fragment(23, KEY, 8 * 1_000, false, &[2u8; 96], KEY.2);
    assert!(matches!(table.push(&last, now), Fragment::Pending));
    assert_eq!(table.len(), 1);
    // A fragment that starts past the total the last one recorded.
    let late = raw_fragment(23, KEY, 8 * 1_200, true, &[3u8; 96], KEY.2);
    assert!(matches!(table.push(&late, now), Fragment::Dropped(_)));
    assert_eq!(table.len(), 0, "the whole set goes, not just the fragment");
}

#[test]
fn a_duplicate_refreshes_the_idle_timer() {
    let payload: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
    let packets = fragments_of(25, KEY, &payload, TUN_MTU);
    let start = Instant::now();
    let mut table = Fragments::new();
    assert!(matches!(table.push(&packets[0], start), Fragment::Pending));
    // 29 seconds later the same fragment repeats, and it still waits.
    assert!(matches!(
        table.push(&packets[0], start + Duration::from_secs(29)),
        Fragment::Pending
    ));
    // Reaped two seconds after the repeat, so the repeat held the set.
    table.reap(start + Duration::from_secs(31));
    assert_eq!(table.len(), 1);
    match table.push(&packets[1], start + Duration::from_secs(31)) {
        Fragment::Pending => {}
        _ => panic!("two of three fragments are still pending"),
    }
}

#[test]
fn a_non_unicast_destination_is_dropped_before_it_is_stored() {
    let now = Instant::now();
    let mut table = Fragments::new();
    let multicast = raw_fragment(27, KEY, 0, true, &[1u8; 64], Ipv4Addr::new(239, 1, 2, 3));
    assert!(matches!(table.push(&multicast, now), Fragment::Dropped(_)));
    let broadcast = raw_fragment(28, KEY, 0, true, &[2u8; 64], Ipv4Addr::BROADCAST);
    assert!(matches!(table.push(&broadcast, now), Fragment::Dropped(_)));
    let loopback = raw_fragment(29, KEY, 0, true, &[3u8; 64], Ipv4Addr::LOCALHOST);
    assert!(matches!(table.push(&loopback, now), Fragment::Dropped(_)));
    assert_eq!(table.len(), 0);
}

#[test]
fn the_maximal_udp_payload_reassembles_byte_for_byte() {
    let payload: Vec<u8> = (0..65_507u32).map(|i| (i % 251) as u8).collect();
    let packets = fragments_of(31, KEY, &payload, TUN_MTU);
    assert!(packets.len() > 40, "the payload needs many fragments");
    let now = Instant::now();
    let mut table = Fragments::new();
    let mut done = None;
    for pkt in &packets {
        if let Fragment::Datagram(key, data) = table.push(pkt, now) {
            assert!(done.is_none(), "only the last fragment completes it");
            done = Some((key, data));
        }
    }
    let (key, data) = done.expect("the last fragment completes the datagram");
    assert_eq!(key, KEY);
    assert_eq!(data.len(), payload.len());
    assert_eq!(data, payload);
    assert_eq!(table.len(), 0);
}

#[test]
fn a_set_with_too_many_pieces_is_dropped() {
    let now = Instant::now();
    let mut table = Fragments::new();
    let mut offset = 0;
    for _ in 0..MAX_PIECES {
        let pkt = raw_fragment(33, KEY, offset, true, &[7u8; 8], KEY.2);
        assert!(matches!(table.push(&pkt, now), Fragment::Pending));
        offset += 8;
    }
    assert_eq!(table.len(), 1);
    let pkt = raw_fragment(33, KEY, offset, true, &[7u8; 8], KEY.2);
    assert!(matches!(table.push(&pkt, now), Fragment::Dropped(_)));
    assert_eq!(table.len(), 0);
}

#[test]
fn a_full_fragment_table_evicts_the_oldest_set() {
    let payload: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    let start = Instant::now();
    let mut table = Fragments::new();
    let mut first_sets = Vec::new();
    for i in 0..MAX_FRAGMENTS {
        let id = (100 + i) as u16;
        let packets = fragments_of(id, KEY, &payload, TUN_MTU);
        let at = start + Duration::from_millis(i as u64);
        assert!(matches!(table.push(&packets[0], at), Fragment::Pending));
        if i < 2 {
            first_sets.push(packets);
        }
    }
    assert_eq!(table.len(), MAX_FRAGMENTS);
    // One more set evicts the oldest (i == 0) and keeps the cap.
    let extra = fragments_of((100 + MAX_FRAGMENTS) as u16, KEY, &payload, TUN_MTU);
    let later = start + Duration::from_secs(1);
    assert!(matches!(table.push(&extra[0], later), Fragment::Pending));
    assert_eq!(table.len(), MAX_FRAGMENTS, "the cap is kept");
    // The second-oldest set survived eviction and still reassembles whole.
    let mut completed = None;
    for pkt in &first_sets[1][1..] {
        if let Fragment::Datagram(key, data) = table.push(pkt, later) {
            completed = Some((key, data));
        }
    }
    let (key, data) = completed.expect("the second-oldest set survived");
    assert_eq!(key, KEY);
    assert_eq!(data, payload);
    // The evicted oldest set lost its first fragment, so its remaining
    // fragments cannot complete it.
    for pkt in &first_sets[0][1..] {
        assert!(matches!(table.push(pkt, later), Fragment::Pending));
    }
}
