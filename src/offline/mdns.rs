//! The mDNS record of a signed announcement, and the two commands.
//!
//! An announcement of `pod-offline` is a DNS-SD TXT record: a sequence of
//! length-prefixed `key=value` strings. This module puts that record inside
//! one DNS answer of type TXT, and reads the TXT answers of a packet. The
//! record itself, and its signature, are `pod-offline`: there is no second
//! discovery record here.
//!
//! `announce` sends the answer to a UDP address (the mDNS group by default,
//! or a unicast address for a test or one interface). `listen` binds a UDP
//! socket, joins the group when the bind address is the wildcard or the
//! group, and feeds each TXT record to
//! [`pod_offline::discovery::Discovery::hear`]. A record with a bad
//! signature is counted and ignored.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use pod_identity::Identity;
use pod_offline::discovery::{Announcement, Discovery};

/// The mDNS group and port of the service.
pub const GROUP: &str = "224.0.0.251:5353";

/// The DNS type of a TXT record.
pub const TYPE_TXT: u16 = 16;

/// The time to live of the answer of an announcement.
pub const TTL: u32 = 120;

/// The DNS name of the DNS-SD service and an instance of it.
pub fn service_name(name: &str) -> String {
    format!("{name}.{}", pod_offline::discovery::SERVICE)
}

/// The DNS wire name of `name`: one length byte and the bytes for each
/// label, then the root.
pub fn encode_name(name: &str) -> Result<Vec<u8>, pod_offline::Error> {
    let mut out = Vec::new();
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(pod_offline::Error::Txt {
                why: format!("the DNS label {label:?} is empty or longer than 63 bytes"),
            });
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    Ok(out)
}

/// One DNS answer of type TXT: a header with one answer, the name, and the
/// TXT record of the announcement.
pub fn answer(name: &str, txt: &[u8]) -> Result<Vec<u8>, pod_offline::Error> {
    if txt.len() > usize::from(u16::MAX) {
        return Err(pod_offline::Error::Txt {
            why: "the TXT record is above 65535 bytes".to_string(),
        });
    }
    let mut out = Vec::with_capacity(txt.len() + 64);
    // ID 0, QR and AA, no question, one answer.
    out.extend_from_slice(&[0, 0, 0x84, 0x00]);
    out.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
    out.extend_from_slice(&encode_name(name)?);
    out.extend_from_slice(&TYPE_TXT.to_be_bytes());
    // Class IN, with the cache-flush bit that mDNS sets.
    out.extend_from_slice(&[0x80, 0x01]);
    out.extend_from_slice(&TTL.to_be_bytes());
    out.extend_from_slice(&(txt.len() as u16).to_be_bytes());
    out.extend_from_slice(txt);
    Ok(out)
}

/// Each TXT record of one DNS packet, in the order of its answers. A
/// malformed part ends the scan: a reader never invents a record.
pub fn txt_records(packet: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    if packet.len() < 12 {
        return out;
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut at = 12;
    for _ in 0..questions {
        at = match skip_name(packet, at) {
            Some(next) => next + 4,
            None => return out,
        };
        if at > packet.len() {
            return out;
        }
    }
    for _ in 0..answers {
        at = match skip_name(packet, at) {
            Some(next) => next,
            None => return out,
        };
        if at + 10 > packet.len() {
            return out;
        }
        if at + 2 <= packet.len() && u16::from_be_bytes([packet[at], packet[at + 1]]) == TYPE_TXT {
            let length = u16::from_be_bytes([packet[at + 8], packet[at + 9]]) as usize;
            let start = at + 10;
            let end = start.saturating_add(length);
            if end > packet.len() {
                return out;
            }
            out.push(packet[start..end].to_vec());
        }
        let length = u16::from_be_bytes([packet[at + 8], packet[at + 9]]) as usize;
        at = match (at + 10).checked_add(length) {
            Some(next) if next <= packet.len() => next,
            _ => return out,
        };
    }
    out
}

/// The place after the name at `at`, with a compression pointer taken.
fn skip_name(packet: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let byte = *packet.get(at)?;
        if byte == 0 {
            return Some(at + 1);
        }
        if byte & 0xc0 == 0xc0 {
            return Some(at + 2);
        }
        at += 1 + usize::from(byte);
        if at > packet.len() {
            return None;
        }
    }
}

/// Send the signed announcement of `identity` to `to` `repeat` times. It
/// gives the announcement that went.
pub fn announce(
    identity: &Identity,
    name: &str,
    address: &str,
    port: u16,
    to: &str,
    repeat: u32,
) -> anyhow::Result<Announcement> {
    let announcement =
        Announcement::signed(identity, name, address, port).map_err(anyhow::Error::new)?;
    let txt = announcement.txt().map_err(anyhow::Error::new)?;
    let packet = answer(&service_name(name), &txt).map_err(anyhow::Error::new)?;
    let to: SocketAddr = to
        .parse()
        .map_err(|e| anyhow::anyhow!("{to}: the destination is not IP:PORT: {e}"))?;
    // EGRESS-3 and EGRESS-4: the multicast group is a destination of the
    // policy, checked before the datagram socket is bound.
    crate::policy::permit_fault(&to.ip().to_string(), to.port())?;
    // T-615: a host that refuses UDP, or has no route to `to`, gives one
    // line with the step and the address, and exit 69.
    let unavailable = |what: String, e: std::io::Error| {
        crate::exit::fault(
            crate::exit::Fault::Unavailable,
            anyhow::anyhow!("{what}: {e}; this host gives no UDP to {to} (podnet net doctor)"),
        )
    };
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|e| unavailable("a UDP socket for the announcement".into(), e))?;
    socket.set_multicast_ttl_v4(255)?;
    socket.set_multicast_loop_v4(true)?;
    for _ in 0..repeat.max(1) {
        socket
            .send_to(&packet, to)
            .map_err(|e| unavailable(format!("sending the announcement to {to}"), e))?;
    }
    Ok(announcement)
}

/// What a listen heard.
pub struct Heard {
    /// The peers whose announcements verified.
    pub discovery: Discovery,
    /// The TXT records that did not parse or did not verify.
    pub ignored: usize,
    /// The TXT records that came.
    pub records: usize,
}

/// Listen on `bind` for `timeout`. The group is joined when `bind` names
/// the wildcard address or the group itself.
pub fn listen(bind: &str, timeout: Duration) -> anyhow::Result<Heard> {
    let bind: SocketAddr = bind
        .parse()
        .map_err(|e| anyhow::anyhow!("{bind}: the bind address is not IP:PORT: {e}"))?;
    // egress-exempt: a discovery listener that joins the group
    let socket = UdpSocket::bind(bind)?;
    if bind.ip().is_unspecified() || bind.ip().is_multicast() {
        socket.join_multicast_v4(&Ipv4Addr::new(224, 0, 0, 251), &Ipv4Addr::UNSPECIFIED)?;
    }
    let deadline = Instant::now() + timeout;
    let mut heard = Heard {
        discovery: Discovery::new(),
        ignored: 0,
        records: 0,
    };
    let mut packet = vec![0u8; 9000];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        socket.set_read_timeout(Some(left.min(Duration::from_millis(200))))?;
        match socket.recv_from(&mut packet) {
            Ok((count, _)) => {
                for txt in txt_records(&packet[..count]) {
                    heard.records += 1;
                    if !heard.discovery.hear(&txt) {
                        heard.ignored += 1;
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(heard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> Announcement {
        Announcement::signed(&Identity::from_seed(&[21; 32]), "node-b", "10.0.0.8", 4242).unwrap()
    }

    #[test]
    fn the_answer_carries_the_record_and_the_record_reads_back() {
        let announcement = peer();
        let txt = announcement.txt().unwrap();
        let packet = answer(&service_name("node-b"), &txt).unwrap();
        assert_eq!(txt_records(&packet), vec![txt]);
    }

    #[test]
    fn a_record_of_two_answers_comes_apart() {
        let one = peer();
        let two =
            Announcement::signed(&Identity::from_seed(&[22; 32]), "node-c", "10.0.0.9", 1).unwrap();
        let mut packet = answer("one", &one.txt().unwrap()).unwrap();
        let second = answer("two", &two.txt().unwrap()).unwrap();
        // Keep the header of the first and add the second answer.
        packet[7] = 2;
        packet.extend_from_slice(&second[12..]);
        let records = txt_records(&packet);
        assert_eq!(records.len(), 2);
        let mut discovery = Discovery::new();
        assert!(discovery.hear(&records[0]));
        assert!(discovery.hear(&records[1]));
        assert_eq!(discovery.len(), 2);
    }

    #[test]
    fn a_bad_signature_is_ignored_on_the_wire() {
        let mut bad = peer();
        bad.port = 9;
        let packet = answer("bad", &bad.txt().unwrap()).unwrap();
        let records = txt_records(&packet);
        assert_eq!(records.len(), 1);
        let mut discovery = Discovery::new();
        assert!(!discovery.hear(&records[0]));
        assert!(discovery.is_empty());
    }

    #[test]
    fn a_packet_that_ends_in_a_record_gives_no_part_of_it() {
        let packet = answer("one", &peer().txt().unwrap()).unwrap();
        assert!(txt_records(&packet[..packet.len() - 1]).is_empty());
        assert!(txt_records(&[0u8; 4]).is_empty());
    }

    /// T-615: a send that the host refuses exits 69 with the step and the
    /// address (a send to port 0 fails on each host: EINVAL). A host that
    /// refuses the bind of a UDP socket fails at the step before.
    #[test]
    fn a_refused_announcement_names_its_step_and_exits_69() {
        let identity = Identity::from_seed(&[23; 32]);
        let e = announce(&identity, "node-d", "10.0.0.9", 22, "127.0.0.1:0", 1).unwrap_err();
        let text = format!("{e:#}");
        assert_eq!(crate::exit::code_of(&e), crate::exit::UNAVAILABLE, "{text}");
        assert!(
            text.contains("sending the announcement to 127.0.0.1:0")
                || text.contains("a UDP socket for the announcement"),
            "{text}"
        );
        assert!(text.contains("no UDP to 127.0.0.1:0"), "{text}");
    }
}
