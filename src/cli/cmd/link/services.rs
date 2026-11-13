//! The services of a link end (T-266, T-291, T-292, T-296, T-297): what
//! a stream that the peer opens runs. `link listen` and `link connect`
//! share this code: the first line of the stream names the service, the
//! grant of the peer is checked, and then the service runs.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::Args;
use pod_core::BoxedIo;
use pod_identity::PublicKey;
use podnet::exit::{self, Fault};
use podnet::identity::{AllowList, Grant, Identity};
use podnet::link;
use podnet::link::Link;
use podnet::link_announce::{self as announce, Mesh};
use podnet::link_ctl::{Hub, Slot};
use podnet::link_record::{self as record, Options as RecordOptions};
use podnet::link_service as service;
use podnet::relay_reverse::Target;

/// The most services of one end.
const MAX_SERVICES: usize = 16;

/// The time that a connection to a target can take.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);

/// The flags that name the services of an end.
#[derive(Args, Debug, Default)]
pub(crate) struct ServiceArgs {
    /// A service of this listener: NAME=TARGET, with a NAME of 1 to 32
    /// characters from A-Z a-z 0-9 . _ - and a TARGET as in --to. A peer
    /// reaches it with `link dial --service NAME` or `peer:FINGERPRINT/NAME`.
    /// The service `socks5` takes no target: `--service socks5` makes this
    /// end the server side of SOCKS5 (CONNECT, and UDP as frames on the stream with the
    /// command 0x83; a name resolves here; BIND listens here on the loopback or --socks-bind-ip; command 3 gets reply 7).
    /// The service `http` takes no target either: `--service http` makes
    /// this end an HTTP proxy (`CONNECT` and absolute form, one request for
    /// each stream, 8 KiB of head at most). A target is the name of
    /// another service of this listener; there is no DNS lookup.
    /// Repeat it: 16 services at most.
    ///
    /// A TARGET `exec:/ABS/PATH` runs a program for each stream (no shell):
    /// its stdin and stdout are the stream and its stderr goes to the log.
    /// Give its arguments with --exec-arg, its environment with --exec-env
    /// (the environment is empty beyond the names given) and its user with
    /// --exec-user (root only).
    #[arg(long, value_name = "NAME=TARGET")]
    pub(crate) service: Vec<String>,
    /// The address where `BIND` of the service `socks5` listens (default
    /// 127.0.0.1). Give a public address only when the incoming connections
    /// must come from the network.
    #[arg(long, value_name = "IP")]
    pub(crate) socks_bind_ip: Option<String>,
    /// An argument of the program of the service NAME: NAME=ARG. Repeat it
    /// for each argument, in order (64 at most for each service).
    #[arg(long, value_name = "NAME=ARG")]
    pub(crate) exec_arg: Vec<String>,
    /// A name of the environment that the program of the service NAME gets
    /// from this process: NAME=VAR. No other variable passes.
    #[arg(long, value_name = "NAME=VAR")]
    pub(crate) exec_env: Vec<String>,
    /// The user of the program of the service NAME: NAME=USER. It needs
    /// root; without it the listener exits 64.
    #[arg(long, value_name = "NAME=USER")]
    pub(crate) exec_user: Vec<String>,
}

/// What a service does with its streams.
pub(super) enum Service {
    /// Copy the bytes to a target (`raw` and each `NAME=TARGET`).
    Target(Target),
    /// Serve SOCKS5 on the stream (`--service socks5`, T-291).
    Socks5(IpAddr),
    /// Serve an HTTP proxy on the stream (`--service http`, T-292).
    Http,
    /// Run a program for each stream (`--service NAME=exec:/PATH`, T-296).
    Exec(podnet::link_exec::Spec),
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Service::Target(target) => write!(f, "{target}"),
            Service::Socks5(_) => f.write_str("socks5"),
            Service::Http => f.write_str("http"),
            Service::Exec(spec) => write!(f, "{spec}"),
        }
    }
}

/// The services of a listener: a name and what its streams do.
pub(super) type Services = Arc<BTreeMap<String, Service>>;

/// The services of --to and --service. A bad name, a bad target, a
/// duplicate name or more than 16 services exits 64.
pub(super) fn services_of(
    to: Option<&str>,
    args: &ServiceArgs,
    announce: bool,
) -> Result<BTreeMap<String, Service>> {
    let usage = |text: String| exit::fault(Fault::Usage, anyhow!(text));
    let target =
        |text: &str, flag: &str| Target::parse(text).map_err(|e| usage(format!("{flag} {e}")));
    let mut out = BTreeMap::new();
    if let Some(text) = to {
        out.insert(
            service::RAW.to_string(),
            Service::Target(target(text, "--to")?),
        );
    }
    for item in &args.service {
        if item == podnet::link_socks::NAME || item == podnet::link_http::NAME {
            let kind = if item == podnet::link_http::NAME {
                Service::Http
            } else {
                Service::Socks5(bind_ip(args)?)
            };
            if out.insert(item.clone(), kind).is_some() {
                return Err(usage(format!("--service {item}: given twice")));
            }
            continue;
        }
        let (name, text) = item.split_once('=').ok_or_else(|| {
            usage(format!(
                "--service {item:?}: give NAME=TARGET (or `socks5` or `http` with no target)"
            ))
        })?;
        service::check_name(name)?;
        if name == podnet::link_socks::NAME || name == podnet::link_http::NAME {
            return Err(usage(format!(
                "--service {name}: the service {name} takes no target: give `--service {name}`"
            )));
        }
        let found = match text.strip_prefix(podnet::link_exec::PREFIX) {
            Some(program) => Service::Exec(exec_of(args, name, program)?),
            None => Service::Target(target(text, "--service")?),
        };
        if out.insert(name.to_string(), found).is_some() {
            return Err(usage(format!(
                "--service {name}: the service {name:?} has two targets (--to is the service raw)"
            )));
        }
    }
    for (flag, list) in [
        ("--exec-arg", &args.exec_arg),
        ("--exec-env", &args.exec_env),
        ("--exec-user", &args.exec_user),
    ] {
        for item in list {
            let name = item.split_once('=').map_or("", |(n, _)| n);
            if !matches!(out.get(name), Some(Service::Exec(_))) {
                return Err(usage(format!(
                    "{flag} {item:?}: give NAME=VALUE for a service NAME with an exec: target"
                )));
            }
        }
    }
    if announce && out.contains_key(announce::NAME) {
        return Err(usage(format!(
            "--service {}: this name is the stream of --announce",
            announce::NAME
        )));
    }
    if announce && out.contains_key(podnet::link_forward::NAME) {
        return Err(usage(format!(
            "--service {}: this name is the stream of a forwarded stream (--announce)",
            podnet::link_forward::NAME
        )));
    }
    if out.len() > MAX_SERVICES {
        return Err(usage(format!(
            "a listener has {MAX_SERVICES} services at most"
        )));
    }
    Ok(out)
}

/// The address of `--socks-bind-ip`. A bad address exits 64.
fn bind_ip(args: &ServiceArgs) -> Result<IpAddr> {
    match &args.socks_bind_ip {
        None => Ok(IpAddr::from([127, 0, 0, 1])),
        Some(text) => text.parse().map_err(|_| {
            exit::fault(
                Fault::Usage,
                anyhow!("--socks-bind-ip {text:?} is no IP address"),
            )
        }),
    }
}

/// The values of the flag `list` (NAME=VALUE) for the service `name`.
fn values_of<'a>(list: &'a [String], name: &str) -> Vec<&'a str> {
    list.iter()
        .filter_map(|i| i.split_once('=').filter(|(n, _)| *n == name))
        .map(|(_, v)| v)
        .collect()
}

/// What a listener needs to seal the streams that reach it (`--seal`).
#[derive(Clone)]
pub(super) struct Seal {
    pub(super) me: Arc<Identity>,
    /// The keys that may seal a stream to this end: the allow list.
    pub(super) allow: AllowList,
    pub(super) options: RecordOptions,
}

/// The bytes of a stream, as the service sees them: the stream itself or its
/// sealed form.
pub(super) trait Wire: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Wire for T {}

/// The stream of a service: plain, or sealed (`--seal`).
enum Io<'a> {
    Plain(&'a mut link::Stream),
    Sealed(record::Sealed),
}

macro_rules! each {
    ($self:ident, $io:ident => $call:expr) => {
        match $self.get_mut() {
            Io::Plain($io) => $call,
            Io::Sealed($io) => $call,
        }
    };
}

impl tokio::io::AsyncRead for Io<'_> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        each!(self, io => std::pin::Pin::new(io).poll_read(cx, buf))
    }
}

impl tokio::io::AsyncWrite for Io<'_> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        each!(self, io => std::pin::Pin::new(io).poll_write(cx, buf))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        each!(self, io => std::pin::Pin::new(io).poll_flush(cx))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        each!(self, io => std::pin::Pin::new(io).poll_shutdown(cx))
    }
}

/// Answer OPEN, then run `serve` over the stream. With --seal, the two ends
/// run the end-to-end handshake first, and `serve` gets the sealed stream
/// (T-298).
async fn over<'a, Fut: std::future::Future<Output = Result<()>>>(
    stream: &'a mut link::Stream,
    seal: Option<&Seal>,
    serve: impl FnOnce(Io<'a>) -> Fut,
) -> Result<()> {
    service::reply(stream, service::OPEN).await?;
    let Some(seal) = seal else {
        return serve(Io::Plain(stream)).await;
    };
    record::accept_with(
        stream,
        &seal.me,
        &seal.allow,
        seal.options.clone(),
        |sealed| serve(Io::Sealed(sealed)),
    )
    .await?
}

/// The program of the service `name`, from `--exec-*`.
fn exec_of(args: &ServiceArgs, name: &str, program: &str) -> Result<podnet::link_exec::Spec> {
    let own = |list: &[String]| -> Vec<String> {
        values_of(list, name)
            .into_iter()
            .map(String::from)
            .collect()
    };
    let users = values_of(&args.exec_user, name);
    if users.len() > 1 {
        return Err(exit::fault(
            Fault::Usage,
            anyhow!("--exec-user: the service {name:?} has two users"),
        ));
    }
    podnet::link_exec::Spec::new(
        program,
        own(&args.exec_arg),
        own(&args.exec_env),
        users.first().copied(),
    )
}

/// Read the service of a stream, connect to its target, and copy the
/// stream to the connection and back.
pub(super) async fn bridge(
    stream: &mut link::Stream,
    (services, hub): (&BTreeMap<String, Service>, &Hub),
    announced: Option<(Arc<Mesh>, Arc<Link>, Option<u32>)>,
    peer: PublicKey,
    (grant, refusals): (&Grant, &podnet::link_ctl::Refusals),
    seal: Option<&Seal>,
) -> Result<()> {
    let mut name = service::accept(stream).await?;
    if let Some((mesh, link, weight)) = announced {
        if name == announce::NAME {
            announce::receive_stream(mesh, link, peer, weight, stream).await;
            return Ok(());
        }
        // A stream for another node goes on; a stream for this node is a
        // request for its service.
        if name == podnet::link_forward::NAME {
            let counted = || refusals.forward();
            match podnet::link_forward::accept_with(&mesh, stream, grant.may_forward(), counted)
                .await?
            {
                podnet::link_forward::Arrived::Local(local) => name = local,
                podnet::link_forward::Arrived::Done => return Ok(()),
            }
        }
    }
    // One decision for each OPEN: the grant of the peer, then the service.
    if !grant.allows_service(&name) {
        refusals.service();
        service::reply(stream, service::FORBIDDEN).await?;
        tracing::warn!(peer = %peer.display_fingerprint(), service = %name, "a stream is refused: no grant");
        return Ok(());
    }
    // A service of a client of the control socket (`accept`) is a unix target.
    let registered = hub.claimed(&name).map(|p| Service::Target(Target::Unix(p)));
    let Some(found) = services.get(&name).or(registered.as_ref()) else {
        service::reply(stream, service::NO_SERVICE).await?;
        return Err(anyhow!(
            "the peer asked for the service {name:?}, which this listener does not have"
        ));
    };
    let target = match found {
        Service::Target(target) => target,
        Service::Exec(spec) => {
            let running = match podnet::link_exec::start(&name, spec) {
                Ok(running) => running,
                Err((answer, err)) => {
                    service::reply(stream, answer).await?;
                    return Err(err);
                }
            };
            return over(stream, seal, |io| running.pump(io)).await;
        }
        Service::Socks5(bind_ip) => {
            return over(stream, seal, |io| {
                podnet::link_socks::serve_with(io, *bind_ip)
            })
            .await;
        }
        Service::Http => {
            let resolve = resolver(services);
            return over(stream, seal, |io| podnet::link_http::serve(io, resolve)).await;
        }
    };
    let mut io = match connect(target).await {
        Ok(io) => io,
        Err(err) => {
            service::reply(stream, service::NO_ORIGIN).await?;
            return Err(err);
        }
    };
    over(stream, seal, |mut wire| async move {
        tokio::io::copy_bidirectional(&mut wire, &mut io).await?;
        Ok(())
    })
    .await
}

/// The names that the service `http` reaches: each service with a target.
fn resolver(services: &BTreeMap<String, Service>) -> podnet::link_http::Resolver {
    use podnet::link_http::Origin;
    let map: BTreeMap<String, Origin> = services
        .iter()
        .filter_map(|(name, service)| match service {
            Service::Target(Target::Tcp { host, port }) => {
                Some((name.clone(), Origin::Tcp(host.clone(), *port)))
            }
            Service::Target(Target::Unix(path)) => Some((name.clone(), Origin::Unix(path.clone()))),
            _ => None,
        })
        .collect();
    Arc::new(move |name| map.get(name).cloned())
}

/// A connection to `target`. A name resolves with the resolver of this
/// host, and no proxy is used: the service is on this host or its network.
async fn connect(target: &Target) -> Result<BoxedIo> {
    let work = async {
        Ok::<BoxedIo, std::io::Error>(match target {
            Target::Tcp { host, port } => {
                let s = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
                let _ = s.set_nodelay(true);
                Box::new(s)
            }
            Target::Unix(path) => Box::new(tokio::net::UnixStream::connect(path).await?),
        })
    };
    tokio::time::timeout(CONNECT_LIMIT, work)
        .await
        .map_err(|_| anyhow!("no answer from {target} in {} s", CONNECT_LIMIT.as_secs()))?
        .with_context(|| format!("connecting to {target}"))
}

/// What the task of each stream that a peer opens needs.
pub(super) struct Streams {
    pub(super) hub: Arc<Hub>,
    pub(super) services: Services,
    pub(super) grant: Grant,
    pub(super) peer: PublicKey,
    pub(super) shown: String,
    /// The route announcements, for a listener with `--announce`.
    pub(super) mesh: Option<(Arc<Mesh>, Arc<Link>, Option<u32>)>,
    /// Seal the streams that reach a service (`--seal`).
    pub(super) seal: Option<Seal>,
}

impl Streams {
    /// Serve one stream that the peer opened, in its own task.
    pub(super) fn spawn(&self, stream: link::Stream, slot: &Slot) {
        slot.count_accepted();
        // `close-stream` of the control socket names the stream by its id.
        let tracked = slot.track(&stream);
        let refusals = slot.refusals();
        let (hub, services, grant) = (self.hub.clone(), self.services.clone(), self.grant.clone());
        let (peer, shown, mesh) = (self.peer, self.shown.clone(), self.mesh.clone());
        let seal = self.seal.clone();
        tokio::spawn(async move {
            let mut stream = stream;
            tokio::select! {
                done = bridge(&mut stream, (&services, &hub), mesh, peer, (&grant, &refusals), seal.as_ref()) => {
                    if let Err(err) = done {
                        // Tell the opener that the stream is over.
                        let _ = tokio::io::AsyncWriteExt::shutdown(&mut stream).await;
                        hub.note_error(&format!("a stream of {shown} ended: {err:#}"));
                        tracing::warn!(peer = shown, error = %format!("{err:#}"), "a stream ended");
                    }
                }
                _ = tracked.cancelled() => stream.reset(link::ResetCode::CANCEL),
            }
        });
    }
}
