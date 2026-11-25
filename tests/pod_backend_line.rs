//! T-071: `warp exec` and `warp shell` take `--backend` and give it to the
//! pod; the pod names the backend in one line on stderr; with `-v`, the
//! seccomp backend logs each socket and each `setsockopt` that it emulates.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn podnet() -> Command {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
}

/// The backends that `podnet pod doctor` finds on this host.
fn backends() -> (bool, bool) {
    let out = podnet().args(["pod", "doctor", "--json"]).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let yes = |key: &str| report[key] == serde_json::Value::Bool(true);
    (yes("backend_netns"), yes("backend_seccomp"))
}

/// A C compiler: `$CC`, then `cc`, `gcc` or `clang` on `PATH`.
fn compiler() -> Option<PathBuf> {
    let mut names: Vec<String> = std::env::var("CC").into_iter().collect();
    names.extend(["cc", "gcc", "clang"].map(String::from));
    let path = std::env::var_os("PATH")?;
    names.iter().find_map(|name| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

/// A program that makes a TCP socket, sets `TCP_NODELAY` on it, makes a
/// UDP socket of IPv6, and exits 0. It opens no connection.
const CLIENT: &str = r#"
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

int main(void) {
    int t = socket(AF_INET, SOCK_STREAM, 0);
    int one = 1;
    if (t < 0 || setsockopt(t, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one) != 0) {
        perror("tcp");
        return 1;
    }
    int u = socket(AF_INET6, SOCK_DGRAM, 0);
    if (u < 0) { perror("udp"); return 1; }
    close(t);
    close(u);
    return 0;
}
"#;

fn build(dir: &Path, cc: &Path) -> PathBuf {
    let src = dir.join("client.c");
    std::fs::write(&src, CLIENT).unwrap();
    let out = dir.join("client");
    let status = Command::new(cc)
        .arg("-O2")
        .arg("-o")
        .arg(&out)
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success(), "compiling the client failed");
    out
}

/// A temporary directory with a unix socket that answers (a stand-in of
/// the proxy of `warp up`: no program here opens a connection).
struct Host {
    dir: tempfile::TempDir,
    socket: PathBuf,
    _listener: UnixListener,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("warp.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        Host {
            dir,
            socket,
            _listener: listener,
        }
    }

    /// `podnet GLOBAL warp exec --socket S --backend B -- PROGRAM`.
    fn warp_exec(&self, global: &[&str], backend: &str, program: &Path) -> Output {
        self.warp_exec_with(global, backend, program, &[])
    }

    /// [`Host::warp_exec`] with the variables `env`.
    fn warp_exec_with(
        &self,
        global: &[&str],
        backend: &str,
        program: &Path,
        env: &[(&str, &str)],
    ) -> Output {
        let home = self.dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let out = podnet()
            .args(global)
            .args(["warp", "exec", "--socket"])
            .arg(&self.socket)
            .args(["--backend", backend, "--"])
            .arg(program)
            .env("HOME", &home)
            .env("PODNET_STATE_DIR", self.dir.path().join("state"))
            .env("XDG_RUNTIME_DIR", self.dir.path())
            .env_remove("PODNET_WARP_SOCKET")
            .env_remove("PODNET_NETNS_QUIET")
            .env_remove("PODNET_NETNS_TEST_NO_NETNS")
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        out
    }

    /// `podnet pod OPTIONS -x unix:S -- PROGRAM`.
    fn pod(&self, options: &[&str], program: &Path) -> Output {
        let out = podnet()
            .arg("pod")
            .args(options)
            .arg("-x")
            .arg(format!("unix:{}", self.socket.display()))
            .arg("--")
            .arg(program)
            .env_remove("PODNET_NETNS_QUIET")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        out
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The lines of stderr that name a backend.
fn backend_lines(out: &Output) -> Vec<String> {
    stderr(out)
        .lines()
        .filter(|l| l.contains(" backend: "))
        .map(String::from)
        .collect()
}

#[test]
fn warp_exec_names_its_backend_once() {
    let (netns, seccomp) = backends();
    if !netns && !seccomp {
        eprintln!("skipping: no backend of the pod on this host");
        return;
    }
    let host = Host::new();
    let truth = Path::new("true");
    let out = host.warp_exec(&[], "auto", truth);
    let lines = backend_lines(&out);
    assert_eq!(lines.len(), 1, "{}", stderr(&out));
    assert!(
        lines[0].contains("seccomp backend: ") || lines[0].contains("netns backend: "),
        "{lines:?}"
    );
    // GitHub #713: where netns cannot run, the line says so in place of
    // the step to it.
    assert!(
        lines[0].contains("use --backend")
            || lines[0].contains("the netns backend cannot run here"),
        "{lines:?}"
    );
    // -q removes the line.
    let out = host.warp_exec(&["-q"], "auto", truth);
    assert!(backend_lines(&out).is_empty(), "{}", stderr(&out));
    // podnet pod names the backend only when it is asked to.
    let backend = if seccomp { "seccomp" } else { "netns" };
    let out = host.pod(&["--backend", backend], truth);
    assert!(backend_lines(&out).is_empty(), "{}", stderr(&out));
}

#[test]
fn warp_exec_passes_the_backend_to_the_pod() {
    let (netns, seccomp) = backends();
    let host = Host::new();
    let truth = Path::new("true");
    for (backend, available) in [("netns", netns), ("seccomp", seccomp)] {
        if !available {
            eprintln!("skipping {backend}: not available on this host");
            continue;
        }
        let out = host.warp_exec(&[], backend, truth);
        let lines = backend_lines(&out);
        assert_eq!(lines.len(), 1, "{}", stderr(&out));
        assert!(
            lines[0].contains(&format!("{backend} backend: ")),
            "{lines:?}"
        );
    }
    // podnet -v warp exec gives -v to the pod.
    let Some(cc) = compiler() else {
        eprintln!("skipping -v: no C compiler");
        return;
    };
    if !seccomp {
        eprintln!("skipping -v: no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let client = build(host.dir.path(), &cc);
    let out = host.warp_exec(&["-v"], "seccomp", &client);
    assert!(
        stderr(&out).contains("socket emulated as a socket pair"),
        "{}",
        stderr(&out)
    );
    let out = host.warp_exec(&[], "seccomp", &client);
    assert!(
        !stderr(&out).contains("socket emulated"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn seccomp_logs_its_emulation_at_info() {
    let Some(cc) = compiler() else {
        eprintln!("skipping: no C compiler");
        return;
    };
    if !backends().1 {
        eprintln!("skipping: no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let host = Host::new();
    let client = build(host.dir.path(), &cc);
    let out = host.pod(&["--backend", "seccomp", "-v"], &client);
    let err = stderr(&out);
    for want in [
        "podnet pod: socket emulated as a socket pair (IPv4 TCP)",
        "podnet pod: socket emulated as a socket pair (IPv6 UDP)",
        "podnet pod: setsockopt answered by the pod (fd ",
    ] {
        assert!(
            err.lines().any(|l| l.starts_with(want)),
            "{want:?} in:\n{err}"
        );
    }
    // Without -v: no line about the emulation.
    let out = host.pod(&["--backend", "seccomp"], &client);
    let err = stderr(&out);
    assert!(!err.contains("socket emulated"), "{err}");
    assert!(!err.contains("setsockopt answered"), "{err}");
}

/// T-113: `auto` of `warp exec` takes the netns backend first, and the line
/// gives the reason.
#[test]
fn warp_exec_tries_netns_first() {
    let (netns, _) = backends();
    if !netns {
        eprintln!("skipping: the netns backend cannot run on this host");
        return;
    }
    let host = Host::new();
    let out = host.warp_exec(&[], "auto", Path::new("true"));
    let lines = backend_lines(&out);
    assert_eq!(lines.len(), 1, "{}", stderr(&out));
    assert!(lines[0].contains("netns backend: "), "{lines:?}");
    assert!(
        lines[0].contains("Choice: auto takes netns first; "),
        "{lines:?}"
    );
    // An explicit backend gives that reason.
    let out = host.warp_exec(&[], "netns", Path::new("true"));
    assert!(
        backend_lines(&out)[0].contains("Choice: --backend netns"),
        "{}",
        stderr(&out)
    );
}

/// T-113: when the netns backend cannot run, `auto` of `warp exec` takes
/// the seccomp backend, and the line names the failure.
#[test]
fn warp_exec_falls_back_to_seccomp() {
    let (_, seccomp) = backends();
    if !seccomp {
        eprintln!("skipping: no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let host = Host::new();
    let hook = [("PODNET_NETNS_TEST_NO_NETNS", "1")];
    let out = host.warp_exec_with(&[], "auto", Path::new("true"), &hook);
    let lines = backend_lines(&out);
    assert_eq!(lines.len(), 1, "{}", stderr(&out));
    assert!(lines[0].contains("seccomp backend: "), "{lines:?}");
    assert!(
        lines[0].contains(
            "Choice: auto takes netns first, but netns cannot run here (PODNET_NETNS_TEST_NO_NETNS is set)"
        ),
        "{lines:?}"
    );
}

/// Refuse each TCP connect and bind of the calling process with Landlock
/// (ABI 4 or later), as the lab `bind-denied` of pod-lab does. Call it in
/// the child, before the exec. The refusal holds in each network namespace.
fn landlock_refuse_tcp() -> std::io::Result<()> {
    // struct landlock_ruleset_attr of ABI 4: handled_access_fs and
    // handled_access_net.
    let attr: [u64; 2] = [0, 0b11]; // LANDLOCK_ACCESS_NET_BIND_TCP | _CONNECT_TCP
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            attr.as_ptr(),
            std::mem::size_of_val(&attr),
            0,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = fd as libc::c_int;
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe { libc::syscall(libc::SYS_landlock_restrict_self, fd, 0) } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    unsafe { libc::close(fd) };
    Ok(())
}

/// The Landlock ABI of this kernel, or 0.
fn landlock_abi() -> i64 {
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0,
            1, // LANDLOCK_CREATE_RULESET_VERSION
        )
    };
    abi.max(0)
}

/// `podnet pod --backend B --name-backend -x socks5://unix:S -- true` with
/// each TCP connect of podnet and of the program refused by Landlock.
fn pod_with_tcp_refused(host: &Host, backend: &str) -> Output {
    use std::os::unix::process::CommandExt;
    let mut cmd = podnet();
    cmd.args(["pod", "--backend", backend, "--name-backend", "-x"])
        .arg(format!("socks5://unix:{}", host.socket.display()))
        .args(["--", "true"])
        .env("HOME", host.dir.path())
        .env("XDG_RUNTIME_DIR", host.dir.path())
        .env_remove("PODNET_NETNS_QUIET")
        .env_remove("PODNET_NETNS_TEST_NO_NETNS");
    unsafe { cmd.pre_exec(landlock_refuse_tcp) };
    cmd.output().unwrap()
}

/// T-382: where each TCP connect is refused in each namespace (Landlock:
/// the lab `bind-denied`), the netns backend cannot carry a TCP flow. An
/// explicit `--backend netns` exits 3 with the reason; `auto` takes the
/// seccomp backend and names the reason.
#[test]
fn netns_is_refused_where_tcp_connect_is_refused() {
    if landlock_abi() < 4 {
        eprintln!("skipping: no Landlock with TCP rules (ABI 4) on this host");
        return;
    }
    let (netns, seccomp) = backends();
    if !netns {
        eprintln!("skipping: the netns backend cannot run on this host");
        return;
    }
    let host = Host::new();
    let out = pod_with_tcp_refused(&host, "netns");
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("the netns backend cannot carry a TCP connect here"),
        "{}",
        stderr(&out)
    );
    if !seccomp {
        return;
    }
    let out = pod_with_tcp_refused(&host, "auto");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let lines = backend_lines(&out);
    assert!(lines[0].contains("seccomp backend: "), "{lines:?}");
    assert!(
        lines[0].contains("auto takes netns first, but netns cannot run here (a TCP connect"),
        "{lines:?}"
    );
}

/// Refuse each `unshare` of the calling process and of its children with
/// `EPERM` (a seccomp filter), as the lab `userns-off` of pod-lab refuses a
/// new user namespace. Call it in the child, before the exec.
fn seccomp_refuse_unshare() -> std::io::Result<()> {
    let nr = libc::SYS_unshare as u32;
    let refuse = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    let filter = [
        // Load the number of the system call.
        libc::sock_filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        // unshare: the next instruction, else skip it.
        libc::sock_filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: nr,
        },
        libc::sock_filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: refuse,
        },
        libc::sock_filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ALLOW,
        },
    ];
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut libc::sock_filter,
    };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        || unsafe {
            libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &program as *const libc::sock_fprog,
            )
        } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// `podnet pod ARGS` with `fault` in the process (a hook of `pre_exec`).
fn pod_under(fault: fn() -> std::io::Result<()>, host: &Host, args: &[&str]) -> Output {
    use std::os::unix::process::CommandExt;
    let mut cmd = podnet();
    cmd.arg("pod")
        .args(args)
        .env("HOME", host.dir.path())
        .env("XDG_RUNTIME_DIR", host.dir.path())
        .env_remove("PODNET_NETNS_QUIET")
        .env_remove("PODNET_NETNS_TEST_NO_NETNS");
    unsafe { cmd.pre_exec(fault) };
    cmd.output().unwrap()
}

/// GitHub #713 and #740: where the netns backend cannot run (each TCP
/// connect refused: the lab `bind-denied`; each `unshare` refused: the lab
/// `userns-off`), `pod doctor` says `fail` for netns and netns-kernel,
/// `--backend netns` exits 3 with one line that has the cause and the next
/// step, its report names no backend and holds that line, and the seccomp
/// line of `auto` does not send the user to `--backend netns`.
#[test]
fn the_doctor_and_the_start_agree_where_netns_cannot_run() {
    let (netns, seccomp) = backends();
    if !netns {
        eprintln!("skipping: the netns backend cannot run on this host");
        return;
    }
    type Fault = fn() -> std::io::Result<()>;
    // (the fault, its name, the words of the line)
    let mut table: Vec<(Fault, &str, &str)> = vec![(
        seccomp_refuse_unshare,
        "unshare refused",
        "the netns backend cannot run here (unshare(NEWUSER|NEWNET): Operation not permitted",
    )];
    if landlock_abi() >= 4 {
        table.push((
            landlock_refuse_tcp,
            "TCP connect refused",
            "the netns backend cannot carry a TCP connect here",
        ));
    }
    let host = Host::new();
    let proxy = format!("socks5://unix:{}", host.socket.display());
    let report = host.dir.path().join("report.json");
    let report_arg = report.display().to_string();
    let mut faults = Vec::new();
    for (fault, name, words) in table {
        let out = pod_under(fault, &host, &["doctor", "--json"]);
        let doctor: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        for key in ["backend_netns", "backend_netns_kernel"] {
            if doctor[key] != serde_json::Value::Bool(false) {
                faults.push(format!("{name}: doctor {key} is {}", doctor[key]));
            }
        }
        if doctor["verdicts"]["netns"] != "fail" || doctor["hold"]["netns"]["level"] != "none" {
            faults.push(format!("{name}: doctor says that netns can run: {doctor}"));
        }
        let args = [
            "--backend",
            "netns",
            "--report",
            &report_arg,
            "-x",
            &proxy,
            "--",
            "true",
        ];
        let out = pod_under(fault, &host, &args);
        let text = stderr(&out);
        if out.status.code() != Some(3) || !text.contains(words) {
            faults.push(format!("{name}: --backend netns: {:?} {text}", out.status));
        }
        let lines: Vec<&str> = text.lines().filter(|l| !l.contains("warning")).collect();
        if lines.len() != 1 || !lines[0].contains("; use --backend seccomp") {
            faults.push(format!("{name}: not one line with the next step: {text}"));
        }
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
        let reason = json["reason"].as_str().unwrap_or_default();
        if json["backend"] != serde_json::Value::Null
            || json["hold"] != "none"
            || !reason.contains(words)
        {
            faults.push(format!("{name}: the report is not true: {json}"));
        }
        if !seccomp {
            continue;
        }
        let args = ["--name-backend", "-x", &proxy, "--", "true"];
        let out = pod_under(fault, &host, &args);
        let lines = backend_lines(&out);
        if !out.status.success() || lines.len() != 1 || lines[0].contains("use --backend netns") {
            faults.push(format!("{name}: auto: {lines:?} {}", stderr(&out)));
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
