//! The stock-client gate of PodUtils/podnet#486: an independent HTTP/3
//! client completes a request against the HTTP/3 endpoint of `podnet warp
//! h3`, selected by `--h3-shape`.
//!
//! `curl` on this host has nghttp3 but no HTTP/3 protocol, so it is not a
//! stock client. The client here is quic-go's `http3.Transport`, a second
//! implementation of HTTP/3 and QPACK, run as a Go program against the
//! endpoint on the loopback.
//!
//! The test needs a Go toolchain and quic-go v0.59.0 in the Go module cache;
//! `scripts/ci/reference-tests.sh` refuses to run without both.

mod test_host;

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Stop the endpoint when the test ends, whatever the assertions did.
struct Endpoint(Child);

impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `go` on the PATH, or None.
fn go() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("go"))
        .find(|path| path.is_file())
}

/// The gate: the stock client gets `200` and the body of the shape from the
/// endpoint that `podnet warp h3` serves, and the endpoint writes the qlog
/// of the control stream and the request.
#[test]
fn a_stock_quic_go_client_completes_a_request_against_the_shape() {
    test_host::need_loopback!();
    let Some(go) = go() else {
        test_host::skip_line(
            "this host has no Go toolchain, so the stock quic-go HTTP/3 client cannot run",
        );
        return;
    };
    let dir = tempfile::tempdir().expect("a directory for the trace");
    let qlog = dir.path().join("h3.qlog");
    let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["warp", "--h3-shape", "common", "h3"])
        .arg("--listen")
        .arg("127.0.0.1:0")
        .arg("--qlog")
        .arg(&qlog)
        .args(["--run-for", "120"])
        .env_remove("PODNET_WARP_H3_SHAPE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("podnet warp h3 starts");
    let stdout = child.stdout.take().expect("the endpoint's stdout");
    let mut reader = BufReader::new(stdout);
    let mut said = String::new();
    let mut addr = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while addr.is_none() || !said.contains("congestion=") {
        if Instant::now() > deadline {
            break;
        }
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap_or(0);
        if read == 0 {
            break;
        }
        said.push_str(&line);
        if let Some(rest) = line.strip_prefix("h3 listen ") {
            addr = Some(rest.trim().to_string());
        }
    }
    let endpoint = Endpoint(child);
    let addr = match addr {
        Some(addr) => addr,
        None => panic!("the endpoint printed no address\n{said}"),
    };
    eprintln!("{said}");
    assert!(said.contains("congestion=cubic"), "{said}");
    let stock = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/quic_stock");
    let output = Command::new(go)
        .current_dir(stock)
        .arg("run")
        .arg(".")
        .arg(&addr)
        .arg("/")
        .output()
        .expect("the stock client runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("{}", stdout.trim());
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim());
    }
    assert!(
        output.status.success(),
        "the stock client exited {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );
    assert!(
        stdout.contains("client=quic-go/http3"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("version=v0.59.0"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("proto=HTTP/3.0"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("status=200"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("body=\"pod-warp h3 shape\\n\""),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    drop(endpoint);
    let events = std::fs::read_to_string(&qlog).expect("the trace of the shape");
    assert!(
        events.contains("\"name\":\"http3:control_stream\""),
        "the trace: {events}"
    );
    assert!(
        events.contains("\"name\":\"http3:request\""),
        "the trace: {events}"
    );
    assert!(events.contains("\"status\":200"), "the trace: {events}");
}
