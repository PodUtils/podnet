//! The packet sidecar at the podnet surface.
//!
//! `pod-sidecar` of PodUtils/podcrates does the work: the five primitives,
//! the local middlebox simulator, the genetic search, the three-valued
//! probe, split and tamper, fake TCP and the per-flow bond. This module is
//! the glue of `podnet sidecar`. It opens no socket of its own and reads no
//! environment variable.
//!
//! The feature `sidecar` holds the whole module ([`crate::build_features`]).
//! A default build has no packet path, so `podnet sidecar` exits 69 with
//! the feature name (LIM-2). Where a raw socket is unavailable the probe
//! reports `could not determine`, never a pass (SYN-1, PROBE-2).

pub use pod_sidecar::limits;
pub use pod_sidecar::limits::{Limit, LIMITS};
pub use pod_sidecar::{
    Behaviour, Bond, Date, Divergence, Finding, Outcome, Params, Policy, Primitive, Probe, Run,
    Scenario, Search, Simulator, SplitConfig, SplitError, SplitRun, Step, Strategy, StrategyError,
    Trigger, Verdict,
};

/// The name of the build feature of this module.
pub const FEATURE: &str = pod_sidecar::FEATURE;

/// The local simulator seed of each command. A fixed seed makes a run
/// reproducible, and no command reads the host's entropy.
pub const SEED: u64 = 42;

/// The network that the search attributes its result to. The search runs
/// against the local simulator only (SYN-3).
pub const NETWORK: &str = "local-simulator";

/// The date that a search result carries. It is the date of this crate's
/// local simulator, not a date read from the host. A result is evidence of
/// a run on that date, and of nothing later.
pub const DATE: &str = "2026-11-20";

/// The date of a search result (SYN-4).
pub fn date() -> Date {
    // The crate refuses a strategy with no date; this one has one.
    Date::parse(DATE).expect("DATE is a calendar date")
}

/// A strategy from a file. A file with no date is refused by name (SYN-4).
pub fn strategy_of(text: &str) -> Result<Strategy, StrategyError> {
    Strategy::from_toml(text)
}

/// Run one strategy against the local simulator, and report the divergence
/// from the same run with no strategy.
pub fn run_strategy(strategy: &Strategy) -> Run {
    Simulator::new(SEED).run(strategy)
}

/// Run one strategy against the local simulator in the scenario where the
/// server refuses the connection with a RST. It is the only scenario in
/// which [`Trigger::FirstInboundRst`] can fire: the search's `Echo` run has
/// no inbound RST, so a strategy with that trigger is silent there.
pub fn run_strategy_refused(strategy: &Strategy) -> Run {
    Simulator::new(SEED).run_with(strategy, Scenario::Refused)
}

/// The three-valued probe of rule PROBE-1. A silent path reports
/// `could not determine`, as the issue's gate requires: a path that answers
/// nothing gives no evidence either way. A raw socket that the host refuses
/// is the same verdict, with the reason.
pub fn probe(path: ProbePath, target: Option<std::net::SocketAddrV4>) -> Finding {
    match path {
        ProbePath::Simulator => Probe::run(&mut Simulator::new(SEED)),
        ProbePath::Disagreeing => {
            Probe::run(&mut Simulator::new(SEED).with_policy(Policy::disagreeing()))
        }
        ProbePath::Silent => {
            let mut finding = Probe::run(&mut pod_sidecar::probe::Silent);
            // The crate treats silence as "agrees" because it is not
            // evidence of interference. The issue's gate asks for the third
            // value here: no measurement is not a pass.
            finding.verdict = Verdict::Unknown;
            finding.reason =
                "could not determine: the path answered no probe, so it gave no evidence".into();
            finding
        }
        ProbePath::Unavailable(reason) => {
            let mut device = pod_sidecar::probe::Unavailable::new(reason);
            Probe::run(&mut device)
        }
        ProbePath::Raw => match target {
            None => {
                let mut device = pod_sidecar::probe::Unavailable::new(
                    "raw socket probe needs --target IP:PORT, so it could not determine a path",
                );
                Probe::run(&mut device)
            }
            Some(target) => match pod_sidecar::probe::RawPath::open(target, SEED as u32) {
                Ok(mut raw) => Probe::run(&mut raw),
                Err(refusal) => {
                    let mut device = pod_sidecar::probe::Unavailable::new(refusal.reason());
                    Probe::run(&mut device)
                }
            },
        },
    }
}

/// A path that the probe can measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbePath {
    /// A correct local middlebox, with no disagreement.
    Simulator,
    /// The local middlebox with its known disagreement.
    Disagreeing,
    /// A path that answers nothing.
    Silent,
    /// A device that cannot run, with the reason.
    Unavailable(String),
    /// The host's raw socket, or a named skip when it refuses one.
    Raw,
}

impl ProbePath {
    /// The path named by `text`.
    pub fn parse(text: &str) -> Option<ProbePath> {
        match text {
            "simulator" => Some(ProbePath::Simulator),
            "disagreeing" => Some(ProbePath::Disagreeing),
            "silent" => Some(ProbePath::Silent),
            "raw" => Some(ProbePath::Raw),
            _ => None,
        }
    }

    pub const ALL: [&'static str; 4] = ["simulator", "disagreeing", "silent", "raw"];
}

/// The verdict as the issue words it. `Verdict::Unknown` is never a pass.
pub fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Disagrees => "disagrees",
        Verdict::Agrees => "agrees",
        Verdict::Unknown => "could not determine",
    }
}

/// The genetic search against the local simulator: the best strategy that
/// it found (SYN-3).
pub fn search_find(seed: u64, generations: usize) -> Result<Outcome, pod_sidecar::SearchError> {
    Search::new(seed, date(), NETWORK)
        .population(32)
        .generations(generations)
        .find()
}

/// The genetic search towards a hand-written strategy: the shortest
/// strategy that reproduces its divergence.
pub fn search_towards(
    target: &Strategy,
    seed: u64,
    generations: usize,
) -> Result<Outcome, pod_sidecar::SearchError> {
    Search::new(seed, date(), NETWORK)
        .population(32)
        .generations(generations)
        .towards(target)
}

/// One split behaviour and what the simulator saw.
#[derive(Clone, Debug)]
pub struct SplitOutcome {
    pub behaviour: Behaviour,
    /// The run against a path that reassembles or checks, or why it was
    /// defeated.
    pub run: Result<SplitRun, SplitError>,
    /// The same behaviour against a path that reassembles and checks every
    /// segment. A behaviour with no defeat is a claim (SPLIT-2).
    pub strict: Result<SplitRun, SplitError>,
    /// The condition that defeats this behaviour and the check that sees it
    /// (LIM-1).
    pub limit: Option<&'static Limit>,
}

impl SplitOutcome {
    /// The defeat of this behaviour, in one line.
    pub fn defeat(&self) -> String {
        match &self.strict {
            Err(error) => error.to_string(),
            Ok(_) => "the path did not defeat it".to_string(),
        }
    }
}

/// Each split behaviour of SPLIT-1 with its defeat.
pub fn split_all() -> Vec<SplitOutcome> {
    let simulator = Simulator::new(SEED);
    let strict = Simulator::new(SEED).with_policy(Policy::strict());
    Behaviour::ALL
        .into_iter()
        .map(|behaviour| SplitOutcome {
            behaviour,
            run: simulator.split(behaviour),
            strict: strict.split(behaviour),
            limit: pod_sidecar::limits::limit_of(behaviour.name()),
        })
        .collect()
}

/// The five primitives of SYN-2 with a trigger that fires on the first
/// outbound data packet.
pub fn primitive_strategies() -> Vec<(Primitive, Strategy)> {
    Primitive::ALL
        .into_iter()
        .map(|primitive| {
            let step = Step::new(Trigger::FirstOutboundData, primitive);
            let strategy = Strategy::new(date(), NETWORK, "hand", vec![step]);
            (primitive, strategy)
        })
        .collect()
}

/// The reason that the host cannot run a fake TCP carrier, when it cannot.
/// Fake TCP needs a raw IPv4 socket; a host without `CAP_NET_RAW` and
/// without uid 0 gives a named skip (SYN-1, FAKE-1).
pub fn raw_skip() -> Option<String> {
    pod_sidecar::probe::raw_capability()
        .err()
        .map(|refusal| refusal.reason().to_string())
}

/// A UDP echo over fake TCP against a middlebox that refuses a plain UDP
/// datagram. The message crosses in frames, and the echo comes back
/// (FAKE-1). No socket opens: both ends are the local simulator.
pub fn fake_tcp_echo(payload: &[u8]) -> Result<Vec<u8>, String> {
    let simulator = Simulator::new(SEED).with_policy(Policy::refusing_udp());
    let refusal = simulator
        .udp_echo(payload)
        .err()
        .map(|refusal| refusal.reason().to_string())
        .unwrap_or_else(|| "the middlebox carried a plain UDP datagram".to_string());
    let (a, b) = simulator.fake_tcp_pair();
    let mut client = pod_sidecar::FakeTcp::new(a);
    let mut server = pod_sidecar::FakeTcp::new(b);
    client.send(payload).map_err(|e| e.to_string())?;
    let got = server
        .recv()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "the far end got no message".to_string())?;
    server.send(&got).map_err(|e| e.to_string())?;
    let echo = client
        .recv()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "the near end got no echo".to_string())?;
    if echo != payload {
        return Err(format!("the echo differs: {refusal}"));
    }
    Ok(echo)
}

/// Two emulated links at 10:1 carry one flow within 20 percent of the sum,
/// and the flow does not move while its link lives (BOND-1, BOND-2).
#[derive(Clone, Debug)]
pub struct BondReport {
    pub first: u64,
    pub second: u64,
    pub sum: u64,
    pub flow: Option<u64>,
    pub moves: u64,
}

impl BondReport {
    /// True when the flow gets at least 80 percent of the sum.
    pub fn within_twenty_percent(&self) -> bool {
        self.flow.is_some_and(|flow| flow * 100 >= self.sum * 80)
    }
}

/// The bond of two links at 10:1, with one session on the larger link.
pub fn bond_report() -> Result<BondReport, pod_sidecar::BondError> {
    let mut bond = Bond::new(&[(1, 10), (2, 1)])?;
    let flow: u64 = 1;
    let _ = bond.assign(flow);
    for _ in 0..1000 {
        bond.send(flow, 1200)?;
    }
    Ok(BondReport {
        first: 10,
        second: 1,
        sum: bond.sum(),
        flow: bond.estimate(flow),
        moves: bond.moves(),
    })
}

/// The state divergence of one primitive, for the gate: the
/// simulator reports what changed in the states, the bytes and the segment
/// counts.
pub fn divergence_line(strategy: &Strategy) -> String {
    let run = run_strategy(strategy);
    run.divergence.summary()
}

/// A concise line for a probe finding: the verdict and its reason.
pub fn finding_line(finding: &Finding) -> String {
    format!("{}: {}", verdict_word(finding.verdict), finding.reason)
}

/// The divergence is not empty, and the strategy changed the state of at
/// least one end.
pub fn produced_state_divergence(run: &Run) -> bool {
    !run.divergence.states.is_empty()
}

/// A `Divergence` for a strategy, for a test that pins the intended change
/// of each primitive.
pub fn divergence_of(strategy: &Strategy) -> Divergence {
    run_strategy(strategy).divergence
}
