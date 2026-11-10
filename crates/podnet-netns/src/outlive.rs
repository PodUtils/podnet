//! The pod lives past its first program (T-107, T-108).
//!
//! The caller of `podnet pod` waits for the exit code of the first program.
//! A program can start a daemon and exit (`rad node start`). The daemon must
//! keep the network of the pod. Thus the pod splits into two processes
//! before it starts a thread:
//!
//! * the front waits for the exit code of the first program, and returns it;
//! * the back runs the pod until no process of the pod lives.
//!
//! When the first program exits and no other process of the pod lives, the
//! back ends the pod as before, and then gives the code. The front then
//! returns after the relays finish.

#[allow(unused_imports)]
use crate::prelude::*;
use std::io::Read;
use std::sync::Mutex;

/// The back side of a split pod. It gives the exit code of the first program
/// to the front. A copy is the same reporter.
#[derive(Clone, Default)]
pub(crate) struct Reporter(Arc<Mutex<Option<OwnedFd>>>);

const TAG_CODE: u8 = b'C';
const TAG_ERROR: u8 = b'E';

impl Reporter {
    /// A reporter with no front: the pod runs in the calling process and
    /// ends with its first program.
    pub(crate) fn none() -> Self {
        Self::default()
    }

    /// True when a front waits: the pod can live past its first program.
    pub(crate) fn outlives(&self) -> bool {
        self.0.lock().map(|fd| fd.is_some()).unwrap_or(false)
    }

    /// Give `code` to the front, one time. The front then returns `code`.
    /// The back then lets go of the standard input and output of the
    /// caller, so a pipe of the caller can close.
    pub(crate) fn report(&self, code: i32, verbose: bool) {
        // T-158: the learned set is written, or its violations change the
        // code, at the exit of the first program.
        let code = crate::learn::finish(code);
        let Some(fd) = self.0.lock().ok().and_then(|mut fd| fd.take()) else {
            return;
        };
        let mut message = vec![TAG_CODE];
        message.extend_from_slice(&code.to_le_bytes());
        let _ = write_all_fd(fd.as_raw_fd(), &message);
        drop(fd);
        release_stdio(verbose);
    }

    /// Give an error to the front, when the front still waits.
    fn report_error(&self, error: &anyhow::Error) -> bool {
        let Some(fd) = self.0.lock().ok().and_then(|mut fd| fd.take()) else {
            return false;
        };
        let mut message = vec![TAG_ERROR];
        message.extend_from_slice(format!("{error:#}").as_bytes());
        write_all_fd(fd.as_raw_fd(), &message).is_ok()
    }

    /// Close the descriptor in a forked child that does not exec. Else the
    /// child holds the pipe open, and the front does not see the end of a
    /// back that fails.
    pub(crate) fn close_in_child(&self) {
        if let Ok(mut fd) = self.0.try_lock() {
            fd.take();
        }
    }
}

/// Put `/dev/null` on the standard input and output. Keep the standard error
/// for `-v` when it is a terminal or a file; a pipe goes to `/dev/null` too.
fn release_stdio(verbose: bool) {
    let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if null < 0 {
        return;
    }
    let mut keep_stderr = verbose;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(2, &mut stat) } == 0 {
        let kind = stat.st_mode & libc::S_IFMT;
        keep_stderr &= kind != libc::S_IFIFO && kind != libc::S_IFSOCK;
    }
    for fd in [0, 1] {
        unsafe { libc::dup2(null, fd) };
    }
    if !keep_stderr {
        unsafe { libc::dup2(null, 2) };
    }
    unsafe { libc::close(null) };
}

/// The number of threads of this process (`/proc/self/status`).
fn threads() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))?
        .trim()
        .parse()
        .ok()
}

/// The exit code of a `waitpid` status: the code, or 128 and the signal.
pub(crate) fn exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        128 + libc::WTERMSIG(status)
    }
}

/// Run the pod `body` in a back process, and return the exit code that the
/// back reports. A process with more than one thread (the pod of
/// `podnet warp run`) does not fork: `body` runs here with
/// [`Reporter::none`].
pub(crate) fn split(verbose: bool, body: impl FnOnce(Reporter) -> Result<i32>) -> Result<i32> {
    if threads() != Some(1) {
        return body(Reporter::none()).map(crate::learn::finish);
    }
    let mut fds = [0 as RawFd; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        bail!("pipe2: {}", std::io::Error::last_os_error());
    }
    // SAFETY: pipe2 gave two new descriptors that nothing else owns.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let back = unsafe { libc::fork() };
    if back < 0 {
        bail!("fork: {}", std::io::Error::last_os_error());
    }
    if back == 0 {
        drop(read);
        let reporter = Reporter(Arc::new(Mutex::new(Some(write))));
        let code = match body(reporter.clone()) {
            Ok(code) => {
                reporter.report(code, verbose);
                code
            }
            Err(e) => {
                if !reporter.report_error(&e) {
                    eprintln!("{}: {e:#}", crate::cli::program_name());
                }
                EXIT_NO_BACKEND
            }
        };
        let _ = std::io::stdout().flush();
        unsafe { libc::_exit(code) };
    }
    drop(write);
    front(read, back)
}

/// Wait for the report of the back. At a code, return it at once. At an
/// error or at the end of the back, reap the back.
fn front(read: OwnedFd, back: libc::pid_t) -> Result<i32> {
    let mut pipe = std::fs::File::from(read);
    let mut tag = [0u8; 1];
    let n = loop {
        match pipe.read(&mut tag) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            other => break other.unwrap_or(0),
        }
    };
    if n == 1 && tag[0] == TAG_CODE {
        let mut code = [0u8; 4];
        if pipe.read_exact(&mut code).is_ok() {
            return Ok(i32::from_le_bytes(code));
        }
    }
    let mut text = Vec::new();
    if n == 1 && tag[0] == TAG_ERROR {
        let _ = pipe.read_to_end(&mut text);
    }
    let mut status = 0;
    unsafe { libc::waitpid(back, &mut status, 0) };
    if !text.is_empty() {
        bail!("{}", String::from_utf8_lossy(&text));
    }
    Ok(exit_code(status))
}

/// True when this process has a child that is not reaped yet. Reaps
/// nothing.
pub(crate) fn has_children() -> bool {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    unsafe { libc::waitid(libc::P_ALL, 0, &mut info, flags) == 0 }
}

/// Make this process the reaper of its orphan descendants.
pub(crate) fn become_subreaper() -> Result<()> {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        bail!(
            "prctl(PR_SET_CHILD_SUBREAPER): {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}
