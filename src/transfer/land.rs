//! The rules of the names that `podnet receive` writes (T-105, GitHub
//! #436), and the write itself.
//!
//! 1. Each name goes through `pod_extract::safety::components`. A refusal
//!    ends the whole transfer before a byte is written: `refused entry
//!    "NAME": REASON`. A bad name is never rewritten.
//! 2. Also refused: a backslash in a component, a component over 255
//!    bytes, a path over 4096 bytes, and two entries that are equal after
//!    ASCII case folding.
//! 3. Only regular files and directories: a link, a device, a fifo or a
//!    socket is refused.
//! 4. A file is made with `openat` under the output directory, each
//!    directory of its path opened with `O_NOFOLLOW`, and the file with
//!    `O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW` and the mode `0666 & ~umask`:
//!    never an executable or a set-id bit. A name that exists is an error
//!    that names it: the receiver never overwrites.
//! 5. The top name of the transfer follows the same rules, and must not
//!    exist.
//! 6. The caps ([`Caps`]) are checked from the zip directory before the
//!    write, and again while it writes, as a header can lie.
//! 7. The checks of all entries come before the first write. When a write
//!    fails, each file and directory that this run made is removed, and no
//!    other.

use std::collections::HashSet;
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use super::zip::Entry;

/// The most entries of a directory, by default.
pub const MAX_FILES: u64 = 100_000;
/// The most bytes of a transfer, by default: 64 GiB.
pub const MAX_BYTES: u64 = 64 << 30;
/// The longest component and the longest path.
pub const COMPONENT_MOST: usize = 255;
pub const PATH_MOST: usize = 4096;

/// The caps of a receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub max_files: u64,
    pub max_bytes: u64,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps {
            max_files: MAX_FILES,
            max_bytes: MAX_BYTES,
        }
    }
}

/// Input that the receiver refuses (exit 65).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

fn refuse(name: &str, why: impl fmt::Display) -> Refused {
    Refused(format!("refused entry {name:?}: {why}"))
}

/// The components of `name`, by the rules 1 and 2.
pub fn components(name: &str) -> Result<Vec<String>, Refused> {
    if name.len() > PATH_MOST {
        return Err(refuse(name, format!("a path over {PATH_MOST} bytes")));
    }
    let parts = pod_extract::safety::components(name).map_err(|why| refuse(name, why.why()))?;
    for part in &parts {
        if part.contains('\\') {
            return Err(refuse(name, "a backslash in a component"));
        }
        if part.len() > COMPONENT_MOST {
            return Err(refuse(
                name,
                format!("a component over {COMPONENT_MOST} bytes"),
            ));
        }
    }
    Ok(parts.into_iter().map(str::to_string).collect())
}

/// The top name of a transfer: one component.
pub fn top(name: &str) -> Result<String, Refused> {
    let parts = components(name)?;
    match parts.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err(refuse(name, "the name of a transfer is one component")),
    }
}

/// The entries of a zip, checked by the rules 1 to 3 and 6 before any
/// write: the components of each one.
pub fn plan(entries: &[Entry], caps: &Caps) -> Result<Vec<Vec<String>>, Refused> {
    if entries.len() as u64 > caps.max_files {
        return Err(Refused(format!(
            "{} entries: the cap is {} (--max-files)",
            entries.len(),
            caps.max_files
        )));
    }
    let total: u64 = entries.iter().map(|e| e.size).fold(0, u64::saturating_add);
    if total > caps.max_bytes {
        return Err(Refused(format!(
            "{total} bytes: the cap is {} (--max-bytes)",
            caps.max_bytes
        )));
    }
    let mut folded = HashSet::new();
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(kind) = entry.odd_kind() {
            return Err(refuse(
                &entry.name,
                format!("{kind}: podnet takes regular files and directories only"),
            ));
        }
        let parts = components(&entry.name)?;
        if !folded.insert(parts.join("/").to_ascii_lowercase()) {
            return Err(refuse(
                &entry.name,
                "two entries with this name (ASCII case folded)",
            ));
        }
        out.push(parts);
    }
    Ok(out)
}

/// The writes of one receive under one output directory, with what they
/// made, so that a failure removes it.
pub struct Landing {
    out: PathBuf,
    dir: OwnedFd,
    /// Each file and directory made, relative to the output directory, in
    /// the order made.
    made: Vec<(String, bool)>,
    dirs: HashSet<String>,
}

impl Landing {
    /// The writes under the directory `out`, which must exist.
    pub fn open(out: &Path) -> io::Result<Landing> {
        let path = cstring(out.as_os_str().as_encoded_bytes())?;
        // SAFETY: a call with a valid C string; the descriptor is owned.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let err = io::Error::last_os_error();
            return Err(io::Error::new(
                err.kind(),
                format!("{}: {err}", out.display()),
            ));
        }
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        let dir = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Landing {
            out: out.to_path_buf(),
            dir,
            made: Vec::new(),
            dirs: HashSet::new(),
        })
    }

    /// Make the directory `parts` and each one above it that this run did
    /// not make yet. One that exists and that this run did not make is an
    /// error.
    pub fn dir(&mut self, parts: &[String]) -> io::Result<()> {
        self.parent(parts, parts.len()).map(drop)
    }

    /// Make the file `parts` (its directories too), new: one that exists
    /// is an error that names it.
    pub fn file(&mut self, parts: &[String]) -> io::Result<File> {
        let (last, above) = parts
            .split_last()
            .ok_or_else(|| io::Error::other("an empty name"))?;
        let parent = self.parent(parts, above.len())?;
        let name = cstring(last.as_bytes())?;
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let base = parent.as_ref().map_or(self.dir.as_fd(), |fd| fd.as_fd());
        // SAFETY: a valid directory descriptor and C string; the mode is
        // 0666, which the umask narrows.
        let fd = unsafe {
            libc::openat(
                base.as_raw_fd(),
                name.as_ptr(),
                flags,
                0o666 as libc::c_uint,
            )
        };
        let joined = parts.join("/");
        if fd < 0 {
            return Err(named(&joined, io::Error::last_os_error()));
        }
        self.made.push((joined, false));
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    /// The directory of the first `depth` components of `parts`, opened
    /// with `O_NOFOLLOW` from the output directory, each one made when this
    /// run did not make it: `None` for the output directory itself.
    fn parent(&mut self, parts: &[String], depth: usize) -> io::Result<Option<OwnedFd>> {
        let mut at: Option<OwnedFd> = None;
        for i in 0..depth {
            let joined = parts[..=i].join("/");
            let name = cstring(parts[i].as_bytes())?;
            let base = at.as_ref().map_or(self.dir.as_fd(), |fd| fd.as_fd());
            if !self.dirs.contains(&joined) {
                mkdir_at(base, &name).map_err(|e| named(&joined, e))?;
                self.made.push((joined.clone(), true));
                self.dirs.insert(joined.clone());
            }
            at = Some(open_dir_at(base, &name).map_err(|e| named(&joined, e))?);
        }
        Ok(at)
    }

    /// Remove each file and directory that this run made, the last first.
    pub fn undo(&mut self) {
        for (name, is_dir) in self.made.drain(..).rev() {
            let path = self.out.join(&name);
            let _ = if is_dir {
                std::fs::remove_dir(&path)
            } else {
                std::fs::remove_file(&path)
            };
        }
        self.dirs.clear();
    }

    /// Keep what this run made.
    pub fn keep(mut self) {
        self.made.clear();
    }
}

/// Write each entry of the zip in `zip` under the new directory `top` of
/// `out`, by the rules above: the whole directory of the zip is checked
/// first, and a failure removes what this call made. The count of entries.
pub fn unzip(zip: &mut File, out: &Path, top: &str, caps: &Caps) -> anyhow::Result<u64> {
    use super::refused;
    let top = vec![self::top(top).map_err(refused)?];
    let entries = super::zip::entries(zip).map_err(refused)?;
    let plan = plan(&entries, caps).map_err(refused)?;
    let mut landing = Landing::open(out).map_err(refused)?;
    landing.dir(&top).map_err(refused)?;
    let mut written = 0u64;
    for (entry, parts) in entries.iter().zip(&plan) {
        let path: Vec<String> = top.iter().chain(parts).cloned().collect();
        if entry.is_dir {
            landing.dir(&path).map_err(refused)?;
            continue;
        }
        let mut file = landing.file(&path).map_err(refused)?;
        let mut capped = Capped {
            out: &mut file,
            left: caps.max_bytes.saturating_sub(written),
        };
        written += super::zip::copy(zip, entry, &mut capped).map_err(refused)?;
    }
    landing.keep();
    Ok(entries.len() as u64)
}

/// A writer that refuses more than `left` bytes: the cap while a zip is
/// written, as its headers can lie.
struct Capped<'a> {
    out: &'a mut File,
    left: u64,
}

impl Write for Capped<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.left {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "more bytes than the cap (--max-bytes)",
            ));
        }
        let n = self.out.write(buf)?;
        self.left -= n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

fn mkdir_at(base: BorrowedFd<'_>, name: &CString) -> io::Result<()> {
    // SAFETY: a valid directory descriptor and C string.
    if unsafe { libc::mkdirat(base.as_raw_fd(), name.as_ptr(), 0o777) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn open_dir_at(base: BorrowedFd<'_>, name: &CString) -> io::Result<OwnedFd> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: a valid directory descriptor and C string.
    let fd = unsafe { libc::openat(base.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn cstring(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in a name"))
}

/// `err` with the name; an existing name says that the receiver never
/// overwrites.
fn named(name: &str, err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::AlreadyExists {
        return io::Error::new(
            err.kind(),
            format!("{name:?} exists (EEXIST): the receiver never overwrites"),
        );
    }
    io::Error::new(err.kind(), format!("{name:?}: {err}"))
}

impl Drop for Landing {
    fn drop(&mut self) {
        self.undo();
    }
}
