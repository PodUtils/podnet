//! The report of a failure (`podnet_core::report`, at its old path), and
//! the line that podnet prints for an error. The line reads the refusal of
//! a proxy ([`crate::egress`]), so it stays in this crate.

#[doc(inline)]
pub use podnet_core::report::*;

/// The line that podnet prints for the error `err`: masked and clean.
pub fn error_line(err: &anyhow::Error) -> String {
    let mut line = format!("podnet: {}", clean(&mask(&format!("{err:#}"))));
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
    } else if let Some(hop) = crate::hop::of(err) {
        line.push_str(&format!("\npodnet: the {hop} hop failed; {}", hop.hint()));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

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
