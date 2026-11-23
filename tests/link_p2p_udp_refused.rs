//! GitHub #485: a host that refuses the UDP socket makes `podnet link p2p`
//! print the named skip with the host's own error, end to end.
//!
//! The host of this test permits UDP, so the refusal is made real for the
//! child: a seccomp filter that answers `EPERM` to `socket(AF_INET,
//! SOCK_DGRAM)`, before `exec`. The filter is installed in `pre_exec`, so
//! podnet itself meets the kernel's refusal in its first UDP allocation and
//! nothing of the test stands in for the host.
//!
//! A network namespace cannot express this refusal: `net.ipv4.udp` is not a
//! setting, and an `iptables -A OUTPUT -p udp -j DROP` drops packets after
//! the socket is made, so the client would time out rather than hear the
//! host refuse. The socket is the layer that the skip names, and seccomp is
//! the kernel's own way to refuse it.

use std::io;
use std::process::{Command, Stdio};

/// The kernel refuses each UDP socket of the child with `EPERM`. A TCP
/// socket, and every other system call, is allowed.
fn refuse_udp_sockets() -> io::Result<()> {
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_ERRNO: u32 = 0x0005_0000;
    let stmt = |code: u32, k: u32| libc::sock_filter {
        code: code as u16,
        jt: 0,
        jf: 0,
        k,
    };
    let jeq = |k: u32, jt: u8, jf: u8| libc::sock_filter {
        code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
        jt,
        jf,
        k,
    };
    let load = |offset: u32| stmt(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, offset);
    // seccomp_data: nr at 0, args[0] at 16, args[1] at 24 (the low 32
    // bits on a little-endian machine).
    let filter = [
        load(0),
        jeq(libc::SYS_socket as u32, 0, 6),
        load(16),
        jeq(libc::AF_INET as u32, 1, 0),
        jeq(libc::AF_INET6 as u32, 0, 3),
        load(24),
        stmt(libc::BPF_ALU | libc::BPF_AND | libc::BPF_K, 0xf),
        jeq(libc::SOCK_DGRAM as u32, 1, 0),
        stmt(libc::BPF_RET | libc::BPF_K, RET_ALLOW),
        stmt(libc::BPF_RET | libc::BPF_K, RET_ERRNO | libc::EPERM as u32),
    ];
    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut libc::sock_filter,
    };
    // SAFETY: prctl with valid arguments; `prog` and `filter` live for the
    // calls.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &prog as *const libc::sock_fprog,
            ) != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// The command of the test: the direct peer is refused at once, so the relay
/// is the one that meets the host's refusal.
fn command() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args([
        "link",
        "p2p",
        "--peer",
        "127.0.0.1:1",
        "--relay",
        "turn://127.0.0.1:3478",
        "--check",
        "--timeout",
        "1000",
    ]);
    for name in [
        "PODNET_TURN_USER",
        "PODNET_TURN_PASSWORD",
        "PODNET_TURN_PASSWORD_FILE",
        "PODNET_TURN_REALM",
        "PODNET_TURN_LIFETIME",
        "PODNET_TURN_RATE_LIMIT",
        "PODNET_PROXY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NODE_USE_ENV_PROXY",
        "RUST_LOG",
    ] {
        cmd.env_remove(name);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// The command under the filter refuses the relay with the named skip and
/// the kernel's words, and exits 69.
#[test]
fn a_host_that_refuses_udp_is_a_named_skip_with_the_host_error() {
    let dir = tempfile::tempdir().expect("a directory of the test");
    let password = dir.path().join("turn-password");
    std::fs::write(&password, "made-up-secret\n").expect("the password file");
    let mut cmd = command();
    cmd.env("PODNET_TURN_USER", "turn-user")
        .env("PODNET_TURN_PASSWORD_FILE", &password);
    // SAFETY: the closure installs a seccomp filter and returns; it calls
    // no allocator and no lock after the filter is in place.
    unsafe {
        use std::os::unix::process::CommandExt as _;
        cmd.pre_exec(refuse_udp_sockets);
    }
    let out = cmd.output().expect("podnet runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    println!("{text}");
    assert_eq!(out.status.code(), Some(69), "{text}");
    assert!(
        text.contains("path relay skipped: the host refuses UDP:"),
        "{text}"
    );
    assert!(text.contains("no UDP socket for TURN"), "{text}");
    assert!(text.contains("Operation not permitted"), "{text}");
    // The direct path was refused and the skip is the relay's: the line
    // names the relay, not a made-up failure of the peer.
    assert!(text.contains("turn://127.0.0.1:3478"), "{text}");
}
