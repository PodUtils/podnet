//! The inbound sockets of the seccomp backend at the end of a pod
//! (COORD-142 of podrunner, T-619): the back removes the sockets that it
//! made, the front sweeps the directory after the back, and a start removes
//! the sockets that no socket of the host holds.

use std::path::PathBuf;

/// The inbound sockets that this process made (COORD-142 of podrunner): the
/// end of the supervisor removes them.
static INBOUND: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Note an inbound socket that this process bound, for [`remove_inbound`].
pub(crate) fn note_inbound(path: &std::path::Path) {
    if let Ok(mut made) = INBOUND.lock() {
        made.push(path.to_path_buf());
    }
}

/// Remove each inbound socket that this process made, then `dir` when it is
/// empty. A socket of another process in `dir` stays, and so does `dir`.
pub(crate) fn remove_inbound(dir: &std::path::Path) {
    let made = INBOUND
        .lock()
        .map(|mut made| std::mem::take(&mut *made))
        .unwrap_or_default();
    for path in made {
        let _ = std::fs::remove_file(path);
    }
    // remove_dir refuses a directory that is not empty.
    let _ = std::fs::remove_dir(dir);
}

/// The inbound directory of the pod, for the front: it outlives the back,
/// which a stop signal can end with SIGKILL before its own cleanup.
static END_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Name the inbound directory that [`sweep_end_dir`] cleans.
pub(crate) fn set_end_dir(dir: &std::path::Path) {
    let _ = END_DIR.set(dir.to_path_buf());
}

/// At the end of the pod: remove each socket of the inbound directory that
/// no process answers, then the directory when it is empty.
pub(crate) fn sweep_end_dir() {
    if let Some(dir) = END_DIR.get() {
        remove_stale_inbound(dir);
        let _ = std::fs::remove_dir(dir);
    }
}

/// Remove each socket `*.sock` of `dir` that no socket of the host holds:
/// the socket of a process that ended with no cleanup (a SIGKILL). The test
/// is the list of bound paths of `/proc/net/unix`, not a connect: a connect
/// to a live socket is a connection to the program of a pod. A socket that a
/// process holds, a file that is not a socket, and a name with another end
/// stay. With no readable list, nothing is removed. It gives the count of
/// the removed sockets.
pub(crate) fn remove_stale_inbound(dir: &std::path::Path) -> usize {
    let Ok(list) = std::fs::read_to_string("/proc/net/unix") else {
        return 0;
    };
    remove_unbound(dir, &bound_paths(&list))
}

/// The paths of the sockets of `/proc/net/unix` (the last field of a line
/// that has eight fields).
fn bound_paths(list: &str) -> std::collections::HashSet<PathBuf> {
    list.lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(7))
        .filter(|path| path.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

/// Remove each socket `*.sock` of `dir` whose path is not in `bound`.
fn remove_unbound(dir: &std::path::Path, bound: &std::collections::HashSet<PathBuf>) -> usize {
    use std::os::unix::fs::FileTypeExt as _;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_socket = entry.file_type().map(|t| t.is_socket()).unwrap_or(false);
        if !is_socket || path.extension().is_none_or(|e| e != "sock") || bound.contains(&path) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::private_dir;

    /// COORD-142 of podrunner: the end removes the sockets of this process
    /// and the empty directory; the next start removes a stale socket and
    /// keeps a live one and a file.
    #[test]
    fn the_inbound_sockets_go_at_the_end_and_a_stale_one_at_the_start() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("inbound");
        private_dir(&dir).unwrap();
        let mine = dir.join("8080.sock");
        let held = std::os::unix::net::UnixListener::bind(&mine).unwrap();
        note_inbound(&mine);
        drop(held);
        remove_inbound(&dir);
        assert!(
            !mine.exists() && !dir.exists(),
            "the socket and the directory stay"
        );

        private_dir(&dir).unwrap();
        let stale = dir.join("1.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        let live = dir.join("2.sock");
        let _live = std::os::unix::net::UnixListener::bind(&live).unwrap();
        let file = dir.join("3.sock");
        std::fs::write(&file, "").unwrap();
        // The list of the host: only the live socket is bound.
        let bound = bound_paths(&format!(
            "Num RefCount Protocol Flags Type St Inode Path\n\
             0000: 00000002 00000000 00010000 0001 01 1 {}\n\
             0000: 00000002 00000000 00000000 0001 03 2\n",
            live.display()
        ));
        assert_eq!(remove_unbound(&dir, &bound), 1);
        assert!(!stale.exists() && live.exists() && file.exists());
        // The check makes no connection to a live socket: it would be a
        // connection to the program of a pod.
        _live.set_nonblocking(true).unwrap();
        assert!(
            _live.accept().is_err(),
            "the check connected to a live socket"
        );
        // On this host, /proc/net/unix holds the live socket.
        if std::path::Path::new("/proc/net/unix").exists() {
            assert_eq!(remove_stale_inbound(&dir), 0);
            assert!(live.exists());
        }
        // A directory that is not empty stays.
        remove_inbound(&dir);
        assert!(dir.exists());
    }
}
