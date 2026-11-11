//! The table of the sockets of the program under the seccomp backend
//! (T-064), and the flow budget (T-181). pod-hold (PodUtils/podcrates) has
//! the table (the key is the inode of the socket of the program end; a
//! sweep removes the sockets that closed with no `close` call), the
//! derived ceiling and the classes of a refusal. This module keeps the
//! ceiling of this pod and the counts of its refusals.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::prelude::*;

pub(crate) use pod_hold::table::{
    derived_ceiling, inode_of, nofile, raise_nofile, Key, Refusal, Refusals, MAX_CEILING,
    MIN_CEILING,
};

/// The table of the sockets of the pod.
pub(crate) type Table = pod_hold::table::Table<Sock>;

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

/// The refusals of this pod.
pub(crate) static REFUSALS: Refusals = Refusals::new();

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
        if let Some(line) = REFUSALS.summary(ceiling()) {
            crate::warn(&format!("{line} (--flow-budget)"));
        }
    });
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
    Ok(pod_hold::table::handle_close(lfd, id, fd, key, pid, map)?)
}
