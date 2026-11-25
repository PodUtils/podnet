//! The host filesystem probe of podnet#500, FS-1 to FS-3.
//!
//! A host file system states five properties, as required or as emulated:
//! timestamps, atomic rename, hard links, permission bits and case
//! sensitivity. The probe runs at mount time, so a property that a file
//! system cannot give is known before the first transfer, not in the middle
//! of one.
//!
//! The fallback is part of the report. A FAT volume has a two-second
//! timestamp granularity and no permission bits: a sequence ordered by
//! timestamp is not reliable, so the runtime keeps its own serial number and
//! copies a file instead of linking it. A read-only mount refuses each write
//! with `Refusal::ReadOnly`, before the write starts.

use std::ffi::CString;
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// Whether a file system gives a property, or emulates it above the file
/// system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Required,
    Emulated,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Required => "required",
            Status::Emulated => "emulated",
        }
    }
}

/// One of the five properties, with the reason of its status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
}

/// The filesystem families that the probe can name from the image or from
/// the mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Fat,
    ExFat,
    Iso9660,
    Network,
    Memory,
    Unix,
    Other,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Fat => "fat",
            Family::ExFat => "exfat",
            Family::Iso9660 => "iso9660",
            Family::Network => "network",
            Family::Memory => "tmpfs",
            Family::Unix => "unix",
            Family::Other => "other",
        }
    }

    /// The file systems that cannot give the five properties.
    fn emulates(self) -> bool {
        matches!(self, Family::Fat | Family::ExFat | Family::Iso9660)
    }
}

/// The report of one mount or image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub path: PathBuf,
    pub family: Family,
    pub filesystem: String,
    pub read_only: bool,
    pub properties: Vec<Property>,
    pub fallback: String,
}

impl Report {
    /// Whether each property is required by this file system.
    pub fn required(&self) -> bool {
        self.properties.iter().all(|p| p.status == Status::Required)
    }

    /// The JSON document of the report.
    pub fn json(&self) -> String {
        let properties: Vec<serde_json::Value> = self
            .properties
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "status": p.status.as_str(),
                    "detail": p.detail,
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({
            // GitHub #724: the stamp of docs/cli.md; the form is in `kind`.
            "schema": 1,
            "kind": "podnet.host-probe/1",
            "podnet": podnet_core::build::version(),
            "path": self.path,
            "filesystem": self.filesystem,
            "family": self.family.name(),
            "read_only": self.read_only,
            "properties": properties,
            "fallback": self.fallback,
        }))
        .unwrap_or_else(|_| "{}".into())
    }
}

/// A named refusal, before a write starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub reason: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for Refusal {}

/// The status of the five properties for `family`.
pub fn properties(family: Family) -> Vec<Property> {
    let emulated = family.emulates();
    let reason = |what: &str| {
        if emulated {
            format!(
                "the {} file system cannot give {what}; podnet emulates it",
                family.name()
            )
        } else {
            format!("the {} file system gives {what}", family.name())
        }
    };
    vec![
        Property {
            name: "timestamps",
            status: if matches!(family, Family::Fat | Family::ExFat) {
                Status::Emulated
            } else {
                Status::Required
            },
            detail: if matches!(family, Family::Fat | Family::ExFat) {
                "the timestamp granularity is two seconds, so an order by timestamp is not reliable; keep a serial number in the transfer record".into()
            } else {
                reason("a timestamp with a sub-second granularity")
            },
        },
        Property {
            name: "atomic rename",
            status: if emulated {
                Status::Emulated
            } else {
                Status::Required
            },
            detail: if emulated {
                format!("{}: write to a new file, sync it, then rename; verify the length and the hash after the rename", family.name())
            } else {
                reason("an atomic rename(2)")
            },
        },
        Property {
            name: "hard links",
            status: if emulated {
                Status::Emulated
            } else {
                Status::Required
            },
            detail: if emulated {
                "link(2) is not available: copy the file and compare the hash, or keep one copy and a record".into()
            } else {
                reason("link(2)")
            },
        },
        Property {
            name: "permission bits",
            status: if emulated {
                Status::Emulated
            } else {
                Status::Required
            },
            detail: if emulated {
                "the file system has no permission bits; the mount options give one mode for each file, and podnet refuses a private file there".into()
            } else {
                reason("permission bits")
            },
        },
        Property {
            name: "case sensitivity",
            status: if matches!(family, Family::Fat | Family::ExFat | Family::Iso9660) {
                Status::Emulated
            } else {
                Status::Required
            },
            detail: if matches!(family, Family::Fat | Family::ExFat | Family::Iso9660) {
                "the file system ignores the case of a name; compare names as lower case before a write".into()
            } else {
                reason("case-sensitive names")
            },
        },
    ]
}

/// Probe `path` now, as at mount time. `path` can be a directory, a file or
/// an image: an image is read for its boot sector, so no mount is needed.
pub fn probe(path: &Path) -> anyhow::Result<Report> {
    let meta = std::fs::metadata(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let (family, filesystem) = detect(path, &meta)?;
    let read_only =
        meta.permissions().readonly() || access_refuses_write(path) || mount_is_read_only(path);
    let fallback = fallback(family, read_only);
    Ok(Report {
        path: path.to_path_buf(),
        family,
        filesystem,
        read_only,
        properties: properties(family),
        fallback,
    })
}

/// The fallback in the words of the runtime, from the report.
pub fn fallback(family: Family, read_only: bool) -> String {
    if read_only {
        return "read-only: the runtime keeps the mount read-only, refuses each write with EROFS before it starts, and writes state and keys to a writable directory".into();
    }
    if family.emulates() {
        return format!(
            "emulated: {} does not give all five properties; the runtime keeps a serial number, copies instead of hard links, verifies each hash after an atomic rename and compares names as lower case",
            family.name()
        );
    }
    "none: the file system gives the five properties".into()
}

/// Refuse a write to `report` before it starts, with a named reason.
pub fn require_writable(report: &Report) -> Result<(), Refusal> {
    if report.read_only {
        return Err(Refusal {
            reason: format!(
                "{}: the mount is read-only; the runtime refuses the write before it starts (INIT-3: no partial state)",
                report.path.display()
            ),
        });
    }
    Ok(())
}

fn access_refuses_write(path: &Path) -> bool {
    let Ok(text) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // access(2) with W_OK. A path that does not exist was refused above.
    unsafe { libc::access(text.as_ptr(), libc::W_OK) != 0 }
}

fn mount_is_read_only(path: &Path) -> bool {
    let Ok(text) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(text.as_ptr(), &mut stat) } != 0 {
        return false;
    }
    // ST_RDONLY is bit 0 of f_flag.
    stat.f_flag & libc::ST_RDONLY as libc::c_ulong != 0
}

fn detect(path: &Path, meta: &std::fs::Metadata) -> anyhow::Result<(Family, String)> {
    if meta.is_file() {
        if let Some((family, name)) = detect_image(path)? {
            return Ok((family, name));
        }
    }
    Ok(mount_family(path))
}

/// Detect a filesystem image from its boot sector, with no mount.
fn detect_image(path: &Path) -> anyhow::Result<Option<(Family, String)>> {
    let mut file =
        std::fs::File::open(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let mut head = vec![0u8; 512];
    let read = file
        .read(&mut head)
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    head.truncate(read);
    if head.len() >= 62 {
        let fat12 = &head[54..62];
        let fat32 = head.get(82..90);
        if fat12 == b"FAT12   " || fat12 == b"FAT16   " {
            let kind = if fat12 == b"FAT12   " {
                "fat12"
            } else {
                "fat16"
            };
            return Ok(Some((Family::Fat, kind.to_string())));
        }
        if fat32 == Some(&b"FAT32   "[..]) {
            return Ok(Some((Family::Fat, "fat32".into())));
        }
    }
    if head.len() < 512 {
        return Ok(None);
    }
    let mut sector = vec![0u8; 8];
    use std::io::Seek as _;
    if file.seek(std::io::SeekFrom::Start(0x8000)).is_ok()
        && file.read_exact(&mut sector).is_ok()
        && &sector[1..6] == b"CD001"
    {
        return Ok(Some((Family::Iso9660, "iso9660".into())));
    }
    Ok(None)
}

fn mount_family(path: &Path) -> (Family, String) {
    let Ok(text) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return (Family::Other, "unknown".into());
    };
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(text.as_ptr(), &mut stat) } != 0 {
        return (Family::Other, "unknown".into());
    }
    match stat.f_type as i64 {
        0x4d44 => (Family::Fat, "fat".into()),
        0x2011bab0 => (Family::ExFat, "exfat".into()),
        0x9660 => (Family::Iso9660, "iso9660".into()),
        0x6969 | 0xff534d42 | 0x517b => (Family::Network, "network".into()),
        0x01021994 => (Family::Memory, "tmpfs".into()),
        0xef53 | 0x9123683e | 0x58465342 | 0x2fc12fc1 => (Family::Unix, "unix".into()),
        _ => (Family::Other, "other".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FS-2: a FAT volume emulates timestamps, hard links, permission bits
    /// and case sensitivity; a read-only mount refuses a write with a name.
    #[test]
    fn a_fat_volume_states_emulations_and_a_read_only_mount_refuses() {
        let report = Report {
            path: PathBuf::from("/mnt/fat"),
            family: Family::Fat,
            filesystem: "fat32".into(),
            read_only: false,
            properties: properties(Family::Fat),
            fallback: fallback(Family::Fat, false),
        };
        for property in &report.properties {
            assert_eq!(property.status, Status::Emulated, "{property:?}");
        }
        assert!(report.fallback.contains("two seconds") || report.fallback.contains("emulated"));
        let readonly = Report {
            read_only: true,
            fallback: fallback(Family::Fat, true),
            ..report.clone()
        };
        assert!(require_writable(&readonly).is_err());
        assert!(readonly.fallback.contains("read-only"));
        assert!(properties(Family::Unix)
            .iter()
            .all(|p| p.status == Status::Required));
    }
}
