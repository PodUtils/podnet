//! The initial configuration and enrolment of podnet#500, INIT-1 to INIT-3.
//!
//! A generated configuration listens on nothing. It names the node, the
//! relay and the allow list, and it is valid against [`SCHEMA`]. A
//! hand-edited listener is refused with a named error rather than accepted
//! and ignored.
//!
//! [`enrol`] issues the identity, asks the registrar to register the node
//! with the relay, and writes the allow list. It builds all of that in a
//! sibling temporary directory and renames it into place, so a failure
//! leaves no partial state and the destination is either absent or whole.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::exit::{self, Fault};
use crate::identity::{self, AllowList, KEY_FILE};

/// The version of the configuration document.
pub const SCHEMA_VERSION: u32 = 1;

/// The file names inside an enrolled state directory.
pub const CONFIG_FILE: &str = "node.json";
pub const SCHEMA_FILE: &str = "node-config.schema.json";
pub const ALLOW_FILE: &str = "allow.txt";
pub const RELAY_FILE: &str = "relay.json";

/// The JSON Schema of a node configuration, draft 2020-12.
pub const SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://podutils.example/schema/node-config-1.json",
  "title": "podnet node configuration",
  "type": "object",
  "required": ["schema", "node", "listen", "relay", "allow", "egress"],
  "additionalProperties": false,
  "properties": {
    "schema": {"const": 1},
    "node": {"type": "string", "minLength": 1},
    "listen": {
      "type": "array",
      "maxItems": 0,
      "description": "INIT-1: a generated configuration listens on nothing."
    },
    "relay": {"type": "string", "pattern": "^https?://"},
    "allow": {"type": "array", "items": {"type": "string"}},
    "egress": {
      "type": "object",
      "required": ["default", "rules"],
      "additionalProperties": false,
      "properties": {
        "default": {"enum": ["direct", "proxy", "relay", "refuse", "queue"]},
        "rules": {"type": "array"}
      }
    }
  }
}"#;

/// The egrees policy of the configuration, in the form of
/// [`crate::policy::Policy`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Egress {
    pub default: String,
    pub rules: Vec<Value>,
}

/// The configuration of one node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub schema: u32,
    pub node: String,
    pub listen: Vec<String>,
    pub relay: String,
    pub allow: Vec<String>,
    pub egress: Egress,
}

/// A configuration that cannot be used, with the part that is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitError {
    /// A listener, which INIT-1 refuses.
    Listener { index: usize, value: String },
    /// A document that is not the schema.
    Schema(String),
    /// A relay that is not reachable in the form the configuration needs.
    NoReach(String),
    /// An enrolment that did not complete.
    Enrol(String),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::Listener { index, value } => write!(
                f,
                "listener {index} {value:?} listens on nothing (INIT-1): a generated configuration has no reachable surface, so remove it"
            ),
            InitError::Schema(text) => write!(f, "the configuration is not schema {SCHEMA_VERSION}: {text}"),
            InitError::NoReach(text) => write!(f, "the relay cannot be reached (INIT-2): {text}"),
            InitError::Enrol(text) => write!(f, "the enrolment did not complete (INIT-3): {text}"),
        }
    }
}

impl std::error::Error for InitError {}

impl InitError {
    /// The process exit code of this error.
    pub fn code(&self) -> i32 {
        match self {
            InitError::Listener { .. } | InitError::Schema(_) | InitError::NoReach(_) => {
                exit::CONFIG
            }
            InitError::Enrol(_) => exit::UNAVAILABLE,
        }
    }

    /// The error as the CLI reports it.
    pub fn fault(self) -> anyhow::Error {
        let class = match self.code() {
            exit::UNAVAILABLE => Fault::Unavailable,
            _ => Fault::Config,
        };
        exit::fault(class, anyhow::anyhow!(self.to_string()))
    }
}

impl NodeConfig {
    /// A configuration that listens on nothing and refuses each egress
    /// destination that no rule names.
    pub fn generated(node: &str, relay: &str) -> NodeConfig {
        NodeConfig {
            schema: SCHEMA_VERSION,
            node: node.to_string(),
            listen: Vec::new(),
            relay: relay.to_string(),
            allow: Vec::new(),
            egress: Egress {
                default: "refuse".into(),
                rules: Vec::new(),
            },
        }
    }

    /// The document of this configuration.
    pub fn text(&self) -> anyhow::Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }

    /// Parse and validate a document.
    pub fn parse(text: &str) -> Result<NodeConfig, InitError> {
        let config: NodeConfig =
            serde_json::from_str(text).map_err(|e| InitError::Schema(format!("{e}")))?;
        config.validate()?;
        Ok(config)
    }

    /// Read and validate a file.
    pub fn load(path: &Path) -> Result<NodeConfig, InitError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| InitError::Schema(format!("{}: {e}", path.display())))?;
        NodeConfig::parse(&text).map_err(|e| match e {
            InitError::Schema(text) => InitError::Schema(format!("{}: {text}", path.display())),
            other => other,
        })
    }

    /// Each part of the document that the schema and the policy refuse.
    pub fn validate(&self) -> Result<(), InitError> {
        if self.schema != SCHEMA_VERSION {
            return Err(InitError::Schema(format!(
                "the schema is {}, and this build reads {SCHEMA_VERSION}",
                self.schema
            )));
        }
        if self.node.trim().is_empty() {
            return Err(InitError::Schema("the node name is empty".into()));
        }
        if let Some((index, value)) = self.listen.iter().enumerate().next() {
            return Err(InitError::Listener {
                index,
                value: value.clone(),
            });
        }
        let relay = self.relay.trim();
        if !(relay.starts_with("http://") || relay.starts_with("https://")) {
            return Err(InitError::NoReach(format!(
                "{:?} is not an http(s) URL",
                self.relay
            )));
        }
        if Url::parse(relay).is_none() {
            return Err(InitError::NoReach(format!(
                "{:?} has no host and port",
                self.relay
            )));
        }
        if !self.allow.is_empty() {
            let text = self.allow.join("\n");
            AllowList::parse(&text)
                .map_err(|e| InitError::Schema(format!("the allow list: {e}")))?;
        }
        let policy = serde_json::json!({
            "default": self.egress.default,
            "rules": self.egress.rules,
        });
        crate::policy::Policy::parse(&policy.to_string())
            .map_err(|e| InitError::Schema(format!("the egress policy: {e}")))?;
        Ok(())
    }

    /// Write the configuration and its schema into an existing directory.
    pub fn write(&self, dir: &Path) -> anyhow::Result<()> {
        self.validate().map_err(InitError::fault)?;
        std::fs::write(dir.join(CONFIG_FILE), self.text()?)?;
        std::fs::write(dir.join(SCHEMA_FILE), SCHEMA)?;
        Ok(())
    }
}

/// A relay URL split into the parts of a plain HTTP request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    /// Parse `http://HOST[:PORT][/PATH]` or `https://...`.
    pub fn parse(text: &str) -> Option<Url> {
        let (https, rest) = match text.split_once("://") {
            Some(("http", rest)) => (false, rest),
            Some(("https", rest)) => (true, rest),
            _ => return None,
        };
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => {
                let port = port.parse::<u16>().ok()?;
                (host.to_string(), port)
            }
            _ => (authority.to_string(), if https { 443 } else { 80 }),
        };
        if host.contains(':') && !host.starts_with('[') {
            return None;
        }
        Some(Url {
            https,
            host,
            port,
            path,
        })
    }

    fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Where the registration of a node goes.
#[derive(Clone, Debug)]
pub enum Registrar {
    /// The relay receives the registration over HTTP, or over TLS when its
    /// URL is `https`.
    Http { timeout: Duration },
    /// No registration leaves this host, and the result names that. It is
    /// for a test and for an install that will register later.
    Offline,
    /// A registrar that always fails, for the rollback test.
    Failing(String),
}

impl Registrar {
    /// Register `node` with the relay at `relay`, with the public key `key`.
    pub async fn register(&self, node: &str, key: &str, relay: &str) -> Result<Value, String> {
        match self {
            Registrar::Failing(reason) => Err(reason.clone()),
            Registrar::Offline => Ok(serde_json::json!({
                "node": node,
                "public_key": key,
                "relay": relay,
                "registered": false,
                "reason": "offline registrar: the node is not registered with the relay yet",
            })),
            Registrar::Http { timeout } => {
                let url = Url::parse(relay).ok_or_else(|| {
                    format!("the relay {relay:?} is not an http(s) URL with a host")
                })?;
                let endpoint = if url.path == "/" {
                    "/v1/enrol".to_string()
                } else {
                    url.path.clone()
                };
                let body = serde_json::json!({"node": node, "public_key": key}).to_string();
                let host = url.host.clone();
                let port = url.port;
                let authority = url.authority();
                let request = format!(
                    "POST {endpoint} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let stream = crate::egress::dial(&host, port, None, *timeout)
                    .await
                    .map_err(|e| dial_failed(&authority, &e))?;
                if url.https {
                    let config = crate::trust::context()
                        .client_config_with(&[], &[b"http/1.1"])
                        .map_err(|e| format!("the roots for the relay {authority}: {e}"))?;
                    let name = rustls::pki_types::ServerName::try_from(host.clone())
                        .map_err(|e| format!("the name {host:?} is not a TLS name: {e}"))?;
                    let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
                        .connect(name, stream)
                        .await
                        .map_err(|e| format!("TLS with the relay {authority}: {e}"))?;
                    post(tls, &request, &authority).await
                } else {
                    post(stream, &request, &authority).await
                }
            }
        }
    }
}

/// The line of a failed dial of the relay (GitHub #722): each cause one
/// time, and the next step of its hop.
fn dial_failed(authority: &str, e: &anyhow::Error) -> String {
    let step = match crate::hop::of(e) {
        Some(hop) => hop.hint(),
        None => "check the host and the port of --relay, and that the relay listens there; a host that reaches out only through a proxy needs --proxy",
    };
    format!(
        "dialling the relay {authority}: {}; {step}",
        crate::exit::text(e)
    )
}

/// The line of a relay that answered with an error status (GitHub #723):
/// the status, and a short text body on one line; an HTML page or a long
/// body is left out (it is not the answer of a relay of podnet).
fn refused_by_relay(status: u16, head: &str, body: &str) -> String {
    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
    let text = body.trim();
    let html = text.starts_with('<') || chunked && text.contains('<');
    let shown = match text.lines().next() {
        Some(line) if !html && !chunked && !line.is_empty() && line.len() <= 200 => {
            format!(": {}", line.trim())
        }
        _ => String::new(),
    };
    format!("the relay answered HTTP {status}{shown}; this URL may not be a relay of podnet: check --relay")
}

/// Send one request on `stream` and parse the reply.
async fn post<S>(mut stream: S, request: &str, authority: &str) -> Result<Value, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("writing to the relay {authority}: {e}"))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("writing to the relay {authority}: {e}"))?;
    let mut reply = Vec::new();
    stream
        .take(64 * 1024)
        .read_to_end(&mut reply)
        .await
        .map_err(|e| format!("reading from the relay {authority}: {e}"))?;
    parse_reply(&reply)
}

fn parse_reply(reply: &[u8]) -> Result<Value, String> {
    let split = reply
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "the relay answered no complete HTTP head".to_string())?;
    let head = String::from_utf8_lossy(&reply[..split]);
    let body = String::from_utf8_lossy(&reply[split + 4..]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("the relay answered no status: {head:?}"))?;
    if !(200..300).contains(&status) {
        return Err(refused_by_relay(status, &head, &body));
    }
    if body.trim().is_empty() {
        return Ok(serde_json::json!({"status": status}));
    }
    serde_json::from_str(&body)
        .map_err(|e| format!("the relay answered status {status} and a body that is not JSON: {e}"))
}

/// What an enrolment produced.
#[derive(Clone, Debug, Serialize)]
pub struct Enrolled {
    pub node: String,
    pub path: String,
    pub fingerprint: String,
    pub public_key: String,
    pub allowed: usize,
    pub registration: Value,
}

/// The destination is present or whole: it is never partially written.
pub fn is_enrolled(dest: &Path) -> bool {
    dest.join(KEY_FILE).is_file()
        && dest.join(CONFIG_FILE).is_file()
        && dest.join(ALLOW_FILE).is_file()
        && dest.join(RELAY_FILE).is_file()
}

/// Enrol `config` into `dest`: the identity, the allow list, the relay
/// registration, the configuration and its schema, or none of them.
pub async fn enrol(
    dest: &Path,
    config: &NodeConfig,
    registrar: &Registrar,
) -> anyhow::Result<Enrolled> {
    config.validate().map_err(InitError::fault)?;
    if dest.exists() {
        return Err(InitError::Enrol(format!(
            "{} already exists, so it is not enrolled over",
            dest.display()
        ))
        .fault());
    }
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state".to_string())
        // A prefix of pod-core holds no "..".
        .replace("..", "_");
    // T-545: a new directory with a random name; a pid is not unique
    // across PID namespaces.
    let tmp: PathBuf = podnet_core::state::temp::new_temp_dir(parent, &format!(".{name}.enrol"))
        .map_err(|e| InitError::Enrol(e.to_string()).fault())?;

    let outcome = build(&tmp, config, registrar).await;
    match outcome {
        Ok(mut enrolled) => match std::fs::rename(&tmp, dest) {
            Ok(()) => {
                enrolled.path = dest.display().to_string();
                Ok(enrolled)
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                Err(InitError::Enrol(format!(
                    "renaming {} to {}: {e}",
                    tmp.display(),
                    dest.display()
                ))
                .fault())
            }
        },
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

async fn build(tmp: &Path, config: &NodeConfig, registrar: &Registrar) -> anyhow::Result<Enrolled> {
    let identity = identity::create(tmp)?;
    let public = identity.public();
    if !config.allow.is_empty() {
        AllowList::parse(&config.allow.join("\n"))
            .map_err(|e| InitError::Schema(format!("the allow list: {e}")).fault())?;
    }
    let allow = if config.allow.is_empty() {
        String::new()
    } else {
        format!("{}\n", config.allow.join("\n"))
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(tmp.join(ALLOW_FILE))?;
    file.write_all(allow.as_bytes())?;
    file.sync_all()?;

    let registration = registrar
        .register(&config.node, &public.hex(), &config.relay)
        .await
        .map_err(|reason| InitError::Enrol(reason).fault())?;
    std::fs::write(
        tmp.join(RELAY_FILE),
        serde_json::to_string_pretty(&registration)?,
    )?;
    config.write(tmp)?;
    Ok(Enrolled {
        node: config.node.clone(),
        path: tmp.display().to_string(),
        fingerprint: identity.fingerprint(),
        public_key: public.hex(),
        allowed: config.allow.len(),
        registration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn config() -> NodeConfig {
        NodeConfig::generated("node-a", "http://relay.example:8080")
    }

    /// GitHub #722 and #723: a relay that cannot be reached gives each cause
    /// and a next step; a URL that is not a relay gives its status and one
    /// line, not the body of an HTML page with its chunk sizes.
    #[tokio::test]
    async fn a_failed_enrolment_gives_one_line_with_the_cause_and_a_step() {
        crate::test_host::need_loopback!();
        let (_held, refused) = pod_testkit::refused::refused();
        let registrar = Registrar::Http {
            timeout: std::time::Duration::from_secs(5),
        };
        let relay = format!("http://{refused}");
        let err = registrar
            .register("node-a", "key", &relay)
            .await
            .unwrap_err();
        assert!(err.contains("refused"), "the cause: {err}");
        assert!(
            err.contains("; check") || err.contains("; give"),
            "a step: {err}"
        );
        let page = "HTTP/1.1 405 Method Not Allowed\r\nTransfer-Encoding: chunked\r\n\r\n22e\r\n<!doctype html><html><body>Example</body></html>\r\n0\r\n\r\n";
        let plain = "HTTP/1.1 403 Forbidden\r\nContent-Length: 12\r\n\r\nnode refused";
        for (reply, has, lacks) in [
            (page, "HTTP 405;", "<"),
            (plain, "HTTP 403: node refused;", "\n"),
        ] {
            let err = parse_reply(reply.as_bytes()).unwrap_err();
            assert!(err.contains(has) && err.contains("check --relay"), "{err}");
            assert!(!err.contains(lacks) && !err.contains("22e"), "{err}");
        }
    }

    #[test]
    fn a_generated_configuration_listens_on_nothing() {
        let config = config();
        assert!(config.listen.is_empty());
        assert!(config.validate().is_ok());
        let text = config.text().unwrap();
        assert!(text.contains("\"listen\": []"), "{text}");
        let parsed = NodeConfig::parse(&text).unwrap();
        assert_eq!(parsed, config);
    }

    /// INIT-1: a hand-added listener is refused with a named error.
    #[test]
    fn a_hand_edited_listener_is_refused() {
        let mut config = config();
        config.listen.push("0.0.0.0:443".into());
        let err = config.validate().unwrap_err();
        assert!(matches!(err, InitError::Listener { ref value, .. } if value == "0.0.0.0:443"));
        assert!(
            err.to_string().contains("listens on nothing (INIT-1)"),
            "{err}"
        );
    }

    /// INIT-3: the generated document is valid against the schema that the
    /// generator writes, not only against the hand-written checks.
    #[test]
    fn the_generated_configuration_is_valid_against_the_emitted_schema() {
        let config = config();
        let value = serde_json::to_value(&config).unwrap();
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        validate(&value, &schema);
        let dir = tempfile::tempdir().unwrap();
        config.write(dir.path()).unwrap();
        let written = std::fs::read_to_string(dir.path().join(SCHEMA_FILE)).unwrap();
        let written: Value = serde_json::from_str(&written).unwrap();
        validate(&value, &written);
    }

    /// The keywords that [`SCHEMA`] uses. A keyword that this does not read
    /// is a schema change that needs this function.
    fn validate(value: &Value, schema: &Value) {
        if let Some(expected) = schema.get("const") {
            assert_eq!(value, expected);
        }
        match schema.get("type").and_then(Value::as_str) {
            Some("object") => {
                let object = value.as_object().expect("an object");
                if let Some(required) = schema.get("required").and_then(Value::as_array) {
                    for key in required {
                        assert!(object.contains_key(key.as_str().unwrap()), "missing {key}");
                    }
                }
                if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                    for (key, child) in properties {
                        if let Some(field) = object.get(key) {
                            validate(field, child);
                        }
                    }
                    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                        for key in object.keys() {
                            assert!(properties.contains_key(key), "an unknown field {key}");
                        }
                    }
                }
            }
            Some("array") => {
                let array = value.as_array().expect("an array");
                if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
                    assert!(
                        array.len() as u64 <= max,
                        "{} items over {max}",
                        array.len()
                    );
                }
                if let Some(items) = schema.get("items") {
                    for item in array {
                        validate(item, items);
                    }
                }
            }
            Some("string") => {
                let text = value.as_str().expect("a string");
                if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
                    assert!(text.len() as u64 >= min, "{text:?} is shorter than {min}");
                }
                if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
                    if pattern == "^https?://" {
                        assert!(
                            text.starts_with("http://") || text.starts_with("https://"),
                            "{text:?} does not match {pattern}"
                        );
                    }
                }
            }
            Some(other) => panic!("the validator does not read the type {other}"),
            None => {}
        }
        if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
            assert!(allowed.contains(value), "{value:?} is not in {allowed:?}");
        }
    }

    #[test]
    fn a_relay_that_is_not_http_is_refused() {
        let mut config = config();
        config.relay = "tcp://relay.example".into();
        let err = config.validate().unwrap_err();
        assert!(matches!(err, InitError::NoReach(_)), "{err}");
    }

    #[test]
    fn the_url_keeps_the_path_and_the_default_ports() {
        let url = Url::parse("http://relay.example/v1/enrol").unwrap();
        assert_eq!(url.host, "relay.example");
        assert_eq!(url.port, 80);
        assert_eq!(url.path, "/v1/enrol");
        assert_eq!(Url::parse("https://relay.example").unwrap().port, 443);
        assert!(Url::parse("relay.example:80").is_none());
    }

    /// INIT-3: a failed registrar leaves no destination and no sibling temp.
    #[tokio::test]
    async fn a_failed_enrolment_leaves_no_partial_state() {
        let root = std::env::temp_dir().join(format!("podnet-enrol-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dest = root.join("state");
        let err = enrol(&dest, &config(), &Registrar::Failing("no relay".into()))
            .await
            .unwrap_err();
        assert_eq!(exit::code_of(&err), exit::UNAVAILABLE);
        assert!(!dest.exists(), "the destination exists after a failure");
        let left: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "partial state left: {left:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// INIT-2 and INIT-3: the three parts are written together, the key is
    /// 0600 and the allow list is 0644.
    #[tokio::test]
    async fn an_enrolment_writes_the_identity_the_allow_list_and_the_relay() {
        let root = std::env::temp_dir().join(format!("podnet-enrol-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dest = root.join("state");
        let mut config = config();
        config.allow = vec!["0".repeat(64), "1".repeat(64)];
        let enrolled = enrol(&dest, &config, &Registrar::Offline).await.unwrap();
        assert_eq!(enrolled.allowed, 2);
        assert!(is_enrolled(&dest));
        let key = std::fs::metadata(dest.join(KEY_FILE)).unwrap();
        assert_eq!(key.permissions().mode() & 0o777, 0o600);
        let allow = std::fs::metadata(dest.join(ALLOW_FILE)).unwrap();
        assert_eq!(allow.permissions().mode() & 0o777, 0o644);
        let relay: Value =
            serde_json::from_str(&std::fs::read_to_string(dest.join(RELAY_FILE)).unwrap()).unwrap();
        assert_eq!(relay["registered"], false);
        let read = NodeConfig::load(&dest.join(CONFIG_FILE)).unwrap();
        assert_eq!(read, config);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_existing_destination_is_not_enrolled_over() {
        let root = std::env::temp_dir().join(format!("podnet-enrol-taken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("state")).unwrap();
        let err = enrol(&root.join("state"), &config(), &Registrar::Offline)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
