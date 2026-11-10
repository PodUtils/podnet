//! The listen and accept calls of the program under the seccomp backend.
//! A listener of the program is a unix socket in the inbound directory.

#[allow(unused_imports)]
use crate::prelude::*;

/// A listening socket inside the sandbox. Connections arrive on a unix
/// listener (the only bind a sealed host permits) and are handed to the child's
/// `accept` as injected socketpair ends.
/// The queue of a listener of the child. The child's listener fd is one end
/// of a socketpair: for each queued connection the reader task writes one
/// byte to the other end, so poll, select and epoll on the listener wake.
/// Each accept takes one byte back through `drain` (a copy of the child's
/// end), so the listener is readable while a connection waits, and only
/// then.
pub(crate) struct Inbound {
    pub(crate) rx:
        Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<tokio::net::UnixStream>>>,
    pub(crate) drain: Arc<std::os::unix::net::UnixStream>,
}

/// `listen` creates the real listener the sandbox can have: a unix socket named
/// after the port. A reader task turns accepted connections into a queue that
/// `accept` draws from, and makes the child's listener fd readable while a
/// connection waits (see [`Inbound`]).
pub(crate) async fn handle_listen(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    (map, key): (&mut HashMap<Key, Sock>, Key),
    inbound_dir: &std::path::Path,
    pid: u32,
) -> Result<()> {
    let fd = a[0] as i32;
    let Some(Sock::Stream {
        bound: Some((_, port)),
        ..
    }) = map.get(&key)
    else {
        return notif_continue(lfd, id);
    };
    let port = *port;
    crate::listen::private_dir(inbound_dir)?;
    let path = inbound_dir.join(format!("{port}.sock"));
    let _ = std::fs::remove_file(&path);
    let listener = crate::listen::bind_private(&path)
        .with_context(|| format!("bind inbound listener {}", path.display()))?;

    // The new listener fd of the child: one end of a socketpair, with the
    // O_NONBLOCK and close-on-exec of the old fd.
    let flags = child_fd_flags(pid, fd).unwrap_or(0);
    let (signal, child_end) = std::os::unix::net::UnixStream::pair()?;
    if flags & libc::O_NONBLOCK != 0 {
        set_nonblocking(child_end.as_raw_fd())?;
    }
    let drain = child_end.try_clone()?;
    let newfd_flags = if flags & libc::O_CLOEXEC != 0 {
        libc::O_CLOEXEC as u32
    } else {
        0
    };
    // The new end is a new socket: the entry moves to its key.
    let new_key = crate::fdtable::inode_of(child_end.as_raw_fd())?;
    notif_addfd_setfd(lfd, id, child_end.as_raw_fd(), fd, newfd_flags)?;
    drop(child_end);
    if let Some(entry) = map.remove(&key) {
        map.insert(new_key, entry);
    }
    set_nonblocking(signal.as_raw_fd())?;
    let mut signal = tokio::net::UnixStream::from_std(signal)?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            // The byte first: when accept finds the connection, its byte is
            // there to take.
            if tokio::io::AsyncWriteExt::write_all(&mut signal, &[1])
                .await
                .is_err()
            {
                break;
            }
            if tx.send(conn).is_err() {
                break;
            }
        }
    });
    if let Some(Sock::Stream { inbound, .. }) = map.get_mut(&new_key) {
        *inbound = Some(Inbound {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
            drain: Arc::new(drain),
        });
    }
    notif_respond(lfd, id, 0, 0)?;
    // The listener is a real unix socket; say where it is.
    eprintln!(
        "{}: listening on unix:{} (port {port})",
        crate::cli::program_name(),
        path.display()
    );
    Ok(())
}

/// Take the byte of one queued connection from the child's listener.
fn drain_one(drain: &std::os::unix::net::UnixStream) {
    let mut byte = [0u8; 1];
    unsafe {
        libc::recv(
            drain.as_raw_fd(),
            byte.as_mut_ptr() as *mut libc::c_void,
            1,
            libc::MSG_DONTWAIT,
        )
    };
}

/// Give the connection `conn` to the child as the answer of `accept`.
fn install_accepted(
    lfd: RawFd,
    id: u64,
    conn: tokio::net::UnixStream,
    nonblocking: bool,
    newfd_flags: u32,
) -> Result<()> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    set_nonblocking(ours.as_raw_fd())?;
    if nonblocking {
        set_nonblocking(theirs.as_raw_fd())?;
    }
    // ADDFD with SEND already answers the syscall with the installed fd.
    notif_addfd_send(lfd, id, theirs.as_raw_fd(), newfd_flags)?;
    drop(theirs);
    let mut ours = tokio::net::UnixStream::from_std(ours)?;
    let mut conn = conn;
    spawn_relay(async move {
        let _ = tokio::io::copy_bidirectional(&mut ours, &mut conn).await;
    });
    Ok(())
}

/// `accept` takes a queued connection at once. With none queued, a
/// non-blocking listener gets EAGAIN, and a blocking accept waits in its own
/// task: the other system calls of the child go on (before, one accept that
/// waited stopped each other call of the child).
pub(crate) async fn handle_accept(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    (map, key): (&mut HashMap<Key, Sock>, Key),
    accept4: bool,
    pid: u32,
) -> Result<()> {
    let fd = a[0] as i32;
    // The caller's flags are on accept4 only; ADDFD takes just O_CLOEXEC.
    let flags = if accept4 { a[3] as i32 } else { 0 };
    let mut newfd_flags = 0u32;
    if flags & libc::SOCK_CLOEXEC != 0 {
        newfd_flags |= libc::O_CLOEXEC as u32;
    }
    let nonblocking_conn = flags & libc::SOCK_NONBLOCK != 0;
    let (rx, drain) = {
        let Some(Sock::Stream {
            inbound: Some(inc), ..
        }) = map.get(&key)
        else {
            return notif_continue(lfd, id);
        };
        (inc.rx.clone(), inc.drain.clone())
    };
    let queued = rx.try_lock().ok().and_then(|mut rx| rx.try_recv().ok());
    if let Some(conn) = queued {
        drain_one(&drain);
        return install_accepted(lfd, id, conn, nonblocking_conn, newfd_flags);
    }
    let listener_nonblocking = child_fd_flags(pid, fd).is_some_and(|f| f & libc::O_NONBLOCK != 0);
    if listener_nonblocking {
        return notif_respond(lfd, id, -1, libc::EAGAIN);
    }
    tokio::spawn(async move {
        let conn = rx.lock().await.recv().await;
        match conn {
            Some(conn) => {
                drain_one(&drain);
                // ENOENT: the child left the accept (a signal); the
                // connection is closed.
                let _ = install_accepted(lfd, id, conn, nonblocking_conn, newfd_flags);
            }
            None => {
                let _ = notif_respond(lfd, id, -1, libc::EINVAL);
            }
        }
    });
    Ok(())
}
