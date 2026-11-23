//! The bridge between a blocking serial line and the async link.
//!
//! `pod-link` gives its carrier a `tokio` stream. A serial device is a
//! blocking file descriptor with no runtime of its own. [`pair`] makes one
//! end of a `UnixStream` pair for the link and gives the other end to one
//! detached thread, which polls the device and the socket.
//!
//! The thread owns the line. It polls both descriptors, so the bytes of
//! each direction move in one loop and neither blocks the other. A write
//! to the socket is small (the bytes of the link that the ARQ released),
//! and the socket buffer holds it.

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The two ends of one bridge.
pub struct Pair {
    /// The end of the link. It is async.
    pub app: tokio::net::UnixStream,
    /// The end of the thread that owns the line. It is blocking.
    pub device: StdUnixStream,
}

/// Make the two ends. The device end is blocking; the link end is not.
pub fn pair() -> io::Result<Pair> {
    let (app, device) = tokio::net::UnixStream::pair()?;
    let device = device.into_std()?;
    device.set_nonblocking(false)?;
    Ok(Pair { app, device })
}

/// The most bytes of one read of the line.
pub const READ_MOST: usize = 4096;

/// The last failure of a bridge, for a caller that asks after the stream
/// ended.
pub type Last = Arc<Mutex<Option<String>>>;

/// A place to keep the last failure of a bridge.
pub fn last() -> Last {
    Arc::new(Mutex::new(None))
}

/// Record `why` as the failure of a bridge.
pub fn note(last: &Last, why: impl Into<String>) {
    *last.lock().unwrap_or_else(|e| e.into_inner()) = Some(why.into());
}

/// Read the last failure of a bridge, when there is one.
pub fn failure(last: &Last) -> Option<String> {
    last.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The descriptor of the open device at `path`.
///
/// `pod_offline::serial::SerialLine` keeps its file private and gives no
/// descriptor, so this finds the descriptor that the line opened by
/// matching the links of `/proc/self/fd` with the canonical path. It is a
/// descriptor, not a second serial implementation: the settings, the
/// reads and the writes stay in `pod-offline`.
pub fn device_fd(path: &str) -> io::Result<RawFd> {
    let want = std::fs::canonicalize(path)?;
    for entry in std::fs::read_dir("/proc/self/fd")? {
        let entry = entry?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };
        let fd: RawFd = match name.parse() {
            Ok(fd) => fd,
            Err(_) => continue,
        };
        if fd <= 2 {
            continue;
        }
        let target = match std::fs::read_link(entry.path()) {
            Ok(target) => target,
            Err(_) => continue,
        };
        if target == want {
            return Ok(fd);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("{path}: the open device has no descriptor"),
    ))
}

/// Wait until one of the two descriptors is readable, or the time ends.
/// It gives `(device, socket)`.
pub fn wait_readable(device: RawFd, socket: RawFd, timeout: Duration) -> io::Result<(bool, bool)> {
    let mut fds = [
        libc::pollfd {
            fd: device,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: socket,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: poll reads the array that this function owns, and the two
    // descriptors are open for the life of the call.
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) };
    if ready < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok((false, false));
        }
        return Err(error);
    }
    Ok((
        fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0,
        fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0,
    ))
}

/// Read up to [`READ_MOST`] bytes of `line`.
pub fn read_line(line: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; READ_MOST];
    let count = line.read(&mut buf)?;
    buf.truncate(count);
    Ok(buf)
}

/// Write each byte of `bytes` to the socket and flush it.
pub fn write_socket(socket: &mut StdUnixStream, bytes: &[u8]) -> io::Result<()> {
    socket.write_all(bytes)?;
    socket.flush()
}

/// Write each byte of `bytes` to the line and flush it.
pub fn write_line(line: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    line.write_all(bytes)?;
    line.flush()
}

/// The descriptor of the socket that the thread owns.
pub fn socket_fd(socket: &StdUnixStream) -> RawFd {
    socket.as_raw_fd()
}
