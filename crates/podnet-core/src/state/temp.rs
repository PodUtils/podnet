//! Temp paths with a random name (T-545; lesson 5 of PodUtils/podvm#567).
//!
//! A pid is not unique: two processes in two PID namespaces that share one
//! directory can have the same pid. A name of these functions has a nonce of
//! 64 bits from the random source of the kernel, and the directory or the
//! file is made with `mkdir` or `O_EXCL`, so a second process never takes or
//! removes the path of the first. An existing name gives a new name.
//!
//! The functions have the contract of the ask COORD-119 (PodUtils/podnet#711)
//! for `pod-core`; podnet takes that function when its tag is out.

use std::fs::File;
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

/// The number of names that a call tries before it gives up.
const TRIES: usize = 32;

/// 16 hex digits from the random source of the kernel. When the source
/// fails, the second path mixes the time, a counter and the random seed of
/// the hasher of the standard library (`RandomState`); the `O_EXCL` or the
/// `mkdir` of the caller still refuses a name that exists.
pub fn nonce() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_ok() {
        return format!("{:016x}", u64::from_le_bytes(bytes));
    }
    format!("{:016x}", fallback())
}

fn fallback() -> u64 {
    use std::hash::{BuildHasher as _, Hasher as _};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(nanos);
    hasher.write_u64(n);
    hasher.finish()
}

/// The path `PARENT/PREFIXNONCESUFFIX`. Nothing is made: the caller makes it
/// with `O_EXCL`, `mkdir` or `bind` (each refuses a path that exists).
pub fn random_path(parent: &Path, prefix: &str, suffix: &str) -> PathBuf {
    parent.join(format!("{prefix}{}{suffix}", nonce()))
}

/// Make a new directory `PARENT/PREFIXNONCE` with mode 0700. A name that
/// exists gives a new name; another error is returned as it is.
pub fn new_temp_dir(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    for _ in 0..TRIES {
        let path = random_path(parent, prefix, "");
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
        }
    }
    Err(exhausted(parent))
}

/// Make a new file `PARENT/PREFIXNONCESUFFIX` with `O_EXCL` and mode 0600,
/// and open it for writing. A name that exists gives a new name; another
/// error is returned as it is.
pub fn new_temp_file(parent: &Path, prefix: &str, suffix: &str) -> io::Result<(PathBuf, File)> {
    for _ in 0..TRIES {
        let path = random_path(parent, prefix, suffix);
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path);
        match opened {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display()))),
        }
    }
    Err(exhausted(parent))
}

/// A new name beside `path` for a temporary file: `.NAME.NONCE.tmp`, in the
/// directory of `path` (a rename then puts it at `path`).
pub fn beside(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(|| "file".into(), |name| name.to_string_lossy().into_owned());
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    random_path(parent, &format!(".{name}."), ".tmp")
}

fn exhausted(parent: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{}: {TRIES} random names exist already", parent.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn scratch() -> PathBuf {
        new_temp_dir(&std::env::temp_dir(), "podnet-temp-test-").unwrap()
    }

    /// T-545: two calls (as two processes with one pid in two PID
    /// namespaces) get two paths, and neither takes the path of the other;
    /// each kind of path gets its mode, and a planted name is not taken.
    #[test]
    fn temp_paths_are_new_private_and_distinct() {
        let root = scratch();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        type Make = fn(&Path) -> PathBuf;
        let cases: [(&str, Make, u32); 2] = [
            ("dir", |p| new_temp_dir(p, "d-").unwrap(), 0o700),
            ("file", |p| new_temp_file(p, "f-", ".x").unwrap().0, 0o600),
        ];
        for (kind, make, want) in cases {
            let (a, b) = (make(&root), make(&root));
            assert_ne!(a, b, "{kind}: two calls gave one path");
            assert_eq!(mode(&a), want, "{kind}: mode");
            assert!(
                a.starts_with(&root) && b.starts_with(&root),
                "{kind}: parent"
            );
        }
        assert_ne!(nonce(), nonce());
        assert_ne!(fallback(), fallback());
        // The old form: a path of the pid that another process holds.
        let planted = root.join(format!("pid-{}", std::process::id()));
        std::fs::write(&planted, "of another process").unwrap();
        let (mine, _) = new_temp_file(&root, &format!("pid-{}", std::process::id()), "").unwrap();
        assert_ne!(mine, planted);
        assert_eq!(
            std::fs::read_to_string(&planted).unwrap(),
            "of another process"
        );
        let side = beside(&root.join("capture.json"));
        assert!(side
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".capture.json."));
        assert_eq!(side.parent(), Some(root.as_path()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The lines of `text` (one source file) where a path is named by the
    /// pid: `process::id()` in a statement with `.join(`, `with_extension(`,
    /// `temp_dir()` or an abstract socket name, outside the test module at
    /// the end of the file.
    fn pid_paths(text: &str) -> Vec<usize> {
        let lines: Vec<&str> = text.lines().collect();
        let end = (0..lines.len())
            .find(|&i| {
                lines[i].starts_with("#[cfg(test)]")
                    && lines.get(i + 1).is_some_and(|l| l.starts_with("mod "))
            })
            .unwrap_or(lines.len());
        let marks = [".join(", "with_extension(", "temp_dir()", "abstract"];
        (0..end)
            .filter(|&i| lines[i].contains("process::id()"))
            .filter(|&i| {
                let window = lines[i.saturating_sub(4)..=i].join("\n");
                marks.iter().any(|m| window.contains(m))
            })
            .map(|i| i + 1)
            .collect()
    }

    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" && name != "fuzzing" {
                    sources(&path, out);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                out.push(path);
            }
        }
    }

    /// T-545, the class: no code of the workspace names a temp path, a
    /// socket or an abstract name by the pid. The plant must be found.
    #[test]
    fn no_temp_path_is_named_by_the_pid() {
        let plant = "fn f() {\n    let p = std::env::temp_dir()\n        .join(format!(\"x-{}\", std::process::id()));\n}\n#[cfg(test)]\nmod tests {\n    fn g() { dir.join(std::process::id()); }\n}\n";
        assert_eq!(pid_paths(plant), vec![3]);
        assert!(pid_paths("let r = Report { pid: std::process::id() };").is_empty());
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        sources(&root.join("src"), &mut files);
        for krate in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
            if krate.path().join("src").is_dir() {
                sources(&krate.path().join("src"), &mut files);
            }
        }
        assert!(files.len() > 100, "the scan read {} files", files.len());
        let faults: Vec<String> = files
            .iter()
            .flat_map(|f| {
                let text = std::fs::read_to_string(f).unwrap();
                pid_paths(&text)
                    .into_iter()
                    .map(move |n| format!("{}:{n}", f.display()))
            })
            .collect();
        assert!(
            faults.is_empty(),
            "a path named by the pid (T-545): {faults:#?}"
        );
    }

    /// A parent that does not exist gives its error with the path, not a loop.
    #[test]
    fn a_missing_parent_is_an_error_with_the_path() {
        let gone = Path::new("/nonexistent-podnet-parent");
        let e = new_temp_dir(gone, "d-").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(
            e.to_string().contains("/nonexistent-podnet-parent/d-"),
            "{e}"
        );
        let e = new_temp_file(gone, "f-", "").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }
}
