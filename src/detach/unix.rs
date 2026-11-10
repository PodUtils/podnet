use std::os::fd::AsRawFd;

/// Close each descriptor from 3; reset the signal mask and the actions
/// of INT, TERM and QUIT; name the process `podnet`.
pub(super) fn prepare() {
    // SAFETY: the system call closes descriptors of this process; no
    // thread runs yet, so no descriptor is in use.
    let closed = unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) } == 0;
    if !closed {
        let fds: Vec<i32> = std::fs::read_dir("/proc/self/fd")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .filter(|fd| *fd >= 3)
            .collect();
        for fd in fds {
            // SAFETY: as above.
            unsafe { libc::close(fd) };
        }
    }
    // SAFETY: a name of 4 bytes and its NUL; an empty signal set and
    // the default action of three signals, before any thread.
    unsafe {
        libc::prctl(libc::PR_SET_NAME, c"podnet".as_ptr());
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGQUIT] {
            libc::signal(signal, libc::SIG_DFL);
        }
    }
}

/// Stdin to `/dev/null`, stderr to the log; `setsid()`, SIGHUP
/// ignored; `line` to the parent; stdout to the log. Whether the line
/// went.
pub(super) fn leave(log: Option<&std::fs::File>, line: &[u8]) -> bool {
    let Ok(null) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
    else {
        return false;
    };
    let sink = log.map_or(null.as_raw_fd(), |f| f.as_raw_fd());
    // SAFETY: dup2 of open files onto the standard descriptors of this
    // process; setsid has no precondition (a child is not the leader
    // of a group); the action of SIGHUP.
    unsafe {
        libc::dup2(null.as_raw_fd(), libc::STDIN_FILENO);
        libc::dup2(sink, libc::STDERR_FILENO);
        libc::setsid();
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let said = write_all(libc::STDOUT_FILENO, line);
    // SAFETY: as above.
    unsafe { libc::dup2(sink, libc::STDOUT_FILENO) };
    said
}

fn write_all(fd: i32, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        // SAFETY: a write of a live slice to a descriptor of this process.
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n <= 0 {
            return false;
        }
        bytes = &bytes[n as usize..];
    }
    true
}
