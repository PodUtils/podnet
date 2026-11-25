//! The lines of a detached child before its ready, in its log too (GitHub
//! #714). The tee of the log lines ([`crate::report::set_log_tee`]) copies
//! the lines of `tracing` only; a line that a command prints itself (its
//! banner on stdout: the address, the URL; a line on stderr) did not reach
//! the log.
//!
//! [`start`] puts a pipe at stdout and at stderr. A thread for each copies
//! each line to the descriptor of before as it is, and to the log masked and
//! with no colour codes. At the ready, [`stop`] puts the descriptors of
//! before back and waits for the threads (1 s at most: a program that the
//! command started can hold a pipe). An exit before the ready runs [`stop`]
//! too (`atexit`), so the last lines (the error) reach the caller. When a
//! pipe cannot be made, its descriptor stays as before, and the caller keeps
//! the tee.

use std::fs::File;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::FromRawFd as _;
use std::sync::Mutex;
use std::time::Duration;

use std::sync::mpsc::Receiver;

/// For each copied descriptor: its number, the descriptor of before, and
/// the end of its thread.
static COPY: Mutex<Vec<(i32, i32, Receiver<()>)>> = Mutex::new(Vec::new());

/// Copy stdout and stderr to their descriptors of before and to `log`, from
/// now to [`stop`]. True when the copy of stderr runs.
pub(super) fn start(log: &File) -> bool {
    let out = copy_fd(libc::STDOUT_FILENO, log);
    let err = copy_fd(libc::STDERR_FILENO, log);
    if out || err {
        static AT_EXIT: std::sync::Once = std::sync::Once::new();
        // SAFETY: atexit with a function of no arguments that never unwinds.
        AT_EXIT.call_once(|| unsafe {
            libc::atexit(stop_at_exit);
        });
    }
    err
}

extern "C" fn stop_at_exit() {
    // No unwind may leave an extern "C" function.
    let _ = std::panic::catch_unwind(stop);
}

fn copy_fd(fd: i32, log: &File) -> bool {
    let Ok(mut log) = log.try_clone() else {
        return false;
    };
    // SAFETY: fcntl and pipe2 make new descriptors of this process.
    let before = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if before < 0 {
        return false;
    }
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        unsafe { libc::close(before) };
        return false;
    }
    // SAFETY: the read end is a new descriptor that only this File owns.
    let read = unsafe { File::from_raw_fd(fds[0]) };
    // SAFETY: dup2 of the write end onto `fd`; then its own descriptor
    // closes, and `fd` is the one write end.
    unsafe {
        libc::dup2(fds[1], fd);
        libc::close(fds[1]);
    }
    let (done, end) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("detach-copy".into())
        .spawn(move || {
            // The descriptor of before is closed by [`stop`], not here.
            let mut to = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(before) });
            for line in BufReader::new(read).split(b'\n').map_while(Result::ok) {
                let _ = to.write_all(&line);
                let _ = to.write_all(b"\n");
                let text = String::from_utf8_lossy(&line);
                let clean = crate::report::mask(&crate::report::strip_ansi(&text));
                let _ = log.write_all(format!("{clean}\n").as_bytes());
            }
            let _ = done.send(());
        });
    if spawned.is_err() {
        // SAFETY: the descriptor of before comes back.
        unsafe {
            libc::dup2(before, fd);
            libc::close(before);
        }
        return false;
    }
    COPY.lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((fd, before, end));
    true
}

/// Put the descriptors of before back, and wait until the lines before are
/// in the log. A no-op when no copy runs.
pub(super) fn stop() {
    let copies = std::mem::take(&mut *COPY.lock().unwrap_or_else(|e| e.into_inner()));
    if copies.is_empty() {
        return;
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    for (fd, before, _) in &copies {
        // SAFETY: dup2 onto `fd` closes the write end of its pipe.
        unsafe { libc::dup2(*before, *fd) };
    }
    for (_, before, end) in copies {
        if end.recv_timeout(Duration::from_secs(1)).is_ok() {
            // SAFETY: the thread ended; nothing else uses `before`.
            unsafe { libc::close(before) };
        }
    }
}
