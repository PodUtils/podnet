//! Find the socket of a running `podnet warp up` (T-070).
//!
//! `warp exec`, `warp shell`, `warp nc` and the agent try these sockets, in
//! this order, and take the first that answers:
//!
//! 1. `--socket PATH` (when it is given, no other socket is tried);
//! 2. `PODNET_WARP_SOCKET`;
//! 3. `warp.sock` in the run directory of this process;
//! 4. the socket of the pointer file that `warp up` writes at a fixed place
//!    of the user: `$XDG_RUNTIME_DIR/podnet/warp-up.json`, else
//!    `podnet-<uid>/warp-up.json` in the temporary directory.
//!
//! The pointer file has mode 0600 in a directory of mode 0700, and podnet
//! reads it only when this user owns it. It names the socket and the state
//! directory of `warp up`, so that a `warp up` with another state directory
//! is found.

use std::path::{Path, PathBuf};

use anyhow::Result;

pub use pod_warp::find::Pointer;

/// The variable that names the socket of the running `podnet warp up`.
pub const SOCKET_VARIABLE: &str = "PODNET_WARP_SOCKET";

/// The option `--socket` of `warp exec`, `warp shell` and `warp nc`.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct At {
    /// The socket of the running podnet warp up. Without it: PODNET_WARP_SOCKET,
    /// warp.sock in the run directory, then the pointer file that podnet warp up
    /// writes ($XDG_RUNTIME_DIR/podnet/warp-up.json, else podnet-UID in the
    /// temporary directory)
    #[arg(long, value_name = "PATH")]
    pub socket: Option<PathBuf>,
}

/// The directory of the pointer file: `$XDG_RUNTIME_DIR/podnet`, else
/// `podnet-<uid>` in the temporary directory. It does not depend on the
/// state directory.
pub fn pointer_dir() -> PathBuf {
    pod_warp::find::pointer_dir_of(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::temp_dir(),
        "podnet",
    )
}

/// The path of the pointer file.
pub fn pointer_path() -> PathBuf {
    pointer_dir().join(pod_warp::find::POINTER_FILE)
}

/// Write the pointer file for `socket` (mode 0600, directory mode 0700).
pub fn write_pointer(socket: &Path) -> Result<()> {
    let dir = pointer_dir();
    crate::state::prepare(&dir)?;
    let pointer = Pointer {
        socket: socket.to_path_buf(),
        state_dir: crate::state::state_dir().ok().flatten(),
    };
    pod_warp::find::write_pointer(&dir.join(pod_warp::find::POINTER_FILE), &pointer)
        .map_err(super::failed)
}

/// Read the pointer file: none when it is missing, when another user owns
/// it, or when other users can read or write it.
pub fn read_pointer() -> Option<Pointer> {
    pod_warp::find::read_pointer_at(&pointer_path())
}

/// Remove the pointer file when it names `socket`.
pub fn remove_pointer(socket: &Path) {
    pod_warp::find::remove_pointer(&pointer_path(), socket);
}

/// The sockets to try, in order, each with where it came from.
pub fn candidates(at: &At) -> Vec<(&'static str, PathBuf)> {
    pod_warp::find::candidates_of(
        at.socket.clone().map(|s| ("--socket", s)),
        std::env::var_os(SOCKET_VARIABLE).map(|v| (SOCKET_VARIABLE, PathBuf::from(v))),
        crate::instances::run_dir().ok(),
        read_pointer(),
    )
}

/// The socket of the running `podnet warp up`: the first candidate that
/// answers. Else an error (exit 69) that names each socket tried, the state
/// directory of the pointer file, and the command that starts it.
pub fn running(at: &At) -> Result<PathBuf> {
    pod_warp::find::running(&candidates(at), read_pointer().as_ref()).map_err(|failure| {
        let text = format!("{}; start it with: podnet warp up", failure.text());
        super::failed(pod_core::Failure::new(failure.stage, failure.class, text))
    })
}

/// The lines of `warp up` on stderr when a listener did not start (issue
/// 115: the working listener comes first, then the one that failed).
pub fn up_summary(tcp: bool, port: u16, socket_up: bool, socket: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if !tcp && socket_up {
        out.push(format!(
            "podnet: warp up: the WARP proxy runs on unix:{}; use: podnet warp exec -- CMD (or podnet warp shell). The host refused the TCP listener 127.0.0.1:{port}, so there are no proxy variables",
            socket.display()
        ));
    } else if !tcp {
        out.push(format!(
            "podnet: warp up: the TCP listener 127.0.0.1:{port} did not start; there are no proxy variables"
        ));
    }
    if !socket_up {
        out.push("podnet: warp up: the unix socket did not start; podnet warp exec and podnet warp shell do not work. Use the proxy variables: eval \"$(podnet warp env)\"".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue 115: with the TCP listener refused, the first line names the
    // unix socket that works.
    #[test]
    fn the_warp_up_summary_starts_with_what_works() {
        let socket = std::path::Path::new("/run/podnet/warp.sock");
        let lines = up_summary(false, 40000, true, socket);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("runs on unix:/run/podnet/warp.sock"),
            "{lines:?}"
        );
        assert!(lines[0].contains("podnet warp exec"), "{lines:?}");
        assert!(up_summary(true, 40000, true, socket).is_empty());
        assert_eq!(up_summary(true, 40000, false, socket).len(), 1);
    }
}
