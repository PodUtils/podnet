//! The probes of the lanes `masque_udp443` and `ws_upgrade` (T-260).

use super::*;

/// The result of the probe of one lane.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneProbe {
    pub verdict: Verdict,
    /// The time of the answer.
    pub time: Option<Duration>,
    pub reason: String,
}

/// The results of both probes.
#[derive(Clone, Debug, PartialEq)]
pub struct Probes {
    pub masque_udp: LaneProbe,
    pub ws_upgrade: LaneProbe,
}

impl Probes {
    /// Both probes did not run.
    pub fn untested() -> Self {
        Self {
            masque_udp: LaneProbe::unknown("no probe sends a datagram to MASQUE on UDP 443"),
            ws_upgrade: LaneProbe::unknown("no probe makes a WebSocket upgrade to the edge host"),
        }
    }
}

impl LaneProbe {
    pub(super) fn unknown(reason: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::Unknown,
            time: None,
            reason: reason.into(),
        }
    }
}

/// The datagram that was sent, or the reason it was not.
pub(super) enum Udp {
    Skipped(String),
    Sent(Egress, Duration),
}

/// Send one QUIC packet to `target`. A proxy carries no UDP: with a proxy
/// for the target, no packet is sent. This blocks.
pub(super) fn udp_send(target: std::net::SocketAddr, via_proxy: bool, deadline: Duration) -> Udp {
    if via_proxy {
        return Udp::Skipped(format!(
            "a proxy is set for {target}, and no proxy carries UDP"
        ));
    }
    let start = Instant::now();
    let got = crate::udp::probe(target, deadline);
    Udp::Sent(got, start.elapsed())
}

impl Udp {
    /// The lane. `edge` is the result of the probe of UDP 7844: when it
    /// answers, UDP is open on this host, so a silent MASQUE endpoint is a
    /// fail. Without that proof, silence is `unknown`.
    pub(super) fn lane(self, edge: &Egress) -> LaneProbe {
        let (got, took) = match self {
            Udp::Skipped(reason) => return LaneProbe::unknown(reason),
            Udp::Sent(got, took) => (got, took),
        };
        let lane = |verdict, time, reason| LaneProbe {
            verdict,
            time,
            reason,
        };
        match got {
            Egress::Answered(d) => lane(Verdict::Ok, Some(took), d),
            Egress::Refused(d) => lane(Verdict::Refused, None, d),
            Egress::Unknown(d) if matches!(edge, Egress::Answered(_)) => lane(
                Verdict::Refused,
                None,
                format!("{d}; UDP 7844 answers, so UDP is open on this host"),
            ),
            Egress::Unknown(d) => lane(
                Verdict::Unknown,
                None,
                format!("{d}; UDP 7844 gave no answer either, so UDP may be closed"),
            ),
        }
    }
}

/// The request of the WebSocket probe.
fn ws_request(sni: &str) -> Vec<u8> {
    let mut key = [0u8; 16];
    let _ = getrandom::fill(&mut key);
    let key = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(key)
    };
    format!(
        "GET / HTTP/1.1\r\nHost: {sni}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\nUser-Agent: podnet-doctor\r\n\r\n"
    )
    .into_bytes()
}

/// An HTTP/1.1 `Upgrade: websocket` request to `target` over TLS, through
/// the proxy when one is set. `ok` on `101`; `fail` on another status, a TLS
/// fault or a refusal; `unknown` when no answer comes in `2 * timeout`.
pub(super) async fn ws_upgrade(
    target: &Target,
    timeout: Duration,
    verifier: Arc<dyn ServerCertVerifier>,
) -> LaneProbe {
    let start = Instant::now();
    let work = ws_exchange(target, timeout, verifier);
    match tokio::time::timeout(timeout * 2, work).await {
        Ok(Ok(status)) if status.starts_with("HTTP/1.1 101") => LaneProbe {
            verdict: Verdict::Ok,
            time: Some(start.elapsed()),
            reason: format!("{}: {status}", target.authority()),
        },
        Ok(Ok(status)) => fail(format!("{}: the upgrade got {status}", target.authority())),
        Ok(Err((true, why))) => fail(format!("{}: {why}", target.authority())),
        Ok(Err((false, why))) => LaneProbe::unknown(format!("{}: {why}", target.authority())),
        Err(_) => LaneProbe::unknown(format!(
            "{}: no answer in {} ms",
            target.authority(),
            (timeout * 2).as_millis()
        )),
    }
}

fn fail(reason: String) -> LaneProbe {
    LaneProbe {
        verdict: Verdict::Refused,
        time: None,
        reason,
    }
}

/// The status line of the answer, or `(fault, why)`: `fault` is true when
/// the peer, the proxy or TLS refused.
async fn ws_exchange(
    target: &Target,
    timeout: Duration,
    verifier: Arc<dyn ServerCertVerifier>,
) -> Result<String, (bool, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let io = crate::egress::dial(&target.host, target.port, None, timeout)
        .await
        .map_err(|err| match classify(&err, Duration::ZERO) {
            Sample::Refused(_, why) => (true, why),
            Sample::Open(_) | Sample::Lost(_) => (false, format!("{err:#}")),
        })?;
    let sni = match target.tls {
        Tls::Edge => crate::cloudflare::h2_sni().to_string(),
        _ => target.host.clone(),
    };
    let name = ServerName::try_from(sni.clone())
        .map_err(|_| (false, format!("the name {sni} is not a TLS name")))?;
    let mut tls = tls_check::connector(verifier, vec![b"http/1.1".to_vec()])
        .connect(name, io)
        .await
        .map_err(|err| (true, format!("TLS failed: {err}")))?;
    tls.write_all(&ws_request(&sni))
        .await
        .map_err(|err| (true, format!("the request was not sent: {err}")))?;
    let mut head = Vec::new();
    let mut chunk = [0u8; 256];
    while !head.windows(2).any(|w| w == b"\r\n") && head.len() < 1024 {
        match tls.read(&mut chunk).await {
            Ok(0) => {
                return Err((
                    true,
                    "the peer closed the connection with no status line".into(),
                ))
            }
            Ok(n) => head.extend_from_slice(&chunk[..n]),
            Err(err) => return Err((true, format!("TLS failed: {err}"))),
        }
    }
    let line = String::from_utf8_lossy(&head);
    Ok(line.lines().next().unwrap_or("").trim().to_string())
}

/// The probes that run beside the paths.
pub(super) struct Pending {
    udp: tokio::task::JoinHandle<Udp>,
    ws: tokio::task::JoinHandle<LaneProbe>,
}

/// Start both probes. The UDP probe goes to the MASQUE endpoint; the
/// WebSocket probe goes to the target `edge-name` (the edge host).
pub(super) fn start(targets: &[Target], timeout: Duration) -> Pending {
    let port = crate::warp::register::masque_port();
    let host = masque_address();
    let via_proxy = crate::egress::proxy_for(host, port).is_some();
    let address = host.parse::<std::net::IpAddr>();
    let udp = tokio::task::spawn_blocking(move || match address {
        Ok(ip) => udp_send(std::net::SocketAddr::new(ip, port), via_proxy, timeout),
        Err(_) => Udp::Skipped(format!("the MASQUE address {host} is not an IP address")),
    });
    let edge = targets.iter().find(|t| t.id == "edge-name").cloned();
    let ws = tokio::spawn(async move {
        let Some(edge) = edge else {
            return LaneProbe::unknown("the edge host is not a target");
        };
        match tls_check::verifier(edge.tls) {
            Ok(Some(verifier)) => ws_upgrade(&edge, timeout, verifier).await,
            Ok(None) => LaneProbe::unknown("no TLS verifier for the edge host"),
            Err(e) => LaneProbe::unknown(format!("the roots of podnet: {e}")),
        }
    });
    Pending { udp, ws }
}

impl Pending {
    /// The results. A probe that is not done at `end` (`--quick`) is
    /// `unknown`. `edge` is the result of the probe of UDP 7844.
    pub(super) async fn finish(self, end: Option<tokio::time::Instant>, edge: &Egress) -> Probes {
        let late = || async move {
            match end {
                Some(end) => tokio::time::sleep_until(end).await,
                None => std::future::pending().await,
            }
        };
        let masque_udp = tokio::select! {
            r = self.udp => match r {
                Ok(udp) => udp.lane(edge),
                Err(e) => LaneProbe::unknown(format!("the probe did not run: {e}")),
            },
            () = late() => LaneProbe::unknown(LATE),
        };
        let ws_upgrade = tokio::select! {
            r = self.ws => r.unwrap_or_else(|e| LaneProbe::unknown(format!("the probe did not run: {e}"))),
            () = late() => LaneProbe::unknown(LATE),
        };
        Probes {
            masque_udp,
            ws_upgrade,
        }
    }
}

#[cfg(test)]
mod tests;
