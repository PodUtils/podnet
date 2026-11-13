//! `podnet serve DIR --at peer:FINGERPRINT/SERVICE --at-listen ADDRESS`
//! (T-297): the file server answers on a service stream of a link.
//!
//! The file server is the one of `podnet serve` (the router of
//! `podnet::files`), bound on a unix socket in a private directory. A link
//! listener (the code of `podnet link listen`) runs at ADDRESS with the
//! allow list `{FINGERPRINT}` and one service, SERVICE, whose target is
//! that socket. A peer opens a stream of SERVICE and speaks HTTP on it.

use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Parser;

use podnet::exit::{self, Fault};

use super::link::listen::{listen, ListenArgs};
use super::local::bind_local;
use crate::cli::signals::sleep_or_signal;

/// The arguments of `link listen`, parsed from a list of strings.
#[derive(Parser, Debug)]
struct Wrapped {
    #[command(flatten)]
    args: ListenArgs,
}

/// The peer and the service of `--at`.
fn parse_at(text: &str) -> Result<(String, String)> {
    let usage = |t: String| exit::fault(Fault::Usage, anyhow!(t));
    let rest = text.strip_prefix("peer:").ok_or_else(|| {
        usage(format!(
            "--at {text:?}: give peer:FINGERPRINT/SERVICE (the fingerprint that podnet link status shows)"
        ))
    })?;
    let (fingerprint, service) = rest.rsplit_once('/').ok_or_else(|| {
        usage(format!(
            "--at {text:?}: give peer:FINGERPRINT/SERVICE (the service is missing)"
        ))
    })?;
    if fingerprint.is_empty() {
        return Err(usage(format!("--at {text:?}: the fingerprint is empty")));
    }
    podnet::link_service::check_name(service)?;
    Ok((fingerprint.to_string(), service.to_string()))
}

/// A private directory (mode 0700) that is removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!("pn-at-{}-{nanos:x}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .with_context(|| format!("making {}", path.display()))
            .map_err(|e| exit::fault(Fault::OsError, e))?;
        Ok(Scratch(path))
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write `text` to `path` with the mode 0600.
fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    writeln!(file, "{text}")?;
    Ok(())
}

/// `serve --ingress peer:FINGERPRINT/SERVICE` (T-297): serve `router` on
/// the streams that the peer opens for SERVICE on a link of a running
/// instance (`link connect`, found as `forward` finds it).
///
/// The file server is bound on a unix socket in a private directory. The
/// instance gets that socket with the verb `accept` of its control socket
/// and keeps the service while this process holds the connection.
pub(crate) async fn serve_peer(
    at: &str,
    router: axum::Router,
    dir: &Path,
    pin: Option<&str>,
    run_for: Option<u64>,
) -> Result<()> {
    let (fingerprint, service) = parse_at(at)?;
    let scratch = Scratch::new()?;
    let sock = scratch.path("files.sock");
    let local = bind_local(&format!("unix:{}", sock.display()), false, router).await?;
    let (server, _file) = local.split();
    let mut held = podnet::link_ctl::accept_peer(&fingerprint, &service, &sock).await?;
    println!(
        "podnet: serve  {}  at {at}  (a stream that the peer opens for the service {service})",
        dir.display()
    );
    if let Some(pin) = pin {
        println!("podnet: PIN    {pin}");
    }
    tokio::select! {
        _ = held.lost() => Err(exit::fault(
            Fault::Unavailable,
            anyhow!("the link instance ended the service {service}"),
        )),
        ended = server => {
            ended
                .context("the file server stopped")?
                .context("the file server stopped")
        }
        _ = sleep_or_signal(run_for) => Ok(()),
    }
}

/// Serve `router` at the peer named by `at`, on the link listener `address`.
pub(crate) async fn serve_at(
    at: &str,
    address: &str,
    router: axum::Router,
    dir: &Path,
    pin: Option<&str>,
    run_for: Option<u64>,
) -> Result<()> {
    let (fingerprint, service) = parse_at(at)?;
    let scratch = Scratch::new()?;
    let (allow, sock) = (scratch.path("allow"), scratch.path("files.sock"));
    write_private(&allow, &fingerprint)?;
    let local = bind_local(&format!("unix:{}", sock.display()), false, router).await?;
    let (server, _file) = local.split();
    let target = format!("{service}=unix:{}", sock.display());
    let argv = [
        "listen",
        address,
        "--allow",
        &allow.display().to_string(),
        "--service",
        &target,
    ];
    let args = Wrapped::try_parse_from(argv)
        .map_err(|e| exit::fault(Fault::Usage, anyhow!("--at-listen: {e}")))?
        .args;
    println!(
        "podnet: serve  {}  at {at}  (a stream of the service {service})",
        dir.display()
    );
    if let Some(pin) = pin {
        println!("podnet: PIN    {pin}");
    }
    tokio::select! {
        done = listen(args) => done,
        ended = server => {
            ended
                .context("the file server stopped")?
                .context("the file server stopped")
        }
        _ = sleep_or_signal(run_for) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_of_at_is_checked() {
        let (fp, name) = parse_at("peer:AB/CD/files").unwrap();
        assert_eq!((fp.as_str(), name.as_str()), ("AB/CD", "files"));
        for bad in ["AB/files", "peer:AB", "peer:/files", "peer:AB/b d"] {
            let err = parse_at(bad).unwrap_err();
            assert_eq!(exit::code_of(&err), 64, "{bad}");
        }
    }
}
