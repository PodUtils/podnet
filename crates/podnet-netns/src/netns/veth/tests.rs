use super::*;

fn frame(dst: [u8; 6], src: [u8; 6], kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&dst);
    out.extend_from_slice(&src);
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn arp_request(target: Ipv4Addr) -> Vec<u8> {
    let mut arp = vec![0, 1, 8, 0, 6, 4, 0, 1];
    arp.extend_from_slice(&GUEST_MAC);
    arp.extend_from_slice(&TUN_ADDR.octets());
    arp.extend_from_slice(&[0; 6]);
    arp.extend_from_slice(&target.octets());
    frame([0xff; 6], GUEST_MAC, ETH_P_ARP, &arp)
}

#[test]
fn the_wire_answers_arp_for_the_gateway() {
    let Frame::Reply(reply) = take_frame(&arp_request(TUN_GW), TUN_MTU) else {
        panic!("no ARP reply for the gateway");
    };
    assert_eq!(&reply[0..6], &GUEST_MAC, "to the program end");
    assert_eq!(&reply[6..12], &GATEWAY_MAC);
    assert_eq!(&reply[14..22], &[0, 1, 8, 0, 6, 4, 0, 2], "an ARP reply");
    assert_eq!(&reply[22..28], &GATEWAY_MAC);
    assert_eq!(&reply[28..32], &TUN_GW.octets());
    assert_eq!(&reply[32..38], &GUEST_MAC);
    assert_eq!(&reply[38..42], &TUN_ADDR.octets());
    // No reply for another address.
    let other = Ipv4Addr::new(10, 66, 255, 1);
    assert_eq!(take_frame(&arp_request(other), TUN_MTU), Frame::Drop);
}

#[test]
fn the_wire_strips_the_header_and_the_padding_of_an_ip_frame() {
    let mut packet = vec![0x45, 0, 0, 28];
    packet.resize(28, 7);
    let mut padded = packet.clone();
    padded.resize(46, 0);
    let got = take_frame(&frame(GATEWAY_MAC, GUEST_MAC, ETH_P_IP, &padded), TUN_MTU);
    assert_eq!(got, Frame::Ip(packet.clone()));
    // The own frames, and frames to another MAC, are dropped.
    let own = frame(GUEST_MAC, GATEWAY_MAC, ETH_P_IP, &padded);
    assert_eq!(take_frame(&own, TUN_MTU), Frame::Drop);
    let other = frame([2, 0, 0, 0, 0, 9], GUEST_MAC, ETH_P_IP, &padded);
    assert_eq!(take_frame(&other, TUN_MTU), Frame::Drop);
    // A frame larger than the MTU (a segment of an offload) is dropped.
    let mut big = vec![0x45, 0, 0x10, 0];
    big.resize(4096, 0);
    let big = frame(GATEWAY_MAC, GUEST_MAC, ETH_P_IP, &big);
    assert_eq!(take_frame(&big, TUN_MTU), Frame::Drop);
    // The frame of a packet of the stack.
    let out = frame_for(&packet).unwrap();
    assert_eq!(out, frame(GUEST_MAC, GATEWAY_MAC, ETH_P_IP, &packet));
}

#[test]
fn the_newlink_request_names_both_ends() {
    let msg = newlink_request("eth0", "wire0", 1500);
    assert_eq!(
        u32::from_ne_bytes([msg[0], msg[1], msg[2], msg[3]]) as usize,
        msg.len()
    );
    assert_eq!(u16::from_ne_bytes([msg[4], msg[5]]), RTM_NEWLINK);
    let text = String::from_utf8_lossy(&msg);
    assert!(text.contains("eth0\0"), "{text:?}");
    assert!(text.contains("veth"), "{text:?}");
    assert!(text.contains("wire0\0"), "{text:?}");
    assert_eq!(msg.len() % 4, 0);
    let mut ack = vec![0u8; 36];
    ack[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
    assert_eq!(netlink_error(&ack), Some(0));
    ack[16..20].copy_from_slice(&(-libc::EPERM).to_ne_bytes());
    assert_eq!(netlink_error(&ack), Some(-libc::EPERM));
}
