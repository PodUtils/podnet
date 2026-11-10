//! `podnet warp exec` and `podnet warp shell`: a program under the pod of
//! the running `podnet warp up` (issue 78, T-071, T-113).
//!
//! The pod gets `--backend` of the command, and `--name-backend`: it prints
//! one line on stderr that names the backend that runs, its limits and the
//! reason of the choice. `podnet -q` removes the line (the quiet variable of
//! podnet-netns). With `podnet -v`, the pod gets `-v` and logs each flow and
//! each emulation of the seccomp backend.
//!
//! T-113: `auto` tries the netns backend first, then the seccomp backend
//! when the user namespace or the TUN fails. The pod thus gets no
//! `--prefer-udp`. This module replaces the launcher of `src/warp/pod.rs`,
//! which is frozen until podnet takes `pod-warp`.

use std::path::Path;

use podnet::warp::pod::PodOptions;
use podnet::warp::up::{env_path, CA_VARIABLES, SHELL_VARIABLE};

/// The pod of a command: the socket of `warp up`, the backend, the rules,
/// and the directory of the session of a named pod (T-110).
#[derive(Clone, Copy)]
pub(crate) struct Pod<'a> {
    pub(crate) socket: &'a Path,
    pub(crate) backend: &'a str,
    pub(crate) rules: &'a [String],
    pub(crate) session: Option<&'a Path>,
}

/// True when podnet logs at the debug level (`podnet -v`): the pod then
/// gets `-v`.
fn verbose() -> bool {
    tracing::enabled!(target: "podnet", tracing::Level::DEBUG)
}

/// The arguments of `podnet pod` for the socket of `podnet warp up`. With
/// no `--prefer-udp`, `auto` takes the netns backend first (T-113).
pub(crate) fn pod_args(pod: &Pod<'_>, verbose: bool, program: &[String]) -> Vec<String> {
    let Pod {
        socket,
        backend,
        rules,
        session,
    } = *pod;
    let mut argv = vec![
        "pod".to_string(),
        "--backend".to_string(),
        backend.to_string(),
        "--name-backend".to_string(),
    ];
    if verbose {
        argv.push("-v".into());
    }
    // T-110: the pods of a session share the state of the seccomp backend.
    if let Some(dir) = session {
        argv.push("--session".into());
        argv.push(dir.display().to_string());
    }
    argv.push("-x".into());
    argv.push(format!("socks5://unix:{}", socket.display()));
    for rule in rules {
        argv.push("-r".into());
        argv.push(rule.clone());
    }
    argv.push("--".into());
    argv.extend(program.iter().cloned());
    argv
}

/// The CA variables of the env file (the fix of issue 86), for a command
/// that runs under the pod and does not read the env file.
fn ca_exports() -> Vec<(String, String)> {
    let Ok(text) = env_path().and_then(|p| Ok(std::fs::read_to_string(p)?)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| line.strip_prefix("export ")?.split_once('='))
        .filter(|(k, _)| CA_VARIABLES.contains(k))
        .map(|(k, v)| (k.to_string(), v.trim_matches('\'').to_string()))
        .collect()
}

/// `podnet warp exec -- CMD`: run CMD under the pod (this process becomes
/// it). Gives the error when it cannot.
pub(crate) fn exec_in_pod(
    options: &PodOptions,
    rules: &[String],
    program: &[String],
) -> anyhow::Error {
    use std::os::unix::process::CommandExt as _;
    let socket = match podnet::warp::find::running(&options.at) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return e.into(),
    };
    let pod = Pod {
        socket: &socket,
        backend: &options.backend,
        rules,
        session: None,
    };
    let err = pod_command(exe, &pod, program).exec();
    anyhow::Error::new(err).context("running podnet pod")
}

/// The command of `warp exec` (issue 116): the program gets
/// PODNET_WARP_SOCKET, also when the shell did not read the env file.
pub(crate) fn pod_command(
    exe: std::path::PathBuf,
    pod: &Pod<'_>,
    program: &[String],
) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command
        .args(pod_args(pod, verbose(), program))
        .env("PODNET_WARP_SOCKET", pod.socket)
        .envs(ca_exports());
    command
}

/// `podnet warp shell`: `$SHELL` under the pod; when the proxy does not
/// answer, `$SHELL` as it is, with the proxy variables of the env file.
pub(crate) fn shell(options: &PodOptions) -> anyhow::Error {
    use std::os::unix::process::CommandExt as _;
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    match (
        podnet::warp::find::running(&options.at),
        std::env::current_exe(),
    ) {
        (Ok(socket), Ok(exe)) => {
            let program = std::slice::from_ref(&shell);
            let pod = Pod {
                socket: &socket,
                backend: &options.backend,
                rules: &[],
                session: None,
            };
            let err = pod_command(exe, &pod, program)
                .env(SHELL_VARIABLE, "1")
                .exec();
            eprintln!("podnet: warp shell: {err}; the shell starts with no pod");
        }
        (Err(err), _) => eprintln!("podnet: warp shell: {err}; the shell starts with no pod"),
        (_, Err(err)) => eprintln!("podnet: warp shell: {err}; the shell starts with no pod"),
    }
    let mut cmd = std::process::Command::new(&shell);
    cmd.env(SHELL_VARIABLE, "1");
    if let Ok(text) = env_path().and_then(|p| Ok(std::fs::read_to_string(p)?)) {
        for line in text.lines() {
            if let Some((k, v)) = line.strip_prefix("export ").and_then(|l| l.split_once('=')) {
                cmd.env(k, v.trim_matches('\''));
            }
        }
    }
    let err = cmd.exec();
    if err.kind() == std::io::ErrorKind::NotFound {
        return anyhow::anyhow!("no shell at {shell}");
    }
    anyhow::Error::new(err).context(format!("running {shell}"))
}

#[cfg(test)]
#[path = "pod_tests.rs"]
mod tests;
