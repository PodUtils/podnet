//! Origin connections: TCP, TLS, unix sockets, and TLS over unix.

use std::sync::Once;

pub mod loopback;

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncWrite};

pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

pub type BoxedIo = Box<dyn AsyncReadWrite>;

/// Make `path` free for a new unix listener (issues 202 and 115, and the
/// rule of podssh): a socket that answers belongs to another process and
/// is refused (exit 69), unless `replace`; a stale socket file is removed;
/// another kind of file is never removed (exit 64).
#[cfg(unix)]
pub fn claim_socket_path(path: &std::path::Path, replace: bool) -> Result<()> {
    use crate::exit::{fault, Fault};
    use std::os::unix::fs::FileTypeExt as _;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if !replace && socket_answers(path) {
                return Err(fault(
                    Fault::Unavailable,
                    anyhow::anyhow!(
                        "unix:{} is served by another process (podnet ps; podnet stop NAME, or --replace where the command has it)",
                        path.display()
                    ),
                ));
            }
            std::fs::remove_file(path)
                .with_context(|| format!("removing the old socket unix:{}", path.display()))?;
            Ok(())
        }
        Ok(_) => Err(fault(
            Fault::Usage,
            anyhow::anyhow!("unix:{} exists and is not a socket", path.display()),
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("unix:{}", path.display())),
    }
}

/// Whether a process serves the unix socket at `path` (issue 115, T-534):
/// the probe of pod-core (3 connects, 50 ms apart; COORD-117,
/// PodUtils/podcrates@T-1209).
#[cfg(unix)]
pub use pod_core::files::socket_answers;

/// The kind of the address of a listener: the next step of a refused bind
/// depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listener {
    /// A TCP or UDP address (HOST:PORT).
    Ip,
    /// A unix socket (unix:PATH) in a directory that this process cannot
    /// write, or whose directory podnet did not test.
    Unix,
    /// A unix socket in a directory that this process can write and search
    /// (`access(2)`): a refused bind is a refusal of the host (T-536).
    UnixWritableDir,
    /// A device or another kind (a TUN or veth wire): the class, no step.
    Other,
    /// A UDP port above 1023 (GitHub #748): a refusal is a host that gives
    /// no UDP, not a port that needs privilege.
    Udp,
    /// A UDP port below 1024.
    UdpPrivileged,
}

/// The kind of the unix listener at `path`: [`Listener::UnixWritableDir`]
/// when this process can write and search the directory of `path`, else
/// [`Listener::Unix`].
#[cfg(unix)]
pub fn unix_listener(path: &std::path::Path) -> Listener {
    use std::os::unix::ffi::OsStrExt as _;
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => std::path::Path::new("."),
    };
    let Ok(text) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return Listener::Unix;
    };
    // SAFETY: access(2) reads the path of a CString that lives to the end
    // of the call.
    if unsafe { libc::access(text.as_ptr(), libc::W_OK | libc::X_OK) } == 0 {
        Listener::UnixWritableDir
    } else {
        Listener::Unix
    }
}

/// The next step for a refused bind of a `listener` with the error `kind`.
/// `None` for an error that is not a refused bind.
pub fn bind_step(listener: Listener, kind: std::io::ErrorKind) -> Option<&'static str> {
    use std::io::ErrorKind::{AddrInUse, AddrNotAvailable, NotFound, PermissionDenied};
    Some(match (listener, kind) {
        (Listener::Ip, AddrInUse) => {
            "another process has this address; give another port (port 0 gives a free one), or a unix socket (unix:PATH) where the command takes one"
        }
        (Listener::Ip, PermissionDenied) => {
            "this host refuses the bind (a port below 1024 needs privilege, and some hosts refuse each TCP listener); give a port above 1023, or a unix socket (unix:PATH) where the command takes one (podnet net doctor)"
        }
        (Listener::Udp | Listener::UdpPrivileged, AddrInUse) => {
            "another process has this UDP port; give another port (port 0 gives a free one)"
        }
        (Listener::Udp, PermissionDenied) => {
            "this host refuses UDP sockets (a seccomp rule or a policy of the host): this command needs UDP and cannot run here (podnet net doctor)"
        }
        (Listener::UdpPrivileged, PermissionDenied) => {
            "a port below 1024 needs privilege; give a port above 1023 (podnet net doctor)"
        }
        (Listener::Ip | Listener::Udp | Listener::UdpPrivileged, AddrNotAvailable) => {
            "no interface of this host has this address; give 127.0.0.1, 0.0.0.0 or an address of this host"
        }
        (Listener::Unix, AddrInUse) => {
            "another socket is at this path; stop its process, or give another path"
        }
        (Listener::Unix, PermissionDenied) => {
            "the directory of the socket is not writable; give a path in a directory that you can write"
        }
        (Listener::Unix, AddrNotAvailable) => "give a path on a local file system",
        (Listener::Unix, NotFound) => {
            "the directory of the socket does not exist; create it, or give a path in a directory that exists"
        }
        (Listener::UnixWritableDir, PermissionDenied) => {
            "the directory of the socket is writable: this host refuses the unix listener (a seccomp or LSM rule); give a TCP address where the command takes one (podnet net doctor)"
        }
        (Listener::UnixWritableDir, kind) => return bind_step(Listener::Unix, kind),
        _ => return None,
    })
}

/// Give a refused bind of a TCP or UDP address (`AddrInUse`,
/// `PermissionDenied`, `AddrNotAvailable` in the chain of `err`) the class
/// `Fault::Unavailable` (exit 69), and the next step (T-515, T-523). Another
/// error stays as it is.
pub fn bind_fault(err: anyhow::Error) -> anyhow::Error {
    bind_fault_of(Listener::Ip, err)
}

/// [`bind_fault`] for a UDP socket on `port` (GitHub #748).
pub fn bind_fault_udp(port: u16) -> impl FnOnce(anyhow::Error) -> anyhow::Error {
    let listener = if (1..1024).contains(&port) {
        Listener::UdpPrivileged
    } else {
        Listener::Udp
    };
    move |err| bind_fault_of(listener, err)
}

/// [`bind_fault`] for a unix socket at `path`. `NotFound` (a directory that
/// does not exist) is a refused bind too. `PermissionDenied` names the
/// directory only when this process cannot write it ([`unix_listener`]).
#[cfg(unix)]
pub fn bind_fault_unix(path: &std::path::Path) -> impl FnOnce(anyhow::Error) -> anyhow::Error {
    let listener = unix_listener(path);
    move |err| bind_fault_of(listener, err)
}

/// [`bind_fault`] for the address kind `listener`. The text is one line: the
/// chain of `err` (each cause one time), `; ` and the next step.
pub fn bind_fault_of(listener: Listener, err: anyhow::Error) -> anyhow::Error {
    use crate::exit::{fault, Fault};
    use std::io::ErrorKind::{AddrInUse, AddrNotAvailable, NotFound, PermissionDenied};
    let refused = |k: &std::io::ErrorKind| match k {
        AddrInUse | PermissionDenied | AddrNotAvailable => true,
        // A unix socket in a directory that does not exist.
        NotFound => matches!(listener, Listener::Unix | Listener::UnixWritableDir),
        _ => false,
    };
    let Some(kind) = err.chain().find_map(|e| {
        e.downcast_ref::<std::io::Error>()
            .map(|io| io.kind())
            .filter(refused)
    }) else {
        return err;
    };
    let Some(step) = bind_step(listener, kind) else {
        return fault(Fault::Unavailable, err);
    };
    let mut text = String::new();
    for cause in err.chain() {
        let cause = cause.to_string();
        if text.is_empty() {
            text = cause;
        } else if !text.contains(&cause) {
            text.push_str(": ");
            text.push_str(&cause);
        }
    }
    if text.contains(step) {
        return fault(Fault::Unavailable, err);
    }
    fault(Fault::Unavailable, anyhow::anyhow!("{text}; {step}"))
}

/// The address of a bound listener (GitHub #750): `getsockname` (`got`),
/// else the address that the command asked for (`asked`) when its port is
/// fixed. With port 0 the port that the kernel gave is not known: exit 69,
/// one line with `label`, the cause and the next step.
pub fn bound_addr(
    got: std::io::Result<std::net::SocketAddr>,
    asked: Option<std::net::SocketAddr>,
    label: &str,
) -> Result<std::net::SocketAddr> {
    let e = match got {
        Ok(addr) => return Ok(addr),
        Err(e) => e,
    };
    if let Some(asked) = asked.filter(|a| a.port() != 0) {
        tracing::debug!(error = %e, "getsockname refused: the address of the command line is used");
        return Ok(asked);
    }
    Err(crate::exit::fault(
        crate::exit::Fault::Unavailable,
        anyhow::anyhow!(
            "{label}: this host refuses getsockname ({e}), so the port that the kernel gave for port 0 is not known; give a fixed port, or a unix socket (unix:PATH) where the command takes one"
        ),
    ))
}

/// Bind a TCP listener. A refused bind exits 69, and so does a loopback
/// address that no connect reaches ([`loopback::check`], GitHub #736).
pub async fn bind_tcp(addr: std::net::SocketAddr) -> Result<tokio::net::TcpListener> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding tcp://{addr}"))
        .map_err(bind_fault)?;
    loopback::check(addr.ip(), &format!("tcp://{addr}"))?;
    Ok(listener)
}

pub fn init_crypto() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    /// GitHub #750: a refused getsockname takes the asked address when its
    /// port is fixed; with port 0 it exits 69 with the label and a step.
    #[test]
    fn a_refused_getsockname_has_a_second_path() {
        use std::net::SocketAddr;
        let refused = || Err(std::io::Error::from_raw_os_error(libc::EOPNOTSUPP));
        let fixed: SocketAddr = "127.0.0.1:18220".parse().unwrap();
        let zero: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert_eq!(
            super::bound_addr(refused(), Some(fixed), "--local").unwrap(),
            fixed
        );
        assert_eq!(
            super::bound_addr(Ok(fixed), Some(zero), "--local").unwrap(),
            fixed
        );
        for asked in [Some(zero), None] {
            let err = super::bound_addr(refused(), asked, "--local 127.0.0.1:0").unwrap_err();
            let line = format!("{err:#}");
            assert_eq!(
                crate::exit::code_of(&err),
                crate::exit::UNAVAILABLE,
                "{line}"
            );
            assert!(
                line.contains("--local 127.0.0.1:0") && line.contains("give a fixed port"),
                "{line}"
            );
        }
    }

    use super::*;

    /// T-515, T-523: a refused bind of each kind of listener is exit 69 and
    /// one line that names the address one time and gives a next step;
    /// another error keeps its text and has no class.
    #[test]
    fn a_refused_bind_is_69_with_one_line_and_a_next_step() {
        use crate::exit::{code_of, FAILURE, UNAVAILABLE};
        use std::io::ErrorKind::{AddrInUse, AddrNotAvailable, NotFound, PermissionDenied};
        let cases = [
            (
                Listener::Ip,
                AddrInUse,
                UNAVAILABLE,
                Some("give another port"),
            ),
            (
                Listener::Ip,
                PermissionDenied,
                UNAVAILABLE,
                Some("unix:PATH"),
            ),
            (
                Listener::Ip,
                AddrNotAvailable,
                UNAVAILABLE,
                Some("give 127.0.0.1"),
            ),
            (
                Listener::Unix,
                AddrInUse,
                UNAVAILABLE,
                Some("give another path"),
            ),
            (
                Listener::Unix,
                PermissionDenied,
                UNAVAILABLE,
                Some("not writable"),
            ),
            (Listener::Other, PermissionDenied, UNAVAILABLE, None),
            (
                Listener::Unix,
                NotFound,
                UNAVAILABLE,
                Some("does not exist"),
            ),
            // T-536: a refused bind in a writable directory is a refusal
            // of the host, not a directory that is not writable.
            (
                Listener::UnixWritableDir,
                PermissionDenied,
                UNAVAILABLE,
                Some("this host refuses the unix listener"),
            ),
            (
                Listener::UnixWritableDir,
                AddrInUse,
                UNAVAILABLE,
                Some("give another path"),
            ),
            (
                Listener::UnixWritableDir,
                NotFound,
                UNAVAILABLE,
                Some("does not exist"),
            ),
            (Listener::Ip, NotFound, FAILURE, None),
            (Listener::Other, NotFound, FAILURE, None),
            // GitHub #748: a refused UDP socket on a high port is a host
            // with no UDP; a low port needs privilege.
            (
                Listener::Udp,
                PermissionDenied,
                UNAVAILABLE,
                Some("this host refuses UDP sockets"),
            ),
            (
                Listener::UdpPrivileged,
                PermissionDenied,
                UNAVAILABLE,
                Some("below 1024 needs privilege"),
            ),
            (Listener::Udp, AddrInUse, UNAVAILABLE, Some("this UDP port")),
        ];
        for (listener, kind, code, step) in cases {
            let err = anyhow::Error::new(std::io::Error::from(kind)).context("binding ADDR");
            let err = bind_fault_of(listener, err);
            let line = format!("{err:#}");
            assert_eq!(code_of(&err), code, "{listener:?} {kind:?}: {line}");
            assert_eq!(
                line.matches("ADDR").count(),
                1,
                "{listener:?} {kind:?}: {line}"
            );
            assert!(!line.contains('\n'), "{listener:?} {kind:?}: {line}");
            match step {
                Some(step) => assert!(line.contains(step), "{listener:?} {kind:?}: {line}"),
                None => assert!(!line.contains("; "), "{listener:?} {kind:?}: {line}"),
            }
            if listener == Listener::Udp {
                assert!(
                    !line.contains("1023") && !line.contains("unix:"),
                    "{kind:?}: {line}"
                );
            }
            if listener == Listener::UnixWritableDir {
                assert!(!line.contains("not writable"), "{kind:?}: {line}");
            }
        }
    }

    /// T-536: `bind_fault_unix` tests the directory of the path. A refused
    /// bind in a directory that this process can write names the host; in
    /// a directory that does not exist or that it cannot write, the
    /// directory.
    #[test]
    fn a_refused_unix_bind_names_the_directory_only_when_it_is_not_writable() {
        use crate::exit::{code_of, UNAVAILABLE};
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o500))
            .unwrap();
        // SAFETY: geteuid has no precondition.
        let root = unsafe { libc::geteuid() } == 0;
        // (the path, its kind, the line has); root writes each directory.
        let cases = [
            (
                dir.path().join("a.sock"),
                Listener::UnixWritableDir,
                "this host refuses the unix listener",
            ),
            (
                dir.path().join("missing/a.sock"),
                Listener::Unix,
                "the directory of the socket is not writable",
            ),
            (
                locked.join("a.sock"),
                if root {
                    Listener::UnixWritableDir
                } else {
                    Listener::Unix
                },
                if root {
                    "this host refuses the unix listener"
                } else {
                    "the directory of the socket is not writable"
                },
            ),
        ];
        for (path, kind, has) in cases {
            assert_eq!(unix_listener(&path), kind, "{}", path.display());
            let err =
                anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                    .context(format!("binding unix:{}", path.display()));
            let err = bind_fault_unix(&path)(err);
            let line = format!("{err:#}");
            assert_eq!(code_of(&err), UNAVAILABLE, "{line}");
            assert!(line.contains(has), "{line}");
        }
    }

    /// T-534: a forked child keeps a copy of the listener of its parent
    /// until its exec. A socket that its server closed is stale, also while
    /// such a child holds it for a short time: the claim takes it. A socket
    /// that its server keeps answers each probe: exit 69.
    #[test]
    fn a_socket_that_a_forked_child_holds_until_its_exec_is_stale() {
        use std::os::unix::process::CommandExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("held.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        // spawn() returns after the exec of the child: spawn on a thread.
        let child = std::thread::spawn(|| {
            let mut command = std::process::Command::new("true");
            // SAFETY: the closure only sleeps (nanosleep) between the fork
            // and the exec; it takes no lock and allocates nothing.
            unsafe {
                command.pre_exec(|| {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    Ok(())
                });
            }
            command.status()
        });
        std::thread::sleep(std::time::Duration::from_millis(15));
        drop(listener);
        let claim = claim_socket_path(&path, false);
        assert!(child.join().unwrap().unwrap().success());
        assert!(claim.is_ok(), "{:#}", claim.unwrap_err());
        assert!(!path.exists());
        let _live = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(socket_answers(&path));
    }

    #[tokio::test]
    async fn a_second_bind_on_one_port_exits_69() {
        pod_testkit::need_loopback!();
        let first = bind_tcp(([127, 0, 0, 1], 0).into()).await.unwrap();
        let err = bind_tcp(first.local_addr().unwrap()).await.unwrap_err();
        assert_eq!(crate::exit::code_of(&err), crate::exit::UNAVAILABLE);
    }

    #[test]
    fn a_claim_gives_69_for_a_live_socket_64_for_a_file_and_removes_a_stale_one() {
        use crate::exit::{code_of, UNAVAILABLE, USAGE};
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
        let err = claim_socket_path(&live, false).unwrap_err();
        assert_eq!(code_of(&err), UNAVAILABLE);
        let file = dir.path().join("plain");
        std::fs::write(&file, b"x").unwrap();
        let err = claim_socket_path(&file, false).unwrap_err();
        assert_eq!(code_of(&err), USAGE);
        assert!(file.exists());
        let stale = dir.path().join("stale.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        claim_socket_path(&stale, false).unwrap();
        assert!(!stale.exists());
    }
}
