//! `podnet ssh-relay` (T-165, T-166): publish a unix socket through an SSH
//! relay, and keep the session. The endpoints, the user and the URL rule of
//! a known relay are data of `pod-services`; a flag overrides each of them.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Args;
use podnet::exit::{Fault, ResultExt};
use podnet::relay::facts::{self, Facts};
use podnet::relay::supervise::{supervise, Backoff, Event};
use podnet::relay::{RelayConfig, TunnelHandle, UrlRule};

use crate::cli::opts::DetachArgs;
use crate::cli::signals::shutdown_signal;

/// The interval of the keepalive request to the relay. A relay that does
/// not answer more than three requests in a row ends the session, and the
/// supervisor opens the next one.
const KEEPALIVE: Duration = Duration::from_secs(30);

#[derive(Args, Debug)]
pub(crate) struct SshRelayArgs {
    /// The known relay to use: its endpoints, its user and its URL rule
    /// come from the data of pod-services. Default: pinggy. The flags
    /// --server, --user, --url-match and --url-skip override these facts.
    /// An unknown name lists the known relays.
    #[arg(long, value_name = "NAME")]
    service: Option<String>,
    /// The SSH server of the relay: HOST:PORT. Use a port that the
    /// egress allows (443 for most proxies). Default: the plain SSH
    /// endpoints of the relay, tried in order. With --server and no
    /// --service, the relay is not a known one: give --user and a URL rule.
    #[arg(long, value_name = "HOST:PORT")]
    server: Option<String>,
    /// The SSH user of the relay. podnet logs in with a key made for the
    /// run, then with no key, then with an empty keyboard-interactive
    /// answer, as far as the relay offers them.
    /// Default: the user of the facts of the relay, else "podnet".
    #[arg(long, value_name = "USER")]
    user: Option<String>,
    /// Which URL of the banner of the relay is the tunnel URL: it holds
    /// this text (the tunnel domain of the relay). Repeatable: the URL
    /// holds each text. Replaces the texts of the facts.
    #[arg(long, value_name = "TEXT")]
    url_match: Vec<String>,
    /// Which URL of the banner is not the tunnel URL: it holds this text
    /// (the dashboard of the relay). Repeatable: the URL holds none of the
    /// texts. Replaces the texts of the facts, which are the dashboards of
    /// all known relays.
    #[arg(long, value_name = "TEXT")]
    url_skip: Vec<String>,
    /// The unix socket of the origin: the relay opens one channel for each
    /// visitor, and podnet connects it to this socket. The origin listens
    /// on the socket before the first visitor comes. Default: origin.sock
    /// in the private run directory of this user.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
    /// The port that the relay opens for the forward (1 to 65535). Default:
    /// the `remote-port` fact of the relay (80 for sandhole), else port 0,
    /// and the relay chooses. A relay can refuse port 0: the Sandhole demo
    /// needs 80. The flag replaces the fact.
    #[arg(long, value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
    remote_port: Option<u16>,
    /// Reach the relay through this HTTP CONNECT proxy: HOST:PORT. Default:
    /// a direct connection.
    #[arg(long, value_name = "HOST:PORT")]
    connect_proxy: Option<String>,
    /// Print each URL as one JSON line on stdout: {"event": "ready" or
    /// "reconnected", "url", "remote_port", "relay"}.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    detach: DetachArgs,
}

impl SshRelayArgs {
    pub(crate) fn detach(&self) -> &DetachArgs {
        &self.detach
    }

    /// The facts that the flags start from: those of the named relay, or of
    /// the default one. None: a `--server` with no `--service` is a relay
    /// that no data describes.
    fn facts(&self) -> Result<Option<Facts>> {
        if self.service.is_none() && self.server.is_some() {
            return Ok(None);
        }
        let name = self.service.as_deref().unwrap_or(facts::DEFAULT);
        let found = facts::lookup(name)
            .map_err(|e| anyhow::anyhow!("--service: {e}"))
            .fault(Fault::Usage)?;
        Ok(Some(found))
    }

    /// The configuration of the leg, or the usage error of a bad flag.
    fn config(&self) -> Result<RelayConfig> {
        let found = self.facts()?;
        let usage = |text: &str| Err(anyhow::anyhow!("{text}")).fault(Fault::Usage);
        if let Some(server) = &self.server {
            podnet::relay::check_address(server)
                .map_err(anyhow::Error::from)
                .map_err(|e| e.context("--server"))
                .fault(Fault::Usage)?;
        }
        if let Some(proxy) = &self.connect_proxy {
            podnet::relay::check_address(proxy)
                .map_err(anyhow::Error::from)
                .map_err(|e| e.context("--connect-proxy"))
                .fault(Fault::Usage)?;
        }
        for (flag, texts) in [
            ("--url-match", &self.url_match),
            ("--url-skip", &self.url_skip),
        ] {
            if texts.iter().any(|t| t.is_empty()) {
                return usage(&format!("{flag}: give a text, not an empty word"));
            }
        }
        let own_rule = !self.url_match.is_empty() || !self.url_skip.is_empty();
        if found.is_none() {
            if self.user.is_none() {
                return usage("--server with no --service needs --user");
            }
            if !own_rule {
                return usage("--server with no --service needs --url-match or --url-skip");
            }
        }
        let (mut addresses, mut user, mut rule, mut keepalive, mut remote_port) =
            (Vec::new(), None, UrlRule::default(), None, None);
        if let Some(found) = found {
            (addresses, user, rule, keepalive, remote_port) = (
                found.addresses,
                found.user,
                found.rule,
                found.keepalive,
                found.remote_port,
            );
        }
        if let Some(server) = &self.server {
            addresses = vec![server.clone()];
        }
        if !self.url_match.is_empty() {
            rule.matches = self.url_match.clone();
        }
        if !self.url_skip.is_empty() {
            rule.skips = self.url_skip.clone();
        }
        let mut addresses = addresses.into_iter();
        let Some(address) = addresses.next() else {
            return usage("the relay has no plain SSH endpoint: give --server");
        };
        let mut config = RelayConfig {
            address,
            fallbacks: addresses.collect(),
            proxy: self.connect_proxy.clone(),
            // The flag replaces the fact; with neither, port 0.
            remote_port: self.remote_port.or(remote_port),
            username: self
                .user
                .clone()
                .or(user)
                .unwrap_or_else(|| "podnet".to_string()),
            url_rule: rule,
            // The relay ends a session at its own time. The keepalive finds
            // a dead session; an idle session is not dead.
            inactivity: None,
            keepalive: Some(keepalive.unwrap_or(KEEPALIVE)),
            ..RelayConfig::default()
        };
        if let Some(socket) = &self.socket {
            config.unix_socket = socket.clone();
        }
        Ok(config)
    }
}

/// The ready document of a session: the URL, the remote port and the relay.
fn document(tunnel: &TunnelHandle, relay: &str) -> serde_json::Value {
    serde_json::json!({
        "url": tunnel.url,
        "remote_port": tunnel.remote_port,
        "relay": relay,
    })
}

/// Put the new URL into the ready document of the record (`podnet ps`).
/// The keys of the supervisor (name, pid) stay.
fn amend_record(record: &mut podnet::instances::Record, new: &serde_json::Value) {
    match record.ready.as_mut().and_then(|d| d.as_object_mut()) {
        Some(object) => {
            if let Some(new) = new.as_object() {
                for (key, value) in new {
                    object.insert(key.clone(), value.clone());
                }
            }
        }
        None => record.ready = Some(new.clone()),
    }
}

/// Run the leg until a stop signal.
pub(crate) async fn ssh_relay(args: SshRelayArgs) -> Result<()> {
    let config = args.config()?;
    let relay = config.address.clone();
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(supervise(config, Backoff::default(), events));
    let mut first = true;
    let result = loop {
        tokio::select! {
            event = received.recv() => match event {
                Some(Event::Up(tunnel)) => {
                    let doc = document(&tunnel, &relay);
                    if args.json {
                        let name = if first { "ready" } else { "reconnected" };
                        let mut line = doc.clone();
                        line["event"] = name.into();
                        println!("{line}");
                    } else {
                        println!(
                            "podnet: ssh-relay  {}  (relay {relay}, remote port {})",
                            tunnel.url, tunnel.remote_port
                        );
                    }
                    if first {
                        first = false;
                        podnet::detach::ready(doc);
                    } else {
                        podnet::detach::update_record(|record| amend_record(record, &doc));
                    }
                }
                Some(Event::Down { reason }) => {
                    println!("podnet: ssh-relay  down: {reason}; the supervisor connects again");
                }
                None => break Err(anyhow::anyhow!("the supervisor of the relay session ended"))
                    .fault(Fault::Unavailable),
            },
            _ = shutdown_signal() => break Ok(()),
        }
    };
    task.abort();
    result
}

#[cfg(test)]
mod tests {
    use crate::cli::{Cli, Command};
    use clap::Parser as _;

    fn parse(words: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("podnet").chain(words.iter().copied()))
    }

    const FULL: [&str; 9] = [
        "ssh-relay",
        "--server",
        "relay.example.test:443",
        "--user",
        "podnet",
        "--url-skip",
        "dashboard.example.test",
        "--url-match",
        ".tunnel.example.test",
    ];

    fn config(words: &[&str]) -> anyhow::Result<podnet::relay::RelayConfig> {
        let Command::SshRelay(args) = parse(words).unwrap().command else {
            panic!("not ssh-relay");
        };
        args.config()
    }

    #[test]
    fn ssh_relay_takes_the_relay_the_user_and_the_url_rule_as_flags() {
        let config = config(&FULL).unwrap();
        assert_eq!(config.address, "relay.example.test:443");
        assert!(config.fallbacks.is_empty());
        assert_eq!(config.username, "podnet");
        assert_eq!(config.url_rule.skips, ["dashboard.example.test"]);
        assert_eq!(config.url_rule.matches, [".tunnel.example.test"]);
        assert!(config.keepalive.is_some());
        assert_eq!(config.inactivity, None);
    }

    #[test]
    fn ssh_relay_with_no_flag_uses_the_facts_of_the_default_relay() {
        let config = config(&["ssh-relay"]).unwrap();
        let found = podnet::relay::facts::default_facts();
        assert_eq!(config.address, found.addresses[0]);
        assert_eq!(config.username, found.user.unwrap());
        assert_eq!(config.url_rule, found.rule);
        assert_eq!(config.keepalive, found.keepalive);
    }

    #[test]
    fn ssh_relay_takes_the_facts_of_a_named_relay_and_the_address_list() {
        let config = config(&["ssh-relay", "--service", "serveo"]).unwrap();
        let found = podnet::relay::facts::lookup("serveo").unwrap();
        assert_eq!(config.address, found.addresses[0]);
        assert_eq!(config.fallbacks, found.addresses[1..]);
        assert!(!config.fallbacks.is_empty());
        assert_eq!(config.url_rule, found.rule);
    }

    #[test]
    fn a_flag_overrides_the_facts_of_the_relay() {
        let config = config(&[
            "ssh-relay",
            "--service",
            "serveo",
            "--server",
            "other.example.test:2222",
            "--user",
            "me",
            "--url-match",
            ".mine.test",
        ])
        .unwrap();
        let found = podnet::relay::facts::lookup("serveo").unwrap();
        assert_eq!(config.address, "other.example.test:2222");
        assert!(config.fallbacks.is_empty());
        assert_eq!(config.username, "me");
        assert_eq!(config.url_rule.matches, [".mine.test"]);
        assert_eq!(config.url_rule.skips, found.rule.skips);
    }

    #[test]
    fn ssh_relay_refuses_an_unknown_relay_and_lists_the_known_ones() {
        let err = config(&["ssh-relay", "--service", "nope"]).unwrap_err();
        assert_eq!(podnet::exit::code_of(&err), podnet::exit::USAGE);
        let text = format!("{err:#}");
        assert!(text.contains("nope") && text.contains("pinggy"), "{text}");
    }

    #[test]
    fn ssh_relay_refuses_a_server_of_no_known_relay_without_user_and_rule() {
        // The flag and its value go; the other flags stay.
        for (range, name) in [(3..5, "--user"), (5..9, "--url")] {
            let mut words = FULL.to_vec();
            words.drain(range);
            let err = config(&words).unwrap_err();
            assert_eq!(podnet::exit::code_of(&err), podnet::exit::USAGE);
            assert!(format!("{err:#}").contains(name), "{name}: {err:#}");
        }
    }

    #[test]
    fn ssh_relay_refuses_a_bad_address() {
        let mut words = FULL.to_vec();
        words[2] = "no-port";
        let err = config(&words).unwrap_err();
        assert_eq!(podnet::exit::code_of(&err), podnet::exit::USAGE);
        assert!(format!("{err:#}").contains("--server"), "{err:#}");
    }

    #[test]
    fn ssh_relay_remote_port_reaches_the_config() {
        assert_eq!(config(&["ssh-relay"]).unwrap().remote_port, None);
        let with = config(&["ssh-relay", "--remote-port", "80"]).unwrap();
        assert_eq!(with.remote_port, Some(80));
        let high = config(&["ssh-relay", "--remote-port=65535"]).unwrap();
        assert_eq!(high.remote_port, Some(65535));
    }

    #[test]
    fn ssh_relay_takes_the_remote_port_fact_and_the_flag_overrides_it() {
        // Sandhole refuses port 0: its fact is port 80.
        let fact = config(&["ssh-relay", "--service", "sandhole"]).unwrap();
        assert_eq!(fact.remote_port, Some(80));
        let flag = config(&[
            "ssh-relay",
            "--service",
            "sandhole",
            "--remote-port",
            "8080",
        ])
        .unwrap();
        assert_eq!(flag.remote_port, Some(8080));
        // A relay with no fact keeps port 0.
        for words in [
            &["ssh-relay", "--service", "pinggy"][..],
            &["ssh-relay"],
            &FULL,
        ] {
            assert_eq!(config(words).unwrap().remote_port, None, "{words:?}");
        }
    }

    #[test]
    fn ssh_relay_remote_port_is_one_to_65535() {
        for port in ["0", "70000", "-1", "http"] {
            let err = parse(&["ssh-relay", &format!("--remote-port={port}")]).unwrap_err();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{port}"
            );
        }
        let text = parse(&["ssh-relay", "--remote-port", "0"])
            .unwrap_err()
            .to_string();
        assert!(text.contains("1..=65535"), "{text}");
    }

    #[test]
    fn a_new_url_goes_into_the_ready_document_and_keeps_the_name() {
        let mut record = podnet::instances::Record {
            name: "x".into(),
            pid: 1,
            supervisor: None,
            started: 0,
            command: "ssh-relay".into(),
            args: vec![],
            hash: String::new(),
            cwd: String::new(),
            state: "ready".into(),
            ready: Some(serde_json::json!({"url": "https://one", "name": "x", "pid": 7})),
            log: String::new(),
            restarts: 0,
            retry_after_s: None,
        };
        super::amend_record(&mut record, &serde_json::json!({"url": "https://two"}));
        let ready = record.ready.unwrap();
        assert_eq!(ready["url"], "https://two");
        assert_eq!(ready["name"], "x");
        assert_eq!(ready["pid"], 7);
    }
}
