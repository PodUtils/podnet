//! `podnet warp exec --detach` and `--name` (T-109): the pod under the
//! supervisor of podnet.
//!
//! The command does not become the pod: it starts `podnet pod` as a child,
//! and is the subreaper of each process of the pod. A process that the
//! program starts and leaves (a daemon) thus stays a descendant of this
//! process. The command ends when no process of the pod lives, with the exit
//! code of the first program. A stop signal (`podnet stop N`) sends SIGTERM
//! to each descendant, and SIGKILL after [`GRACE`].

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _, Result};

use podnet::warp::pod::PodOptions;

/// The time from SIGTERM to SIGKILL at a stop.
const GRACE: Duration = Duration::from_secs(5);

/// The exit code of a `waitpid` status: the code, or 128 and the signal.
fn exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        128 + libc::WTERMSIG(status)
    }
}

/// The parent of each process (`/proc/PID/stat`, field 4).
fn parents() -> Vec<(i32, i32)> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // The name is in parentheses and can hold spaces.
            let rest = &stat[stat.rfind(')')? + 1..];
            let ppid = rest.split_whitespace().nth(1)?.parse().ok()?;
            Some((pid, ppid))
        })
        .collect()
}

/// Each descendant of `root` that lives now.
pub(crate) fn descendants(root: i32) -> Vec<i32> {
    let table = parents();
    let mut out = Vec::new();
    let mut next = vec![root];
    while let Some(parent) = next.pop() {
        for &(pid, ppid) in &table {
            if ppid == parent && !out.contains(&pid) {
                out.push(pid);
                next.push(pid);
            }
        }
    }
    out
}

/// Reap each child until none is left. Gives the exit code of `first`.
fn reap_all(first: i32) -> i32 {
    let mut code = None;
    loop {
        let mut status = 0;
        // SAFETY: waitpid() of any child, with a valid status pointer.
        let pid = unsafe { libc::waitpid(-1, &mut status, 0) };
        if pid == first {
            code = Some(exit_code(status));
        } else if pid < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EINTR) {
                return code.unwrap_or(0);
            }
        }
    }
}

/// SIGTERM to each descendant of this process; SIGKILL after [`GRACE`] to
/// each that still lives.
fn stop_all() {
    let me = std::process::id() as i32;
    for pid in descendants(me) {
        // SAFETY: a signal to a descendant of this process.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    let until = Instant::now() + GRACE;
    while Instant::now() < until && !descendants(me).is_empty() {
        std::thread::sleep(Duration::from_millis(50));
    }
    for pid in descendants(me) {
        // SAFETY: as above.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

/// The variables of `--detach` that the program of the pod does not get.
const DETACH_VARIABLES: [&str; 5] = [
    podnet::detach::NAME_VARIABLE,
    podnet::detach::HASH_VARIABLE,
    podnet::detach::COMMAND_VARIABLE,
    podnet::detach::SUPERVISED_VARIABLE,
    podnet::detach::READY_TIMEOUT_VARIABLE,
];

/// `warp exec --detach` (in the detached child) and `warp exec --name N`:
/// run the pod as a child of this process, and wait until no process of
/// the pod lives. Gives the exit code of the first program, or 0 after a
/// stop signal.
pub(crate) async fn run(options: &PodOptions, rules: &[String], program: &[String]) -> Result<i32> {
    let socket = podnet::warp::find::running(&options.at)?;
    let exe = std::env::current_exe()?;
    // SAFETY: prctl() with integer arguments.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        bail!(
            "prctl(PR_SET_CHILD_SUBREAPER): {}",
            std::io::Error::last_os_error()
        );
    }
    // The ready first: a detached child then leaves the terminal of the
    // caller, and the pod writes to the log of the instance.
    podnet::detach::ready(serde_json::json!({
        "socket": socket.display().to_string(),
        "backend": options.backend,
        "program": program,
    }));
    let mut command = super::pod::pod_command(exe, &socket, &options.backend, rules, program);
    for name in DETACH_VARIABLES {
        command.env_remove(name);
    }
    let child = command
        .stdin(Stdio::inherit())
        .spawn()
        .context("running podnet pod")?;
    let first = child.id() as i32;
    let reaper = tokio::task::spawn_blocking(move || reap_all(first));
    tokio::select! {
        code = reaper => Ok(code.unwrap_or(podnet::exit::FAILURE)),
        _ = crate::cli::signals::shutdown_signal() => {
            let _ = tokio::task::spawn_blocking(stop_all).await;
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The descendants of this process include a child and its child.
    #[test]
    fn descendants_include_the_children_of_a_child() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & wait"])
            .spawn()
            .unwrap();
        let me = std::process::id() as i32;
        let until = Instant::now() + Duration::from_secs(5);
        let found = loop {
            let all = descendants(me);
            if all.len() >= 2 || Instant::now() > until {
                break all;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(found.contains(&(child.id() as i32)), "{found:?}");
        assert!(found.len() >= 2, "{found:?}");
        for pid in found {
            // SAFETY: a signal to a descendant of this test.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let _ = child.wait();
    }

    #[test]
    fn exit_codes_of_a_status() {
        assert_eq!(exit_code(3 << 8), 3);
        assert_eq!(exit_code(libc::SIGTERM), 128 + libc::SIGTERM);
    }
}
