//! The commands of the host runtime and the verification harness: podnet
//! #500 and #501.
//!
//! `podnet host probe` states five file-system properties. `podnet stat`
//! measures a peer. `podnet rules check` names a shadowed rule. `podnet
//! init` generates, checks and enrols a node configuration. `podnet
//! harness` runs the road by profile matrix.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

use podnet::exit::Fault;
use podnet::initcfg::{NodeConfig, Registrar};
use podnet::policy::Policy;

/// `podnet host`.
#[derive(Args, Debug)]
pub(crate) struct HostArgs {
    #[command(subcommand)]
    pub(crate) command: HostCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum HostCommand {
    /// State the five properties of the file system of PATH: timestamps,
    /// atomic rename, hard links, permission bits and case sensitivity, as
    /// required or as emulated. It reads a boot sector of an image and the
    /// mount, and it opens no connection.
    Probe {
        /// A file or a directory on the file system.
        path: PathBuf,
        /// Print one JSON document.
        #[arg(long)]
        json: bool,
    },
    /// Record an event and panic, for the crash-log test. The value is
    /// hidden from the log.
    #[command(hide = true)]
    Panic {
        /// The value to seed, and to search the log for.
        #[arg(long, value_name = "VALUE")]
        seed: String,
    },
}

pub(crate) fn host(args: HostArgs) -> Result<()> {
    match args.command {
        HostCommand::Probe { path, json } => probe(&path, json),
        HostCommand::Panic { seed } => {
            podnet::crashlog::record(format!("the test seed is {seed}"));
            podnet::report::hide(&seed);
            panic!("podnet host panic: the crash-log test ({seed})");
        }
    }
}

fn probe(path: &std::path::Path, json: bool) -> Result<()> {
    let report = podnet::hostprobe::probe(path)?;
    if json {
        println!("{}", report.json());
        return Ok(());
    }
    println!("path: {}", report.path.display());
    println!("filesystem: {}", report.filesystem);
    println!("family: {}", report.family.name());
    println!("read-only: {}", if report.read_only { "yes" } else { "no" });
    println!("{:<18} {:<10} detail", "property", "status");
    for property in &report.properties {
        println!(
            "{:<18} {:<10} {}",
            property.name,
            property.status.as_str(),
            property.detail
        );
    }
    println!("fallback: {}", report.fallback);
    Ok(())
}

/// `podnet stat`.
#[derive(Args, Debug)]
pub(crate) struct StatArgs {
    /// The peer that timestamps each arrival, HOST:PORT.
    #[arg(long, value_name = "HOST:PORT", conflicts_with = "serve")]
    pub(crate) peer: Option<String>,
    /// Serve as the timestamping peer on ADDR, until Ctrl-C.
    #[arg(long, value_name = "ADDR", conflicts_with = "peer")]
    pub(crate) serve: Option<String>,
    /// The packets to send.
    #[arg(long, default_value_t = 10)]
    pub(crate) count: u32,
    /// The bytes of each packet.
    #[arg(long, default_value_t = 64)]
    pub(crate) size: usize,
    /// The rate in bits per second.
    #[arg(long, default_value_t = 1_000_000)]
    pub(crate) rate: u64,
    /// The read limit of one packet, in milliseconds.
    #[arg(long, default_value_t = podnet::stat::DEFAULT_TIMEOUT_MS)]
    pub(crate) timeout_ms: u64,
    /// The capture file.
    #[arg(long, value_name = "FILE", default_value = "pod-stat.txt")]
    pub(crate) out: PathBuf,
    /// Print the capture as JSON; the file is still written.
    #[arg(long)]
    pub(crate) json: bool,
}

pub(crate) fn stat(args: StatArgs) -> Result<()> {
    if let Some(addr) = args.serve {
        return podnet::stat::serve(&addr);
    }
    let peer = args.peer.ok_or_else(|| {
        podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!("podnet stat needs --peer HOST:PORT, or --serve ADDR"),
        )
    })?;
    let options = podnet::stat::Options {
        peer,
        count: args.count,
        size: args.size,
        rate_bps: args.rate,
        timeout: std::time::Duration::from_millis(args.timeout_ms),
    };
    let capture = podnet::stat::run(&options)?;
    capture.write(&args.out)?;
    if args.json {
        println!("{}", capture.json());
    } else {
        println!("{}", capture.text());
        println!("capture: {}", args.out.display());
    }
    Ok(())
}

/// `podnet rules`.
#[derive(Args, Debug)]
pub(crate) struct RulesArgs {
    #[command(subcommand)]
    pub(crate) command: RulesCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum RulesCommand {
    /// Parse the policy in FILE and name each rule that an earlier rule
    /// covers, so the later rule is unreachable. It exits 78 when it finds
    /// one, and when FILE cannot be read or parsed.
    Check {
        /// The JSON policy: {"default": "refuse", "rules": [{"name": "api",
        /// "host": "*.example.com", "port": 443, "family": "any",
        /// "action": "direct"}]}. docs/host-runtime.md gives each field.
        file: PathBuf,
        /// Print one JSON document.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn rules(args: RulesArgs) -> Result<()> {
    match args.command {
        RulesCommand::Check { file, json } => {
            let policy = Policy::load(&file)?;
            let shadows = policy.shadowed();
            if json {
                let rows: Vec<serde_json::Value> = shadows
                    .iter()
                    .map(|shadow| {
                        serde_json::json!({
                            "earlier": shadow.earlier,
                            "later": shadow.later,
                            "reason": shadow.reason,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    // T-519: the common stamp (`schema` 1 and `podnet`);
                    // the old string id is the key `kind`.
                    serde_json::to_string_pretty(&podnet::status::stamped(serde_json::json!({
                        "schema": 1,
                        "kind": "podnet.rules-check/1",
                        "file": file,
                        "rules": policy.rules.len(),
                        "default": policy.default.as_str(),
                        "shadowed": rows,
                    })))?
                );
            } else {
                println!(
                    "{}: {} rules, default {}",
                    file.display(),
                    policy.rules.len(),
                    policy.default.as_str()
                );
                for shadow in &shadows {
                    println!("{shadow}");
                }
            }
            if shadows.is_empty() {
                return Ok(());
            }
            Err(podnet::exit::fault(
                Fault::Config,
                anyhow::anyhow!(
                    "{}: {} rule(s) are unreachable: {}",
                    file.display(),
                    shadows.len(),
                    shadows
                        .iter()
                        .map(|shadow| shadow.later.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ))
        }
    }
}

/// `podnet init`.
#[derive(Args, Debug)]
pub(crate) struct InitArgs {
    #[command(subcommand)]
    pub(crate) command: InitCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum InitCommand {
    /// Generate a configuration that listens on nothing.
    Generate {
        /// The name of this node.
        node: String,
        /// The relay URL (http or https).
        #[arg(long, value_name = "URL")]
        relay: String,
        /// Write there; without it, print the document.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Validate a configuration against schema 1 and the policy. It exits
    /// 78 with the named part when it cannot be used.
    Check {
        /// The configuration.
        file: PathBuf,
        /// Print one JSON document.
        #[arg(long)]
        json: bool,
    },
    /// Enrol a node: the identity, the allow list and the relay
    /// registration, all three or none.
    Enroll {
        /// The state directory to make.
        dir: PathBuf,
        /// The name of this node.
        #[arg(long)]
        node: String,
        /// The relay URL.
        #[arg(long, value_name = "URL")]
        relay: String,
        /// A file of allow-list entries, one for each line.
        #[arg(long, value_name = "FILE")]
        allow: Option<PathBuf>,
        /// Do not register over the network; write the offline record.
        #[arg(long, conflicts_with = "register")]
        offline: bool,
        /// Register over HTTP, with this timeout in seconds.
        #[arg(long, value_name = "SECS", default_value_t = 10)]
        register: u64,
        /// Print one JSON document.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) async fn init(args: InitArgs) -> Result<()> {
    match args.command {
        InitCommand::Generate { node, relay, out } => {
            let config = NodeConfig::generated(&node, &relay);
            config.validate().map_err(|e| e.fault())?;
            let text = config.text()?;
            match out {
                Some(path) => {
                    std::fs::write(&path, &text)?;
                    println!("{}", path.display());
                }
                None => println!("{text}"),
            }
            Ok(())
        }
        InitCommand::Check { file, json } => {
            let config = NodeConfig::load(&file).map_err(|e| e.fault())?;
            if json {
                println!(
                    "{}",
                    // T-519: the common stamp; the old string id is `kind`.
                    serde_json::to_string_pretty(&podnet::status::stamped(serde_json::json!({
                        "schema": 1,
                        "kind": "podnet.init-check/1",
                        "file": file,
                        "node": config.node,
                        "relay": config.relay,
                        "listeners": config.listen.len(),
                        "allowed": config.allow.len(),
                        "valid": true,
                    })))?
                );
            } else {
                println!(
                    "{}: schema 1, node {}, relay {}, {} listeners, {} allowed",
                    file.display(),
                    config.node,
                    config.relay,
                    config.listen.len(),
                    config.allow.len()
                );
            }
            Ok(())
        }
        InitCommand::Enroll {
            dir,
            node,
            relay,
            allow,
            offline,
            register,
            json,
        } => {
            let mut config = NodeConfig::generated(&node, &relay);
            if let Some(path) = allow {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
                config.allow = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_string)
                    .collect();
            }
            let registrar = if offline {
                Registrar::Offline
            } else {
                Registrar::Http {
                    timeout: std::time::Duration::from_secs(register),
                }
            };
            let enrolled = podnet::initcfg::enrol(&dir, &config, &registrar).await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&podnet::status::stamped(serde_json::to_value(
                        &enrolled
                    )?))?
                );
            } else {
                println!("enrolled {} at {}", enrolled.node, enrolled.path);
                println!("fingerprint: {}", enrolled.fingerprint);
                println!("allowed: {}", enrolled.allowed);
                println!(
                    "registration: {}",
                    enrolled.registration["registered"]
                        .as_bool()
                        .map(|yes| if yes { "registered" } else { "offline" })
                        .unwrap_or("recorded")
                );
            }
            Ok(())
        }
    }
}

/// `podnet harness`.
#[derive(Args, Debug)]
pub(crate) struct HarnessArgs {
    /// The fixture directory: rules.json, flow.json, prefix.json,
    /// cases.json and captures/.
    #[arg(long, value_name = "DIR")]
    pub(crate) data: PathBuf,
    /// Print the table and its conditions as JSON.
    #[arg(long)]
    pub(crate) json: bool,
    /// Also write the relocatable archive (a zip) there.
    #[arg(long, value_name = "FILE")]
    pub(crate) archive: Option<PathBuf>,
}

pub(crate) fn harness(args: HarnessArgs) -> Result<()> {
    let report = podnet::harness::measure(&args.data, args.archive.as_deref())?;
    if args.json {
        println!("{}", report.json()?);
    } else {
        print!("{}", report.text());
    }
    Ok(())
}
