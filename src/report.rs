//! The report of a failure (`podnet_core::report`, at its old path), and
//! the line that podnet prints for an error. The line reads the refusal of
//! a proxy ([`crate::egress`]), so it stays in this crate.

#[doc(inline)]
pub use podnet_core::report::*;

/// The line that podnet prints for the error `err`: masked and clean.
pub fn error_line(err: &anyhow::Error) -> String {
    let mut line = format!("podnet: {}", clean(&mask(&chain_text(err))));
    if let Some(hint) = pod_tls::clock_hint(&format!("{err:#} {err:?}")) {
        line.push_str("\npodnet: ");
        line.push_str(&hint);
    } else if let Some(status) = crate::egress::refusal(err)
        .map(|r| r.status)
        .filter(|s| (502..=504).contains(s))
    {
        // Issue 178: a 502, 503 or 504 of the proxy is the destination:
        // the proxy could not reach it.
        line.push_str(&format!(
            "\npodnet: the proxy answered HTTP {status}: it could not reach the destination; check the address of the destination (--masque-endpoint, --url, the edge)"
        ));
    } else if crate::exit::code_of(err) == crate::exit::TEMPFAIL {
        // A refusal for now (HTTP 429 with Retry-After): a retry after the
        // wait works, so the hint of the hop ("does not change with a
        // retry") would be false (live 44140229441).
        line.push_str(
            "\npodnet: the service refuses for now; try again after the time that it asks for (exit 75)",
        );
    } else if let Some(hop) = crate::hop::of(err) {
        line.push_str(&format!("\npodnet: the {hop} hop failed; {}", hop.hint()));
    }
    line
}

/// The text of `err` and of each cause, joined with `: `, as `{:#}`
/// gives it. A cause whose text is in the line already is not named again:
/// a failure of a crate of podcrates holds the text of its causes
/// (`pod_core::Failure::chain`).
pub fn chain_text(err: &anyhow::Error) -> String {
    let mut out = String::new();
    for cause in err.chain() {
        let text = cause.to_string();
        if out.is_empty() {
            out = text;
        } else if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
    }
    out
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
                "does not change with a retry",
            ),
            (
                pod_core::Class::Refused,
                "does not change with a retry",
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
