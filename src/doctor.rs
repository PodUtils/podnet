//! `podnet doctor`: the path from this host to the Cloudflare edge.
//!
//! The doctor tests each hop that a tunnel needs, and gives a verdict for
//! each: `ok`, `refused` or `unknown`.
//!
//! * The proxy: which proxy each target uses, and if the URL has credentials
//!   (they are never printed).
//! * The TCP path (through the proxy when one applies) to the quick-tunnel
//!   API, the WARP API, the edge on 7844 by name and by address, and the
//!   MASQUE front door on 443. Each path is sampled up to 9 times, 125 ms
//!   apart, in at most 10 s for all paths: best, median, p95, jitter, loss.
//!   A refusal (a proxy 403, a TCP reset) is an answer, not a loss.
//! * TLS on each open path: the issuer of the chain, and if the chain ends at
//!   the roots that podnet expects. A proxy that answers 200 and sends no byte
//!   is named.
//! * UDP 7844: one QUIC packet (see [`crate::udp`]). A bind is not a test.
//!
//! The end is a table of the legs (`quic`, `http2`, `quic+warp`) with the
//! reason for each that cannot work.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::net::BoxedIo;
use crate::udp::Egress;

/// The limits of the doctor.
#[derive(Clone, Debug)]
pub struct Options {
    /// Samples of each TCP path.
    pub samples: usize,
    /// The time between two samples of one path.
    pub gap: Duration,
    /// The time limit of all samples.
    pub budget: Duration,
    /// The time limit of one connection, and of one TLS handshake.
    pub timeout: Duration,
    /// The relay to test (`--relay`); else `PODNET_RELAY`, else the default.
    pub relay: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            samples: 9,
            gap: Duration::from_millis(125),
            budget: Duration::from_secs(10),
            timeout: Duration::from_secs(3),
            relay: None,
        }
    }
}

/// A verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Refused,
    Unknown,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Refused => "fail",
            Verdict::Unknown => "unknown",
        }
    }
}

/// The TLS that a target speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tls {
    /// A public web name: the roots of podnet (`--ca-file`, the system, webpki).
    Web,
    /// The edge: SNI `h2.cftunnel.com`, and the Cloudflare edge roots.
    Edge,
    /// The MASQUE front door: SNI of MASQUE, ALPN h2. Its certificate is
    /// pinned to a key that only a registered device knows, so the doctor
    /// gives the issuer and does not judge it.
    Masque,
    /// A loopback relay on `http://`: a plain check, no TLS.
    Plain,
}

/// One target of the doctor.
#[derive(Clone, Debug)]
pub struct Target {
    pub id: &'static str,
    pub host: String,
    pub port: u16,
    pub tls: Tls,
}

impl Target {
    fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

pub use crate::services::{masque_address, masque_address_v6};

mod targets;
pub use targets::targets;

/// One sample of a TCP path.
#[derive(Clone, Debug)]
enum Sample {
    Open(Duration),
    Refused(Duration, String),
    Lost(String),
}

/// The result of one TCP path.
#[derive(Clone, Debug)]
pub struct Path {
    pub target: Target,
    pub via: Option<String>,
    pub verdict: Verdict,
    pub detail: String,
    pub sent: usize,
    pub answered: usize,
    pub best: Option<Duration>,
    pub median: Option<Duration>,
    pub p95: Option<Duration>,
    pub jitter: Option<Duration>,
    pub tls: Option<TlsResult>,
    /// The answer of the resolver chain for the host (issue 128): the
    /// addresses and the time, or the error. None for an address, and for
    /// a path through a proxy (the proxy resolves the name).
    pub dns: Option<Result<(Vec<std::net::IpAddr>, Duration), String>>,
}

impl Path {
    pub fn loss(&self) -> f64 {
        if self.sent == 0 {
            0.0
        } else {
            (self.sent - self.answered) as f64 / self.sent as f64
        }
    }

    /// The verdict of the path and of its TLS together.
    pub fn usable(&self) -> (Verdict, String) {
        match (&self.verdict, &self.tls) {
            (Verdict::Ok, Some(tls)) => (tls.verdict, tls.detail.clone()),
            (Verdict::Ok, None) => (Verdict::Unknown, "no TLS result".into()),
            (verdict, _) => (*verdict, self.detail.clone()),
        }
    }
}

/// The result of a TLS handshake.
#[derive(Clone, Debug)]
pub struct TlsResult {
    pub verdict: Verdict,
    pub issuer: Option<String>,
    pub detail: String,
}

/// One leg and its verdict.
#[derive(Clone, Debug)]
pub struct Leg {
    pub name: &'static str,
    pub verdict: Verdict,
    pub reason: String,
}

/// The report of the doctor.
#[derive(Clone, Debug)]
pub struct Report {
    pub proxy: Value,
    pub paths: Vec<Path>,
    pub udp: (std::net::SocketAddr, Egress),
    pub names: Option<String>,
    pub legs: Vec<Leg>,
    /// The clock of this host against the Date of an API (issue 128).
    pub clock: Clock,
    /// The installed git, for git serve (T-094).
    pub git: Git,
}

/// The clock of this host against the `Date` header of an answer.
#[derive(Clone, Debug)]
pub struct Clock {
    pub verdict: Verdict,
    /// This host minus the remote, in seconds.
    pub skew: Option<i64>,
    pub detail: String,
}

/// A skew of the clock above this breaks TLS (certificates) and tokens.
pub const CLOCK_LIMIT: i64 = 300;

/// The roads of podnet and their legs: a road works when one of its legs
/// works.
pub const ROADS: &[(&str, &[&str])] = &[
    ("direct", &["quic", "http2"]),
    ("warp", &["quic+warp"]),
    ("relay", &["http2+relay"]),
];

impl Report {
    /// True when at least one leg can work.
    pub fn ok(&self) -> bool {
        self.legs.iter().any(|l| l.verdict == Verdict::Ok)
    }

    /// The verdict of each road of [`ROADS`], with the reason of its first
    /// leg.
    pub fn roads(&self) -> Vec<(&'static str, Verdict, String)> {
        ROADS
            .iter()
            .filter_map(|(road, legs)| {
                let legs: Vec<&Leg> = self
                    .legs
                    .iter()
                    .filter(|l| legs.contains(&l.name))
                    .collect();
                let first = legs.first()?;
                Some(match legs.iter().find(|l| l.verdict == Verdict::Ok) {
                    Some(ok) => (*road, Verdict::Ok, format!("{}: {}", ok.name, ok.reason)),
                    None => (
                        *road,
                        first.verdict,
                        format!("{}: {}", first.name, first.reason),
                    ),
                })
            })
            .collect()
    }

    pub fn to_json(&self) -> Value {
        let ms = |d: Option<Duration>| d.map(|d| (d.as_secs_f64() * 10000.0).round() / 10.0);
        crate::status::stamped(json!({
            "ok": self.ok(),
            "roads": self.roads().iter().map(|(road, verdict, reason)| json!({
                "road": road,
                "verdict": verdict.name(),
                "status": verdict.name(),
                "reason": reason,
            })).collect::<Vec<_>>(),
            "clock": {
                "verdict": self.clock.verdict.name(),
                "status": self.clock.verdict.name(),
                "skew_s": self.clock.skew,
                "detail": self.clock.detail,
            },
            "git": self.git.to_json(),
            "proxy": self.proxy,
            "paths": self.paths.iter().map(|p| json!({
                "id": p.target.id,
                "target": p.target.authority(),
                "via": p.via,
                "verdict": p.verdict.name(),
                "status": p.verdict.name(),
                "detail": p.detail,
                "samples": {
                    "sent": p.sent,
                    "answered": p.answered,
                    "loss": p.loss(),
                    "best_ms": ms(p.best),
                    "median_ms": ms(p.median),
                    "p95_ms": ms(p.p95),
                    "jitter_ms": ms(p.jitter),
                },
                "tls": p.tls.as_ref().map(|t| json!({
                    "verdict": t.verdict.name(),
                    "status": t.verdict.name(),
                    "issuer": t.issuer,
                    "detail": t.detail,
                })),
                "dns": p.dns.as_ref().map(|d| match d {
                    Ok((addrs, took)) => json!({
                        "addresses": addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                        "ms": ms(Some(*took)),
                    }),
                    Err(e) => json!({ "error": e }),
                }),
            })).collect::<Vec<_>>(),
            "udp": {
                "target": self.udp.0.to_string(),
                "verdict": self.udp.1.name(),
                "status": match self.udp.1 {
                    Egress::Answered(_) => "ok",
                    Egress::Refused(_) => "fail",
                    Egress::Unknown(_) => "unknown",
                },
                "detail": self.udp.1.detail(),
            },
            "names": self.names,
            "next": self.next(),
            "legs": self.legs.iter().map(|l| json!({
                "leg": l.name,
                "verdict": l.verdict.name(),
                "status": l.verdict.name(),
                "reason": l.reason,
            })).collect::<Vec<_>>(),
        }))
    }

    pub fn text(&self) -> String {
        let ms = |d: Option<Duration>| match d {
            Some(d) => format!("{:.1}", d.as_secs_f64() * 1000.0),
            None => "-".into(),
        };
        let mut out = String::new();
        out.push_str(&format!(
            "proxy  {}\n\n",
            self.proxy["text"].as_str().unwrap_or("")
        ));
        out.push_str(
            "path                                  verdict  best ms  median   p95  jitter  loss\n",
        );
        for p in &self.paths {
            out.push_str(&format!(
                "{:<37} {:<8} {:>7} {:>7} {:>5} {:>7} {:>4.0}%\n",
                p.target.authority(),
                p.verdict.name(),
                ms(p.best),
                ms(p.median),
                ms(p.p95),
                ms(p.jitter),
                p.loss() * 100.0
            ));
            match &p.dns {
                Some(Ok((addrs, took))) => out.push_str(&format!(
                    "    dns {} ({} ms)\n",
                    addrs
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    ms(Some(*took))
                )),
                Some(Err(e)) => out.push_str(&format!("    dns failed: {e}\n")),
                None => {}
            }
            if p.verdict != Verdict::Ok && !matches!(p.dns, Some(Err(_))) {
                out.push_str(&format!("    {}\n", p.detail));
            }
            if let Some(tls) = &p.tls {
                out.push_str(&format!("    tls {}: {}\n", tls.verdict.name(), tls.detail));
            }
        }
        out.push_str(&format!(
            "\nudp    {}  {}: {}\n",
            self.udp.0,
            self.udp.1.name(),
            self.udp.1.detail()
        ));
        if let Some(names) = &self.names {
            out.push_str(&format!("names  {names}\n"));
        }
        out.push_str("\nlegs\n");
        for leg in &self.legs {
            out.push_str(&format!(
                "  {:<10} {:<8} {}\n",
                leg.name,
                leg.verdict.name(),
                leg.reason
            ));
        }
        out.push_str(&format!(
            "\nclock  {}: {}\n",
            self.clock.verdict.name(),
            self.clock.detail
        ));
        out.push_str(&self.git.line());
        out.push_str("\nroads\n");
        for (road, verdict, reason) in self.roads() {
            out.push_str(&format!("  {road:<10} {:<8} {reason}\n", verdict.name()));
        }
        let can = self
            .legs
            .iter()
            .filter(|l| l.verdict == Verdict::Ok)
            .count();
        out.push_str(&format!(
            "\npodnet doctor: {can} of {} legs can work on this host\n",
            self.legs.len()
        ));
        out.push_str(&format!("next: {}\n", self.next()));
        out
    }

    /// The step to take now (E5): the command that uses a leg that can
    /// work, else what to look at first.
    pub fn next(&self) -> String {
        if self.clock.verdict == Verdict::Refused {
            // The detail names the fix (timedatectl, ntpdate).
            return self.clock.detail.clone();
        }
        let ok = |name: &str| {
            self.legs
                .iter()
                .any(|l| l.name == name && l.verdict == Verdict::Ok)
        };
        let url = "--url http://localhost:8080";
        if ok("quic") || ok("http2") || ok("quic+warp") {
            // --transport auto tries quic, http2 and quic+warp in turn.
            format!("podnet tunnel {url} (the default legs work here)")
        } else if ok("http2+relay") {
            format!("podnet tunnel --transport relay {url} (only the relay works here)")
        } else {
            "no leg works: read the proxy and path lines above, then podnet bug-report --doctor for an issue".to_string()
        }
    }
}

/// Run the doctor.
pub async fn run(options: Options) -> Report {
    crate::init_crypto();
    let targets = targets(options.relay.as_deref());
    let proxy = proxy_report(&targets);
    let start = Instant::now();
    let udp_target = crate::udp::probe_target();
    let udp = tokio::task::spawn_blocking(move || {
        crate::udp::probe(udp_target, crate::udp::DOCTOR_DEADLINE)
    });
    let paths =
        futures::future::join_all(targets.into_iter().map(|t| path(t, &options, start))).await;
    let udp = udp
        .await
        .unwrap_or_else(|e| Egress::Unknown(format!("the probe did not run: {e}")));
    let names = names(&paths);
    let legs = legs(&paths, &udp);
    let clock = clock(&paths, options.timeout).await;
    Report {
        proxy,
        paths,
        udp: (udp_target, udp),
        names,
        legs,
        clock,
        git: git_check::git("git"),
    }
}

mod clock_check;
mod git_check;
use clock_check::clock;
pub use clock_check::parse_imf_fixdate;
pub use git_check::Git;
mod tls_check;
use tls_check::tls;

fn proxy_report(targets: &[Target]) -> Value {
    let mut used = Vec::new();
    for t in targets {
        if let Some(p) = crate::egress::proxy_for(&t.host, t.port) {
            used.push((t.id, p));
        }
    }
    let direct: Vec<&str> = targets
        .iter()
        .filter(|t| crate::egress::proxy_for(&t.host, t.port).is_none())
        .map(|t| t.id)
        .collect();
    let first = used.first().map(|(_, p)| p.clone());
    let text = match &first {
        None => "none: each connection is direct".to_string(),
        Some(p) => {
            let mut text = format!(
                "{} (credentials: {})",
                p.label(),
                if p.has_credentials() {
                    "present"
                } else {
                    "none"
                }
            );
            if !direct.is_empty() {
                text.push_str(&format!("; direct by NO_PROXY: {}", direct.join(", ")));
            }
            text
        }
    };
    json!({
        "url": first.as_ref().map(|p| p.label()),
        "credentials": first.as_ref().map(|p| p.has_credentials()),
        "direct": direct,
        "text": text,
    })
}

async fn path(target: Target, options: &Options, start: Instant) -> Path {
    let via = crate::egress::proxy_for(&target.host, target.port).map(|p| p.label());
    // Issue 128: the answer of the resolver chain, before the samples.
    let dns = if via.is_none() && target.host.parse::<std::net::IpAddr>().is_err() {
        let took = Instant::now();
        Some(
            match tokio::time::timeout(options.timeout * 3, crate::resolve::resolve(&target.host))
                .await
            {
                Ok(Ok(addrs)) => Ok((addrs, took.elapsed())),
                Ok(Err(e)) => Err(format!("{e:#}")),
                Err(_) => Err(format!(
                    "no answer in {:.1} s",
                    (options.timeout * 3).as_secs_f64()
                )),
            },
        )
    } else {
        None
    };
    let mut samples = Vec::new();
    // Issue 180: a failed lookup is the answer; the samples would only
    // fail the same lookup again (a second line for one failure).
    let dns_failed = matches!(dns, Some(Err(_)));
    for i in 0..if dns_failed {
        0
    } else {
        options.samples.max(1)
    } {
        let left = options.budget.saturating_sub(start.elapsed());
        if left.is_zero() {
            break;
        }
        if i > 0 {
            tokio::time::sleep(options.gap.min(left)).await;
        }
        let limit = options
            .timeout
            .min(options.budget.saturating_sub(start.elapsed()));
        if limit.is_zero() {
            break;
        }
        samples.push(sample(&target, limit).await);
    }
    let times: Vec<Duration> = samples
        .iter()
        .filter_map(|s| match s {
            Sample::Open(d) | Sample::Refused(d, _) => Some(*d),
            Sample::Lost(_) => None,
        })
        .collect();
    let (verdict, detail) = if samples.iter().any(|s| matches!(s, Sample::Open(_))) {
        let note = if target.tls == Tls::Plain {
            "open; loopback relay, no TLS"
        } else {
            "open"
        };
        (Verdict::Ok, note.to_string())
    } else if let Some(Sample::Refused(_, why)) =
        samples.iter().find(|s| matches!(s, Sample::Refused(..)))
    {
        (Verdict::Refused, why.clone())
    } else if let Some(Sample::Lost(why)) = samples.first() {
        (Verdict::Unknown, why.clone())
    } else if dns_failed {
        (
            Verdict::Unknown,
            "the name does not resolve (see dns)".to_string(),
        )
    } else {
        (Verdict::Unknown, "no sample in the time limit".to_string())
    };
    let tls = if verdict == Verdict::Ok && target.tls != Tls::Plain {
        Some(tls(&target, options.timeout).await)
    } else {
        None
    };
    let (best, median, p95, jitter) = stats(&times);
    Path {
        sent: samples.len(),
        answered: times.len(),
        target,
        via,
        verdict,
        detail,
        best,
        median,
        p95,
        jitter,
        tls,
        dns,
    }
}

/// Best, median, 95th percentile, and jitter (the mean difference of two
/// samples in a row).
fn stats(
    times: &[Duration],
) -> (
    Option<Duration>,
    Option<Duration>,
    Option<Duration>,
    Option<Duration>,
) {
    if times.is_empty() {
        return (None, None, None, None);
    }
    let mut sorted = times.to_vec();
    sorted.sort();
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let jitter = (times.len() > 1).then(|| {
        let sum: Duration = times.windows(2).map(|w| w[1].abs_diff(w[0])).sum();
        sum / (times.len() - 1) as u32
    });
    (Some(sorted[0]), Some(at(0.5)), Some(at(0.95)), jitter)
}

async fn sample(target: &Target, limit: Duration) -> Sample {
    let t = Instant::now();
    let result = tokio::time::timeout(
        limit,
        crate::egress::dial(&target.host, target.port, None, limit),
    )
    .await;
    let took = t.elapsed();
    match result {
        Err(_) => Sample::Lost(format!("no answer in {} ms", limit.as_millis())),
        Ok(Ok(_)) => Sample::Open(took),
        Ok(Err(err)) => classify(&err, took),
    }
}

fn classify(err: &anyhow::Error, took: Duration) -> Sample {
    if let Some(refusal) = crate::egress::refusal(err) {
        let text = if refusal.status > 0 {
            format!("proxy {}: {}", refusal.status, refusal.text)
        } else {
            format!("proxy: {}", refusal.text)
        };
        return Sample::Refused(took, text);
    }
    // Issue 180: a local answer is a refusal: the port is closed, the
    // network or the host is unreachable, a firewall refused (EPERM).
    let refused = err.chain().find_map(|e| {
        e.downcast_ref::<std::io::Error>().and_then(|io| {
            use std::io::ErrorKind::*;
            match io.kind() {
                ConnectionRefused => Some("connection refused"),
                NetworkUnreachable => Some("network unreachable"),
                HostUnreachable => Some("host unreachable"),
                PermissionDenied => Some("not permitted"),
                _ => None,
            }
        })
    });
    if let Some(what) = refused {
        return Sample::Refused(took, format!("{what}: {err:#}"));
    }
    Sample::Lost(format!("{err:#}"))
}

/// What the proxy does with names and with addresses (the edge on 7844).
fn names(paths: &[Path]) -> Option<String> {
    let find = |id: &str| paths.iter().find(|p| p.target.id == id);
    let (name, address) = (find("edge-name")?, find("edge-address")?);
    name.via.as_ref()?;
    let text = match (name.verdict, address.verdict) {
        (Verdict::Ok, Verdict::Ok) => "the proxy carries names and IP addresses",
        (Verdict::Ok, Verdict::Refused) => {
            "the proxy allows names and refuses IP addresses; the edge legs dial addresses"
        }
        (Verdict::Refused, Verdict::Ok) => "the proxy refuses names and allows IP addresses",
        (Verdict::Refused, Verdict::Refused) => "the proxy refuses names and IP addresses on 7844",
        _ => "no answer for a name or an address on 7844",
    };
    Some(text.to_string())
}

fn legs(paths: &[Path], udp: &Egress) -> Vec<Leg> {
    let find = |id: &str| paths.iter().find(|p| p.target.id == id);
    let via_proxy = crate::egress::proxy_for(crate::tunnel::edge_region_name(), 7844).is_some();
    let udp_off = crate::udp::Choice::from_env().ok() == Some(crate::udp::Choice::Off);
    let quic = if udp_off {
        Leg {
            name: "quic",
            verdict: Verdict::Refused,
            reason: "PODNET_UDP=off".into(),
        }
    } else if via_proxy {
        Leg {
            name: "quic",
            verdict: Verdict::Refused,
            reason: "a proxy is set, and no proxy carries UDP".into(),
        }
    } else {
        match udp {
            Egress::Answered(d) => Leg {
                name: "quic",
                verdict: Verdict::Ok,
                reason: format!("UDP 7844 answered: {d}"),
            },
            Egress::Refused(d) => Leg {
                name: "quic",
                verdict: Verdict::Refused,
                reason: format!("UDP 7844: {d}"),
            },
            Egress::Unknown(d) => Leg {
                name: "quic",
                verdict: Verdict::Unknown,
                reason: format!("UDP 7844: {d}"),
            },
        }
    };
    let leg = |name: &'static str, ids: &[&str]| {
        for id in ids {
            let Some(p) = find(id) else { continue };
            let (verdict, why) = p.usable();
            if verdict != Verdict::Ok {
                return Leg {
                    name,
                    verdict,
                    reason: format!("{}: {why}", p.target.authority()),
                };
            }
        }
        Leg {
            name,
            verdict: Verdict::Ok,
            reason: ids
                .iter()
                .filter_map(|id| find(id).map(|p| p.target.authority()))
                .collect::<Vec<_>>()
                .join(" and ")
                + ": TCP and TLS work",
        }
    };
    // The front door of IPv6 counts when the one of IPv4 does not work.
    let masque = match find("masque-v6") {
        Some(p)
            if p.usable().0 == Verdict::Ok
                && find("masque").is_none_or(|m| m.usable().0 != Verdict::Ok) =>
        {
            "masque-v6"
        }
        _ => "masque",
    };
    let mut legs = vec![
        quic,
        leg("http2", &["edge-address"]),
        leg("quic+warp", &[masque, "warp-api"]),
    ];
    if find("relay").is_some() {
        let mut relay = leg("http2+relay", &["relay"]);
        if relay.verdict == Verdict::Ok {
            relay.reason.push_str("; the relay protocol is not tried");
        }
        legs.push(relay);
    }
    legs
}

#[cfg(test)]
mod tests;
