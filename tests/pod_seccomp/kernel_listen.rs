//! `listen` on a socket of the kernel.
//!
//! When the pod cannot write the memory of the program, `socket(AF_INET,
//! SOCK_STREAM)` returns a real TCP socket of the kernel (T-456). `bind` and
//! `listen` were continued to the kernel for that socket, so `listen` with no
//! `bind` made the kernel autobind a wildcard address and opened a port of the
//! host. The pod refuses the call now.
//!
//! Three programs are run, each with the read-only memory path forced by
//! `PODNET_NETNS_TEST_MEM=none`, so the test also runs on a host that allows
//! the write: a bare `listen` on IPv4, a `bind` followed by `listen`, and a
//! bare `listen` on IPv6. Each run must show two things. The program's own
//! output must say the call was refused with `EPERM`, and the host's
//! `/proc/net/tcp` and `/proc/net/tcp6` must gain no LISTEN row whose inode
//! belongs to the program's sockets. The allowed path, a socket the pod
//! owns, is `seccomp_backend_accepts_inbound` in `listen.rs`.

use super::*;
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::process::Stdio;

/// A bare `listen()` on an unbound TCP socket, then a pause to inspect the
/// host while the socket is alive.
const BARE_LISTEN: &str = r#"
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
int main(void) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) { printf("socket -1 %d\n", errno); return 1; }
    errno = 0;
    int r = listen(fd, 8);
    printf("listen %d %d %s\n", r, r ? errno : 0, r ? strerror(errno) : "ok");
    printf("PORT %d\n", fd);
    fflush(stdout);
    sleep(3);
    return 0;
}
"#;

/// `bind(127.0.0.1:0)` succeeds, then `listen()` must still be refused. The
/// first half proves the bind reached the kernel and claimed a port; the
/// second proves the refusal is not limited to an unbound socket.
const BOUND_LISTEN: &str = r#"
#include <arpa/inet.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
int main(void) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) { printf("socket -1 %d\n", errno); return 1; }
    struct sockaddr_in a = {0};
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    a.sin_port = 0;
    errno = 0;
    int b = bind(fd, (struct sockaddr *)&a, sizeof a);
    printf("bind %d %d %s\n", b, b ? errno : 0, b ? strerror(errno) : "ok");
    socklen_t n = sizeof a;
    if (getsockname(fd, (struct sockaddr *)&a, &n) == 0) {
        printf("BOUND %d\n", ntohs(a.sin_port));
    }
    errno = 0;
    int r = listen(fd, 8);
    printf("listen %d %d %s\n", r, r ? errno : 0, r ? strerror(errno) : "ok");
    printf("PORT %d\n", fd);
    fflush(stdout);
    sleep(3);
    return 0;
}
"#;

/// The same bare `listen()` on an IPv6 socket. Its LISTEN row lands in
/// `/proc/net/tcp6`, which the IPv4 check does not read.
const BARE_LISTEN6: &str = r#"
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
int main(void) {
    int fd = socket(AF_INET6, SOCK_STREAM, 0);
    if (fd < 0) { printf("socket -1 %d\n", errno); return 1; }
    errno = 0;
    int r = listen(fd, 8);
    printf("listen %d %d %s\n", r, r ? errno : 0, r ? strerror(errno) : "ok");
    printf("PORT %d\n", fd);
    fflush(stdout);
    sleep(3);
    return 0;
}
"#;

/// The LISTEN rows of one `/proc/net/tcp`-shaped file: the inode and the whole
/// line, so a row that appears during the run can be traced to its process.
fn listening_rows(path: &str) -> Vec<(u64, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .skip(1)
        .filter(|line| line.split_whitespace().nth(3) == Some("0A"))
        .filter_map(|line| {
            let inode = line.split_whitespace().nth(9)?.parse().ok()?;
            Some((inode, line.to_string()))
        })
        .collect()
}

/// Both address families, because an IPv6 listener never appears in
/// `/proc/net/tcp`.
fn listening_rows_both() -> Vec<(u64, String)> {
    let mut rows = listening_rows("/proc/net/tcp");
    rows.extend(listening_rows("/proc/net/tcp6"));
    rows
}

/// The processes below `root`, `root` included.
fn descendants(root: u32) -> Vec<u32> {
    let mut all = vec![root];
    let mut at = 0;
    while at < all.len() {
        let pid = all[at];
        at += 1;
        let tasks = match std::fs::read_dir(format!("/proc/{pid}/task")) {
            Ok(tasks) => tasks,
            Err(_) => continue,
        };
        for task in tasks.flatten() {
            let text = std::fs::read_to_string(task.path().join("children")).unwrap_or_default();
            for child in text.split_whitespace().filter_map(|p| p.parse().ok()) {
                if !all.contains(&child) {
                    all.push(child);
                }
            }
        }
    }
    all
}

/// The socket inodes held by the process whose `argv[0]` is `program`.
fn program_sockets(root: u32, program: &str) -> BTreeSet<u64> {
    let mut inodes = BTreeSet::new();
    for pid in descendants(root) {
        let raw = match std::fs::read(format!("/proc/{pid}/cmdline")) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let argv0 = raw.split(|b| *b == 0).next().unwrap_or_default();
        if String::from_utf8_lossy(argv0) != program {
            continue;
        }
        let fds = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
            Ok(fds) => fds,
            Err(_) => continue,
        };
        for fd in fds.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path()) {
                if let Some(n) = target
                    .to_str()
                    .and_then(|t| t.strip_prefix("socket:["))
                    .and_then(|t| t.strip_suffix(']'))
                    .and_then(|t| t.parse().ok())
                {
                    inodes.insert(n);
                }
            }
        }
    }
    inodes
}

/// Run `source` under the pod with the read-only memory path and check both
/// halves. `expect_bind` is the `bind` line the program must print, when the
/// program binds first.
fn refuses_listen(stem: &str, source: &str, expect_bind: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let proxy = dir.path().join("socks.sock");
    // The program makes no connection, so nothing accepts on this socket.
    let _listener = UnixListener::bind(&proxy).unwrap();
    let program = build_program(dir.path(), stem, source, false);

    let before: BTreeSet<u64> = listening_rows_both().into_iter().map(|(i, _)| i).collect();
    let mut child = pod_command()
        .env("PODNET_NETNS_TEST_MEM", "none")
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", proxy.display()))
        .arg("--")
        .arg(&program)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let mut said = Vec::new();
    let mut during = Vec::new();
    for line in lines.by_ref() {
        let line = line.unwrap();
        if line.starts_with("PORT ") {
            // The program is inside its pause and the listen has been
            // answered, so a host listener would be visible now.
            during = listening_rows_both();
        }
        said.push(line);
        if said.iter().any(|l| l.starts_with("listen ")) && !during.is_empty() {
            break;
        }
    }
    let program_name = program.to_string_lossy().into_owned();
    let sockets = program_sockets(child.id(), &program_name);
    let status = child.wait().unwrap();
    let out = said.join("\n");

    if let Some(want) = expect_bind {
        assert!(
            out.contains(want),
            "{stem}: the bind must reach the kernel and claim a port, got:\n{out}"
        );
    }
    assert!(
        out.contains("listen -1 1 "),
        "{stem}: listen on a kernel socket must get EPERM, got:\n{out}"
    );
    assert!(
        !sockets.is_empty(),
        "{stem}: the program held no socket inode that /proc could show, so this check would say nothing"
    );
    let leaked: Vec<&str> = during
        .iter()
        .filter(|(inode, _)| sockets.contains(inode) && !before.contains(inode))
        .map(|(_, line)| line.as_str())
        .collect();
    assert!(
        leaked.is_empty(),
        "{stem}: a listening socket of the program appeared on the host:\n{leaked:#?}"
    );
    assert!(status.success(), "{stem}: the program failed:\n{out}");
}

#[test]
fn seccomp_kernel_socket_refuses_listen() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    refuses_listen("bare_listen", BARE_LISTEN, None);
}

#[test]
fn seccomp_kernel_socket_refuses_listen_after_bind() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    refuses_listen("bound_listen", BOUND_LISTEN, Some("bind 0 0 ok"));
}

#[test]
fn seccomp_kernel_socket_refuses_listen_v6() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    refuses_listen("bare_listen6", BARE_LISTEN6, None);
}
