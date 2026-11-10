//! The harness that the `LD_PRELOAD` shim tests share: find the C compiler,
//! build a C program, and run a server and a client as a pair.

#![allow(dead_code)]

use std::process::{Command, Stdio};
use std::time::Duration;

use podnet::vnet::shim;

pub fn have_compiler() -> bool {
    shim::compiler().is_ok()
}

/// Build the shim and a program into a scratch directory.
pub fn build_program(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let source_path = dir.join(format!("{name}.c"));
    std::fs::write(&source_path, source).unwrap();
    let output = dir.join(name);
    let cc = shim::compiler().unwrap();
    let status = Command::new(cc)
        .arg("-O2")
        .arg("-o")
        .arg(&output)
        .arg(&source_path)
        .status()
        .unwrap();
    assert!(status.success(), "compiling {name} failed");
    output
}

/// Run `server` until it prints `ready`, then `client`, then wait for the
/// server. Each step has a time limit of 30 s: a lost datagram or a missed
/// connection fails the test, and does not hang it.
pub fn run_pair(
    server: &mut Command,
    client: &mut Command,
) -> (std::process::Output, std::process::Output) {
    use std::io::{BufRead, Read};
    let limit = Duration::from_secs(30);
    let mut child = server
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    loop {
        match rx.recv_timeout(limit) {
            Ok(line) if line == "ready" => break,
            Ok(line) => lines.push(line),
            Err(_) => {
                let _ = child.kill();
                panic!("the server did not print ready in 30 s: {lines:?}");
            }
        }
    }
    let wait = |child: &mut std::process::Child, what: &str| {
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            if start.elapsed() > limit {
                let _ = child.kill();
                panic!("the {what} did not end in 30 s");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let mut client_child = client
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let client_status = wait(&mut client_child, "client");
    let mut client_stdout = Vec::new();
    let mut client_stderr = Vec::new();
    client_child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut client_stdout)
        .unwrap();
    client_child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut client_stderr)
        .unwrap();
    let server_status = wait(&mut child, "server");
    reader.join().unwrap();
    lines.extend(rx.try_iter());
    let mut server_stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut server_stderr)
        .unwrap();
    let mut server_stdout = lines.join("\n").into_bytes();
    server_stdout.push(b'\n');
    (
        std::process::Output {
            status: server_status,
            stdout: server_stdout,
            stderr: server_stderr,
        },
        std::process::Output {
            status: client_status,
            stdout: client_stdout,
            stderr: client_stderr,
        },
    )
}

/// Compile `source` into `dir/name` and give the path. It gives `None` when
/// the compiler is not found or the compile fails, and prints nothing.
pub fn compile_quiet(
    dir: &std::path::Path,
    name: &str,
    source: &str,
) -> Option<std::path::PathBuf> {
    let cc = shim::compiler().ok()?;
    let path = dir.join(name);
    let src_path = dir.join(format!("{name}.c"));
    std::fs::write(&src_path, source).ok()?;
    let status = Command::new(&cc)
        .arg("-O2")
        .arg("-o")
        .arg(&path)
        .arg(&src_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    status.success().then_some(path)
}

/// The compiler, or skip with a printed reason.
pub fn compiler_or_skip(test: &str) -> Option<String> {
    match shim::compiler() {
        Ok(cc) => Some(cc),
        Err(_) => {
            eprintln!("{test}: skipping, no C compiler");
            None
        }
    }
}
