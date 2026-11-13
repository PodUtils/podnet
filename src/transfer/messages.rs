//! The messages of the file-transfer protocol of magic-wormhole
//! (magic-wormhole/magic-wormhole-protocols:file-transfer-protocol.md and
//! transit.md): each one is a JSON object with one key, sealed in one
//! numbered phase of the wormhole. The sender says `transit` (its abilities
//! and hints) and `offer` (a text, a file or a directory); the receiver says
//! `transit` and `answer`, or `error`. The ack of the data comes over the
//! transit, as one record.

use serde_json::{json, Map, Value};

/// The kinds of hint that this end can use.
pub const DIRECT: &str = "direct-tcp-v1";
pub const RELAY: &str = "relay-v1";
/// The kind of a WebSocket end of a relay.
pub const WEBSOCKET: &str = "websocket-v1";
/// podnet's own kind of hint (T-104): the node of a pair of the podnet
/// relay. A client of magic-wormhole ignores a kind that it does not know.
pub const PODNET_RELAY: &str = "podnet-relay-v1";
/// The key of the connect token of that node in the `transit` message of a
/// podnet sender (the message is sealed in its phase of the wormhole).
pub const PODNET_RELAY_TOKEN: &str = "podnet-relay-token";
/// podnet's kind of hint of a piping server (T-252; the words of
/// PodUtils/podnet#452): the last way of the transit, between two podnet
/// ends.
pub const PIPING: &str = "piping-v1";
/// The mode of a directory, as magic-wormhole names it. A zip with stored
/// entries is a valid zip of this mode.
pub const ZIP_MODE: &str = "zipfile/deflated";
/// The key of podnet's transit key in its `transit` message, until
/// `pod-wormhole` derives keys (`docs/transfer.md`, The transit key).
pub const PODNET_KEY: &str = "podnet-transit-key-v1";

/// What a sender offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// A text, which goes in the offer itself: no transit.
    Message(String),
    /// A file of `size` bytes.
    File { name: String, size: u64 },
    /// A directory as a zip of `zipsize` bytes, with `numfiles` entries and
    /// `numbytes` bytes when unpacked (the sender's estimate).
    Directory {
        mode: String,
        name: String,
        zipsize: u64,
        numbytes: u64,
        numfiles: u64,
    },
}

/// One end of a transit relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayEnd {
    Tcp { host: String, port: u16 },
    WebSocket { url: String },
}

/// Where the other end can be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// A TCP listener of the other end.
    Direct { host: String, port: u16 },
    /// A transit relay, by one or more of its ends.
    Relay {
        name: Option<String>,
        ends: Vec<RelayEnd>,
    },
    /// podnet's own kind: the node of the pair `name` at the podnet relay
    /// `url` (the reverse road). The connect token comes in
    /// [`Transit::relay_token`].
    PodnetRelay { url: String, name: String },
    /// podnet's own kind: a piping server at `url` ([`mod@super::piping`]).
    Piping { url: String },
}

/// The `transit` message: the abilities and the hints of one end.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transit {
    pub abilities: Vec<String>,
    pub hints: Vec<Hint>,
    /// podnet's transit key, from a podnet sender.
    pub key: Option<[u8; 32]>,
    /// The connect token of the node of a [`Hint::PodnetRelay`], from a
    /// podnet sender.
    pub relay_token: Option<String>,
}

/// The answer of a receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    MessageAck,
    FileAck,
    /// An answer that is not `ok`: its words.
    Other(String),
}

/// One message of the other end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Transit(Transit),
    Offer(Offer),
    Answer(Answer),
    Error(String),
    /// A message with no key that this end knows: ignored.
    Other,
}

impl Offer {
    /// The `offer` message.
    pub fn encode(&self) -> Vec<u8> {
        let offer = match self {
            Offer::Message(text) => json!({ "message": text }),
            Offer::File { name, size } => json!({ "file": { "filename": name, "filesize": size } }),
            Offer::Directory {
                mode,
                name,
                zipsize,
                numbytes,
                numfiles,
            } => json!({ "directory": {
                "mode": mode, "dirname": name, "zipsize": zipsize, "numbytes": numbytes, "numfiles": numfiles,
            } }),
        };
        json!({ "offer": offer }).to_string().into_bytes()
    }

    fn parse(value: &Value) -> Result<Offer, String> {
        if let Some(text) = value.get("message") {
            let text = text
                .as_str()
                .ok_or("the text of the offer is not a string")?;
            return Ok(Offer::Message(text.to_string()));
        }
        if let Some(file) = value.get("file") {
            let name = text_of(file, "filename")?;
            return Ok(Offer::File {
                name,
                size: number_of(file, "filesize")?,
            });
        }
        if let Some(dir) = value.get("directory") {
            return Ok(Offer::Directory {
                mode: text_of(dir, "mode")?,
                name: text_of(dir, "dirname")?,
                zipsize: number_of(dir, "zipsize")?,
                numbytes: number_of(dir, "numbytes").unwrap_or(0),
                numfiles: number_of(dir, "numfiles").unwrap_or(0),
            });
        }
        Err("the offer is not a text, a file or a directory".into())
    }
}

impl Transit {
    /// The `transit` message.
    pub fn encode(&self) -> Vec<u8> {
        let abilities: Vec<Value> = self
            .abilities
            .iter()
            .map(|a| json!({ "type": a }))
            .collect();
        let hints: Vec<Value> = self.hints.iter().map(Hint::to_json).collect();
        let mut body = Map::new();
        body.insert("abilities-v1".into(), Value::Array(abilities));
        body.insert("hints-v1".into(), Value::Array(hints));
        if let Some(key) = &self.key {
            body.insert(PODNET_KEY.into(), Value::String(pod_wormhole::hex(key)));
        }
        if let Some(token) = &self.relay_token {
            body.insert(PODNET_RELAY_TOKEN.into(), Value::String(token.clone()));
        }
        json!({ "transit": body }).to_string().into_bytes()
    }

    fn parse(value: &Value) -> Transit {
        let list = |name: &str| {
            value
                .get(name)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let abilities = list("abilities-v1")
            .into_iter()
            .filter_map(|a| a["type"].as_str().map(str::to_string));
        let hints = list("hints-v1")
            .iter()
            .filter_map(Hint::from_json)
            .collect();
        let key = value
            .get(PODNET_KEY)
            .and_then(Value::as_str)
            .and_then(pod_wormhole::unhex);
        let key = key.and_then(|k| <[u8; 32]>::try_from(k.as_slice()).ok());
        let relay_token = value
            .get(PODNET_RELAY_TOKEN)
            .and_then(Value::as_str)
            .map(str::to_string);
        Transit {
            abilities: abilities.collect(),
            hints,
            key,
            relay_token,
        }
    }
}

impl Hint {
    fn to_json(&self) -> Value {
        match self {
            Hint::Direct { host, port } => {
                json!({ "type": DIRECT, "hostname": host, "port": port, "priority": 0.0 })
            }
            Hint::Relay { name, ends } => {
                let ends: Vec<Value> = ends.iter().map(RelayEnd::to_json).collect();
                let mut hint = json!({ "type": RELAY, "hints": ends });
                if let Some(name) = name {
                    hint["name"] = Value::String(name.clone());
                }
                hint
            }
            Hint::PodnetRelay { url, name } => {
                json!({ "type": PODNET_RELAY, "url": url, "name": name, "priority": 0.0 })
            }
            Hint::Piping { url } => json!({ "type": PIPING, "url": url, "priority": 0.0 }),
        }
    }

    /// A hint of a kind that this end knows; `None` for another kind, which
    /// the protocol says to ignore.
    fn from_json(value: &Value) -> Option<Hint> {
        match value["type"].as_str()? {
            DIRECT => {
                let (host, port) = host_port(value)?;
                Some(Hint::Direct { host, port })
            }
            RELAY => {
                let ends = value["hints"]
                    .as_array()?
                    .iter()
                    .filter_map(RelayEnd::from_json)
                    .collect();
                let name = value["name"].as_str().map(str::to_string);
                Some(Hint::Relay { name, ends })
            }
            PODNET_RELAY => Some(Hint::PodnetRelay {
                url: value["url"].as_str()?.to_string(),
                name: value["name"].as_str()?.to_string(),
            }),
            PIPING => Some(Hint::Piping {
                url: value["url"].as_str()?.to_string(),
            }),
            _ => None,
        }
    }
}

impl RelayEnd {
    fn to_json(&self) -> Value {
        match self {
            RelayEnd::Tcp { host, port } => {
                json!({ "type": DIRECT, "hostname": host, "port": port, "priority": 0.0 })
            }
            RelayEnd::WebSocket { url } => {
                json!({ "type": WEBSOCKET, "url": url, "priority": 0.0 })
            }
        }
    }

    fn from_json(value: &Value) -> Option<RelayEnd> {
        match value["type"].as_str()? {
            DIRECT => host_port(value).map(|(host, port)| RelayEnd::Tcp { host, port }),
            WEBSOCKET => Some(RelayEnd::WebSocket {
                url: value["url"].as_str()?.to_string(),
            }),
            _ => None,
        }
    }
}

/// The `answer` message.
pub fn answer(answer: &Answer) -> Vec<u8> {
    let body = match answer {
        Answer::MessageAck => json!({ "message_ack": "ok" }),
        Answer::FileAck => json!({ "file_ack": "ok" }),
        Answer::Other(words) => json!({ "file_ack": words }),
    };
    json!({ "answer": body }).to_string().into_bytes()
}

/// The `error` message.
pub fn error(words: &str) -> Vec<u8> {
    json!({ "error": words }).to_string().into_bytes()
}

/// The ack record of the receiver, over the transit.
pub fn ack(sha256: &[u8]) -> Vec<u8> {
    json!({ "ack": "ok", "sha256": pod_wormhole::hex(sha256) })
        .to_string()
        .into_bytes()
}

/// The SHA-256 of an ack record that says `ok`, in hex; an error with the
/// words of an ack that does not.
pub fn read_ack(record: &[u8]) -> Result<Option<String>, String> {
    let value: Value =
        serde_json::from_slice(record).map_err(|_| "the ack of the receiver is not JSON")?;
    if value["ack"].as_str() != Some("ok") {
        return Err(format!("the receiver did not ack the data: {}", value));
    }
    Ok(value["sha256"].as_str().map(str::to_ascii_lowercase))
}

/// One message of the other end. A message with more than one key is read
/// by its first known key, `error` first, as magic-wormhole reads it.
pub fn parse(bytes: &[u8]) -> Result<Message, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "a message of the other end is not JSON")?;
    if !value.is_object() {
        return Err("a message of the other end is not a JSON object".into());
    }
    if let Some(words) = value.get("error") {
        return Ok(Message::Error(
            words
                .as_str()
                .map_or_else(|| words.to_string(), str::to_string),
        ));
    }
    if let Some(transit) = value.get("transit") {
        return Ok(Message::Transit(Transit::parse(transit)));
    }
    if let Some(offer) = value.get("offer") {
        return Offer::parse(offer).map(Message::Offer);
    }
    if let Some(answer) = value.get("answer") {
        let answer = if answer["message_ack"].as_str() == Some("ok") {
            Answer::MessageAck
        } else if answer["file_ack"].as_str() == Some("ok") {
            Answer::FileAck
        } else {
            Answer::Other(answer.to_string())
        };
        return Ok(Message::Answer(answer));
    }
    Ok(Message::Other)
}

fn text_of(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("the offer has no text {key}"))
}

fn number_of(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("the offer has no number {key}"))
}

fn host_port(value: &Value) -> Option<(String, u16)> {
    let host = value["hostname"].as_str()?.to_string();
    let port = u16::try_from(value["port"].as_u64()?).ok()?;
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wormhole_transfer_an_offer_of_magic_wormhole_reads_back() {
        // The forms of cmd_send.py of magic-wormhole 0.24 (filename,
        // filesize; mode, dirname, zipsize, numbytes, numfiles).
        let file = br#"{"offer": {"file": {"filename": "a.txt", "filesize": 12}}}"#;
        assert_eq!(
            parse(file).unwrap(),
            Message::Offer(Offer::File {
                name: "a.txt".into(),
                size: 12
            })
        );
        let dir = br#"{"offer": {"directory": {"mode": "zipfile/deflated", "dirname": "d", "zipsize": 300, "numbytes": 40, "numfiles": 2}}}"#;
        let Message::Offer(offer) = parse(dir).unwrap() else {
            panic!("an offer")
        };
        assert_eq!(parse(&offer.encode()).unwrap(), Message::Offer(offer));
        let text = Offer::Message("hi".into());
        assert_eq!(parse(&text.encode()).unwrap(), Message::Offer(text));
        assert_eq!(
            parse(br#"{"offer": {"other": 1}}"#).unwrap_err(),
            "the offer is not a text, a file or a directory"
        );
    }

    #[test]
    fn wormhole_transfer_hints_of_unknown_kinds_are_ignored() {
        let body = br#"{"transit": {"abilities-v1": [{"type": "direct-tcp-v1"}, {"type": "relay-v1"}],
            "hints-v1": [{"type": "direct-tcp-v1", "hostname": "10.0.0.2", "port": 4000, "priority": 0.0},
                         {"type": "tor-tcp-v1", "hostname": "x.onion", "port": 80},
                         {"type": "relay-v1", "hints": [{"type": "direct-tcp-v1", "hostname": "r.example", "port": 4001},
                                                         {"type": "websocket-v1", "url": "wss://r.example/"},
                                                         {"type": "quic-v9"}]}]}}"#;
        let Message::Transit(transit) = parse(body).unwrap() else {
            panic!("a transit message")
        };
        assert_eq!(transit.abilities, vec![DIRECT, RELAY]);
        assert_eq!(transit.hints.len(), 2);
        let Hint::Relay { ends, .. } = &transit.hints[1] else {
            panic!("a relay")
        };
        assert_eq!(ends.len(), 2);
        assert_eq!(transit.key, None);
        let mine = Transit {
            key: Some([7; 32]),
            relay_token: Some("t".into()),
            ..transit
        };
        let Message::Transit(again) = parse(&mine.encode()).unwrap() else {
            panic!("a transit message")
        };
        assert_eq!(again, mine);
    }

    #[test]
    fn transfer_podnet_relay_hint_is_its_own_kind_and_the_token_is_out_of_the_hints() {
        let mine = Transit {
            abilities: vec![DIRECT.into(), RELAY.into(), PODNET_RELAY.into()],
            hints: vec![Hint::PodnetRelay {
                url: "https://relay.example".into(),
                name: "abc".into(),
            }],
            key: Some([1; 32]),
            relay_token: Some("secret".into()),
        };
        let bytes = mine.encode();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let hints = value["transit"]["hints-v1"].as_array().unwrap();
        assert_eq!(hints[0]["type"], PODNET_RELAY);
        assert!(
            !hints.iter().any(|h| h.to_string().contains("secret")),
            "the token is not in a hint"
        );
        assert_eq!(value["transit"][PODNET_RELAY_TOKEN], "secret");
        let Message::Transit(again) = parse(&bytes).unwrap() else {
            panic!("a transit message")
        };
        assert_eq!(again, mine);
        // A reader that knows only the kinds of magic-wormhole keeps the
        // other hints and drops this one, as it drops `tor-tcp-v1`.
        let known = hints
            .iter()
            .filter(|h| matches!(h["type"].as_str(), Some(DIRECT | RELAY)))
            .count();
        assert_eq!(known, 0);
    }

    #[test]
    fn transit_over_piping_server_the_hint_is_its_own_kind() {
        let mine = Transit {
            abilities: vec![DIRECT.into(), PIPING.into()],
            hints: vec![Hint::Piping {
                url: "https://piping.example".into(),
            }],
            key: Some([1; 32]),
            relay_token: None,
        };
        let bytes = mine.encode();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let hint = &value["transit"]["hints-v1"][0];
        assert_eq!(hint["type"], "piping-v1");
        assert_eq!(hint["url"], "https://piping.example");
        let Message::Transit(again) = parse(&bytes).unwrap() else {
            panic!("a transit message")
        };
        assert_eq!(again, mine);
    }

    #[test]
    fn wormhole_transfer_an_error_wins_over_each_other_key() {
        assert_eq!(
            parse(br#"{"offer": {"message": "x"}, "error": "no"}"#).unwrap(),
            Message::Error("no".into())
        );
        assert_eq!(
            parse(&answer(&Answer::FileAck)).unwrap(),
            Message::Answer(Answer::FileAck)
        );
        assert_eq!(
            parse(br#"{"answer": {"file_ack": "no"}}"#).unwrap(),
            Message::Answer(Answer::Other(r#"{"file_ack":"no"}"#.into()))
        );
        assert_eq!(parse(br#"{"what": 1}"#).unwrap(), Message::Other);
        assert!(parse(b"[1]").is_err());
        assert_eq!(read_ack(&ack(&[0xab; 2])).unwrap().as_deref(), Some("abab"));
        assert!(read_ack(br#"{"ack": "no"}"#).is_err());
    }
}
