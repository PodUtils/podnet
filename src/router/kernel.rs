//! The kernel mode of the router (T-119): policy routing tables, rules and a
//! multipath default route, written with rtnetlink. It needs CAP_NET_ADMIN.
//! It uses no `ip` program.
//!
//! ```yaml
//! kernel:
//!   wans:
//!     - name: a            # letters, digits, - and _
//!       dev: tun0          # the interface of the WAN
//!       gateway: 10.0.0.1  # optional IPv4 next hop
//!       weight: 2          # 1 to 256 in the aggregate route (default 1)
//!       table: 101         # optional (default 100 + position)
//!       mark: 1            # optional fwmark (default 1 + position)
//!     - name: b
//!       dev: tun1
//!   table: 200             # optional: the aggregate table (default 200)
//!   priority: 1000         # optional: the first rule priority (default 1000)
//! ```
//!
//! The plan has these rules, in this order of priority:
//! 1. `lookup main suppress_prefixlength 0`: the routes of the local subnets
//!    win, and the default route of the main table does not.
//! 2. `fwmark M lookup T` for each WAN: a marked packet leaves by its WAN.
//! 3. `lookup AGGREGATE`: the other packets use the multipath default route.
//!
//! A change is made before it is broken: [`apply`] writes the new routes
//! with `NLM_F_REPLACE`, adds the new rules, and then deletes the old rules
//! that the new plan does not hold. There is no moment with no rule.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// The kernel section of the config file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KernelConfig {
    pub wans: Vec<WanConfig>,
    /// The aggregate table (default 200).
    #[serde(default)]
    pub table: Option<u32>,
    /// The priority of the first rule (default 1000).
    #[serde(default)]
    pub priority: Option<u32>,
}

/// One WAN of the file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WanConfig {
    pub name: String,
    pub dev: String,
    #[serde(default)]
    pub gateway: Option<String>,
    #[serde(default)]
    pub weight: Option<u32>,
    #[serde(default)]
    pub table: Option<u32>,
    #[serde(default)]
    pub mark: Option<u32>,
}

/// One WAN of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wan {
    pub name: String,
    pub dev: String,
    pub gateway: Option<Ipv4Addr>,
    pub weight: u32,
    pub table: u32,
    pub mark: u32,
}

/// What a rule selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Select {
    /// Each packet, with the prefixes of length 0 of the table suppressed.
    SuppressDefault,
    /// The packets with this fwmark.
    Mark(u32),
    /// Each packet.
    Any,
}

/// One rule: at `priority`, a packet that `select` takes uses `table`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rule {
    pub priority: u32,
    pub select: Select,
    pub table: u32,
}

/// The kernel state that the router wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub wans: Vec<Wan>,
    pub aggregate: u32,
    pub priority: u32,
}

/// The table `main`.
pub const MAIN: u32 = 254;
const MAX_WANS: usize = 64;

impl Plan {
    /// Check the section and give its plan. An error names the key.
    pub fn new(config: &KernelConfig) -> Result<Plan> {
        let n = config.wans.len();
        if n == 0 || n > MAX_WANS {
            bail!("kernel.wans: give 1 to {MAX_WANS} WANs");
        }
        let aggregate = config.table.unwrap_or(200);
        let priority = config.priority.unwrap_or(1000);
        let bad_table = |t: u32| t == 0 || (253..=255).contains(&t);
        if bad_table(aggregate) {
            bail!("kernel.table: {aggregate} is a table of the kernel");
        }
        if priority == 0 || priority as usize + n + 2 >= 32766 {
            bail!("kernel.priority: give a number from 1 to {}", 32766 - n - 3);
        }
        let mut wans = Vec::new();
        for (i, w) in config.wans.iter().enumerate() {
            let key = format!("kernel.wans[{i}]");
            if w.name.is_empty()
                || !w
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                bail!("{key}.name: letters, digits, - and _");
            }
            if w.dev.is_empty() || w.dev.len() > 15 || w.dev.contains(['/', '\0', ' ']) {
                bail!("{key}.dev: give an interface name of 1 to 15 bytes");
            }
            let gateway = match &w.gateway {
                None => None,
                Some(g) => Some(
                    g.parse::<Ipv4Addr>()
                        .map_err(|_| anyhow::anyhow!("{key}.gateway: give an IPv4 address"))?,
                ),
            };
            let weight = w.weight.unwrap_or(1);
            if !(1..=256).contains(&weight) {
                bail!("{key}.weight: give 1 to 256");
            }
            let table = w.table.unwrap_or(100 + i as u32);
            if bad_table(table) || table == aggregate || table == MAIN {
                bail!("{key}.table: {table} is not free");
            }
            let mark = w.mark.unwrap_or(1 + i as u32);
            if mark == 0 {
                bail!("{key}.mark: give 1 or more");
            }
            wans.push(Wan {
                name: w.name.clone(),
                dev: w.dev.clone(),
                gateway,
                weight,
                table,
                mark,
            });
        }
        for (i, w) in wans.iter().enumerate() {
            if let Some(j) = wans[..i].iter().position(|o| {
                o.name == w.name || o.table == w.table || o.mark == w.mark || o.dev == w.dev
            }) {
                bail!("kernel.wans[{i}]: the name, device, table or mark is the same as in kernel.wans[{j}]");
            }
        }
        Ok(Plan {
            wans,
            aggregate,
            priority,
        })
    }

    /// The plan in the JSON form of the mwan director (T-121): `wans` with
    /// `name`, `iface`, `table`, `weight` and the optional `gateway` and
    /// `check_target`, and `clients` (none: the router file has no source
    /// rules).
    pub fn mwan_json(&self, check_target: Option<&str>) -> serde_json::Value {
        let wans: Vec<serde_json::Value> = self
            .wans
            .iter()
            .map(|w| {
                let mut wan = serde_json::json!({
                    "name": w.name,
                    "iface": w.dev,
                    "table": w.table,
                    "weight": w.weight,
                });
                if let Some(gateway) = w.gateway {
                    wan["gateway"] = gateway.to_string().into();
                }
                if let Some(target) = check_target {
                    wan["check_target"] = target.into();
                }
                wan
            })
            .collect();
        serde_json::json!({ "wans": wans, "clients": [] })
    }

    /// The rules of the plan, by priority.
    pub fn rules(&self) -> Vec<Rule> {
        let mut rules = vec![Rule {
            priority: self.priority,
            select: Select::SuppressDefault,
            table: MAIN,
        }];
        for (i, w) in self.wans.iter().enumerate() {
            rules.push(Rule {
                priority: self.priority + 1 + i as u32,
                select: Select::Mark(w.mark),
                table: w.table,
            });
        }
        rules.push(Rule {
            priority: self.priority + 1 + self.wans.len() as u32,
            select: Select::Any,
            table: self.aggregate,
        });
        rules
    }

    /// The default route of each table: the table and its next hops
    /// (device, gateway, weight).
    pub fn routes(&self) -> Vec<(u32, Vec<Hop>)> {
        let hop = |w: &Wan| Hop {
            dev: w.dev.clone(),
            gateway: w.gateway,
            weight: w.weight,
        };
        let mut routes: Vec<_> = self.wans.iter().map(|w| (w.table, vec![hop(w)])).collect();
        routes.push((self.aggregate, self.wans.iter().map(hop).collect()));
        routes
    }

    /// The tables that the plan writes.
    fn tables(&self) -> BTreeSet<u32> {
        self.routes().into_iter().map(|(t, _)| t).collect()
    }
}

/// A resolved next hop: the ifindex, the gateway and the weight.
pub type NextHop = (u32, Option<Ipv4Addr>, u32);

/// One next hop of a default route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub dev: String,
    pub gateway: Option<Ipv4Addr>,
    pub weight: u32,
}

// ── the messages ────────────────────────────────────────────────────────────

const RTM_NEWROUTE: u16 = 24;
const RTM_DELROUTE: u16 = 25;
const RTM_GETROUTE: u16 = 26;
const RTM_NEWRULE: u16 = 32;
const RTM_DELRULE: u16 = 33;
const RTM_GETRULE: u16 = 34;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_DUMP: u16 = 0x300;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const RTA_OIF: u16 = 4;
const RTA_GATEWAY: u16 = 5;
const RTA_MULTIPATH: u16 = 9;
const RTA_TABLE: u16 = 15;
const FRA_PRIORITY: u16 = 6;
const FRA_FWMARK: u16 = 10;
const FRA_SUPPRESS_PREFIXLEN: u16 = 14;
const FRA_TABLE: u16 = 15;
const FRA_FWMASK: u16 = 16;
const FR_ACT_TO_TBL: u8 = 1;
const AF_INET: u8 = libc::AF_INET as u8;
const RTPROT_STATIC: u8 = 4;
const RT_SCOPE_UNIVERSE: u8 = 0;
const RT_SCOPE_LINK: u8 = 253;
const RT_SCOPE_NOWHERE: u8 = 255;
const RTN_UNICAST: u8 = 1;

fn attr(buf: &mut Vec<u8>, kind: u16, data: &[u8]) {
    buf.extend_from_slice(&((4 + data.len()) as u16).to_ne_bytes());
    buf.extend_from_slice(&kind.to_ne_bytes());
    buf.extend_from_slice(data);
    buf.resize(buf.len().next_multiple_of(4), 0);
}

fn attr_u32(buf: &mut Vec<u8>, kind: u16, value: u32) {
    attr(buf, kind, &value.to_ne_bytes());
}

/// The header of a request. `body` follows it.
fn message(kind: u16, flags: u16, body: Vec<u8>) -> Vec<u8> {
    let mut msg = Vec::with_capacity(16 + body.len());
    msg.extend_from_slice(&((16 + body.len()) as u32).to_ne_bytes());
    msg.extend_from_slice(&kind.to_ne_bytes());
    msg.extend_from_slice(&(NLM_F_REQUEST | flags).to_ne_bytes());
    msg.extend_from_slice(&1u32.to_ne_bytes());
    msg.extend_from_slice(&0u32.to_ne_bytes());
    msg.extend_from_slice(&body);
    msg
}

/// The body of a rule message (`struct fib_rule_hdr` and the attributes).
fn rule_body(rule: &Rule) -> Vec<u8> {
    let mut b = vec![AF_INET, 0, 0, 0, 0, 0, 0, FR_ACT_TO_TBL, 0, 0, 0, 0];
    attr_u32(&mut b, FRA_PRIORITY, rule.priority);
    attr_u32(&mut b, FRA_TABLE, rule.table);
    match rule.select {
        Select::SuppressDefault => attr_u32(&mut b, FRA_SUPPRESS_PREFIXLEN, 0),
        Select::Mark(mark) => {
            attr_u32(&mut b, FRA_FWMARK, mark);
            attr_u32(&mut b, FRA_FWMASK, u32::MAX);
        }
        Select::Any => {}
    }
    b
}

/// `RTM_NEWRULE`. `NLM_F_EXCL` makes the kernel refuse a copy of a rule.
pub fn new_rule_request(rule: &Rule) -> Vec<u8> {
    let flags = NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL;
    message(RTM_NEWRULE, flags, rule_body(rule))
}

pub fn del_rule_request(rule: &Rule) -> Vec<u8> {
    message(RTM_DELRULE, NLM_F_ACK, rule_body(rule))
}

/// The body of a route message (`struct rtmsg` and the attributes). `hops`
/// holds the next hops as (ifindex, gateway, weight).
fn route_body(table: u32, hops: &[NextHop], delete: bool) -> Vec<u8> {
    let scope = match (delete, hops) {
        (true, _) => RT_SCOPE_NOWHERE,
        (false, [(_, None, _)]) => RT_SCOPE_LINK,
        _ => RT_SCOPE_UNIVERSE,
    };
    let (protocol, kind) = if delete {
        (0, 0)
    } else {
        (RTPROT_STATIC, RTN_UNICAST)
    };
    let mut b = vec![AF_INET, 0, 0, 0, 0, protocol, scope, kind, 0, 0, 0, 0];
    attr_u32(&mut b, RTA_TABLE, table);
    match hops {
        [] => {}
        [(ifindex, gateway, _)] => {
            attr_u32(&mut b, RTA_OIF, *ifindex);
            if let Some(g) = gateway {
                attr(&mut b, RTA_GATEWAY, &g.octets());
            }
        }
        many => {
            let start = b.len();
            b.extend_from_slice(&[0; 4]);
            for (ifindex, gateway, weight) in many {
                let at = b.len();
                b.extend_from_slice(&[0; 4]);
                b[at + 3] = (weight - 1) as u8;
                b.extend_from_slice(&ifindex.to_ne_bytes());
                if let Some(g) = gateway {
                    attr(&mut b, RTA_GATEWAY, &g.octets());
                }
                let len = (b.len() - at) as u16;
                b[at..at + 2].copy_from_slice(&len.to_ne_bytes());
            }
            let len = (b.len() - start) as u16;
            b[start..start + 2].copy_from_slice(&len.to_ne_bytes());
            b[start + 2..start + 4].copy_from_slice(&RTA_MULTIPATH.to_ne_bytes());
        }
    }
    b
}

/// `RTM_NEWROUTE` of a default route. `NLM_F_REPLACE` replaces a route of
/// the table in place.
pub fn new_route_request(table: u32, hops: &[NextHop]) -> Vec<u8> {
    let flags = NLM_F_ACK | NLM_F_CREATE | NLM_F_REPLACE;
    message(RTM_NEWROUTE, flags, route_body(table, hops, false))
}

pub fn del_route_request(table: u32) -> Vec<u8> {
    message(RTM_DELROUTE, NLM_F_ACK, route_body(table, &[], true))
}

// ── the socket ──────────────────────────────────────────────────────────────

struct Socket(OwnedFd);

impl Socket {
    fn open() -> Result<Socket> {
        // SAFETY: socket with constant arguments.
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                libc::NETLINK_ROUTE,
            )
        };
        if fd < 0 {
            bail!("netlink socket: {}", std::io::Error::last_os_error());
        }
        // SAFETY: a new descriptor that nothing else owns.
        Ok(Socket(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    fn send(&self, msg: &[u8]) -> std::io::Result<()> {
        // SAFETY: a zeroed sockaddr_nl is valid; the kernel is pid 0.
        let mut kernel: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        kernel.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        // SAFETY: `msg` and `kernel` live for the call.
        let sent = unsafe {
            libc::sendto(
                self.0.as_raw_fd(),
                msg.as_ptr().cast(),
                msg.len(),
                0,
                std::ptr::from_ref(&kernel).cast(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if sent < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn recv(&self) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; 65536];
        // SAFETY: `buf` has room for the read.
        let n = unsafe { libc::recv(self.0.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        buf.truncate(n as usize);
        Ok(buf)
    }

    /// Send a request and read its acknowledgement: the errno (0 is success).
    fn ack(&self, msg: &[u8]) -> std::io::Result<i32> {
        self.send(msg)?;
        let answer = self.recv()?;
        for (kind, body) in messages(&answer) {
            if kind == NLMSG_ERROR && body.len() >= 4 {
                return Ok(-i32::from_ne_bytes([body[0], body[1], body[2], body[3]]));
            }
        }
        Err(std::io::Error::other("no acknowledgement"))
    }

    /// Send a dump request: the bodies of the answers of `kind`.
    fn dump(&self, kind: u16, body: Vec<u8>) -> Result<Vec<Vec<u8>>> {
        self.send(&message(kind, NLM_F_DUMP, body))?;
        let mut out = Vec::new();
        loop {
            let answer = self.recv()?;
            for (k, body) in messages(&answer) {
                match k {
                    NLMSG_DONE => return Ok(out),
                    NLMSG_ERROR if body.len() >= 4 => {
                        let code = i32::from_ne_bytes([body[0], body[1], body[2], body[3]]);
                        bail!("netlink dump: {}", std::io::Error::from_raw_os_error(-code));
                    }
                    k if k == kind - 2 => out.push(body.to_vec()),
                    _ => {}
                }
            }
        }
    }
}

/// The (type, body) of each message of a netlink answer.
fn messages(mut data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    while data.len() >= 16 {
        let len = u32::from_ne_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if len < 16 || len > data.len() {
            break;
        }
        out.push((u16::from_ne_bytes([data[4], data[5]]), &data[16..len]));
        data = &data[len.next_multiple_of(4).min(data.len())..];
    }
    out
}

/// The (type, data) of each attribute of `data`.
fn attrs(mut data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    while data.len() >= 4 {
        let len = u16::from_ne_bytes([data[0], data[1]]) as usize;
        if len < 4 || len > data.len() {
            break;
        }
        out.push((
            u16::from_ne_bytes([data[2], data[3]]) & 0x3fff,
            &data[4..len],
        ));
        data = &data[len.next_multiple_of(4).min(data.len())..];
    }
    out
}

fn u32_of(data: &[u8]) -> Option<u32> {
    Some(u32::from_ne_bytes(data.get(..4)?.try_into().ok()?))
}

fn ifindex(dev: &str) -> Result<u32> {
    let name = CString::new(dev).with_context(|| dev.to_string())?;
    // SAFETY: `name` is a C string that lives for the call.
    match unsafe { libc::if_nametoindex(name.as_ptr()) } {
        0 => bail!("kernel: the interface {dev:?} does not exist"),
        n => Ok(n),
    }
}

fn check(what: &str, code: std::io::Result<i32>, allowed: &[i32]) -> Result<()> {
    match code.with_context(|| what.to_string())? {
        0 => Ok(()),
        c if allowed.contains(&c) => Ok(()),
        c => bail!("{what}: {}", std::io::Error::from_raw_os_error(c)),
    }
}

fn describe(rule: &Rule) -> String {
    format!(
        "rule {} {:?} lookup {}",
        rule.priority, rule.select, rule.table
    )
}

// ── apply, remove, read ─────────────────────────────────────────────────────

/// Write `new` to the kernel of this network namespace. With `old` (the plan
/// that was applied before), the rules and the tables that `new` does not
/// hold go away after the new ones are in place.
pub fn apply(new: &Plan, old: Option<&Plan>) -> Result<()> {
    let sock = Socket::open()?;
    for (table, hops) in new.routes() {
        let mut resolved = Vec::new();
        for h in &hops {
            resolved.push((ifindex(&h.dev)?, h.gateway, h.weight));
        }
        let what = format!("route default table {table}");
        check(&what, sock.ack(&new_route_request(table, &resolved)), &[])?;
    }
    let wanted = new.rules();
    for rule in &wanted {
        // EEXIST: the rule is in the kernel already.
        let code = sock.ack(&new_rule_request(rule));
        check(&describe(rule), code, &[libc::EEXIST])?;
    }
    if let Some(old) = old {
        remove_except(&sock, old, &wanted, &new.tables())?;
    }
    Ok(())
}

/// Delete the rules and the routes of `plan`.
pub fn remove(plan: &Plan) -> Result<()> {
    let sock = Socket::open()?;
    remove_except(&sock, plan, &[], &BTreeSet::new())
}

fn remove_except(
    sock: &Socket,
    plan: &Plan,
    keep_rules: &[Rule],
    keep_tables: &BTreeSet<u32>,
) -> Result<()> {
    let mut first = Ok(());
    for rule in plan.rules().iter().filter(|r| !keep_rules.contains(r)) {
        let code = sock.ack(&del_rule_request(rule));
        let done = check(&describe(rule), code, &[libc::ENOENT]);
        first = first.and(done);
    }
    for table in plan.tables().difference(keep_tables) {
        let code = sock.ack(&del_route_request(*table));
        let done = check(&format!("delete route table {table}"), code, &[libc::ESRCH]);
        first = first.and(done);
    }
    first
}

/// The rules of the kernel with a priority in `from..=to`.
pub fn read_rules(from: u32, to: u32) -> Result<Vec<Rule>> {
    let sock = Socket::open()?;
    let mut out = Vec::new();
    for body in sock.dump(RTM_GETRULE, vec![AF_INET, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])? {
        if body.len() < 12 || body[0] != AF_INET {
            continue;
        }
        let (mut priority, mut mark, mut table, mut suppress) =
            (0, None, u32::from(body[4]), false);
        for (kind, data) in attrs(&body[12..]) {
            match kind {
                FRA_PRIORITY => priority = u32_of(data).unwrap_or(0),
                FRA_FWMARK => mark = u32_of(data),
                FRA_TABLE => table = u32_of(data).unwrap_or(table),
                FRA_SUPPRESS_PREFIXLEN => suppress = u32_of(data) == Some(0),
                _ => {}
            }
        }
        if (from..=to).contains(&priority) {
            let select = match (suppress, mark) {
                (true, _) => Select::SuppressDefault,
                (false, Some(m)) => Select::Mark(m),
                (false, None) => Select::Any,
            };
            out.push(Rule {
                priority,
                select,
                table,
            });
        }
    }
    out.sort();
    Ok(out)
}

/// The next hops (ifindex, gateway, weight) of the default route of `table`;
/// `None` when the table has none.
pub fn read_default(table: u32) -> Result<Option<Vec<NextHop>>> {
    let sock = Socket::open()?;
    let gateway = |d: &[u8]| <[u8; 4]>::try_from(d).ok().map(Ipv4Addr::from);
    for body in sock.dump(RTM_GETROUTE, vec![AF_INET, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])? {
        if body.len() < 12 || body[0] != AF_INET || body[1] != 0 {
            continue;
        }
        let (mut t, mut oif, mut gw, mut hops) = (u32::from(body[4]), 0, None, Vec::new());
        for (kind, data) in attrs(&body[12..]) {
            match kind {
                RTA_TABLE => t = u32_of(data).unwrap_or(t),
                RTA_OIF => oif = u32_of(data).unwrap_or(0),
                RTA_GATEWAY => gw = gateway(data),
                RTA_MULTIPATH => {
                    let mut rest = data;
                    while rest.len() >= 8 {
                        let len = usize::from(u16::from_ne_bytes([rest[0], rest[1]]));
                        if len < 8 || len > rest.len() {
                            break;
                        }
                        let w = u32::from(rest[3]) + 1;
                        let i = u32_of(&rest[4..]).unwrap_or(0);
                        let g = attrs(&rest[8..len])
                            .into_iter()
                            .find(|(k, _)| *k == RTA_GATEWAY)
                            .and_then(|(_, d)| gateway(d));
                        hops.push((i, g, w));
                        rest = &rest[len.next_multiple_of(4).min(rest.len())..];
                    }
                }
                _ => {}
            }
        }
        if t == table {
            return Ok(Some(if hops.is_empty() {
                vec![(oif, gw, 1)]
            } else {
                hops
            }));
        }
    }
    Ok(None)
}

/// The index of the interface `dev`, for a reader of [`read_default`].
pub fn index_of(dev: &str) -> Result<u32> {
    ifindex(dev)
}

#[cfg(test)]
mod tests;
