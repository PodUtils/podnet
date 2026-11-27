//! The report of a failure (`podnet_core::report`, at its old path), and
//! the line that podnet prints for an error. The line reads the refusal of
//! a proxy ([`crate::egress`]), so it stays in this crate.

#[doc(inline)]
pub use podnet_core::report::*;

/// The line that podnet prints for the error `err`: masked and clean.
pub fn error_line(err: &anyhow::Error) -> String {
    let mut line = format!("podnet: {}", clean(&mask(&chain_text(err))));
    if let Some(stop) = crate::policy::refusal_in(err) {
        // GitHub #727: a refusal of the policy comes before any connect.
        line.push_str(&format!("\npodnet: {}", stop.hint()));
    } else if let Some(hint) = pod_tls::clock_hint(&format!("{err:#} {err:?}")) {
        line.push_str("\npodnet: ");
        line.push_str(&hint);
    } else if let Some(status) = crate::egress::refusal(err)
        .map(|r| r.status)
        .filter(|s| *s != 0)
    {
        if (502..=504).contains(&status) {
            // Issue 178: a 502, 503 or 504 of the proxy is the destination:
            // the proxy could not reach it.
            line.push_str(&format!(
                "\npodnet: the proxy answered HTTP {status}: it could not reach the destination; check the address of the destination (--masque-endpoint, --url, the edge)"
            ));
        } else if status == 407 {
            // T-629: 407 is proxy authentication.
            line.push_str(
                "\npodnet: the proxy hop failed; the proxy requires authentication (HTTP 407); give the user and password in --proxy or PODNET_PROXY, or use --no-proxy",
            );
        } else {
            // T-629: 403/405/etc. is a CONNECT refusal by the proxy's rules.
            line.push_str(&format!(
                "\npodnet: the proxy hop failed; the proxy refused CONNECT (HTTP {status}); check the port allow-list and host allow-list of the proxy, or use --no-proxy"
            ));
        }
    } else if let Some(refusal) = crate::hop::refusal_of(err) {
        // The class of a refused registration (live 45169011007): a quick
        // tunnel that the edge does not know yet is not a bad token.
        line.push_str(&format!(
            "\npodnet: the {} hop failed; {}",
            crate::hop::Hop::EdgeRegister,
            refusal.hint()
        ));
    } else if crate::exit::code_of(err) == crate::exit::TEMPFAIL {
        // A refusal for now (HTTP 429 with Retry-After): a retry after the
        // wait works, so the hint of the hop ("does not change with a
        // retry") would be false (live 44140229441).
        line.push_str(
            "\npodnet: the service refuses for now; try again after the time that it asks for (exit 75)",
        );
    } else if crate::hop::quick_service_answer(err) {
        // GitHub #695: the service answered with a page that is not its
        // API; no registration with the edge ran.
        line.push_str(&format!(
            "\npodnet: the quick-tunnel service did not answer with its API: a proxy or a gateway on the way can answer with its own page; check the proxy and its allow-list for the host of the service ({} or the host of --service-url); a named tunnel (--token) does not use this service",
            quick_service_host()
        ));
    } else if let Some(issuer) = crate::hop::door_issuer(&format!("{err:#}")) {
        // GitHub #715: the key of MASQUE is pinned; the issuer of the
        // certificate that the door showed tells the cause.
        let hint = crate::hop::door_cert(issuer).hint();
        line.push_str(&format!("\npodnet: the MASQUE door failed; {hint}"));
    } else if let Some(hop) = crate::hop::of(err) {
        line.push_str(&format!("\npodnet: the {hop} hop failed; {}", hop.hint()));
    }
    line
}

/// The text of `err` and of each cause, joined with `: `, as `{:#}`
/// gives it. A cause whose text is in the line already is not named again:
/// a failure of a crate of podcrates holds the text of its causes
/// (`pod_core::Failure::chain`).
/// A part that comes again at once, and the Debug form of a name of a
/// certificate, are said once and plainly (`crate::exit::tidy`).
pub fn chain_text(err: &anyhow::Error) -> String {
    crate::exit::text(err)
}

/// The host of the quick-tunnel service of `pod-services`.
fn quick_service_host() -> String {
    let url = pod_cloudflare::quicktunnel::default_quick_service();
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', ':']).next())
        .unwrap_or("the quick-tunnel service")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The second line of each class of failure that has its own hint: a
    /// refusal for now says to wait, not that a retry does not help.
    #[test]
    fn the_hint_follows_the_class_of_the_failure() {
        let cases = [
            (
                pod_core::Class::TempFail,
                "try again after the time",
                "the tunnel did not register with the edge",
            ),
            (
                pod_core::Class::Refused,
                "the tunnel did not register with the edge",
                "try again after the time",
            ),
        ];
        for (class, has, lacks) in cases {
            let failure = pod_core::Failure::new(pod_core::Stage::EdgeRegister, class, "HTTP 429");
            let err = anyhow::Error::new(failure);
            let line = error_line(&err);
            assert!(line.contains(has), "{class:?}: {line}");
            assert!(!line.contains(lacks), "{class:?}: {line}");
        }
    }

    /// GitHub #715: a MASQUE door that showed a certificate of a middlebox
    /// gives no `--ca-file` step (the key is pinned): the issuer names the
    /// class and its step.
    #[test]
    fn a_masque_door_failure_names_the_class_of_its_certificate() {
        let cases = [
            (
                "O=Anthropic, CN=Egress Gateway SDS Issuing CA (production)",
                "middlebox",
            ),
            ("C=US, O=Let's Encrypt, CN=YE1", "try again later"),
            (
                "C=US, O=Cloudflare, Inc., CN=2024-02-27 Self-Signed Root",
                "warp register",
            ),
        ];
        for (issuer, has) in cases {
            let text = format!("TLS handshake with the MASQUE front door 162.159.198.2:443: unexpected error: MASQUE peer key does not match the pinned key; the door showed a certificate of {issuer}");
            let failure =
                pod_core::Failure::new(pod_core::Stage::Tls, pod_core::Class::Unavailable, text);
            let line = error_line(&anyhow::Error::new(failure));
            assert!(line.contains(has), "{issuer}: {line}");
            assert!(
                !line.contains("give its root with --ca-file"),
                "{issuer}: {line}"
            );
        }
    }

    /// Live 45169011007: each kind of refused registration gives a true
    /// second line and its exit code. A quick tunnel that the edge does not
    /// know yet, and a refusal with retry: yes, are not a bad token.
    #[test]
    fn a_refused_registration_gives_the_hint_and_the_code_of_its_kind() {
        use crate::hop::TunnelKind::{Named, Quick};
        // As pod-cloudflare v0.82.0 joins the failures of the legs: the
        // typed refusal is not in the chain.
        let joined = |said: &[(&str, bool)]| {
            let legs: Vec<String> = said
                .iter()
                .map(|(cause, retry)| {
                    let retry = if *retry { "yes" } else { "no" };
                    format!("quic: the edge refused the registration: {cause} (retry: {retry}, after 1 s; podnet speaks cloudflared 2026.11.1)")
                })
                .collect();
            anyhow::Error::new(pod_core::Failure::new(
                pod_core::Stage::EdgeRegister,
                pod_core::Class::Unavailable,
                format!("no edge leg worked: {}", legs.join("; ")),
            ))
        };
        let typed = |cause: &str, retry: bool| {
            anyhow::Error::new(pod_core::Failure::from(pod_cloudflare::fail::Refused {
                client: "podnet".into(),
                cause: cause.into(),
                should_retry: retry,
                retry_after: std::time::Duration::from_secs(1),
            }))
        };
        // As pod-cloudflare v0.82.1 joins the failures of the legs: the
        // text of each leg, and the typed refusal of the last leg in the
        // chain. The class is TempFail when each leg said retry: yes.
        let last_typed = |said: &[(&str, bool)]| {
            let text = joined(said).to_string();
            let class = if said.iter().all(|(_, retry)| *retry) {
                pod_core::Class::TempFail
            } else {
                pod_core::Class::Unavailable
            };
            let (cause, retry) = said.last().copied().unwrap();
            anyhow::Error::new(pod_core::Failure::caused(
                pod_core::Stage::EdgeRegister,
                class,
                text,
                pod_core::Failure::from(pod_cloudflare::fail::Refused {
                    client: "podnet".into(),
                    cause: cause.into(),
                    should_retry: retry,
                    retry_after: std::time::Duration::from_secs(1),
                }),
            ))
        };
        let not_found = "Unauthorized: Tunnel not found";
        let bad_token = "Unauthorized: Invalid tunnel secret";
        let not_a_refusal = || {
            anyhow::Error::new(pod_core::Failure::new(
                pod_core::Stage::EdgeRegister,
                pod_core::Class::Unavailable,
                "no edge leg worked: quic: the registration did not end in 30 s",
            ))
        };
        let wrong = ["a refused token or tunnel", "use a quick tunnel"];
        // (the error, the kind of the tunnel, the hint has, the exit code)
        let cases = [
            (
                joined(&[(not_found, true)]),
                Quick,
                "does not know the new quick tunnel yet",
                75,
            ),
            (
                joined(&[(not_found, true), (not_found, true)]),
                Quick,
                "does not know the new quick tunnel yet",
                75,
            ),
            (
                joined(&[(not_found, false)]),
                Quick,
                "does not know the new quick tunnel yet",
                75,
            ),
            (
                joined(&[(not_found, true)]),
                Named,
                "the edge asks for a retry",
                75,
            ),
            (
                typed(not_found, true),
                Named,
                "the edge asks for a retry",
                75,
            ),
            (
                joined(&[(not_found, false)]),
                Named,
                "check that the named tunnel exists",
                69,
            ),
            (
                typed(not_found, false),
                Named,
                "check that the named tunnel exists",
                77,
            ),
            (
                joined(&[(bad_token, false)]),
                Named,
                "check the token or the credentials file, or use a quick tunnel",
                69,
            ),
            (
                joined(&[(bad_token, false)]),
                Quick,
                "start podnet again to get a new quick tunnel",
                69,
            ),
            (
                joined(&[(bad_token, false), (not_found, true)]),
                Named,
                "check that the named tunnel exists",
                69,
            ),
            (
                last_typed(&[(bad_token, false), (not_found, true)]),
                Named,
                "check that the named tunnel exists",
                69,
            ),
            (
                last_typed(&[(not_found, true), (not_found, true)]),
                Named,
                "the edge asks for a retry",
                75,
            ),
            (
                not_a_refusal(),
                Quick,
                "the tunnel did not register with the edge",
                69,
            ),
        ];
        for (err, kind, has, code) in cases {
            let err = crate::hop::read_refusal(err, kind);
            let line = error_line(&err);
            let hint = line.lines().nth(1).unwrap_or_default();
            assert!(
                hint.starts_with("podnet: the edge-register hop failed; "),
                "{kind:?}: {line}"
            );
            assert!(hint.contains(has), "{kind:?}: {line}");
            assert_eq!(crate::exit::code_of(&err), code, "{kind:?}: {line}");
            // A retry can work: no word says that it cannot. A quick
            // tunnel is not told to use a quick tunnel.
            if code == 75 || kind == Quick {
                for wrong in wrong {
                    assert!(!hint.contains(wrong), "{kind:?}: {line}");
                }
            }
            if code == 75 {
                assert!(
                    !hint.contains("does not change with a retry"),
                    "{kind:?}: {line}"
                );
            }
        }
    }

    /// A failure of a crate that holds the text of its cause is not said
    /// twice.
    #[test]
    fn a_cause_is_said_once() {
        let inner = pod_core::Failure::new(
            pod_core::Stage::Network,
            pod_core::Class::Unavailable,
            "reset",
        );
        let outer = pod_core::Failure::caused(
            pod_core::Stage::Network,
            pod_core::Class::Unavailable,
            "POST https://x/tunnel: reset",
            inner,
        );
        let err = anyhow::Error::new(outer).context("provisioning quick tunnel");
        assert_eq!(
            chain_text(&err),
            "provisioning quick tunnel: POST https://x/tunnel: reset"
        );
        let plain = anyhow::anyhow!("a").context("b");
        assert_eq!(chain_text(&plain), format!("{plain:#}"));
    }

    /// GitHub #695: an answer of the quick-tunnel service that is not its
    /// API gets the hint of the service, not the hint of a registration
    /// with a token.
    #[test]
    fn an_answer_that_is_not_the_api_has_the_hint_of_the_service() {
        use pod_cloudflare::quicktunnel::ProvisionError;
        let cases = [
            ProvisionError::Decode("expected value at line 1 column 1".into()),
            ProvisionError::Service(Vec::new()),
            ProvisionError::MissingResult,
        ];
        for case in cases {
            let err = anyhow::Error::new(pod_core::Failure::from(case))
                .context("provisioning quick tunnel");
            let line = error_line(&err);
            let hint = line.lines().nth(1).unwrap_or_default();
            assert!(
                hint.starts_with("podnet: the quick-tunnel service did not answer with its API"),
                "{line}"
            );
            assert!(!line.contains("edge-register"), "{line}");
            assert!(!line.contains("credentials"), "{line}");
        }
    }

    // Issue 178: a 502 of the proxy names the destination, not the proxy.
    #[test]
    fn a_502_of_the_proxy_names_the_destination() {
        let err = anyhow::Error::new(crate::egress::Refusal {
            status: 502,
            text: "the proxy refused CONNECT with HTTP 502".into(),
        });
        let err = crate::hop::at(crate::hop::Hop::Proxy, err);
        let line = error_line(&err);
        assert!(line.contains("could not reach the destination"), "{line}");
        assert!(!line.contains("allow-list"), "{line}");
    }

    #[test]
    fn the_error_line_has_no_control_characters() {
        let peer = "bad gateway\u{1b}[2J\u{1b}]0;owned\u{7}\u{7} from\rthe edge\u{9b}31m\u{7f}";
        let err = anyhow::anyhow!("{peer}").context("registering with the edge");
        let line = error_line(&err);
        assert_eq!(
            line,
            "podnet: registering with the edge: bad gateway fromthe edge31m"
        );
        for c in ['\u{1b}', '\u{7}', '\r', '\u{9b}', '\u{7f}'] {
            assert!(!line.contains(c), "{c:?} in {line:?}");
        }
        let doc = document(None, 1, Some(&err));
        assert!(!doc.to_string().contains("\\u001b"), "{doc}");
        assert_eq!(clean("a\tb\nc"), "a\tb\nc");
    }
}
