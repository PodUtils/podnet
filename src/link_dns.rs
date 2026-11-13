//! The DNS simplex carrier of the link (T-278, feature `dns`).
//!
//! `pod_link::dns` of PodUtils/podcrates does the work: the frames of the
//! link go up in base32 labels of query names under a zone and come down in
//! TXT answers, one way at a time. The selective-repeat ARQ of `pod-link`
//! runs over the queries, and TLS 1.3 (ALPN `podlink/1`) over the ARQ, each
//! side pinning the Ed25519 key of the other. The dialer starts at most 20
//! queries a second, 8 in flight, and polls when it has nothing to send.
//! This module is the glue:
//!
//! * [`carrier`] makes the carrier from the key of this install, the allow
//!   list, the zone of a listener (`--dns-zone`) and the server of a dialer
//!   (`--dns-server`, else the first `nameserver` of `/etc/resolv.conf`).
//! * [`listen`] runs an authoritative server for the zone on UDP `IP:PORT`.
//! * [`dial`] connects to `KEY@ZONE` through the server. It pins `KEY`.
//! * A refused key exits 77 ([`crate::link::carrier_fault`]).

use std::net::{IpAddr, SocketAddr};

use anyhow::anyhow;
use pod_identity::PublicKey;
use pod_link::dns::Dns;
use pod_link::{Link, Settings};

use crate::exit::{self, Fault};
use crate::identity::{AllowList, Identity};
use crate::link::{carrier_fault, Door};
use crate::link_keyed::{dialed, listen_on, Tune};
use crate::link_probe::{self, Probe};
use crate::link_tcp::{key_of, keys, provider};

/// The resolver file of the host.
pub const RESOLV_CONF: &str = "/etc/resolv.conf";

/// The first `nameserver` of the text of a resolver file, at port 53.
pub fn first_nameserver(text: &str) -> Option<SocketAddr> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? != "nameserver" {
            return None;
        }
        // A link-local IPv6 address can name its interface (`fe80::1%eth0`).
        let ip = words.next()?.split('%').next()?;
        ip.parse::<IpAddr>().ok().map(|ip| SocketAddr::new(ip, 53))
    })
}

/// The carrier of this install: it presents the key of `me`, a listener
/// takes the clients of `allow` and serves the zone of `tune`, and a dial
/// sends its queries to the server of `tune`.
pub fn carrier(me: &Identity, allow: &AllowList, tune: &Tune) -> Dns {
    let mut dns = Dns::new(provider(), key_of(me)).with_clients(keys(allow));
    if let Some(zone) = &tune.dns_zone {
        dns = dns.zone(zone.clone());
    }
    if let Some(server) = tune.dns_server {
        dns = dns.to(server);
    }
    dns
}

fn usage(text: &str) -> anyhow::Error {
    exit::fault(Fault::Usage, anyhow!("the dns carrier: {text}"))
}

/// Listen at `address` (`IP:PORT`, UDP) through `dns`: an authoritative
/// server for its zone. A carrier with no zone exits 64.
pub async fn listen(dns: &Dns, address: &str) -> anyhow::Result<Door> {
    if dns.zone.is_empty() {
        return Err(usage("give the zone of the listener with --dns-zone ZONE"));
    }
    listen_on(dns, address).await
}

/// Connect to `address` (`KEY@ZONE`) through `dns` as the dialer. With no
/// server in `dns`, the queries go to the first `nameserver` of
/// [`RESOLV_CONF`]; with none there, exit 64. The handshake pins `KEY`.
/// Then the key that the listener proved must be in `allow` (else exit 77).
pub async fn dial(
    dns: &Dns,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, zone) = Dns::address(address).map_err(carrier_fault)?;
    let mut dns = dns.clone();
    if dns.server.is_none() {
        let text = std::fs::read_to_string(RESOLV_CONF).unwrap_or_default();
        let server = first_nameserver(&text).ok_or_else(|| {
            usage("give the server with --dns-server IP:PORT (no nameserver in /etc/resolv.conf)")
        })?;
        dns = dns.to(server);
    }
    pod_link::Carrier::available(&dns).map_err(carrier_fault)?;
    dialed(dns.connect(&zone, &name).await, allow, settings)
}

/// The carrier for the registry: the facts (the cost class `highest`) and
/// the host check, no key of this install.
pub fn probe() -> Probe<Dns> {
    Probe::new(Dns::new(provider(), link_probe::key()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_nameserver_of_the_resolver_file() {
        let text = "# a comment\nsearch example\nnameserver fe80::1%eth0\nnameserver 10.0.0.2\n";
        assert_eq!(
            first_nameserver(text),
            Some("[fe80::1]:53".parse().unwrap())
        );
        assert_eq!(
            first_nameserver("nameserver 10.0.0.2\n"),
            Some("10.0.0.2:53".parse().unwrap())
        );
        assert_eq!(first_nameserver("nameserver nope\n"), None);
        assert_eq!(first_nameserver(""), None);
    }
}
