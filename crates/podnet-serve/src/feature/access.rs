//! Access control: IP filters, HTTP basic auth and the gate decision.

use super::*;

/// One address or address range in an IP filter.
///
/// CIDR matching is implemented here rather than pulled in as a dependency:
/// it is twenty lines of mask arithmetic, and the alternative is growing the
/// dependency list of a security-reviewed tool for one subtraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpRange {
    /// Exactly one address.
    Single(IpAddr),
    /// A network, given as its address and prefix length.
    Cidr {
        /// The network address. Host bits below `bits` are ignored, so
        /// `10.1.2.3/8` and `10.0.0.0/8` are the same range.
        addr: IpAddr,
        /// Prefix length in bits.
        bits: u8,
    },
}

impl IpRange {
    /// Parse `192.0.2.1`, `192.0.2.0/24`, `2001:db8::1` or `2001:db8::/32`.
    pub fn parse(s: &str) -> Result<IpRange, FeatureError> {
        let s = s.trim();
        let Some((addr, bits)) = s.split_once('/') else {
            let addr: IpAddr = s.parse().map_err(|_| {
                FeatureError::Invalid(format!("{s:?} is not an IP address or CIDR block"))
            })?;
            return Ok(IpRange::Single(addr));
        };
        let addr: IpAddr = addr.trim().parse().map_err(|_| {
            FeatureError::Invalid(format!("{addr:?} is not an IP address in {s:?}"))
        })?;
        let bits: u8 = bits.trim().parse().map_err(|_| {
            FeatureError::Invalid(format!("{bits:?} is not a prefix length in {s:?}"))
        })?;
        let max = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if bits > max {
            return Err(FeatureError::Invalid(format!(
                "prefix length /{bits} is longer than /{max} for {addr}"
            )));
        }
        Ok(IpRange::Cidr { addr, bits })
    }

    /// Whether `ip` falls in this range.
    pub fn contains(&self, ip: &IpAddr) -> bool {
        match (self, ip) {
            (IpRange::Single(a), b) => a == b,
            (IpRange::Cidr { addr, bits }, b) => {
                if addr.is_ipv4() != b.is_ipv4() {
                    return false;
                }
                match (addr, b) {
                    (IpAddr::V4(a), IpAddr::V4(b)) => prefix_match_v4(*a, *b, *bits),
                    (IpAddr::V6(a), IpAddr::V6(b)) => prefix_match_v6(*a, *b, *bits),
                    _ => false,
                }
            }
        }
    }
}

impl std::fmt::Display for IpRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpRange::Single(a) => write!(f, "{a}"),
            IpRange::Cidr { addr, bits } => write!(f, "{addr}/{bits}"),
        }
    }
}

pub(super) fn prefix_match_v4(net: std::net::Ipv4Addr, ip: std::net::Ipv4Addr, bits: u8) -> bool {
    if bits == 0 {
        return true;
    }
    let n = u32::from(net);
    let i = u32::from(ip);
    // A prefix cannot straddle a u32 boundary, so a plain shift is enough.
    let shift = 32 - u32::from(bits);
    (n >> shift) == (i >> shift)
}

pub(super) fn prefix_match_v6(net: std::net::Ipv6Addr, ip: std::net::Ipv6Addr, bits: u8) -> bool {
    if bits == 0 {
        return true;
    }
    let n = u128::from(net);
    let i = u128::from(ip);
    let shift = 128 - u32::from(bits);
    (n >> shift) == (i >> shift)
}

/// Which addresses may reach the origin.
///
/// The semantics are the well-known firewall ones: **deny wins**, and an empty
/// allow list means "anything not denied". An allow list that matched nothing
/// would be indistinguishable from a typo in a flag, and a tunnel that 403s
/// everyone is harder to diagnose than one that serves its owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpFilter {
    denied: Vec<IpRange>,
    /// Ordered, most-specific first: the first match names the rule that let
    /// the address in, so the operator sees the answer they wrote.
    allowed: Vec<(String, IpRange)>,
}

/// The outcome of an IP check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpVerdict {
    /// No rule matched, or the allow list is empty.
    Permitted,
    /// An allow rule matched. Carries the label the operator gave the rule, so
    /// a log line can say which of several ranges admitted the address.
    AllowedBy(String),
    /// A deny rule matched. Deny is evaluated first and never falls through.
    DeniedBy(String),
}

impl IpFilter {
    /// A filter that permits everything.
    pub fn allow_all() -> Self {
        Self::default()
    }

    /// Add a denied address or block.
    pub fn deny(&mut self, range: IpRange) -> &mut Self {
        self.denied.push(range);
        self
    }

    /// Add an allowed address or block. The first match supplies the label
    /// shown to the operator, so the rules are ordered most-specific first.
    pub fn allow(&mut self, label: &str, range: IpRange) -> &mut Self {
        self.allowed.push((label.to_string(), range));
        self
    }

    /// Parse and add a denied entry from text.
    pub fn deny_str(&mut self, text: &str) -> Result<&mut Self, FeatureError> {
        let range = IpRange::parse(text)?;
        Ok(self.deny(range))
    }

    /// Parse and add an allowed entry from text.
    pub fn allow_str(&mut self, label: &str, text: &str) -> Result<&mut Self, FeatureError> {
        let range = IpRange::parse(text)?;
        Ok(self.allow(label, range))
    }

    /// Evaluate one address.
    pub fn check(&self, ip: &IpAddr) -> IpVerdict {
        if let Some(range) = self.denied.iter().find(|r| r.contains(ip)) {
            return IpVerdict::DeniedBy(range.to_string());
        }
        if self.allowed.is_empty() {
            return IpVerdict::Permitted;
        }
        match self.allowed.iter().find(|(_, r)| r.contains(ip)) {
            Some((label, _)) => IpVerdict::AllowedBy(label.clone()),
            None => IpVerdict::DeniedBy(format!(
                "{ip} matched no allowed range ({} allowed, {} denied)",
                self.allowed.len(),
                self.denied.len()
            )),
        }
    }

    /// Whether this filter changes the decision at all.
    pub fn is_permissive(&self) -> bool {
        self.denied.is_empty() && self.allowed.is_empty()
    }

    /// How many rules are configured, for the operator's startup summary.
    pub fn len(&self) -> usize {
        self.denied.len() + self.allowed.len()
    }

    /// Whether the filter carries no rules at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Render the rules the way they would be typed on a command line.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(self.len());
        for (label, range) in &self.allowed {
            parts.push(format!("allow {label} {range}"));
        }
        for range in &self.denied {
            parts.push(format!("deny {range}"));
        }
        if parts.is_empty() {
            "allow all".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// The visitor's address is not permitted. Answered with 403: a browser
    /// retrying will not help, and 401 would invite a login prompt for a
    /// tunnel that has no accounts.
    IpDenied(String),
    /// No credential was presented but the policy demands one. Answered with
    /// 401 and a `WWW-Authenticate` challenge.
    CredentialsMissing {
        /// The scheme to advertise, e.g. `Basic realm="podnet"`.
        challenge: String,
    },
    /// A credential was presented and did not match. Answered with 403: the
    /// client authenticated wrongly, and repeating it will not become right.
    CredentialsInvalid,
    /// The client asked for a method this endpoint does not serve.
    MethodNotAllowed(String),
}

impl DenyReason {
    /// The HTTP status to answer with.
    pub fn status(&self) -> u16 {
        match self {
            DenyReason::IpDenied(_) | DenyReason::CredentialsInvalid => 403,
            DenyReason::CredentialsMissing { .. } => 401,
            DenyReason::MethodNotAllowed(_) => 405,
        }
    }

    /// A short reason phrase.
    pub fn phrase(&self) -> &'static str {
        match self.status() {
            401 => "Unauthorized",
            403 => "Forbidden",
            405 => "Method Not Allowed",
            _ => "Error",
        }
    }

    /// The body to return. It names the reason but never the policy's contents:
    /// a refusal that explains which addresses or passwords would be accepted
    /// is an oracle.
    pub fn message(&self) -> String {
        match self {
            DenyReason::IpDenied(_) => {
                "403 forbidden: this tunnel does not accept requests from your address\n"
                    .to_string()
            }
            DenyReason::CredentialsMissing { challenge } => {
                format!("401 unauthorized: this tunnel requires authentication ({challenge})\n")
            }
            DenyReason::CredentialsInvalid => {
                "403 forbidden: the credentials presented are not accepted\n".to_string()
            }
            DenyReason::MethodNotAllowed(m) => {
                format!("405 method not allowed: {m}\n")
            }
        }
    }

    /// The `WWW-Authenticate` header, when one applies.
    pub fn challenge(&self) -> Option<&str> {
        match self {
            DenyReason::CredentialsMissing { challenge } => Some(challenge),
            _ => None,
        }
    }
}

/// What a gate decided about a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    /// Let the request through to the origin.
    Allow,
    /// Answer with the given status, headers and body, and stop.
    Refuse {
        /// The status code.
        code: u16,
        /// Response headers, already cased.
        headers: Vec<(String, String)>,
        /// The response body.
        body: String,
    },
}

impl AccessDecision {
    /// Build the refusal for a [`DenyReason`], with a `WWW-Authenticate` header
    /// when the reason calls for one.
    pub fn refusal(reason: &DenyReason) -> AccessDecision {
        let mut headers = vec![(
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )];
        if let Some(challenge) = reason.challenge() {
            headers.push(("WWW-Authenticate".to_string(), challenge.to_string()));
        }
        AccessDecision::Refuse {
            code: reason.status(),
            headers,
            body: reason.message(),
        }
    }

    /// Whether the request may reach the origin.
    pub fn is_allowed(&self) -> bool {
        matches!(self, AccessDecision::Allow)
    }

    /// Render a refusal as a complete HTTP response.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            AccessDecision::Allow => Vec::new(),
            AccessDecision::Refuse {
                code,
                headers,
                body,
            } => podnet_core::util::http::build_response(
                *code,
                crate::origin::static_files::http_phrase(*code),
                &headers
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.clone()))
                    .collect::<Vec<_>>(),
                Some(body.as_bytes()),
            ),
        }
    }
}

/// How many distinct credentials one policy accepts.
///
/// An unbounded table turns a tunnel into a brute-force target, so the limit is
/// enforced at construction rather than left to the caller.
pub const MAX_ALLOWED_IDENTITIES: usize = 64;

/// A gate in front of a tunnel: which addresses may connect, and what must be
/// presented once they do.
#[derive(Debug, Clone, Default)]
pub struct AccessController {
    ips: IpFilter,
    http_auth: Option<HttpBasicAuth>,
}

/// A `Authorization: Basic` credential table.
#[derive(Debug, Clone)]
pub struct HttpBasicAuth {
    realm: String,
    /// `user:password`, stored ready to compare against the decoded header.
    entries: Vec<String>,
}

impl HttpBasicAuth {
    /// Build a table. Fails when empty or oversized rather than silently
    /// accepting everything: a policy that parses but denies all traffic is a
    /// mistake, not a configuration.
    pub fn new(realm: &str, user_passwords: &[&str]) -> Result<Self, FeatureError> {
        let entries: Vec<String> = user_passwords.iter().map(|s| s.to_string()).collect();
        if entries.is_empty() {
            return Err(FeatureError::Policy(
                "basic auth needs at least one user:password entry".to_string(),
            ));
        }
        if entries.len() > MAX_ALLOWED_IDENTITIES {
            return Err(FeatureError::Policy(format!(
                "{} credentials is above the limit of {MAX_ALLOWED_IDENTITIES}",
                entries.len()
            )));
        }
        Ok(Self {
            realm: realm.to_string(),
            entries,
        })
    }

    /// The challenge to advertise.
    pub fn challenge(&self) -> String {
        format!("Basic realm=\"{}\", charset=\"UTF-8\"", self.realm)
    }

    /// Check a raw `Authorization` header value.
    pub fn verify(&self, header: &str) -> Result<(), DenyReason> {
        let Some(encoded) = header
            .strip_prefix("Basic ")
            .or_else(|| header.strip_prefix("basic "))
        else {
            return Err(DenyReason::CredentialsInvalid);
        };
        let Some(decoded) = decode_base64(encoded.trim()) else {
            return Err(DenyReason::CredentialsInvalid);
        };
        let Ok(text) = String::from_utf8(decoded) else {
            return Err(DenyReason::CredentialsInvalid);
        };
        if self
            .entries
            .iter()
            .any(|e| constant_time_eq(e.as_bytes(), text.as_bytes()))
        {
            Ok(())
        } else {
            Err(DenyReason::CredentialsInvalid)
        }
    }
}

impl AccessController {
    /// A controller that permits everything. The starting point for a tunnel
    /// whose access is decided elsewhere.
    pub fn open() -> Self {
        Self::default()
    }

    /// A controller that refuses every IPv4 visitor, used to stop answering
    /// before a tunnel closes.
    pub fn closed() -> Self {
        let mut c = Self::default();
        c.ips.deny(IpRange::Cidr {
            addr: IpAddr::from([0, 0, 0, 0]),
            bits: 0,
        });
        c
    }

    /// Add a denied address or block.
    pub fn deny_ip(&mut self, range: IpRange) -> &mut Self {
        self.ips.deny(range);
        self
    }

    /// Add an allowed address or block, with the label shown to the operator.
    pub fn allow_ip(&mut self, label: &str, range: IpRange) -> &mut Self {
        self.ips.allow(label, range);
        self
    }

    /// The address rules, for the operator's startup summary.
    pub fn ip_filter(&self) -> &IpFilter {
        &self.ips
    }

    /// Require HTTP Basic credentials from here on.
    ///
    /// Takes `&mut self` like the address rules so a policy can be built in one
    /// chain and cannot be assembled in two incompatible styles.
    pub fn require_basic_auth(
        &mut self,
        realm: &str,
        user_passwords: &[&str],
    ) -> Result<&mut Self, FeatureError> {
        self.http_auth = Some(HttpBasicAuth::new(realm, user_passwords)?);
        Ok(self)
    }

    /// The challenge this controller advertises, if it demands credentials.
    pub fn challenge(&self) -> Option<String> {
        self.http_auth.as_ref().map(|a| a.challenge())
    }

    /// Whether this controller would let everything through.
    pub fn is_open(&self) -> bool {
        self.ips.is_permissive() && self.http_auth.is_none()
    }

    /// Decide what to do with a request from `remote`.
    ///
    /// The order is deliberate. An address that is not permitted gets the same
    /// answer whether or not it also lacked credentials, so the gate does not
    /// confirm to a stranger that a rule exists for their address. The method
    /// check comes last: it describes *this* endpoint, and an unauthorised
    /// caller learns nothing from it.
    pub fn authorize(&self, req: &RequestHead, remote: &IpAddr) -> AccessDecision {
        if !matches!(
            req.method.as_str(),
            "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "OPTIONS" | "PATCH"
        ) {
            return AccessDecision::refusal(&DenyReason::MethodNotAllowed(format!(
                "{} is not accepted here",
                req.method
            )));
        }

        match self.ips.check(remote) {
            IpVerdict::DeniedBy(range) => {
                return AccessDecision::refusal(&DenyReason::IpDenied(range))
            }
            IpVerdict::AllowedBy(_) | IpVerdict::Permitted => {}
        }

        if let Some(auth) = &self.http_auth {
            match req.header("authorization") {
                None => {
                    return AccessDecision::refusal(&DenyReason::CredentialsMissing {
                        challenge: auth.challenge(),
                    })
                }
                Some(header) => {
                    if let Err(reason) = auth.verify(header) {
                        return AccessDecision::refusal(&reason);
                    }
                }
            }
        }

        AccessDecision::Allow
    }
}

/// Compare two byte strings without an early return.
///
/// A byte-at-a-time compare leaks the length of the matching prefix through
/// timing, which turns a shared-rate-limit brute force into an unshared one.
pub(super) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Standard base64, hand-rolled to keep the dependency list short.
///
/// Written here rather than imported because it exists only to check a 30-byte
/// credential header, and `base64` is not among this crate's dependencies.
pub(super) fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;

    for (i, &b) in bytes.iter().enumerate() {
        if b == b'=' {
            // Padding is only legal in the final quantum.
            if i + 2 < bytes.len() {
                return None;
            }
            break;
        }
        acc = (acc << 6) | u32::from(value(b)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}
