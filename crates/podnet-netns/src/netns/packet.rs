//! The packet builders of the netns backend: the DNS reply of the fake
//! DNS. The IPv4 checksum is `pod_hold::packet` (PodUtils/podcrates); the
//! hold of a SYN (`pod_hold::syn_hold`) parses each SYN and makes its RST.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::packet::ipv4_checksum;

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
