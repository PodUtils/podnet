//! The external measurement of podnet#500, STAT-1 to STAT-3.
//!
//! The peer timestamps each datagram when it arrives, and returns its time.
//! Throughput and loss come from the peer's timestamps; latency comes from
//! the round trip of this process minus the time that the peer spent on the
//! packet. The sender's own counter is measured too, only to compare: a
//! disagreement is reported as a disagreement, with the numbers, and is
//! never averaged into one value.
//!
//! The output is a capture file: one line for each packet, then the summary.
//! A regression is a diff of two files.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant};

/// The first line of a capture file.
pub const FORMAT: &str = "pod-stat/1";
/// The version word of the request and of the reply.
const MAGIC: &str = "pod-stat/1";
/// The largest payload of one probe.
pub const MAX_SIZE: usize = 4096;
/// The time limit of one reply, when the argument gives none.
pub const DEFAULT_TIMEOUT_MS: u64 = 500;

/// One packet, as the two ends saw it. A time in microseconds, from the
/// start of its own process.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub seq: u32,
    pub sent_us: u64,
    pub recv_us: Option<u64>,
    pub peer_recv_us: Option<u64>,
    pub peer_proc_us: Option<u64>,
    pub bytes: u32,
}

/// The two measurements of one run, and their comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub sent: u32,
    pub received: u32,
    pub loss: f64,
    /// Bits per second from the first to the last timestamp of the peer.
    pub peer_bps: Option<f64>,
    /// Bits per second from the first to the last send of this process.
    pub sender_bps: Option<f64>,
    /// The median one-way delay in microseconds, with the peer's work
    /// removed.
    pub latency_p50_us: Option<u64>,
    /// The disagreement in percent of the larger rate, when the two rates
    /// differ by more than [`DISAGREE_PERCENT`].
    pub disagreement: Option<String>,
}

/// The relative difference that is a disagreement.
pub const DISAGREE_PERCENT: f64 = 5.0;

/// A run of one peer, and the file that it writes.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub peer: String,
    pub rate_bps: u64,
    pub size: u32,
    pub samples: Vec<Sample>,
    pub summary: Summary,
    pub external: Option<String>,
}

impl Capture {
    /// The capture file: a header, one line for each sample, then the
    /// summary.
    pub fn text(&self) -> String {
        let mut out = String::new();
        out.push_str(FORMAT);
        out.push('\n');
        out.push_str(&format!("peer {}\n", self.peer));
        out.push_str(&format!("rate_bps {}\n", self.rate_bps));
        out.push_str(&format!("size {}\n", self.size));
        out.push_str("seq sent_us recv_us peer_recv_us peer_proc_us bytes\n");
        for sample in &self.samples {
            out.push_str(&format!(
                "{} {} {} {} {} {}\n",
                sample.seq,
                sample.sent_us,
                number(sample.recv_us),
                number(sample.peer_recv_us),
                number(sample.peer_proc_us),
                sample.bytes
            ));
        }
        out.push_str("summary\n");
        out.push_str(&format!("sent {}\n", self.summary.sent));
        out.push_str(&format!("received {}\n", self.summary.received));
        out.push_str(&format!("loss {:.4}\n", self.summary.loss));
        out.push_str(&format!("peer_bps {}\n", float(self.summary.peer_bps)));
        out.push_str(&format!("sender_bps {}\n", float(self.summary.sender_bps)));
        out.push_str(&format!(
            "latency_p50_us {}\n",
            number(self.summary.latency_p50_us)
        ));
        match &self.summary.disagreement {
            Some(why) => out.push_str(&format!("disagreement {why}\n")),
            None => out.push_str("disagreement none\n"),
        }
        if let Some(text) = &self.external {
            out.push_str(&format!("external {text}\n"));
        }
        out
    }

    /// The JSON document of the run.
    pub fn json(&self) -> String {
        let samples: Vec<serde_json::Value> = self
            .samples
            .iter()
            .map(|s| {
                serde_json::json!({
                    "seq": s.seq,
                    "sent_us": s.sent_us,
                    "recv_us": s.recv_us,
                    "peer_recv_us": s.peer_recv_us,
                    "peer_proc_us": s.peer_proc_us,
                    "bytes": s.bytes,
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "podnet.stat/1",
            "peer": self.peer,
            "rate_bps": self.rate_bps,
            "size": self.size,
            "samples": samples,
            "summary": {
                "sent": self.summary.sent,
                "received": self.summary.received,
                "loss": self.summary.loss,
                "peer_bps": self.summary.peer_bps,
                "sender_bps": self.summary.sender_bps,
                "latency_p50_us": self.summary.latency_p50_us,
                "disagreement": self.summary.disagreement,
            },
            "external": self.external,
        }))
        .unwrap_or_else(|_| "{}".into())
    }

    /// Write the capture file. A new file is written beside it and then
    /// renamed, so a reader never sees half a line.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let text = self.text();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        {
            let mut file = std::fs::File::create(&tmp)
                .map_err(|e| anyhow::anyhow!("{}: {e}", tmp.display()))?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Ok(())
    }
}

/// The argument of one run.
#[derive(Clone, Debug)]
pub struct Options {
    pub peer: String,
    pub count: u32,
    pub size: usize,
    pub rate_bps: u64,
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            peer: String::new(),
            count: 10,
            size: 64,
            rate_bps: 1_000_000,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }
}

/// Measure `options.peer`. Each packet is paced by `rate_bps`, and the peer
/// times its arrival.
pub fn run(options: &Options) -> anyhow::Result<Capture> {
    if options.count == 0 {
        anyhow::bail!("the count is 1 or more");
    }
    if options.size == 0 || options.size > MAX_SIZE {
        anyhow::bail!("the size is 1 to {MAX_SIZE} bytes");
    }
    if options.rate_bps == 0 {
        anyhow::bail!("the rate is 1 bit/s or more");
    }
    let addr = resolve(&options.peer)?;
    // EGRESS-3 and EGRESS-4: the peer is a destination of the policy,
    // checked before the datagram socket is bound.
    crate::policy::permit_fault(&addr.ip().to_string(), addr.port())?;
    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .map_err(|e| anyhow::anyhow!("the local datagram socket: {e}"))?;
    socket
        .set_read_timeout(Some(options.timeout))
        .map_err(|e| anyhow::anyhow!("the read limit: {e}"))?;
    socket
        .connect(addr)
        .map_err(|e| anyhow::anyhow!("{}: {e}", addr))?;
    let start = Instant::now();
    let mut samples = Vec::with_capacity(options.count as usize);
    let gap = Duration::from_secs_f64(options.size as f64 * 8.0 / options.rate_bps as f64);
    let mut pending = Duration::ZERO;
    let mut buf = vec![0u8; MAX_SIZE + 128];
    for seq in 0..options.count {
        if pending > Duration::ZERO {
            std::thread::sleep(pending);
        }
        let sent_us = start.elapsed().as_micros() as u64;
        let request = format!(
            "{MAGIC} P {seq} {sent_us} {} {}\n",
            options.size,
            "x".repeat(options.size)
        );
        socket
            .send(request.as_bytes())
            .map_err(|e| anyhow::anyhow!("{}: {e}", addr))?;
        let reply = match socket.recv(&mut buf) {
            Ok(n) => String::from_utf8_lossy(&buf[..n]).into_owned(),
            // A connected datagram socket reports the ICMP port-unreachable
            // of a dead peer as `ConnectionRefused`. That is a lost reply,
            // not an error of this run.
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                samples.push(Sample {
                    seq,
                    sent_us,
                    recv_us: None,
                    peer_recv_us: None,
                    peer_proc_us: None,
                    bytes: options.size as u32,
                });
                pending = gap;
                continue;
            }
            Err(e) => return Err(anyhow::anyhow!("{}: {e}", addr)),
        };
        let recv_us = start.elapsed().as_micros() as u64;
        pending = gap.saturating_sub(Duration::from_micros(recv_us - sent_us));
        match parse_reply(&reply, seq) {
            Some((peer_recv_us, peer_proc_us)) => samples.push(Sample {
                seq,
                sent_us,
                recv_us: Some(recv_us),
                peer_recv_us: Some(peer_recv_us),
                peer_proc_us: Some(peer_proc_us),
                bytes: options.size as u32,
            }),
            None => samples.push(Sample {
                seq,
                sent_us,
                recv_us: Some(recv_us),
                peer_recv_us: None,
                peer_proc_us: None,
                bytes: options.size as u32,
            }),
        }
    }
    let summary = summarise(&samples, options.size as u32);
    Ok(Capture {
        peer: addr.to_string(),
        rate_bps: options.rate_bps,
        size: options.size as u32,
        samples,
        summary,
        external: None,
    })
}

fn resolve(peer: &str) -> anyhow::Result<SocketAddr> {
    let mut addrs = peer
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("{peer}: {e}"))?;
    addrs
        .next()
        .ok_or_else(|| anyhow::anyhow!("{peer}: no address"))
}

fn parse_reply(text: &str, seq: u32) -> Option<(u64, u64)> {
    let line = text.lines().next()?;
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() < 5 || words[0] != MAGIC || words[1] != "R" {
        return None;
    }
    if words[2].parse::<u32>().ok()? != seq {
        return None;
    }
    Some((words[3].parse().ok()?, words[4].parse().ok()?))
}

/// The two measurements from the samples.
pub fn summarise(samples: &[Sample], size: u32) -> Summary {
    let sent = samples.len() as u32;
    let received: Vec<&Sample> = samples
        .iter()
        .filter(|s| s.recv_us.is_some() && s.peer_recv_us.is_some())
        .collect();
    let loss = if sent == 0 {
        0.0
    } else {
        (sent as f64 - received.len() as f64) / sent as f64
    };
    let peer_bps = rate(
        received.iter().filter_map(|s| s.peer_recv_us).min(),
        received.iter().filter_map(|s| s.peer_recv_us).max(),
        received.len(),
        size,
    );
    let sender_bps = rate(
        received.iter().map(|s| s.sent_us).min(),
        received.iter().map(|s| s.sent_us).max(),
        received.len(),
        size,
    );
    // The two clocks are not comparable, so the delay is the round trip of
    // this process with the time that the peer reported for its own work
    // removed. A one-way delay needs one clock, and the harness does not
    // assume one.
    let mut latency: Vec<u64> = received
        .iter()
        .filter_map(|s| {
            let recv = s.recv_us?;
            let proc = s.peer_proc_us?;
            Some(recv.saturating_sub(s.sent_us).saturating_sub(proc))
        })
        .collect();
    latency.sort_unstable();
    let latency_p50_us = latency.get(latency.len() / 2).copied();
    let disagreement = disagreement_of(peer_bps, sender_bps);
    Summary {
        sent,
        received: received.len() as u32,
        loss,
        peer_bps,
        sender_bps,
        latency_p50_us,
        disagreement,
    }
}

fn rate(first: Option<u64>, last: Option<u64>, packets: usize, size: u32) -> Option<f64> {
    let (first, last) = (first?, last?);
    if packets < 2 || last <= first {
        return None;
    }
    // The bytes between the first arrival and the last, over that time.
    Some((packets as f64 - 1.0) * size as f64 * 8.0 / ((last - first) as f64 / 1e6))
}

/// The disagreement of the two rates. A difference of more than
/// [`DISAGREE_PERCENT`] of the larger rate is a disagreement, with the two
/// numbers and no average.
pub fn disagreement_of(peer_bps: Option<f64>, sender_bps: Option<f64>) -> Option<String> {
    let (peer, sender) = (peer_bps?, sender_bps?);
    let larger = peer.max(sender);
    if larger <= 0.0 {
        return None;
    }
    let percent = (peer - sender).abs() / larger * 100.0;
    if percent <= DISAGREE_PERCENT {
        return None;
    }
    Some(format!(
        "peer {peer:.0} bit/s against the sender {sender:.0} bit/s ({percent:.1}% of the larger): the peer's timestamps and the sender's counter disagree; neither is averaged"
    ))
}

/// Serve as the peer: timestamp each request and return its arrival and
/// processing time. Runs until the process ends.
pub fn serve(addr: &str) -> anyhow::Result<()> {
    // egress-exempt: a listener that answers the measurement peer
    let socket = UdpSocket::bind(addr).map_err(|e| anyhow::anyhow!("{addr}: {e}"))?;
    let start = Instant::now();
    let mut buf = vec![0u8; MAX_SIZE + 128];
    loop {
        let Ok((n, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        let arrived = start.elapsed().as_micros() as u64;
        let text = String::from_utf8_lossy(&buf[..n]);
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.len() < 4 || words[0] != MAGIC || words[1] != "P" {
            continue;
        }
        let Ok(seq) = words[2].parse::<u32>() else {
            continue;
        };
        let reply = format!("{MAGIC} R {seq} {arrived} 0\n");
        // egress-exempt: the answer to the client that sent the probe
        let _ = socket.send_to(reply.as_bytes(), from);
    }
}

/// Read a capture file again, for a diff or a test.
pub fn read(path: &Path) -> anyhow::Result<Capture> {
    let file = std::fs::File::open(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let mut lines = BufReader::new(file).lines();
    let first = lines.next().transpose()?.unwrap_or_default();
    if first != FORMAT {
        anyhow::bail!("{}: the first line is not {FORMAT}", path.display());
    }
    let mut peer = String::new();
    let mut rate_bps = 0;
    let mut size = 0;
    let mut samples = Vec::new();
    let mut summary = Summary {
        sent: 0,
        received: 0,
        loss: 0.0,
        peer_bps: None,
        sender_bps: None,
        latency_p50_us: None,
        disagreement: None,
    };
    for line in lines {
        let line = line?;
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["peer", value] => peer = (*value).to_string(),
            ["rate_bps", value] => rate_bps = value.parse()?,
            ["size", value] => size = value.parse()?,
            ["sent", value] => summary.sent = value.parse()?,
            ["received", value] => summary.received = value.parse()?,
            ["loss", value] => summary.loss = value.parse()?,
            ["peer_bps", value] => summary.peer_bps = value.parse().ok(),
            ["sender_bps", value] => summary.sender_bps = value.parse().ok(),
            ["latency_p50_us", value] => summary.latency_p50_us = value.parse().ok(),
            ["disagreement", "none"] => summary.disagreement = None,
            ["disagreement", ..] => summary.disagreement = Some(words[1..].join(" ")),
            [seq, sent, recv, peer_recv, proc, bytes] if seq.parse::<u32>().is_ok() => {
                samples.push(Sample {
                    seq: seq.parse()?,
                    sent_us: sent.parse()?,
                    recv_us: recv.parse().ok(),
                    peer_recv_us: peer_recv.parse().ok(),
                    peer_proc_us: proc.parse().ok(),
                    bytes: bytes.parse()?,
                });
            }
            _ => {}
        }
    }
    Ok(Capture {
        peer,
        rate_bps,
        size,
        samples,
        summary,
        external: None,
    })
}

fn number(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

fn float(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.0}"))
        .unwrap_or_else(|| "-".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// STAT-2: a peer and a sender that differ by more than 5 percent are a
    /// disagreement, not an average.
    #[test]
    fn a_disagreement_is_named() {
        assert!(disagreement_of(Some(1000.0), Some(1010.0)).is_none());
        let text = disagreement_of(Some(1000.0), Some(500.0)).unwrap();
        assert!(text.contains("peer 1000"), "{text}");
        assert!(text.contains("500"), "{text}");
        assert!(text.contains("neither is averaged"), "{text}");
    }

    /// STAT-3: a capture file reads again with the same samples.
    #[test]
    fn a_capture_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let samples = vec![Sample {
            seq: 0,
            sent_us: 10,
            recv_us: Some(20),
            peer_recv_us: Some(15),
            peer_proc_us: Some(1),
            bytes: 8,
        }];
        let capture = Capture {
            peer: "127.0.0.1:9".into(),
            rate_bps: 1000,
            size: 8,
            summary: summarise(&samples, 8),
            samples,
            external: None,
        };
        let path = dir.path().join("capture.txt");
        capture.write(&path).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.samples, capture.samples);
        assert_eq!(back.peer, capture.peer);
    }
}
