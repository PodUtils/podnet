//! `podnet serve`: the file server, through a tunnel, through the relay,
//! or on local listeners only.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};

use podnet::config::Config;
use podnet::exit::{Fault, ResultExt};
use podnet::files::{Access, FilesConfig, Role, Users};
use podnet::gate::Gate;
use podnet::ingress::{Ingress, Service};

use crate::cli::cmd::local::{bind_local, Local};
use crate::cli::cmd::run::{run_share, run_tunnel};
use crate::cli::cmd::tunnel::preflight;
use crate::cli::opts::{Show, TunnelOpts};
use crate::cli::signals::sleep_or_signal;

/// The road of the files to the visitors (`--via`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Via {
    /// A Cloudflare tunnel: a quick tunnel, or the named tunnel of --token.
    Tunnel,
    /// A node of a relay pair (podnet relay pair); the operator reaches
    /// it with podnet relay connect NAME --listen ADDR.
    Relay,
    /// Only the listeners of --local.
    None,
}

#[derive(Args, Debug)]
pub(crate) struct ServeArgs {
    #[command(flatten)]
    pub(crate) common: TunnelOpts,
    /// Directory to serve.
    #[arg(value_name = "DIR", conflicts_with = "dir")]
    path: Option<PathBuf>,
    /// Directory to serve (the same as DIR).
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
    /// Single-page-app fallback to index.html.
    #[arg(long)]
    spa: bool,
    /// Give a directory as one tar download: GET /PATH/?tar answers the
    /// directory PATH as a ustar stream (application/x-tar). Names that
    /// start with a dot and links out of the directory are left out.
    #[arg(long)]
    tar: bool,
    /// A directory with no index.html answers a listing of its files:
    /// HTML, or JSON with the header Accept: application/json. Names
    /// that start with a dot and links out of the directory are left out.
    #[arg(long)]
    list: bool,
    /// Accept uploads into the directory, for a write user (or the PIN):
    /// PUT /PATH (curl -T FILE URL/PATH) writes one file, and a multipart
    /// form POST to a directory adds files. A file goes to its name only
    /// when it is complete. Names that start with a dot are refused.
    #[arg(long)]
    upload: bool,
    /// The most bytes of one upload request. A larger request gets 413
    /// and leaves no file. The Cloudflare edge accepts at most 100 MB in
    /// one request.
    #[arg(long, value_name = "BYTES", requires = "upload", default_value_t = podnet::share::MAX_REQUEST, value_parser = clap::value_parser!(u64).range(1..))]
    max_upload: u64,
    /// The users of the file server (HTTP Basic), one on each line:
    /// USER:ROLE:HASH or USER:ROLE:@PASSWORD-FILE. ROLE is read or write.
    /// HASH comes from --hash-password. The file must have mode 0600.
    /// Each request must give a user, except a read with --anon-read.
    #[arg(long, value_name = "FILE")]
    users: Option<PathBuf>,
    /// Lock the files with a 6-digit PIN, as share --password does. podnet
    /// prints the PIN. With no --users, the PIN gives the write role.
    #[arg(long)]
    pin: bool,
    /// A GET or HEAD needs no user and no PIN. A change still needs a
    /// write user.
    #[arg(long)]
    anon_read: bool,
    /// Read a password from the first line of stdin, print its hash for
    /// --users, and exit.
    #[arg(long, conflicts_with_all = ["path", "dir", "users", "pin", "anon_read", "local", "via"])]
    hash_password: bool,
    /// The road to the visitors: tunnel (the default with no --local),
    /// relay, or none (the default with --local). --via tunnel or relay
    /// with --local serves the local listeners too.
    #[arg(long, value_enum)]
    via: Option<Via>,
    /// With --via relay: the pair name. Default: the pair with the most
    /// time left; at its expiry the node takes the next stored pair.
    #[arg(long, value_name = "NAME")]
    pair: Option<String>,
    /// Serve the files on this local listener (more than one is
    /// permitted): HOST:PORT (or tcp://HOST:PORT), or unix:PATH (mode
    /// 0600). Port 0 takes a free port; podnet prints the bound address. A
    /// unix socket that answers is refused (exit 69, --replace takes it
    /// over), a stale socket is removed, a file that is not a socket is
    /// refused (exit 64). With no --via, podnet makes no tunnel.
    #[arg(long, value_name = "ADDR", conflicts_with = "check")]
    local: Vec<String>,
    /// Named-tunnel token. Prefer the variable TUNNEL_TOKEN or
    /// --token-file: other users of the host can read a command line.
    #[arg(long, env = "TUNNEL_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Read the named-tunnel token from this file (for example a secret
    /// of a container). The token is not on the command line.
    #[arg(
        long,
        value_name = "PATH",
        env = "TUNNEL_TOKEN_FILE",
        hide_env_values = true
    )]
    token_file: Option<PathBuf>,
}

impl ServeArgs {
    /// DIR or --dir.
    fn dir(&self) -> &Path {
        self.path
            .as_deref()
            .or(self.dir.as_deref())
            .expect("serve checks DIR first")
    }
}

pub(crate) async fn serve(args: ServeArgs) -> Result<()> {
    if args.hash_password {
        return hash_password();
    }
    let usage = |msg: &str| podnet::exit::fault(Fault::Usage, anyhow::anyhow!("{msg}"));
    if args.path.is_none() && args.dir.is_none() {
        return Err(usage(
            "serve: give the directory to serve: podnet serve DIR",
        ));
    }
    let via = args.via.unwrap_or(if args.local.is_empty() {
        Via::Tunnel
    } else {
        Via::None
    });
    if via == Via::None && args.local.is_empty() {
        return Err(usage("serve --via none needs --local ADDR"));
    }
    if args.pair.is_some() && via != Via::Relay {
        return Err(usage("serve --pair is for --via relay"));
    }
    if args.common.check && via != Via::Tunnel {
        return Err(usage(
            "serve --check checks a tunnel: it is for --via tunnel",
        ));
    }
    let mut files = files_config(&args)?;
    // T-089: the file server checks the user of --auth-user, with the
    // write role, on each listener; the tunnel does not check it.
    if let (Some(user), Some(file)) = (&args.common.auth_user, &args.common.auth_password_file) {
        let credentials = podnet::ingress::credentials(user, file)
            .context("--auth-password-file")
            .fault(Fault::Config)?;
        add_user(&mut files.access, credentials)?;
    }
    let router = podnet::files::router(&files)?;
    let pin = files.access.pin.as_ref().and_then(Gate::pin_display);
    let show = args.common.show();
    let mut locals = Vec::new();
    for local in &args.local {
        locals.push(bind_local(local, args.common.detach.replace, router.clone()).await?);
    }
    match via {
        Via::None => serve_local(&args, locals, pin).await,
        Via::Relay => {
            announce_locals(show, args.dir(), &locals);
            crate::cli::cmd::serve_relay::serve_relay(
                args.pair.as_deref(),
                router,
                args.dir(),
                show,
                pin,
                args.common.run_for,
            )
            .await
        }
        Via::Tunnel => {
            announce_locals(show, args.dir(), &locals);
            serve_tunnel(&args, files).await
        }
    }
}

/// `serve --via tunnel`: the files through a Cloudflare tunnel.
async fn serve_tunnel(args: &ServeArgs, files: FilesConfig) -> Result<()> {
    let mut config = Config::default();
    if let Some(token) = &args.token {
        podnet::report::hide(token);
        config.credentials = podnet::config::parse_token(token).fault(Fault::Config)?;
    }
    config.ingress = Ingress::new().catch_all(Service::Files(files.clone()));
    config = args.common.apply(config)?;
    // The file server has the user of --auth-user (T-089).
    config.ingress.auth = None;
    let mut show = serve_show(args.common.show(), args.dir(), args.spa || args.list);
    show.auth = files.access.users.is_some() && !files.access.anon_read;
    if args.common.check {
        return preflight(&config, show).await;
    }
    if let Some(pin) = files.access.pin.as_ref().and_then(Gate::pin_display) {
        let pin = Some(pin.to_string());
        return run_share(config, show, args.common.run_for, pin, "/".into(), 0).await;
    }
    run_tunnel(config, show, args.common.run_for, vec![]).await
}

/// One line for each local listener beside the tunnel or the relay.
fn announce_locals(show: Show, dir: &Path, locals: &[Local]) {
    for local in locals {
        if show.json {
            let doc = serde_json::json!({
                "event": "listen",
                "url": local.url,
                "listen": local.listen,
                "dir": dir.display().to_string(),
            });
            println!("{}", podnet::status::stamped(doc));
        } else {
            println!(
                "podnet: serve  {}  on {}  (local)",
                dir.display(),
                local.url
            );
        }
    }
}

/// `--hash-password`: the hash of the first line of stdin.
fn hash_password() -> Result<()> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading the password from stdin")?;
    let password = line.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        return Err(podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!("--hash-password: give the password on the first line of stdin"),
        ));
    }
    println!("{}", podnet::files::hash_password(password));
    Ok(())
}

/// Add the user of `--auth-user` with the write role.
fn add_user(access: &mut Access, credentials: pod_proxy::Credentials) -> Result<()> {
    access
        .users
        .get_or_insert_with(Users::default)
        .add(credentials, Role::Write)
        .fault(Fault::Usage)
}

/// The file server of `serve`.
fn files_config(args: &ServeArgs) -> Result<FilesConfig> {
    let users = match &args.users {
        Some(path) => Some(Users::load(path).context("--users").fault(Fault::Config)?),
        None => None,
    };
    if args.anon_read && users.is_none() && !args.pin && args.common.auth_user.is_none() {
        return Err(podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!("--anon-read needs --users, --auth-user or --pin"),
        ));
    }
    Ok(FilesConfig {
        spa: args.spa,
        tar: args.tar,
        list: args.list,
        upload: args.upload.then_some(args.max_upload),
        access: Access {
            users,
            pin: args.pin.then(Gate::pin),
            anon_read: args.anon_read,
        },
        ..FilesConfig::new(args.dir())
    })
}

/// `show` for `serve`: a directory with no `index.html`, and no `--spa`,
/// answers 404 for `/`. That is its normal answer (T-202).
fn serve_show(show: Show, dir: &Path, spa: bool) -> Show {
    Show {
        root_404: !spa && !dir.join("index.html").is_file(),
        ..show
    }
}

/// `serve --local ADDR` (issue 307): serve the files on local listeners,
/// with the router of the tunnel, and provision no tunnel. T-089: the
/// users, the PIN and --auth-user are checked on the local listener too.
/// The ready line names the first listener; `listens` names each.
async fn serve_local(args: &ServeArgs, locals: Vec<Local>, pin: Option<&str>) -> Result<()> {
    let show = args.common.show();
    let (url, listen) = (locals[0].url.clone(), locals[0].listen.clone());
    let listens: Vec<&str> = locals.iter().map(|l| l.listen.as_str()).collect();
    podnet::detach::arm_stop();
    if show.json {
        let doc = serde_json::json!({
            "event": "ready",
            "url": url,
            "listen": listen,
            "listens": listens,
            "dir": args.dir().display().to_string(),
            "pin": pin,
        });
        println!("{}", podnet::status::stamped(doc));
    } else {
        for local in &locals {
            println!(
                "podnet: serve  {}  on {}  (local, no tunnel)",
                args.dir().display(),
                local.url
            );
        }
        if let Some(pin) = pin {
            println!("podnet: PIN    {pin}");
        }
    }
    // A QR code of a unix socket has no use.
    if show.qr && url.starts_with("http://") {
        print!("{}", podnet::qr::render_terminal(&url)?);
    }
    podnet::detach::ready(serde_json::json!({
        "url": url,
        "listen": listen,
        "listens": listens,
        "pin": pin,
    }));
    let mut servers = tokio::task::JoinSet::new();
    let mut files = Vec::new();
    for local in locals {
        let (server, file) = local.split();
        files.push(file);
        servers.spawn(server);
    }
    tokio::select! {
        _ = sleep_or_signal(args.common.run_for) => {}
        Some(ended) = servers.join_next() => {
            ended
                .context("the local server stopped")?
                .context("the local server stopped")?
                .context("the local server stopped")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_show_root_404() {
        let dir = tempfile::tempdir().unwrap();
        let show = Show::default();
        assert!(serve_show(show, dir.path(), false).root_404);
        assert!(!serve_show(show, dir.path(), true).root_404);
        std::fs::write(dir.path().join("index.html"), "x").unwrap();
        assert!(!serve_show(show, dir.path(), false).root_404);
    }
}
