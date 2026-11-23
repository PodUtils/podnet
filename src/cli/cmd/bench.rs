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
    /// The first error, with its chain: its hop gives the second line of
    /// the error (GitHub #697).
    first_error: Option<anyhow::Error>,
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
                    .get_or_insert_with(|| anyhow::anyhow!("HTTP {}", response.status));
            }
            Err(err) => {
                tally.errors += 1;
                tally.first_error.get_or_insert(err);
            }
        }
    }
    tally
}

/// The document of a run (before the stamp), with `first_error` in the
/// words of the error line.
fn document(
    url: &str,
    connections: u32,
    elapsed: f64,
    tallies: Vec<Tally>,
    first_error: Option<&anyhow::Error>,
) -> Value {
    let first_error = first_error.map(podnet::report::chain_text);
    let mut good: Vec<f64> = Vec::new();
    let (mut errors, mut bytes) = (0, 0);
    for t in tallies {
        good.extend(t.good);
        errors += t.errors;
        bytes += t.bytes;
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

/// One value of the text output: `-` when the document has no sample (`null`).
fn shown(v: &Value) -> String {
    match v {
        Value::Null => "-".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn text(doc: &Value) -> String {
    let l = &doc["latency_ms"];
    format!(
        "podnet bench: {}\n  requests {} (ok {}, errors {}) in {} s on {} connections\n  \
         rate {} requests/s\n  latency ms: min {} p50 {} p95 {} p99 {} max {}\n",
        doc["url"].as_str().unwrap_or_default(),
        shown(&doc["requests"]),
        shown(&doc["ok"]),
        shown(&doc["errors"]),
        shown(&doc["duration_s"]),
        shown(&doc["connections"]),
        shown(&doc["rate"]),
        shown(&l["min"]),
        shown(&l["p50"]),
        shown(&l["p95"]),
        shown(&l["p99"]),
        shown(&l["max"]),
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
    let first = tallies.iter_mut().find_map(|t| t.first_error.take());
    let doc = document(
        &args.url,
        args.connections,
        elapsed,
        tallies,
        first.as_ref(),
    );
    if args.json {
        println!("{}", podnet::status::stamped(doc.clone()));
    } else {
        print!("{}", text(&doc));
    }
    if doc["ok"] == 0 {
        return Err(no_request(
            first.unwrap_or_else(|| anyhow::anyhow!("no request")),
        ));
    }
    Ok(())
}

/// The error of a run with no good request: the first error in the words
/// of the error line, with its hop (GitHub #697, #698). The exit code stays
/// 1, as the help of bench says: the error has no class.
fn no_request(first: anyhow::Error) -> anyhow::Error {
    let err = anyhow::anyhow!(
        "bench: no request worked: {}",
        podnet::report::chain_text(&first)
    );
    match podnet::hop::of(&first) {
        Some(hop) => podnet::hop::at(hop, err),
        None => err,
    }
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

    /// The text output never shows the JSON word `null`: a value with no
    /// sample is `-`, and the JSON document keeps `null` (T-530).
    #[test]
    fn the_text_of_bench_has_no_null() {
        let tally = |good: Vec<f64>, errors: u64| Tally {
            good,
            errors,
            bytes: 0,
            first_error: None,
        };
        let rows: [(&str, Value, &str); 4] = [
            (
                "samples",
                document("http://a/", 1, 1.0, vec![tally(vec![0.010, 0.020], 0)]),
                "min 10",
            ),
            (
                "no sample",
                document("http://a/", 4, 2.8, vec![tally(vec![], 50)]),
                "min - p50 - p95 - p99 - max -",
            ),
            (
                "no time",
                document("http://a/", 1, 0.0, vec![tally(vec![], 0)]),
                "rate - requests/s",
            ),
            ("empty document", json!({}), "min - p50 -"),
        ];
        for (name, doc, want) in rows {
            let out = text(&doc);
            assert!(!out.contains("null"), "{name}: {out}");
            assert!(out.contains(want), "{name}: want {want:?} in {out}");
        }
        let none = document("http://a/", 1, 1.0, vec![tally(vec![], 1)]);
        assert!(none["latency_ms"]["p50"].is_null(), "the JSON keeps null");
    }
}
