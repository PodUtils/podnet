//! `podnet warp exec --join N -- CMD` (T-110): a program in the pod of the
//! instance N of `warp exec --detach` or `warp exec --name`.
//!
//! * netns: the program enters the namespaces of a process of the pod
//!   (`podnet_netns::join`). It has the loopback, the names and the routes
//!   of the pod.
//! * seccomp: a new pod with the socket, the rules and the session of N.
//!   The two pods share the inbound listeners (a loopback listener of N)
//!   and the fake DNS table (the names of N).

use std::time::{Duration, Instant};

use anyhow::Result;

use podnet::exit::{fault, Fault};
use podnet_netns::join::PodKind;

/// The longest wait for a process of the pod (a pod that starts now).
const FIND: Duration = Duration::from_secs(10);
/// The wait for the mount namespace of a netns pod.
const SETTLE: Duration = Duration::from_secs(1);

/// What `--join` reads from the ready document of N.
struct Target {
    socket: std::path::PathBuf,
    rules: Vec<String>,
    session: std::path::PathBuf,
    /// The process of the instance (the subreaper of the pod).
    pid: i32,
}

fn target(name: &str) -> Result<Target> {
    let dir = podnet::instances::run_dir()?;
    let unknown = || {
        fault(
            Fault::Unavailable,
            anyhow::anyhow!("no pod {name} (podnet ps)"),
        )
    };
    let record = podnet::instances::read(&dir, name)
        .filter(|_| podnet::instances::is_alive(&dir, name))
        .ok_or_else(unknown)?;
    if record.command != "warp exec" {
        return Err(fault(
            Fault::Usage,
            anyhow::anyhow!(
                "the instance {name} is not a pod of warp exec (it is {:?})",
                record.command
            ),
        ));
    }
    let ready = record
        .ready
        .filter(|_| record.state == "ready")
        .ok_or_else(|| {
            fault(
                Fault::Unavailable,
                anyhow::anyhow!("the pod {name} is not ready (podnet wait {name})"),
            )
        })?;
    let text = |key: &str| ready[key].as_str().map(std::path::PathBuf::from);
    let (Some(socket), Some(session)) = (text("socket"), text("session")) else {
        return Err(fault(
            Fault::Unavailable,
            anyhow::anyhow!("the ready document of {name} has no socket or no session"),
        ));
    };
    let rules = ready["rules"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(Target {
        socket,
        rules,
        session,
        pid: record.pid as i32,
    })
}

/// The backend of the pod of the instance process `pid`: the first
/// descendant that is in the pod. Waits [`FIND`] for a pod that starts. A
/// process of a netns pod with no own mount namespace yet is taken only
/// after [`SETTLE`]: the pod puts its `resolv.conf` in place first.
fn find_pod(name: &str, pid: i32) -> Result<PodKind> {
    let until = Instant::now() + FIND;
    let mut first_seen = None;
    loop {
        let found: Vec<PodKind> = super::kept::descendants(pid)
            .into_iter()
            .filter_map(podnet_netns::join::kind_of)
            .collect();
        let settled = found.iter().find(|kind| match kind {
            PodKind::Netns(member) => podnet_netns::join::has_own_mounts(*member),
            PodKind::Seccomp => true,
        });
        if let Some(kind) = settled {
            return Ok(*kind);
        }
        if let Some(kind) = found.first() {
            let seen = *first_seen.get_or_insert_with(Instant::now);
            if seen.elapsed() > SETTLE {
                return Ok(*kind);
            }
        }
        if Instant::now() > until {
            return Err(fault(
                Fault::Unavailable,
                anyhow::anyhow!("the pod {name} has no process in its pod now"),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `warp exec --join NAME -- PROGRAM`. Gives the exit code of the program.
pub(crate) async fn join(name: &str, program: &[String]) -> Result<i32> {
    use std::os::unix::process::ExitStatusExt as _;
    let target = target(name)?;
    let pid = target.pid;
    let owned = name.to_string();
    let kind = tokio::task::spawn_blocking(move || find_pod(&owned, pid)).await??;
    match kind {
        PodKind::Netns(member) => {
            let (mut command, fds) = podnet_netns::join::command_in(member, program)?;
            command.env("PODNET_WARP_SOCKET", &target.socket);
            let mut child = command
                .spawn()
                .map_err(|e| anyhow::anyhow!("running {} in the pod {name}: {e}", program[0]))?;
            drop(fds);
            let status = tokio::task::spawn_blocking(move || child.wait()).await??;
            Ok(status
                .code()
                .or_else(|| status.signal().map(|s| 128 + s))
                .unwrap_or(podnet::exit::FAILURE))
        }
        PodKind::Seccomp => {
            use std::os::unix::process::CommandExt as _;
            let exe = std::env::current_exe()?;
            let pod = super::pod::Pod {
                socket: &target.socket,
                backend: "seccomp",
                rules: &target.rules,
                session: Some(&target.session),
            };
            let err = super::pod::pod_command(exe, &pod, program).exec();
            Err(anyhow::Error::new(err).context("running podnet pod"))
        }
    }
}
