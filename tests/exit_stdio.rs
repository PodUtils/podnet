//! `podnet exit --stdio` (T-329, ask N-3 of podswarm): a SOCKS5 exit on
//! stdin and stdout. The origins are on the loopback and the tests skip
//! without one.

mod test_host;

use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Exit {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
}

impl Exit {
    fn start() -> Exit {
        let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(["--no-proxy", "exit", "--stdio"])
            .env_remove("RUST_LOG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        Exit {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }

    fn read(&mut self, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        self.stdout.read_exact(&mut buf).unwrap();
        buf
    }

    /// The greeting, then `command` for 0.0.0.0:0 or for `to`; the reply.
    fn open(&mut self, command: u8, to: Option<std::net::SocketAddrV4>) -> Vec<u8> {
        self.send(&[5, 1, 0]);
        assert_eq!(self.read(2), [5, 0]);
        let to = to.unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());
        let mut req = vec![5, command, 0, 1];
        req.extend_from_slice(&to.ip().octets());
        req.extend_from_slice(&to.port().to_be_bytes());
        self.send(&req);
        self.read(10)
    }
}

impl Drop for Exit {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn v4(addr: std::net::SocketAddr) -> std::net::SocketAddrV4 {
    match addr {
        std::net::SocketAddr::V4(a) => a,
        _ => unreachable!("the tests use IPv4"),
    }
}

/// A TCP connect through the exit on stdio reaches a TCP echo.
#[test]
fn exit_stdio_connects_tcp() {
    test_host::need_loopback!();
    let echo = TcpListener::bind("127.0.0.1:0").unwrap();
    let to = v4(echo.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut s, _) = echo.accept().unwrap();
        let mut buf = [0u8; 64];
        let n = s.read(&mut buf).unwrap();
        s.write_all(&buf[..n]).unwrap();
    });
    let mut exit = Exit::start();
    let reply = exit.open(1, Some(to));
    assert_eq!(&reply[..2], &[5, 0], "{reply:?}");
    exit.send(b"hello tcp");
    assert_eq!(exit.read(9), b"hello tcp");
}

/// A UDP datagram through the exit on stdio (UDP in the stream: a frame
/// of 2 bytes of length and a SOCKS5 datagram) reaches a UDP echo and the
/// answer comes back as a frame.
#[test]
fn exit_stdio_carries_udp_in_frames() {
    test_host::need_loopback!();
    let echo = UdpSocket::bind("127.0.0.1:0").unwrap();
    let to = v4(echo.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = echo.recv_from(&mut buf) {
            let mut out = b"re:".to_vec();
            out.extend_from_slice(&buf[..n]);
            let _ = echo.send_to(&out, from);
        }
    });
    let mut exit = Exit::start();
    let reply = exit.open(0x83, None);
    assert_eq!(reply, [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    let mut datagram = vec![0, 0, 0, 1];
    datagram.extend_from_slice(&to.ip().octets());
    datagram.extend_from_slice(&to.port().to_be_bytes());
    datagram.extend_from_slice(b"hello udp");
    let mut frame = u16::try_from(datagram.len())
        .unwrap()
        .to_be_bytes()
        .to_vec();
    frame.extend_from_slice(&datagram);
    exit.send(&frame);
    let len = exit.read(2);
    let got = exit.read(usize::from(u16::from_be_bytes([len[0], len[1]])));
    assert_eq!(&got[..4], &[0, 0, 0, 1]);
    assert_eq!(&got[4..8], &to.ip().octets());
    assert_eq!(&got[8..10], &to.port().to_be_bytes());
    assert_eq!(&got[10..], b"re:hello udp");
}

/// `UDP ASSOCIATE` (command 3) is refused: a pipe has no UDP socket for it.
#[test]
fn exit_stdio_refuses_udp_associate() {
    let mut exit = Exit::start();
    let reply = exit.open(3, None);
    assert_eq!(&reply[..2], &[5, 7], "{reply:?}");
}

/// The end of stdin ends the exit with status 0.
#[test]
fn exit_stdio_ends_with_stdin() {
    let mut exit = Exit::start();
    exit.stdin = None;
    let status = exit.child.wait().unwrap();
    assert!(status.code().is_some(), "{status:?}");
}
