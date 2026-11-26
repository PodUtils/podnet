//! `--acme DOMAIN --listen tcp://ADDR:PORT` of `serve` and `git serve`
//! (T-129): the router over HTTPS on a public TCP listener, with a
//! certificate for DOMAIN from an ACME CA (`podnet::acme`, the crate
//! `pod-acme`).

use anyhow::Result;
use clap::Args;

use podnet::exit::{Fault, ResultExt};

use crate::cli::cmd::local::Local;

/// The flags of a public HTTPS listener with a certificate of ACME.
#[derive(Args, Debug, Default)]
pub(crate) struct AcmeOpts {
    /// Serve HTTPS on --listen with a certificate for DOMAIN from an ACME
    /// CA (Let's Encrypt by default). DOMAIN must point to this host, and the
    /// CA must reach TCP port 443 of DOMAIN: the listener answers the
    /// challenge TLS-ALPN-01 itself. podnet keeps the account and the
    /// certificate in acme/ of the state directory (mode 0700, files 0600):
    /// a restart asks the CA for nothing while the certificate holds.
    /// podnet renews it 30 days before its end. Off by default.
    #[arg(long, value_name = "DOMAIN", requires = "listen")]
    pub(crate) acme: Option<String>,
    /// With --acme: the TCP listener of HTTPS, tcp://ADDR:PORT (for example
    /// tcp://0.0.0.0:443). Another port needs a forward of port 443 to it.
    #[arg(long, value_name = "tcp://ADDR:PORT", requires = "acme")]
    pub(crate) listen: Option<String>,
    /// With --acme: the directory URL of the ACME CA (https://). Default:
    /// the directory of Let's Encrypt. --ca-file gives the roots of a test
    /// CA.
    #[arg(long, value_name = "URL", requires = "acme")]
    pub(crate) acme_directory: Option<String>,
}

impl AcmeOpts {
    /// The name of --acme, checked (exit 64 for a name that TLS-ALPN-01
    /// cannot prove), or `None` with no --acme.
    pub(crate) fn domain(&self) -> Result<Option<String>> {
        self.acme
            .as_deref()
            .map(podnet::acme::check_domain)
            .transpose()
    }

    /// The listener of --listen, as a row of `--check`.
    pub(crate) fn rows(&self) -> Vec<String> {
        self.listen.iter().cloned().collect()
    }
}

/// Serve `router` over HTTPS on --listen with the certificate of --acme,
/// or `None` with no --acme. The listener runs before the first order, as
/// the CA connects to it. A failure of the first order exits with its code
/// and one line that names the CA and the step (`podnet::acme::failure`).
pub(crate) async fn bind_acme(opts: &AcmeOpts, router: axum::Router) -> Result<Option<Local>> {
    let Some(name) = opts.domain()? else {
        return Ok(None);
    };
    let Some(listen) = opts.listen.as_deref() else {
        return Err(podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!("--acme needs --listen tcp://ADDR:PORT"),
        ));
    };
    let places = podnet::acme::places(podnet::state::state_dir().fault(Fault::Config)?)?;
    let client = pod_proxy::Client::new(
        podnet::trust::context().clone(),
        podnet::egress::egress(None),
    )
    .with_user_agent(format!("podnet/{}", env!("CARGO_PKG_VERSION")));
    let http = podnet::acme::Http {
        client,
        trust: podnet::trust::ws_trust(),
        timeout: podnet::acme::REQUEST,
    };
    // The listener first: a bad --listen makes no state.
    let (listener, bound) = podnet::acme::bind(listen).await?;
    let acme = podnet::acme::open(&name, opts.acme_directory.as_deref(), http, &places)?;
    let served = podnet::acme::Served::new(None);
    let acceptor = podnet::acme::acceptor(served.clone())?;
    let server = tokio::spawn(podnet::acme::serve_tls(listener, acceptor, router));
    tracing::info!(
        kind = "acme.start",
        "--acme {name}: HTTPS on tcp://{bound}, the certificate from {}",
        acme.host()
    );
    let (_, keep) = match podnet::acme::start(acme, served).await {
        Ok(started) => started,
        Err(err) => {
            server.abort();
            return Err(err);
        }
    };
    // The keep task ends only when no certificate is served: not after the
    // start. The server ends with an error of its listener.
    let run = tokio::spawn(async move {
        tokio::select! {
            ended = server => ended.unwrap_or_else(|e| Err(std::io::Error::other(e))),
            failed = keep => Err(std::io::Error::other(match failed {
                Ok(failed) => failed.to_string(),
                Err(e) => e.to_string(),
            })),
        }
    });
    Ok(Some(Local::tcp(
        format!("https://{name}/"),
        format!("tcp://{bound}"),
        run,
    )))
}
