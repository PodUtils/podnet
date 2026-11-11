//! `podnet bench URL` (T-221): send GET requests to a URL through the way out
//! of the process (`--egress`, `--proxy`) and measure the latency, the rate
//! and the errors.

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};

use podnet::exit::Fault;

#[derive(Args, Debug)]
pub(crate) struct BenchArgs {
    /// The http:// or https:// URL to request.
    #[arg(value_name = "URL")]
    url: String,
    /// Print one JSON document (schema 1) in place of the text.
    #[arg(long)]
    json: bool,
    /// Requests in flight, each one on its own connection.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u32).range(1..=256))]
    connections: u32,
    /// Seconds to send requests. A request that started in time ends.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=3600))]
    duration: u64,
    /// The time limit of one request, in seconds.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=300))]
    timeout: u64,
}

/// The result of one worker.
#[derive(Default)]
struct Tally {
    /// The latency of each good request (status below 400), in seconds.
    good: Vec<f64>,
    errors: u64,
    bytes: u64,
    first_error: Option<String>,
}

/// The value at the percentile `p` (0-100) of the sorted `v`, by the
/// nearest rank. `None` for an empty list.
pub(crate) fn percentile(v: &[f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    Some(v[rank.clamp(1, v.len()) - 1])
}

fn ms(seconds: Option<f64>) -> Value {
    match seconds {
        Some(s) => json!((s * 1_000_000.0).round() / 1000.0),
        None => Value::Null,
    }
}

async fn worker(url: String, deadline: Instant, timeout: Duration) -> Tally {
    let mut tally = Tally::default();
    while Instant::now() < deadline {
        let request = podnet::fetch::Request::new("GET", url.clone()).timeout(timeout);
        let start = Instant::now();
        match podnet::fetch::send(request).await {
            Ok(response) if response.status < 400 => {
                tally.good.push(start.elapsed().as_secs_f64());
                tally.bytes += response.body.len() as u64;
            }
            Ok(response) => {
                tally.errors += 1;
                tally
                    .first_error
                    .get_or_insert_with(|| format!("HTTP {}", response.status));
            }
            Err(err) => {
                tally.errors += 1;
                tally.first_error.get_or_insert_with(|| format!("{err:#}"));
            }
        }
    }
    tally
}

/// The document of a run (before the stamp).
fn document(url: &str, connections: u32, elapsed: f64, tallies: Vec<Tally>) -> Value {
    let mut good: Vec<f64> = Vec::new();
    let (mut errors, mut bytes, mut first_error) = (0, 0, None);
    for t in tallies {
        good.extend(t.good);
        errors += t.errors;
        bytes += t.bytes;
        first_error = first_error.or(t.first_error);
    }
    good.sort_by(f64::total_cmp);
    let ok = good.len() as u64;
    let rate = if elapsed > 0.0 {
        json!((ok as f64 / elapsed * 100.0).round() / 100.0)
    } else {
        Value::Null
    };
    json!({
        "url": url,
        "connections": connections,
        "duration_s": (elapsed * 1000.0).round() / 1000.0,
        "requests": ok + errors,
        "ok": ok,
        "errors": errors,
        "first_error": first_error,
        "rate": rate,
        "bytes": bytes,
        "latency_ms": {
            "min": ms(good.first().copied()),
            "p50": ms(percentile(&good, 50.0)),
            "p95": ms(percentile(&good, 95.0)),
            "p99": ms(percentile(&good, 99.0)),
            "max": ms(good.last().copied()),
        },
    })
}

fn text(doc: &Value) -> String {
    let l = &doc["latency_ms"];
    format!(
        "podnet bench: {}\n  requests {} (ok {}, errors {}) in {} s on {} connections\n  \
         rate {} requests/s\n  latency ms: min {} p50 {} p95 {} p99 {} max {}\n",
        doc["url"].as_str().unwrap_or_default(),
        doc["requests"],
        doc["ok"],
        doc["errors"],
        doc["duration_s"],
        doc["connections"],
        doc["rate"],
        l["min"],
        l["p50"],
        l["p95"],
        l["p99"],
        l["max"],
    )
}

pub(crate) async fn bench(args: BenchArgs) -> Result<()> {
    if !(args.url.starts_with("http://") || args.url.starts_with("https://")) {
        return Err(podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!("bench: the URL must start with http:// or https://"),
        ));
    }
    let timeout = Duration::from_secs(args.timeout);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(args.duration);
    let workers: Vec<_> = (0..args.connections)
        .map(|_| tokio::spawn(worker(args.url.clone(), deadline, timeout)))
        .collect();
    let mut tallies = Vec::new();
    for w in workers {
        tallies.push(w.await?);
    }
    let elapsed = started.elapsed().as_secs_f64();
    let doc = document(&args.url, args.connections, elapsed, tallies);
    if args.json {
        println!("{}", podnet::status::stamped(doc.clone()));
    } else {
        print!("{}", text(&doc));
    }
    if doc["ok"] == 0 {
        anyhow::bail!(
            "bench: no request worked: {}",
            doc["first_error"].as_str().unwrap_or("no request")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_the_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 50.0), Some(50.0));
        assert_eq!(percentile(&v, 95.0), Some(95.0));
        assert_eq!(percentile(&v, 99.0), Some(99.0));
        assert_eq!(percentile(&v, 100.0), Some(100.0));
        assert_eq!(percentile(&[7.0], 99.0), Some(7.0));
        assert_eq!(percentile(&[], 50.0), None);
    }
}
