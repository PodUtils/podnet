//! T-516: check the program of the pod before a backend starts it, with
//! `pod_hold::spawn::resolve` (a name in `PATH`, or a path with a `/`: an
//! absent path, a directory and a file with no execute bit fail). Each
//! backend gets the same check: one line and exit 3. pod-hold v0.83.0 also
//! reports the errno of a failed `execve` of `spawn::start`
//! (PodUtils/podcrates@T-1125).

use std::ffi::OsStr;

/// Check that `name` can run: a name with a `/` is a file that this process
/// can execute; another name is in the directories of `path` (the `PATH`
/// of the program). The error is one line that names the program.
pub(crate) fn can_run(name: &str, path: Option<&OsStr>) -> anyhow::Result<()> {
    pod_hold::spawn::resolve(name, path)?;
    Ok(())
}

/// The path that `execve` of a backend runs: `name` found as `execvp`
/// finds it (a name with a `/` as it is, another name in `PATH`). Each
/// backend calls `execve` with it, not `execvpe`: `execvpe` runs a file
/// that the kernel refuses (ENOEXEC) with `/bin/sh`, and the pod then
/// exits 127 with no line (T-516).
pub(crate) fn exec_path(name: &str) -> anyhow::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt as _;
    let path = std::env::var_os("PATH");
    let found = pod_hold::spawn::resolve(name, path.as_deref())?;
    Ok(std::ffi::CString::new(found.as_os_str().as_bytes())?)
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
