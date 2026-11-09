//! The verify of the public URL of a tunnel: fetch it until it answers.

use std::time::Duration;

use anyhow::Result;
use tracing::info;

/// The loop of `TunnelHandle::verify_accepting`, with the budget and the
/// interval as arguments (for the tests).
pub(super) async fn verify_fetch(
    url: &str,
    expected: Option<&str>,
    accepted: &[u16],
    budget: Duration,
    interval: Duration,
    strict: bool,
) -> Result<Verified> {
    let mut last;
    let mut last_status;
    // Each attempt so far failed before the edge: at name resolution, or at
    // the CONNECT of the proxy (T-160).
    let mut resolution_only = true;
    let deadline = tokio::time::Instant::now() + budget;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let request = crate::fetch::Request::new("GET", url).timeout(Duration::from_secs(15));
        match crate::fetch::send(request).await {
            Ok(response) => {
                resolution_only = false;
                let body = response.text();
                let ok = response.is_success()
                    && expected.map(|token| body.contains(token)).unwrap_or(true);
                if ok {
                    let dns = public_dns(url).await;
                    info!(%url, status = response.status, attempt, dns, "verified public URL");
                    if dns == "doh" {
                        info!(
                            "the system resolver of this host does not resolve the new name yet; \
                             public DNS does. A visitor whose resolver cached NXDOMAIN gets an \
                             error for some seconds more"
                        );
                    }
                    return Ok(Verified {
                        dns,
                        unverified: None,
                    });
                }
                last = format!("HTTP {}", response.status);
                last_status = Some(response.status);
                // A status of the origin crossed the tunnel: the tunnel is up.
                let edge_error = matches!(response.status, 502..=504 | 520..=530);
                if expected.is_none() && !edge_error {
                    if accepted.contains(&response.status) {
                        info!(%url, status = response.status, "the origin answered HTTP {}, its normal answer; the tunnel is up", response.status);
                    } else {
                        tracing::warn!(%url, status = response.status, "the origin answered HTTP {}; the tunnel is up", response.status);
                    }
                    return Ok(Verified {
                        dns: public_dns(url).await,
                        unverified: None,
                    });
                }
            }
            Err(err) => {
                last_status = None;
                if is_resolution_failure(&err) {
                    last = format!("waiting for DNS: {err:#}");
                    if attempt == 1 && crate::hop::of(&err) == Some(crate::hop::Hop::Dns) {
                        info!("this host does not resolve the new name yet; {}", REMEDY);
                    }
                } else {
                    resolution_only = false;
                    last = format!("{err:#}");
                }
            }
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        // Issue 94: the retries were at DEBUG only, so a slow new name
        // looked like a hang for up to a minute.
        let left_s = left.as_secs();
        if attempt > 10 && attempt.is_multiple_of(5) {
            tracing::warn!(attempt, left_s, %last, "the public URL does not answer yet; podnet tries again");
        } else if attempt == 1 || attempt.is_multiple_of(5) {
            info!(attempt, left_s, %last, "the public URL does not answer yet (a new name takes some seconds); podnet tries again");
        } else {
            tracing::debug!(attempt, %last, "verification retry");
        }
        tokio::time::sleep(interval.min(left)).await;
    }
    if expected.is_none() && matches!(last_status, Some(s) if s != 530) {
        tracing::warn!(%url, "the public URL answers {last}; the tunnel is up, check the origin");
        return Ok(Verified {
            dns: public_dns(url).await,
            unverified: None,
        });
    }
    if resolution_only && !strict {
        tracing::warn!(
            %url, %last,
            "the tunnel is registered; this host cannot reach the name yet; the URL is not proved. \
             Set PODNET_VERIFY_STRICT=1 to make this an error"
        );
        return Ok(Verified {
            dns: "none",
            unverified: Some("resolution"),
        });
    }
    let hint = if resolution_only {
        format!("; {REMEDY}")
    } else {
        String::new()
    };
    Err(crate::exit::fault(
        crate::exit::Fault::Unavailable,
        anyhow::anyhow!(
            "could not verify {url} in {} s: {last}{hint}",
            budget.as_secs()
        ),
    ))
}

/// The time of the retries of the verify fetch (issue 119: 20 tries in
/// about 40 s were too few for a new name in public DNS).
pub(super) const VERIFY_BUDGET: Duration = Duration::from_secs(180);
/// The pause between two verify fetches.
pub(super) const VERIFY_INTERVAL: Duration = Duration::from_secs(2);

/// What may help when this host cannot resolve the public name.
const REMEDY: &str = "give the address with --resolve HOST=IP, or use --no-verify";

/// True when the operator wants exit 69 at a verify that is not proved
/// (`PODNET_VERIFY_STRICT` is set to a value that is not empty or `0`).
pub(super) fn strict() -> bool {
    std::env::var("PODNET_VERIFY_STRICT").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// True for a failure before the edge: name resolution, or a refusal of
/// CONNECT by a proxy (a proxy that is not allowed to reach the name says
/// the same). The tunnel can be up in both cases.
fn is_resolution_failure(err: &anyhow::Error) -> bool {
    crate::hop::of(err) == Some(crate::hop::Hop::Dns)
        || err
            .chain()
            .any(|e| e.downcast_ref::<crate::egress::Refusal>().is_some())
}

/// The result of a verify of the public URL.
#[derive(Clone, Copy, Debug)]
pub struct Verified {
    /// `Some("resolution")` when the verify did not reach the edge because
    /// this host cannot resolve the name, or a proxy refused it. The tunnel
    /// is up; the URL is not proved.
    pub unverified: Option<&'static str>,
    /// The step of the resolver chain that resolves the public name now:
    /// `literal`, `pin`, `system`, `doh` or `none`
    /// ([`crate::resolve::answering_step`]).
    pub dns: &'static str,
}

/// [`crate::resolve::answering_step`] for the host of `url`, in at most 10 s.
async fn public_dns(url: &str) -> &'static str {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    if host.is_empty() {
        return "none";
    }
    tokio::time::timeout(
        Duration::from_secs(10),
        crate::resolve::answering_step(host),
    )
    .await
    .unwrap_or("none")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local HTTP origin: answer N of `statuses` gives the Nth status,
    /// then the last one again.
    async fn origin(statuses: Vec<u16>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut n = 0;
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let status = statuses[n.min(statuses.len() - 1)];
                n += 1;
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
                );
                let _ = s.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{addr}/")
    }

    // Issue 119: the retries go on for a time budget, not for a count.
    #[tokio::test]
    async fn verify_waits_through_edge_errors() {
        crate::test_host::need_loopback!();
        let url = origin(vec![530, 530, 502, 200]).await;
        let ok = verify_fetch(
            &url,
            Some("ok"),
            &[],
            Duration::from_secs(10),
            Duration::from_millis(20),
            false,
        )
        .await;
        assert!(ok.is_ok(), "{ok:?}");
    }

    // Issue 119: a verify that fails exits 69 (unavailable), not 1.
    #[tokio::test]
    async fn a_failed_verify_exits_69_after_the_budget() {
        crate::test_host::need_loopback!();
        let url = origin(vec![530]).await;
        let start = std::time::Instant::now();
        let err = verify_fetch(
            &url,
            None,
            &[],
            Duration::from_millis(300),
            Duration::from_millis(50),
            false,
        )
        .await
        .unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(crate::exit::code_of(&err), 69, "{err:#}");
        assert!(format!("{err:#}").contains("HTTP 530"), "{err:#}");
    }

    // Issue 119: a 404 of `serve` with no index.html is its normal answer.
    #[tokio::test]
    async fn an_accepted_status_of_the_origin_verifies() {
        crate::test_host::need_loopback!();
        let url = origin(vec![404]).await;
        let ok = verify_fetch(
            &url,
            None,
            &[404],
            Duration::from_secs(5),
            Duration::from_millis(20),
            false,
        )
        .await;
        assert!(ok.is_ok(), "{ok:?}");
    }

    /// Issue 94: the host of the URL goes to the resolver chain; an
    /// address needs no lookup, and a URL with no host is "none".
    #[tokio::test]
    async fn the_dns_step_of_a_url() {
        crate::test_host::need_localhost!();
        assert_eq!(public_dns("https://127.0.0.1:8443/s/x/").await, "literal");
        assert_eq!(public_dns("https://localhost/").await, "system");
        assert_eq!(public_dns("https:///x").await, "none");
    }

    // T-160: a name that this host cannot resolve keeps the tunnel up.
    #[tokio::test]
    async fn a_name_that_does_not_resolve_keeps_the_tunnel_up() {
        if crate::egress::proxy_for("podnet-no-such-name.invalid", 80).is_some() {
            // A proxy of this host answers for the name.
            return;
        }
        let url = "http://podnet-no-such-name.invalid/";
        let ok = verify_fetch(
            url,
            None,
            &[],
            Duration::from_millis(300),
            Duration::from_millis(50),
            false,
        )
        .await
        .unwrap();
        assert_eq!(ok.unverified, Some("resolution"));
        assert_eq!(ok.dns, "none");
        // A body token (the demo) does not change it.
        let ok = verify_fetch(
            url,
            Some("x"),
            &[],
            Duration::from_millis(300),
            Duration::from_millis(50),
            false,
        )
        .await
        .unwrap();
        assert_eq!(ok.unverified, Some("resolution"));
    }

    // T-160, T-171: the strict switch gives exit 69, and the error names the remedy.
    #[tokio::test]
    async fn verify_failure_names_the_remedy() {
        if crate::egress::proxy_for("podnet-no-such-name.invalid", 80).is_some() {
            // A proxy of this host answers for the name.
            return;
        }
        let err = verify_fetch(
            "http://podnet-no-such-name.invalid/",
            None,
            &[],
            Duration::from_millis(300),
            Duration::from_millis(50),
            true,
        )
        .await
        .unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 69, "{err:#}");
        assert!(format!("{err:#}").contains("--resolve HOST=IP"), "{err:#}");
        assert!(format!("{err:#}").contains("--no-verify"), "{err:#}");
    }

    // T-160: a refusal of CONNECT by a proxy is a failure before the edge.
    #[test]
    fn a_proxy_refusal_is_a_resolution_failure() {
        let refusal = anyhow::Error::new(crate::egress::Refusal {
            status: 502,
            text: "refused".into(),
        });
        assert!(is_resolution_failure(&refusal));
        assert!(!is_resolution_failure(&anyhow::anyhow!("reset")));
    }
}
