//! A ustar (POSIX.1-1988) writer and reader: regular files and directories
//! only (T-085).
//!
//! * A name longer than 100 bytes goes in the ustar prefix (155 bytes),
//!   split at a `/`. A name that does not fit is refused.
//! * A size of 8 GiB or more is written in base-256 (the GNU form, which
//!   GNU tar and bsdtar read).
//! * The reader refuses links, devices, an absolute name and a `..`
//!   segment. [`unpack`] writes no file out of its root.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context as _, Result};

/// The size of a block of the archive.
pub const BLOCK: usize = 512;

/// The kind of an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

/// The header of one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name in the archive, with `/` between the segments, and no `/`
    /// at the end.
    pub path: String,
    pub kind: Kind,
    /// The permission bits (`0o7777` at most).
    pub mode: u32,
    /// The time of the last change, in seconds since the epoch.
    pub mtime: u64,
    /// The size of the data (0 for a directory).
    pub size: u64,
}

/// Check one name of an entry: not empty, relative, no `.` or `..`
/// segment, no NUL.
fn check_name(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.contains('\0') {
        bail!("the tar name {path:?} is not a relative path");
    }
    if path
        .split('/')
        .any(|s| s.is_empty() || s == "." || s == "..")
    {
        bail!("the tar name {path:?} has an empty, . or .. segment");
    }
    Ok(())
}

/// Split `name` into the ustar prefix and name fields.
fn split_name(name: &str) -> Result<(&str, &str)> {
    if name.len() <= 100 {
        return Ok(("", name));
    }
    // The last `/` that leaves a name of 100 bytes or less and a prefix of
    // 155 bytes or less.
    for (at, _) in name.match_indices('/') {
        let (prefix, rest) = (&name[..at], &name[at + 1..]);
        if prefix.len() <= 155 && !rest.is_empty() && rest.len() <= 100 {
            return Ok((prefix, rest));
        }
    }
    bail!(
        "the tar name {name:?} ({} bytes) does not fit in ustar",
        name.len()
    )
}

fn put_octal(field: &mut [u8], value: u64) -> Result<()> {
    let digits = field.len() - 1;
    let text = format!("{value:0digits$o}");
    if text.len() > digits {
        bail!("the value {value} does not fit in {digits} octal digits");
    }
    field[..digits].copy_from_slice(text.as_bytes());
    field[digits] = 0;
    Ok(())
}

/// Make the header block of `entry`.
pub fn header(entry: &Entry) -> Result<[u8; BLOCK]> {
    check_name(&entry.path)?;
    let mut name = entry.path.clone();
    if entry.kind == Kind::Dir {
        name.push('/');
    }
    let (prefix, short) = split_name(&name)?;
    let mut h = [0u8; BLOCK];
    h[..short.len()].copy_from_slice(short.as_bytes());
    put_octal(&mut h[100..108], u64::from(entry.mode & 0o7777))?;
    put_octal(&mut h[108..116], 0)?;
    put_octal(&mut h[116..124], 0)?;
    let size = if entry.kind == Kind::Dir {
        0
    } else {
        entry.size
    };
    if size < 0o77777777777 {
        put_octal(&mut h[124..136], size)?;
    } else {
        // Base-256: the high bit of the first byte, then big-endian.
        h[124] = 0x80;
        h[128..136].copy_from_slice(&size.to_be_bytes());
    }
    put_octal(&mut h[136..148], entry.mtime.min(0o77777777777))?;
    h[156] = if entry.kind == Kind::Dir { b'5' } else { b'0' };
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    let text = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(text.as_bytes());
    Ok(h)
}

/// A writer of a ustar archive to `W`.
pub struct Writer<W: Write> {
    out: W,
}

impl<W: Write> Writer<W> {
    pub fn new(out: W) -> Self {
        Writer { out }
    }

    /// Add a directory.
    pub fn dir(&mut self, path: &str, mode: u32, mtime: u64) -> Result<()> {
        let entry = Entry {
            path: path.to_string(),
            kind: Kind::Dir,
            mode,
            mtime,
            size: 0,
        };
        self.out.write_all(&header(&entry)?)?;
        Ok(())
    }

    /// Add a file of `size` bytes from `data`. A short `data` is padded
    /// with zeros and a long one is cut: the header stays true.
    pub fn file(
        &mut self,
        path: &str,
        mode: u32,
        mtime: u64,
        size: u64,
        data: &mut dyn Read,
    ) -> Result<()> {
        let entry = Entry {
            path: path.to_string(),
            kind: Kind::File,
            mode,
            mtime,
            size,
        };
        self.out.write_all(&header(&entry)?)?;
        let copied = std::io::copy(&mut data.take(size), &mut self.out)?;
        let mut left = size - copied;
        let zeros = [0u8; 8192];
        while left > 0 {
            let n = left.min(zeros.len() as u64) as usize;
            self.out.write_all(&zeros[..n])?;
            left -= n as u64;
        }
        let pad = (BLOCK - (size % BLOCK as u64) as usize) % BLOCK;
        self.out.write_all(&zeros[..pad])?;
        Ok(())
    }

    /// Write the end (two zero blocks), and give back the output.
    pub fn finish(mut self) -> Result<W> {
        self.out.write_all(&[0u8; 2 * BLOCK])?;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn field_str(field: &[u8]) -> Result<&str> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).context("a tar name that is not UTF-8")
}

fn get_number(field: &[u8]) -> Result<u64> {
    if field[0] & 0x80 != 0 {
        let mut value: u64 = 0;
        for (i, &b) in field.iter().enumerate() {
            let b = if i == 0 { b & 0x7f } else { b };
            if value >> 56 != 0 {
                bail!("a base-256 tar number that does not fit in 64 bits");
            }
            value = (value << 8) | u64::from(b);
        }
        return Ok(value);
    }
    let text = field_str(field)?.trim_matches(|c| c == ' ' || c == '\0');
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).with_context(|| format!("the tar number {text:?}"))
}

/// Parse one header block. `None` for a zero block (the end).
pub fn parse_header(h: &[u8; BLOCK]) -> Result<Option<Entry>> {
    if h.iter().all(|&b| b == 0) {
        return Ok(None);
    }
    let stored = get_number(&h[148..156])?;
    let sum: u64 = h
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u64::from(b)
            }
        })
        .sum();
    if stored != sum {
        bail!("a tar header with a bad checksum ({stored:o}, computed {sum:o})");
    }
    if &h[257..262] != b"ustar" {
        bail!("a tar header that is not ustar");
    }
    let short = field_str(&h[..100])?;
    let prefix = field_str(&h[345..500])?;
    let mut path = if prefix.is_empty() {
        short.to_string()
    } else {
        format!("{prefix}/{short}")
    };
    let kind = match h[156] {
        b'0' | 0 => Kind::File,
        b'5' => Kind::Dir,
        other => bail!(
            "the tar entry {path:?} has the type {:?}: only files and directories are read",
            other as char
        ),
    };
    if kind == Kind::Dir || path.ends_with('/') {
        while path.ends_with('/') {
            path.pop();
        }
    }
    check_name(&path)?;
    Ok(Some(Entry {
        path,
        kind,
        mode: (get_number(&h[100..108])? & 0o7777) as u32,
        mtime: get_number(&h[136..148])?,
        size: if kind == Kind::Dir {
            0
        } else {
            get_number(&h[124..136])?
        },
    }))
}

/// A reader of a ustar archive.
pub struct Reader<R: Read> {
    input: R,
    /// The bytes of the data of the last entry that are not read, and its
    /// padding.
    left: u64,
    pad: u64,
}

impl<R: Read> Reader<R> {
    pub fn new(input: R) -> Self {
        Reader {
            input,
            left: 0,
            pad: 0,
        }
    }

    /// The next entry, or `None` at the end. The data of the entry before
    /// it that is not read is skipped.
    pub fn next_entry(&mut self) -> Result<Option<Entry>> {
        let skip = self.left + self.pad;
        let skipped = std::io::copy(&mut (&mut self.input).take(skip), &mut std::io::sink())?;
        if skipped != skip {
            bail!("the tar archive ends in the data of an entry");
        }
        self.left = 0;
        self.pad = 0;
        let mut h = [0u8; BLOCK];
        self.input
            .read_exact(&mut h)
            .context("the tar archive ends before its end blocks")?;
        let entry = parse_header(&h)?;
        if let Some(entry) = &entry {
            self.left = entry.size;
            self.pad = (BLOCK as u64 - entry.size % BLOCK as u64) % BLOCK as u64;
        }
        Ok(entry)
    }

    /// Copy the data of the current entry to `out`.
    pub fn copy_data(&mut self, out: &mut dyn Write) -> Result<u64> {
        let want = self.left;
        let n = std::io::copy(&mut (&mut self.input).take(want), out)?;
        self.left -= n;
        if n != want {
            bail!("the tar archive ends in the data of an entry");
        }
        Ok(n)
    }
}

/// Write the entries of `input` below `root`. A file that exists is not
/// replaced. Each file gets its mode without the set-id bits.
pub fn unpack<R: Read>(input: R, root: &Path) -> Result<Vec<Entry>> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let mut reader = Reader::new(input);
    let mut done = Vec::new();
    while let Some(entry) = reader.next_entry()? {
        let mut path = PathBuf::from(root);
        for part in Path::new(&entry.path).components() {
            match part {
                Component::Normal(name) => path.push(name),
                _ => bail!("the tar name {:?} leaves the root", entry.path),
            }
        }
        // A link that exists below the root is not followed.
        if let Some(parent) = path.parent() {
            let mut at = PathBuf::from(root);
            for part in parent.strip_prefix(root).unwrap_or(Path::new("")).iter() {
                at.push(part);
                if let Ok(meta) = std::fs::symlink_metadata(&at) {
                    if !meta.is_dir() {
                        bail!("{} is not a directory", at.display());
                    }
                }
            }
            std::fs::create_dir_all(parent)?;
        }
        let mode = entry.mode & 0o777;
        match entry.kind {
            Kind::Dir => {
                match std::fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_dir() => {}
                    Ok(_) => bail!("{} exists and is not a directory", path.display()),
                    Err(_) => std::fs::create_dir(&path)?,
                }
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode | 0o700))?;
            }
            Kind::File => {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&path)
                    .with_context(|| format!("creating {}", path.display()))?;
                reader.copy_data(&mut file)?;
            }
        }
        done.push(entry);
    }
    Ok(done)
}

fn mtime_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Add the tree at `dir` under the name `name`, as the static guard serves
/// it: a name that starts with a dot is left out; a link is followed only to
/// a regular file in `root` (the canonical root); a link to a directory, a
/// FIFO, a socket or a device is left out. A name that is not UTF-8 or that
/// does not fit in ustar is left out with a warning. The order is sorted.
pub fn append_tree<W: Write>(w: &mut Writer<W>, root: &Path, dir: &Path, name: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let meta = std::fs::metadata(dir)?;
    w.dir(name, meta.permissions().mode(), mtime_of(&meta))?;
    let mut children: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();
    children.sort();
    for child in children {
        let Some(text) = child.to_str() else {
            tracing::warn!(dir = %dir.display(), "tar: a name that is not UTF-8 is left out");
            continue;
        };
        if text.starts_with('.') {
            continue;
        }
        let path = dir.join(&child);
        let inner = format!("{name}/{text}");
        let Ok(link) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if link.is_dir() {
            match split_name(&format!("{inner}/")) {
                Ok(_) => append_tree(w, root, &path, &inner)?,
                Err(err) => tracing::warn!(error = %err, "tar: a directory is left out"),
            }
            continue;
        }
        if link.file_type().is_symlink() {
            match std::fs::canonicalize(&path) {
                Ok(real) if real.starts_with(root) => {}
                _ => continue,
            }
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if let Err(err) = split_name(&inner).and_then(|_| check_name(&inner)) {
            tracing::warn!(error = %err, "tar: an entry is left out");
            continue;
        }
        let Ok(mut file) = std::fs::File::open(&path) else {
            continue;
        };
        w.file(
            &inner,
            meta.permissions().mode(),
            mtime_of(&meta),
            meta.len(),
            &mut file,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(entries: &[(&str, Kind, &[u8])]) -> Vec<u8> {
        let mut w = Writer::new(Vec::new());
        for (path, kind, data) in entries {
            match kind {
                Kind::Dir => w.dir(path, 0o755, 1_700_000_000).unwrap(),
                Kind::File => w
                    .file(
                        path,
                        0o644,
                        1_700_000_000,
                        data.len() as u64,
                        &mut &data[..],
                    )
                    .unwrap(),
            }
        }
        w.finish().unwrap()
    }

    #[test]
    fn ustar_round_trip_keeps_odd_names() {
        let long_dir = "d".repeat(150);
        let long = format!("{long_dir}/{}", "f".repeat(99));
        let names = [
            ("top", Kind::Dir, &b""[..]),
            ("top/a b.txt", Kind::File, b"hello"),
            ("top/ünïcödé ✓", Kind::File, b""),
            ("top/x%20y&z=1;'\"", Kind::File, &[0u8, 1, 2, 255][..]),
            (long.as_str(), Kind::File, &[7u8; 513][..]),
        ];
        let bytes = archive(&names);
        assert_eq!(bytes.len() % BLOCK, 0);
        let mut reader = Reader::new(&bytes[..]);
        for (path, kind, data) in names {
            let entry = reader.next_entry().unwrap().unwrap();
            assert_eq!((entry.path.as_str(), entry.kind), (path, kind));
            let mut back = Vec::new();
            reader.copy_data(&mut back).unwrap();
            assert_eq!(back, data, "{path}");
        }
        assert!(reader.next_entry().unwrap().is_none());
        // GNU tar reads the same names, when it is on the host.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("t.tar");
        std::fs::write(&file, &bytes).unwrap();
        let out = std::process::Command::new("tar")
            .args(["--quoting-style=literal", "-tf"])
            .arg(&file)
            .output();
        if let Some(out) = out.ok().filter(|o| o.status.success()) {
            let list = String::from_utf8(out.stdout).unwrap();
            assert!(list.lines().any(|l| l == long), "{list}");
            assert!(list.lines().any(|l| l == "top/ünïcödé ✓"), "{list}");
        }
    }

    #[test]
    fn ustar_refuses_names_that_do_not_fit_or_leave_the_root() {
        let entry = |path: &str| Entry {
            path: path.to_string(),
            kind: Kind::File,
            mode: 0o644,
            mtime: 0,
            size: 0,
        };
        for bad in ["", "/etc/passwd", "a/../b", "./a", "a//b"] {
            assert!(header(&entry(bad)).is_err(), "{bad:?}");
        }
        assert!(header(&entry(&"x".repeat(101))).is_err());
        assert!(header(&entry(&format!("{}/{}", "p".repeat(156), "n"))).is_err());
        // A header that names `..` is refused by the reader too.
        let mut h = header(&entry("ab/cd")).unwrap();
        h[0..5].copy_from_slice(b"../cd");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        assert!(parse_header(&h).is_err());
        // A symbolic link is refused.
        let mut h = header(&entry("link")).unwrap();
        h[156] = b'2';
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        assert!(parse_header(&h).is_err());
        // A bad checksum is refused.
        let mut h = header(&entry("ok")).unwrap();
        h[0] = b'k';
        assert!(parse_header(&h).is_err());
    }

    #[test]
    fn ustar_big_sizes_use_base_256() {
        let entry = Entry {
            path: "big".into(),
            kind: Kind::File,
            mode: 0o600,
            mtime: 5,
            size: 9 << 30,
        };
        let h = header(&entry).unwrap();
        assert_eq!(h[124], 0x80);
        assert_eq!(parse_header(&h).unwrap().unwrap(), entry);
    }

    #[test]
    fn unpack_writes_below_the_root_only() {
        let bytes = archive(&[
            ("t", Kind::Dir, b""),
            ("t/s", Kind::Dir, b""),
            ("t/s/f.txt", Kind::File, b"data"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let entries = unpack(&bytes[..], dir.path()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            std::fs::read(dir.path().join("t/s/f.txt")).unwrap(),
            b"data"
        );
        // A second unpack does not replace the file.
        assert!(unpack(&bytes[..], dir.path()).is_err());
        // A link in the root is not followed.
        let other = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), root.path().join("t")).unwrap();
        assert!(unpack(&bytes[..], root.path()).is_err());
        assert_eq!(std::fs::read_dir(other.path()).unwrap().count(), 0);
    }
}
