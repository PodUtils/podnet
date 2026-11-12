//! `podnet pod --serve --early-reply` (T-237): the process answers a SOCKS5
//! CONNECT before the dial. An upstream that refuses then shows as a closed
//! connection, and the flow log names the target and the code.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod test_host;

fn pod_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.arg("pod");
    command
}

/// A SOCKS5 upstream that refuses each request with code 5.
fn refusing_upstream() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            std::thread::spawn(move || {
                let mut head = [0u8; 3];
                stream.read_exact(&mut head).unwrap();
                stream.write_all(&[5, 0]).unwrap();
                let mut request = [0u8; 4];
                stream.read_exact(&mut request).unwrap();
                let mut len = [0u8; 1];
                stream.read_exact(&mut len).unwrap();
                let mut rest = vec![0u8; len[0] as usize + 2];
                stream.read_exact(&mut rest).unwrap();
                stream.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            });
        }
    });
    port
}

/// Start the door and return it with its address.
fn serve(args: &[&str], upstream: u16) -> (std::process::Child, String) {
    let mut child = pod_command()
        .args(["--serve", "tcp://127.0.0.1:0", "-x"])
        .arg(format!("socks5://127.0.0.1:{upstream}"))
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stderr.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    // `podnet: serving tcp://127.0.0.1:PORT (1 upstream)`
    let addr = line
        .split("tcp://")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no address in {line:?}"))
        .to_string();
    (child, addr)
}

/// Ask for `refused.example:443`; return the reply code and the bytes that
/// follow it.
fn ask(addr: &str) -> (u8, Vec<u8>) {
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    client.write_all(&[5, 1, 0]).unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).unwrap();
    let host = "refused.example";
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&443u16.to_be_bytes());
    client.write_all(&request).unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).unwrap();
    let mut rest = Vec::new();
    let _ = client.read_to_end(&mut rest);
    (reply[1], rest)
}

fn refused_line(log: &std::path::Path) -> serde_json::Value {
    let start = Instant::now();
    loop {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        let found = text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|line| line["event"] == "refused");
        if let Some(line) = found {
            return line;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no line: {text}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn early_reply_logs_the_refusal_that_the_client_cannot_see() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("flows.jsonl");
    let log_arg = log.display().to_string();
    let (mut child, addr) = serve(
        &["--early-reply", "--flow-log", &log_arg],
        refusing_upstream(),
    );
    let (code, rest) = ask(&addr);
    let line = refused_line(&log);
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(code, 0, "the early reply says yes");
    assert!(rest.is_empty(), "the client sees a closed connection");
    assert_eq!(line["target"], "refused.example:443");
    assert_eq!(line["code"], 5);
}

/// With no option, the client gets an error code.
#[test]
fn without_early_reply_the_client_gets_an_error_code() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("flows.jsonl");
    let log_arg = log.display().to_string();
    let (mut child, addr) = serve(&["--flow-log", &log_arg], refusing_upstream());
    let (code, _) = ask(&addr);
    let line = refused_line(&log);
    let _ = child.kill();
    let _ = child.wait();
    assert_ne!(code, 0);
    assert_eq!(line["code"], 5);
}

#[test]
fn early_reply_needs_serve() {
    let out = pod_command()
        .args(["--early-reply", "-x", "socks5://127.0.0.1:1", "--", "true"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--early-reply works with --serve"));
}
