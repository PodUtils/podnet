//! A TUIC server as its share link names it:
//! `tuic://UUID:PASSWORD@HOST:PORT?sni=&alpn=h3&congestion_control=bbr#NAME`.
//! The UUID and the password are the credential: the link is read from a
//! private file, and nothing that podnet shows holds either.
//!
//! The credential types and the shape of the parse are
//! `podssh-ws/src/tuic/link.rs`, moved here so that the org reads one link
//! grammar. Two deliberate differences: podnet accepts `allow_insecure=1`,
//! because its tests and its `--tuic` gate run a self-signed server on the
//! loopback, and a parameter that is not known is refused, so that a typo is
//! not a silent default.

use std::fmt;

/// A TUIC user's UUID: its 16 bytes are `Authenticate`'s first field and the
/// label of the token's exporter.
#[derive(Clone, PartialEq, Eq)]
pub struct Uuid(pub [u8; 16]);

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Uuid(<redacted>)")
    }
}

impl Uuid {
    /// The 36 characters with hyphens, or the 32 hex digits alone, in either
    /// case, as servers read it.
    pub fn parse(text: &str) -> Result<Uuid, String> {
        let hex: Vec<u8> = text.bytes().filter(|b| *b != b'-').collect();
        let hyphens = text.len() - hex.len();
        let shaped = (hyphens == 0 && text.len() == 32)
            || (hyphens == 4
                && text.len() == 36
                && [8, 13, 18, 23].iter().all(|&i| text.as_bytes()[i] == b'-'));
        if !shaped || hex.len() != 32 {
            return Err("the user is not a UUID".into());
        }
        let mut out = [0u8; 16];
        for (i, pair) in hex.chunks(2).enumerate() {
            let digit = |b: u8| (b as char).to_digit(16).ok_or("the user is not a UUID");
            out[i] = (digit(pair[0])? * 16 + digit(pair[1])?) as u8;
        }
        Ok(Uuid(out))
    }
}

/// A TUIC user's password, the context of the token's exporter: the raw
/// bytes, which go to no line and no log, and are wiped when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(Vec<u8>);

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

impl Password {
    pub fn new(password: &str) -> Password {
        Password(password.as_bytes().to_vec())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        // podnet has no `zeroize` dependency; a volatile write keeps the
        // compiler from dropping the wipe as a dead store.
        for byte in &mut self.0 {
            // SAFETY: `byte` is a valid, exclusive reference to one byte.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

/// The ALPN that TUIC's clients offer when a link names none: `h3`, which
/// sing-box, mihomo and Itsusinn's tuic serve by default.
pub const ALPN: &str = "h3";

/// A link of the road: where QUIC goes, what TLS checks, what it offers.
#[derive(Clone)]
pub struct Link {
    pub uuid: Uuid,
    pub password: Password,
    pub host: String,
    pub port: u16,
    /// The name that TLS asks for and checks: `sni`, else `peer`, else HOST.
    pub sni: String,
    /// The protocols that TLS offers by ALPN, in order.
    pub alpn: Vec<Vec<u8>>,
    /// A self-signed server, as the loopback tests and the `--tuic` gate use.
    pub allow_insecure: bool,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("uuid", &"<redacted>")
            .field("password", &"<redacted>")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("sni", &self.sni)
            .finish()
    }
}

impl Link {
    /// Parse a link. The password may hold a percent escape, and so may a
    /// query value; a parameter that is not known is refused, so that a
    /// typo is not a silent default.
    pub fn parse(text: &str) -> anyhow::Result<Link> {
        parse_link(text).map_err(anyhow::Error::msg)
    }
}

/// The parse, in its own words: a caller that shows a line takes them as
/// they are.
fn parse_link(text: &str) -> Result<Link, String> {
    let link = text.trim();
    let rest = match link.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("tuic://") => &link[7..],
        _ => return Err("a TUIC link starts with tuic://".into()),
    };
    let rest = rest.split_once('#').map_or(rest, |(rest, _)| rest);
    let (authority, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (user, hostport) = authority
        .rsplit_once('@')
        .ok_or("a TUIC link has UUID:PASSWORD@HOST:PORT")?;
    let (uuid, password) = user
        .split_once(':')
        .ok_or("a TUIC link has UUID:PASSWORD@HOST:PORT")?;
    let uuid = percent(uuid);
    let uuid = Uuid::parse(&uuid)?;
    let password = percent(password);
    if password.is_empty() {
        return Err("the password is empty".into());
    }
    let (host, port) = split_host_port(hostport)?;
    if host.is_empty() {
        return Err("the link names no host".into());
    }
    let params = Params::parse(query)?;
    let mut link = Link {
        uuid,
        password: Password::new(&password),
        host,
        port,
        sni: String::new(),
        alpn: vec![ALPN.as_bytes().to_vec()],
        allow_insecure: false,
    };
    if let Some(sni) = params
        .get("sni")
        .or_else(|| params.get("peer"))
        .filter(|s| !s.is_empty())
    {
        link.sni = sni.to_string();
    }
    if let Some(list) = params.get("alpn") {
        let alpn: Vec<Vec<u8>> = list
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(|a| a.as_bytes().to_vec())
            .collect();
        if !alpn.is_empty() {
            link.alpn = alpn;
        }
    }
    if let Some(value) = params.get("allow_insecure") {
        link.allow_insecure = value == "1" || value.eq_ignore_ascii_case("true");
    }
    params.check()?;
    if link.sni.is_empty() {
        link.sni = link.host.clone();
    }
    if link.sni.is_empty() {
        return Err("the TUIC link needs an SNI, or a host that is a name".into());
    }
    Ok(link)
}

/// The link's query, each value percent-decoded.
struct Params(Vec<(String, String)>);

impl Params {
    fn parse(query: &str) -> Result<Params, String> {
        let mut out = Vec::new();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            out.push((key.to_ascii_lowercase(), percent(value)));
        }
        Ok(Params(out))
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// A setting whose value this road does not speak is refused by its
    /// name. `congestion_control` and `udp_relay_mode` name the sender's
    /// choices: the first is the sender's own, and the second is fixed to
    /// the `quic` mode that this road carries.
    fn check(&self) -> Result<(), String> {
        match self.get("version") {
            None | Some("5") | Some("") => {}
            Some(other) => {
                return Err(format!(
                    "version={other} is not spoken: this road speaks TUIC version 5"
                ))
            }
        }
        if let Some(cc) = self.get("congestion_control") {
            let cc = cc.to_ascii_lowercase();
            if !matches!(cc.as_str(), "" | "cubic" | "bbr" | "new_reno" | "newreno") {
                return Err(format!(
                    "congestion_control={cc} is no controller: cubic, new_reno or bbr"
                ));
            }
        }
        for (key, _) in &self.0 {
            match key.as_str() {
                "sni" | "peer" | "alpn" | "allow_insecure" | "congestion_control" | "version"
                | "udp_relay_mode" | "disable_sni" | "reduce_rtt" => {}
                other => return Err(format!("the TUIC link has no parameter {other:?}")),
            }
        }
        Ok(())
    }
}

/// The host and the port of an authority: the port after the last colon,
/// and an IPv6 literal in brackets.
fn split_host_port(text: &str) -> Result<(String, u16), String> {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, tail) = rest.split_once(']').ok_or("an IPv6 link without ]")?;
        let port = tail
            .strip_prefix(':')
            .ok_or("a TUIC link has UUID:PASSWORD@HOST:PORT")?;
        return Ok((
            host.to_string(),
            port.parse().map_err(|_| "the port of the link")?,
        ));
    }
    let (host, port) = text
        .rsplit_once(':')
        .ok_or("a TUIC link has UUID:PASSWORD@HOST:PORT")?;
    Ok((
        host.to_string(),
        port.parse().map_err(|_| "the port of the link")?,
    ))
}

/// The bytes of a percent escape of the link (`%2F`), else the bytes.
fn percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The link of the specification: its UUID, host, SNI and ALPN.
    #[test]
    fn the_link_has_the_form_of_the_specification() {
        let link = Link::parse(
            "tuic://123e4567-e89b-12d3-a456-426614174000:pa%20ss@example.test:443?sni=edge.test",
        )
        .unwrap();
        assert_eq!(link.password.bytes(), b"pa ss");
        assert_eq!(link.host, "example.test");
        assert_eq!(link.port, 443);
        assert_eq!(link.sni, "edge.test");
        assert_eq!(link.alpn, vec![b"h3".to_vec()]);
        let v6 = Link::parse("tuic://123e4567-e89b-12d3-a456-426614174000:x@[2001:db8::1]:8443")
            .unwrap();
        assert_eq!(v6.host, "2001:db8::1");
        assert_eq!(v6.port, 8443);
        assert!(Link::parse("tuic://h:1").is_err());
        assert!(Link::parse("tuic://123e4567-e89b-12d3-a456-426614174000:x@h:1?nope=1").is_err());
    }

    /// The credential never reaches a line, and the UUID takes the two forms
    /// that servers write.
    #[test]
    fn the_credential_is_redacted_and_the_uuid_has_two_forms() {
        let uuid = Uuid::parse("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let plain = Uuid::parse("123e4567e89b12d3a456426614174000").unwrap();
        assert_eq!(uuid, plain);
        assert!(Uuid::parse("123e4567-e89b-12d3-a456-42661417400").is_err());
        assert!(Uuid::parse("123e4567-e89b-12d3-a456-4266141740000").is_err());
        let link =
            Link::parse("tuic://123e4567-e89b-12d3-a456-426614174000:secret@h.test:443").unwrap();
        let shown = format!("{link:?}");
        assert!(!shown.contains("secret"));
        assert!(!shown.contains("123e4567"));
    }

    /// A version other than 5, an unknown parameter and an empty password
    /// are refused by name.
    #[test]
    fn a_setting_the_road_does_not_speak_is_refused() {
        let base = "tuic://123e4567-e89b-12d3-a456-426614174000:x@h.test:443";
        assert!(Link::parse(&format!("{base}?version=4"))
            .unwrap_err()
            .to_string()
            .contains("version=4"));
        assert!(Link::parse(&format!("{base}?nope=1"))
            .unwrap_err()
            .to_string()
            .contains("nope"));
        assert!(
            Link::parse("tuic://123e4567-e89b-12d3-a456-426614174000:@h.test:443")
                .unwrap_err()
                .to_string()
                .contains("password is empty")
        );
        let insecure = Link::parse(&format!("{base}?allow_insecure=1")).unwrap();
        assert!(insecure.allow_insecure);
    }
}
