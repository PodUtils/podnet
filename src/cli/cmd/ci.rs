//! `podnet ci`: a pipeline under the egress policy (CI-1).
//!
//! `check` reads a declaration of the hosts, ports and UDP needs of a
//! pipeline and refuses before the first step when the installed egress
//! policy does not cover one of them. The offending host is named and the
//! exit code is 69.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

use podnet::exit::Fault;

/// `podnet ci`.
#[derive(Args, Debug)]
pub(crate) struct CiArgs {
    #[command(subcommand)]
    pub(crate) command: CiCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum CiCommand {
    /// Check a declaration against the installed egress policy. It exits 69
    /// with the offending host named when the policy does not cover it.
    Check {
        /// The declaration: {"schema": ..., "hosts": [{host, port, udp}]}.
        file: PathBuf,
        /// Print one JSON document.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn ci(args: CiArgs) -> Result<()> {
    match args.command {
        CiCommand::Check { file, json } => check(&file, json),
    }
}

fn check(file: &std::path::Path, json: bool) -> Result<()> {
    let policy = podnet::policy::installed().ok_or_else(|| {
        podnet::exit::fault(
            Fault::Config,
            anyhow::anyhow!(
                "no egress policy is installed: set PODNET_POLICY or PODNET_NODE_CONFIG before a CI check"
            ),
        )
    })?;
    let text = std::fs::read_to_string(file).map_err(|e| {
        podnet::exit::fault(Fault::NoInput, anyhow::anyhow!("{}: {e}", file.display()))
    })?;
    let report = podnet::ci::check_text(&text, policy).map_err(|e| {
        podnet::exit::fault(Fault::Config, anyhow::anyhow!("{}: {e}", file.display()))
    })?;
    if json {
        let refused: Vec<serde_json::Value> = report
            .refused
            .iter()
            .map(|refusal| {
                serde_json::json!({
                    "host": refusal.host,
                    "port": refusal.port,
                    "udp": refusal.udp,
                    "reason": refusal.reason,
                })
            })
            .collect();
        println!(
            "{}",
            // T-519: the common stamp; the old string id is `kind`.
            serde_json::to_string_pretty(&podnet::status::stamped(serde_json::json!({
                "schema": 1,
                "kind": "podnet.ci-check/1",
                "file": file,
                "declaration": report.schema,
                "allowed": report.allowed,
                "refused": refused,
            })))?
        );
    } else {
        println!(
            "{}: {} need(s) covered, {} refused",
            file.display(),
            report.allowed,
            report.refused.len()
        );
        for refusal in &report.refused {
            println!("{refusal}");
        }
    }
    if report.covered() {
        return Ok(());
    }
    let names: Vec<String> = report
        .refused
        .iter()
        .map(|refusal| format!("{}:{}", refusal.host, refusal.port))
        .collect();
    Err(podnet::exit::fault(
        Fault::Unavailable,
        anyhow::anyhow!(
            "the egress policy does not cover {}: the pipeline is refused before its first step",
            names.join(", ")
        ),
    ))
}
