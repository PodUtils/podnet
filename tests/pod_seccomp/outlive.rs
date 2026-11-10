//! The pod lives past its first program (T-107, T-108): a program starts a
//! daemon and exits. The pod returns the code of the program at once, and
//! the daemon keeps the network of the pod.

use super::*;

/// Start a daemon and exit 0. The daemon connects after 3 s, and writes
/// what it reads to the file of `argv[1]`.
const STARTER: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    pid_t pid = fork();
    if (pid < 0) return 1;
    if (pid > 0) return 0;
    setsid();
    int null = open("/dev/null", O_RDWR);
    dup2(null, 0); dup2(null, 1); dup2(null, 2);
    alarm(20);
    sleep(3);
    int out = open(argv[1], O_WRONLY | O_CREAT | O_TRUNC, 0600);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_port = htons(80);
    inet_pton(AF_INET, "93.184.216.34", &a.sin_addr);
    if (connect(s, (struct sockaddr *)&a, sizeof a) < 0) { dprintf(out, "CONNECT-FAILED\n"); return 1; }
    write(s, "PING\n", 5);
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    dprintf(out, "READ:%d:%s", n, b);
    return 0;
}
"#;

fn outlives_its_first_child(backend: &str) {
    let dir = std::env::temp_dir().join(format!(
        "podnet-pod-outlive-{backend}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    let program = build_program(&dir, "starter", STARTER, false);
    let result = dir.join("daemon.txt");

    let started = std::time::Instant::now();
    let output = pod_command()
        .args(["--backend", backend, "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg(&program)
        .arg(&result)
        .stdin(Stdio::null())
        .output()
        .expect("run podnet pod");
    let took = started.elapsed();
    assert!(
        output.status.success(),
        "the pod gives the code of the first program: {:?} {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        took < Duration::from_millis(2500),
        "the pod returns when the first program exits, not after the daemon: {took:?}"
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(15)).as_deref(),
        Ok("93.184.216.34:80"),
        "the daemon connects through the pod after the first program exits"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut text = String::new();
    while std::time::Instant::now() < deadline {
        text = std::fs::read_to_string(&result).unwrap_or_default();
        if text.contains("HELLO-FROM-TARGET") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        text.contains("HELLO-FROM-TARGET"),
        "the daemon read {text:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_pod_outlives_its_first_child() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    outlives_its_first_child("seccomp");
}

#[test]
fn netns_pod_outlives_its_first_child() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    outlives_its_first_child("netns");
}
