//! The small values of the control socket: the clock, the token path and
//! the constant-time compare.

use std::path::{Path, PathBuf};

/// The seconds of the clock.
pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The path of the token file of a socket.
pub fn token_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_os_string();
    name.push(".token");
    PathBuf::from(name)
}

/// True when two byte strings are equal, in constant time.
pub(super) fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
