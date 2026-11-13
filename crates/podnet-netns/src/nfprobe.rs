//! The probe of in-namespace netfilter (T-152): can a process with no
//! privilege on the host use netfilter in its own user and network
//! namespace? This is the data plane of the issue (GitHub #352): `nat
//! OUTPUT REDIRECT` rules, `SO_ORIGINAL_DST` for TCP and conntrack for UDP.
//! The `netns-kernel` backend (T-153) uses it; `pod doctor` shows the row
//! and the verdict of the backend.
//!
//! In a forked child: `unshare(CLONE_NEWUSER | CLONE_NEWNET)`, the uid and
//! gid maps, `lo` up; then through nfnetlink a table, a chain `type nat hook
//! output priority -100` and the rule `tcp dport 80 redirect to :9999`; a
//! connect to 127.0.0.1:80 and `SO_ORIGINAL_DST` on the accepted socket; a
//! UDP datagram to 127.0.0.1:5353 and a ctnetlink dump that finds its
//! entry. The parent builds each message: the child does not allocate.

use crate::nfnl::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// The ports of the probe.
const PORT_FROM: u16 = 80;
const PORT_TO: u16 = 9999;
const PORT_UDP: u16 = 5353;
/// The sequence numbers of the three messages of the batch that ask for an
/// acknowledgement.
const SEQ_TABLE: u32 = 1;
const SEQ_CHAIN: u32 = 2;
const SEQ_RULE: u32 = 3;
const SEQ_DUMP: u32 = 10;

/// The batch: a table, a nat chain of the output hook, and the rule
/// `meta l4proto tcp tcp dport 80 redirect to :9999`.
fn nft_batch() -> Vec<u8> {
    const TABLE: &str = "podnet_probe";
    const CHAIN: &str = "out";
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
    b.msg(nft(NFT_MSG_NEWTABLE), create, SEQ_TABLE, NFPROTO_IPV4, 0);
    b.text(1, TABLE); // NFTA_TABLE_NAME
    b.end_msg();
    b.msg(nft(NFT_MSG_NEWCHAIN), create, SEQ_CHAIN, NFPROTO_IPV4, 0);
    b.text(1, TABLE); // NFTA_CHAIN_TABLE
    b.text(3, CHAIN); // NFTA_CHAIN_NAME
    b.nest(4); // NFTA_CHAIN_HOOK
    b.be32(1, NF_INET_LOCAL_OUT); // NFTA_HOOK_HOOKNUM
    b.be32(2, (-100i32) as u32); // NFTA_HOOK_PRIORITY
    b.end_nest();
    b.text(7, "nat"); // NFTA_CHAIN_TYPE
    b.end_msg();
    b.msg(
        nft(NFT_MSG_NEWRULE),
        create | NLM_F_APPEND,
        SEQ_RULE,
        NFPROTO_IPV4,
        0,
    );
    b.text(1, TABLE); // NFTA_RULE_TABLE
    b.text(2, CHAIN); // NFTA_RULE_CHAIN
    b.nest(4); // NFTA_RULE_EXPRESSIONS
    b.expr("meta", |b| {
        b.be32(1, NFT_REG_1); // NFTA_META_DREG
        b.be32(2, NFT_META_L4PROTO); // NFTA_META_KEY
    });
    b.expr("cmp", |b| {
        b.be32(1, NFT_REG_1); // NFTA_CMP_SREG
        b.be32(2, 0); // NFTA_CMP_OP: NFT_CMP_EQ
        b.value(3, &[libc::IPPROTO_TCP as u8]); // NFTA_CMP_DATA
    });
    b.expr("payload", |b| {
        b.be32(1, NFT_REG_1); // NFTA_PAYLOAD_DREG
        b.be32(2, NFT_PAYLOAD_TRANSPORT_HEADER); // NFTA_PAYLOAD_BASE
        b.be32(3, 2); // NFTA_PAYLOAD_OFFSET: the destination port
        b.be32(4, 2); // NFTA_PAYLOAD_LEN
    });
    b.expr("cmp", |b| {
        b.be32(1, NFT_REG_1);
        b.be32(2, 0);
        b.value(3, &PORT_FROM.to_be_bytes());
    });
    b.expr("immediate", |b| {
        b.be32(1, NFT_REG_1); // NFTA_IMMEDIATE_DREG
        b.value(2, &PORT_TO.to_be_bytes()); // NFTA_IMMEDIATE_DATA
    });
    b.expr("redir", |b| {
        b.be32(1, NFT_REG_1); // NFTA_REDIR_REG_PROTO_MIN
    });
    b.end_nest();
    b.end_msg();
    b.msg(
        NFNL_MSG_BATCH_END,
        NLM_F_REQUEST,
        4,
        0,
        NFNL_SUBSYS_NFTABLES,
    );
    b.end_msg();
    b.buf
}

/// The dump request of the conntrack table (IPv4).
fn ct_dump() -> Vec<u8> {
    let mut b = Builder::new();
    b.msg(
        (NFNL_SUBSYS_CTNETLINK << 8) | IPCTNL_MSG_CT_GET,
        NLM_F_REQUEST | NLM_F_DUMP,
        SEQ_DUMP,
        libc::AF_INET as u8,
        0,
    );
    b.end_msg();
    b.buf
}

/// The step of the probe that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
enum Step {
    Done = 0,
    Unshare = 1,
    Maps = 2,
    Loopback = 3,
    Netlink = 4,
    Table = 5,
    Chain = 6,
    Rule = 7,
    Connect = 8,
    OriginalDst = 9,
    Conntrack = 10,
}

impl Step {
    fn from(n: i32) -> Option<Step> {
        use Step::*;
        [
            Done,
            Unshare,
            Maps,
            Loopback,
            Netlink,
            Table,
            Chain,
            Rule,
            Connect,
            OriginalDst,
            Conntrack,
        ]
        .into_iter()
        .find(|s| *s as i32 == n)
    }

    fn name(self) -> &'static str {
        match self {
            Step::Done => "done",
            Step::Unshare => "unshare(NEWUSER|NEWNET)",
            Step::Maps => "the uid and gid maps",
            Step::Loopback => "lo up",
            Step::Netlink => "socket(NETLINK_NETFILTER)",
            Step::Table => "nft table",
            Step::Chain => "nft chain type nat hook output",
            Step::Rule => "nft rule redirect",
            Step::Connect => "connect to the redirected port",
            Step::OriginalDst => "SO_ORIGINAL_DST",
            Step::Conntrack => "the UDP entry of conntrack (ctnetlink)",
        }
    }
}

/// The result of the child: the step, the errno, and a value (the port of
/// `SO_ORIGINAL_DST`).
type Outcome = (Step, i32, i32);

/// Write `data` to the file `path` (a C string); false on an error.
fn write_file(path: &std::ffi::CStr, data: &[u8]) -> bool {
    // SAFETY: open, write and close of a path and a buffer that live.
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return false;
        }
        let n = libc::write(fd, data.as_ptr().cast(), data.len());
        libc::close(fd);
        n == data.len() as isize
    }
}
fn udp_entry_to(entry: &[u8], port: u16) -> bool {
    // CTA_TUPLE_ORIG { CTA_TUPLE_PROTO { CTA_PROTO_NUM, CTA_PROTO_DST_PORT } }
    attrs(entry)
        .filter(|(ty, _)| *ty == 1)
        .flat_map(|(_, tuple)| attrs(tuple))
        .filter(|(ty, _)| *ty == 2)
        .any(|(_, proto)| {
            let num = attrs(proto).find(|(ty, _)| *ty == 1).map(|(_, v)| v);
            let dport = attrs(proto).find(|(ty, _)| *ty == 3).map(|(_, v)| v);
            num == Some(&[libc::IPPROTO_UDP as u8][..]) && dport == Some(&port.to_be_bytes()[..])
        })
}

fn loopback(port: u16) -> libc::sockaddr_in {
    // SAFETY: a sockaddr_in of zeros is valid.
    let mut a: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    a.sin_family = libc::AF_INET as libc::sa_family_t;
    a.sin_port = port.to_be();
    a.sin_addr.s_addr = u32::from(std::net::Ipv4Addr::LOCALHOST).to_be();
    a
}

/// The steps of the child, after the fork. No allocation.
fn child(batch: &[u8], dump: &[u8], maps: &[(&std::ffi::CStr, &[u8])]) -> Outcome {
    // SAFETY: each call gets valid arguments; the buffers live.
    unsafe {
        if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) != 0 {
            return (Step::Unshare, errno(), 0);
        }
        for (path, data) in maps {
            if !write_file(path, data) {
                return (Step::Maps, errno(), 0);
            }
        }
        if crate::netns::bringup_loopback().is_err() {
            return (Step::Loopback, errno(), 0);
        }
        let nl = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_NETFILTER);
        if nl < 0 {
            return (Step::Netlink, errno(), 0);
        }
        let limit = libc::timeval {
            tv_sec: 2,
            tv_usec: 0,
        };
        let size = std::mem::size_of::<libc::timeval>() as libc::socklen_t;
        libc::setsockopt(
            nl,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&limit as *const libc::timeval).cast(),
            size,
        );
        if let Err((seq, e)) = exchange(nl, batch, &[SEQ_TABLE, SEQ_CHAIN, SEQ_RULE], &mut |_| {}) {
            let step = match seq {
                SEQ_TABLE => Step::Table,
                SEQ_CHAIN => Step::Chain,
                _ => Step::Rule,
            };
            return (step, e, 0);
        }
        let sa = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        let listener = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        let to = loopback(PORT_TO);
        if libc::bind(listener, (&to as *const libc::sockaddr_in).cast(), sa) != 0
            || libc::listen(listener, 1) != 0
        {
            return (Step::Connect, errno(), 0);
        }
        let client = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        let from = loopback(PORT_FROM);
        if libc::connect(client, (&from as *const libc::sockaddr_in).cast(), sa) != 0 {
            return (Step::Connect, errno(), 0);
        }
        let accepted = libc::accept(listener, std::ptr::null_mut(), std::ptr::null_mut());
        if accepted < 0 {
            return (Step::Connect, errno(), 0);
        }
        let mut original: libc::sockaddr_in = std::mem::zeroed();
        let mut len = sa;
        if libc::getsockopt(
            accepted,
            libc::SOL_IP,
            SO_ORIGINAL_DST,
            (&mut original as *mut libc::sockaddr_in).cast(),
            &mut len,
        ) != 0
        {
            return (Step::OriginalDst, errno(), 0);
        }
        let port = i32::from(u16::from_be(original.sin_port));
        let udp = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        let dest = loopback(PORT_UDP);
        libc::sendto(
            udp,
            b"x".as_ptr().cast(),
            1,
            0,
            (&dest as *const libc::sockaddr_in).cast(),
            sa,
        );
        let mut found = 0;
        let dumped = exchange(nl, dump, &[], &mut |entry| {
            if udp_entry_to(entry, PORT_UDP) {
                found += 1;
            }
        });
        match dumped {
            Ok(()) if found > 0 => (Step::Done, 0, port),
            Ok(()) => (Step::Conntrack, libc::ENOENT, port),
            Err((_, e)) => (Step::Conntrack, e, port),
        }
    }
}

/// The text of an outcome, and true when each step passed.
fn describe((step, error, port): Outcome) -> (bool, String) {
    let error_text = || std::io::Error::from_raw_os_error(error).to_string();
    match step {
        Step::Done if port == i32::from(PORT_FROM) => (
            true,
            format!(
                "nft table, nat chain and REDIRECT: ok; SO_ORIGINAL_DST: port {port}; UDP conntrack (ctnetlink): ok"
            ),
        ),
        Step::Done => (false, format!("SO_ORIGINAL_DST gave port {port}, not {PORT_FROM}")),
        Step::Conntrack if error == libc::ENOENT => (
            false,
            format!("REDIRECT and SO_ORIGINAL_DST (port {port}) work; ctnetlink has no UDP entry"),
        ),
        Step::Conntrack => (
            false,
            format!("REDIRECT and SO_ORIGINAL_DST (port {port}) work; ctnetlink: {}", error_text()),
        ),
        step => (false, format!("{}: {}", step.name(), error_text())),
    }
}

/// Run the probe in a forked child.
pub(crate) fn probe_netfilter() -> crate::doctor::Probe {
    const NAME: &str = "netfilter in a namespace";
    let batch = nft_batch();
    let dump = ct_dump();
    // SAFETY: getuid and getgid have no arguments.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let uid_map = format!("0 {uid} 1\n");
    let gid_map = format!("0 {gid} 1\n");
    let maps: [(&std::ffi::CStr, &[u8]); 3] = [
        (c"/proc/self/setgroups", b"deny"),
        (c"/proc/self/uid_map", uid_map.as_bytes()),
        (c"/proc/self/gid_map", gid_map.as_bytes()),
    ];
    let mut fds = [0; 2];
    // SAFETY: pipe2 writes two descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return probe(
            NAME,
            false,
            format!("pipe2: {}", std::io::Error::last_os_error()),
        );
    }
    // SAFETY: pipe2 gave two new descriptors that nothing else owns.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // SAFETY: the child calls no allocator and exits with _exit.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        let (step, error, port) = child(&batch, &dump, &maps);
        let mut out = [0u8; 12];
        out[..4].copy_from_slice(&(step as i32).to_ne_bytes());
        out[4..8].copy_from_slice(&error.to_ne_bytes());
        out[8..].copy_from_slice(&port.to_ne_bytes());
        // SAFETY: write of a buffer that lives, then _exit.
        unsafe {
            libc::write(write.as_raw_fd(), out.as_ptr().cast(), out.len());
            libc::_exit(0);
        }
    }
    drop(write);
    if pid < 0 {
        return probe(
            NAME,
            false,
            format!("fork: {}", std::io::Error::last_os_error()),
        );
    }
    let mut out = [0u8; 12];
    // SAFETY: read into a buffer that lives.
    let n = unsafe { libc::read(read.as_raw_fd(), out.as_mut_ptr().cast(), out.len()) };
    // SAFETY: waitpid of our child.
    unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) };
    if n != 12 {
        return probe(NAME, false, "the probe process gave no result".into());
    }
    let word = |i: usize| i32::from_ne_bytes(out[i..i + 4].try_into().unwrap_or([0; 4]));
    let Some(step) = Step::from(word(0)) else {
        return probe(NAME, false, "the probe process gave a bad result".into());
    };
    let (ok, detail) = describe((step, word(4), word(8)));
    probe(NAME, ok, detail)
}

fn probe(name: &'static str, ok: bool, detail: String) -> crate::doctor::Probe {
    crate::doctor::Probe { name, ok, detail }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-152: the probe gives a verdict and a text on each host; on a host
    // with user namespaces and nf_tables, each step passes.
    #[test]
    fn doctor_netfilter_row_has_a_verdict() {
        let probe = probe_netfilter();
        eprintln!("{}: {} {}", probe.name, probe.ok, probe.detail);
        assert!(!probe.detail.is_empty());
        assert!(!probe.detail.contains("gave no result"), "{}", probe.detail);
        if probe.ok {
            assert!(probe.detail.contains("port 80"), "{}", probe.detail);
        }
    }

    #[test]
    fn the_batch_has_its_lengths_and_the_dump_parses() {
        let batch = nft_batch();
        let mut at = 0;
        let mut types = Vec::new();
        while at < batch.len() {
            let len = u32::from_ne_bytes(batch[at..at + 4].try_into().unwrap()) as usize;
            types.push(u16::from_ne_bytes([batch[at + 4], batch[at + 5]]));
            at += len;
        }
        assert_eq!(at, batch.len());
        assert_eq!(types, [0x10, 0xa00, 0xa03, 0xa06, 0x11]);
        // A conntrack entry: CTA_TUPLE_ORIG { CTA_TUPLE_PROTO { NUM 17, DST_PORT 5353 } }.
        let mut b = Builder::new();
        b.nest(1);
        b.nest(2);
        b.attr(1, &[17]);
        b.attr(3, &PORT_UDP.to_be_bytes());
        b.end_nest();
        b.end_nest();
        assert!(udp_entry_to(&b.buf, PORT_UDP));
        assert!(!udp_entry_to(&b.buf, 53));
    }
}
