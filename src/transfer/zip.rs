//! The zip of a directory, as magic-wormhole sends one (`zipfile/deflated`).
//! podnet writes stored entries (no compression: a zip reader of each
//! client reads them) into a file of its own, then sends that file. It
//! reads stored and deflated entries (`flate2`, the Rust backend).
//!
//! No zip64: a zip of more than 65,535 entries or an entry of 4 GiB or
//! more is refused at both ends.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use flate2::Crc;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// Made by Unix (3), version 2.0.
const MADE_BY: u16 = (3 << 8) | 20;
const NEEDED: u16 = 20;
/// The language encoding flag: the names are UTF-8.
const UTF8: u16 = 1 << 11;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;

/// What [`write()`] made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// The bytes of the zip.
    pub zipsize: u64,
    /// The bytes of the files.
    pub numbytes: u64,
    /// The files and the directories.
    pub numfiles: u64,
}

struct Central {
    name: Vec<u8>,
    crc: u32,
    size: u32,
    offset: u32,
    mode: u32,
}

/// Write the tree under `dir` into `out` as a zip of stored entries, each
/// name relative to `dir`, in a fixed order. A link, a device, a socket or
/// a fifo in the tree is an error: podnet sends regular files and
/// directories only.
pub fn write(dir: &Path, out: &mut File) -> io::Result<Written> {
    let mut entries = Vec::new();
    walk(dir, "", &mut entries)?;
    if entries.len() > usize::from(u16::MAX) {
        return Err(refused(format!(
            "{} entries: a zip of podnet holds 65,535 at most",
            entries.len()
        )));
    }
    let mut central = Vec::with_capacity(entries.len());
    let mut numbytes = 0u64;
    for (name, is_dir, mode) in &entries {
        let offset = u32::try_from(out.stream_position()?)
            .map_err(|_| refused("a zip of 4 GiB or more".into()))?;
        let mut crc = Crc::new();
        let mut size = 0u64;
        let header_at = out.stream_position()?;
        local_header(out, name.as_bytes(), 0, 0)?;
        if !is_dir {
            let mut file = File::open(dir.join(name))?;
            let mut buf = vec![0u8; 256 << 10];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                crc.update(&buf[..n]);
                out.write_all(&buf[..n])?;
                size += n as u64;
            }
        }
        let size32 =
            u32::try_from(size).map_err(|_| refused(format!("{name}: a file of 4 GiB or more")))?;
        let end = out.stream_position()?;
        out.seek(SeekFrom::Start(header_at))?;
        local_header(out, name.as_bytes(), crc.sum(), size32)?;
        out.seek(SeekFrom::Start(end))?;
        numbytes += size;
        let kind = if *is_dir { S_IFDIR } else { S_IFREG };
        central.push(Central {
            name: name.as_bytes().to_vec(),
            crc: crc.sum(),
            size: size32,
            offset,
            mode: kind | mode,
        });
    }
    let start = out.stream_position()?;
    for entry in &central {
        let mut row = Vec::with_capacity(46 + entry.name.len());
        row.extend_from_slice(&CENTRAL.to_le_bytes());
        row.extend_from_slice(&MADE_BY.to_le_bytes());
        row.extend_from_slice(&NEEDED.to_le_bytes());
        row.extend_from_slice(&UTF8.to_le_bytes());
        row.extend_from_slice(&STORED.to_le_bytes());
        row.extend_from_slice(&[0, 0, 0x21, 0]); // 1980-01-01 00:00
        row.extend_from_slice(&entry.crc.to_le_bytes());
        row.extend_from_slice(&entry.size.to_le_bytes());
        row.extend_from_slice(&entry.size.to_le_bytes());
        row.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        row.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
        let dos = if entry.mode & S_IFMT == S_IFDIR {
            0x10
        } else {
            0
        };
        row.extend_from_slice(&((entry.mode << 16) | dos).to_le_bytes());
        row.extend_from_slice(&entry.offset.to_le_bytes());
        row.extend_from_slice(&entry.name);
        out.write_all(&row)?;
    }
    let end = out.stream_position()?;
    let count = central.len() as u16;
    let too_big = || refused("a zip of 4 GiB or more".into());
    let mut tail = Vec::with_capacity(22);
    tail.extend_from_slice(&END.to_le_bytes());
    tail.extend_from_slice(&[0; 4]);
    tail.extend_from_slice(&count.to_le_bytes());
    tail.extend_from_slice(&count.to_le_bytes());
    tail.extend_from_slice(
        &u32::try_from(end - start)
            .map_err(|_| too_big())?
            .to_le_bytes(),
    );
    tail.extend_from_slice(&u32::try_from(start).map_err(|_| too_big())?.to_le_bytes());
    tail.extend_from_slice(&[0, 0]);
    out.write_all(&tail)?;
    out.flush()?;
    Ok(Written {
        zipsize: out.stream_position()?,
        numbytes,
        numfiles: central.len() as u64,
    })
}

fn local_header(out: &mut File, name: &[u8], crc: u32, size: u32) -> io::Result<()> {
    let mut row = Vec::with_capacity(30 + name.len());
    row.extend_from_slice(&LOCAL.to_le_bytes());
    row.extend_from_slice(&NEEDED.to_le_bytes());
    row.extend_from_slice(&UTF8.to_le_bytes());
    row.extend_from_slice(&STORED.to_le_bytes());
    row.extend_from_slice(&[0, 0, 0x21, 0]);
    row.extend_from_slice(&crc.to_le_bytes());
    row.extend_from_slice(&size.to_le_bytes());
    row.extend_from_slice(&size.to_le_bytes());
    row.extend_from_slice(&(name.len() as u16).to_le_bytes());
    row.extend_from_slice(&[0, 0]);
    row.extend_from_slice(name);
    out.write_all(&row)
}

/// Each entry under `dir/prefix`, sorted: (name, is a directory, the
/// permission bits).
fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, bool, u32)>) -> io::Result<()> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir.join(prefix))? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|n| refused(format!("{n:?}: a name that is not UTF-8")))?;
        names.push(name);
    }
    names.sort();
    for name in names {
        let path = format!("{prefix}{name}");
        let meta = fs::symlink_metadata(dir.join(&path))?;
        let mode = meta.permissions().mode() & 0o777;
        if meta.is_dir() {
            out.push((format!("{path}/"), true, mode));
            walk(dir, &format!("{path}/"), out)?;
        } else if meta.is_file() {
            out.push((path, false, mode));
        } else {
            let kind = if meta.file_type().is_symlink() {
                "a symbolic link"
            } else {
                "not a regular file"
            };
            return Err(refused(format!(
                "{path}: {kind}: podnet sends regular files and directories only"
            )));
        }
    }
    Ok(())
}

/// One entry of a zip that is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// The Unix type and mode of the entry; 0 when the zip gives none.
    pub mode: u32,
    pub method: u16,
    pub crc: u32,
    pub compressed: u64,
    /// The size that the zip says: a read stops after it.
    pub size: u64,
    pub offset: u64,
}

impl Entry {
    /// The kind of a Unix type that is not a regular file or a directory.
    pub fn odd_kind(&self) -> Option<&'static str> {
        match self.mode & S_IFMT {
            0 | S_IFREG | S_IFDIR => None,
            0o120_000 => Some("a symbolic link"),
            0o020_000 | 0o060_000 => Some("a device"),
            0o010_000 => Some("a fifo"),
            0o140_000 => Some("a socket"),
            _ => Some("not a regular file"),
        }
    }
}

/// The most bytes of one row of the central directory: the fixed part, a
/// path of `PATH_MOST` bytes, and 1024 bytes of extra fields and comment.
pub const ROW_MOST: u64 = 46 + super::land::PATH_MOST as u64 + 1024;

/// The directory of the zip in `file`: each entry, in order. A zip with more
/// than `most` entries is refused, and a central directory that is larger
/// than its entries need (`ROW_MOST` for each one) is refused before it is
/// read (T-347).
pub fn entries(file: &mut File, most: u64) -> io::Result<Vec<Entry>> {
    let length = file.seek(SeekFrom::End(0))?;
    let back = length.min(22 + 0xffff);
    let mut tail = vec![0u8; back as usize];
    file.seek(SeekFrom::Start(length - back))?;
    file.read_exact(&mut tail)?;
    let at = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| tail[i..i + 4] == END.to_le_bytes())
        .ok_or_else(|| refused("not a zip: no end of the central directory".into()))?;
    let end = &tail[at..];
    let count = u16_at(end, 10);
    let (size, start) = (u32_at(end, 12) as u64, u32_at(end, 16) as u64);
    if count == 0xffff || size == 0xffff_ffff || start == 0xffff_ffff {
        return Err(refused("a zip64: podnet reads no zip64".into()));
    }
    if start + size > length {
        return Err(refused("the central directory is outside the zip".into()));
    }
    if u64::from(count) > most {
        return Err(refused(format!(
            "{count} entries: the cap is {most} (--max-files)"
        )));
    }
    if size > u64::from(count) * ROW_MOST {
        return Err(refused(format!(
            "the central directory of {size} bytes is larger than its {count} entries need"
        )));
    }
    let mut dir = vec![0u8; size as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut dir)?;
    let mut out = Vec::with_capacity(usize::from(count));
    let mut at = 0usize;
    for _ in 0..count {
        let row = dir
            .get(at..at + 46)
            .ok_or_else(|| refused("the central directory is cut short".into()))?;
        if u32_at(row, 0) != CENTRAL {
            return Err(refused("a bad row of the central directory".into()));
        }
        let made_by = u16_at(row, 4) >> 8;
        let (method, crc) = (u16_at(row, 10), u32_at(row, 16));
        let (compressed, size) = (u32_at(row, 20) as u64, u32_at(row, 24) as u64);
        let (name_len, extra, comment) = (
            u16_at(row, 28) as usize,
            u16_at(row, 30) as usize,
            u16_at(row, 32) as usize,
        );
        let external = u32_at(row, 38);
        let offset = u32_at(row, 42) as u64;
        let name = dir
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(|| refused("a name is cut short".into()))?;
        let name = String::from_utf8(name.to_vec())
            .map_err(|_| refused("a name that is not UTF-8".into()))?;
        let mode = if made_by == 3 { external >> 16 } else { 0 };
        let is_dir = name.ends_with('/') || mode & S_IFMT == S_IFDIR;
        out.push(Entry {
            name,
            is_dir,
            mode,
            method,
            crc,
            compressed,
            size,
            offset,
        });
        at += 46 + name_len + extra + comment;
    }
    Ok(out)
}

/// Copy the data of `entry` into `out`: at most `entry.size` bytes, as a
/// zip may lie in a header. More data than the zip said, a CRC that
/// differs or a method other than stored and deflated is an error.
pub fn copy(file: &mut File, entry: &Entry, out: &mut dyn Write) -> io::Result<u64> {
    let mut head = [0u8; 30];
    file.seek(SeekFrom::Start(entry.offset))?;
    file.read_exact(&mut head)?;
    if u32_at(&head, 0) != LOCAL {
        return Err(refused(format!("{}: no local header", entry.name)));
    }
    let skip = u16_at(&head, 26) as u64 + u16_at(&head, 28) as u64;
    file.seek(SeekFrom::Start(entry.offset + 30 + skip))?;
    let raw = (&mut *file).take(entry.compressed);
    let mut data: Box<dyn Read> = match entry.method {
        STORED => Box::new(raw),
        DEFLATED => Box::new(flate2::read::DeflateDecoder::new(raw)),
        other => {
            return Err(refused(format!(
                "{}: the method {other}: podnet reads stored and deflated",
                entry.name
            )))
        }
    };
    let mut crc = Crc::new();
    let mut copied = 0u64;
    let mut buf = vec![0u8; 256 << 10];
    loop {
        let n = data.read(&mut buf)?;
        if n == 0 {
            break;
        }
        copied += n as u64;
        if copied > entry.size {
            return Err(refused(format!(
                "{}: more data than the {} bytes that the zip says",
                entry.name, entry.size
            )));
        }
        crc.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    if copied != entry.size || crc.sum() != entry.crc {
        return Err(refused(format!(
            "{}: the data does not agree with the zip (size or CRC)",
            entry.name
        )));
    }
    Ok(copied)
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn refused(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}
