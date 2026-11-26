//! The measurements of the Prove of T-244, with the harness of
//! `tests/tunnel_inspect.rs` (a local origin through `serve_decoded`, no
//! edge): the cost of the capture at 1000 requests per second, and the
//! resident set of a 50 MiB upload and download with `--inspect`.
//!
//! Both tests are `#[ignore]`: the numbers change with the load of the
//! host. Run them alone, when the load is low (`docs/development.md`,
//! "Measurements"):
//!
//! ```sh
//! cargo test --test tunnel_inspect -- --ignored --test-threads=1 --nocapture measure::
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pod_cloudflare::cloudflare::stream::{self as cqstream, ConnectRequest, HTTP_STATUS_KEY};
use pod_cloudflare::proxy::{serve_decoded, Runtime};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};

use podnet::ingress::{Ingress, Service};
use podnet::tunnel::inspect::{Inspector, Options, State};

use super::{request, tunnel, visit, Tunnel};

const MIB: u64 = 1 << 20;
/// The size of each large transfer of the Prove.
const LARGE: u64 = 50 * MIB;
/// The bound of the Prove: the resident set over the baseline.
const RSS_BOUND: u64 = 8 * MIB;
/// The part that the origin and the visitor write at one time.
const PART: usize = 64 * 1024;

/// An origin that keeps no body: it reads the head, drops the request
/// body, and answers. `/ok` and `/upload` answer `ok`; `/download` sends
/// [`LARGE`] bytes with a Content-Length; `/chunked` sends them as chunks
/// of [`PART`] bytes.
async fn quiet_origin() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            tokio::spawn(answer(s));
        }
    });
    at
}

async fn answer(s: tokio::net::TcpStream) {
    let mut s = tokio::io::BufReader::new(s);
    let mut line = String::new();
    let (mut path, mut length) = (String::new(), 0u64);
    loop {
        line.clear();
        if s.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let lower = line.trim_end().to_ascii_lowercase();
        if lower.is_empty() {
            break;
        }
        if path.is_empty() {
            path = lower.split(' ').nth(1).unwrap_or_default().to_string();
        } else if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = (&mut s).take(length);
    if tokio::io::copy(&mut body, &mut tokio::io::sink())
        .await
        .is_err()
    {
        return;
    }
    let part = vec![b'x'; PART];
    let mut s = s.into_inner();
    match path.as_str() {
        "/download" => {
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {LARGE}\r\n\r\n");
            let _ = s.write_all(head.as_bytes()).await;
            for _ in 0..LARGE / PART as u64 {
                if s.write_all(&part).await.is_err() {
                    return;
                }
            }
        }
        "/chunked" => {
            let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = s.write_all(head.as_bytes()).await;
            let size = format!("{PART:x}\r\n");
            for _ in 0..LARGE / PART as u64 {
                let sent = async {
                    s.write_all(size.as_bytes()).await?;
                    s.write_all(&part).await?;
                    s.write_all(b"\r\n").await
                };
                if sent.await.is_err() {
                    return;
                }
            }
            let _ = s.write_all(b"0\r\n\r\n").await;
        }
        _ => {
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
        }
    }
    let _ = s.shutdown().await;
}

/// A runtime of pod-cloudflare with no sink: `tunnel` with no `--inspect`.
fn plain(origin: &str) -> Tunnel {
    let ingress = Ingress::new().catch_all(Service::Http(origin.to_string()));
    let inspector = Inspector::new(Options::default(), ingress.clone());
    let host = Arc::new(podnet::tunnel::base_host());
    let metrics = podnet::metrics::Metrics::new();
    let runtime = Arc::new(Runtime::build(host.clone(), &ingress, metrics).unwrap());
    Tunnel {
        runtime,
        ingress: Arc::new(ingress),
        inspector,
        _host: host,
    }
}

/// Serve `request` with an upload of `up` bytes that the visitor writes in
/// parts while the exchange runs. Gives the status and the count of the
/// body bytes that the visitor got; it keeps no body.
async fn transfer(tunnel: &Tunnel, request: ConnectRequest, up: u64) -> (String, u64) {
    let (ours, theirs) = tokio::io::duplex(PART);
    let (mut visitor, input) = tokio::io::duplex(PART);
    let write = async move {
        let part = vec![b'u'; PART];
        let mut left = up;
        while left > 0 {
            let n = left.min(PART as u64) as usize;
            if visitor.write_all(&part[..n]).await.is_err() {
                return;
            }
            left -= n as u64;
        }
    };
    let serve = serve_decoded(
        tunnel.runtime.clone(),
        tunnel.ingress.clone(),
        request,
        Box::pin(input.compat()),
        Box::pin(ours.compat_write()),
    );
    let read = async {
        let mut reader = theirs.compat();
        let (_, meta) = cqstream::read_connect_response(&mut reader).await.unwrap();
        let status = meta
            .iter()
            .find(|(k, _)| k == HTTP_STATUS_KEY)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let mut got = 0u64;
        let mut buf = vec![0u8; PART];
        loop {
            match futures::io::AsyncReadExt::read(&mut reader, &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n as u64,
            }
        }
        (status, got)
    };
    // Boxed: the futures of a debug build are large for the stack of a
    // test thread.
    let all = Box::pin(async { tokio::join!(write, serve, read) });
    let ((), (), got) = tokio::time::timeout(Duration::from_secs(120), all)
        .await
        .expect("the transfer ends");
    got
}

/// The resident set of this process, in bytes.
fn rss() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    // SAFETY: sysconf() has no precondition.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    pages * u64::try_from(page).unwrap_or(4096)
}

/// The largest resident set while `work` runs (a sample each 2 ms).
async fn peak<F: std::future::Future>(work: F) -> (F::Output, u64) {
    let stop = Arc::new(AtomicBool::new(false));
    let top = Arc::new(AtomicU64::new(rss()));
    let sampler = {
        let (stop, top) = (stop.clone(), top.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                top.fetch_max(rss(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };
    let out = work.await;
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    top.fetch_max(rss(), Ordering::Relaxed);
    (out, top.load(Ordering::Relaxed))
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / MIB as f64
}

/// The three large transfers of the Prove: an upload, a download with a
/// Content-Length and a chunked download. Gives the peak of each.
async fn large_transfers(tunnel: &Tunnel) -> [u64; 3] {
    let length = LARGE.to_string();
    let upload = request("/upload", &[("Content-Length", length.as_str())]);
    let ((status, _), up) = peak(transfer(tunnel, upload, LARGE)).await;
    assert_eq!(status, "200");
    let ((status, got), down) = peak(transfer(tunnel, request("/download", &[]), 0)).await;
    assert_eq!((status.as_str(), got), ("200", LARGE));
    let ((status, got), chunked) = peak(transfer(tunnel, request("/chunked", &[]), 0)).await;
    assert_eq!((status.as_str(), got), ("200", LARGE));
    [up, down, chunked]
}

/// A 50 MiB upload, a 50 MiB download and a 50 MiB chunked download with
/// `--inspect` keep the resident set within 8 MiB above the baseline, and
/// the ring keeps each body at the cap with the count of the cut bytes.
/// The same transfers with no sink run first, for a comparison.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "a measurement: run it alone (docs/development.md, Measurements)"]
async fn measure_the_resident_set_of_50_mib_each_way() {
    pod_testkit::need_loopback!();
    let at = quiet_origin().await;
    let with = tunnel(&at, Options::default());
    let without = plain(&at);
    // The warm-up: the threads of the runtime and the first buffers.
    for t in [&without, &with] {
        let (status, _) = transfer(t, request("/ok", &[("Content-Length", "1024")]), 1024).await;
        assert_eq!(status, "200");
    }
    let baseline = rss();
    let plain_peaks = large_transfers(&without).await;
    let inspect_peaks = large_transfers(&with).await;
    let names = ["upload", "download", "chunked download"];
    println!("baseline resident set: {:.1} MiB", mib(baseline));
    for (at, name) in names.iter().enumerate() {
        println!(
            "{name}: no --inspect +{:.1} MiB, --inspect +{:.1} MiB",
            mib(plain_peaks[at].saturating_sub(baseline)),
            mib(inspect_peaks[at].saturating_sub(baseline)),
        );
    }
    for (at, name) in names.iter().enumerate() {
        let over = inspect_peaks[at].saturating_sub(baseline);
        assert!(
            over <= RSS_BOUND,
            "{name} with --inspect: +{:.1} MiB",
            mib(over)
        );
    }
    // The ring: the warm-up, then the three large exchanges.
    let cap = Options::default().body;
    let ring: Vec<_> = (2..=4).map(|id| with.inspector.get(id).unwrap()).collect();
    let bodies = [&ring[0].request, &ring[1].response, &ring[2].response];
    for (body, name) in bodies.iter().zip(names) {
        assert_eq!(body.total, LARGE, "{name}");
        assert_eq!(body.bytes.len(), cap, "{name}: the stored body is the cap");
        assert_eq!(body.cut(), LARGE - cap as u64, "{name}: the cut is counted");
    }
    for (exchange, name) in ring.iter().zip(names) {
        assert_eq!(exchange.state, State::Complete, "{name}");
        assert!(exchange.summary()["cut"].as_u64().unwrap() > 0, "{name}");
    }
}

/// The latency of each request and the CPU time of the process for
/// `count` requests at 1000 per second, each a POST of 1 KiB.
async fn paced(tunnel: &Arc<Tunnel>, count: usize) -> (Vec<Duration>, Duration, Duration) {
    let cpu = cpu_time();
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(1));
    let mut runs = Vec::with_capacity(count);
    for _ in 0..count {
        tick.tick().await;
        let tunnel = tunnel.clone();
        runs.push(tokio::spawn(async move {
            let body = [b'j'; 1024];
            let headers = [("Content-Length", "1024"), ("Authorization", "Bearer t")];
            visit(&tunnel, request("/ok", &headers), &body).await
        }));
    }
    let mut times = Vec::with_capacity(count);
    for run in runs {
        let (status, body, _, all) = run.await.unwrap();
        assert_eq!((status.as_str(), body.as_slice()), ("200", &b"ok"[..]));
        times.push(all);
    }
    (times, start.elapsed(), cpu_time() - cpu)
}

/// The user and the system CPU time of this process.
fn cpu_time() -> Duration {
    // SAFETY: getrusage() writes into the struct that it gets.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let of = |t: libc::timeval| {
        Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64)
    };
    of(usage.ru_utime) + of(usage.ru_stime)
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

/// The cost of the capture: 3000 requests at 1000 per second against a
/// loopback origin, two rounds with no sink and two with `--inspect`, in
/// turn. It prints the latency (p50, p99) and the CPU time for each
/// request; the record keeps the numbers (T-244, Correction).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "a measurement: run it alone (docs/development.md, Measurements)"]
async fn measure_the_cost_of_the_capture_at_1000_requests_per_second() {
    pod_testkit::need_loopback!();
    const COUNT: usize = 3000;
    let at = quiet_origin().await;
    let with = Arc::new(tunnel(&at, Options::default()));
    let without = Arc::new(plain(&at));
    paced(&without, 500).await;
    paced(&with, 500).await;
    let mut rows = Vec::new();
    for round in 0..4 {
        let (name, t) = match round % 2 {
            0 => ("no --inspect", &without),
            _ => ("--inspect", &with),
        };
        let (mut times, wall, cpu) = paced(t, COUNT).await;
        times.sort();
        let row = (
            name,
            percentile(&times, 50),
            percentile(&times, 99),
            cpu.as_micros() as f64 / COUNT as f64,
            COUNT as f64 / wall.as_secs_f64(),
        );
        println!(
            "{:<13} p50 {:>7.3} ms  p99 {:>7.3} ms  cpu {:>6.1} us/request  {:>6.0} requests/s",
            row.0,
            row.1.as_secs_f64() * 1e3,
            row.2.as_secs_f64() * 1e3,
            row.3,
            row.4
        );
        rows.push(row);
    }
    let keep = Options::default().keep;
    let list = with.inspector.list();
    assert_eq!(list.len(), keep, "the ring keeps the last {keep}");
    assert!(list.iter().all(|e| e["state"] == "complete"));
    let mean = |name: &str, pick: fn(&(&str, Duration, Duration, f64, f64)) -> f64| {
        let values: Vec<f64> = rows.iter().filter(|r| r.0 == name).map(pick).collect();
        values.iter().sum::<f64>() / values.len() as f64
    };
    let p50 = |r: &(&str, Duration, Duration, f64, f64)| r.1.as_secs_f64() * 1e3;
    let p99 = |r: &(&str, Duration, Duration, f64, f64)| r.2.as_secs_f64() * 1e3;
    let cpu = |r: &(&str, Duration, Duration, f64, f64)| r.3;
    println!(
        "added by --inspect: p50 {:+.3} ms, p99 {:+.3} ms, cpu {:+.1} us/request",
        mean("--inspect", p50) - mean("no --inspect", p50),
        mean("--inspect", p99) - mean("no --inspect", p99),
        mean("--inspect", cpu) - mean("no --inspect", cpu),
    );
}
