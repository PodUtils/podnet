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
