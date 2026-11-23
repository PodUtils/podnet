//! T-516: check the program of the pod before a backend starts it. The
//! seccomp backend starts the program with `pod_hold::spawn::start`, and
//! a failed `execve` there exits 127 with no line (pod-hold v0.82.0 does not
//! report the errno). Each backend gets the same check here: one line and
//! exit 3. Remove the check of a path with a `/` when podnet takes the
//! tag of pod-hold that reports the errno of `execve`.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Check that `name` can run: a name with a `/` is a regular file with an
/// execute bit that this process can execute; another name is in the
/// directories of `path` (the `PATH` of the program). The error is one line,
/// in the form of the netns backend (`execvpe PATH: REASON`).
pub(crate) fn can_run(name: &str, path: Option<&OsStr>) -> anyhow::Result<()> {
    if !name.contains('/') {
        pod_hold::spawn::resolve(name, path)?;
        return Ok(());
    }
    let error = |e: std::io::Error| anyhow::anyhow!("execvpe {name}: {e}");
    let meta = std::fs::metadata(Path::new(name)).map_err(error)?;
    // execve(2) gives EACCES for a file that is not regular (a directory)
    // and for a file with no execute bit.
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(error(std::io::Error::from_raw_os_error(libc::EACCES)));
    }
    let Ok(c_name) = std::ffi::CString::new(OsStr::new(name).as_bytes()) else {
        return Err(anyhow::anyhow!("execvpe {name}: the path has a NUL byte"));
    };
    // SAFETY: a valid C string; access(2) only reads it.
    if unsafe { libc::access(c_name.as_ptr(), libc::X_OK) } != 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::can_run;
    use std::ffi::OsStr;

    /// One rule: a program that cannot run gives one line that names it
    /// and its cause; a program that can run passes.
    #[test]
    fn a_program_that_cannot_run_gives_one_line() {
        let dir = std::env::temp_dir().join(format!("podnet-program-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("plain");
        std::fs::write(&plain, "#!/bin/sh\n").unwrap();
        let plain = plain.to_str().unwrap().to_owned();
        let dir_name = dir.to_str().unwrap().to_owned();
        let rows: [(&str, Option<&str>); 6] = [
            ("/nonexistent/prog", Some("No such file or directory")),
            (&dir_name, Some("Permission denied")),
            (&plain, Some("Permission denied")),
            ("podnet-no-such-program", Some("is not in PATH")),
            ("/bin/sh", None),
            ("sh", None),
        ];
        let path = OsStr::new("/usr/bin:/bin");
        for (name, want) in rows {
            let got = can_run(name, Some(path)).map_err(|e| format!("{e:#}"));
            match (want, &got) {
                (None, Ok(())) => {}
                (Some(text), Err(line)) => {
                    assert!(line.contains(name) && line.contains(text), "{name}: {line}");
                    assert!(!line.contains('\n'), "{name}: more than one line: {line}");
                }
                _ => panic!("{name}: want {want:?}, got {got:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
