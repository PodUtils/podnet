//! The calls on a TCP socket of the program that the pod answers as the
//! kernel does (T-223), before the usual handlers of the seccomp backend:
//!
//! - `shutdown` of a socket with no connection gives `ENOTCONN`. The
//!   socket of the program is one end of a connected `AF_UNIX` socket
//!   pair, where the kernel gives 0.
//! - `listen` with no `bind` binds the unspecified address and an
//!   ephemeral port first.
//! - `getsockopt(SO_ERROR)` during a nonblocking dial waits for the end of
//!   the dial (`src/connect.rs`).

use crate::connect::{Dials, ErrorCall};
use crate::prelude::*;
use crate::seccomp::{notif_respond, read_mem, Sock};
use crate::sockopt::{ephemeral_port, unspecified};

/// A call: its number, its arguments, its task, and the memory of the task
/// (the descriptor, and whether it is writable).
pub(crate) type Call = (i32, [u64; 6], u32, (RawFd, bool));

/// The longest `optlen` that a waiting `getsockopt` takes.
const MAX_OPTION: usize = 256;

/// Answer the call when it is one of the module. `None` for another call
/// or another socket: the usual handler takes it.
pub(crate) fn handle(
    lfd: RawFd,
    id: u64,
    (nr, a, pid, (mem, writable)): Call,
    (map, key): (&mut HashMap<Key, Sock>, Key),
    dials: &mut Dials,
) -> Option<Result<()>> {
    let Some(Sock::Stream {
        bound, v6, inbound, ..
    }) = map.get_mut(&key)
    else {
        return None;
    };
    let fd = a[0] as i32;
    let busy = inbound.is_some() || dials.in_progress(key);
    if nr == libc::SYS_shutdown as i32 {
        // A listener and a socket with a dial in progress go to the kernel.
        if busy {
            return None;
        }
        let errno = match a[1] as i32 {
            libc::SHUT_RD | libc::SHUT_WR | libc::SHUT_RDWR => libc::ENOTCONN,
            _ => libc::EINVAL,
        };
        return Some(notif_respond(lfd, id, -1, errno));
    }
    if nr == libc::SYS_listen as i32 {
        if bound.is_none() && !busy {
            // Then `handle_listen` makes the listener of this port.
            *bound = Some((unspecified(*v6), ephemeral_port(fd)));
        }
        return None;
    }
    let so_error = nr == libc::SYS_getsockopt as i32
        && a[1] as i32 == libc::SOL_SOCKET
        && a[2] as i32 == libc::SO_ERROR;
    if !so_error || !dials.nonblocking(key) || !writable || a[3] == 0 || a[4] == 0 {
        return None;
    }
    let len = read_mem(mem, a[4], 4).ok()?;
    let len = u32::from_ne_bytes(len.get(..4)?.try_into().ok()?) as usize;
    if len == 0 || len > MAX_OPTION {
        return None;
    }
    let call = ErrorCall {
        id,
        pid,
        fd,
        optval: a[3],
        optlen: a[4],
        len,
    };
    dials.wait_error(key, call);
    Some(Ok(()))
}
