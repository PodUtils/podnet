//! STAT-1 to STAT-3: throughput, latency and loss come from a peer that
//! timestamps each arrival, and the capture file is the output.
//!
//! The peer here is a socket of this test process with its own clock. The
//! rate of that clock is compared with the `peer_bps` that the binary
//! reports, within 5 percent. A forged peer that compresses its timestamps
//! makes the two rates disagree, and the output says so.

use std::net::UdpSocket;
use std::process::Command;
use std::thread::JoinHandle;
use std::time::Instant;

const MAGIC: &str = "pod-stat/1";

/// Serve `count` probes and give the arrival times of this clock.
fn peer(count: u32, forged: bool) -> (String, JoinHandle<Vec<u64>>) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap().to_string();
    let handle = std::thread::spawn(move || {
        let start = Instant::now();
        let mut arrivals = Vec::with_capacity(count as usize);
        let mut buf = [0u8; 8192];
        for _ in 0..count {
            let (n, from) = socket.recv_from(&mut buf).unwrap();
            let arrived = start.elapsed().as_micros() as u64;
            arrivals.push(arrived);
            let text = String::from_utf8_lossy(&buf[..n]);
            let words: Vec<&str> = text.split_whitespace().collect();
            let seq: u32 = words
                .get(2)
                .and_then(|w| w.parse().ok())
                .expect("a probe with a sequence number");
            let reported = if forged { seq as u64 } else { arrived };
            let reply = format!("{MAGIC} R {seq} {reported} 0\n");
            socket.send_to(reply.as_bytes(), from).unwrap();
        }
        arrivals
    });
    (addr, handle)
}

fn rate(arrivals: &[u64], size: u32) -> f64 {
    let first = arrivals.first().copied().unwrap();
    let last = arrivals.last().copied().unwrap();
    (arrivals.len() as f64 - 1.0) * size as f64 * 8.0 / ((last - first) as f64 / 1e6)
}

fn capture(
    addr: &str,
    dir: &std::path::Path,
    count: u32,
) -> (std::process::Output, std::path::PathBuf) {
    let file = dir.join("pod-stat.txt");
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "stat",
            "--peer",
            addr,
            "--rate",
            "1000",
            "--size",
            "50",
            "--count",
            &count.to_string(),
            "--timeout-ms",
            "2000",
            "--out",
            file.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("podnet stat runs");
    (out, file)
}

/// STAT-1 and STAT-3: at 1 Kbit/s, the rate of an external clock and the
/// reported rate agree within 5 percent, and the capture file holds the
/// samples.
#[test]
fn a_throttled_loopback_agrees_with_an_external_clock() {
    let dir = tempfile::tempdir().unwrap();
    let count = 10;
    let (addr, handle) = peer(count, false);
    let (out, file) = capture(&addr, dir.path(), count);
    let arrivals = handle.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let reported = doc["summary"]["peer_bps"].as_f64().expect("the peer rate");
    let external = rate(&arrivals, 50);
    let difference = (reported - external).abs() / external;
    assert!(
        difference < 0.05,
        "reported {reported:.0} bit/s and external {external:.0} bit/s differ by {:.1} percent",
        difference * 100.0
    );
    assert_eq!(doc["summary"]["sent"], 10);
    assert_eq!(doc["summary"]["received"], 10);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("pod-stat/1\n"), "{text}");
    assert!(text.contains("sent 10\n"), "{text}");
    assert!(text.contains("received 10\n"), "{text}");
    assert!(text.contains("latency_p50_us "), "{text}");
}

/// STAT-2: a forged sender counter makes the two rates disagree, and the
/// capture says so instead of averaging them.
#[test]
fn a_forged_timestamp_is_reported_as_a_disagreement() {
    let dir = tempfile::tempdir().unwrap();
    let count = 5;
    let (addr, handle) = peer(count, true);
    let (out, file) = capture(&addr, dir.path(), count);
    handle.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("disagreement ") && !text.contains("disagreement none"),
        "the rates did not disagree: {text}"
    );
    assert!(text.contains("neither is averaged"), "{text}");
}

/// The summary of a capture names loss, so a peer that answers all probes
/// gives zero loss and one that answers none gives one.
#[test]
fn a_peer_that_answers_nothing_is_all_loss() {
    let dir = tempfile::tempdir().unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap().to_string();
    drop(socket);
    let (out, file) = capture(&addr, dir.path(), 3);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("loss 1.0000"), "{text}");
}
