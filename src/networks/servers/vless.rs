//! A VLESS server as a network (T-127):
//! `vless://HOST:PORT?type=ws|xhttp&security=tls|none&sni=&host=&path=&mode=`.
//!
//! The spec is the share link of Xray-core with no ID: the server (Xray-core,
//! sing-box, shoes) takes the target of each stream from VLESS's header and
//! dials it. The stream goes over a WebSocket (`type=ws`) or over XHTTP
//! (`type=xhttp`, on HTTP/2, or on HTTP/1.1 with two connections). podnet has
//! no code of this road: it is `vless` of `pod-egress`, which refuses by name
//! each setting that it does not speak (`type=tcp`, a flow, REALITY).
//!
//! * The ID is a setting, not a part of the spec: `PODNET_VLESS_ID_FILE` (a
//!   file with the ID: a UUID, or a word of 1 to 30 bytes as Xray-core maps
//!   it) or `PODNET_VLESS_ID` (the same text). An ID in the spec is a usage
//!   error that does not repeat it.
//! * The network does not wait for the answer of the server: a client may
//!   speak first. A server that refuses the ID ends each stream before its
//!   answer, and the read of the stream gives the words of `pod-egress`.

use std::sync::Arc;
use std::time::Duration;

use pod_egress::vless::{self, AtEnd, Id, Server};
use pod_tls::Trust;

use super::super::secret::{self, Get};
use super::super::usage;
use super::super::via::{probe_as, Labeled};
use super::STEP;
use crate::network::{Failure, Link, Target, Way};

/// The variable of the file of the ID.
pub const ID_FILE_VARIABLE: &str = "PODNET_VLESS_ID_FILE";
/// The variable of the ID. The file is the better choice.
pub const ID_VARIABLE: &str = "PODNET_VLESS_ID";

/// The ID of the parse: the spec has none, and `up` puts the ID of the
/// setting in its place.
const NO_ID: &str = "podnet-no-id";

/// A server that a spec names, with no ID.
#[derive(Clone, Debug)]
pub(in crate::networks) struct Spec {
    pub(in crate::networks) server: Server,
}

impl Spec {
    /// `vless://HOST:PORT/PATH`: no ID and no query.
    pub(in crate::networks) fn label(&self) -> String {
        self.server.shown()
    }

    /// The host that the network must reach first.
    pub(in crate::networks) fn target(&self) -> Target {
        Target::new(&self.server.host, self.server.port)
    }
}

/// True when `spec` is a VLESS spec.
pub(super) fn is_vless(spec: &str) -> bool {
    spec.get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("vless://"))
}

/// Parse `spec`. The error names the spec with no ID, no path and no query.
pub(super) fn parse(spec: &str) -> Result<Spec, Failure> {
    let shown = secret::bare(spec);
    let rest = &spec[8..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if rest[..end].contains('@') {
        return Err(usage(format!(
            "{shown}: the ID goes in {ID_FILE_VARIABLE} or {ID_VARIABLE}, not in the spec; give vless://HOST:PORT?type=ws&security=tls&path=/PATH"
        )));
    }
    Server::parse(&format!("vless://{NO_ID}@{rest}"))
        .map(|server| Spec { server })
        .map_err(|why| usage(format!("{shown}: {why}")))
}

/// The ID of the settings, and the form that gave it.
pub(in crate::networks) struct Given {
    id: Id,
    form: &'static str,
}

/// Read the ID with `get`, for the router network `name`. The ID is
/// required; a bad ID is named by its variable, never shown.
pub(super) fn settings(get: Get<'_>, name: Option<&str>, spec: &Spec) -> Result<Given, Failure> {
    let vars = [ID_FILE_VARIABLE, ID_VARIABLE];
    let suffix = secret::suffix_of(name, get, &vars);
    let file_var = format!("{ID_FILE_VARIABLE}{suffix}");
    let value_var = format!("{ID_VARIABLE}{suffix}");
    let Some(given) = secret::read(get, &file_var, &value_var)? else {
        return Err(usage(format!(
            "{}: give the ID of the VLESS user: set {file_var} (a file with the ID) or {value_var}",
            spec.label()
        )));
    };
    let id =
        Id::parse(given.text.trim()).map_err(|why| usage(format!("{}: {why}", given.variable)))?;
    Ok(Given {
        id,
        form: given.form.word(),
    })
}

/// Bring the network up through `way`. One connection to the server (TLS
/// for its name over `security=tls`) proves its reach and its certificate;
/// the ID goes with each stream.
pub(super) async fn up(
    spec: &Spec,
    given: Given,
    way: pod_ws::Way,
    trust: Trust,
) -> Result<Link, Failure> {
    let label = spec.label();
    let mut server = spec.server.clone();
    server.id = given.id;
    let what = "the VLESS server";
    let name = server.sni.clone();
    probe_as(
        &way,
        &trust,
        &spec.target(),
        Some(&name),
        server.tls,
        &label,
        what,
    )
    .await?;
    // The status holds no ID: its form alone.
    let status = serde_json::json!({
        "scheme": "vless",
        "spec": label,
        "transport": server.transport.name(),
        "tls": server.tls,
        "id": given.form,
    });
    let road = vless::Road {
        server,
        dial: vless::Dial {
            way,
            trust,
            timeout: STEP,
            // A client may speak first: the stream is taken at once, and
            // VLESS's answer is read before the first bytes of the target.
            answer: Duration::ZERO,
            silence_is_ready: true,
            // A pipe: the target may still answer after the end of input.
            at_end: AtEnd::AfterPeer,
            early: None,
        },
    };
    let dial = Arc::new(Labeled {
        label,
        inner: Arc::new(road),
    });
    Ok(Link::new(Some(Way::destination(dial))).with_status(status))
}
