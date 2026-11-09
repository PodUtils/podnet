//! T-142: the close of a UDP socket stops its relay, and the supervisor
//! frees its descriptors.

use super::{expect_lines, Form, Proxy, C_HEAD};

/// 100 sockets each send one datagram, get the echo, and close. The count
/// of the descriptors of the supervisor goes back to the count before.
const CLOSE: &str = r#"
int main(void) {
    alarm(30);
    int before = supervisor_fds();
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    for (int i = 0; i < 100; i++) {
        int fd = socket(AF_INET, SOCK_DGRAM, 0);
        if (fd < 0) { perror("socket"); return 1; }
        if (sendto(fd, "PING", 4, 0, (struct sockaddr *)&to, sizeof to) != 4) {
            perror("sendto");
            return 1;
        }
        char b[64];
        if (wait_datagram(fd, b, sizeof b) != 9) { printf("no echo %d\n", i); return 1; }
        close(fd);
    }
    int after = -1;
    for (int t = 0; t < 50; t++) {
        after = supervisor_fds();
        if (after <= before + 2) break;
        usleep(100000);
    }
    printf("before %d after %d\n", before, after);
    printf("freed %s\n", before > 0 && after <= before + 2 ? "yes" : "no");
    return 0;
}
"#;

#[test]
fn seccomp_udp_close_frees_the_relay_descriptors() {
    let proxy = Proxy::start(Form::Associate);
    let Some(out) = proxy.run(&format!("{C_HEAD}{CLOSE}"), &[]) else {
        return;
    };
    expect_lines(&out, &["freed yes"]);
}

/// A child process sends on 100 sockets and exits with no `close`. New
/// sockets of the parent then start a sweep of the table, and the sweep
/// stops the relays of the child.
const EXIT: &str = r#"
int main(void) {
    alarm(30);
    int before = supervisor_fds();
    pid_t child = fork();
    if (child == 0) {
        struct sockaddr_in to = addr4("192.0.2.10", 7777);
        for (int i = 0; i < 100; i++) {
            int fd = socket(AF_INET, SOCK_DGRAM, 0);
            char b[64];
            if (fd < 0 || sendto(fd, "PING", 4, 0, (struct sockaddr *)&to, sizeof to) != 4
                || wait_datagram(fd, b, sizeof b) != 9)
                _exit(1);
        }
        _exit(0);
    }
    int status = 0;
    waitpid(child, &status, 0);
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) { printf("child failed\n"); return 1; }
    int fds[150];
    for (int i = 0; i < 150; i++) fds[i] = socket(AF_INET, SOCK_DGRAM, 0);
    for (int i = 0; i < 150; i++) close(fds[i]);
    int after = -1;
    for (int t = 0; t < 50; t++) {
        after = supervisor_fds();
        if (after <= before + 2) break;
        usleep(100000);
    }
    printf("before %d after %d\n", before, after);
    printf("freed %s\n", before > 0 && after <= before + 2 ? "yes" : "no");
    return 0;
}
"#;

#[test]
fn seccomp_udp_exit_frees_the_relay_descriptors() {
    let proxy = Proxy::start(Form::Associate);
    let Some(out) = proxy.run(&format!("{C_HEAD}{EXIT}"), &[]) else {
        return;
    };
    expect_lines(&out, &["freed yes"]);
}

/// UDP in the stream: the close of the socket ends the control
/// connection. The program prints the count of live associations of the
/// stand-in, which it reads from the file of argv[1].
const STREAM: &str = r#"
static int live(const char *path) {
    FILE *f = fopen(path, "r");
    if (!f) return -1;
    int n = -1;
    if (fscanf(f, "%d", &n) != 1) n = -1;
    fclose(f);
    return n;
}

int main(int argc, char **argv) {
    alarm(30);
    if (argc < 2) return 2;
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (sendto(fd, "PING", 4, 0, (struct sockaddr *)&to, sizeof to) != 4) { perror("sendto"); return 1; }
    char b[64];
    if (wait_datagram(fd, b, sizeof b) != 9) { printf("no echo\n"); return 1; }
    int n = -1;
    for (int t = 0; t < 50 && n != 1; t++) {
        n = live(argv[1]);
        if (n != 1) usleep(100000);
    }
    printf("open %d\n", n);
    close(fd);
    for (int t = 0; t < 50; t++) {
        n = live(argv[1]);
        if (n == 0) break;
        usleep(100000);
    }
    printf("closed %d\n", n);
    return 0;
}
"#;

#[test]
fn seccomp_udp_stream_control_ends_on_close() {
    let proxy = Proxy::start(Form::Stream);
    // A thread writes the count of live associations into a file that the
    // program reads.
    let file = proxy.dir.path().join("live");
    let (count, path) = (proxy.live.clone(), file.clone());
    std::thread::spawn(move || loop {
        let n = count.load(std::sync::atomic::Ordering::SeqCst);
        let _ = std::fs::write(path.with_extension("tmp"), n.to_string());
        let _ = std::fs::rename(path.with_extension("tmp"), &path);
        std::thread::sleep(std::time::Duration::from_millis(20));
    });
    let Some(out) = proxy.run(&format!("{C_HEAD}{STREAM}"), &[file.to_str().unwrap()]) else {
        return;
    };
    expect_lines(&out, &["open 1", "closed 0"]);
}
