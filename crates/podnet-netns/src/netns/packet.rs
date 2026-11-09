//! The packet builders of the netns backend: the DNS reply, the RST to a
//! SYN, the IPv4 checksum and the parse of a SYN.

#[allow(unused_imports)]
use crate::prelude::*;

/// If `pkt` is an IPv4 UDP datagram to port 53 that holds a DNS query, return
/// the reply datagram. Return `None` for any other packet.
pub(crate) fn dns_reply_packet(pkt: &[u8], dns: &mut FakeDns) -> Option<Vec<u8>> {
    if pkt.len() < 28 || pkt[0] >> 4 != 4 || pkt[9] != libc::IPPROTO_UDP as u8 {
        return None;
    }
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    // A fragment cannot be parsed alone.
    let fragment = u16::from_be_bytes([pkt[6], pkt[7]]) & 0x3fff;
    if ihl < 20 || fragment != 0 || pkt.len() < ihl + 8 {
        return None;
    }
    let udp = &pkt[ihl..];
    let sport = u16::from_be_bytes([udp[0], udp[1]]);
    let dport = u16::from_be_bytes([udp[2], udp[3]]);
    let udp_len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if dport != DNS_PORT || udp_len < 8 || udp_len > udp.len() {
        return None;
    }
    let query = crate::dns::ParsedQuery::parse(&udp[8..udp_len]).ok()?;
    let answer = dns.answer(&query);

    let total = 20 + 8 + answer.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&[0x45, 0, 0, 0]);
    out[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    out.extend_from_slice(&[0, 0, 0x40, 0, 64, libc::IPPROTO_UDP as u8, 0, 0]);
    out.extend_from_slice(&pkt[16..20]); // source: the resolver address
    out.extend_from_slice(&pkt[12..16]); // destination: the client
    let checksum = ipv4_checksum(&out[..20]);
    out[10..12].copy_from_slice(&checksum.to_be_bytes());
    out.extend_from_slice(&dport.to_be_bytes());
    out.extend_from_slice(&sport.to_be_bytes());
    out.extend_from_slice(&((8 + answer.len()) as u16).to_be_bytes());
    // A zero UDP checksum means "none" on IPv4.
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&answer);
    Some(out)
}

/// The one's-complement checksum of an IPv4 header.
pub(crate) fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for pair in header.chunks(2) {
        let word = u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// The RST|ACK answer to the IPv4 TCP SYN `pkt`: the connect of the
/// program fails with ECONNREFUSED.
pub(crate) fn rst_reply_for_syn(pkt: &[u8]) -> Option<Vec<u8>> {
    parse_tcp_syn(pkt)?;
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    let tcp = &pkt[ihl..];
    let seq = u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]);
    let mut out = Vec::with_capacity(40);
    out.extend_from_slice(&[0x45, 0, 0, 40, 0, 0, 0x40, 0, 64, 6, 0, 0]);
    out.extend_from_slice(&pkt[16..20]); // source: the destination of the SYN
    out.extend_from_slice(&pkt[12..16]); // destination: the program
    let checksum = ipv4_checksum(&out[..20]);
    out[10..12].copy_from_slice(&checksum.to_be_bytes());
    out.extend_from_slice(&tcp[2..4]); // source port: the destination port
    out.extend_from_slice(&tcp[0..2]);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&seq.wrapping_add(1).to_be_bytes());
    // Data offset 5 words; flags RST and ACK; window 0; checksum; urgent 0.
    out.extend_from_slice(&[0x50, 0x14, 0, 0, 0, 0, 0, 0]);
    // The TCP checksum covers a pseudo-header and the segment.
    let mut pseudo = Vec::with_capacity(32);
    pseudo.extend_from_slice(&out[12..20]);
    pseudo.extend_from_slice(&[0, 6, 0, 20]);
    pseudo.extend_from_slice(&out[20..40]);
    let checksum = ipv4_checksum(&pseudo);
    out[36..38].copy_from_slice(&checksum.to_be_bytes());
    Some(out)
}

/// Parse a bare TCP SYN and return (src ip, src port, dst ip, dst port).
pub(crate) fn parse_tcp_syn(pkt: &[u8]) -> Option<(Ipv4Addr, u16, Ipv4Addr, u16)> {
    if pkt.len() < 40 || (pkt[0] >> 4) != 4 {
        return None;
    }
    if pkt[9] != 6 {
        return None;
    }
    let ihl = (pkt[0] & 0x0f) as usize * 4;
    if pkt.len() < ihl + 20 {
        return None;
    }
    let flags = pkt[ihl + 13];
    if flags & 0x02 == 0 || flags & 0x10 != 0 {
        return None; // SYN set, ACK clear
    }
    let src = Ipv4Addr::new(pkt[12], pkt[13], pkt[14], pkt[15]);
    let dst = Ipv4Addr::new(pkt[16], pkt[17], pkt[18], pkt[19]);
    let sport = u16::from_be_bytes([pkt[ihl], pkt[ihl + 1]]);
    let dport = u16::from_be_bytes([pkt[ihl + 2], pkt[ihl + 3]]);
    Some((src, sport, dst, dport))
}
