//! The wire of a guest (T-248): a source and a sink of IP packets, one
//! packet for each read and each write. The wire is a TUN device
//! (`tun:NAME`) or a unix socket of type `SOCK_SEQPACKET` (`unix:/PATH`),
//! where each message is one IPv4 packet.

use std::fmt;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use tokio::io::unix::AsyncFd;

/// The longest packet that a read takes.
pub const PACKET_MAX: usize = 65536;

/// The wire of a guest, as the file names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireSpec {
    /// `unix:/PATH`: the router listens on a `SOCK_SEQPACKET` socket. One
    /// guest is connected at a time; a new connection replaces the old one.
    Unix(PathBuf),
    /// `tun:NAME`: the router opens the TUN device `NAME`. The operator
    /// sets its address and its routes.
    Tun(String),
}

impl FromStr for WireSpec {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        if let Some(path) = text.strip_prefix("unix:") {
            if path.is_empty() || path.contains('\0') {
                bail!("give unix:/PATH");
            }
            return Ok(WireSpec::Unix(PathBuf::from(path)));
        }
        if let Some(name) = text.strip_prefix("tun:") {
            if name.is_empty() || name.len() >= libc::IF_NAMESIZE || name.contains(['\0', '/']) {
                bail!("give tun:NAME (1 to 15 bytes, no /)");
            }
            return Ok(WireSpec::Tun(name.to_string()));
        }
        bail!("{text:?}: give unix:/PATH or tun:NAME")
    }
}

impl fmt::Display for WireSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireSpec::Unix(path) => write!(f, "unix:{}", path.display()),
            WireSpec::Tun(name) => write!(f, "tun:{name}"),
        }
    }
}

/// A wire that is open.
pub enum Opened {
    /// A descriptor of packets: the TUN, or one end of a socket pair.
    Packets(OwnedFd),
    /// The listener of a `unix:` wire.
    Listener(OwnedFd),
}

impl WireSpec {
    /// Open the wire. A `unix:` path that a router serves is an error unless
    /// `replace`; an old socket file that nothing serves is removed.
    pub fn open(&self, replace: bool) -> Result<Opened> {
        match self {
            WireSpec::Unix(path) => {
                crate::net::claim_socket_path(path, replace)?;
                bind_seqpacket(path)
                    .map(Opened::Listener)
                    .with_context(|| format!("unix:{}", path.display()))
            }
            WireSpec::Tun(name) => open_tun(name)
                .map(Opened::Packets)
                .with_context(|| format!("tun:{name}")),
        }
    }
}

fn last_os(what: &str) -> io::Error {
    let e = io::Error::last_os_error();
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

/// Open the TUN device `name`: one IP packet for each read and each write.
pub fn open_tun(name: &str) -> io::Result<OwnedFd> {
    // SAFETY: a valid path; the flags are constants.
    let fd = unsafe {
        libc::open(
            c"/dev/net/tun".as_ptr(),
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(last_os("open /dev/net/tun"));
    }
    // SAFETY: a new descriptor that nothing else owns.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut ifr = pod_hold::netns::ifreq_for(name);
    ifr.ifr_ifru.ifru_flags =
        (pod_hold::netns::IFF_TUN | pod_hold::netns::IFF_NO_PI) as libc::c_short;
    // SAFETY: a valid ifreq for TUNSETIFF.
    let rc = unsafe {
        libc::ioctl(
            owned.as_raw_fd(),
            pod_hold::abi::TUNSETIFF as _,
            &mut ifr as *mut libc::ifreq,
        )
    };
    if rc < 0 {
        return Err(last_os("ioctl TUNSETIFF"));
    }
    Ok(owned)
}

/// A pair of connected `SOCK_SEQPACKET` sockets (non-blocking): the wire of a
/// test, and any program that holds one end.
pub fn seqpacket_pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: `fds` has room for two descriptors.
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    if rc < 0 {
        return Err(last_os("socketpair"));
    }
    // SAFETY: two new descriptors that nothing else owns.
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

fn sockaddr_un(path: &Path) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    use std::os::unix::ffi::OsStrExt as _;
    // SAFETY: a zeroed sockaddr_un is valid.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= addr.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path of the socket is too long",
        ));
    }
    for (slot, byte) in addr.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    let len = std::mem::size_of::<libc::sa_family_t>() + bytes.len() + 1;
    Ok((addr, len as libc::socklen_t))
}

/// Listen on a `SOCK_SEQPACKET` socket at `path` with the mode 0600. The
/// socket never exists at `path` with the mode of the umask: it is bound in
/// a new directory of mode 0700, set to 0600, and then moved to `path`.
pub fn bind_seqpacket(path: &Path) -> io::Result<OwnedFd> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = parent.join(format!(".pnb-{}-{nanos}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let staged = dir.join("s");
    let bound = (|| {
        // SAFETY: a socket with constant arguments.
        let fd = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
            )
        };
        if fd < 0 {
            return Err(last_os("socket"));
        }
        // SAFETY: a new descriptor that nothing else owns.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let (addr, len) = sockaddr_un(&staged)?;
        // SAFETY: `addr` is a valid sockaddr_un of `len` bytes.
        if unsafe { libc::bind(fd, &addr as *const _ as *const libc::sockaddr, len) } < 0 {
            return Err(last_os("bind"));
        }
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))?;
        // SAFETY: a bound socket.
        if unsafe { libc::listen(fd, 4) } < 0 {
            return Err(last_os("listen"));
        }
        std::fs::rename(&staged, path)?;
        Ok(owned)
    })();
    let _ = std::fs::remove_file(&staged);
    let _ = std::fs::remove_dir(&dir);
    bound
}

/// A descriptor of packets with a wait for readiness.
pub struct PacketFd {
    fd: AsyncFd<OwnedFd>,
    /// A socket: a write uses `send` with `MSG_NOSIGNAL`.
    socket: bool,
}

impl PacketFd {
    /// Take `fd`. Call it inside a runtime of tokio.
    pub fn new(fd: OwnedFd) -> io::Result<Self> {
        let raw = fd.as_raw_fd();
        // SAFETY: fcntl on a descriptor that this value owns.
        unsafe {
            let flags = libc::fcntl(raw, libc::F_GETFL);
            if flags < 0 || libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(last_os("fcntl O_NONBLOCK"));
            }
        }
        let mut kind = 0 as libc::c_int;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `kind` has room for an int.
        let socket = unsafe {
            libc::getsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                &mut kind as *mut _ as *mut libc::c_void,
                &mut len,
            )
        } == 0;
        Ok(Self {
            // SAFETY: the AsyncFd owns the descriptor and keeps it open until
            // it drops.
            fd: unsafe { AsyncFd::register(fd) }.map_err(io::Error::from)?,
            socket,
        })
    }

    /// Read one packet into `buf`; its length. 0: the peer closed.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut ready = self.fd.readable().await?;
            // SAFETY: `buf` is a valid buffer of its length.
            let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n >= 0 {
                return Ok(n as usize);
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => ready.clear_ready(),
                io::ErrorKind::Interrupted => {}
                _ => return Err(e),
            }
        }
    }

    /// Write one packet. A packet that the peer's queue cannot hold waits for
    /// room.
    pub async fn send(&self, packet: &[u8]) -> io::Result<()> {
        loop {
            let mut ready = self.fd.writable().await?;
            let fd = self.fd.as_raw_fd();
            // SAFETY: `packet` is a valid buffer of its length.
            let n = unsafe {
                match self.socket {
                    true => {
                        libc::send(fd, packet.as_ptr().cast(), packet.len(), libc::MSG_NOSIGNAL)
                    }
                    false => libc::write(fd, packet.as_ptr().cast(), packet.len()),
                }
            };
            if n >= 0 {
                return Ok(());
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => ready.clear_ready(),
                io::ErrorKind::Interrupted => {}
                _ => return Err(e),
            }
        }
    }
}

/// The listener of a `unix:` wire.
pub struct Listener(AsyncFd<OwnedFd>);

impl Listener {
    pub fn new(fd: OwnedFd) -> io::Result<Self> {
        // SAFETY: the AsyncFd owns the descriptor and keeps it open until
        // it drops.
        Ok(Self(
            unsafe { AsyncFd::register(fd) }.map_err(io::Error::from)?,
        ))
    }

    /// Wait for a guest to connect.
    pub async fn accept(&self) -> io::Result<PacketFd> {
        loop {
            let mut ready = self.0.readable().await?;
            // SAFETY: accept4 on a listening socket; the peer address is not read.
            let fd = unsafe {
                libc::accept4(
                    self.0.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                )
            };
            if fd >= 0 {
                // SAFETY: a new descriptor that nothing else owns.
                return PacketFd::new(unsafe { OwnedFd::from_raw_fd(fd) });
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => ready.clear_ready(),
                io::ErrorKind::Interrupted => {}
                _ => return Err(e),
            }
        }
    }
}

/// Connect to a `unix:` wire (a guest program, and the tests).
pub fn connect_seqpacket(path: &Path) -> io::Result<OwnedFd> {
    // SAFETY: a socket with constant arguments.
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(last_os("socket"));
    }
    // SAFETY: a new descriptor that nothing else owns.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let (addr, len) = sockaddr_un(path)?;
    // SAFETY: `addr` is a valid sockaddr_un of `len` bytes.
    if unsafe { libc::connect(fd, &addr as *const _ as *const libc::sockaddr, len) } < 0 {
        return Err(last_os("connect"));
    }
    Ok(owned)
}
