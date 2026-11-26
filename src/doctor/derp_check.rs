//! The row `derp` of `podnet doctor` (T-128): derper's probe (`GET
//! /derp/probe`, `pod_derp::servers::probe_kept`) of the DERP servers that
//! `podnet derp node` and `podnet send --derp` call home, through the way
//! out of podnet (the proxy, `NO_PROXY`, `--resolve`, the TLS roots).
//!
//! * The servers: the list of `--derp-server`, else Tailscale's four
//!   servers for tailcat. When none of the four answers, the first servers
//!   of the main fleet ([`MAIN_PROBES`], one of each region) are probed
//!   next, as a node calls them home next.
//! * The probes of one list run at the same time, one probe for each
//!   server, and each list ends at the time limit of the row (three times
//!   `--timeout`, and the end of `--quick`).
//! * `ok`: the first server of the list that answered (the home that a node
//!   takes), its round trip, and how many servers answered. `fail`: each
//!   server gave an answer that is not derper's (a refusal, a reset, a TLS
//!   fault, an HTTP status). `unknown`: a server did not answer in the time
//!   limit.
//!
//! The row does not change `ok` of the doctor: the DERP road is not a leg
//! of a tunnel.

use std::future::Future;

use std::time::Duration;

use pod_derp::servers::{self, Missed, Server};
use pod_proxy::Cut;

use super::Verdict;

/// The servers of the main fleet that the row probes when no server for
/// tailcat answers: the first server of each of the first regions.
pub const MAIN_PROBES: usize = 8;

/// The row `derp`.
#[derive(Clone, Debug)]
pub struct Derp {
    pub verdict: Verdict,
    /// The first server of the list that answered: `HOST` or `HOST:PORT`.
    pub server: Option<String>,
    /// The fleet of that server; none for a server of the user's own.
    pub fleet: Option<&'static str>,
    /// The round trip of the probe of that server.
    pub time: Option<Duration>,
    /// The servers that answered.
    pub answered: usize,
    /// The servers that the row probed (`HOST:PORT`), in order.
    pub probed: Vec<String>,
    pub detail: String,
}

impl Derp {
    /// A row with no probe.
    pub fn unknown(detail: impl Into<String>) -> Derp {
        Derp {
            verdict: Verdict::Unknown,
            server: None,
            fleet: None,
            time: None,
            answered: 0,
            probed: Vec::new(),
            detail: detail.into(),
        }
    }

    /// The line of the text report.
    pub fn line(&self) -> String {
        format!("derp   {}: {}\n", self.verdict.name(), self.detail)
    }

    /// The fields of `--json`.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "verdict": self.verdict.name(),
            "status": self.verdict.name(),
            "server": self.server,
            "fleet": self.fleet,
            "ms": self.time.map(|d| (d.as_secs_f64() * 10000.0).round() / 10.0),
            "answered": self.answered,
            "probed": self.probed,
            "detail": self.detail,
        })
    }
}

/// The lists of the row: the servers of `--derp-server` alone, else
/// tailcat's four, then the first [`MAIN_PROBES`] of the main fleet.
pub fn lists(named: Option<&[Server]>) -> (Vec<Server>, Vec<Server>) {
    match named {
        Some(list) => (list.to_vec(), Vec::new()),
        None => (
            servers::tailcat_fleet(),
            servers::main_fleet()
                .into_iter()
                .take(MAIN_PROBES)
                .collect(),
        ),
    }
}

/// Start the row at the start of the doctor, with derper's probe through
/// the way out of podnet. It ends by itself at `limit`, or at `end` of
/// `--quick` when that comes first.
pub fn start(
    named: Option<&[Server]>,
    limit: Duration,
    end: Option<tokio::time::Instant>,
) -> tokio::task::JoinHandle<Derp> {
    let (first, then) = lists(named);
    let until = tokio::time::Instant::now() + limit;
    let until = end.map_or(until, |end| end.min(until));
    tokio::spawn(check(first, then, crate::derp::probe_kept(), until))
}

/// Probe the servers of `first`, then those of `then` when none of `first`
/// answered, with `probe`, until `until`.
pub async fn check<P, F>(
    first: Vec<Server>,
    then: Vec<Server>,
    probe: P,
    until: tokio::time::Instant,
) -> Derp
where
    P: Fn(&Server) -> F,
    F: Future<Output = Result<Duration, Missed>>,
{
    let limit = until.saturating_duration_since(tokio::time::Instant::now());
    let mut probed: Vec<String> = Vec::new();
    let mut missed: Vec<Missed> = Vec::new();
    for list in [first, then] {
        if list.is_empty() || tokio::time::Instant::now() >= until {
            continue;
        }
        let results = futures::future::join_all(list.iter().map(|server| {
            let probing = probe(server);
            async move { tokio::time::timeout_at(until, probing).await }
        }))
        .await;
        probed.extend(list.iter().map(Server::authority));
        let mut answered = Vec::new();
        for (server, result) in list.iter().zip(results) {
            match result {
                Ok(Ok(took)) => answered.push((server, took)),
                Ok(Err(miss)) => missed.push(miss),
                Err(_) => missed.push(Missed {
                    why: format!(
                        "{server} did not answer {} in the time limit of the row ({:.1} s)",
                        servers::probe_path(),
                        limit.as_secs_f64()
                    ),
                    cut: Some(Cut::TimedOut),
                }),
            }
        }
        if let Some((server, took)) = answered.first() {
            let fleet = server.fleet_name();
            let of = fleet.map(|f| format!(" ({f})")).unwrap_or_default();
            return Derp {
                verdict: Verdict::Ok,
                server: Some(server.to_string()),
                fleet,
                time: Some(*took),
                answered: answered.len(),
                detail: format!(
                    "{server}{of} answered {} in {:.1} ms; {} of {} servers answered",
                    servers::probe_path(),
                    took.as_secs_f64() * 1000.0,
                    answered.len(),
                    probed.len()
                ),
                probed,
            };
        }
    }
    none(probed, &missed, limit)
}

/// The row when no server answered: the reason of the first server, and
/// the count of the others.
fn none(probed: Vec<String>, missed: &[Missed], limit: Duration) -> Derp {
    let Some(first) = missed.first() else {
        return Derp {
            probed,
            ..Derp::unknown(format!(
                "no DERP server was probed in the time limit ({:.1} s)",
                limit.as_secs_f64()
            ))
        };
    };
    // A server that did not answer in time says nothing of the path.
    let verdict = if missed.iter().any(|m| m.cut == Some(Cut::TimedOut)) {
        Verdict::Unknown
    } else {
        Verdict::Refused
    };
    let more = match missed.len() - 1 {
        0 => String::new(),
        n => format!("; and {n} more that did not"),
    };
    Derp {
        verdict,
        detail: format!(
            "no DERP server of {} answered {}: {}{more}",
            probed.len(),
            servers::probe_path(),
            first.why
        ),
        probed,
        ..Derp::unknown("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the stand-in probe gives for a server.
    #[derive(Clone, Copy)]
    enum Answer {
        In(u64),
        Refused,
        Silent,
    }

    /// The answers of a, b and m; the verdict, the server, the count of
    /// probes and words of the detail.
    type Case = (
        [Answer; 3],
        Verdict,
        Option<&'static str>,
        usize,
        &'static str,
    );

    fn servers(names: &[&str]) -> Vec<Server> {
        names.iter().map(|n| Server::parse(n).unwrap()).collect()
    }

    // T-128: the row of each shape of answer, with a probe of the test in
    // place of derper's. The row names the first server of the list that
    // answered (the home of a node), counts the others, probes the second
    // list only when none of the first answered, and ends in its time limit
    // when a server is silent.
    #[tokio::test]
    async fn each_shape_of_answer_gives_its_row() {
        use Answer::*;
        let (a, b, m) = ("a.example.org", "b.example.org:8443", "m.example.org");
        let table: [Case; 6] = [
            (
                [In(30), In(5), In(1)],
                Verdict::Ok,
                Some(a),
                2,
                "2 of 2 servers answered",
            ),
            (
                [Refused, In(5), In(1)],
                Verdict::Ok,
                Some(b),
                2,
                "1 of 2 servers answered",
            ),
            (
                [Refused, Refused, In(1)],
                Verdict::Ok,
                Some(m),
                3,
                "1 of 3 servers answered",
            ),
            (
                [Refused, Refused, Refused],
                Verdict::Refused,
                None,
                3,
                "a.example.org: refused; and 2 more",
            ),
            (
                [Silent, Refused, Refused],
                Verdict::Unknown,
                None,
                2,
                "in the time limit of the row",
            ),
            (
                [Silent, Silent, In(1)],
                Verdict::Unknown,
                None,
                2,
                "no DERP server of 2 answered",
            ),
        ];
        for (answers, verdict, server, probed, words) in table {
            let probe = move |s: &Server| {
                let at = [a, b, m]
                    .iter()
                    .position(|n| Server::parse(n).unwrap() == *s);
                let answer = answers[at.unwrap()];
                let why = format!("{s}: refused");
                async move {
                    match answer {
                        In(ms) => Ok(Duration::from_millis(ms)),
                        Refused => Err(Missed {
                            why,
                            cut: Some(Cut::Refused),
                        }),
                        Silent => std::future::pending().await,
                    }
                }
            };
            let limit = Duration::from_millis(300);
            let begin = std::time::Instant::now();
            let until = tokio::time::Instant::now() + limit;
            let row = check(servers(&[a, b]), servers(&[m]), probe, until).await;
            let case = format!("{}: {}", verdict.name(), row.detail);
            assert!(
                begin.elapsed() < limit + Duration::from_millis(200),
                "{case}"
            );
            assert_eq!(row.verdict, verdict, "{case}");
            assert_eq!(row.server.as_deref(), server, "{case}");
            assert_eq!(row.probed.len(), probed, "{case}");
            assert!(row.detail.contains(words), "{case}");
            assert_eq!(row.json()["status"], verdict.name(), "{case}");
            assert!(row.line().starts_with("derp   "), "{case}");
        }
    }

    // T-128: with no --derp-server, the row probes tailcat's four, then
    // one server of each of the first regions of the main fleet.
    #[test]
    fn the_default_lists_are_tailcat_s_four_then_the_main_fleet() {
        let (first, then) = lists(None);
        assert_eq!(first, servers::tailcat_fleet());
        assert_eq!(then.len(), MAIN_PROBES);
        assert!(then
            .iter()
            .all(|s| s.fleet_name() == Some(servers::MAIN_FLEET)));
        let own = servers(&["derp.example.org:8443"]);
        assert_eq!(lists(Some(&own)), (own.clone(), Vec::new()));
    }
}
