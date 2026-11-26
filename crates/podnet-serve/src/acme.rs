//! ACME for a public listener (T-129): `serve --acme DOMAIN --listen
//! tcp://ADDR:443`, and `git serve` with the same flags.
//!
//! The client is `pod-acme` of PodUtils/podcrates (RFC 8555). This module
//! holds the parts of podnet:
//!
//! * the check of the name ([`check_domain`]);
//! * the place of the state: the state directory ([`places`]). The crate
//!   keeps `acme/HOST-DIGEST/account.key` and `acme/HOST-DIGEST/NAME.pem`
//!   there: each directory has mode 0700, each file mode 0600;
//! * the TLS listener ([`serve_tls`]), which answers the challenge of
//!   TLS-ALPN-01 (RFC 8737) itself, so that port 443 is the only port;
//! * the start ([`start`]): a kept certificate that holds is served at once
//!   and podnet asks the CA for nothing; else the first order runs now. Then
//!   the crate renews each certificate before its end;
//! * the exit code of each failure ([`failure`]).
//!
//! The events of the crate (`acme.account`, `acme.issued`, `acme.renewal`)
//! go to the log of podnet (`podnet_core::decisions::report`).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use pod_tls::server::{ACME_TLS_1, HTTP1};
use podnet_core::exit::{self, Fault};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub use pod_acme::certs::Leaf;
pub use pod_acme::client::Http;
pub use pod_acme::{about, lets_encrypt, renew_at, utc, Acme, Failed, REQUEST};
pub use pod_tls::server::Served;

/// The limit of one TLS handshake of a visitor or of the CA.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// How often [`start`] looks for the first certificate.
const LOOK: Duration = Duration::from_millis(200);
/// The port that the CA connects to for TLS-ALPN-01 (RFC 8737, section 3).
pub const CA_PORT: u16 = 443;

/// `given` for `--acme`, in lower case and with no last dot. A name that
/// cannot get a certificate by TLS-ALPN-01 is refused (exit 64): an IP
/// address, a wildcard, a name with no dot, a label that is not a DNS
/// label.
pub fn check_domain(given: &str) -> Result<String> {
    let name = given
        .strip_suffix('.')
        .unwrap_or(given)
        .to_ascii_lowercase();
    let refuse =
        |why: String| exit::fault(Fault::Usage, anyhow::anyhow!("--acme {given:?}: {why}"));
    if name.is_empty() {
        return Err(refuse(
            "give a DNS name, for example files.example.com".into(),
        ));
    }
    if name.contains('*') {
        return Err(refuse(
            "TLS-ALPN-01 cannot prove a wildcard name: give one DNS name".into(),
        ));
    }
    let bare = name.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return Err(refuse(
            "this is an IP address: give a DNS name that points to this host".into(),
        ));
    }
    if name.len() > 253 {
        return Err(refuse("a DNS name has at most 253 characters".into()));
    }
    let labels: Vec<&str> = name.split('.').collect();
    if labels.len() < 2 {
        return Err(refuse(
            "a public CA needs a name with a dot, for example files.example.com".into(),
        ));
    }
    for label in &labels {
        let ok = (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !ok {
            return Err(refuse(format!(
                "{label:?} is not a DNS label: use letters, digits and '-' (1 to 63 characters, no '-' at an end)"
            )));
        }
    }
    if labels
        .last()
        .is_some_and(|top| top.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(refuse(
            "the last label has only digits: give a DNS name".into(),
        ));
    }
    Ok(name)
}

/// The places of the state of ACME: the state directory `state`. With no
/// state directory (`PODNET_STATE_DIR=none`), `--acme` is refused (exit
/// 78): a restart must find the certificate, else each start asks the CA
/// again and meets its rate limits.
pub fn places(state: Option<PathBuf>) -> Result<Vec<PathBuf>> {
    match state {
        Some(dir) => Ok(vec![dir]),
        None => Err(exit::fault(
            Fault::Config,
            anyhow::anyhow!(
                "--acme keeps its account and its certificate in the state directory, and {}=none keeps no file: set {0} to a directory",
                podnet_core::state::STATE_DIR_VARIABLE
            ),
        )),
    }
}

/// The client of `name` at the CA of `directory` (Let's Encrypt with
/// `None`), through `http`, its state in `places`, its events to the log of
/// podnet.
pub fn open(name: &str, directory: Option<&str>, http: Http, places: &[PathBuf]) -> Result<Acme> {
    let events = podnet_core::decisions::report().events;
    Acme::new(name, directory, http, places, events).map_err(|failed| match failed {
        Failed::Directory(_) => failure(failed).context("--acme-directory"),
        failed => failure(failed),
    })
}

/// A failure of the crate with its exit code: 64 for a directory URL that
/// is not `https://`, 71 for a state that cannot be kept or read, 69 for a
/// CA that refused or did not answer, 124 for an order past its limit. The
/// line keeps the words of the crate: the CA, and the step (the
/// directory, the account, the order, the challenge, the certificate).
pub fn failure(failed: Failed) -> anyhow::Error {
    let fault = match &failed {
        Failed::Directory(_) => Fault::Usage,
        Failed::Store(_) => Fault::OsError,
        Failed::Ca(_) => Fault::Unavailable,
        Failed::Late(_) => Fault::Timeout,
    };
    exit::fault(fault, anyhow::Error::new(pod_core::Failure::from(failed)))
}

/// The TCP listener of `--listen`: `tcp://HOST:PORT` or `HOST:PORT`. A
/// unix socket is refused (exit 64): the CA connects over TCP. An address in
/// use, or a port below 1024 with no right to bind it, exits 69 with the next
/// step.
pub async fn bind(listen: &str) -> Result<(TcpListener, SocketAddr)> {
    let usage =
        |why: String| exit::fault(Fault::Usage, anyhow::anyhow!("--listen {listen}: {why}"));
    if listen.starts_with("unix:") {
        return Err(usage(format!(
            "--acme needs a TCP listener (tcp://0.0.0.0:{CA_PORT}): the CA connects over TCP"
        )));
    }
    let addr = listen.strip_prefix("tcp://").unwrap_or(listen);
    let Ok(parsed) = addr.parse::<SocketAddr>() else {
        return Err(usage(format!(
            "give tcp://ADDR:PORT, for example tcp://0.0.0.0:{CA_PORT}"
        )));
    };
    let listener = TcpListener::bind(parsed)
        .await
        .with_context(|| format!("--listen {listen}"))
        .map_err(podnet_core::net::bind_fault)?;
    let bound = listener.local_addr()?;
    if bound.port() != CA_PORT {
        tracing::warn!(
            "--listen {listen}: the CA connects to port {CA_PORT} of the name: forward port {CA_PORT} to {bound}"
        );
    }
    Ok((listener, bound))
}

/// The TLS of the listener: what `served` holds at each handshake, with
/// ALPN `http/1.1` for a visitor and `acme-tls/1` for the CA.
pub fn acceptor(served: Arc<Served>) -> Result<TlsAcceptor> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    pod_tls::server::acceptor_for(provider, served, &[HTTP1, ACME_TLS_1])
        .map_err(|e| exit::fault(Fault::Config, anyhow::anyhow!("the TLS of --acme: {e}")))
}

/// Serve `service` over TLS on each connection of `listener`, until the task
/// ends. A handshake that offers `acme-tls/1` alone is the validation of a CA:
/// it gets the certificate of the challenge, and the connection closes at the
/// end of the handshake (RFC 8737, section 3). A handshake that fails, or
/// that takes more than 10 s, closes its connection only.
pub async fn serve_tls(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    service: axum::Router,
) -> std::io::Result<()> {
    loop {
        let tcp = match listener.accept().await {
            Ok((tcp, _)) => tcp,
            // EMFILE and its kin pass: wait a moment, and accept again.
            Err(err) => {
                tracing::debug!(error = %err, "--acme: an accept failed");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let (acceptor, service) = (acceptor.clone(), service.clone());
        tokio::spawn(async move {
            let tls = match tokio::time::timeout(HANDSHAKE, acceptor.accept(tcp)).await {
                Ok(Ok(tls)) => tls,
                Ok(Err(err)) => {
                    tracing::debug!(error = %err, "--acme: a TLS handshake failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!("--acme: a TLS handshake took more than 10 s");
                    return;
                }
            };
            if tls.get_ref().1.alpn_protocol() == Some(ACME_TLS_1) {
                return;
            }
            crate::road::serve_connection(tls, service).await;
        });
    }
}

/// The certificate of `acme` on `served`, and the task that keeps it.
///
/// A kept certificate that names the name and holds now is served at once,
/// and podnet asks the CA for nothing. A kept file that does not load is
/// said in the log, and a new order replaces it. Else the first order runs
/// now: the listener that serves `served` must run, as the CA connects to
/// it. A failure of the first order is the error (with [`failure`]). The
/// task renews each certificate before its end (`pod_acme::keep`).
pub async fn start(
    acme: Acme,
    served: Arc<Served>,
) -> Result<(Leaf, tokio::task::JoinHandle<Failed>)> {
    let now = pod_acme::order::now_secs();
    let held = match acme.held(now) {
        Ok(held) => held,
        Err(why) => {
            tracing::warn!(
                kind = "acme.held",
                "the kept certificate does not load ({why}); podnet orders a new one"
            );
            None
        }
    };
    if let Some((pair, leaf)) = held {
        served.set(pair);
        tracing::info!(
            kind = "acme.held",
            "the kept certificate {}; renewed from {}",
            about(&leaf),
            utc(renew_at(&leaf))
        );
        let keep = tokio::spawn(pod_acme::keep(acme, served, Some(leaf.clone())));
        return Ok((leaf, keep));
    }
    let look = acme.clone();
    let mut keep = tokio::spawn(pod_acme::keep(acme, served.clone(), None));
    loop {
        tokio::select! {
            ended = &mut keep => {
                let failed = ended.map_err(|e| exit::fault(
                    Fault::Unavailable,
                    anyhow::anyhow!("ACME at {}: the task of the order ended: {e}", look.host()),
                ))?;
                return Err(failure(failed));
            }
            () = tokio::time::sleep(LOOK) => {}
        }
        if served.current().is_some() {
            if let Ok(Some((_, leaf))) = look.held(pod_acme::order::now_secs()) {
                return Ok((leaf, keep));
            }
        }
    }
}

#[cfg(test)]
mod tests;
