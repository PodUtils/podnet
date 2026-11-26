//! An HTTPS proxy as a network: `https://[USER@]HOST[:PORT]` (T-126).
//!
//! The network gives TCP through a forward proxy that is reached over TLS
//! (Caddy's forwardproxy, squid's `https_port`, h2o). Each [`Dial::dial`]
//! opens TLS to the proxy, then a CONNECT inside the TLS: HTTP/2 when the
//! proxy chooses `h2` by ALPN (the tunnels share one connection), else
//! HTTP/1.1. podnet has no code of this road: it is `via` of `pod-egress`.
//!
//! * The default port is 443. A path, a query or a password in the URL is a
//!   usage error that does not repeat the password.
//! * The credentials are a setting, not a part of the spec:
//!   `PODNET_HTTPS_PROXY_AUTH_FILE` (a file whose one line is
//!   `USER:PASSWORD`, or the password alone when the URL names the user) or
//!   `PODNET_HTTPS_PROXY_AUTH` (the same line). With neither, no
//!   `Proxy-Authorization` goes.
//! * `PODNET_HTTPS_PROXY_VERSION` is the versions of HTTP that the client
//!   offers: `auto` (default), `1.1` or `2`.
//! * A network of a router file reads the variables with the suffix of its
//!   name when one of them is set ([`super::secret::suffix_of`]).
//!
//! The TCP to the proxy goes through the way out of the process, or through
//! the way below the network. The roots are those of podnet (`--ca-file`).

use std::sync::Arc;
use std::time::Duration;

use pod_egress::via::{self, Offer, Via};
use pod_tls::Trust;

use super::secret::{self, Get};
use super::usage;
use crate::network::{BoxFuture, Connected, Dial, Failure, Link, Target, Way};

/// The variable of the file of the credentials (one line).
pub const AUTH_FILE_VARIABLE: &str = "PODNET_HTTPS_PROXY_AUTH_FILE";
/// The variable of the credentials line. The file is the better choice.
pub const AUTH_VARIABLE: &str = "PODNET_HTTPS_PROXY_AUTH";
/// The variable of the versions of HTTP that the client offers.
pub const VERSION_VARIABLE: &str = "PODNET_HTTPS_PROXY_VERSION";

/// The bound of each step of a dial: the TCP, TLS, a CONNECT.
pub(super) const STEP: Duration = Duration::from_secs(20);

/// A proxy that a spec names, with no credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Spec {
    pub(super) via: Via,
}

impl Spec {
    /// `https://HOST:PORT`, with no user.
    pub(super) fn label(&self) -> String {
        self.via.shown()
    }

    /// The host that the network must reach first.
    pub(super) fn target(&self) -> Target {
        Target::new(&self.via.host, self.via.port)
    }
}

/// True when `spec` is an HTTPS proxy.
pub(super) fn is_via(spec: &str) -> bool {
    spec.get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
}

/// Parse `spec`. The error names the spec with no user and no password.
pub(super) fn parse(spec: &str) -> Result<Spec, Failure> {
    Via::parse(spec)
        .map(|via| Spec { via })
        .map_err(|why| usage(format!("{}: {why}", secret::bare(spec))))
}

/// The settings of the network: the credentials and the versions offered.
pub(super) struct Settings {
    credentials: Option<secret::Given>,
    offer: Offer,
    file_var: String,
    auth_var: String,
}

/// Read the settings with `get`, for the router network `name`.
pub(super) fn settings(get: Get<'_>, name: Option<&str>) -> Result<Settings, Failure> {
    let vars = [AUTH_FILE_VARIABLE, AUTH_VARIABLE, VERSION_VARIABLE];
    let suffix = secret::suffix_of(name, get, &vars);
    let file_var = format!("{AUTH_FILE_VARIABLE}{suffix}");
    let auth_var = format!("{AUTH_VARIABLE}{suffix}");
    let version_var = format!("{VERSION_VARIABLE}{suffix}");
    let credentials = secret::read(get, &file_var, &auth_var)?;
    let offer = match get(&version_var).filter(|v| !v.is_empty()) {
        Some(text) => Offer::parse(&text).map_err(|why| usage(format!("{version_var}: {why}")))?,
        None => Offer::Both,
    };
    Ok(Settings {
        credentials,
        offer,
        file_var,
        auth_var,
    })
}

/// The proxy of `spec` with the credentials of `settings`. A URL that names
/// a user needs the password; a line that the crate refuses is named by its
/// variable, never shown.
fn with_credentials(spec: &Spec, settings: &Settings) -> Result<Via, Failure> {
    let label = spec.label();
    match &settings.credentials {
        Some(given) => spec.via.clone().with_credentials(&given.text).map_err(|why| {
            usage(format!("{label}: {}: {why}", given.variable))
        }),
        None if spec.via.names_user() => Err(usage(format!(
            "{label}: the URL names a user, and no password is given: set {} (a file with the password) or {}",
            settings.file_var, settings.auth_var
        ))),
        None => Ok(spec.via.clone()),
    }
}

/// The user agent of each connection of a road.
pub(super) fn agent() -> String {
    format!("podnet/{}", env!("CARGO_PKG_VERSION"))
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
/// the CA of its own stand. One TLS handshake with the proxy proves its
/// reach and its certificate; the credentials go with each CONNECT.
pub(super) async fn up_with(
    spec: &Spec,
    settings: Settings,
    below: Option<&Arc<dyn Dial>>,
    tls: Arc<pod_tls::Context>,
    trust: Trust,
) -> Result<Link, Failure> {
    let label = spec.label();
    let via = with_credentials(spec, &settings)?;
    let dial = below.cloned().unwrap_or_else(crate::dialers::process);
    let way = pod_ws::Way::new(tls, dial, agent());
    probe(&way, &trust, &spec.target(), true, &label, "the proxy").await?;
    // The status holds no user and no password: the form alone.
    let status = serde_json::json!({
        "scheme": "https",
        "spec": label,
        "credentials": settings.credentials.as_ref().map_or("none", |g| g.form.word()),
        "offer": match settings.offer {
            Offer::Both => "auto",
            Offer::Http1 => "1.1",
            Offer::Http2 => "2",
        },
    });
    let client = via::Client::new(
        via,
        via::Dial {
            way,
            trust,
            timeout: STEP,
            offer: settings.offer,
            auth_file: settings.file_var.clone(),
        },
    );
    let dial = Arc::new(Labeled {
        label,
        inner: Arc::new(client),
    });
    Ok(Link::new(Some(Way::destination(dial))).with_status(status))
}

/// One connection to the server of a road, as `up` proves it: TLS for its
/// name when `tls`, else TCP. The fault starts with `label` and names the
/// hop (`what`).
pub(super) async fn probe(
    way: &pod_ws::Way,
    trust: &Trust,
    target: &Target,
    tls: bool,
    label: &str,
    what: &str,
) -> Result<(), Failure> {
    probe_as(way, trust, target, None, tls, label, what).await
}

/// [`probe`] with the TLS name `name` (the `sni` of a VLESS link), else
/// the host of `target`.
pub(super) async fn probe_as(
    way: &pod_ws::Way,
    trust: &Trust,
    target: &Target,
    name: Option<&str>,
    tls: bool,
    label: &str,
    what: &str,
) -> Result<(), Failure> {
    let (host, port) = (target.host(), target.port());
    let name = name.map_or_else(|| host.to_string(), str::to_string);
    let failed = |e: pod_ws::ConnectError| {
        let (stage, class) = e.stage_and_class();
        let why = match &e {
            pod_ws::ConnectError::Tls(why) => format!("TLS: {why}"),
            other => other.to_string(),
        };
        Failure::caused(stage, class, format!("{label}: {what}: {why}"), e)
    };
    if tls {
        way.open_tls(&host, port, &name, trust, STEP, &[])
            .await
            .map_err(failed)?;
    } else {
        way.tcp(&host, port, STEP)
            .await
            .map_err(|e| failed(pod_ws::ConnectError::Dial(e)))?;
    }
    Ok(())
}

/// The dialer of a road under the label of its spec (`https://HOST:PORT`,
/// `wstunnel:wss://HOST`), as each network of the registry is named.
pub(super) struct Labeled {
    pub(super) label: String,
    pub(super) inner: Arc<dyn Dial>,
}

impl Dial for Labeled {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn proxy_label(&self, target: &Target) -> Option<String> {
        self.inner.proxy_label(target)
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.inner.dial(target, deadline)
    }

    fn names_inside(&self) -> bool {
        self.inner.names_inside()
    }
}
