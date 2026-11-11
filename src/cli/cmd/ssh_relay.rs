//! `podnet ssh-relay` (T-165): publish a unix socket through an SSH relay,
//! and keep the session. The relay, the user and the URL rule are flags
//! until `pod-services` holds them (T-166).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{ArgGroup, Args};
use podnet::exit::{Fault, ResultExt};
use podnet::relay::supervise::{supervise, Backoff, Event};
use podnet::relay::{RelayConfig, TunnelHandle, UrlRule};

use crate::cli::opts::DetachArgs;
use crate::cli::signals::shutdown_signal;

/// The interval of the keepalive request to the relay. A relay that does
/// not answer more than three requests in a row ends the session, and the
/// supervisor opens the next one.
const KEEPALIVE: Duration = Duration::from_secs(30);

#[derive(Args, Debug)]
#[command(group(ArgGroup::new("url_rule").args(["url_match", "url_skip"]).required(true).multiple(true)))]
pub(crate) struct SshRelayArgs {
    /// The SSH server of the relay: HOST:PORT. Use a port that the
    /// egress allows (443 for most proxies).
    #[arg(long, value_name = "HOST:PORT", required = true)]
    server: String,
    /// The SSH user of the relay. The relay takes an ephemeral key.
    #[arg(long, value_name = "USER", required = true)]
    user: String,
    /// Which URL of the banner of the relay is the tunnel URL: it holds
    /// this text (the tunnel domain of the relay). Repeatable: the URL
    /// holds each text. Give --url-match or --url-skip, or both.
    #[arg(long, value_name = "TEXT")]
    url_match: Vec<String>,
    /// Which URL of the banner is not the tunnel URL: it holds this text
    /// (the dashboard of the relay). Repeatable: the URL holds none of the
    /// texts.
    #[arg(long, value_name = "TEXT")]
    url_skip: Vec<String>,
    /// The unix socket of the origin: the relay opens one channel for each
    /// visitor, and podnet connects it to this socket. The origin listens
    /// on the socket before the first visitor comes. Default: origin.sock
    /// in the private run directory of this user.
    #[arg(long, value_name = "PATH")]
    socket: Option<PathBuf>,
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

    /// The configuration of the leg, or the usage error of a bad flag.
    fn config(&self) -> Result<RelayConfig> {
        podnet::relay::check_address(&self.server)
            .map_err(anyhow::Error::from)
            .map_err(|e| e.context("--server"))
            .fault(Fault::Usage)?;
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
                return Err(anyhow::anyhow!("{flag}: give a text, not an empty word"))
                    .fault(Fault::Usage);
            }
        }
        let mut config = RelayConfig {
            address: self.server.clone(),
            proxy: self.connect_proxy.clone(),
            username: self.user.clone(),
            url_rule: UrlRule {
                skip: self.url_skip.clone(),
                hold: self.url_match.clone(),
            },
            // The relay ends a session at its own time. The keepalive finds
            // a dead session; an idle session is not dead.
            inactivity: None,
            keepalive: Some(KEEPALIVE),
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

    #[test]
    fn ssh_relay_takes_the_relay_the_user_and_the_url_rule_as_flags() {
        let cli = parse(&FULL).unwrap();
        let Command::SshRelay(args) = cli.command else {
            panic!("not ssh-relay");
        };
        let config = args.config().unwrap();
        assert_eq!(config.address, "relay.example.test:443");
        assert_eq!(config.username, "podnet");
        assert_eq!(config.url_rule.skip, ["dashboard.example.test"]);
        assert_eq!(config.url_rule.hold, [".tunnel.example.test"]);
        assert!(config.keepalive.is_some());
        assert_eq!(config.inactivity, None);
    }

    #[test]
    fn ssh_relay_refuses_a_missing_flag_and_names_it() {
        // The flag and its value go; the other flags stay.
        for (at, name) in [(1, "--server"), (3, "--user")] {
            let mut words = FULL.to_vec();
            words.drain(at..at + 2);
            let err = parse(&words).unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
            assert!(err.to_string().contains(name), "{name}: {err}");
        }
        // No URL rule: neither --url-skip nor --url-match.
        let err = parse(&FULL[..5]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        let text = err.to_string();
        assert!(
            text.contains("--url-skip") && text.contains("--url-match"),
            "{text}"
        );
    }

    #[test]
    fn ssh_relay_refuses_a_bad_address() {
        let mut words = FULL.to_vec();
        words[2] = "no-port";
        let Command::SshRelay(args) = parse(&words).unwrap().command else {
            panic!("not ssh-relay");
        };
        let err = args.config().unwrap_err();
        assert_eq!(podnet::exit::code_of(&err), podnet::exit::USAGE);
        assert!(format!("{err:#}").contains("--server"), "{err:#}");
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
