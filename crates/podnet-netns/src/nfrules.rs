//! The rules of the `netns-kernel` backend (T-153): the nfnetlink batch
//! that the child installs in its own network namespace, and the conntrack
//! lookup that gives the original destination of a UDP datagram.
//!
//! The table `podnet` (family `ip`) has one chain, `type nat hook output
//! priority -100`, and three rules in this order:
//!
//! 1. `udp dport 53 redirect to :DNS`: each DNS query, whatever its address.
//! 2. `ip daddr != 127.0.0.0/8 tcp redirect to :TCP`.
//! 3. `ip daddr != 127.0.0.0/8 udp redirect to :UDP`.
//!
//! A destination on the loopback stays in the namespace. The original
//! destination of a TCP connection is `SO_ORIGINAL_DST` on the accepted
//! socket. The original destination of a UDP datagram is the conntrack
//! entry whose reply tuple is `(127.0.0.1:UDP, client)`.

use crate::nfnl::*;
use std::net::{Ipv4Addr, SocketAddrV4};

/// The name of the table of the plane.
pub(crate) const TABLE: &str = "podnet";
const CHAIN: &str = "out";
const NFT_CMP_NEQ: u32 = 1;
const NFT_PAYLOAD_NETWORK_HEADER: u32 = 1;
/// The offset of the destination address in an IPv4 header.
const IP_DADDR: u32 = 16;
/// The sequence numbers of the batch: the table, the chain, the rules.
pub(crate) const SEQ_ACKS: [u32; 5] = [1, 2, 3, 4, 5];
/// The sequence number of a conntrack lookup.
const SEQ_GET: u32 = 20;

/// The ports of the listeners of the plane (all on 127.0.0.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ports {
    pub(crate) tcp: u16,
    pub(crate) udp: u16,
    pub(crate) dns: u16,
}

/// One `redirect` rule: the protocol, an optional destination port to
/// match, whether a destination on 127.0.0.0/8 is left alone, and the
/// port to redirect to.
fn redirect_rule(
    b: &mut Builder,
    seq: u32,
    proto: u8,
    dport: Option<u16>,
    skip_loop: bool,
    to: u16,
) {
    let nft = |msg: u16| (NFNL_SUBSYS_NFTABLES << 8) | msg;
    b.msg(
        nft(NFT_MSG_NEWRULE),
        NLM_F_REQUEST | NLM_F_CREATE | NLM_F_ACK | NLM_F_APPEND,
        seq,
        NFPROTO_IPV4,
        0,
    );
    b.text(1, TABLE); // NFTA_RULE_TABLE
    b.text(2, CHAIN); // NFTA_RULE_CHAIN
    b.nest(4); // NFTA_RULE_EXPRESSIONS
    if skip_loop {
        b.expr("payload", |b| {
            b.be32(1, NFT_REG_1);
            b.be32(2, NFT_PAYLOAD_NETWORK_HEADER);
            b.be32(3, IP_DADDR);
            b.be32(4, 1); // the first byte of the destination
        });
        b.expr("cmp", |b| {
            b.be32(1, NFT_REG_1);
            b.be32(2, NFT_CMP_NEQ);
            b.value(3, &[127]);
        });
    }
    b.expr("meta", |b| {
        b.be32(1, NFT_REG_1);
        b.be32(2, NFT_META_L4PROTO);
    });
    b.expr("cmp", |b| {
        b.be32(1, NFT_REG_1);
        b.be32(2, 0); // NFT_CMP_EQ
        b.value(3, &[proto]);
    });
    if let Some(port) = dport {
        b.expr("payload", |b| {
            b.be32(1, NFT_REG_1);
            b.be32(2, NFT_PAYLOAD_TRANSPORT_HEADER);
            b.be32(3, 2); // the destination port
            b.be32(4, 2);
        });
        b.expr("cmp", |b| {
            b.be32(1, NFT_REG_1);
            b.be32(2, 0);
            b.value(3, &port.to_be_bytes());
        });
    }
    b.expr("immediate", |b| {
        b.be32(1, NFT_REG_1);
        b.value(2, &to.to_be_bytes());
    });
    b.expr("redir", |b| {
        b.be32(1, NFT_REG_1);
    });
    b.end_nest();
    b.end_msg();
}

/// The batch of the plane: the table, the chain and the three rules.
pub(crate) fn batch(ports: Ports) -> Vec<u8> {
    let nft = |msg: u16| (NFNL_SUBSYS_NFTABLES << 8) | msg;
    let create = NLM_F_REQUEST | NLM_F_CREATE | NLM_F_ACK;
    let mut b = Builder::new();
    b.msg(
        NFNL_MSG_BATCH_BEGIN,
        NLM_F_REQUEST,
        0,
        0,
        NFNL_SUBSYS_NFTABLES,
    );
    b.end_msg();
    b.msg(nft(NFT_MSG_NEWTABLE), create, SEQ_ACKS[0], NFPROTO_IPV4, 0);
    b.text(1, TABLE); // NFTA_TABLE_NAME
    b.end_msg();
    b.msg(nft(NFT_MSG_NEWCHAIN), create, SEQ_ACKS[1], NFPROTO_IPV4, 0);
    b.text(1, TABLE); // NFTA_CHAIN_TABLE
    b.text(3, CHAIN); // NFTA_CHAIN_NAME
    b.nest(4); // NFTA_CHAIN_HOOK
    b.be32(1, NF_INET_LOCAL_OUT);
    b.be32(2, (-100i32) as u32); // the priority of destination NAT
    b.end_nest();
    b.text(7, "nat"); // NFTA_CHAIN_TYPE
    b.end_msg();
    let udp = libc::IPPROTO_UDP as u8;
    let tcp = libc::IPPROTO_TCP as u8;
    redirect_rule(&mut b, SEQ_ACKS[2], udp, Some(53), false, ports.dns);
    redirect_rule(&mut b, SEQ_ACKS[3], tcp, None, true, ports.tcp);
    redirect_rule(&mut b, SEQ_ACKS[4], udp, None, true, ports.udp);
    b.msg(
        NFNL_MSG_BATCH_END,
        NLM_F_REQUEST,
        SEQ_ACKS[4] + 1,
        0,
        NFNL_SUBSYS_NFTABLES,
    );
    b.end_msg();
    b.buf
}

/// Send `batch` on the netlink socket `fd` and wait for each answer. The
/// text names the step that failed.
pub(crate) fn install(fd: i32, batch: &[u8]) -> Result<(), String> {
    exchange(fd, batch, &SEQ_ACKS, &mut |_| {}).map_err(|(seq, errno)| {
        let step = match seq {
            1 => "the nft table",
            2 => "the nft nat chain of the output hook",
            3..=5 => "an nft redirect rule",
            _ => "the nft batch",
        };
        format!("{step}: {}", std::io::Error::from_raw_os_error(errno))
    })
}

/// The request for the conntrack entry whose reply tuple is the UDP flow
/// from `from` (the redirect port) to `to` (the client).
fn ct_get_reply(from: SocketAddrV4, to: SocketAddrV4) -> Vec<u8> {
    let mut b = Builder::new();
    b.msg(
        (NFNL_SUBSYS_CTNETLINK << 8) | IPCTNL_MSG_CT_GET,
        NLM_F_REQUEST | NLM_F_ACK,
        SEQ_GET,
        libc::AF_INET as u8,
        0,
    );
    b.nest(2); // CTA_TUPLE_REPLY
    b.nest(1); // CTA_TUPLE_IP
    b.attr(1, &from.ip().octets()); // CTA_IP_V4_SRC
    b.attr(2, &to.ip().octets()); // CTA_IP_V4_DST
    b.end_nest();
    b.nest(2); // CTA_TUPLE_PROTO
    b.attr(1, &[libc::IPPROTO_UDP as u8]); // CTA_PROTO_NUM
    b.attr(2, &from.port().to_be_bytes()); // CTA_PROTO_SRC_PORT
    b.attr(3, &to.port().to_be_bytes()); // CTA_PROTO_DST_PORT
    b.end_nest();
    b.end_nest();
    b.end_msg();
    b.buf
}

/// The destination of the original tuple of a conntrack entry (the
/// attributes after the `nfgenmsg`).
pub(crate) fn original_destination(entry: &[u8]) -> Option<SocketAddrV4> {
    let tuple = attrs(entry).find(|(ty, _)| *ty == 1)?.1; // CTA_TUPLE_ORIG
    let ip = attrs(tuple).find(|(ty, _)| *ty == 1)?.1;
    let proto = attrs(tuple).find(|(ty, _)| *ty == 2)?.1;
    let dst: [u8; 4] = attrs(ip).find(|(ty, _)| *ty == 2)?.1.try_into().ok()?;
    let port: [u8; 2] = attrs(proto).find(|(ty, _)| *ty == 3)?.1.try_into().ok()?;
    Some(SocketAddrV4::new(
        Ipv4Addr::from(dst),
        u16::from_be_bytes(port),
    ))
}

/// The original destination of the UDP datagrams that `client` sent to
/// the redirect port `via` (conntrack, on the netlink socket `fd` of the
/// namespace).
pub(crate) fn udp_original_destination(
    fd: i32,
    via: u16,
    client: SocketAddrV4,
) -> Result<SocketAddrV4, String> {
    let request = ct_get_reply(SocketAddrV4::new(Ipv4Addr::LOCALHOST, via), client);
    let mut found = None;
    exchange(fd, &request, &[SEQ_GET], &mut |entry| {
        found = found.or_else(|| original_destination(entry));
    })
    .map_err(|(_, errno)| {
        format!(
            "the conntrack lookup of {client}: {}",
            std::io::Error::from_raw_os_error(errno)
        )
    })?;
    found.ok_or_else(|| format!("the conntrack entry of {client} has no original destination"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The types and the lengths of the messages of a batch.
    fn messages(batch: &[u8]) -> Vec<(u16, u32)> {
        let mut at = 0;
        let mut out = Vec::new();
        while at < batch.len() {
            let len = u32::from_ne_bytes(batch[at..at + 4].try_into().unwrap()) as usize;
            out.push((
                u16::from_ne_bytes([batch[at + 4], batch[at + 5]]),
                u32::from_ne_bytes(batch[at + 8..at + 12].try_into().unwrap()),
            ));
            at += len;
            assert!(at <= batch.len(), "a message is longer than the batch");
        }
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    // T-153: the batch has a begin, a table, a chain, three rules and an
    // end, with the sequence numbers that `install` waits for.
    #[test]
    fn the_rule_builder_makes_one_table_one_chain_and_three_rules() {
        let ports = Ports {
            tcp: 41001,
            udp: 41002,
            dns: 41003,
        };
        let batch = batch(ports);
        let types: Vec<u16> = messages(&batch).iter().map(|m| m.0).collect();
        assert_eq!(types, [0x10, 0xa00, 0xa03, 0xa06, 0xa06, 0xa06, 0x11]);
        let seqs: Vec<u32> = messages(&batch).iter().map(|m| m.1).collect();
        assert_eq!(&seqs[1..6], &SEQ_ACKS);
        // Each redirect port is in the batch, in network order.
        for port in [ports.tcp, ports.udp, ports.dns] {
            assert!(contains(&batch, &port.to_be_bytes()), "port {port}");
        }
        assert!(contains(&batch, b"nat\0"));
        assert!(contains(&batch, b"redir\0"));
        assert!(contains(&batch, b"podnet\0"));
    }

    // T-153: only the DNS rule matches a port; the two others skip the
    // loopback (a `payload` of the network header and `cmp neq 127`).
    #[test]
    fn only_the_dns_rule_names_a_port_and_the_others_skip_the_loopback() {
        let rule = |proto, dport, skip| {
            let mut b = Builder::new();
            redirect_rule(&mut b, 3, proto, dport, skip, 4242);
            b.buf
        };
        let dns = rule(17, Some(53), false);
        let tcp = rule(6, None, true);
        // The payload expression of the transport header (base 2) is in
        // the DNS rule only; the loopback test (base 1) is in the others.
        assert!(contains(&dns, &53u16.to_be_bytes()));
        assert!(!contains(&tcp, &53u16.to_be_bytes()));
        assert!(contains(&tcp, &[127]));
        assert!(!contains(&dns, &[127]));
        assert!(contains(&tcp, &4242u16.to_be_bytes()));
    }

    // T-153: the original destination is read from the original tuple of
    // a conntrack entry, and a lookup request names the reply tuple.
    #[test]
    fn the_conntrack_entry_gives_the_original_destination() {
        let mut b = Builder::new();
        b.nest(1); // CTA_TUPLE_ORIG
        b.nest(1);
        b.attr(1, &[10, 0, 0, 1]);
        b.attr(2, &[93, 184, 216, 34]);
        b.end_nest();
        b.nest(2);
        b.attr(1, &[17]);
        b.attr(2, &40000u16.to_be_bytes());
        b.attr(3, &5353u16.to_be_bytes());
        b.end_nest();
        b.end_nest();
        assert_eq!(
            original_destination(&b.buf),
            Some(SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 5353))
        );
        assert_eq!(original_destination(&[]), None);
        let client = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 40000);
        let request = ct_get_reply(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 41002), client);
        let len = u32::from_ne_bytes(request[..4].try_into().unwrap()) as usize;
        assert_eq!(len, request.len());
        assert!(contains(&request, &41002u16.to_be_bytes()));
        assert!(contains(&request, &40000u16.to_be_bytes()));
    }
}
