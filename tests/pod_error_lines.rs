//! T-452: the error paths of `podnet pod`. A program in a pod sees a failed
//! connect as `ECONNREFUSED` or a reset; podnet names the cause in one line.
//! Each row runs on each backend that the host has: the program connects
//! to one target two times, and the row checks the end within a limit, the
//! exit code, and each cause in exactly one line of stderr.

use std::process::Command;
use std::time::{Duration, Instant};

/// The program: connect to HOST PORT two times; print one word for each
/// try (ok, refused, reset, or the errno); exit 0.
const CLIENT: &str = r#"
import errno, socket, sys
for _ in range(2):
    s = socket.socket()
    s.settimeout(10)
    try:
        s.connect((sys.argv[1], int(sys.argv[2])))
        s.sendall(b"GET / HTTP/1.0\r\n\r\n")
        print("ok" if s.recv(1) else "reset")
    except ConnectionRefusedError:
        print("refused")
    except ConnectionResetError:
        print("reset")
    except OSError as e:
        print(errno.errorcode.get(e.errno, str(e)))
    s.close()
"#;

/// The backends that `podnet pod doctor` finds on this host.
fn backends() -> Vec<&'static str> {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["pod", "doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    [
        ("seccomp", "backend_seccomp"),
        ("netns", "backend_netns"),
        ("netns-kernel", "backend_netns_kernel"),
    ]
    .into_iter()
    .filter(|(_, key)| report[*key] == serde_json::Value::Bool(true))
    .map(|(name, _)| name)
    .collect()
}

fn python3() -> bool {
    Command::new("python3")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// One row: the flags of the pod, the target, and the text of the one
/// line that names the cause (`None`: the program's own `ECONNREFUSED` names
/// the cause, so no line of podnet says "Connection refused"; a host with
/// the loopback down gets a line with its true cause).
struct Row {
    name: &'static str,
    flags: Vec<String>,
    target: (String, u16),
    cause: Option<String>,
}

#[test]
fn each_failed_connect_names_its_cause_once() {
    if !python3() {
        eprintln!("skip: no python3 on this host");
        return;
    }
    let backends = backends();
    if backends.is_empty() {
        eprintln!("skip: no pod backend on this host");
        return;
    }
    // Port 1 of the loopback: nothing listens, and no bind is needed (a
    // restricted host refuses each bind of AF_INET).
    let dead: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
    let rows = [
        Row {
            name: "a name with no answer",
            flags: vec!["-x".into(), "direct".into()],
            target: ("nowhere.invalid".into(), 80),
            cause: Some("upstream for nowhere.invalid:80 failed".into()),
        },
        Row {
            name: "a reject rule",
            flags: vec![
                "-x".into(),
                "direct".into(),
                "-r".into(),
                "domain:lab.test=reject".into(),
            ],
            target: ("lab.test".into(), 80),
            cause: Some("refused by the rule domain:lab.test=reject".into()),
        },
        Row {
            name: "a dead upstream",
            flags: vec!["-x".into(), format!("socks5h://{dead}")],
            target: ("lab.test".into(), 80),
            cause: Some(format!("the upstream {dead} does not answer")),
        },
        Row {
            name: "a refused connect on the loopback",
            flags: vec!["-x".into(), "direct".into()],
            target: ("127.0.0.1".into(), dead.port()),
            cause: None,
        },
    ];
    let mut faults = Vec::new();
    for backend in &backends {
        for row in &rows {
            let start = Instant::now();
            let out = Command::new("timeout")
                .arg("60")
                .arg(env!("CARGO_BIN_EXE_podnet"))
                .args(["pod", "--backend", backend])
                .args(&row.flags)
                .args(["--", "python3", "-I", "-c", CLIENT])
                .arg(&row.target.0)
                .arg(row.target.1.to_string())
                .env_remove("PODNET_NETNS_QUIET")
                .env_remove("HTTPS_PROXY")
                .env_remove("https_proxy")
                .env_remove("HTTP_PROXY")
                .env_remove("http_proxy")
                .env_remove("ALL_PROXY")
                .env_remove("all_proxy")
                .env_remove("NO_PROXY")
                .env_remove("no_proxy")
                .output()
                .unwrap();
            let took = start.elapsed();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let id = format!("{backend}, {}", row.name);
            let seen: Vec<&str> = stdout.lines().collect();
            let podnet_lines: Vec<&str> = stderr
                .lines()
                .filter(|l| l.starts_with("podnet") && !l.contains(" backend: "))
                .collect();
            if took > Duration::from_secs(30) {
                faults.push(format!("{id}: took {took:?}"));
            }
            if out.status.code() != Some(0) {
                faults.push(format!("{id}: exit {:?}; {stderr}", out.status.code()));
            }
            // The program fails each try, and does not wait for its limit.
            if seen.len() != 2 || seen.iter().any(|w| !matches!(*w, "refused" | "reset")) {
                faults.push(format!("{id}: the program saw {seen:?}"));
            }
            match &row.cause {
                Some(cause) => {
                    let lines = podnet_lines
                        .iter()
                        .filter(|l| l.contains(cause.as_str()))
                        .count();
                    if lines != 1 || podnet_lines.len() != 1 {
                        faults.push(format!(
                            "{id}: want one line with {cause:?}; got {podnet_lines:?}"
                        ));
                    }
                }
                None if podnet_lines
                    .iter()
                    .any(|l| l.contains("Connection refused")) =>
                {
                    faults.push(format!(
                        "{id}: a second line for the refusal: {podnet_lines:?}"
                    ));
                }
                None => {}
            }
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

/// T-499: `/etc/resolv.conf` is a mount of a deleted file (the lab
/// `no-proxy-no-dns` of pod-lab; a container whose file was replaced). The
/// pod cannot mount its own file there; the program still resolves each name
/// through the pod, on each backend. Two host files: no nameserver (glibc
/// asks 127.0.0.1), and the file of systemd-resolved (127.0.0.53, edns0) of
/// the GitHub runner, where glibc takes an answer only from 127.0.0.53.
#[test]
fn a_resolv_conf_of_a_deleted_file_still_resolves() {
    if !python3() {
        eprintln!("skip: no python3 on this host");
        return;
    }
    let ok = |args: &[&str]| {
        Command::new("unshare")
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !ok(&["-rm", "true"]) {
        eprintln!("skip: no user and mount namespace on this host");
        return;
    }
    for conf in [
        "options attempts:1 timeout:1",
        "nameserver 127.0.0.53\\noptions edns0 trust-ad attempts:1 timeout:1\\nsearch .",
    ] {
        resolves_with_a_deleted_resolv_conf(conf);
    }
}

fn resolves_with_a_deleted_resolv_conf(conf: &str) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("resolv.conf");
    let script = format!(
        r#"set -e
printf '{conf}\\n' > "{file}"
target=$(readlink -f /etc/resolv.conf)
mount --bind "{file}" "$target"
rm "{file}"
grep -q " $target " /proc/self/mountinfo
for b in "$@"; do
  echo "backend $b"
  timeout 60 "{podnet}" pod --backend "$b" -x direct -- python3 -I -c '
import socket
try:
    print("resolved", socket.getaddrinfo("lab.test", 80)[0][4][0])
except OSError as e:
    print("failed", e)' || echo "exit $?"
done
"#,
        file = file.display(),
        podnet = env!("CARGO_BIN_EXE_podnet"),
    );
    let out = Command::new("unshare")
        .args(["-rm", "sh", "-c", &script, "sh"])
        .args(backends())
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}\n{stderr}");
    let mut faults = Vec::new();
    for block in stdout.split("backend ").skip(1) {
        let backend = block.lines().next().unwrap_or("");
        // A backend that the user namespace refuses exits 3 with its reason.
        if block.contains("exit 3") {
            eprintln!("{backend}: the namespace refuses the backend");
            continue;
        }
        if !block.contains("resolved 198.18.") {
            faults.push(format!("{conf:?}, {backend}: {block}"));
        }
    }
    assert!(faults.is_empty(), "{}\n{stderr}", faults.join("\n"));
}

/// T-482: each refusal of the netns backends gives one line of podnet with
/// its cause, and its exit code. Where the namespaces are refused, the child
/// prints the cause; before, the parent added a false second line ("write
/// uid/gid maps: the child died before it named its namespace").
#[test]
fn each_refusal_of_the_netns_backend_is_one_line() {
    let backends = backends();
    if !backends.contains(&"netns") {
        eprintln!("skip: no netns backend on this host");
        return;
    }
    let podnet = env!("CARGO_BIN_EXE_podnet");
    // In a user namespace, its root may lower the limits of new namespaces.
    let refused = format!(
        "echo 0 > /proc/sys/user/max_net_namespaces && echo 0 > /proc/sys/user/max_user_namespaces && exec {podnet} \"$@\""
    );
    let ns_ok = Command::new("unshare")
        .args(["-rU", "sh", "-c", &refused, "sh", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    let rows: [(&str, bool, &[&str], i32, &str); 3] = [
        (
            "the namespaces are refused",
            true,
            &["pod", "--backend", "netns", "-x", "direct", "--", "true"],
            3,
            "unshare(CLONE_NEWNET) failed",
        ),
        (
            "the program does not exist",
            false,
            &[
                "pod",
                "--backend",
                "netns",
                "-x",
                "direct",
                "--",
                "/nonexistent/prog",
            ],
            3,
            "execvpe /nonexistent/prog: No such file or directory",
        ),
        (
            "a flag of the seccomp backend",
            false,
            &[
                "pod",
                "--backend",
                "netns",
                "-x",
                "direct",
                "--inbound-dir",
                "/tmp",
                "--",
                "true",
            ],
            64,
            "--inbound-dir works with --backend seccomp only",
        ),
    ];
    let mut faults = Vec::new();
    for (name, in_userns, args, code, cause) in rows {
        let mut cmd = if in_userns {
            if !ns_ok {
                eprintln!("skip {name}: no user namespace for the limits");
                continue;
            }
            let mut c = Command::new("unshare");
            c.args(["-rU", "sh", "-c", &refused, "sh"]).args(args);
            c
        } else {
            let mut c = Command::new(podnet);
            c.args(args);
            c
        };
        for var in [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ] {
            cmd.env_remove(var);
        }
        let out = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = stderr.lines().filter(|l| l.starts_with("podnet")).collect();
        if out.status.code() != Some(code) {
            faults.push(format!(
                "{name}: exit {:?}, want {code}: {stderr}",
                out.status.code()
            ));
        }
        if lines.len() != 1 || !lines[0].contains(cause) {
            faults.push(format!(
                "{name}: want one line with {cause:?}; got {lines:?}"
            ));
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
