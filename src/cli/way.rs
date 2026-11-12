//! `--egress SPEC` (T-060): one spelling of a network for each command. A spec
//! is a spec of the registry of networks (`podnet::networks`). `--egress` is
//! an alias: before the parse, it becomes the old flag, which keeps its
//! behaviour and its help:
//!
//! * `direct`: `--no-proxy`;
//! * a proxy of the process (`http://`, `socks5h://`, `SCHEME+unix:/PATH`,
//!   `SCHEME+exec:COMMAND`): `--proxy SPEC`;
//! * `relay`, `relay:URL`: `--transport relay` (and `--relay URL`);
//! * with `podnet pod`, each `--egress` is one `-x SPEC` of the chain, in its
//!   order. The pod parser takes `--egress` after `pod` as `-x`.
//!
//! A spec that the command cannot use is a usage error with one line.

use clap::CommandFactory as _;
use podnet::network::Network as _;
use podnet::networks::{self, SpecNetwork};

use crate::cli::Cli;

/// The flags that `--egress` stands for: a `--egress` beside one of them is a
/// usage error, as the two name one way.
const OLD_FLAGS: &[&str] = &["--proxy", "--no-proxy", "--transport", "--relay"];

/// The arguments with each `--egress` before `--` replaced by its old flag
/// (see the module). An error is one line for a usage error.
pub(crate) fn egress_aliases(
    args: Vec<std::ffi::OsString>,
) -> Result<Vec<std::ffi::OsString>, String> {
    let Some(words) = args
        .iter()
        .map(|a| a.to_str().map(str::to_string))
        .collect::<Option<Vec<String>>>()
    else {
        return Ok(args);
    };
    let out = rewrite(&words)?;
    Ok(out.into_iter().map(Into::into).collect())
}

/// [`egress_aliases`] on text.
pub(crate) fn rewrite(words: &[String]) -> Result<Vec<String>, String> {
    let end = words.iter().position(|w| w == "--").unwrap_or(words.len());
    let command = crate::cli::argv::command_name(words);
    let pod_at = match command.as_deref() {
        Some("pod") => words[..end].iter().position(|w| w == "pod"),
        _ => None,
    };
    // A --egress after the word pod is the pod's own option.
    let scan = pod_at.unwrap_or(end);
    let mut specs = Vec::new();
    let mut kept = Vec::new();
    // The place of the first --egress in `kept`: the old flag goes there.
    let mut place = None;
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if i < scan && (word == "--egress" || word.starts_with("--egress=")) {
            place.get_or_insert(kept.len());
        }
        if i < scan {
            if word == "--egress" {
                let spec = words.get(i + 1).filter(|_| i + 1 < scan);
                let spec = spec
                    .ok_or("--egress needs a value: a network spec (direct, a proxy URL, relay)")?;
                specs.push(spec.clone());
                i += 2;
                continue;
            }
            if let Some(spec) = word.strip_prefix("--egress=") {
                specs.push(spec.to_string());
                i += 1;
                continue;
            }
        }
        kept.push(word.clone());
        i += 1;
    }
    if specs.is_empty() {
        return Ok(words.to_vec());
    }
    let networks = specs
        .iter()
        .map(|s| networks::parse(s).map_err(|e| format!("--egress: {}", e.text())))
        .collect::<Result<Vec<_>, _>>()?;
    match pod_at {
        Some(at) => pod_hops(kept, at - removed_before(words, at), &specs, &networks),
        None => process_way(
            kept,
            place.unwrap_or(1),
            &specs,
            &networks,
            command.as_deref(),
        ),
    }
}

/// The line for `--egress wg:FILE` where the command cannot start the tunnel.
fn wg_refusal(label: &str) -> String {
    format!(
        "--egress {label}: this command does not start a WireGuard tunnel; use podnet forward --network {label}, or a networks: entry of podnet route and give its door to the command"
    )
}

/// The count of the words of `--egress` before `at`.
fn removed_before(words: &[String], at: usize) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < at {
        if words[i] == "--egress" {
            n += 2;
            i += 2;
        } else {
            n += usize::from(words[i].starts_with("--egress="));
            i += 1;
        }
    }
    n
}

/// `podnet --egress A --egress B pod ...`: `-x A -x B` after the word pod.
fn pod_hops(
    mut kept: Vec<String>,
    pod_at: usize,
    specs: &[String],
    networks: &[SpecNetwork],
) -> Result<Vec<String>, String> {
    let mut hops = Vec::new();
    for (spec, network) in specs.iter().zip(networks) {
        if network.relay_url().is_some() {
            return Err(format!(
                "--egress {}: the relay is not a hop of podnet pod; give a proxy, direct or tailscale:/PATH",
                network.label()
            ));
        }
        if network.wg_file().is_some() {
            return Err(wg_refusal(&network.label()));
        }
        if let Some(port) = network.netbird_port() {
            // The listener of NetBird netstack mode is a SOCKS5 hop.
            hops.push("-x".to_string());
            hops.push(format!("127.0.0.1:{port}"));
            continue;
        }
        if let Err(e) = podnet_netns::upstream::Proxy::parse(spec) {
            return Err(format!(
                "--egress {}: podnet pod cannot take it: {e:#}",
                network.label()
            ));
        }
        hops.push("-x".to_string());
        hops.push(spec.clone());
    }
    kept.splice(pod_at + 1..pod_at + 1, hops);
    Ok(kept)
}

/// One `--egress` of a command other than pod: its old flag.
fn process_way(
    mut kept: Vec<String>,
    place: usize,
    specs: &[String],
    networks: &[SpecNetwork],
    command: Option<&str>,
) -> Result<Vec<String>, String> {
    if specs.len() > 1 {
        return Err(
            "--egress: a chain (more than one --egress) is for podnet pod only; give one --egress"
                .into(),
        );
    }
    let end = kept.iter().position(|w| w == "--").unwrap_or(kept.len());
    if let Some(old) = kept[..end].iter().find_map(|w| {
        OLD_FLAGS
            .iter()
            .find(|f| w == *f || w.starts_with(&format!("{f}=")))
    }) {
        return Err(format!("--egress and {old} name one way: give one of them"));
    }
    let (spec, network) = (&specs[0], &networks[0]);
    if network.wg_file().is_some() {
        return Err(wg_refusal(&network.label()));
    }
    let flags: Vec<String> = match (network.scheme(), network.relay_url()) {
        ("direct", _) => vec!["--no-proxy".into()],
        (_, Some(url)) => {
            if !takes(command, "transport") {
                return Err(format!(
                    "--egress {}: the relay carries the tunnel of the edge only (tunnel, serve, share, agent up); {} has no relay way",
                    network.label(),
                    command.unwrap_or("this command")
                ));
            }
            let mut flags = vec!["--transport".to_string(), "relay".to_string()];
            if let Some(url) = url {
                flags.push("--relay".into());
                flags.push(url.to_string());
            }
            flags
        }
        _ if network.is_process_proxy() => vec!["--proxy".into(), spec.clone()],
        _ => {
            return Err(format!(
                "--egress {}: a hop of podnet pod (-x); the egress of the process takes direct, a proxy URL or the relay",
                network.label()
            ))
        }
    };
    // The old flag goes in the place of --egress. --transport and --relay are
    // options of the tunnel commands: they go after the command words.
    let place = match network.relay_url() {
        Some(_) => place.max(after_command(&kept, command)),
        None => place,
    };
    kept.splice(place..place, flags);
    Ok(kept)
}

/// The index after the last word of `command` in `words` (1 when there is
/// no command).
fn after_command(words: &[String], command: Option<&str>) -> usize {
    let mut at = 1;
    for word in command.unwrap_or("").split_whitespace() {
        match words[at..].iter().position(|w| w == word) {
            Some(i) => at += i + 1,
            None => break,
        }
    }
    at
}

/// True when the command `command` (words with spaces) or a command above
/// it has the long option `long`.
fn takes(command: Option<&str>, long: &str) -> bool {
    let mut cmd = Cli::command();
    let has = |c: &clap::Command| c.get_arguments().any(|a| a.get_long() == Some(long));
    if has(&cmd) {
        return true;
    }
    for word in command.unwrap_or("").split_whitespace() {
        let Some(sub) = cmd.find_subcommand(word).cloned() else {
            return false;
        };
        cmd = sub;
        if has(&cmd) {
            return true;
        }
    }
    false
}
