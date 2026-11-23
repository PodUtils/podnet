//! `podnet sidecar`: the packet sidecar, behind the build feature `sidecar`.
//! A default build exits 69 with the feature name.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

#[cfg(feature = "sidecar")]
use anyhow::{anyhow, Context};
use podnet::build_features;
#[cfg(feature = "sidecar")]
use podnet::exit::{Fault, ResultExt};

#[derive(Args, Debug)]
#[command(subcommand_required = true, arg_required_else_help = false)]
pub(crate) struct SidecarArgs {
    #[command(subcommand)]
    pub(crate) command: SidecarCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SidecarCommand {
    /// The three-valued disagreement probe. `simulator` is a correct local
    /// path. `disagreeing` is the simulator with a known disagreement.
    /// `silent` answers nothing, and it reports `could not determine`. `raw`
    /// is the host's raw socket, with a named skip when the host refuses one.
    /// No live network.
    Probe {
        /// The path: simulator, disagreeing, silent or raw.
        #[arg(long, default_value = "disagreeing", value_name = "NAME")]
        path: String,
        /// The target of `--path raw`, as IP:PORT.
        #[arg(long, value_name = "IP:PORT")]
        target: Option<String>,
    },
    /// The genetic search against the local simulator, and no live path.
    Search {
        /// The seed of the search.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// The most generations.
        #[arg(long, default_value_t = 40)]
        generations: usize,
        /// A hand-written strategy file: search towards its divergence.
        #[arg(long, value_name = "FILE")]
        towards: Option<PathBuf>,
    },
    /// Apply a dated strategy to the local simulator. A file with no date
    /// has no evidence, and exits 78.
    Run {
        /// The strategy file: date, network, found_by and an ordered list of
        /// steps with a trigger and one of the five primitives.
        #[arg(long, value_name = "FILE")]
        strategy: PathBuf,
    },
    /// The split and tamper behaviours, each with the condition that
    /// defeats it and the check that sees the defeat.
    Split,
    /// A UDP echo through fake TCP against a middlebox that refuses a plain
    /// UDP datagram. Raw sockets give a named skip, not a failure.
    Faketcp {
        /// The message to echo.
        #[arg(long, default_value = "podnet sidecar", value_name = "TEXT")]
        text: String,
    },
    /// Two emulated links at 10:1 carry one flow within 20 percent of the
    /// sum, and the flow does not move mid-stream.
    Bond,
}

/// `podnet sidecar`. The feature check comes before any file or path.
pub(crate) async fn sidecar(args: SidecarArgs) -> Result<()> {
    build_features::require("sidecar")?;
    #[cfg(feature = "sidecar")]
    {
        run(args).await
    }
    #[cfg(not(feature = "sidecar"))]
    {
        // `require` returned above: this build has the feature.
        let _ = args;
        unreachable!("the sidecar feature is off")
    }
}

#[cfg(feature = "sidecar")]
async fn run(args: SidecarArgs) -> Result<()> {
    use podnet::sidecar as side;

    match args.command {
        SidecarCommand::Probe { path, target } => {
            let path = side::ProbePath::parse(&path).ok_or_else(|| {
                podnet::exit::fault(
                    Fault::Usage,
                    anyhow!(
                        "no path {:?}; use one of {}",
                        path,
                        side::ProbePath::ALL.join(", ")
                    ),
                )
            })?;
            let target = match target {
                Some(text) => Some(text.parse().map_err(|_| {
                    podnet::exit::fault(Fault::Usage, anyhow!("--target {text} is not IP:PORT"))
                })?),
                None => None,
            };
            let finding = side::probe(path, target);
            println!("verdict  {}", side::verdict_word(finding.verdict));
            println!("reason   {}", finding.reason);
            for check in &finding.checks {
                println!("check    {} {:?}", check.kind, check.outcome);
            }
            Ok(())
        }
        SidecarCommand::Search {
            seed,
            generations,
            towards,
        } => {
            let outcome = match towards {
                None => side::search_find(seed, generations),
                Some(path) => {
                    let file = std::fs::read_to_string(&path)
                        .with_context(|| format!("--towards {}", path.display()))
                        .fault(Fault::NoInput)?;
                    let target = side::strategy_of(&file).map_err(strategy_fault)?;
                    side::search_towards(&target, seed, generations)
                }
            }
            .map_err(|error| {
                podnet::exit::fault(Fault::Unavailable, anyhow!("the search: {error}"))
            })?;
            println!("seed       {seed}");
            println!("generation {}", outcome.generation);
            println!("score      {}", outcome.score);
            println!("divergence {}", outcome.run.divergence.summary());
            println!("steps      {}", outcome.best.steps.len());
            for step in &outcome.best.steps {
                println!("  {} {}", step.trigger, step.primitive);
            }
            println!("history    {}", outcome.history.len());
            Ok(())
        }
        SidecarCommand::Run { strategy } => {
            let text = std::fs::read_to_string(&strategy)
                .with_context(|| format!("--strategy {}", strategy.display()))
                .fault(Fault::NoInput)?;
            let strategy = side::strategy_of(&text).map_err(strategy_fault)?;
            println!(
                "date       {} network {} found_by {}",
                strategy.date, strategy.network, strategy.found_by
            );
            for (index, step) in strategy.steps.iter().enumerate() {
                println!("step {index:02}  {} {}", step.trigger, step.primitive);
            }
            let run = side::run_strategy(&strategy);
            println!("divergence {}", run.divergence.summary());
            println!(
                "states     {}",
                if side::produced_state_divergence(&run) {
                    "changed"
                } else {
                    "unchanged"
                }
            );
            println!(
                "segments   out {} in {}",
                run.frames
                    .iter()
                    .filter(|frame| frame.direction == pod_sidecar::Direction::Out)
                    .count(),
                run.frames
                    .iter()
                    .filter(|frame| frame.direction == pod_sidecar::Direction::In)
                    .count()
            );
            Ok(())
        }
        SidecarCommand::Split => {
            for outcome in side::split_all() {
                println!("behaviour  {}", outcome.behaviour);
                match &outcome.run {
                    Ok(run) => println!(
                        "  frames   {} (baseline {})",
                        run.frames.len(),
                        run.baseline.len()
                    ),
                    Err(error) => println!("  run      {error}"),
                }
                println!("  defeat   {}", outcome.defeat());
                match outcome.limit {
                    Some(limit) => {
                        println!("  limit    {}", limit.condition);
                        println!("  detect   {}", limit.detection);
                    }
                    None => println!("  limit    MISSING (a behaviour with no limit is a fault)"),
                }
            }
            Ok(())
        }
        SidecarCommand::Faketcp { text } => {
            let payload = text.as_bytes();
            match side::fake_tcp_echo(payload) {
                Ok(echo) => {
                    println!("udp      refused a plain datagram (the middlebox refuses UDP)");
                    println!("framed   {} bytes, echoed {}", echo.len(), echo == payload);
                }
                Err(error) => {
                    println!("framed   failed: {error}");
                    match side::raw_skip() {
                        Some(reason) => println!("skip     the live fake TCP carrier: {reason}"),
                        None => println!("raw      this host allows a raw socket"),
                    }
                    return Err(podnet::exit::fault(
                        Fault::Unavailable,
                        anyhow!("framed failed: {error}"),
                    ));
                }
            }
            match side::raw_skip() {
                Some(reason) => println!("skip     the live fake TCP carrier: {reason}"),
                None => println!("raw      this host allows a raw socket"),
            }
            Ok(())
        }
        SidecarCommand::Bond => {
            let report = side::bond_report().map_err(|error| {
                podnet::exit::fault(Fault::Unavailable, anyhow!("the bond: {error}"))
            })?;
            println!(
                "links     {} and {} (ratio 10:1), sum {}",
                report.first, report.second, report.sum
            );
            let flow = report.flow.unwrap_or(0);
            let percent = report.flow.map_or(0, |flow| flow * 100 / report.sum.max(1));
            println!(
                "flow      {flow} of {} ({percent} percent), moves {}",
                report.sum, report.moves
            );
            println!(
                "within    {}",
                if report.within_twenty_percent() {
                    "20 percent of the sum"
                } else {
                    "NOT within 20 percent of the sum"
                }
            );
            Ok(())
        }
    }
}

/// A strategy file that does not load: a file with no date exits 78 with
/// the reason (SYN-4).
#[cfg(feature = "sidecar")]
fn strategy_fault(error: podnet::sidecar::StrategyError) -> anyhow::Error {
    let text = error.to_string();
    podnet::exit::fault(Fault::Config, anyhow!("--strategy: {text}"))
}
