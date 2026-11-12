//! `podnet git serve` (T-094): the repositories of a root over smart HTTP
//! (`podnet::git::http`) and SSH (`podnet::git::ssh`).
//!
//! * `--local ADDR`: the HTTP router on a local listener (as `serve
//!   --local`).
//! * `--ingress tunnel`: the HTTP router through a quick tunnel (the ingress
//!   service `git:`).
//! * `--ssh LISTEN`: the SSH server on HOST:PORT or unix:PATH.
//! * `--ingress relay`: the SSH server on a private unix socket of the run
//!   directory, and a node of a relay pair that connects each session to
//!   it (as `serve --ingress relay`). The operator uses `podnet relay connect
//!   NAME` as the ProxyCommand of ssh.
//!
//! The URLs go to a record in the state directory for `podnet git urls`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;

use podnet::config::Config;
use podnet::exit::{Fault, ResultExt};
use podnet::files::{Access, Role, Users};
use podnet::git::http::HttpOptions;
use podnet::ingress::{Ingress, Service};

use crate::cli::cmd::connect::unix_listener;
use crate::cli::cmd::git::{default_host_key, real_root, record_path};
use crate::cli::cmd::local::bind_local;
use crate::cli::cmd::serve::Road;
use crate::cli::cmd::tunnel::preflight;
use crate::cli::opts::{Show, TunnelOpts};
use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
pub(crate) struct GitServeArgs {
    #[command(flatten)]
    pub(crate) common: TunnelOpts,
    /// The directory that holds the repositories. A request names a
    /// repository under it (NAME or NAME.git); .. and names that start
    /// with a dot are refused.
    #[arg(long, value_name = "DIR")]
    root: PathBuf,
    /// Serve smart HTTP on this local listener (more than one is
    /// permitted): HOST:PORT (or tcp://HOST:PORT), or unix:PATH. Port 0
    /// takes a free port.
    #[arg(long, value_name = "ADDR")]
    local: Vec<String>,
    /// Serve SSH on HOST:PORT (or tcp://HOST:PORT) or unix:PATH. Only
    /// ed25519 and ECDSA keys are accepted.
    #[arg(long, value_name = "LISTEN")]
    ssh: Option<String>,
    /// The road to the clients: tunnel (smart HTTP through a quick
    /// tunnel; the default with no --local and no --ssh), relay (SSH
    /// through a relay pair), or none (only --local and --ssh).
    #[arg(long, value_enum)]
    ingress: Option<Road>,
    /// With --ingress relay: the pair name. Default: the pair with the most
    /// time left.
    #[arg(long, value_name = "NAME")]
    pair: Option<String>,
    /// A clone and a fetch need no user and no key (SSH: the user
    /// anonymous, or a key that is not in --authorized-keys).
    #[arg(long)]
    anon_read: bool,
    /// The user that can push over HTTP (HTTP Basic), with the password of
    /// --push-password-file.
    #[arg(long, value_name = "USER", requires = "push_password_file")]
    push_user: Option<String>,
    /// The file with the password of --push-user on its first line (mode
    /// 0600).
    #[arg(long, value_name = "FILE", requires = "push_user")]
    push_password_file: Option<PathBuf>,
    /// The HTTP users, one on each line: USER:ROLE:HASH or
    /// USER:ROLE:@PASSWORD-FILE, as serve --users. A read user can clone,
    /// a write user can also push.
    #[arg(long, value_name = "FILE")]
    users: Option<PathBuf>,
    /// The keys that can push over SSH (an authorized_keys file).
    #[arg(long, value_name = "FILE")]
    authorized_keys: Option<PathBuf>,
    /// A push of a write user to a path with no repository makes a bare
    /// repository there (NAME.git).
    #[arg(long)]
    create_on_push: bool,
    /// The SSH host key (ed25519). podnet makes it one time with mode
    /// 0600. Default: git/ssh_host_ed25519_key in the state directory.
    #[arg(long, value_name = "FILE")]
    host_key: Option<PathBuf>,
}

fn usage(message: &str) -> anyhow::Error {
    podnet::exit::fault(Fault::Usage, anyhow::anyhow!("git serve: {message}"))
}

/// The URLs of a running `git serve`, for `git urls`.
#[derive(Clone, Default)]
struct Urls {
    root: PathBuf,
    http: Vec<String>,
    ssh: Vec<String>,
    pair: Option<String>,
    fingerprint: Option<String>,
}

impl Urls {
    fn doc(&self, tunnel: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "root": self.root.display().to_string(),
            "pid": std::process::id(),
            "http": self.http,
            "ssh": self.ssh,
            "tunnel": tunnel,
            "pair": self.pair,
            "fingerprint": self.fingerprint,
        })
    }

    /// Write the record of `git urls` (mode 0600). A failure is a warning.
    fn record(&self, tunnel: Option<&str>) {
        let write = || -> Result<()> {
            let path = record_path(&self.root)?;
            if let Some(dir) = path.parent() {
                podnet::state::prepare(dir)?;
            }
            let text = serde_json::to_vec_pretty(&self.doc(tunnel))?;
            podnet::state::replace_file(&path, &text, 0o600)
        };
        if let Err(err) = write() {
            tracing::warn!(error = %format!("{err:#}"), "git serve: no record for git urls");
        }
    }
}

/// Removes the record at the end of `git serve`.
struct RecordFile(Option<PathBuf>);

impl Drop for RecordFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The servers of one run: each one ends only with an error.
type Servers = tokio::task::JoinSet<Result<()>>;

pub(crate) async fn serve(args: GitServeArgs) -> Result<()> {
    let root = real_root(&args.root)?;
    let ingress = args
        .ingress
        .unwrap_or(if args.local.is_empty() && args.ssh.is_none() {
            Road::Tunnel
        } else {
            Road::None
        });
    if ingress == Road::None && args.local.is_empty() && args.ssh.is_none() {
        return Err(usage("--ingress none needs --local or --ssh"));
    }
    if args.pair.is_some() && ingress != Road::Relay {
        return Err(usage("--pair is for --ingress relay"));
    }
    if args.common.check && ingress != Road::Tunnel {
        return Err(usage("--check checks a tunnel: it is for --ingress tunnel"));
    }
    if args.common.auth_user.is_some() {
        return Err(usage("give the HTTP user with --push-user or --users"));
    }
    let ok = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await
        .is_ok_and(|out| out.status.success());
    if !ok {
        return Err(podnet::exit::fault(
            Fault::Unavailable,
            anyhow::anyhow!("git serve runs the installed git, and git is not on PATH"),
        ));
    }
    let access = access(&args)?;
    let keys = match &args.authorized_keys {
        Some(path) => podnet::git::ssh::read_authorized_keys(path).fault(Fault::Config)?,
        None => Vec::new(),
    };
    if !args.anon_read && access.users.is_none() && keys.is_empty() {
        return Err(usage(
            "give --anon-read, --users, --push-user or --authorized-keys",
        ));
    }
    let http = HttpOptions {
        access: Access {
            // No users: each request but an anonymous read gets 401.
            users: Some(access.users.unwrap_or_default()),
            ..access
        },
        create_on_push: args.create_on_push,
        ..HttpOptions::new(&root)
    };
    let show = args.common.show();
    let mut urls = Urls {
        root: root.clone(),
        ..Urls::default()
    };
    let mut servers = Servers::new();
    let mut files = Vec::new();
    let router = podnet::git::http::router(http.clone())?;
    for local in &args.local {
        let bound = bind_local(local, args.common.detach.replace, router.clone()).await?;
        if bound.url.starts_with("http://") {
            urls.http.push(bound.url.trim_end_matches('/').to_string());
        }
        announce(show, "http", &bound.url);
        let (server, file) = bound.split();
        files.push(file);
        servers.spawn(async move { Ok(server.await??) });
    }
    let ssh_needed = args.ssh.is_some() || ingress == Road::Relay;
    let ssh = if ssh_needed {
        let key_path = match &args.host_key {
            Some(path) => path.clone(),
            None => {
                let path = default_host_key()?;
                if let Some(dir) = path.parent() {
                    podnet::state::prepare(dir).context("the state directory")?;
                }
                path
            }
        };
        let key = podnet::git::ssh::load_or_create_host_key(&key_path).fault(Fault::Config)?;
        urls.fingerprint = Some(podnet::git::ssh::fingerprint(&key));
        let options = podnet::git::ssh::Options {
            authorized_keys: keys,
            anon_read: args.anon_read,
            create_on_push: args.create_on_push,
            ..podnet::git::ssh::Options::new(&root)
        };
        Some((Arc::new(podnet::git::ssh::config(key)), Arc::new(options)))
    } else {
        None
    };
    if let (Some(listen), Some((config, options))) = (&args.ssh, &ssh) {
        let (shown, file) = listen_ssh(
            listen,
            args.common.detach.replace,
            config,
            options,
            &mut servers,
        )
        .await?;
        if let Some(addr) = shown.strip_prefix("tcp://") {
            urls.ssh.push(format!("ssh://git@{addr}"));
        }
        announce(show, "ssh", &shown);
        files.push(file);
    }
    if let Some(fingerprint) = &urls.fingerprint {
        if !show.json {
            println!("podnet: host key  {fingerprint}");
        }
    }
    let _record = RecordFile(record_path(&root).ok());
    let result = match ingress {
        Road::None => {
            urls.record(None);
            podnet::detach::arm_stop();
            let doc = urls.doc(None);
            if show.json {
                let mut line = doc.clone();
                line["event"] = "ready".into();
                println!("{}", podnet::status::stamped(line));
            }
            podnet::detach::ready(doc);
            run_until(args.common.run_for, &mut servers).await
        }
        Road::Relay => {
            let (config, options) = ssh.expect("--ingress relay serves SSH");
            serve_relay(&args, &mut urls, config, options, &mut servers).await
        }
        Road::Tunnel => serve_tunnel(&args, &urls, http, &mut servers).await,
    };
    drop(files);
    result
}

/// The access of HTTP: --users, --push-user and --anon-read.
fn access(args: &GitServeArgs) -> Result<Access> {
    let mut users = match &args.users {
        Some(path) => Some(Users::load(path).context("--users").fault(Fault::Config)?),
        None => None,
    };
    if let (Some(user), Some(file)) = (&args.push_user, &args.push_password_file) {
        let credentials = podnet::ingress::credentials(user, file)
            .context("--push-password-file")
            .fault(Fault::Config)?;
        users
            .get_or_insert_with(Users::default)
            .add(credentials, Role::Write)
            .fault(Fault::Usage)?;
    }
    Ok(Access {
        users,
        pin: None,
        anon_read: args.anon_read,
    })
}

/// One line for a listener: text, or a JSON `listen` event.
fn announce(show: Show, kind: &str, url: &str) {
    if show.json {
        let doc = serde_json::json!({ "event": "listen", "kind": kind, "url": url });
        println!("{}", podnet::status::stamped(doc));
    } else {
        println!("podnet: git {kind:<4}  on {url}");
    }
}

/// Serve SSH on `listen` (HOST:PORT, tcp://HOST:PORT or unix:PATH). Gives
/// the bound address (`tcp://HOST:PORT` or `unix:PATH`) and the socket
/// file.
async fn listen_ssh(
    listen: &str,
    replace: bool,
    config: &Arc<russh::server::Config>,
    options: &Arc<podnet::git::ssh::Options>,
    servers: &mut Servers,
) -> Result<(String, Option<crate::cli::cmd::connect::SocketFile>)> {
    let (config, options) = (Arc::clone(config), Arc::clone(options));
    if let Some(path) = listen.strip_prefix("unix:") {
        let (listener, file) = unix_listener(Path::new(path), replace)?;
        servers.spawn(podnet::git::ssh::serve_unix(listener, config, options));
        return Ok((format!("unix:{path}"), Some(file)));
    }
    let addr = listen.strip_prefix("tcp://").unwrap_or(listen);
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|err| {
        let fault = if addr.contains(':') {
            Fault::Unavailable
        } else {
            Fault::Usage
        };
        podnet::exit::fault(fault, anyhow::anyhow!("--ssh {listen}: {err}"))
    })?;
    let bound = listener.local_addr()?;
    servers.spawn(podnet::git::ssh::serve_tcp(listener, config, options));
    Ok((format!("tcp://{bound}"), None))
}

/// Wait for `run_for`, a signal, or the end of a server.
async fn run_until(run_for: Option<u64>, servers: &mut Servers) -> Result<()> {
    tokio::select! {
        _ = sleep_or_signal(run_for) => Ok(()),
        Some(ended) = servers.join_next() => {
            ended.context("a server of git serve stopped")?.context("a server of git serve stopped")
        }
    }
}

/// `--ingress tunnel`: the HTTP router through a quick tunnel.
async fn serve_tunnel(
    args: &GitServeArgs,
    urls: &Urls,
    http: HttpOptions,
    servers: &mut Servers,
) -> Result<()> {
    let config = Config {
        ingress: Ingress::new().catch_all(Service::Git(http)),
        ..Config::default()
    };
    let config = args.common.apply(config)?;
    // Each path but the smart protocol answers 404, also `/`.
    let show = Show {
        root_404: true,
        ..args.common.show()
    };
    if args.common.check {
        return preflight(&config, show).await;
    }
    let ready = |url: Option<&str>| {
        urls.record(url);
        if let (Some(url), false) = (url, show.json) {
            println!("podnet: git clone {}/REPO", url.trim_end_matches('/'));
        }
    };
    let tunnel =
        crate::cli::cmd::run::run_tunnel_then(config, show, args.common.run_for, vec![], &ready);
    tokio::select! {
        result = tunnel => result,
        Some(ended) = servers.join_next() => {
            ended.context("a server of git serve stopped")?.context("a server of git serve stopped")
        }
    }
}

/// `--ingress relay`: the SSH server behind the node of a relay pair.
async fn serve_relay(
    args: &GitServeArgs,
    urls: &mut Urls,
    config: Arc<russh::server::Config>,
    options: Arc<podnet::git::ssh::Options>,
    servers: &mut Servers,
) -> Result<()> {
    use podnet::relay_reverse as reverse;
    let pairs = reverse::pairs_dir()?;
    let follow = args.pair.is_none().then_some(pairs.as_path());
    let pair = reverse::load(&pairs, args.pair.as_deref())?;
    podnet::report::hide(pair.node_token());
    podnet::ws_relay::Relay::single(&pair.relay, None).fault(Fault::Config)?;
    let socket = podnet::instances::run_dir()?.join(format!(
        "git-ssh-{}.sock",
        &podnet::util::random_token()[..16]
    ));
    let listen = format!("unix:{}", socket.display());
    let (_, _file) = listen_ssh(&listen, false, &config, &options, servers).await?;
    let target = reverse::Target::Unix(socket);
    urls.pair = Some(pair.name.clone());
    urls.record(None);
    podnet::detach::arm_stop();
    let show = args.common.show();
    if show.json {
        let mut doc = urls.doc(None);
        doc["event"] = "ready".into();
        doc["relay"] = pair.relay.clone().into();
        println!("{}", podnet::status::stamped(doc));
    } else {
        println!(
            "podnet: git ssh   through the relay pair {}  ({})",
            pair.name, pair.relay
        );
        println!(
            "The operator: GIT_SSH_COMMAND=\"ssh -o ProxyCommand='podnet relay connect {0}'\" git clone ssh://git@{0}/REPO",
            pair.name
        );
    }
    // T-236: --detach returns when the node is connected (run_node calls
    // ready), not before: a clone before that gets "node offline".
    podnet::detach::ready_when_reachable(urls.doc(None));
    tokio::select! {
        result = reverse::run_node(follow, pair, &target) => result,
        result = run_until(args.common.run_for, servers) => result,
    }
}
