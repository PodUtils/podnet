//! A wstunnel server as a network: `wstunnel:wss://HOST[:PORT]` (T-126).
//!
//! The network gives TCP through a server of erebe/wstunnel (9.1 to 11)
//! that the user runs (`wstunnel server wss://[::]:443`). Each
//! [`Dial::dial`] opens a WebSocket to the server with a token that names
//! the target; the server dials the target before it answers. podnet has no
//! code of this road: it is `wstunnel` of `pod-egress`.
//!
//! * `wstunnel:wss://HOST[:PORT]` (TLS, default port 443),
//!   `wstunnel:ws://HOST[:PORT]` (no TLS, default port 80) and
//!   `wstunnel:HOST[:PORT]` (the same as `wss://`). The URL has no path: the
//!   path prefix is the secret of the server.
//! * The secret is a setting, not a part of the spec:
//!   `PODNET_WSTUNNEL_SECRET_FILE` (a file with the lines `path-prefix
//!   PREFIX`, `credentials USER[:PASSWORD]` or `authorization VALUE`, or one
//!   line with the prefix alone) or `PODNET_WSTUNNEL_SECRET` (the same text).
//!   With neither, the client sends the defaults of wstunnel (the prefix
//!   `v1`, no `Authorization`). A secret over `ws://` to a host that is not
//!   the loopback is a usage error.
//! * A network of a router file reads the variables with the suffix of its
//!   name when one of them is set ([`super::secret::suffix_of`]).
//! * `wstunnel:PORT` is still the SOCKS5 hop of a host named `wstunnel`.

use std::sync::Arc;

use pod_egress::wstunnel::{self, AtEnd, Secret, Server, Tunnel};
use pod_tls::Trust;

use super::secret::{self, Get};
use super::usage;
use super::via::{agent, probe, Labeled, STEP};
use crate::network::{Dial, Failure, Link, Target, Way};

/// The variable of the file of the secret.
pub const SECRET_FILE_VARIABLE: &str = "PODNET_WSTUNNEL_SECRET_FILE";
/// The variable of the secret. The file is the better choice.
pub const SECRET_VARIABLE: &str = "PODNET_WSTUNNEL_SECRET";

/// A server that a spec names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Spec {
    pub(super) server: Server,
}

impl Spec {
    /// `wstunnel:wss://HOST[:PORT]`.
    pub(super) fn label(&self) -> String {
        format!("wstunnel:{}", self.server.shown())
    }

    /// The host that the network must reach first.
    pub(super) fn target(&self) -> Target {
        Target::new(self.server.host(), self.server.port())
    }
}

/// True when `spec` is a wstunnel spec. `wstunnel:PORT` is the SOCKS5 hop
/// of a host named `wstunnel`, as it was.
pub(super) fn is_wstunnel(spec: &str) -> bool {
    spec.strip_prefix("wstunnel:")
        .is_some_and(|rest| rest.parse::<u16>().is_err())
}

/// Parse `spec`. The error names the spec with no user and no password.
pub(super) fn parse(spec: &str) -> Result<Spec, Failure> {
    let rest = spec.strip_prefix("wstunnel:").unwrap_or(spec);
    let shown = secret::bare(spec);
    if rest.is_empty() {
        return Err(usage(format!(
            "{shown}: give the server, wstunnel:wss://HOST[:PORT]"
        )));
    }
    let url = if rest.contains("://") {
        rest.to_string()
    } else if rest.starts_with("//") {
        format!("wss:{rest}")
    } else {
        format!("wss://{rest}")
    };
    Server::parse(&url)
        .map(|server| Spec { server })
        .map_err(|why| usage(format!("{shown}: {why}")))
}

/// The settings of the network: the secret and its form.
pub(super) struct Settings {
    secret: Secret,
    form: &'static str,
}

/// Read the settings with `get`, for the router network `name`. A secret
/// that does not parse is named by its variable and the number of its line.
pub(super) fn settings(get: Get<'_>, name: Option<&str>) -> Result<Settings, Failure> {
    let vars = [SECRET_FILE_VARIABLE, SECRET_VARIABLE];
    let suffix = secret::suffix_of(name, get, &vars);
    let file_var = format!("{SECRET_FILE_VARIABLE}{suffix}");
    let value_var = format!("{SECRET_VARIABLE}{suffix}");
    match secret::read(get, &file_var, &value_var)? {
        Some(given) => Ok(Settings {
            secret: Secret::parse(&given.text)
                .map_err(|why| usage(format!("{}: {why}", given.variable)))?,
            form: given.form.word(),
        }),
        None => Ok(Settings {
            secret: Secret::default(),
            form: "default",
        }),
    }
}

/// Bring the network up with the roots of podnet.
pub(super) async fn up(
    spec: &Spec,
    settings: Settings,
    below: Option<&Arc<dyn Dial>>,
) -> Result<Link, Failure> {
    let tls = crate::trust::context().clone();
    up_with(spec, settings, below, tls, crate::trust::ws_trust()).await
}

/// [`up`] with the TLS context and the trust given, so that a test can trust
/// the CA of its own stand. One connection to the server (TLS for `wss://`)
/// proves its reach and its certificate; the secret goes with each upgrade.
pub(super) async fn up_with(
    spec: &Spec,
    settings: Settings,
    below: Option<&Arc<dyn Dial>>,
    tls: Arc<pod_tls::Context>,
    trust: Trust,
) -> Result<Link, Failure> {
    let label = spec.label();
    wstunnel::check(&spec.server, &settings.secret)
        .map_err(|why| usage(format!("{label}: {why}")))?;
    let dial = below.cloned().unwrap_or_else(crate::dialers::process);
    let way = pod_ws::Way::new(tls, dial, agent());
    let target = spec.target();
    probe(
        &way,
        &trust,
        &target,
        spec.server.tls(),
        &label,
        "the wstunnel server",
    )
    .await?;
    // The status holds no secret: its form alone.
    let status = serde_json::json!({
        "scheme": "wstunnel",
        "spec": label,
        "tls": spec.server.tls(),
        "secret": settings.form,
    });
    let tunnel = Tunnel {
        server: spec.server.clone(),
        secret: settings.secret,
        dial: wstunnel::Dial {
            way,
            trust,
            timeout: STEP,
            // A pipe: the target may still answer after the end of input.
            at_end: AtEnd::AfterPeer,
        },
    };
    let dial = Arc::new(Labeled {
        label,
        inner: Arc::new(tunnel),
    });
    Ok(Link::new(Some(Way::destination(dial))).with_status(status))
}
