//! An SSH server for git (T-093): `git upload-pack`, `git receive-pack`
//! and `git upload-archive` under russh.
//!
//! * The host key is ed25519. [`load_or_create_host_key`] makes it one
//!   time and keeps it with mode 0600.
//! * A key of the `authorized_keys` file gets read and write access.
//!   Only ed25519 and ECDSA keys are accepted (no `rsa` feature, so
//!   `cargo deny` stays green).
//! * With `anon_read`, the user [`ANON_USER`] gets read access with the
//!   method `none`, and a key that is not in the file gets read access.
//!   An OpenSSH client sends `none` first: an accept of `none` for each
//!   user stops each client that has a key from a push.
//! * `env` keeps only `GIT_PROTOCOL`. `exec` takes only the commands of
//!   [`super::parse_exec`]. A shell, a pty, a subsystem, X11,
//!   `direct-tcpip`, `tcpip-forward` and `streamlocal-forward` are
//!   refused.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use russh::keys::ssh_key::rand_core::UnwrapErr;
use russh::keys::ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, PublicKeyOffer, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, ChannelMsg, MethodKind, MethodSet};
use ssh_key::getrandom::SysRng;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

/// The user that gets read access with no key when `anon_read` is set.
pub const ANON_USER: &str = "anonymous";

/// The settings of the server.
#[derive(Clone, Debug)]
pub struct Options {
    /// The directory that holds the repositories.
    pub root: PathBuf,
    /// The keys with read and write access.
    pub authorized_keys: Vec<PublicKey>,
    /// Give read access to [`ANON_USER`] with no key, and to each key.
    pub anon_read: bool,
    /// The `git` program.
    pub git: OsString,
}

impl Options {
    /// The options for `root` with no key, no anonymous access and `git`
    /// from `PATH`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Options {
            root: root.into(),
            authorized_keys: Vec::new(),
            anon_read: false,
            git: OsString::from("git"),
        }
    }
}

/// Read the ed25519 host key at `path`. When no file is there, make a key
/// and write it with mode 0600.
pub fn load_or_create_host_key(path: &Path) -> Result<PrivateKey> {
    if path.exists() {
        let key = PrivateKey::read_openssh_file(path)
            .with_context(|| format!("reading the host key {}", path.display()))?;
        if key.algorithm() != Algorithm::Ed25519 {
            bail!("the host key {} is not ed25519", path.display());
        }
        return Ok(key);
    }
    let key = PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519)
        .context("making the host key")?;
    let text = key
        .to_openssh(LineEnding::LF)
        .context("encoding the host key")?;
    podnet_core::state::replace_file(path, text.as_bytes(), 0o600)?;
    Ok(key)
}

/// True for the algorithms that the server accepts for a user key.
pub fn key_allowed(algorithm: &Algorithm) -> bool {
    matches!(algorithm, Algorithm::Ed25519 | Algorithm::Ecdsa { .. })
}

/// Read the keys of an `authorized_keys` file. An entry with options, or
/// with a key that is not ed25519 or ECDSA, is left out with a warning.
pub fn read_authorized_keys(path: &Path) -> Result<Vec<PublicKey>> {
    let entries = ssh_key::AuthorizedKeys::read_file(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let mut keys = Vec::new();
    for entry in entries {
        let key = entry.public_key();
        if !entry.config_opts().is_empty() {
            tracing::warn!(file = %path.display(), "git ssh: a key with options is left out");
        } else if !key_allowed(&key.algorithm()) {
            tracing::warn!(file = %path.display(), algorithm = %key.algorithm(), "git ssh: a key that is not ed25519 or ECDSA is left out");
        } else {
            keys.push(key.clone());
        }
    }
    Ok(keys)
}

/// The russh configuration of the server with `host_key`.
pub fn config(host_key: PrivateKey) -> russh::server::Config {
    russh::server::Config {
        server_id: russh::SshId::Standard(std::borrow::Cow::Borrowed(concat!(
            "SSH-2.0-podnet_",
            env!("CARGO_PKG_VERSION")
        ))),
        methods: MethodSet::from(&[MethodKind::None, MethodKind::PublicKey][..]),
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![host_key],
        inactivity_timeout: Some(Duration::from_secs(600)),
        preferred: russh::Preferred::DEFAULT,
        ..Default::default()
    }
}

/// Serve one SSH connection on `stream` until it ends.
pub async fn serve_stream<S>(
    config: Arc<russh::server::Config>,
    options: Arc<Options>,
    stream: S,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let running = russh::server::run_stream(config, stream, Conn::new(options)).await?;
    running.await
}

/// Accept connections on a unix socket and serve each one in a task.
pub async fn serve_unix(
    listener: tokio::net::UnixListener,
    config: Arc<russh::server::Config>,
    options: Arc<Options>,
) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await.context("accepting on the socket")?;
        spawn_conn(Arc::clone(&config), Arc::clone(&options), stream);
    }
}

/// Accept connections on a TCP listener and serve each one in a task.
pub async fn serve_tcp(
    listener: tokio::net::TcpListener,
    config: Arc<russh::server::Config>,
    options: Arc<Options>,
) -> Result<()> {
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .context("accepting on the listener")?;
        let _ = stream.set_nodelay(true);
        spawn_conn(Arc::clone(&config), Arc::clone(&options), stream);
    }
}

fn spawn_conn<S>(config: Arc<russh::server::Config>, options: Arc<Options>, stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(err) = serve_stream(config, options, stream).await {
            tracing::debug!(error = %err, "git ssh: the connection ended");
        }
    });
}

/// The access of an authenticated user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    None,
    Read,
    Write,
}

/// The state of one connection.
struct Conn {
    options: Arc<Options>,
    access: Access,
    channels: HashMap<ChannelId, Channel<Msg>>,
    protocol: HashMap<ChannelId, String>,
}

impl Conn {
    fn new(options: Arc<Options>) -> Self {
        Conn {
            options,
            access: Access::None,
            channels: HashMap::new(),
            protocol: HashMap::new(),
        }
    }

    fn known(&self, key: &PublicKey) -> bool {
        self.options
            .authorized_keys
            .iter()
            .any(|known| known.key_data() == key.key_data())
    }

    /// Refuse a request on `channel`, with `message` on its stderr.
    fn refuse(&self, channel: ChannelId, message: &str, session: &mut Session) -> Result<()> {
        tracing::info!(reason = message, "git ssh: a request is refused");
        let line = format!("podnet: {message}\n");
        session.extended_data(channel, 1, line.into_bytes())?;
        session.channel_failure(channel)?;
        Ok(())
    }
}

/// A value of `GIT_PROTOCOL` that git can send: `version=2` and other
/// `key=value` pairs with `:` between them.
fn protocol_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"=:.,-_".contains(&b))
}

impl russh::server::Handler for Conn {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth> {
        if self.options.anon_read && user == ANON_USER {
            self.access = Access::Read;
            return Ok(Auth::Accept);
        }
        Ok(Auth::reject())
    }

    async fn auth_publickey_offered(&mut self, _: &str, offer: PublicKeyOffer<'_>) -> Result<Auth> {
        if key_allowed(offer.algorithm) && (self.options.anon_read || self.known(offer.public_key))
        {
            return Ok(Auth::Accept);
        }
        Ok(Auth::reject())
    }

    async fn auth_publickey(&mut self, _: &str, key: VerifiedPublicKey<'_>) -> Result<Auth> {
        if !key_allowed(key.algorithm) {
            return Ok(Auth::reject());
        }
        if self.known(key.public_key) {
            self.access = Access::Write;
        } else if self.options.anon_read {
            self.access = Access::Read;
        } else {
            return Ok(Auth::reject());
        }
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<()> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_close(&mut self, channel: ChannelId, _: &mut Session) -> Result<()> {
        self.channels.remove(&channel);
        self.protocol.remove(&channel);
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        value: &str,
        session: &mut Session,
    ) -> Result<()> {
        if name == "GIT_PROTOCOL" && protocol_ok(value) {
            self.protocol.insert(channel, value.to_string());
            session.channel_success(channel)?;
        } else {
            session.channel_failure(channel)?;
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<()> {
        let command = String::from_utf8_lossy(data);
        let (service, path) = match super::parse_exec(&command) {
            Ok(parsed) => parsed,
            Err(err) => return self.refuse(channel, &err, session),
        };
        if service.writes() && self.access != Access::Write {
            return self.refuse(
                channel,
                "a push needs a key of the authorized_keys file",
                session,
            );
        }
        let dir = match super::repo_dir(&self.options.root, &path) {
            Ok(dir) => dir,
            Err(err) => return self.refuse(channel, &err, session),
        };
        let Some(chan) = self.channels.remove(&channel) else {
            return self.refuse(channel, "the channel has a command already", session);
        };
        let protocol = self.protocol.remove(&channel);
        session.channel_success(channel)?;
        let git = self.options.git.clone();
        tokio::spawn(async move {
            if let Err(err) = run_git(chan, &git, service, &dir, protocol).await {
                tracing::debug!(error = %err, "git ssh: the command ended");
            }
        });
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<()> {
        self.refuse(channel, "this server gives no shell", session)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        session: &mut Session,
    ) -> Result<()> {
        self.refuse(channel, "this server has no subsystem", session)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<()> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _: bool,
        _: &str,
        _: &str,
        _: u32,
        session: &mut Session,
    ) -> Result<()> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _: Channel<Msg>,
        _: &str,
        _: u32,
        _: &str,
        _: u32,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<()> {
        reply
            .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn channel_open_direct_streamlocal(
        &mut self,
        _: Channel<Msg>,
        _: &str,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<()> {
        reply
            .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn tcpip_forward(&mut self, _: &str, _: &mut u32, _: &mut Session) -> Result<bool> {
        Ok(false)
    }

    async fn streamlocal_forward(&mut self, _: &str, _: &mut Session) -> Result<bool> {
        Ok(false)
    }
}

/// Run `git SERVICE DIR` and connect its stdin, stdout and stderr to
/// `channel`. Send the exit status, EOF and close at the end.
async fn run_git(
    channel: Channel<Msg>,
    git: &OsString,
    service: super::Service,
    dir: &Path,
    protocol: Option<String>,
) -> Result<()> {
    let mut command = tokio::process::Command::new(git);
    command
        .arg(service.subcommand())
        .arg(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_PROTOCOL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(protocol) = protocol {
        command.env("GIT_PROTOCOL", protocol);
    }
    let (mut read, write) = channel.split();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let line = format!("podnet: git does not start: {err}\n");
            let _ = write.extended_data_bytes(1, line.into_bytes()).await;
            let _ = write.exit_status(127).await;
            let _ = write.eof().await;
            let _ = write.close().await;
            return Err(err).context("starting git");
        }
    };
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let input = tokio::spawn(async move {
        while let Some(msg) = read.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    let Some(pipe) = stdin.as_mut() else { continue };
                    if pipe.write_all(&data).await.is_err() {
                        stdin = None;
                    }
                }
                ChannelMsg::Eof => stdin = None,
                ChannelMsg::Close => break,
                _ => {}
            }
        }
    });
    let mut out = write.make_writer();
    let mut err = write.make_writer_ext(Some(1));
    let (copied_out, copied_err) = tokio::join!(
        tokio::io::copy(&mut stdout, &mut out),
        tokio::io::copy(&mut stderr, &mut err),
    );
    // A closed channel stops a copy: close the pipes so that git ends.
    drop(stdout);
    drop(stderr);
    let status = child.wait().await.context("waiting for git")?;
    input.abort();
    copied_out.context("copying the output of git")?;
    copied_err.context("copying the errors of git")?;
    let code = status.code().unwrap_or_else(|| {
        128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0)
    });
    write.exit_status(u32::try_from(code).unwrap_or(1)).await?;
    write.eof().await?;
    write.close().await?;
    Ok(())
}
