//! A program in the pod of another process (T-110: `warp exec --join`).
//!
//! * netns backend: the program enters the user, the mount and the network
//!   namespace of a process of the pod with `setns`. It then has the
//!   loopback, the TUN, the fake DNS and the `resolv.conf` of the pod.
//! * seccomp backend: there is no namespace. The caller starts a new pod
//!   with `--session` of the pod: the two pods share the inbound listeners
//!   and the fake DNS table.

use std::os::fd::{AsRawFd as _, OwnedFd};
use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};

/// The backend of a pod, as a process of the pod shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodKind {
    /// The process `pid` is in the namespaces of the pod.
    Netns(i32),
    /// The process is under the seccomp filter of the pod.
    Seccomp,
}

/// The namespace link of `ns` of `pid` (`self` for this process).
fn ns_link(pid: &str, ns: &str) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/ns/{ns}")).ok()
}

/// The count of seccomp filters of `pid` (`Seccomp_filters`, else the
/// mode of `Seccomp`).
fn filters(pid: &str) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let field = |key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().parse::<u32>().ok())
    };
    field("Seccomp_filters:").or_else(|| field("Seccomp:"))
}

/// The backend of the pod of `pid`, a process of the pod: another network
/// namespace than this process (netns), or more seccomp filters than this
/// process (seccomp). `None` for a process that shows neither.
pub fn kind_of(pid: i32) -> Option<PodKind> {
    let id = pid.to_string();
    let net = ns_link(&id, "net")?;
    if Some(net) != ns_link("self", "net") {
        return Some(PodKind::Netns(pid));
    }
    let theirs = filters(&id)?;
    (theirs > filters("self").unwrap_or(0)).then_some(PodKind::Seccomp)
}

/// Whether `pid` has its own mount namespace too: a process of a netns pod
/// has one after it put the `resolv.conf` of the pod in place.
pub fn has_own_mounts(pid: i32) -> bool {
    ns_link(&pid.to_string(), "mnt").is_some_and(|m| Some(m) != ns_link("self", "mnt"))
}

/// The environment of a program that joins a netns pod: the proxy
/// variables go, as for the program of the pod (issue 61).
fn without_proxy_variables(command: &mut std::process::Command) {
    for key in crate::PROXY_VARIABLES {
        command.env_remove(key);
    }
}

/// `program` in the namespaces of `pid` (a process of a netns pod): the
/// child enters the user namespace (when it is not the one of this
/// process), then the mount and the network namespace, and goes back to
/// the current directory. Spawn the command; `exec` from a process with
/// threads cannot enter a user namespace.
pub fn command_in(pid: i32, program: &[String]) -> Result<(std::process::Command, Vec<OwnedFd>)> {
    let Some((first, rest)) = program.split_first() else {
        bail!("no program");
    };
    let id = pid.to_string();
    let mut fds = Vec::new();
    for (ns, flag) in [
        ("user", libc::CLONE_NEWUSER),
        ("mnt", libc::CLONE_NEWNS),
        ("net", libc::CLONE_NEWNET),
    ] {
        let theirs = ns_link(&id, ns).with_context(|| format!("the {ns} namespace of {pid}"))?;
        if Some(theirs) == ns_link("self", ns) {
            continue;
        }
        let file = std::fs::File::open(format!("/proc/{pid}/ns/{ns}"))
            .with_context(|| format!("opening the {ns} namespace of {pid}"))?;
        fds.push((OwnedFd::from(file), flag));
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let cwd = std::ffi::CString::new(cwd.into_os_string().into_encoded_bytes())?;
    let raw: Vec<(i32, libc::c_int)> = fds.iter().map(|(fd, f)| (fd.as_raw_fd(), *f)).collect();
    let mut command = std::process::Command::new(first);
    command.args(rest);
    without_proxy_variables(&mut command);
    {
        use std::os::unix::process::CommandExt as _;
        // SAFETY: the closure calls only setns() and chdir() (async-signal
        // safe) with descriptors and a path that the parent holds.
        unsafe {
            command.pre_exec(move || {
                for (fd, flag) in &raw {
                    if libc::setns(*fd, *flag) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                // The new mount namespace starts at its root.
                if libc::chdir(cwd.as_ptr()) != 0 {
                    libc::chdir(c"/".as_ptr());
                }
                Ok(())
            });
        }
    }
    Ok((command, fds.into_iter().map(|(fd, _)| fd).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This process is in no pod.
    #[test]
    fn this_process_is_in_no_pod() {
        assert_eq!(kind_of(std::process::id() as i32), None);
    }
}
