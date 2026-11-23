//! `podnet cover`: the TLS listener that answers as a static site until a
//! client proves that it holds the key (T-369, GitHub #491).

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::Args;

use podnet::cover::{self, Config};
use podnet::exit::{self, Fault};

#[derive(Args, Debug)]
pub(crate) struct CoverArgs {
    /// The address of the listener. Give a loopback address unless other
    /// hosts must reach the cover.
    #[arg(long, value_name = "ADDR")]
    listen: SocketAddr,
    /// The directory of the cover site: index.html, its assets, robots.txt
    /// and 404.html. A tree with no index.html exits 78.
    #[arg(long, value_name = "DIR")]
    site: PathBuf,
    /// The cover script, as the TOML of a measurement with its date and the
    /// version of the cover. A missing script, or one with no date, exits 78
    /// at load (SCRIPT-2 and SCRIPT-3).
    #[arg(long, value_name = "FILE")]
    script: PathBuf,
    /// The certificate chain of the site, in PEM.
    #[arg(long, value_name = "FILE")]
    cert: PathBuf,
    /// The P-256 key of the certificate, in PEM.
    #[arg(long, value_name = "FILE")]
    key: PathBuf,
    /// The key of the switch: 64 hexadecimal characters. Both ends hold it.
    #[arg(long, value_name = "FILE")]
    switch_key: PathBuf,
    /// The target of an authenticated session, HOST:PORT.
    #[arg(long, value_name = "HOST:PORT")]
    to: String,
    /// The name of the site, for the routing of the handshake's name. An
    /// unknown name gets the default site's own 404 page.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// The value of the `Server` header. The default is `nginx`. A value
    /// that names this project is refused.
    #[arg(long, value_name = "VALUE")]
    server: Option<String>,
    /// Carry a page load and a session over one connection: the page
    /// completes first and both lanes share one budget (MUX-1 and MUX-2).
    #[arg(long)]
    mux: bool,
}

fn config_fault(text: String) -> anyhow::Error {
    exit::fault(Fault::Config, anyhow!(text))
}

fn read(path: &PathBuf, what: &str) -> Result<Vec<u8>> {
    std::fs::read(path)
        .map_err(|e| config_fault(format!("{what} {}: {e}", path.display())))
        .with_context(|| what.to_string())
}

/// `podnet cover`.
pub(crate) async fn cover(args: CoverArgs) -> Result<()> {
    let cert = read(&args.cert, "the certificate")?;
    let key = read(&args.key, "the key")?;
    let switch_text = String::from_utf8(read(&args.switch_key, "the switch key")?)
        .map_err(|_| config_fault("the switch key is not text".into()))?;
    let switch_key = cover::key_from_hex(&switch_text)?;
    let config = Config {
        site: args
            .site
            .to_str()
            .ok_or_else(|| config_fault("the site path is not UTF-8".into()))?
            .to_string(),
        name: args.name.clone(),
        script: args
            .script
            .to_str()
            .ok_or_else(|| config_fault("the script path is not UTF-8".into()))?
            .to_string(),
        switch_key,
        target: args.to.clone(),
        server: args.server.clone(),
        mux: args.mux,
    };
    let cover = cover::Cover::load(config)?;
    println!(
        "cover     {}  site {}  script dated {} (cover {})  target {}  mux {}",
        args.listen,
        args.site.display(),
        cover.script().date,
        cover.script().cover_version,
        args.to,
        if args.mux { "on" } else { "off" }
    );
    cover::run(args.listen, cert, key, std::sync::Arc::new(cover)).await
}
