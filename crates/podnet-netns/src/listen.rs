//! Listen addresses for the proxy front doors.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

/// Where the proxy accepts clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyListen {
    /// An `AF_UNIX` socket, usually an abstract-free filesystem path.
    Unix(PathBuf),
    /// A real TCP listener, for a host that permits `AF_INET` binds.
    Tcp(SocketAddr),
}

impl ProxyListen {
    /// Parse `unix:/path`, `unix:///path` or `tcp://host:port`.
    pub fn parse(value: &str) -> Result<Self> {
        if let Some(path) = value
            .strip_prefix("unix://")
            .or_else(|| value.strip_prefix("unix:"))
        {
            if path.is_empty() {
                bail!("unix: listen address has no path");
            }
            // unix:///tmp/x is /tmp/x, not ///tmp/x (issue 95).
            let path = if path.starts_with('/') {
                format!("/{}", path.trim_start_matches('/'))
            } else {
                path.to_string()
            };
            return Ok(Self::Unix(PathBuf::from(path)));
        }
        for prefix in ["tcp://", "tcp:"] {
            if let Some(rest) = value.strip_prefix(prefix) {
                return Ok(Self::Tcp(
                    rest.parse()
                        .with_context(|| format!("bad TCP listen address {rest:?}"))?,
                ));
            }
        }
        if let Ok(addr) = value.parse::<SocketAddr>() {
            return Ok(Self::Tcp(addr));
        }
        bail!("listen address {value:?} is not unix:/path or tcp://host:port")
    }
}

impl std::fmt::Display for ProxyListen {
    /// `unix:PATH` or `tcp://HOST:PORT`, the form that [`ProxyListen::parse`]
    /// reads.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "unix:{}", path.display()),
            Self::Tcp(addr) => write!(f, "tcp://{addr}"),
        }
    }
}

/// The default directory of the inbound sockets of the seccomp backend
/// (issue 306): `$XDG_RUNTIME_DIR/podnet-netns-inbound` when that directory is
/// of this user, else `podnet-netns-inbound-UID` in the temporary directory.
/// Before, the default was `/tmp/podnet-netns-inbound`, one name for all users.
pub fn default_inbound_dir() -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid() has no precondition.
    let uid = unsafe { libc::geteuid() };
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let runtime = PathBuf::from(runtime);
        let mine = std::fs::metadata(&runtime)
            .map(|m| m.is_dir() && m.uid() == uid)
            .unwrap_or(false);
        if runtime.is_absolute() && mine {
            return runtime.join("podnet-netns-inbound");
        }
    }
    std::env::temp_dir().join(format!("podnet-netns-inbound-{uid}"))
}

/// Make `dir` a private directory of this user, or check the directory that
/// is there (issue 306). A new directory gets mode 0700. A directory that
/// is there must be a directory (not a symbolic link), of the effective
/// user, and not writable by the group or by others. Each other directory
/// is an error: another user could put a socket in it first.
pub fn private_dir(dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("making the directory {}", parent.display()))?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).with_context(|| format!("making the directory {}", dir.display())),
    }
    let meta = std::fs::symlink_metadata(dir)
        .with_context(|| format!("reading the directory {}", dir.display()))?;
    // SAFETY: geteuid() has no precondition.
    let uid = unsafe { libc::geteuid() };
    if !meta.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    if meta.uid() != uid {
        bail!(
            "the directory {} is of uid {}, not of this user (uid {uid}): give --inbound-dir with a directory of your own",
            dir.display(),
            meta.uid()
        );
    }
    if meta.mode() & 0o022 != 0 {
        bail!(
            "the directory {} is writable by the group or by others (mode {:o}): remove that permission (chmod go-w), or give another --inbound-dir",
            dir.display(),
            meta.mode() & 0o7777
        );
    }
    Ok(())
}

/// Bind a Unix socket at `path` that only its owner can use: mode 0600. A
/// client must have write permission on the socket to connect. The socket
/// is bound in a new directory of mode 0700 beside `path` and moved to
/// `path` with no replace (T-238), so it never exists at `path` with the
/// mode of the umask; its own name (`getsockname`) is the staging name.
/// An existing `path` gives `AddrInUse`.
pub fn bind_private(
    path: impl AsRef<std::path::Path>,
) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    let path = path.as_ref();
    // T-238: the socket must never exist at the mode of the umask at its
    // final path. Bind it in a new directory of mode 0700 beside the
    // path, set 0600, then move it to the path with no replace.
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let dir = parent.join(format!(".pnb-{}", random_hex()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let staged = dir.join("s");
    let bound = tokio::net::UnixListener::bind(&staged).and_then(|listener| {
        #[cfg(test)]
        tests::after_bind(&staged, path);
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))?;
        move_no_replace(&staged, path)?;
        Ok(listener)
    });
    let _ = std::fs::remove_file(&staged);
    let _ = std::fs::remove_dir(&dir);
    match bound {
        Ok(listener) => Ok(listener),
        // A long path: the staged name does not fit in a sockaddr_un. Bind
        // at the path, as before, and set the mode at once.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => bind_in_private_dir(path),
        Err(e) => Err(e),
    }
}

/// Bind a unix listener at `path` in a directory that only this user can
/// reach (`private_dir`), and set the mode 0600. The socket keeps the name
/// `path` (`getsockname`, the name of an accepted socket): the seccomp
/// inbound listener needs it (T-150). In a shared directory, use
/// [`bind_private`].
pub fn bind_in_private_dir(path: &std::path::Path) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt as _;
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// 16 hex digits from the kernel's random source.
fn random_hex() -> String {
    let mut bytes = [0u8; 8];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| {
        use std::io::Read as _;
        f.read_exact(&mut bytes)
    });
    let pid = u64::from(std::process::id());
    format!("{:016x}", u64::from_le_bytes(bytes) ^ pid)
}

/// Move `from` to `to` when `to` does not exist: `renameat2` with
/// `RENAME_NOREPLACE`, else (a file system with no such flag) `link` and
/// `unlink`. An existing `to` is `AddrInUse`.
fn move_no_replace(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    let (a, b) = (
        std::ffi::CString::new(from.as_os_str().as_bytes())?,
        std::ffi::CString::new(to.as_os_str().as_bytes())?,
    );
    const RENAME_NOREPLACE: libc::c_uint = 1;
    // SAFETY: two NUL-terminated paths, relative to the working directory.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    let err = std::io::Error::last_os_error();
    let in_use = |e: std::io::Error| match e.raw_os_error() {
        Some(libc::EEXIST) => std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            format!("unix:{} exists", to.display()),
        ),
        _ => e,
    };
    if rc == 0 {
        return Ok(());
    }
    match err.raw_os_error() {
        Some(libc::ENOSYS) | Some(libc::EINVAL) => {
            std::fs::hard_link(from, to).map_err(in_use)?;
            std::fs::remove_file(from)
        }
        _ => Err(in_use(err)),
    }
}

/// Make `path` free for a new unix listener. A socket that answers belongs
/// to another process: `AddrInUse`. A file that is not a socket is never
/// removed: `InvalidInput`. A stale socket is removed.
pub fn claim_unix_path(path: &std::path::Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    use std::os::unix::fs::FileTypeExt as _;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(Error::new(
                    ErrorKind::AddrInUse,
                    format!("unix:{} is served by another process", path.display()),
                ));
            }
            std::fs::remove_file(path)
        }
        Ok(_) => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("unix:{} exists and is not a socket", path.display()),
        )),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// [`claim_unix_path`], then [`bind_private`].
pub fn bind_claimed(path: &std::path::Path) -> std::io::Result<tokio::net::UnixListener> {
    claim_unix_path(path)?;
    bind_private(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Seam = Box<dyn Fn(&std::path::Path, &std::path::Path)>;

    thread_local! {
        /// T-238: a seam that runs after the bind and before the chmod.
        static AFTER_BIND: std::cell::RefCell<Option<Seam>> = const { std::cell::RefCell::new(None) };
    }

    pub(super) fn after_bind(staged: &std::path::Path, path: &std::path::Path) {
        AFTER_BIND.with(|hook| {
            if let Some(hook) = hook.borrow().as_ref() {
                hook(staged, path);
            }
        });
    }

    fn mode(path: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// T-238: between the bind and the chmod, the socket is in a directory
    /// of mode 0700 and the final path does not exist yet.
    #[test]
    fn bind_private_is_never_wider_than_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("door.sock");
        let seen = std::rc::Rc::new(std::cell::Cell::new(false));
        let mark = seen.clone();
        AFTER_BIND.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |staged, path| {
                assert_eq!(
                    mode(staged.parent().unwrap()),
                    0o700,
                    "the staging directory"
                );
                assert!(!path.exists(), "the final path exists before the chmod");
                mark.set(true);
            }));
        });
        let rt = runtime();
        let _listener = rt.block_on(async { bind_private(&path) }).unwrap();
        AFTER_BIND.with(|hook| *hook.borrow_mut() = None);
        assert!(seen.get(), "the seam ran");
        assert_eq!(mode(&path), 0o600);
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left, ["door.sock"], "no staging directory stays");
        std::os::unix::net::UnixStream::connect(&path).expect("the moved socket answers");
    }

    /// T-238: under each umask the socket has the mode 0600 when
    /// bind_private returns. The umask is of the process: each case runs in
    /// a new process (this test binary again), not a fork, so that no
    /// descriptor of a parallel test leaks into it.
    #[test]
    fn bind_private_gives_0600_under_each_umask() {
        if let Ok(umask) = std::env::var("PODNET_TEST_UMASK") {
            let umask = u32::from_str_radix(&umask, 8).unwrap();
            // SAFETY: the umask of this child process, which runs this test only.
            unsafe { libc::umask(umask as libc::mode_t) };
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("u.sock");
            let rt = runtime();
            let _l = rt.block_on(async { bind_private(&path) }).unwrap();
            assert_eq!(mode(&path), 0o600, "umask {umask:03o}");
            return;
        }
        for umask in ["000", "002", "022"] {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "listen::tests::bind_private_gives_0600_under_each_umask",
                    "--nocapture",
                ])
                .env("PODNET_TEST_UMASK", umask)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "umask {umask}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// T-238: two binds of one path at the same time: one wins, the other
    /// gets AddrInUse, and the socket of the winner stays.
    #[test]
    fn two_bind_private_calls_on_one_path_give_one_winner() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("one.sock"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let rt = runtime();
                    barrier.wait();
                    let result = rt.block_on(async { bind_private(path.as_path()) });
                    // Keep the winner alive until both have ended.
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    result.map(|_| ()).map_err(|e| e.kind())
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        assert!(
            results.contains(&Err(std::io::ErrorKind::AddrInUse)),
            "{results:?}"
        );
        assert!(path.exists(), "the socket of the winner stays");
    }

    #[test]
    fn a_claim_refuses_a_live_socket_and_a_file_and_removes_a_stale_socket() {
        use std::io::ErrorKind;
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
        let err = claim_unix_path(&live).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AddrInUse);
        assert!(live.exists());

        let file = dir.path().join("plain");
        std::fs::write(&file, b"data").unwrap();
        let err = claim_unix_path(&file).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(file.exists());

        let stale = dir.path().join("stale.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        assert!(stale.exists());
        claim_unix_path(&stale).unwrap();
        assert!(!stale.exists());
        claim_unix_path(&dir.path().join("none")).unwrap();
    }

    #[test]
    fn a_unix_url_with_three_slashes_is_an_absolute_path() {
        for value in [
            "unix:///tmp/x.sock",
            "unix://tmp/x.sock",
            "unix:/tmp/x.sock",
            "unix:////tmp/x.sock",
        ] {
            let want = if value == "unix://tmp/x.sock" {
                "tmp/x.sock"
            } else {
                "/tmp/x.sock"
            };
            assert_eq!(
                ProxyListen::parse(value).unwrap(),
                ProxyListen::Unix(PathBuf::from(want)),
                "{value}"
            );
        }
    }

    #[test]
    fn parses_listen_addresses() {
        assert_eq!(
            ProxyListen::parse("unix:/tmp/x.sock").unwrap(),
            ProxyListen::Unix(PathBuf::from("/tmp/x.sock"))
        );
        assert_eq!(
            ProxyListen::parse("tcp://127.0.0.1:1080").unwrap(),
            ProxyListen::Tcp("127.0.0.1:1080".parse().unwrap())
        );
        assert_eq!(
            ProxyListen::parse("127.0.0.1:1080").unwrap(),
            ProxyListen::Tcp("127.0.0.1:1080".parse().unwrap())
        );
        assert!(ProxyListen::parse("nonsense").is_err());
        assert!(ProxyListen::parse("unix:").is_err());
    }

    /// Issue 306: a new inbound directory has mode 0700. A directory that
    /// the group or others can write, a symbolic link, and (as root) a
    /// directory of another user are refused.
    #[test]
    fn the_inbound_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let new = base.path().join("a/inbound");
        private_dir(&new).unwrap();
        let mode = std::fs::metadata(&new).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        private_dir(&new).unwrap();

        let open = base.path().join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = format!("{:#}", private_dir(&open).unwrap_err());
        assert!(err.contains("writable by the group or by others"), "{err}");

        let link = base.path().join("link");
        std::os::unix::fs::symlink(&new, &link).unwrap();
        let err = format!("{:#}", private_dir(&link).unwrap_err());
        assert!(err.contains("not a directory"), "{err}");

        // SAFETY: geteuid() has no precondition.
        if unsafe { libc::geteuid() } == 0 {
            let foreign = base.path().join("foreign");
            std::fs::create_dir(&foreign).unwrap();
            std::os::unix::fs::chown(&foreign, Some(65534), Some(65534)).unwrap();
            let err = format!("{:#}", private_dir(&foreign).unwrap_err());
            assert!(err.contains("not of this user"), "{err}");
        }
    }

    /// Issue 306: the default inbound directory is not one name for all
    /// users.
    #[test]
    fn the_default_inbound_directory_is_of_this_user() {
        let dir = default_inbound_dir();
        assert_ne!(dir, PathBuf::from("/tmp/podnet-netns-inbound"));
        // SAFETY: geteuid() has no precondition.
        let uid = unsafe { libc::geteuid() };
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name == "podnet-netns-inbound" || name == format!("podnet-netns-inbound-{uid}"),
            "{}",
            dir.display()
        );
    }
}
