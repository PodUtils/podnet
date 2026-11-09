#[allow(unused_imports)]
use crate::prelude::*;
use std::net::Ipv6Addr;

// ── fake-IP DNS ─────────────────────────────────────────────────────────────

pub(crate) struct FakeDns {
    pub(crate) by_ip: HashMap<Ipv4Addr, String>,
    pub(crate) by_name: HashMap<String, Ipv4Addr>,
    pub(crate) next: u32,
    /// Answer AAAA with a fake IPv6 address (the seccomp backend only: the
    /// TUN of the netns backend has no IPv6).
    pub(crate) v6: bool,
}

impl FakeDns {
    /// Answer a query from the pool. An A query gets the fake address of the
    /// name. Other types get an empty NOERROR answer, so the caller uses IPv4.
    /// A name that does not fit in the pool gets SERVFAIL (T-145).
    pub(crate) fn answer(&mut self, query: &crate::dns::ParsedQuery) -> Vec<u8> {
        let servfail = || crate::dns::build_response(query, &[], crate::dns::RCODE_SERVFAIL);
        if query.qtype == crate::dns::TYPE_A {
            let Some(fake) = self.address_for(&query.name) else {
                return servfail();
            };
            crate::dns::build_response(query, &[IpAddr::V4(fake)], crate::dns::RCODE_NOERROR)
        } else if query.qtype == crate::dns::TYPE_AAAA && self.v6 {
            // A pinned name (--map) has no fake address: it gets no AAAA.
            let Some(v4) = self.address_for(&query.name) else {
                return servfail();
            };
            let answers = if self.by_ip.contains_key(&v4) {
                vec![IpAddr::V6(fake_v6(v4))]
            } else {
                Vec::new()
            };
            crate::dns::build_response(query, &answers, crate::dns::RCODE_NOERROR)
        } else {
            crate::dns::build_response(query, &[], crate::dns::RCODE_NOERROR)
        }
    }

    /// The name of a fake address of either family.
    pub(crate) fn name_for_ip(&self, ip: IpAddr) -> Option<&str> {
        match ip {
            IpAddr::V4(v4) => self.name_for(v4),
            IpAddr::V6(v6) => unfake_v6(v6).and_then(|v4| self.name_for(v4)),
        }
    }

    /// Pin a name to a real address, so it resolves locally with no fake IP.
    /// A name that has a pool address keeps it: a program can use it.
    pub(crate) fn pin(&mut self, name: &str, ip: IpAddr) {
        let name = crate::dns::normalize(name);
        let live = self
            .by_name
            .get(&name)
            .is_some_and(|v4| self.by_ip.contains_key(v4));
        if let (IpAddr::V4(v4), false) = (ip, live) {
            self.by_name.insert(name, v4);
        }
    }

    pub(crate) fn new() -> Self {
        Self {
            by_ip: HashMap::new(),
            by_name: HashMap::new(),
            next: 0,
            v6: false,
        }
    }

    /// The address of `name` (no case): its pin, its pool address, else the
    /// next pool address. `None` at the end of the pool: the pool does not
    /// wrap, because a reused address would send the flows of an older name
    /// to the newer name (T-145).
    pub(crate) fn address_for(&mut self, name: &str) -> Option<Ipv4Addr> {
        let name = crate::dns::normalize(name);
        if let Some(ip) = self.by_name.get(&name) {
            return Some(*ip);
        }
        if self.next >= FAKE_IP_COUNT {
            return None;
        }
        let ip = Ipv4Addr::from((FAKE_IP_BASE + self.next).to_be_bytes());
        self.next += 1;
        self.by_name.insert(name.clone(), ip);
        self.by_ip.insert(ip, name);
        Some(ip)
    }

    pub(crate) fn name_for(&self, ip: Ipv4Addr) -> Option<&str> {
        self.by_ip.get(&ip).map(String::as_str)
    }
}

/// The fake IPv6 address of a fake IPv4 address: `FAKE_IP6_PREFIX` (a ULA)
/// and the 32 bits of the IPv4 address.
pub(crate) fn fake_v6(v4: Ipv4Addr) -> Ipv6Addr {
    let mut octets = FAKE_IP6_PREFIX;
    octets[12..].copy_from_slice(&v4.octets());
    Ipv6Addr::from(octets)
}

/// The fake IPv4 address of a fake IPv6 address, or `None` for any other.
pub(crate) fn unfake_v6(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    let o = v6.octets();
    (o[..12] == FAKE_IP6_PREFIX[..12]).then(|| Ipv4Addr::new(o[12], o[13], o[14], o[15]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue 99: a fake IPv6 address carries the fake IPv4 address, and
    /// only an address of the prefix maps back.
    #[test]
    fn a_fake_ipv6_address_maps_back() {
        let v4 = Ipv4Addr::new(198, 18, 0, 7);
        let v6 = fake_v6(v4);
        assert_eq!(v6.to_string(), "fd63:6672:7300::c612:7");
        assert_eq!(unfake_v6(v6), Some(v4));
        assert_eq!(unfake_v6("2606:4700:4700::1111".parse().unwrap()), None);
        let mut dns = FakeDns::new();
        let fake = dns.address_for("example.com").unwrap();
        assert_eq!(
            dns.name_for_ip(IpAddr::V6(fake_v6(fake))),
            Some("example.com")
        );
    }

    fn query(name: &str) -> crate::dns::ParsedQuery {
        let message = crate::dns::build_query(7, name, crate::dns::TYPE_A).unwrap();
        crate::dns::ParsedQuery::parse(&message).unwrap()
    }

    /// The pool gives each address once: the name after the last address
    /// gets `None`, and the first name keeps its address.
    #[test]
    fn fake_pool_refuses_past_its_end() {
        let mut dns = FakeDns::new();
        let first = dns.address_for("n0.example").unwrap();
        for n in 1..FAKE_IP_COUNT {
            assert!(dns.address_for(&format!("n{n}.example")).is_some());
        }
        assert_eq!(dns.address_for("over.example"), None);
        assert_eq!(dns.name_for(first), Some("n0.example"));
        assert_eq!(dns.address_for("n0.example"), Some(first));
        assert_eq!(dns.by_ip.len(), FAKE_IP_COUNT as usize);
    }

    #[test]
    fn fake_names_ignore_case() {
        let mut dns = FakeDns::new();
        let a = dns.address_for("Example.COM").unwrap();
        assert_eq!(dns.address_for("example.com."), Some(a));
        assert_eq!(dns.name_for(a), Some("example.com"));
        assert_eq!(query("WWW.Example.Com").name, "www.example.com");
        assert_eq!(dns.next, 1);
        dns.pin("Pinned.Example", "10.0.0.5".parse().unwrap());
        assert_eq!(
            dns.address_for("pinned.example"),
            Some(Ipv4Addr::new(10, 0, 0, 5))
        );
    }

    #[test]
    fn fake_pin_does_not_evict_a_live_name() {
        let mut dns = FakeDns::new();
        let fake = dns.address_for("live.example").unwrap();
        dns.pin("LIVE.example", "10.0.0.5".parse().unwrap());
        assert_eq!(dns.address_for("live.example"), Some(fake));
        assert_eq!(dns.name_for(fake), Some("live.example"));
    }

    #[test]
    fn servfail_answers_a_name_that_does_not_fit() {
        let mut dns = FakeDns::new();
        dns.v6 = true;
        dns.next = FAKE_IP_COUNT;
        for qtype in [crate::dns::TYPE_A, crate::dns::TYPE_AAAA] {
            let mut parsed = query("late.example");
            parsed.qtype = qtype;
            let answer = dns.answer(&parsed);
            assert_eq!(answer[3] & 0x0f, crate::dns::RCODE_SERVFAIL);
            assert_eq!(&answer[6..8], &[0, 0], "no answer record");
        }
        let full = dns.answer(&query("other.example"));
        assert_eq!(full[3] & 0x0f, crate::dns::RCODE_SERVFAIL);
        dns.next = 0;
        let fits = dns.answer(&query("other.example"));
        assert_eq!(fits[3] & 0x0f, crate::dns::RCODE_NOERROR);
    }
}
