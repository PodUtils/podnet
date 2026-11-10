//! `podnet serve`: the file server, through a tunnel or on local
//! listeners.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use podnet::config::Config;
use podnet::exit::{Fault, ResultExt};
use podnet::files::{Access, FilesConfig, Role, Users};
use podnet::gate::Gate;
use podnet::ingress::{Ingress, Service};

use crate::cli::cmd::connect::{unix_listener, SocketFile};
use crate::cli::cmd::run::{run_share, run_tunnel};
use crate::cli::cmd::tunnel::preflight;
use crate::cli::opts::{Show, TunnelOpts};
use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
pub(crate) struct ServeArgs {
    #[command(flatten)]
    pub(crate) common: TunnelOpts,
    /// Directory to serve.
    #[arg(long, value_name = "DIR", required_unless_present = "hash_password")]
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
    #[arg(long, conflicts_with_all = ["dir", "users", "pin", "anon_read", "local"])]
    hash_password: bool,
    /// Serve the files on this local listener and make no tunnel:
    /// HOST:PORT (or tcp://HOST:PORT), or unix:PATH (mode 0600). Port 0
    /// takes a free port; podnet prints the bound address. A unix socket
    /// that answers is refused (exit 69, --replace takes it over), a
    /// stale socket is removed, a file that is not a socket is refused
    /// (exit 64). The tunnel flags have no effect.
    #[arg(long, value_name = "ADDR", conflicts_with = "check")]
    local: Option<String>,
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
    fn dir(&self) -> &std::path::Path {
        self.dir.as_deref().expect("clap requires --dir")
    }
}

pub(crate) async fn serve(args: ServeArgs) -> Result<()> {
    if args.hash_password {
        return hash_password();
    }
    let mut files = files_config(&args)?;
    if let Some(local) = &args.local {
        return serve_local(&args, files, local).await;
    }
    let mut config = Config::default();
    if let Some(token) = &args.token {
        podnet::report::hide(token);
        config.credentials = podnet::config::parse_token(token).fault(Fault::Config)?;
    }
    config.ingress = Ingress::new().catch_all(Service::Files(files.clone()));
    config = args.common.apply(config)?;
    // T-089: the file server checks the user of --auth-user, with the
    // write role, on each listener; the tunnel does not check it.
    if let Some(credentials) = config.ingress.auth.take() {
        add_user(&mut files.access, credentials)?;
        for rule in &mut config.ingress.rules {
            rule.service = Service::Files(files.clone());
        }
    }
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
fn serve_show(show: Show, dir: &std::path::Path, spa: bool) -> Show {
    Show {
        root_404: !spa && !dir.join("index.html").is_file(),
        ..show
    }
}

/// A router on a local listener.
pub(crate) struct Local {
    /// `http://HOST:PORT/` or `unix:PATH`.
    pub(crate) url: String,
    /// `tcp://HOST:PORT` or `unix:PATH`.
    pub(crate) listen: String,
    pub(crate) server: tokio::task::JoinHandle<std::io::Result<()>>,
    /// The socket file, removed on drop.
    _file: Option<SocketFile>,
}

/// Serve `router` on the local listener `local` (issue 307): HOST:PORT,
/// tcp://HOST:PORT or unix:PATH.
pub(crate) async fn bind_local(local: &str, replace: bool, router: axum::Router) -> Result<Local> {
    let usage = |msg: String| podnet::exit::fault(Fault::Usage, anyhow::anyhow!(msg));
    if local.starts_with("unix:") {
        let path = match podnet::vnet::proxy::ProxyListen::parse(local) {
            Ok(podnet::vnet::proxy::ProxyListen::Unix(path)) => path,
            Ok(_) => unreachable!("a unix: address is a unix socket"),
            Err(err) => return Err(usage(format!("--local {local}: {err:#}"))),
        };
        let (listener, file) = unix_listener(&path, replace)?;
        let name = format!("unix:{}", path.display());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        return Ok(Local {
            url: name.clone(),
            listen: name,
            server,
            _file: Some(file),
        });
    }
    let addr = local.strip_prefix("tcp://").unwrap_or(local);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(err) if !addr.contains(':') || err.kind() == std::io::ErrorKind::InvalidInput => {
            return Err(usage(format!(
                "--local {local}: give HOST:PORT (for example 127.0.0.1:8080) or unix:PATH ({err})"
            )));
        }
        // An address in use, or an address of no interface: exit 69.
        Err(err) => {
            return Err(podnet::exit::fault(
                Fault::Unavailable,
                anyhow::anyhow!("--local {local}: binding {addr}: {err}"),
            ));
        }
    };
    let bound = listener.local_addr()?;
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    Ok(Local {
        url: format!("http://{bound}/"),
        listen: format!("tcp://{bound}"),
        server,
        _file: None,
    })
}

/// `serve --local ADDR` (issue 307): serve the files on a local listener,
/// with the router of the tunnel, and provision no tunnel. T-089: the
/// users, the PIN and --auth-user are checked on the local listener too.
async fn serve_local(args: &ServeArgs, mut files: FilesConfig, local: &str) -> Result<()> {
    if let (Some(user), Some(file)) = (&args.common.auth_user, &args.common.auth_password_file) {
        let credentials = podnet::ingress::credentials(user, file)
            .context("--auth-password-file")
            .fault(Fault::Config)?;
        add_user(&mut files.access, credentials)?;
    }
    let router = podnet::files::router(&files)?;
    let show = args.common.show();
    let local = bind_local(local, args.common.detach.replace, router).await?;
    let (url, listen) = (local.url.clone(), local.listen.clone());
    let pin = files.access.pin.as_ref().and_then(Gate::pin_display);
    podnet::detach::arm_stop();
    if show.json {
        let doc = serde_json::json!({
            "event": "ready",
            "url": url,
            "listen": listen,
            "dir": args.dir().display().to_string(),
            "pin": pin,
        });
        println!("{}", podnet::status::stamped(doc));
    } else {
        println!(
            "podnet: serve  {}  on {url}  (local, no tunnel)",
            args.dir().display()
        );
        if let Some(pin) = pin {
            println!("podnet: PIN    {pin}");
        }
    }
    // A QR code of a unix socket has no use.
    if show.qr && url.starts_with("http://") {
        print!("{}", podnet::qr::render_terminal(&url)?);
    }
    podnet::detach::ready(serde_json::json!({ "url": url, "listen": listen, "pin": pin }));
    tokio::select! {
        _ = sleep_or_signal(args.common.run_for) => {}
        ended = local.server => {
            ended
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
