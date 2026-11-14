//! The guards of the kernel mode (T-120): a masquerade and an MSS clamp
//! through nf_tables netlink, an `unreachable` default route as the kill
//! switch, and `rp_filter` 0 on each WAN. It uses no `nft` program.
//!
//! * The table `ip podnet_router` has the chain `nat` (hook postrouting,
//!   priority 100) with the rule `oifname WAN masquerade` for each WAN, and
//!   the chain `mss` (hook forward, priority -150) with the rule
//!   `oifname WAN tcp flags & (syn|rst) == syn tcp option maxseg size set
//!   rt mtu` for each WAN. One batch replaces the table: the change is one
//!   transaction of the kernel.
//! * The route `unreachable default metric 4096` is in the aggregate table.
//!   When each next hop of the multipath route is dead, the lookup ends in
//!   the aggregate table with "Network is unreachable". Without it, the
//!   lookup goes on to the rule `lookup main` and a packet leaves by the
//!   default route of the host.
//! * `rp_filter` 0 of each WAN device. The kernel uses the larger value of
//!   `all` and the device, so a message names a value of `all` above 0.
//!   `remove` writes the old values back.
//!
//! The keys of the `kernel` section are `masquerade`, `mss_clamp`,
//! `kill_switch` and `rp_filter`. Each is `true` when the file has no key.
//! The packets of the clients need `net.ipv4.ip_forward` 1; this code does
//! not set it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::{bail, Context, Result};

use super::{
    attr_u32, attrs, message, messages, Plan, Socket, AF_INET, NLMSG_DONE, NLMSG_ERROR, NLM_F_ACK,
    NLM_F_CREATE, NLM_F_DUMP, NLM_F_REPLACE, NLM_F_REQUEST, RTA_TABLE, RTM_DELROUTE, RTM_GETROUTE,
    RTM_NEWROUTE, RTPROT_STATIC, RT_SCOPE_NOWHERE, RT_SCOPE_UNIVERSE,
};

/// Which guards a plan holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guard {
    pub masquerade: bool,
    pub mss_clamp: bool,
    pub kill_switch: bool,
    pub rp_filter: bool,
}

impl Default for Guard {
    fn default() -> Self {
        Guard {
            masquerade: true,
            mss_clamp: true,
            kill_switch: true,
            rp_filter: true,
        }
    }
}

impl Guard {
    fn nft(&self) -> bool {
        self.masquerade || self.mss_clamp
    }
}

/// The metric of the unreachable route. The multipath route has metric 0.
pub const UNREACHABLE_METRIC: u32 = 4096;
const RTN_UNREACHABLE: u8 = 7;
const RTA_PRIORITY: u16 = 6;

/// The name of the table of nf_tables.
pub const TABLE: &str = "podnet_router";

// ── the unreachable route ───────────────────────────────────────────────────

fn unreachable_body(table: u32, delete: bool) -> Vec<u8> {
    let (protocol, scope, kind) = if delete {
        (0, RT_SCOPE_NOWHERE, 0)
    } else {
        (RTPROT_STATIC, RT_SCOPE_UNIVERSE, RTN_UNREACHABLE)
    };
    let mut b = vec![AF_INET, 0, 0, 0, 0, protocol, scope, kind, 0, 0, 0, 0];
    attr_u32(&mut b, RTA_TABLE, table);
    attr_u32(&mut b, RTA_PRIORITY, UNREACHABLE_METRIC);
    b
}

/// `RTM_NEWROUTE` of `unreachable default metric 4096` in `table`.
/// `NLM_F_REPLACE` makes a second request a no-op.
pub fn new_unreachable_request(table: u32) -> Vec<u8> {
    let flags = NLM_F_ACK | NLM_F_CREATE | NLM_F_REPLACE;
    message(RTM_NEWROUTE, flags, unreachable_body(table, false))
}

pub fn del_unreachable_request(table: u32) -> Vec<u8> {
    message(RTM_DELROUTE, NLM_F_ACK, unreachable_body(table, true))
}

/// Write the kill switch of `plan`, when it has one.
pub(super) fn apply_kill_switch(sock: &Socket, plan: &Plan) -> Result<()> {
    if plan.guard.kill_switch {
        let code = sock.ack(&new_unreachable_request(plan.aggregate));
        super::check("route unreachable default", code, &[])?;
    }
    Ok(())
}

/// Delete the kill switch of `old` when `new` does not hold it.
pub(super) fn remove_stale(sock: &Socket, old: &Plan, new: Option<&Plan>) -> Result<()> {
    let keep = new.is_some_and(|n| n.guard.kill_switch && n.aggregate == old.aggregate);
    if old.guard.kill_switch && !keep {
        let code = sock.ack(&del_unreachable_request(old.aggregate));
        super::check("delete unreachable route", code, &[libc::ESRCH])?;
    }
    Ok(())
}

/// True when `table` has the unreachable default route.
pub fn read_unreachable(table: u32) -> Result<bool> {
    let sock = Socket::open()?;
    let dump = sock.dump(RTM_GETROUTE, vec![AF_INET, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
    Ok(dump.iter().any(|body| {
        if body.len() < 12 || body[0] != AF_INET || body[1] != 0 || body[7] != RTN_UNREACHABLE {
            return false;
        }
        let (mut t, mut metric) = (u32::from(body[4]), 0);
        for (kind, data) in attrs(&body[12..]) {
            match kind {
                RTA_TABLE => t = super::u32_of(data).unwrap_or(t),
                RTA_PRIORITY => metric = super::u32_of(data).unwrap_or(0),
                _ => {}
            }
        }
        t == table && metric == UNREACHABLE_METRIC
    }))
}

// ── rp_filter ───────────────────────────────────────────────────────────────

/// The first value of each file that this process changed.
static SAVED: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

fn rp_path(dev: &str) -> String {
    format!("/proc/sys/net/ipv4/conf/{dev}/rp_filter")
}

fn set_rp_filter(dev: &str) -> Result<()> {
    let path = rp_path(dev);
    let old = std::fs::read_to_string(&path).with_context(|| path.clone())?;
    if let Ok(mut saved) = SAVED.lock() {
        saved.entry(path.clone()).or_insert(old);
    }
    std::fs::write(&path, "0").with_context(|| format!("{path}: write 0"))?;
    if let Ok(all) = std::fs::read_to_string(rp_path("all")) {
        if all.trim() != "0" {
            eprintln!(
                "podnet: route  kernel: rp_filter of all is {}; the kernel uses the larger value",
                all.trim()
            );
        }
    }
    Ok(())
}

fn restore_rp_filter(dev: &str) {
    let path = rp_path(dev);
    let old = SAVED.lock().ok().and_then(|mut s| s.remove(&path));
    if let Some(old) = old {
        // The device may be gone already; that is not a fault.
        let _ = std::fs::write(&path, old.trim());
    }
}

/// The value of `rp_filter` of `dev`.
pub fn read_rp_filter(dev: &str) -> Result<u32> {
    let text = std::fs::read_to_string(rp_path(dev)).with_context(|| rp_path(dev))?;
    text.trim().parse().with_context(|| rp_path(dev))
}

// ── nf_tables ───────────────────────────────────────────────────────────────

const NETLINK_NETFILTER: i32 = 12;
const NFNL_SUBSYS_NFTABLES: u16 = 10;
const NFNL_MSG_BATCH_BEGIN: u16 = 0x10;
const NFNL_MSG_BATCH_END: u16 = 0x11;
const NFT_MSG_NEWTABLE: u16 = 0;
const NFT_MSG_DELTABLE: u16 = 2;
const NFT_MSG_NEWCHAIN: u16 = 3;
const NFT_MSG_NEWRULE: u16 = 6;
const NFT_MSG_GETRULE: u16 = 7;
const NLM_F_APPEND: u16 = 0x800;
const NLA_F_NESTED: u16 = 0x8000;
const NFPROTO_IPV4: u8 = 2;
const NF_INET_FORWARD: u32 = 2;
const NF_INET_POST_ROUTING: u32 = 4;
const NFT_REG_1: u32 = 1;
const NFT_META_OIFNAME: u32 = 7;
const NFT_META_L4PROTO: u32 = 16;
const NFT_PAYLOAD_TRANSPORT_HEADER: u32 = 2;
// enum nft_rt_keys: CLASSID 0, NEXTHOP4 1, NEXTHOP6 2, TCPMSS 3.
const NFT_RT_TCPMSS: u32 = 3;
const NFT_EXTHDR_OP_TCPOPT: u32 = 1;
const TCP_SYN_RST: u8 = 0x06;
const TCP_SYN: u8 = 0x02;
const TCPOPT_MAXSEG: u8 = 2;
const NFT_BYTEORDER_HTON: u32 = 1;
const IFNAMSIZ: usize = 16;

/// The messages of a batch of nf_tables.
struct Builder {
    buf: Vec<u8>,
    start: usize,
    nests: Vec<usize>,
    seq: u32,
}

impl Builder {
    /// A batch with its first message.
    fn new() -> Self {
        let mut b = Builder::bare();
        b.header(NFNL_MSG_BATCH_BEGIN, NLM_F_REQUEST, 0, NFNL_SUBSYS_NFTABLES);
        b.end_msg();
        b
    }

    fn bare() -> Self {
        Builder {
            buf: Vec::new(),
            start: 0,
            nests: Vec::new(),
            seq: 0,
        }
    }

    fn header(&mut self, ty: u16, flags: u16, family: u8, res_id: u16) {
        self.start = self.buf.len();
        self.seq += 1;
        self.buf.extend_from_slice(&0u32.to_ne_bytes());
        self.buf.extend_from_slice(&ty.to_ne_bytes());
        self.buf.extend_from_slice(&flags.to_ne_bytes());
        self.buf.extend_from_slice(&self.seq.to_ne_bytes());
        self.buf.extend_from_slice(&0u32.to_ne_bytes());
        self.buf.extend_from_slice(&[family, 0]);
        self.buf.extend_from_slice(&res_id.to_be_bytes());
    }

    /// Start a message of nf_tables that asks for an acknowledgement.
    fn msg(&mut self, ty: u16, flags: u16) {
        let ty = (NFNL_SUBSYS_NFTABLES << 8) | ty;
        self.header(ty, NLM_F_REQUEST | NLM_F_ACK | flags, NFPROTO_IPV4, 0);
    }

    fn end_msg(&mut self) {
        let len = (self.buf.len() - self.start) as u32;
        self.buf[self.start..self.start + 4].copy_from_slice(&len.to_ne_bytes());
    }

    fn attr(&mut self, ty: u16, data: &[u8]) {
        self.buf
            .extend_from_slice(&((4 + data.len()) as u16).to_ne_bytes());
        self.buf.extend_from_slice(&ty.to_ne_bytes());
        self.buf.extend_from_slice(data);
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }

    fn text(&mut self, ty: u16, text: &str) {
        let mut data = text.as_bytes().to_vec();
        data.push(0);
        self.attr(ty, &data);
    }

    fn be32(&mut self, ty: u16, value: u32) {
        self.attr(ty, &value.to_be_bytes());
    }

    fn nest(&mut self, ty: u16) {
        self.nests.push(self.buf.len());
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf
            .extend_from_slice(&(ty | NLA_F_NESTED).to_ne_bytes());
    }

    fn end_nest(&mut self) {
        if let Some(at) = self.nests.pop() {
            let len = (self.buf.len() - at) as u16;
            self.buf[at..at + 2].copy_from_slice(&len.to_ne_bytes());
        }
    }

    /// One expression: `NFTA_LIST_ELEM { NAME, DATA { body } }`.
    fn expr(&mut self, name: &str, body: impl FnOnce(&mut Self)) {
        self.nest(1);
        self.text(1, name);
        self.nest(2);
        body(self);
        self.end_nest();
        self.end_nest();
    }

    /// `NFTA_DATA_VALUE` in a nest of the type `ty`.
    fn value(&mut self, ty: u16, value: &[u8]) {
        self.nest(ty);
        self.attr(1, value);
        self.end_nest();
    }

    fn finish(mut self) -> Vec<u8> {
        self.header(NFNL_MSG_BATCH_END, NLM_F_REQUEST, 0, NFNL_SUBSYS_NFTABLES);
        self.end_msg();
        self.buf
    }

    // ── the parts of the ruleset ──

    fn table(&mut self) {
        self.msg(NFT_MSG_NEWTABLE, NLM_F_CREATE);
        self.text(1, TABLE);
        self.end_msg();
    }

    fn delete_table(&mut self) {
        self.msg(NFT_MSG_DELTABLE, 0);
        self.text(1, TABLE);
        self.end_msg();
    }

    fn chain(&mut self, name: &str, kind: &str, hook: u32, priority: i32) {
        self.msg(NFT_MSG_NEWCHAIN, NLM_F_CREATE);
        self.text(1, TABLE); // NFTA_CHAIN_TABLE
        self.text(3, name); // NFTA_CHAIN_NAME
        self.nest(4); // NFTA_CHAIN_HOOK
        self.be32(1, hook);
        self.be32(2, priority as u32);
        self.end_nest();
        self.text(7, kind); // NFTA_CHAIN_TYPE
        self.end_msg();
    }

    /// A rule of `chain`; `body` writes its expressions.
    fn rule(&mut self, chain: &str, body: impl FnOnce(&mut Self)) {
        self.msg(NFT_MSG_NEWRULE, NLM_F_CREATE | NLM_F_APPEND);
        self.text(1, TABLE); // NFTA_RULE_TABLE
        self.text(2, chain); // NFTA_RULE_CHAIN
        self.nest(4); // NFTA_RULE_EXPRESSIONS
        body(self);
        self.end_nest();
        self.end_msg();
    }

    /// `meta oifname DEV`.
    fn oifname(&mut self, dev: &str) {
        self.expr("meta", |b| {
            b.be32(1, NFT_REG_1); // NFTA_META_DREG
            b.be32(2, NFT_META_OIFNAME); // NFTA_META_KEY
        });
        let mut name = [0u8; IFNAMSIZ];
        let n = dev.len().min(IFNAMSIZ - 1);
        name[..n].copy_from_slice(&dev.as_bytes()[..n]);
        self.cmp_eq(&name);
    }

    fn cmp_eq(&mut self, value: &[u8]) {
        self.expr("cmp", |b| {
            b.be32(1, NFT_REG_1); // NFTA_CMP_SREG
            b.be32(2, 0); // NFTA_CMP_OP: NFT_CMP_EQ
            b.value(3, value); // NFTA_CMP_DATA
        });
    }
}

/// The batch that replaces the table of `plan`. It holds the whole change,
/// so the kernel applies it as one transaction.
pub fn nft_batch(plan: &Plan) -> Vec<u8> {
    let mut b = Builder::new();
    // The first NEWTABLE makes the delete valid when the table is new.
    b.table();
    b.delete_table();
    if plan.guard.nft() {
        b.table();
        if plan.guard.masquerade {
            b.chain("nat", "nat", NF_INET_POST_ROUTING, 100);
            for w in &plan.wans {
                b.rule("nat", |b| {
                    b.oifname(&w.dev);
                    b.expr("masq", |_| {});
                });
            }
        }
        if plan.guard.mss_clamp {
            b.chain("mss", "filter", NF_INET_FORWARD, -150);
            for w in &plan.wans {
                b.rule("mss", |b| mss_rule(b, &w.dev));
            }
        }
    }
    b.finish()
}

/// `oifname DEV tcp flags & (syn|rst) == syn tcp option maxseg size set rt mtu`:
/// the expressions that `nft --debug=netlink` gives for it.
fn mss_rule(b: &mut Builder, dev: &str) {
    b.oifname(dev);
    b.expr("meta", |b| {
        b.be32(1, NFT_REG_1);
        b.be32(2, NFT_META_L4PROTO);
    });
    b.cmp_eq(&[libc::IPPROTO_TCP as u8]);
    b.expr("payload", |b| {
        b.be32(1, NFT_REG_1); // NFTA_PAYLOAD_DREG
        b.be32(2, NFT_PAYLOAD_TRANSPORT_HEADER);
        b.be32(3, 13); // NFTA_PAYLOAD_OFFSET: the flags
        b.be32(4, 1); // NFTA_PAYLOAD_LEN
    });
    b.expr("bitwise", |b| {
        b.be32(1, NFT_REG_1); // NFTA_BITWISE_SREG
        b.be32(2, NFT_REG_1); // NFTA_BITWISE_DREG
        b.be32(3, 1); // NFTA_BITWISE_LEN
        b.value(4, &[TCP_SYN_RST]); // NFTA_BITWISE_MASK
        b.value(5, &[0]); // NFTA_BITWISE_XOR
    });
    b.cmp_eq(&[TCP_SYN]);
    b.expr("rt", |b| {
        b.be32(1, NFT_REG_1); // NFTA_RT_DREG
        b.be32(2, NFT_RT_TCPMSS); // NFTA_RT_KEY
    });
    // `rt` gives the MSS in host order; the option takes network order. With
    // no swap, a little-endian host writes 1240 as 55300, and the kernel
    // keeps the old MSS because the new one is larger (as `nft` writes it).
    b.expr("byteorder", |b| {
        b.be32(1, NFT_REG_1); // NFTA_BYTEORDER_SREG
        b.be32(2, NFT_REG_1); // NFTA_BYTEORDER_DREG
        b.be32(3, NFT_BYTEORDER_HTON); // NFTA_BYTEORDER_OP
        b.be32(4, 2); // NFTA_BYTEORDER_LEN
        b.be32(5, 2); // NFTA_BYTEORDER_SIZE
    });
    b.expr("exthdr", |b| {
        b.attr(2, &[TCPOPT_MAXSEG]); // NFTA_EXTHDR_TYPE
        b.be32(3, 2); // NFTA_EXTHDR_OFFSET: after kind and length
        b.be32(4, 2); // NFTA_EXTHDR_LEN
        b.be32(6, NFT_EXTHDR_OP_TCPOPT); // NFTA_EXTHDR_OP
        b.be32(7, NFT_REG_1); // NFTA_EXTHDR_SREG
    });
}

/// The batch that removes the table.
fn nft_remove_batch() -> Vec<u8> {
    let mut b = Builder::new();
    b.table();
    b.delete_table();
    b.finish()
}

fn nft_socket() -> Result<Socket> {
    let sock = Socket::open_protocol(NETLINK_NETFILTER)?;
    let timeout = libc::timeval {
        tv_sec: 5,
        tv_usec: 0,
    };
    // SAFETY: `timeout` lives for the call; the length is its size.
    let rc = unsafe {
        libc::setsockopt(
            std::os::fd::AsRawFd::as_raw_fd(&sock.0),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&timeout).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        bail!("nfnetlink socket: {}", std::io::Error::last_os_error());
    }
    Ok(sock)
}

/// The number of messages in `batch` that ask for an acknowledgement.
fn acks_in(batch: &[u8]) -> usize {
    messages(batch)
        .iter()
        .filter(|(kind, _)| *kind != NFNL_MSG_BATCH_BEGIN && *kind != NFNL_MSG_BATCH_END)
        .count()
}

/// Send a batch and read the acknowledgement of each message. The error
/// holds the first errno.
fn send_batch(batch: &[u8]) -> Result<()> {
    let wanted = acks_in(batch);
    let sock = nft_socket()?;
    sock.send(batch).context("nf_tables batch")?;
    let (mut first, mut seen) = (0, 0);
    while seen < wanted {
        let answer = sock.recv().context("nf_tables answer")?;
        for (kind, body) in messages(&answer) {
            if kind == NLMSG_ERROR && body.len() >= 4 {
                seen += 1;
                let code = -i32::from_ne_bytes([body[0], body[1], body[2], body[3]]);
                if code != 0 && first == 0 {
                    first = code;
                }
            }
        }
    }
    if first != 0 {
        bail!(
            "nf_tables table {TABLE}: {}",
            std::io::Error::from_raw_os_error(first)
        );
    }
    Ok(())
}

/// A rule of the table, read back from the kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleView {
    pub chain: String,
    /// The names of the expressions, in order.
    pub exprs: Vec<String>,
    /// The device name of the first comparison of 16 bytes.
    pub dev: Option<String>,
}

fn text_of(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

/// The rules of the table `podnet_router`, in the order of the kernel.
pub fn read_nft() -> Result<Vec<RuleView>> {
    let sock = nft_socket()?;
    let mut b = Builder::bare();
    b.header(
        (NFNL_SUBSYS_NFTABLES << 8) | NFT_MSG_GETRULE,
        NLM_F_REQUEST | NLM_F_DUMP,
        NFPROTO_IPV4,
        0,
    );
    b.end_msg();
    sock.send(&b.buf).context("nf_tables dump")?;
    let mut out = Vec::new();
    loop {
        let answer = sock.recv().context("nf_tables dump")?;
        for (kind, body) in messages(&answer) {
            match kind {
                NLMSG_DONE => return Ok(out),
                NLMSG_ERROR if body.len() >= 4 => {
                    let code = -i32::from_ne_bytes([body[0], body[1], body[2], body[3]]);
                    bail!(
                        "nf_tables dump: {}",
                        std::io::Error::from_raw_os_error(code)
                    );
                }
                k if k == (NFNL_SUBSYS_NFTABLES << 8) | NFT_MSG_NEWRULE && body.len() >= 4 => {
                    out.extend(rule_view(&body[4..]));
                }
                _ => {}
            }
        }
    }
}

fn rule_view(body: &[u8]) -> Option<RuleView> {
    let (mut table, mut chain, mut exprs, mut dev) = (String::new(), String::new(), vec![], None);
    for (kind, data) in attrs(body) {
        match kind {
            1 => table = text_of(data),
            2 => chain = text_of(data),
            4 => {
                for (_, elem) in attrs(data) {
                    for (k, d) in attrs(elem) {
                        if k == 1 {
                            exprs.push(text_of(d));
                        }
                        if k == 2 && dev.is_none() {
                            dev = cmp_device(d);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (table == TABLE).then_some(RuleView { chain, exprs, dev })
}

/// The 16 bytes of `NFTA_CMP_DATA` as a device name.
fn cmp_device(data: &[u8]) -> Option<String> {
    let (_, value) = attrs(data).into_iter().find(|(k, _)| *k == 3)?;
    let (_, bytes) = attrs(value).into_iter().find(|(k, _)| *k == 1)?;
    (bytes.len() == IFNAMSIZ).then(|| text_of(bytes))
}

// ── apply and remove ────────────────────────────────────────────────────────

/// Write the guards that are not routes: `rp_filter` and nf_tables. `old` is
/// the plan that was applied before.
pub(super) fn apply_host(new: &Plan, old: Option<&Plan>) -> Result<()> {
    if let Some(old) = old {
        for w in &old.wans {
            if !new.guard.rp_filter || !new.wans.iter().any(|n| n.dev == w.dev) {
                restore_rp_filter(&w.dev);
            }
        }
    }
    if new.guard.rp_filter {
        for w in &new.wans {
            set_rp_filter(&w.dev)?;
        }
    }
    if new.guard.nft() || old.is_some_and(|o| o.guard.nft()) {
        send_batch(&nft_batch(new))?;
    }
    Ok(())
}

/// Delete the table of nf_tables and write the `rp_filter` values back.
pub(super) fn remove_host(plan: &Plan) -> Result<()> {
    for w in &plan.wans {
        restore_rp_filter(&w.dev);
    }
    if plan.guard.nft() {
        send_batch(&nft_remove_batch())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
