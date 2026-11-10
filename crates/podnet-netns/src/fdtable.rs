//! The table of the sockets of the program under the seccomp backend
//! (T-064).
//!
//! The key of an entry is the inode number of the socket of the program
//! end (the link `/proc/PID/fd/N` is `socket:[INODE]`). A `fork`, a `dup`,
//! a `dup2`, a `dup3`, an `fcntl(F_DUPFD)` and a descriptor sent with
//! `SCM_RIGHTS` give the same socket, thus the same entry, with no call to
//! stop. Each process of the pod and each thread finds the entry of its
//! descriptor with one `readlink`.
//!
//! A `close` removes the entry only when no other descriptor of the pod
//! refers to the socket. Some descriptors close with no `close` call (the
//! exit of a process, `execve` with close-on-exec, `close_range`). A sweep
//! removes the entries of these sockets: it runs when the table doubles,
//! and before the table refuses a socket at its ceiling ([`ceiling`]).
//!
//! The budget (T-181): the ceiling is `--flow-budget N`, else it comes from
//! `RLIMIT_NOFILE` ([`derived_ceiling`]). Each refused `socket` call is
//! counted by its class ([`Refusal`]).

use std::collections::HashSet;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::prelude::*;

/// The key of an entry: the inode number of the socket of the program end.
/// 0 is not a key: the sockets file system gives no inode 0.
pub(crate) type Key = u64;

/// The table size of the first sweep.
const FIRST_SWEEP: usize = 64;

/// The supervisor descriptors of one tracked socket: its end of the pair
/// and the upstream (a datagram socket: its end, a copy and the relay).
const FDS_PER_SOCKET: u64 = 2;
/// The descriptors of the supervisor that are not tracked sockets.
const RESERVE: u64 = 64;
/// The smallest and the largest ceiling.
pub(crate) const MIN_CEILING: usize = 64;
pub(crate) const MAX_CEILING: usize = 1 << 20;
/// The ceiling of the derived budget.
const MAX_DERIVED: usize = 65_536;

/// The ceiling of the table of this pod.
static CEILING: AtomicUsize = AtomicUsize::new(MAX_TRACKED_FDS);

/// The ceiling of the table: the number of sockets of the program that the
/// supervisor tracks at one time.
pub(crate) fn ceiling() -> usize {
    CEILING.load(Ordering::Relaxed)
}

/// Set the ceiling (`--flow-budget`, or [`derived_ceiling`]).
pub(crate) fn set_ceiling(n: usize) {
    CEILING.store(n.clamp(MIN_CEILING, MAX_CEILING), Ordering::Relaxed);
}

/// The soft and the hard `RLIMIT_NOFILE` of this process.
// rlim_t is u32 on some 32-bit targets.
#[allow(clippy::unnecessary_cast)]
pub(crate) fn nofile() -> (u64, u64) {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes one rlimit.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return (1024, 1024);
    }
    (limit.rlim_cur as u64, limit.rlim_max as u64)
}

/// Raise the soft `RLIMIT_NOFILE` of the supervisor to the hard limit.
/// Call it after the fork of the program: the program keeps its limit.
pub(crate) fn raise_nofile() {
    let (soft, hard) = nofile();
    if soft < hard {
        let limit = libc::rlimit {
            rlim_cur: hard as libc::rlim_t,
            rlim_max: hard as libc::rlim_t,
        };
        // SAFETY: setrlimit reads one rlimit.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    }
}

/// The ceiling for a descriptor limit `limit` of the supervisor: the
/// descriptors above a reserve, two for each socket, from 64 to 65536.
pub(crate) fn derived_ceiling(limit: u64) -> usize {
    let n = limit.saturating_sub(RESERVE) / FDS_PER_SOCKET;
    (n.min(MAX_DERIVED as u64) as usize).max(MIN_CEILING)
}

/// The class of a refused `socket` call (T-181).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The table is at its ceiling (`EMFILE`).
    TableFull,
    /// The supervisor could not make the socket pair (its own descriptor
    /// limit; `EMFILE`).
    SocketPair,
    /// A handler failed before it answered (`EIO`, or `ECONNREFUSED` for
    /// `connect`).
    Other,
}

impl Refusal {
    const ALL: [Refusal; 3] = [Refusal::TableFull, Refusal::SocketPair, Refusal::Other];

    /// The name of the class in the log and in the record.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Refusal::TableFull => "table full",
            Refusal::SocketPair => "socketpair failed",
            Refusal::Other => "other",
        }
    }
}

/// The counters of the refusals, one for each class.
#[derive(Default)]
pub(crate) struct Refusals([AtomicU64; 3]);

impl Refusals {
    /// Count one refusal of `class`. Return the new count of the class.
    pub(crate) fn count(&self, class: Refusal) -> u64 {
        self.0[class as usize].fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The count of each class, in the order of [`Refusal::ALL`].
    pub(crate) fn counts(&self) -> [(Refusal, u64); 3] {
        Refusal::ALL.map(|class| (class, self.0[class as usize].load(Ordering::Relaxed)))
    }

    /// One line with each count, or `None` when no call was refused.
    pub(crate) fn summary(&self) -> Option<String> {
        let counts = self.counts();
        let total: u64 = counts.iter().map(|(_, n)| n).sum();
        if total == 0 {
            return None;
        }
        let parts: Vec<String> = counts
            .iter()
            .map(|(class, n)| format!("{} {n}", class.name()))
            .collect();
        Some(format!(
            "{total} socket calls refused ({}); the flow budget is {} (--flow-budget)",
            parts.join(", "),
            ceiling()
        ))
    }
}

/// The refusals of this pod.
pub(crate) static REFUSALS: Refusals = Refusals([const { AtomicU64::new(0) }; 3]);

/// Count a refusal of `class`; with `verbose`, print one line.
pub(crate) fn refuse(class: Refusal, what: &str, verbose: bool) {
    let n = REFUSALS.count(class);
    if verbose {
        eprintln!(
            "{}: {what} refused: {} ({n} of this class; the flow budget is {})",
            crate::cli::program_name(),
            class.name(),
            ceiling()
        );
    }
}

/// Print the summary of the refusals one time, unless no call was refused.
pub(crate) fn report_refusals() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        if let Some(line) = REFUSALS.summary() {
            crate::warn(&line);
        }
    });
}

pub(crate) struct Table {
    socks: HashMap<Key, Sock>,
    /// The first process of the pod.
    root: i32,
    /// Each task that made a stopped call, with its thread group id.
    tasks: HashMap<u32, i32>,
    /// The table size at which the next sweep runs.
    sweep_at: usize,
}

impl std::ops::Deref for Table {
    type Target = HashMap<Key, Sock>;
    fn deref(&self) -> &Self::Target {
        &self.socks
    }
}

impl std::ops::DerefMut for Table {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.socks
    }
}

impl Table {
    pub(crate) fn new(root: i32) -> Self {
        Table {
            socks: HashMap::new(),
            root,
            tasks: HashMap::new(),
            sweep_at: FIRST_SWEEP,
        }
    }

    /// The key of the descriptor `fd` of the task `pid` (0 when it is not a
    /// socket). Record the process of the task for the sweep.
    pub(crate) fn key_of(&mut self, pid: u32, fd: i32) -> Key {
        if !self.tasks.contains_key(&pid) {
            if self.tasks.len() >= 4 * ceiling() {
                self.tasks.retain(|tid, _| alive(*tid as i32));
            }
            self.tasks.insert(pid, tgid_of(pid));
        }
        socket_inode(pid as i32, fd)
    }

    /// Whether the table has room for one more entry. Sweep first when
    /// the sweep is due.
    pub(crate) fn has_room(&mut self) -> bool {
        if self.socks.len() >= self.sweep_at {
            self.sweep();
            self.sweep_at = (self.socks.len() * 2).clamp(FIRST_SWEEP, ceiling());
        }
        self.socks.len() < ceiling()
    }

    /// Remove each entry whose socket no process of the pod holds.
    fn sweep(&mut self) {
        self.tasks.retain(|tid, _| alive(*tid as i32));
        let mut live = HashSet::new();
        let mut read = 0;
        for process in self.processes() {
            if let Some(sockets) = sockets_of(process) {
                read += 1;
                live.extend(sockets.into_iter().map(|(_, key)| key));
            }
        }
        // No table could be read: keep each entry.
        if read > 0 {
            self.socks.retain(|key, _| live.contains(key));
        }
    }

    /// The processes of the pod: the root and its descendants, and each
    /// process that made a stopped call (a process whose parent exited).
    fn processes(&self) -> HashSet<i32> {
        let mut found = HashSet::new();
        let mut todo: Vec<i32> = vec![self.root];
        todo.extend(self.tasks.values());
        while let Some(process) = todo.pop() {
            if process <= 0 || !found.insert(process) {
                continue;
            }
            let Ok(threads) = std::fs::read_dir(format!("/proc/{process}/task")) else {
                continue;
            };
            for thread in threads.flatten() {
                let path = thread.path().join("children");
                if let Ok(text) = std::fs::read_to_string(path) {
                    todo.extend(
                        text.split_whitespace()
                            .filter_map(|p| p.parse::<i32>().ok()),
                    );
                }
            }
        }
        found
    }

    /// Whether a descriptor of the pod other than `fd` of the task `pid`
    /// refers to the socket `key`.
    pub(crate) fn other_copy(&self, pid: u32, fd: i32, key: Key) -> bool {
        let own = sockets_of(pid as i32).unwrap_or_default();
        if own.iter().any(|&(n, k)| k == key && n != fd) {
            return true;
        }
        let caller = self.tasks.get(&pid).copied().unwrap_or(pid as i32);
        self.processes()
            .into_iter()
            .filter(|&process| process != caller)
            .any(|process| sockets_of(process).is_some_and(|s| s.iter().any(|&(_, k)| k == key)))
    }
}

/// `close` of the descriptor `fd` (the socket `key`) by the task `pid`:
/// drop the entry when this is the last descriptor of the socket in the
/// pod. The kernel then does the close.
pub(crate) fn handle_close(
    lfd: RawFd,
    id: u64,
    fd: i32,
    key: Key,
    pid: u32,
    map: &mut Table,
) -> Result<()> {
    if key != 0 && map.contains_key(&key) && !map.other_copy(pid, fd, key) {
        map.remove(&key);
    }
    notif_continue(lfd, id)
}

/// The inode number of the descriptor `fd` of the supervisor.
pub(crate) fn inode_of(fd: RawFd) -> Result<Key> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `st` is a valid buffer for one `struct stat`.
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return Err(std::io::Error::last_os_error()).context("fstat");
    }
    Ok(st.st_ino as Key)
}

/// The inode of the socket at the descriptor `fd` of the task `pid`; 0 when
/// the descriptor is not a socket or cannot be read.
fn socket_inode(pid: i32, fd: i32) -> Key {
    match std::fs::read_link(format!("/proc/{pid}/fd/{fd}")) {
        Ok(link) => parse_socket_link(link.as_os_str().as_bytes()),
        Err(_) => 0,
    }
}

/// The inode of a link `socket:[INODE]`; 0 for another link.
fn parse_socket_link(link: &[u8]) -> Key {
    link.strip_prefix(b"socket:[")
        .and_then(|rest| rest.strip_suffix(b"]"))
        .and_then(|digits| std::str::from_utf8(digits).ok()?.parse().ok())
        .unwrap_or(0)
}

/// Each socket of the process `pid`: (descriptor, inode). `None` when its
/// table of descriptors cannot be read.
fn sockets_of(pid: i32) -> Option<Vec<(i32, Key)>> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Some(fd) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        if let Ok(link) = std::fs::read_link(entry.path()) {
            let key = parse_socket_link(link.as_os_str().as_bytes());
            if key != 0 {
                out.push((fd, key));
            }
        }
    }
    Some(out)
}

/// The thread group id of the task `tid` (`Tgid:` of its status).
fn tgid_of(tid: u32) -> i32 {
    std::fs::read_to_string(format!("/proc/{tid}/status"))
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("Tgid:"))
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or(tid as i32)
}

fn alive(pid: i32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-181: each class has its own counter, and the summary names each.
    #[test]
    fn refusals_are_counted_by_class() {
        let refusals = Refusals::default();
        assert!(refusals.summary().is_none());
        assert_eq!(refusals.count(Refusal::TableFull), 1);
        assert_eq!(refusals.count(Refusal::TableFull), 2);
        assert_eq!(refusals.count(Refusal::SocketPair), 1);
        let counts = refusals.counts();
        assert_eq!(counts[0], (Refusal::TableFull, 2));
        assert_eq!(counts[1], (Refusal::SocketPair, 1));
        assert_eq!(counts[2], (Refusal::Other, 0));
        let line = refusals.summary().unwrap();
        assert!(
            line.starts_with("3 socket calls refused (table full 2, socketpair failed 1, other 0)"),
            "{line}"
        );
    }

    // T-181: the ceiling follows the descriptor limit, within its bounds.
    #[test]
    fn the_ceiling_follows_rlimit_nofile() {
        assert_eq!(derived_ceiling(1024), 480);
        assert_eq!(derived_ceiling(20_000), 9968);
        assert_eq!(derived_ceiling(100), MIN_CEILING);
        assert_eq!(derived_ceiling(u64::MAX), 65_536);
    }

    #[test]
    fn a_socket_link_gives_its_inode() {
        assert_eq!(parse_socket_link(b"socket:[123456]"), 123456);
        assert_eq!(parse_socket_link(b"pipe:[77]"), 0);
        assert_eq!(parse_socket_link(b"/dev/null"), 0);
        assert_eq!(parse_socket_link(b"socket:[x]"), 0);
    }

    #[test]
    fn a_copy_of_a_socket_has_the_inode_of_the_socket() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let copy = a.try_clone().unwrap();
        let me = std::process::id();
        let key = Table::new(me as i32).key_of(me, a.as_raw_fd());
        assert_ne!(key, 0);
        assert_eq!(key, inode_of(a.as_raw_fd()).unwrap());
        assert_eq!(socket_inode(me as i32, copy.as_raw_fd()), key);
    }
}
