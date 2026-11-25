//! A TCP listener on a loopback address that no client of this host can
//! reach (GitHub #736). Where the loopback has no address, `bind` of
//! 127.0.0.1 passes and each connect fails (`ENETUNREACH`): the listener is
//! ready, and no client connects. The check binds a second socket of the
//! same address on port 0 and connects to it, so the listener of the
//! command gets no connection of the check.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::Result;

/// The time limit of the connect of the check.
const WAIT: Duration = Duration::from_secs(1);

/// Exit 69 when `ip` is a loopback address and a TCP connect to a listener
/// of it fails here. `label` names the listener (`--local 127.0.0.1:8080`).
/// A check that cannot run (no bind of port 0, no `getsockname`) passes: the
/// bind of the command gives its own error.
pub fn check(ip: IpAddr, label: &str) -> Result<()> {
    check_with(ip, label, probe)
}

/// [`check`] for the address of `listener`. A listener whose address is
/// not known (`getsockname` refused) passes.
pub fn check_listener(listener: &tokio::net::TcpListener, label: &str) -> Result<()> {
    match listener.local_addr() {
        Ok(at) => check(at.ip(), label),
        Err(_) => Ok(()),
    }
}

/// [`check`] with the probe `connect`.
pub(crate) fn check_with(
    ip: IpAddr,
    label: &str,
    connect: impl FnOnce(IpAddr) -> Option<io::Error>,
) -> Result<()> {
    if !ip.is_loopback() {
        return Ok(());
    }
    let Some(e) = connect(ip) else {
        return Ok(());
    };
    // A refusal of this process (Landlock, seccomp) is a policy of this
    // process alone: a client outside it can still connect.
    if matches!(e.raw_os_error(), Some(libc::EACCES | libc::EPERM)) {
        return Ok(());
    }
    Err(crate::exit::fault(
        crate::exit::Fault::Unavailable,
        anyhow::anyhow!(
            "{label}: a TCP connect to {ip} fails on this host ({e}), so no client can reach the listener; \
             use a unix socket (unix:PATH) where the command takes one (podnet net doctor shows the probes)"
        ),
    ))
}

/// The error of a TCP connect to a new listener on `ip`, or `None` when the
/// connect works or the check cannot run.
fn probe(ip: IpAddr) -> Option<io::Error> {
    // egress-exempt: a connect to a listener of this process on the loopback
    let listener = std::net::TcpListener::bind(SocketAddr::new(ip, 0)).ok()?;
    let at = listener.local_addr().ok()?;
    std::net::TcpStream::connect_timeout(&at, WAIT).err()
}

#[cfg(test)]
mod tests {
    use super::*;

    // GitHub #736: a loopback address whose connect fails exits 69 with the
    // cause and the step; another address, a check that cannot run and a
    // refusal of this process pass.
    #[test]
    fn a_loopback_that_no_connect_reaches_exits_69() {
        let unreachable = || Some(io::Error::from_raw_os_error(libc::ENETUNREACH));
        let local: IpAddr = "127.0.0.1".parse().unwrap();
        let six: IpAddr = "::1".parse().unwrap();
        let outside: IpAddr = "192.0.2.1".parse().unwrap();
        // (address, the error of the connect, the check fails)
        let table: [(IpAddr, Option<io::Error>, bool); 6] = [
            (local, unreachable(), true),
            (six, unreachable(), true),
            (local, Some(io::Error::from(io::ErrorKind::TimedOut)), true),
            (local, None, false),
            (outside, unreachable(), false),
            (
                local,
                Some(io::Error::from_raw_os_error(libc::EACCES)),
                false,
            ),
        ];
        for (ip, error, fails) in table {
            let shown = format!("{ip} {error:?}");
            let got = check_with(ip, "--local x", |_| error);
            assert_eq!(got.is_err(), fails, "{shown}");
            if let Err(e) = got {
                assert_eq!(
                    crate::exit::code_of(&e),
                    crate::exit::UNAVAILABLE,
                    "{shown}"
                );
                let text = format!("{e:#}");
                assert!(text.starts_with("--local x: a TCP connect to "), "{text}");
                assert!(text.contains("unix:PATH"), "{text}");
            }
        }
        // The probe of this host: a loopback with an address passes.
        if probe(local).is_none() {
            assert!(check(local, "--local x").is_ok());
        }
    }
}
