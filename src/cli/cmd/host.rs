//! The host commands: `status`, `doctor`, `man`, `update`, `ports`, `qr`,
//! `bug-report` and `completions`.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Args;

use podnet::exit::{Fault, ResultExt};

use crate::cli::Cli;

#[derive(Args, Debug)]
pub(crate) struct DoctorArgs {
    /// Print one JSON document.
    #[arg(long)]
    json: bool,
    /// A short run: one sample for each path, and 15 s for the whole run. A
    /// part that does not end in time gets the verdict `unknown`. The JSON
    /// document keeps the same keys.
    #[arg(long, conflicts_with_all = ["agent", "samples"])]
    quick: bool,
    /// Test what dev work can do here: git over HTTPS and SSH, the package
    /// registries, UDP DNS, IPv6 and localhost, directly, through the proxy
    /// and through a running podnet warp up; then print the commands to run.
    #[arg(long)]
    agent: bool,
    /// Samples of each TCP path (1-9). All samples end in 10 s.
    #[arg(long, default_value_t = 9, value_parser = clap::value_parser!(u8).range(1..=9))]
    samples: u8,
    /// The time limit of one connection and of one TLS handshake, in seconds.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..=10))]
    timeout: u64,
    /// The relay to test, as the `--relay` of `tunnel` (a loopback `http://`
    /// relay is allowed).
    #[arg(long, env = "PODNET_RELAY", hide_env_values = true, value_name = "URL")]
    relay: Option<String>,
    /// Also test the SSH relay of `podnet ssh-relay`: the TCP connect, the
    /// SSH banner and the auth with --ssh-user, with no session. Default: the
    /// relay of a running `ssh-relay` instance; else the row is `unknown`.
    #[arg(
        long,
        value_name = "HOST:PORT",
        requires = "ssh_user",
        conflicts_with = "agent"
    )]
    ssh_relay: Option<String>,
    /// The SSH user of --ssh-relay.
    #[arg(
        long,
        value_name = "USER",
        requires = "ssh_relay",
        conflicts_with = "agent"
    )]
    ssh_user: Option<String>,
    /// The HTTP CONNECT proxy of --ssh-relay: HOST:PORT. It replaces the
    /// proxy of the process (--proxy, PODNET_PROXY, the proxy variables).
    /// Default: that proxy, if it is an HTTP proxy with no credentials; else
    /// the row says why it does not dial.
    #[arg(
        long,
        value_name = "HOST:PORT",
        requires = "ssh_relay",
        conflicts_with = "agent"
    )]
    connect_proxy: Option<String>,
    /// The DERP servers of the row `derp`: HOST[:PORT][,HOST[:PORT]...]
    /// (port 443 by default), for example a derper of your own. Default:
    /// Tailscale's four servers for tailcat, then the first servers of its
    /// main fleet when none of the four answers. The row probes /derp/probe
    /// through the way out of podnet, and names the first server that
    /// answers and its round trip.
    #[arg(long, value_name = "LIST", conflicts_with = "agent")]
    derp_server: Option<String>,
    /// Check the shape of the sealed record layer of each road: its
    /// declared profile (page-load, video-call, terminal, file-transfer)
    /// beside a sample of 1000 records. One line for each road says
    /// `declared`, `observed` or `diverged`. A divergence is a warning line
    /// and not a failed session: the command exits 0. It opens no
    /// connection.
    #[arg(
        long,
        conflicts_with_all = [
            "agent",
            "quick",
            "samples",
            "timeout",
            "relay",
            "ssh_relay",
            "ssh_user",
            "connect_proxy",
            "derp_server"
        ]
    )]
    shape: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ManArgs {
    /// Sections: commands (tunnel, warp run) and topics (environment, files,
    /// inventory, exit-status, examples, pod). `podnet man tunnel serve` gives both.
    #[arg(value_name = "SECTION", num_args = 0..)]
    section: Vec<String>,
    /// Write to stdout, also on a terminal.
    #[arg(long)]
    no_pager: bool,
    /// Write a man(7) page.
    #[arg(long, conflicts_with = "json")]
    roff: bool,
    /// Write the manual as one JSON object.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct UpdateArgs {
    /// Only say which release is available; change nothing.
    #[arg(long)]
    check: bool,
    /// The tag to install, for example v0.4.0-beta.1. Default: the newest
    /// release (a prerelease only when this binary is a prerelease).
    #[arg(long, value_name = "TAG", value_parser = parse_release_tag)]
    version: Option<String>,
    /// Put the binary before the last update (podnet.prev) back, with no
    /// network. A second --rollback goes forward again.
    #[arg(long, conflicts_with_all = ["check", "version", "force"])]
    rollback: bool,
    /// Install the release also when it is the version of this binary.
    /// Without it, podnet says that the version is installed and exits 0.
    #[arg(long)]
    force: bool,
}

/// A `--version` of `podnet update`: a release tag (issue 227).
pub(crate) fn parse_release_tag(value: &str) -> Result<String, String> {
    if podnet::update::valid_tag(value) {
        Ok(value.to_string())
    } else {
        Err("not a release tag: v and a version, for example v0.3.0-beta.9".to_string())
    }
}

#[derive(Args, Clone, Debug)]
pub(crate) struct PortsArgs {
    /// Print one JSON object: {"ports": [PORT, ...]}.
    #[arg(long)]
    json: bool,
    /// Connect to each port, and list only the ports that accept a
    /// connection. With --json, the others are in "unreachable".
    #[arg(long)]
    probe: bool,
}

#[derive(Args, Debug)]
pub(crate) struct QrArgs {
    /// URL to encode (at most 2331 bytes). Another text gets a warning.
    url: String,
}

/// `podnet update`, and `podnet update --rollback`.
pub(crate) async fn update(args: UpdateArgs) -> Result<()> {
    if args.rollback {
        let path = std::env::current_exe()
            .and_then(|p| p.canonicalize())
            .context("finding this binary")?;
        let now = podnet::update::rollback(&path).fault(Fault::Unavailable)?;
        println!(
            "{}: {now} (put back from {})",
            path.display(),
            podnet::update::previous_of(&path).display()
        );
        return Ok(());
    }
    use podnet::update::Outcome;
    // Issue 249: an empty tag asked the network, then exit 69.
    if let Some(tag) = args.version.as_deref() {
        let good = !tag.is_empty()
            && tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b));
        if !good {
            return Err(podnet::exit::fault(
                Fault::Usage,
                anyhow::anyhow!("--version {tag:?}: give a tag like v0.3.0-beta.11"),
            ));
        }
    }
    match podnet::update::run(args.version.as_deref(), args.check, args.force)
        .await
        .fault(Fault::Unavailable)?
    {
        Outcome::UpToDate(tag) => println!("podnet {tag} is the newest release"),
        Outcome::Newer { current, tag } => {
            println!("podnet {current} is newer than the newest release {tag}; nothing to do")
        }
        Outcome::Same(tag) => {
            println!("podnet {tag} is installed; nothing to do (--force installs it again)")
        }
        Outcome::Available { current, tag } => {
            println!("podnet {current}; release {tag} is available (podnet update)")
        }
        Outcome::Updated { from, to, path } => {
            println!(
                "podnet {from} -> {to}: {} (signature and SHA-256 checked)",
                path.display()
            )
        }
    }
    Ok(())
}

/// `podnet completions SHELL`.
pub(crate) fn completions(shell: clap_complete::Shell) -> Result<()> {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut cmd, "podnet", &mut script);
    match std::io::Write::write_all(&mut std::io::stdout(), &script) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other.context("writing the completion script"),
    }
}

/// `podnet ports`.
pub(crate) async fn ports(args: PortsArgs) -> Result<()> {
    let mut ports = podnet::ports::listening_ports();
    if args.probe {
        // Issue 119: a port in the LISTEN table can refuse each
        // connection.
        let (open, unreachable) = podnet::ports::probe(&ports).await;
        ports = open;
        if args.json {
            println!(
                "{}",
                podnet::status::stamped(serde_json::json!({
                    "ports": ports,
                    "probed": true,
                    "unreachable": unreachable
                }))
            );
            return Ok(());
        }
        for port in &unreachable {
            eprintln!("podnet: ports: {port} listens, and refuses a connection");
        }
    }
    if args.json {
        println!(
            "{}",
            podnet::status::stamped(serde_json::json!({ "ports": ports }))
        );
    } else {
        for port in ports {
            println!("{port}");
        }
    }
    Ok(())
}

/// `podnet qr URL`.
pub(crate) fn qr(args: QrArgs) -> Result<()> {
    // Issue 95: a text over the limit is a usage mistake; a text
    // that is not a URL gets a warning (a QR code of any text is
    // valid).
    if args.url.len() > podnet::qr::MAX_BYTES {
        return Err(podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!(
                "the text is {} bytes; a QR code holds at most {} bytes",
                args.url.len(),
                podnet::qr::MAX_BYTES
            ),
        ));
    }
    if !podnet::qr::looks_like_url(&args.url) {
        eprintln!(
            "podnet: warning: {:?} is not a URL (scheme://...); the QR code holds the text as it is",
            args.url
        );
    }
    print!("{}", podnet::qr::render_terminal(&args.url)?);
    Ok(())
}

/// `podnet man`.
pub(crate) fn man(args: ManArgs) -> Result<()> {
    use clap::CommandFactory;
    let manual = podnet::manual::manual(Cli::command());
    let request = podnet::manual::Request {
        section: (!args.section.is_empty()).then(|| args.section.join(" ")),
        no_pager: args.no_pager,
        roff: args.roff,
        json: args.json,
    };
    // pod-manual pages the text only when stdin and stdout are terminals.
    let tty = pod_manual::pager::Tty::probed();
    let fault = |e: String| podnet::exit::fault(podnet::exit::Fault::Usage, anyhow::anyhow!(e));
    if !args.json {
        return pod_manual::run(
            &manual,
            &request,
            tty,
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
            &mut std::io::stderr(),
        )
        .map_err(fault);
    }
    // T-451: the JSON of the manual has `podnet`, as each document.
    let mut out = Vec::new();
    let done = pod_manual::run(
        &manual,
        &request,
        tty,
        &mut std::io::stdin().lock(),
        &mut out,
        &mut std::io::stderr(),
    );
    let text = podnet::status::stamped_text(&String::from_utf8_lossy(&out));
    print!("{text}");
    done.map_err(fault)
}

/// `podnet doctor`, and `podnet doctor --agent`.
/// `podnet doctor --shape`: one line for each road, with a warning line for
/// each divergence. It opens no connection.
fn shape(json: bool) -> Result<()> {
    let lines = podnet::doctor_shape::lines(podnet::doctor_shape::SAMPLES);
    if json {
        let roads: Vec<serde_json::Value> = lines
            .iter()
            .map(|line| {
                serde_json::json!({
                    "road": line.road,
                    "profile": line.profile,
                    "state": line.state,
                    "line": line.text,
                    "warnings": line.warnings,
                })
            })
            .collect();
        println!(
            "{}",
            podnet::status::stamped(serde_json::json!({"roads": roads}))
        );
        return Ok(());
    }
    for line in lines {
        println!("{}", line.text);
        for warning in line.warning_lines() {
            eprintln!("{warning}");
        }
    }
    Ok(())
}

pub(crate) async fn doctor(args: DoctorArgs) -> Result<()> {
    if args.shape {
        return shape(args.json);
    }
    if args.agent {
        let report = podnet::agent::run(Duration::from_secs(args.timeout.max(3))).await;
        if args.json {
            println!(
                "{}",
                podnet::status::stamped(serde_json::to_value(&report)?)
            );
        } else {
            print!("{}", podnet::agent::text(&report));
        }
        return Ok(());
    }
    podnet::udp::check_edge_env()?;
    podnet::udp::choice_from_env()?;
    let base = if args.quick {
        podnet::doctor::Options::quick()
    } else {
        podnet::doctor::Options {
            samples: args.samples as usize,
            ..podnet::doctor::Options::default()
        }
    };
    let ssh_relay = match (args.ssh_relay, args.ssh_user) {
        (Some(address), Some(user)) => {
            podnet::relay::check_address(&address)
                .map_err(anyhow::Error::from)
                .map_err(|e| e.context("--ssh-relay"))
                .fault(podnet::exit::Fault::Usage)?;
            if let Some(proxy) = &args.connect_proxy {
                podnet::relay::check_address(proxy)
                    .map_err(anyhow::Error::from)
                    .map_err(|e| e.context("--connect-proxy"))
                    .fault(podnet::exit::Fault::Usage)?;
            }
            Some(podnet::doctor::SshRelayTarget::of_flags(
                address,
                user,
                args.connect_proxy,
            ))
        }
        _ => None,
    };
    // A bad list exits 64 before the network.
    let derp = match args.derp_server.as_deref() {
        Some(list) => Some(podnet::derp::servers_of(Some(list), None)?),
        None => None,
    };
    let report = podnet::doctor::run(podnet::doctor::Options {
        ssh_relay,
        derp,
        timeout: Duration::from_secs(args.timeout),
        relay: args.relay.filter(|r| !r.trim().is_empty()),
        ..base
    })
    .await;
    if args.json {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.text());
    }
    if !report.ok() {
        bail!("no leg can work on this host; the doctor gives the reason for each");
    }
    Ok(())
}

/// `podnet status`.
pub(crate) fn status(
    page: bool,
    out: Option<std::path::PathBuf>,
    check: Option<std::path::PathBuf>,
    root: Option<std::path::PathBuf>,
) -> Result<()> {
    if !page && out.is_none() && check.is_none() {
        println!("{}", podnet::status::document());
        return Ok(());
    }
    let root = match root {
        Some(root) => root,
        None => std::env::current_dir().context("the current directory")?,
    };
    let page_doc = podnet::page::Page::scan(&root)?;
    if let Some(path) = &check {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let stale = page_doc.check(&text);
        if !stale.is_empty() {
            for entry in &stale {
                eprintln!("podnet: {entry}");
            }
            return Err(podnet::exit::fault(
                Fault::Config,
                anyhow::anyhow!(
                    "{}: the page is stale at {} entr(y/ies); run podnet status --page --out {}",
                    path.display(),
                    stale.len(),
                    path.display()
                ),
            ));
        }
    }
    if page || out.is_some() {
        let text = page_doc.render();
        match &out {
            Some(path) => std::fs::write(path, &text)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?,
            None => print!("{text}"),
        }
    }
    Ok(())
}

/// `podnet bug-report`.
pub(crate) async fn bug_report(doctor: bool, include_link: bool) -> Result<()> {
    let report = if doctor {
        let options = podnet::doctor::Options {
            samples: 3,
            ..Default::default()
        };
        Some(podnet::doctor::run(options).await.to_json())
    } else {
        None
    };
    let link = if include_link {
        Some(podnet::link_ctl::dump_all().await)
    } else {
        None
    };
    print!(
        "{}",
        podnet::bug_report::text_with(report.as_ref(), link.as_deref())
    );
    Ok(())
}
