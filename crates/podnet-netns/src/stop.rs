//! The stop signals of the pod (T-517, GitHub #691): SIGTERM, SIGINT and
//! SIGHUP.
//!
//! A stop signal that a process sends (`kill`, a service manager) goes down
//! the chain of the pod: the front gives it to the back, the back to its
//! child (the reaper of the namespace, or the program of the seccomp
//! backend), and the reaper to the program. Each process of podnet in the
//! chain stays: the pod returns when the program exits, with the status of
//! the program (128+N when the signal ended it).
//!
//! A stop signal of the terminal (Ctrl-C, a hangup) has `si_code`
//! `SI_KERNEL`: the kernel gave it to the whole foreground process group,
//! and the program has it. It is not given again.
//!
//! When the program does not end in [`GRACE`], or at a second stop signal,
//! the front kills each process under it with SIGKILL and returns 128+N.

#[allow(unused_imports)]
use crate::prelude::*;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// The signals that stop the pod.
pub(crate) const SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// The time that the program has to end after a stop signal.
pub(crate) const GRACE: Duration = Duration::from_secs(10);

/// The process that gets each stop signal; 0 for none.
static CHILD: AtomicI32 = AtomicI32::new(0);
/// The process that installed the handler: a child of a fork has a copy of
/// the handler until its `execve`, and gives no signal.
static OWNER: AtomicI32 = AtomicI32::new(0);
/// The last stop signal that a process sent, and the count.
static LAST: AtomicI32 = AtomicI32::new(0);
static COUNT: AtomicU32 = AtomicU32::new(0);

/// Give each stop signal that a process sends to this process to `child`.
/// This process does not end at a stop signal. A `child` of 0 gives the
/// signal to no process. A signal that the caller ignores (SIGHUP under
/// `nohup`) stays ignored: the program ignores it too.
pub(crate) fn forward_to(child: libc::pid_t) {
    OWNER.store(unsafe { libc::getpid() }, Ordering::SeqCst);
    CHILD.store(child, Ordering::SeqCst);
    for signal in SIGNALS {
        // SAFETY: a handler that calls only async-signal-safe functions.
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal, std::ptr::null(), &mut old) == 0
                && old.sa_sigaction == libc::SIG_IGN
            {
                continue;
            }
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_stop as *const () as usize;
            action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

/// Give no signal to the child from now on: it has exited, and its id can
/// belong to another process.
pub(crate) fn forget_child() {
    CHILD.store(0, Ordering::SeqCst);
}

/// The last stop signal that a process sent, and the count of them.
pub(crate) fn received() -> Option<(libc::c_int, u32)> {
    match COUNT.load(Ordering::SeqCst) {
        0 => None,
        count => Some((LAST.load(Ordering::SeqCst), count)),
    }
}

/// True for an `si_code` of a signal that a process sent: `SI_USER`,
/// `SI_QUEUE`, `SI_TKILL` (each is 0 or less). The kernel (a terminal) gives
/// a positive code.
pub(crate) fn sent_by_a_process(si_code: libc::c_int) -> bool {
    si_code <= 0
}

extern "C" fn on_stop(signal: libc::c_int, info: *mut libc::siginfo_t, _: *mut libc::c_void) {
    // SAFETY: the kernel gives a valid siginfo with SA_SIGINFO.
    let code = unsafe { info.as_ref() }.map_or(0, |i| i.si_code);
    if !sent_by_a_process(code) {
        return;
    }
    LAST.store(signal, Ordering::SeqCst);
    COUNT.fetch_add(1, Ordering::SeqCst);
    let child = CHILD.load(Ordering::SeqCst);
    // SAFETY: getpid and kill are async-signal-safe.
    if child > 0 && unsafe { libc::getpid() } == OWNER.load(Ordering::SeqCst) {
        unsafe { libc::kill(child, signal) };
    }
}

/// The name of a stop signal.
pub(crate) fn name(signal: libc::c_int) -> &'static str {
    match signal {
        libc::SIGTERM => "SIGTERM",
        libc::SIGINT => "SIGINT",
        libc::SIGHUP => "SIGHUP",
        _ => "a stop signal",
    }
}

/// The processes under `root` that live (not zombies), from `/proc`.
pub(crate) fn descendants(root: libc::pid_t) -> Vec<libc::pid_t> {
    let mut parents: Vec<(libc::pid_t, libc::pid_t, bool)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // The name is in parentheses and can hold a space: read after it.
        let Some(rest) = stat.rfind(')').map(|at| &stat[at + 1..]) else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let state = fields.next().unwrap_or("");
        let Some(ppid) = fields.next().and_then(|p| p.parse().ok()) else {
            continue;
        };
        parents.push((pid, ppid, state != "Z" && state != "X"));
    }
    let mut found = vec![root];
    let mut at = 0;
    while at < found.len() {
        let parent = found[at];
        for (pid, ppid, _) in &parents {
            if *ppid == parent && !found.contains(pid) {
                found.push(*pid);
            }
        }
        at += 1;
    }
    found.retain(|pid| *pid != root && parents.iter().any(|(p, _, alive)| p == pid && *alive));
    found
}

/// Kill each process under this process with SIGKILL, and reap each child,
/// until none lives (at most 2 s). This process is the reaper of each
/// orphan (it is a subreaper), so no process of the pod gets out.
pub(crate) fn kill_all_under_self() {
    let me = unsafe { libc::getpid() };
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let alive = descendants(me);
        for pid in &alive {
            // SAFETY: a process under this one.
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
        reap_children();
        if alive.is_empty() || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Reap each child of this process that has ended, with no wait.
pub(crate) fn reap_children() {
    let mut status = 0;
    // SAFETY: waitpid of the children of this process.
    while unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) } > 0 {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signal that a process sends is given down the chain; a signal of
    /// the kernel (a terminal) is not.
    #[test]
    fn only_a_signal_of_a_process_goes_down_the_chain() {
        let cases = [
            (libc::SI_USER, true),
            (libc::SI_QUEUE, true),
            (libc::SI_TKILL, true),
            (0x80, false), // SI_KERNEL
            (1, false),
        ];
        for (code, want) in cases {
            assert_eq!(sent_by_a_process(code), want, "si_code {code}");
        }
    }

    #[test]
    fn the_descendants_of_a_process_include_its_grandchildren() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & sleep 30 & wait"])
            .spawn()
            .unwrap();
        let root = child.id() as libc::pid_t;
        let mut found = Vec::new();
        for _ in 0..100 {
            found = descendants(root);
            if found.len() >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        for pid in &found {
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(!found.contains(&root), "{found:?}");
    }
}
