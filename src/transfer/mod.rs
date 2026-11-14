//! The file transfer of magic-wormhole: `podnet send` and `podnet receive`
//! (T-102, T-103, T-105; `docs/transfer.md`).
//!
//! * The rendezvous is `pod-wormhole` (PodUtils/podcrates): the mailbox
//!   client, the code of words and SPAKE2, and the sealed phases. The
//!   mailboxes are the services of the kind `mailbox` of `pod-services`, in
//!   their order, or `--mailbox` and `PODNET_MAILBOX`. The giver takes the
//!   first mailbox that answers; the taker finds the giver's mailbox by its
//!   list of nameplates (`pod_wormhole::joined`).
//! * The file-transfer protocol of magic-wormhole over the wormhole is
//!   [`messages`]; its transit (the hints, the handshakes and the records
//!   sealed with SecretBox) is [`transit`] and [`connect`].
//! * A piping server is the last way of the transit between two podnet
//!   ends ([`mod@piping`], T-252): the servers of the kind `piping` of
//!   `pod-services`, or `--piping` and `PODNET_PIPING`.
//! * A directory goes as a zip ([`zip`]); the receiver checks each name
//!   before it writes one ([`land`]).
//!
//! The transit key is magic-wormhole's: `derive_key` of the key of the
//! wormhole for the purpose `APPID/transit-key` ([`transit_key`]). A text,
//! a file and a directory go to and from each client of magic-wormhole.

pub mod connect;
pub mod land;
pub mod messages;
pub mod piping;
pub mod podnet_relay;
mod receive;
mod send;
pub mod transit;
pub mod zip;

use std::time::Duration;

use pod_wormhole::{Ends, Failed};
use pod_ws::url::WsUrl;
use serde_json::{json, Value};

use crate::exit::{self, Fault};
pub use connect::{Listen, Relay};
pub use podnet_relay::RelayPair;
pub use receive::{receive, Offered, Receive, Received};
pub use send::{open, Opened, Sent, What};

/// The AppID of magic-wormhole's file transfer: a client of magic-wormhole
/// and podnet meet only at it.
pub const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";
/// The variable of the mailboxes: URLs, separated by commas.
pub const MAILBOX_VARIABLE: &str = "PODNET_MAILBOX";
/// The kind of the mailboxes and of the transit relays in `pod-services`.
pub const MAILBOX_KIND: &str = "mailbox";
pub const TRANSIT_KIND: &str = "transit";
/// The kind of the piping servers in `pod-services`.
pub const PIPING_KIND: &str = "piping";
/// The variable of the piping servers: URLs, separated by commas.
pub const PIPING_VARIABLE: &str = "PODNET_PIPING";
/// How long a sender waits for a receiver to take its code, by default.
pub const WAIT: Duration = Duration::from_secs(30 * 60);
/// The bound of one step of the exchange, by default.
pub const STEP: Duration = Duration::from_secs(60);
/// The bound of the connection of the transit, by default.
pub const TRANSIT_WAIT: Duration = Duration::from_secs(120);
/// How long a receiver waits for the giver's first message at a mailbox
/// that lists no nameplate, by default: a giver waits there already.
pub const GIVER_WAIT: Duration = Duration::from_secs(10);

/// How a transfer meets and connects.
pub struct Settings {
    /// The mailboxes, in order, and the settings of the wormhole.
    pub ends: Ends,
    /// The transit relays of this end.
    pub relays: Vec<Relay>,
    /// Where this end listens for a direct connection.
    pub listen: Listen,
    /// How long a sender waits for its receiver, and for the answer of a
    /// person at the other end.
    pub wait: Duration,
    /// The bound of each other step of the exchange.
    pub step: Duration,
    /// The bound of the connection of the transit.
    pub transit_wait: Duration,
    /// How long a receiver waits for the giver at each mailbox.
    pub giver_wait: Duration,
    /// A sender: the stored pair of the podnet relay whose node it offers
    /// to a podnet receiver (T-104).
    pub relay_pair: Option<RelayPair>,
    /// A receiver: the moves of the session of the layer over the podnet
    /// relay.
    pub relay_resume: crate::relay_reverse::Resume,
    /// The piping servers of this end (URLs), in order: the last way of
    /// the transit between two podnet ends (T-252). Empty: no piping
    /// server.
    pub piping: Vec<String>,
}

impl Settings {
    /// The settings for `mailboxes` and `relays`, through `way`, with the
    /// default bounds and a listener on each address.
    pub fn new(mailboxes: Vec<WsUrl>, relays: Vec<Relay>, way: pod_ws::Way) -> Settings {
        let config = pod_wormhole::Config::new(APPID, "podnet", env!("CARGO_PKG_VERSION"), way);
        Settings {
            ends: Ends { mailboxes, config },
            relays,
            listen: Listen::Any,
            wait: WAIT,
            step: STEP,
            transit_wait: TRANSIT_WAIT,
            giver_wait: GIVER_WAIT,
            relay_pair: None,
            relay_resume: crate::relay_reverse::Resume::default(),
            piping: Vec::new(),
        }
    }

    fn transit_way(&self) -> connect::Way {
        connect::Way {
            way: self.ends.config.way.clone(),
            trust: self.ends.config.trust.clone(),
        }
    }
}

/// The mailboxes of `pod-services`, in their order.
pub fn default_mailboxes() -> Vec<WsUrl> {
    let urls = pod_services::of_kind(MAILBOX_KIND)
        .filter_map(|s| s.endpoint("mailbox"))
        .map(|e| e.url());
    urls.filter_map(|url| WsUrl::parse(&url).ok()).collect()
}

/// The transit relays of `pod-services`, in their order.
pub fn default_relays() -> Vec<Relay> {
    let urls = pod_services::of_kind(TRANSIT_KIND)
        .filter_map(|s| s.endpoint("relay"))
        .map(|e| e.url());
    urls.filter_map(|url| Relay::parse(&url).ok()).collect()
}

/// The piping servers of `pod-services`, in their order.
pub fn default_piping() -> Vec<String> {
    pod_services::of_kind(PIPING_KIND)
        .filter_map(|s| s.endpoint("server"))
        .map(|e| e.url())
        .collect()
}

/// The piping servers: the given URLs (flags or the variable), else
/// [`default_piping`]. `none` alone gives no piping server.
pub fn piping(given: &[String]) -> Result<Vec<String>, String> {
    let given: Vec<&str> = given
        .iter()
        .map(|u| u.trim())
        .filter(|u| !u.is_empty())
        .collect();
    match given.as_slice() {
        [] => Ok(default_piping()),
        ["none"] => Ok(Vec::new()),
        urls => urls
            .iter()
            .map(|u| {
                let ok = (u.starts_with("https://") || u.starts_with("http://"))
                    && !u.contains(['?', '#', ' '])
                    && u.split("://").nth(1).is_some_and(|rest| !rest.is_empty());
                ok.then(|| u.trim_end_matches('/').to_string())
                    .ok_or_else(|| {
                        format!(
                            "piping server {u:?}: give https://HOST[:PORT][/PATH] or http://..."
                        )
                    })
            })
            .collect(),
    }
}

/// The mailboxes: the flags, else the variable (URLs separated by commas),
/// else [`default_mailboxes`].
pub fn mailboxes(flags: &[String], variable: Option<&str>) -> Result<Vec<WsUrl>, String> {
    let given: Vec<&str> = if flags.is_empty() {
        variable
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        flags
            .iter()
            .map(|u| u.trim())
            .filter(|u| !u.is_empty())
            .collect()
    };
    if given.is_empty() {
        return Ok(default_mailboxes());
    }
    given
        .iter()
        .map(|u| WsUrl::parse(u).map_err(|e| format!("mailbox {u:?}: {e}")))
        .collect()
}

/// What this end says in its `version`: podnet, and the form of its
/// transit key (2: the key of magic-wormhole).
pub fn versions() -> Value {
    json!({ "podnet": { "version": env!("CARGO_PKG_VERSION"), "transit-key": 2 } })
}

/// True when the other end is a podnet of the form 1 of the transit key
/// (podnet 0.4.0-beta.10 and older: a random key in the `transit`
/// message), which cannot move a file with this end.
fn old_podnet(theirs: &Value) -> bool {
    theirs["podnet"]["transit-key"].as_u64() == Some(1)
}

/// The words of a file transfer with an older podnet.
const OLD_PODNET: &str =
    "the other end is an older podnet (0.4.0-beta.10 or before) with its own transit key: \
     update podnet at both ends to move a file or a directory; a text works";

/// The purpose of the transit key, as magic-wormhole's file transfer names it.
pub const TRANSIT_PURPOSE: &str = "lothar.com/wormhole/text-or-file-xfer/transit-key";

/// The transit key of magic-wormhole: `derive_key` of the key of the
/// wormhole for [`TRANSIT_PURPOSE`], 32 bytes.
pub fn transit_key(hole: &pod_wormhole::Wormhole) -> Result<[u8; 32], Failed> {
    let key = hole.derive_key(TRANSIT_PURPOSE, 32)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&key);
    Ok(out)
}

/// A failure of the wormhole with its exit class.
pub fn failed(failed: Failed) -> anyhow::Error {
    let class = match &failed {
        Failed::Unreachable(_) | Failed::Broken(_) | Failed::Nobody => Some(Fault::Unavailable),
        Failed::Refused(_) | Failed::WrongCode => Some(Fault::NoPermission),
        Failed::Expired(_) => Some(Fault::Timeout),
        Failed::NoRandom => Some(Fault::OsError),
        Failed::Peer(_) | Failed::Mine(_) => None,
    };
    let err = anyhow::Error::new(failed);
    match class {
        Some(class) => exit::fault(class, err),
        None => err,
    }
}

/// Input that the receiver refuses, with its exit class (65).
pub fn refused(why: impl std::fmt::Display) -> anyhow::Error {
    exit::fault(Fault::DataError, anyhow::anyhow!("{why}"))
}

/// A path to the other end that did not work (69).
fn unavailable(why: impl std::fmt::Display) -> anyhow::Error {
    exit::fault(Fault::Unavailable, anyhow::anyhow!("{why}"))
}

/// `text` for a terminal (T-347): each control character as its escape
/// (`\u{1b}`, `\r`), so that a text or a name of the other end cannot send
/// a control sequence to the terminal. With `lines`, a line feed and a tab
/// stay.
pub fn for_terminal(text: &str, lines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && !(lines && (c == '\n' || c == '\t')) {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_for_terminal_escapes_each_control_character() {
        let text = "ok\x1b]0;title\x07\x1b[2Jline\r\nnext\tcol\u{9b}31m\x7f";
        let shown = for_terminal(text, true);
        assert_eq!(
            shown,
            "ok\\u{1b}]0;title\\u{7}\\u{1b}[2Jline\\r\nnext\tcol\\u{9b}31m\\u{7f}"
        );
        assert!(!shown
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t'));
        let name = for_terminal("a\nb\tc", false);
        assert_eq!(name, "a\\nb\\tc");
        assert_eq!(
            for_terminal("plain text, ünïcode", true),
            "plain text, ünïcode"
        );
    }

    #[test]
    fn wormhole_rendezvous_the_mailboxes_come_from_the_flags_the_variable_or_pod_services() {
        let defaults = mailboxes(&[], None).unwrap();
        assert!(!defaults.is_empty(), "pod-services names a mailbox");
        assert!(
            defaults.iter().all(|u| u.tls),
            "each public mailbox is wss://"
        );
        let from_variable =
            mailboxes(&[], Some("ws://127.0.0.1:4000/v1, ws://127.0.0.1:4001/v1")).unwrap();
        assert_eq!(from_variable.len(), 2);
        let flags = vec!["ws://127.0.0.1:5000/v1".to_string()];
        let from_flags = mailboxes(&flags, Some("ws://127.0.0.1:4000/v1")).unwrap();
        assert_eq!(from_flags[0].port, 5000);
        assert!(mailboxes(&["http://x".to_string()], None).is_err());
        let servers = piping(&[]).unwrap();
        assert!(!servers.is_empty(), "pod-services names a piping server");
        assert!(servers.iter().all(|u| u.starts_with("https://")));
        assert!(piping(&["none".to_string()]).unwrap().is_empty());
        assert_eq!(
            piping(&["http://127.0.0.1:9/".to_string()]).unwrap(),
            ["http://127.0.0.1:9"]
        );
        assert!(piping(&["ftp://x".to_string()]).is_err());
        assert!(piping(&["https://".to_string()]).is_err());
        // T-251: each transit of pod-services is a relay, a TCP one
        // (`tcp://HOST:PORT`) among them, with a host and no slash.
        let relays = default_relays();
        let transits = pod_services::of_kind(TRANSIT_KIND)
            .filter(|s| s.endpoint("relay").is_some())
            .count();
        assert_eq!(relays.len(), transits);
        assert!(relays.iter().any(|r| matches!(r, Relay::Tcp { .. })));
        for relay in &relays {
            if let Relay::Tcp { host, port } = relay {
                assert!(
                    !host.is_empty() && !host.contains('/') && *port > 0,
                    "{host}"
                );
            }
        }
        assert!(matches!(
            Relay::parse("tcp://127.0.0.1:4001"),
            Ok(Relay::Tcp { port: 4001, .. })
        ));
        assert!(Relay::parse("tcp:///x:1").is_err());
    }

    #[test]
    fn wormhole_rendezvous_a_failure_has_the_exit_code_of_podnet() {
        let code = |f: Failed| exit::code_of(&failed(f));
        assert_eq!(code(Failed::WrongCode), exit::NO_PERM);
        assert_eq!(code(Failed::Nobody), exit::UNAVAILABLE);
        assert_eq!(
            code(Failed::Expired(Duration::from_secs(60))),
            exit::TIMEOUT
        );
        assert_eq!(code(Failed::Peer("x".into())), exit::FAILURE);
        assert_eq!(exit::code_of(&refused("x")), exit::DATA_ERROR);
        assert!(!old_podnet(&versions()));
        assert!(!old_podnet(&json!({})));
        assert!(old_podnet(
            &json!({ "podnet": { "version": "0.4.0-beta.10", "transit-key": 1 } })
        ));
        assert_eq!(TRANSIT_PURPOSE, format!("{APPID}/transit-key"));
    }
}
