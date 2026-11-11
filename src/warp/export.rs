//! `podnet warp export` (issue 148): a WireGuard device of the WARP service,
//! as a config that other clients read (wg-quick, or JSON).
//!
//! The exported device is a new device with its own X25519 key. It is not
//! the MASQUE device of `warp.json`, which this module never reads or
//! writes. Its data plane is WireGuard on UDP (port 2408 of the front door
//! by default), not MASQUE on TCP 443: on a network that permits only TCP
//! 443, the exported config does not work, and podnet itself does.
//!
//! The reserved bytes (T-074): WARP reads bytes 1 to 3 of each WireGuard
//! message, the `client_id` of the device. Kernel WireGuard and wg-quick send
//! zeros there; `podnet warp --transport wireguard` sends the bytes. A device
//! that needs them has fresh handshakes and no data path in kernel WireGuard.
//! `warp export --check` ([`check`]) tells which.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};

use super::register::{self, Device};
use crate::egress::Proxy;

pub mod check;
pub mod command;

/// The file name of the exported device in the state directory.
pub const FILE_NAME: &str = "warp-wg.json";
/// The UDP port of WireGuard on the front door.
pub const WG_PORT: u16 = 2408;

/// The exported device and its private key.
#[derive(Serialize, Deserialize)]
pub struct Exported {
    pub device: Device,
    /// The X25519 private key (base64).
    pub private_key: String,
}

/// An X25519 key pair: (private, public), each in base64. The private key
/// is clamped (RFC 7748), as WireGuard keeps it.
pub fn keypair() -> Result<(String, String)> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| anyhow::anyhow!("no OS entropy: {e}"))?;
    Ok(keypair_of(secret))
}

fn keypair_of(mut secret: [u8; 32]) -> (String, String) {
    secret[0] &= 248;
    secret[31] &= 127;
    secret[31] |= 64;
    let public = curve25519_dalek::MontgomeryPoint::mul_base_clamped(secret);
    (b64(&secret), b64(public.as_bytes()))
}

fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// `<state dir>/warp-wg.json`; `None` with `PODNET_STATE_DIR=none`.
pub fn default_path() -> Result<Option<PathBuf>> {
    Ok(crate::state::state_dir()?.map(|dir| dir.join(FILE_NAME)))
}

/// Read an exported device.
pub fn load(path: &Path) -> Result<Exported> {
    super::check_private(path)?;
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// The exported device of `path`; a new one when there is none or `new`.
pub async fn load_or_register(path: &Path, new: bool, proxy: Option<&Proxy>) -> Result<Exported> {
    // T-241: one registration at a time, and the file is writable first.
    let _held = super::enrol::lock(path, super::enrol::LOCK_WAIT).await?;
    if crate::state::exists(path)? && !new {
        return load(path);
    }
    crate::state::check_file_writable(path)?;
    let (private_key, public_key) = keypair()?;
    let device = register::create_device_with_key(&public_key, proxy)
        .await
        .context("registering the WireGuard device")?;
    if device.config.peers.is_empty() {
        bail!("the WARP API gave no peer for the WireGuard device");
    }
    let exported = Exported {
        device,
        private_key,
    };
    let text = serde_json::to_string_pretty(&exported)?;
    super::write_private(path, text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(exported)
}

/// Remove the exported device from the service, and its file.
pub async fn delete(path: &Path, proxy: Option<&Proxy>) -> Result<String> {
    // Issue 178: a second --delete names the missing file (before: an
    // error of the WARP identity, exit 69).
    if !crate::state::exists(path)? {
        return Err(crate::exit::fault(
            crate::exit::Fault::Config,
            anyhow::anyhow!("no exported device at {}", path.display()),
        ));
    }
    let exported = load(path)?;
    register::delete_device(&exported.device.id, &exported.device.token, proxy).await?;
    std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    Ok(exported.device.id)
}

/// Issue 177: the output of `warp export` holds the private key. With no
/// `--out`, a terminal is refused (a usage error), so that the key is not
/// on the screen. `--out -` writes to stdout on purpose.
pub fn check_sink(out: Option<&Path>, stdout_is_terminal: bool) -> Result<()> {
    if out.is_none() && stdout_is_terminal {
        bail!(
            "stdout is a terminal, and the output holds the private key; give --out FILE, redirect stdout, or give --out - to print it"
        );
    }
    Ok(())
}

/// Issue 177: when stdout is a regular file of this user that others can
/// read (`podnet warp export > warp.conf` with a umask of 022), set its mode
/// to 0600 before the key is written. True when the mode changed.
pub fn tighten_stdout() -> bool {
    use std::os::fd::AsRawFd as _;
    let fd = std::io::stdout().as_raw_fd();
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return false;
    }
    tighten_fd(fd, &st)
}

fn tighten_fd(fd: i32, st: &libc::stat) -> bool {
    let regular = st.st_mode & libc::S_IFMT == libc::S_IFREG;
    let mine = st.st_uid == unsafe { libc::geteuid() };
    if regular && mine && st.st_mode & 0o077 != 0 {
        return unsafe { libc::fchmod(fd, 0o600) } == 0;
    }
    false
}

/// The head of the config under `Reserved` (T-074).
pub const RESERVED_NOTE: &str = "\
# The reserved bytes go in bytes 1 to 3 of each WireGuard message (the client_id of the device).
# Kernel WireGuard and wg-quick send zeros there. podnet warp --transport wireguard sends them.
# A device that needs them has handshakes and no data in kernel WireGuard: podnet warp export --check tells.
";

/// The values of a config.
#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub private_key: String,
    pub peer_public_key: String,
    pub address_v4: Option<String>,
    pub address_v6: Option<String>,
    pub endpoint: String,
    pub reserved: Option<[u8; 3]>,
    pub keepalive: u16,
}

impl Config {
    /// The config of an exported device. `endpoint` replaces the endpoint
    /// of the record.
    pub fn of(exported: &Exported, endpoint: Option<&str>, keepalive: u16) -> Result<Self> {
        let device = &exported.device;
        let peer = device
            .config
            .peers
            .first()
            .context("the device record has no peer")?;
        if peer.public_key.is_empty() {
            bail!("the device record has no peer key");
        }
        let endpoint = match endpoint {
            Some(e) => e.to_string(),
            None => wireguard_endpoint(&peer.endpoint)?,
        };
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        Ok(Config {
            private_key: exported.private_key.clone(),
            peer_public_key: peer.public_key.clone(),
            address_v4: some(&device.config.interface.addresses.v4),
            address_v6: some(&device.config.interface.addresses.v6),
            endpoint,
            reserved: device.reserved(),
            keepalive,
        })
    }

    /// The wg-quick text.
    pub fn wg_quick(&self) -> String {
        let mut addresses = Vec::new();
        if let Some(v4) = &self.address_v4 {
            addresses.push(format!("{v4}/32"));
        }
        if let Some(v6) = &self.address_v6 {
            addresses.push(format!("{v6}/128"));
        }
        let mut out = String::from(
            "# podnet warp export: a WireGuard device of WARP (UDP). podnet warp export --delete removes it.\n",
        );
        if let Some([a, b, c]) = self.reserved {
            out.push_str(&format!(
                "# Reserved = {a},{b},{c} (for the clients that send it)\n{RESERVED_NOTE}"
            ));
        }
        out.push_str(&format!(
            "[Interface]\nPrivateKey = {}\nAddress = {}\nDNS = 1.1.1.1, 1.0.0.1, 2606:4700:4700::1111, 2606:4700:4700::1001\nMTU = 1280\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = {}\n",
            self.private_key,
            addresses.join(", "),
            self.peer_public_key,
            self.endpoint
        ));
        if self.keepalive > 0 {
            out.push_str(&format!("PersistentKeepalive = {}\n", self.keepalive));
        }
        out
    }

    /// The JSON document.
    pub fn json(&self, device_id: &str) -> serde_json::Value {
        serde_json::json!({
            "device_id": device_id,
            "private_key": self.private_key,
            "peer_public_key": self.peer_public_key,
            "address_v4": self.address_v4,
            "address_v6": self.address_v6,
            "endpoint": self.endpoint,
            "reserved": self.reserved.map(|r| r.to_vec()),
            "mtu": 1280,
            "keepalive": self.keepalive,
        })
    }
}

/// The WireGuard endpoints of a device record, in order (T-213): `host` (with
/// port 2408 when it has none), the IPv4 address and the IPv6 address on port
/// 2408. `ports` of the record is not read.
fn wireguard_candidates(endpoint: &register::Endpoint) -> Vec<String> {
    let mut out = Vec::new();
    let host = endpoint.host.trim();
    if !host.is_empty() {
        let has_port = host
            .rsplit_once(':')
            .is_some_and(|(h, p)| !h.is_empty() && !h.ends_with(':') && p.parse::<u16>().is_ok());
        out.push(if has_port {
            host.to_string()
        } else {
            format!("{host}:{WG_PORT}")
        });
    }
    let v4 = endpoint.v4.trim();
    let ip = v4.rsplit_once(':').map_or(v4, |(ip, _)| ip);
    if ip.parse::<std::net::Ipv4Addr>().is_ok() {
        out.push(format!("{ip}:{WG_PORT}"));
    }
    let v6 = endpoint.v6.trim().trim_start_matches('[');
    let v6 = v6.split(']').next().unwrap_or(v6);
    if v6.parse::<std::net::Ipv6Addr>().is_ok() {
        out.push(format!("[{v6}]:{WG_PORT}"));
    }
    out
}

/// The failure of a record with no usable endpoint: it names the fields that
/// the record has.
fn no_endpoint(endpoint: &register::Endpoint) -> anyhow::Error {
    let present: Vec<&str> = [
        ("host", &endpoint.host),
        ("v4", &endpoint.v4),
        ("v6", &endpoint.v6),
    ]
    .into_iter()
    .filter(|(_, v)| !v.trim().is_empty())
    .map(|(k, _)| k)
    .collect();
    let present = if present.is_empty() {
        "none".to_string()
    } else {
        present.join(", ")
    };
    anyhow::anyhow!(
        "the device record has no usable WireGuard endpoint (fields present: {present}); give --endpoint HOST:PORT"
    )
}

/// The WireGuard endpoint of the record as text, for `warp export`: the first
/// of `host`, `v4` and `v6`.
pub fn wireguard_endpoint(endpoint: &register::Endpoint) -> Result<String> {
    wireguard_candidates(endpoint)
        .into_iter()
        .next()
        .ok_or_else(|| no_endpoint(endpoint))
}

/// The WireGuard endpoint of the record as an address, for the WireGuard leg:
/// the first literal address of the list of [`wireguard_endpoint`]; else the
/// first name of it that the system resolver gives.
pub fn wireguard_socket_addr(endpoint: &register::Endpoint) -> Result<std::net::SocketAddr> {
    use std::net::ToSocketAddrs as _;
    let list = wireguard_candidates(endpoint);
    if let Some(addr) = list.iter().find_map(|c| c.parse().ok()) {
        return Ok(addr);
    }
    list.iter()
        .find_map(|c| c.to_socket_addrs().ok()?.next())
        .ok_or_else(|| no_endpoint(endpoint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_refuses_a_terminal_without_out() {
        assert!(check_sink(None, true).is_err());
        assert!(check_sink(None, false).is_ok());
        assert!(check_sink(Some(Path::new("-")), true).is_ok());
        assert!(check_sink(Some(Path::new("w.conf")), true).is_ok());
    }

    #[test]
    fn a_redirected_file_is_tightened_to_0600() {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("podnet-tighten-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("warp.conf");
        let file = std::fs::File::create(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::fstat(file.as_raw_fd(), &mut st) }, 0);
        assert!(tighten_fd(file.as_raw_fd(), &st));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // Mode 0600 already: no change.
        assert_eq!(unsafe { libc::fstat(file.as_raw_fd(), &mut st) }, 0);
        assert!(!tighten_fd(file.as_raw_fd(), &st));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_x25519_key_pair() {
        // RFC 7748, section 6.1: the key of Alice.
        let secret: [u8; 32] =
            hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let (private, public) = keypair_of(secret);
        use base64::Engine;
        let public = base64::engine::general_purpose::STANDARD
            .decode(public)
            .unwrap();
        assert_eq!(
            public,
            hex::<32>("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        let private = base64::engine::general_purpose::STANDARD
            .decode(private)
            .unwrap();
        assert_eq!(private[0] & 7, 0);
        assert_eq!(private[31] & 0xc0, 0x40);
        let (a, b) = keypair().unwrap();
        assert_eq!((a.len(), b.len()), (44, 44));
    }

    fn hex<const N: usize>(text: &str) -> [u8; N] {
        let mut out = [0u8; N];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    fn exported(host: &str, v4: &str) -> Exported {
        exported_v6(host, v4, "")
    }

    fn exported_v6(host: &str, v4: &str, v6: &str) -> Exported {
        let device: Device = serde_json::from_value(serde_json::json!({
            "id": "dev",
            "account": {"id": "acc"},
            "token": "t",
            "config": {
                "client_id": "AQID",
                "peers": [{"public_key": "PEER=", "endpoint": {"host": host, "v4": v4, "v6": v6}}],
                "interface": {"addresses": {"v4": "172.16.0.2", "v6": "2606:4700:110::2"}},
            },
        }))
        .unwrap();
        Exported {
            device,
            private_key: "PRIV=".into(),
        }
    }

    #[test]
    fn the_wg_quick_config() {
        let config =
            Config::of(&exported("engage.cloudflareclient.com:2408", ""), None, 25).unwrap();
        assert_eq!(config.endpoint, "engage.cloudflareclient.com:2408");
        assert_eq!(config.reserved, Some([1, 2, 3]));
        let text = config.wg_quick();
        assert!(text.contains("PrivateKey = PRIV=\n"));
        assert!(text.contains("Address = 172.16.0.2/32, 2606:4700:110::2/128\n"));
        assert!(text.contains("PublicKey = PEER=\n"));
        assert!(text.contains("Endpoint = engage.cloudflareclient.com:2408\n"));
        assert!(text.contains("PersistentKeepalive = 25\n"));
        assert!(text.contains("# Reserved = 1,2,3"));
        assert!(text.contains("# Kernel WireGuard and wg-quick send zeros there."));
        let json = config.json("dev");
        assert_eq!(json["reserved"], serde_json::json!([1, 2, 3]));
        assert!(json.get("token").is_none());
        // No keepalive line with 0; the endpoint of the IPv4 address.
        let config = Config::of(&exported("", "162.159.192.1:0"), None, 0).unwrap();
        assert_eq!(config.endpoint, "162.159.192.1:2408");
        assert!(!config.wg_quick().contains("PersistentKeepalive"));
        let config = Config::of(
            &exported("engage.example", ""),
            Some("[2606:4700::1]:500"),
            25,
        )
        .unwrap();
        assert_eq!(config.endpoint, "[2606:4700::1]:500");
        assert_eq!(
            Config::of(&exported("engage.example", ""), None, 25)
                .unwrap()
                .endpoint,
            "engage.example:2408"
        );
        assert!(Config::of(&exported("", ""), None, 25).is_err());
    }

    // T-213: the export and the WireGuard leg read the same fields of the
    // record, in the same order.
    #[test]
    fn the_two_wireguard_readers_agree_on_the_record_shapes() {
        // The leg decodes the keys: give it 32 bytes (base64 of zeros).
        let key = "A".repeat(43) + "=";
        let keyed = |mut e: Exported| {
            e.private_key = key.clone();
            e.device.config.peers[0].public_key = key.clone();
            e
        };
        let (exported, exported_v6) = (
            |h: &str, v4: &str| keyed(exported(h, v4)),
            |h: &str, v4: &str, v6: &str| keyed(exported_v6(h, v4, v6)),
        );
        let wg = |e: &Exported| {
            crate::warp::wg::Peer::from_exported(e, None).map(|p| p.endpoint.to_string())
        };
        let text = |e: &Exported| Config::of(e, None, 25).map(|c| c.endpoint);
        for (record, want) in [
            (exported("162.159.192.1", ""), "162.159.192.1:2408"),
            (exported("162.159.192.1:500", ""), "162.159.192.1:500"),
            (exported("", "162.159.192.1:0"), "162.159.192.1:2408"),
            (
                exported_v6("", "", "2606:4700:103::2"),
                "[2606:4700:103::2]:2408",
            ),
            (
                exported_v6("", "", "[2606:4700:103::2]:443"),
                "[2606:4700:103::2]:2408",
            ),
        ] {
            assert_eq!(text(&record).unwrap(), want);
            assert_eq!(wg(&record).unwrap(), want);
        }
        // A name and an address: the export keeps the name; the leg takes the
        // address (no lookup).
        let both = exported("engage.example", "162.159.192.1");
        assert_eq!(text(&both).unwrap(), "engage.example:2408");
        assert_eq!(wg(&both).unwrap(), "162.159.192.1:2408");
        // No field: both fail, and the message names the fields present.
        for err in [text(&exported("", "")), wg(&exported("", "bad"))] {
            let err = format!("{:#}", err.unwrap_err());
            assert!(err.contains("fields present"), "{err}");
        }
        let err = format!("{:#}", wg(&exported("", "bad")).unwrap_err());
        assert!(err.contains("v4"), "{err}");
    }
}
