//! A Shadowsocks server as a network: `ss://HOST:PORT` (T-127).
//!
//! The server takes the target of each stream from the address in its first
//! chunk and dials it. The methods are the AEAD form
//! (`chacha20-ietf-poly1305`, `aes-256-gcm`, `aes-128-gcm`) and Shadowsocks
//! 2022 (`2022-blake3-aes-128-gcm`, `2022-blake3-aes-256-gcm`,
//! `2022-blake3-chacha20-poly1305`). podnet has no code of this road: it is
//! `shadowsocks` of `pod-egress`, which refuses a plugin and a key of several
//! users by name.
//!
//! * The key is a setting, not a part of the spec: `PODNET_SHADOWSOCKS_KEY_FILE`
//!   (a file whose one line is `METHOD:PASSWORD`, or the `ss://` URI of the
//!   server, as an access key gives it) or `PODNET_SHADOWSOCKS_KEY` (the same
//!   text). A URI must name the host and the port of the spec. A key in the
//!   spec is a usage error that does not repeat it.
//! * The network does not wait for the answer of the server: a client may
//!   speak first. A server that does not take the key says nothing and ends
//!   the stream.

use std::sync::Arc;

use pod_egress::shadowsocks::{self, Method, Node};
use pod_tls::Trust;

use super::super::secret::{self, Get};
use super::super::usage;
use super::super::via::{probe_as, Labeled};
use super::STEP;
use crate::network::{Failure, Link, Target, Way};

/// The variable of the file of the key.
pub const KEY_FILE_VARIABLE: &str = "PODNET_SHADOWSOCKS_KEY_FILE";
/// The variable of the key. The file is the better choice.
pub const KEY_VARIABLE: &str = "PODNET_SHADOWSOCKS_KEY";

/// The form of the spec, for each line of a bad spec: the text of a bad spec
/// is not shown, as a whole `ss://` URI holds the password.
const FORM: &str = "ss://HOST:PORT";

/// A server that a spec names, with no key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::networks) struct Spec {
    host: String,
    port: u16,
}

impl Spec {
    /// `ss://HOST:PORT`.
    pub(in crate::networks) fn label(&self) -> String {
        format!("ss://{}", pod_proxy::host::authority(&self.host, self.port))
    }

    /// The host that the network must reach first.
    pub(in crate::networks) fn target(&self) -> Target {
        Target::new(&self.host, self.port)
    }
}

/// True when `spec` is a Shadowsocks spec.
pub(super) fn is_shadowsocks(spec: &str) -> bool {
    spec.get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("ss://"))
}

/// Parse `spec`. No error repeats a part of the spec but the form.
pub(super) fn parse(spec: &str) -> Result<Spec, Failure> {
    let bad = |why: &str| usage(format!("a Shadowsocks spec is {FORM}: {why}"));
    let rest = spec[5..].trim_end_matches('/');
    if rest.contains(['/', '?', '#']) {
        return Err(bad("it has no path, no query and no tag"));
    }
    if rest.contains('@') {
        return Err(bad(&format!(
            "the key goes in {KEY_FILE_VARIABLE} or {KEY_VARIABLE}, not in the spec"
        )));
    }
    let (host, port) = match pod_proxy::host::split_host_port(rest) {
        Ok((host, Some(port))) if !host.is_empty() && port != 0 => (host, port),
        _ => return Err(bad("give the host and a port of 1 to 65535")),
    };
    if pod_core::names::check_target(&host).is_err() {
        return Err(bad("the host is not a host name or an IP address"));
    }
    Ok(Spec { host, port })
}

/// The server of the spec with the key of the settings, and the form that
/// gave the key.
pub(in crate::networks) struct Given {
    node: Node,
    form: &'static str,
}

/// Read the key with `get`, for the router network `name`. The key is
/// required; a bad key is named by its variable, never shown.
pub(super) fn settings(get: Get<'_>, name: Option<&str>, spec: &Spec) -> Result<Given, Failure> {
    let vars = [KEY_FILE_VARIABLE, KEY_VARIABLE];
    let suffix = secret::suffix_of(name, get, &vars);
    let file_var = format!("{KEY_FILE_VARIABLE}{suffix}");
    let value_var = format!("{KEY_VARIABLE}{suffix}");
    let label = spec.label();
    let Some(given) = secret::read(get, &file_var, &value_var)? else {
        return Err(usage(format!(
            "{label}: give the key of the Shadowsocks server: set {file_var} (a file with METHOD:PASSWORD) or {value_var}"
        )));
    };
    let variable = &given.variable;
    let text = given.text.trim();
    if text.contains(['\n', '\r']) {
        return Err(usage(format!(
            "{variable}: the key is one line, METHOD:PASSWORD or an ss:// URI"
        )));
    }
    let node = if is_shadowsocks(text) {
        let mut node = Node::parse(text).map_err(|why| usage(format!("{variable}: {why}")))?;
        if node.port != spec.port || !node.host.eq_ignore_ascii_case(&spec.host) {
            return Err(usage(format!(
                "{variable}: the ss:// URI names another server than {label}"
            )));
        }
        // No tag of the key: the lines name the server of the spec alone.
        node.tag = None;
        node
    } else {
        let Some((method, password)) = text.split_once(':') else {
            return Err(usage(format!(
                "{variable}: the key is METHOD:PASSWORD or an ss:// URI"
            )));
        };
        // The words of the crate show a method that it does not know; a
        // password before the colon would be shown, so the line names the
        // methods alone.
        let method = Method::parse(method).map_err(|_| {
            usage(format!(
                "{variable}: the method before the colon is not one of {}",
                Method::names()
            ))
        })?;
        Node::new(method, password, &spec.host, spec.port)
            .map_err(|why| usage(format!("{variable}: {why}")))?
    };
    Ok(Given {
        node,
        form: given.form.word(),
    })
}

/// Bring the network up through `way`. One TCP connection to the server
/// proves its reach; the key goes with each stream.
pub(super) async fn up(
    spec: &Spec,
    given: Given,
    way: pod_ws::Way,
    trust: Trust,
) -> Result<Link, Failure> {
    let label = spec.label();
    let what = "the Shadowsocks server";
    probe_as(&way, &trust, &spec.target(), None, false, &label, what).await?;
    // The status holds no password: the method and the form of the key.
    let status = serde_json::json!({
        "scheme": "shadowsocks",
        "spec": label,
        "method": given.node.method.name(),
        "key": given.form,
    });
    let road = shadowsocks::Road {
        node: given.node,
        dial: shadowsocks::Dial {
            way,
            trust,
            timeout: STEP,
            // A client may speak first: the stream is taken at once.
            answer: None,
            silence_is_ready: true,
        },
    };
    let dial = Arc::new(Labeled {
        label,
        inner: Arc::new(road),
    });
    Ok(Link::new(Some(Way::destination(dial))).with_status(status))
}
