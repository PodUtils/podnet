//! A reader of the wg-quick format (T-116): the file of `wg-quick`, of a
//! self-hosted WireGuard peer or of the export of a VPN.
//!
//! * `[Interface]`: `PrivateKey`, `Address`, `DNS`, `MTU`, `ListenPort`
//!   (the UDP port of `podnet wg serve`). The keys that only `wg-quick`
//!   uses on the host (`FwMark`, `Table`, `SaveConfig`, `PreUp`, `PostUp`,
//!   `PreDown`, `PostDown`) are ignored.
//! * `[Peer]`: `PublicKey`, `PresharedKey`, `Endpoint`, `AllowedIPs`,
//!   `PersistentKeepalive`, and `Reserved` (bytes 1 to 3 of each message:
//!   three numbers, or the base64 of three bytes). The reserved bytes are
//!   zero when the file has no `Reserved`.
//! * A name is not case sensitive. A `#` starts a comment. An unknown
//!   name or section is an error with its line.
//! * The private key comes from the file only: [`read_file`] takes a path.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context as _, Result};
use base64::Engine as _;

/// A secret key of 32 bytes. Its `Debug` shows no byte.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey(pub [u8; 32]);

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// An address with a prefix length: `10.0.0.2/32`, `fd00::2/128`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl FromStr for Cidr {
    type Err = anyhow::Error;

    /// An address with no prefix gets the full length (32 or 128).
    fn from_str(s: &str) -> Result<Self> {
        let (addr, prefix) = match s.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (s, None),
        };
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| anyhow!("{s:?} is not an IP address"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| anyhow!("{s:?} has a bad prefix length"))?,
            None => max,
        };
        Ok(Cidr { addr, prefix })
    }
}

/// The endpoint of a peer: a host (an IP address or a name) and a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// The host, with no brackets.
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    /// The socket address, when the host is an IP address.
    pub fn socket_addr(&self) -> Option<SocketAddr> {
        let ip: IpAddr = self.host.parse().ok()?;
        Some(SocketAddr::new(ip, self.port))
    }
}

impl FromStr for Endpoint {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (host, port) = if let Some(rest) = s.strip_prefix('[') {
            let (host, port) = rest
                .split_once("]:")
                .ok_or_else(|| anyhow!("{s:?} is not [HOST]:PORT"))?;
            (host, port)
        } else {
            let (host, port) = s
                .rsplit_once(':')
                .ok_or_else(|| anyhow!("{s:?} has no port"))?;
            if host.contains(':') {
                bail!("{s:?}: write an IPv6 address in brackets");
            }
            (host, port)
        };
        if host.is_empty() {
            bail!("{s:?} has no host");
        }
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| anyhow!("{s:?} has a bad port"))?;
        Ok(Endpoint {
            host: host.to_string(),
            port,
        })
    }
}

/// The section `[Interface]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub private_key: SecretKey,
    pub addresses: Vec<Cidr>,
    /// The DNS servers of `DNS`.
    pub dns: Vec<IpAddr>,
    /// The search domains of `DNS` (each value that is not an address).
    pub dns_search: Vec<String>,
    pub mtu: Option<u16>,
    /// The UDP port of `ListenPort` (0: a port of the system).
    pub listen_port: Option<u16>,
}

/// A section `[Peer]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub public_key: [u8; 32],
    pub preshared_key: Option<SecretKey>,
    pub endpoint: Option<Endpoint>,
    pub allowed_ips: Vec<Cidr>,
    /// The interval in seconds; `None` for `off` or no value.
    pub persistent_keepalive: Option<u16>,
    /// Bytes 1 to 3 of each message. Zero by default.
    pub reserved: [u8; 3],
}

/// A wg-quick file: one interface and its peers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WgQuick {
    pub interface: Interface,
    pub peers: Vec<Peer>,
}

impl WgQuick {
    /// The one peer of the file. More peers, or none, is an error.
    pub fn single_peer(&self) -> Result<&Peer> {
        match self.peers.as_slice() {
            [peer] => Ok(peer),
            [] => bail!("the file has no [Peer]"),
            _ => bail!("the file has {} peers: podnet uses one", self.peers.len()),
        }
    }
}

/// Read the wg-quick file at `path`. A file that other users can read
/// gives a warning: it holds a private key.
pub fn read_file(path: &Path) -> Result<WgQuick> {
    use std::os::unix::fs::PermissionsExt as _;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the WireGuard file {}", path.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().mode() & 0o077 != 0 {
            tracing::warn!(file = %path.display(), "the WireGuard file holds a private key, and other users can read it (chmod 600)");
        }
    }
    parse(&text).with_context(|| format!("the WireGuard file {}", path.display()))
}

#[derive(Default)]
struct PartialInterface {
    private_key: Option<SecretKey>,
    addresses: Vec<Cidr>,
    dns: Vec<IpAddr>,
    dns_search: Vec<String>,
    mtu: Option<u16>,
    listen_port: Option<u16>,
}

#[derive(Default)]
struct PartialPeer {
    line: usize,
    public_key: Option<[u8; 32]>,
    preshared_key: Option<SecretKey>,
    endpoint: Option<Endpoint>,
    allowed_ips: Vec<Cidr>,
    persistent_keepalive: Option<u16>,
    reserved: [u8; 3],
}

enum Section {
    None,
    Interface,
    Peer,
}

/// Parse the text of a wg-quick file.
pub fn parse(text: &str) -> Result<WgQuick> {
    let mut section = Section::None;
    let mut interface: Option<PartialInterface> = None;
    let mut peers: Vec<PartialPeer> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let n = index + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = match line.to_ascii_lowercase().as_str() {
                "[interface]" => {
                    if interface.is_some() {
                        bail!("line {n}: a second [Interface]");
                    }
                    interface = Some(PartialInterface::default());
                    Section::Interface
                }
                "[peer]" => {
                    peers.push(PartialPeer {
                        line: n,
                        ..PartialPeer::default()
                    });
                    Section::Peer
                }
                _ => bail!("line {n}: the section {line} is unknown"),
            };
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow!("line {n}: no '=' in the line"))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        let result = match section {
            Section::None => Err(anyhow!("the line is before each section")),
            Section::Interface => match interface.as_mut() {
                Some(iface) => interface_value(iface, &name, value),
                None => Err(anyhow!("no [Interface]")),
            },
            Section::Peer => match peers.last_mut() {
                Some(peer) => peer_value(peer, &name, value),
                None => Err(anyhow!("no [Peer]")),
            },
        };
        result.with_context(|| format!("line {n}"))?;
    }
    let iface = interface.ok_or_else(|| anyhow!("the file has no [Interface]"))?;
    let interface = Interface {
        private_key: iface
            .private_key
            .ok_or_else(|| anyhow!("[Interface] has no PrivateKey"))?,
        addresses: iface.addresses,
        dns: iface.dns,
        dns_search: iface.dns_search,
        mtu: iface.mtu,
        listen_port: iface.listen_port,
    };
    let peers = peers
        .into_iter()
        .map(|p| {
            Ok(Peer {
                public_key: p
                    .public_key
                    .ok_or_else(|| anyhow!("the [Peer] of line {} has no PublicKey", p.line))?,
                preshared_key: p.preshared_key,
                endpoint: p.endpoint,
                allowed_ips: p.allowed_ips,
                persistent_keepalive: p.persistent_keepalive,
                reserved: p.reserved,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WgQuick { interface, peers })
}

fn list(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').map(str::trim).filter(|v| !v.is_empty())
}

fn interface_value(iface: &mut PartialInterface, name: &str, value: &str) -> Result<()> {
    match name {
        "privatekey" => iface.private_key = Some(SecretKey(key(value)?)),
        "address" => {
            for v in list(value) {
                iface.addresses.push(v.parse()?);
            }
        }
        "dns" => {
            for v in list(value) {
                match v.parse::<IpAddr>() {
                    Ok(ip) => iface.dns.push(ip),
                    Err(_) => iface.dns_search.push(v.to_string()),
                }
            }
        }
        "mtu" => {
            iface.mtu = Some(
                value
                    .parse::<u16>()
                    .ok()
                    .filter(|m| *m >= 576)
                    .ok_or_else(|| anyhow!("MTU {value:?} is not a number from 576 to 65535"))?,
            )
        }
        "listenport" => {
            iface.listen_port = Some(
                value
                    .parse::<u16>()
                    .map_err(|_| anyhow!("ListenPort {value:?} is not a number from 0 to 65535"))?,
            )
        }
        "fwmark" | "table" | "saveconfig" | "preup" | "postup" | "predown" | "postdown" => {}
        _ => bail!("[Interface] has no field {name:?}"),
    }
    Ok(())
}

fn peer_value(peer: &mut PartialPeer, name: &str, value: &str) -> Result<()> {
    match name {
        "publickey" => peer.public_key = Some(key(value)?),
        "presharedkey" => peer.preshared_key = Some(SecretKey(key(value)?)),
        "endpoint" => peer.endpoint = Some(value.parse()?),
        "allowedips" => {
            for v in list(value) {
                peer.allowed_ips.push(v.parse()?);
            }
        }
        "persistentkeepalive" => {
            peer.persistent_keepalive = match value {
                "off" | "0" => None,
                v => Some(
                    v.parse::<u16>()
                        .map_err(|_| anyhow!("PersistentKeepalive {v:?} is not a number"))?,
                ),
            }
        }
        "reserved" => peer.reserved = reserved(value)?,
        _ => bail!("[Peer] has no field {name:?}"),
    }
    Ok(())
}

/// A key of 32 bytes in base64.
fn key(value: &str) -> Result<[u8; 32]> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| anyhow!("the key is not 32 bytes in base64"))
}

/// The reserved bytes: `1, 2, 3` (also in brackets) or the base64 of
/// three bytes.
fn reserved(value: &str) -> Result<[u8; 3]> {
    let inner = value.trim_start_matches('[').trim_end_matches(']');
    if inner.contains(',') {
        let bytes = list(inner)
            .map(|v| v.parse::<u8>())
            .collect::<std::result::Result<Vec<u8>, _>>()
            .map_err(|_| anyhow!("Reserved {value:?} has a number that is not 0 to 255"))?;
        return bytes
            .try_into()
            .map_err(|_| anyhow!("Reserved {value:?} is not three numbers"));
    }
    base64::engine::general_purpose::STANDARD
        .decode(inner)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow!("Reserved {value:?} is not three numbers or three bytes in base64"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIVATE: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
    const PUBLIC: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";

    #[test]
    fn wg_quick_file_parses() {
        let text = format!(
            "# A comment\n\
             [Interface]\n\
             PrivateKey = {PRIVATE}\n\
             Address = 10.0.0.2/32, fd00::2\n\
             dns = 1.1.1.1, 2606:4700:4700::1111, corp.example\n\
             MTU = 1280\n\
             ListenPort = 51820\n\
             PostUp = iptables -A x # a hook of wg-quick\n\
             \n\
             [Peer]\n\
             PublicKey = {PUBLIC}\n\
             PresharedKey = {PUBLIC}\n\
             Endpoint = engage.example.com:2408\n\
             AllowedIPs = 0.0.0.0/0, ::/0\n\
             PersistentKeepalive = 25\n\
             Reserved = 1, 2, 3\n\
             [Peer]\n\
             PublicKey = {PUBLIC}\n\
             Endpoint = [2001:db8::1]:51820\n\
             Reserved = AQID\n"
        );
        let conf = parse(&text).unwrap();
        let iface = &conf.interface;
        assert_eq!(iface.private_key.0, key(PRIVATE).unwrap());
        assert_eq!(
            iface.addresses,
            vec![
                "10.0.0.2/32".parse::<Cidr>().unwrap(),
                "fd00::2/128".parse().unwrap()
            ]
        );
        assert_eq!(iface.dns.len(), 2);
        assert_eq!(iface.dns_search, vec!["corp.example".to_string()]);
        assert_eq!(iface.mtu, Some(1280));
        assert_eq!(iface.listen_port, Some(51820));
        assert_eq!(conf.peers.len(), 2);
        let peer = &conf.peers[0];
        assert_eq!(peer.public_key, key(PUBLIC).unwrap());
        assert!(peer.preshared_key.is_some());
        assert_eq!(
            peer.endpoint,
            Some(Endpoint {
                host: "engage.example.com".into(),
                port: 2408
            })
        );
        assert_eq!(peer.allowed_ips.len(), 2);
        assert_eq!(peer.persistent_keepalive, Some(25));
        assert_eq!(peer.reserved, [1, 2, 3]);
        let second = &conf.peers[1];
        assert_eq!(
            second.endpoint.as_ref().unwrap().socket_addr(),
            Some("[2001:db8::1]:51820".parse().unwrap())
        );
        assert_eq!(second.reserved, [1, 2, 3]);
        assert!(conf.single_peer().is_err());
        assert!(!format!("{conf:?}").contains(PRIVATE));
    }

    #[test]
    fn wg_reserved_bytes_are_zero_by_default() {
        let text = format!(
            "[Interface]\nPrivateKey = {PRIVATE}\n[Peer]\nPublicKey = {PUBLIC}\nEndpoint = 127.0.0.1:51820\n"
        );
        let conf = parse(&text).unwrap();
        let peer = conf.single_peer().unwrap();
        assert_eq!(peer.reserved, [0, 0, 0]);
        assert_eq!(peer.persistent_keepalive, None);
        assert_eq!(peer.preshared_key, None);
        assert_eq!(conf.interface.listen_port, None);
    }

    #[test]
    fn wg_quick_file_errors_name_the_line() {
        let base = format!("[Interface]\nPrivateKey = {PRIVATE}\n");
        for (text, word) in [
            (format!("{base}Colour = red\n"), "line 3"),
            (format!("{base}[Peer]\nPublicKey = abc\n"), "line 4"),
            (
                format!("{base}[Peer]\nPublicKey = {PUBLIC}\nReserved = 1, 2\n"),
                "line 5",
            ),
            (
                format!("{base}[Peer]\nPublicKey = {PUBLIC}\nEndpoint = ::1:51820\n"),
                "line 5",
            ),
            (format!("{base}[Wat]\n"), "line 3"),
            (
                format!("{base}[Peer]\nEndpoint = 1.2.3.4:5\n"),
                "no PublicKey",
            ),
            ("[Peer]\n".to_string(), "no [Interface]"),
            (format!("Address = 10.0.0.1\n{base}"), "line 1"),
            ("[Interface]\nAddress = 10.0.0.1/33\n".to_string(), "line 2"),
            (format!("{base}ListenPort = 70000\n"), "line 3"),
        ] {
            let err = format!("{:#}", parse(&text).unwrap_err());
            assert!(err.contains(word), "{text:?}: {err}");
        }
    }

    #[test]
    fn wg_quick_file_reads_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wg0.conf");
        std::fs::write(
            &path,
            format!("[Interface]\nPrivateKey = {PRIVATE}\n[Peer]\nPublicKey = {PUBLIC}\n"),
        )
        .unwrap();
        let conf = read_file(&path).unwrap();
        assert_eq!(conf.peers.len(), 1);
        let err = format!(
            "{:#}",
            read_file(&dir.path().join("none.conf")).unwrap_err()
        );
        assert!(err.contains("none.conf"), "{err}");
    }
}
