//! TLS Encrypted ClientHello (ECH, RFC 9849 / RFC 9460).
//!
//! Uses the pure-Rust RFC 9180 HPKE and ECH handshake path in `pod-tls`, the
//! HTTPS-record reader in `pod-dns`, and the dialers of `pod-proxy` and
//! `pod-ws`. ECH is off by default so wire bytes stay unchanged until enabled
//! with `--ech` or `PODNET_ECH`.

use std::sync::{OnceLock, PoisonError, RwLock};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use base64::Engine as _;
use rustls::client::{EchConfig, EchGreaseConfig, EchMode};
use rustls::pki_types::EchConfigListBytes;
use serde_json::{json, Value};
use tokio::time::Instant;

pub use pod_tls::ech::{Mode, Outcome, Published, Records};

/// The mode or base64 ECHConfigList: `off`, `auto`/`on`, `strict`, or base64.
pub const ECH_VARIABLE: &str = "PODNET_ECH";
/// Optional outer front hostname to cache an explicit ECHConfigList under.
pub const ECH_FRONT_VARIABLE: &str = "PODNET_ECH_FRONT";
/// Comma-separated hops for ECH (`api`, `edge`, `masque`, `all`).
pub const ECH_HOPS_VARIABLE: &str = "PODNET_ECH_HOPS";
/// Send GREASE ECH on TLS 1.3 ClientHellos when no ECHConfigList applies.
pub const ECH_GREASE_VARIABLE: &str = "PODNET_ECH_GREASE";

/// RFC 9849 / draft-ietf-tls-esni-13+ ECHConfig version.
const ECH_VERSION_DRAFT_13: u16 = 0xfe0d;

/// Parsed value of `--ech` or `PODNET_ECH`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EchSetting {
    /// Plain ClientHello (default): wire bytes stay identical.
    #[default]
    Off,
    /// Opportunistic ECH: seal when HTTPS DNS publishes a list, else plain.
    On,
    /// Strict ECH: require a sealed ClientHello; never send a plain ClientHello.
    Strict,
    /// Explicit base64-decoded ECHConfigList, validated against pure-Rust HPKE.
    ConfigList(Vec<u8>),
}

impl EchSetting {
    /// Parse `off`/`no`/`0`, `on`/`yes`/`auto`/`1`, `strict`, or a base64
    /// `ECHConfigList`.
    pub fn parse(value: &str) -> Result<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(Self::Off);
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "off" | "no" | "false" | "0" => return Ok(Self::Off),
            "on" | "yes" | "auto" | "true" | "1" => return Ok(Self::On),
            "strict" => return Ok(Self::Strict),
            _ => {}
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(trimmed)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(trimmed))
            .map_err(|_| {
                anyhow::anyhow!(
                    "invalid ECH value {trimmed:?}: give off, auto, on, strict, or a base64 ECHConfigList"
                )
            })?;
        parse_ech_config_list(&bytes).context("invalid ECHConfigList")?;
        validate_ech_config_list(&bytes)?;
        Ok(Self::ConfigList(bytes))
    }

    /// Corresponding `pod-tls` ECH mode.
    pub fn mode(&self) -> Mode {
        match self {
            Self::Off => Mode::Off,
            Self::On | Self::ConfigList(_) => Mode::On,
            Self::Strict => Mode::Strict,
        }
    }

    /// Short mode word for status and doctor output.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "auto",
            Self::Strict => "strict",
            Self::ConfigList(_) => "config",
        }
    }
}

/// Decoded metadata from one RFC 9849 `ECHConfig` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchConfigInfo {
    pub version: u16,
    pub config_id: u8,
    pub kem_id: u16,
    pub max_name_length: u8,
    pub public_name: String,
}

/// Decode an RFC 9849 `ECHConfigList` to extract `public_name` and
/// `max_name_length` from each supported (`0xfe0d`) `ECHConfig`.
pub fn parse_ech_config_list(bytes: &[u8]) -> Result<Vec<EchConfigInfo>> {
    if bytes.len() < 2 {
        bail!("ECHConfigList is shorter than 2 bytes");
    }
    let total = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    if total == 0 || 2 + total != bytes.len() {
        bail!(
            "ECHConfigList length {total} does not match payload length {}",
            bytes.len() - 2
        );
    }
    let mut infos = Vec::new();
    let mut at = 2;
    while at < bytes.len() {
        let head = bytes
            .get(at..at + 4)
            .ok_or_else(|| anyhow::anyhow!("ECHConfig header is truncated at offset {at}"))?;
        let version = u16::from_be_bytes([head[0], head[1]]);
        let len = usize::from(u16::from_be_bytes([head[2], head[3]]));
        let body = bytes
            .get(at + 4..at + 4 + len)
            .ok_or_else(|| anyhow::anyhow!("ECHConfig body is truncated at offset {at}"))?;
        at += 4 + len;
        if version == ECH_VERSION_DRAFT_13 {
            infos.push(parse_ech_config_contents(version, body)?);
        }
    }
    if infos.is_empty() {
        bail!("ECHConfigList contains no supported ECHConfig (version 0xfe0d)");
    }
    Ok(infos)
}

fn parse_ech_config_contents(version: u16, body: &[u8]) -> Result<EchConfigInfo> {
    if body.len() < 5 {
        bail!("ECHConfig contents are truncated");
    }
    let config_id = body[0];
    let kem_id = u16::from_be_bytes([body[1], body[2]]);
    let pk_len = usize::from(u16::from_be_bytes([body[3], body[4]]));
    if pk_len == 0 || 5 + pk_len > body.len() {
        bail!("ECHConfig public key length {pk_len} is invalid");
    }
    let mut pos = 5 + pk_len;
    let suites_hdr = body
        .get(pos..pos + 2)
        .ok_or_else(|| anyhow::anyhow!("ECHConfig cipher_suites length is truncated"))?;
    let suites_len = usize::from(u16::from_be_bytes([suites_hdr[0], suites_hdr[1]]));
    pos += 2;
    if suites_len == 0 || !suites_len.is_multiple_of(4) || pos + suites_len > body.len() {
        bail!("ECHConfig cipher_suites length {suites_len} is invalid");
    }
    pos += suites_len;
    let max_name_length = *body
        .get(pos)
        .ok_or_else(|| anyhow::anyhow!("ECHConfig maximum_name_length is missing"))?;
    pos += 1;
    let name_len = usize::from(
        *body
            .get(pos)
            .ok_or_else(|| anyhow::anyhow!("ECHConfig public_name length is missing"))?,
    );
    pos += 1;
    if name_len == 0 || pos + name_len > body.len() {
        bail!("ECHConfig public_name length {name_len} is invalid");
    }
    let public_name = std::str::from_utf8(&body[pos..pos + name_len])
        .context("ECHConfig public_name is not UTF-8")?
        .to_string();
    pos += name_len;
    let ext_hdr = body
        .get(pos..pos + 2)
        .ok_or_else(|| anyhow::anyhow!("ECHConfig extensions length is missing"))?;
    let ext_len = usize::from(u16::from_be_bytes([ext_hdr[0], ext_hdr[1]]));
    pos += 2;
    if pos + ext_len != body.len() {
        bail!("ECHConfig extensions length {ext_len} does not match remaining bytes");
    }
    Ok(EchConfigInfo {
        version,
        config_id,
        kem_id,
        max_name_length,
        public_name,
    })
}

/// Check that `inner_name` fits inside `info.max_name_length` when the config
/// sets a non-zero maximum. Returns an error naming both lengths when exceeded.
pub fn check_name_length(inner_name: &str, info: &EchConfigInfo) -> Result<()> {
    let name = inner_name.trim_end_matches('.');
    if info.max_name_length > 0 && name.len() > usize::from(info.max_name_length) {
        bail!(
            "inner name length {} exceeds ECH max_name_length {} for public_name {}",
            name.len(),
            info.max_name_length,
            info.public_name
        );
    }
    Ok(())
}

/// Validate `bytes` with rustls and the pure-Rust HPKE suites of `pod-tls`.
pub fn validate_ech_config_list(bytes: &[u8]) -> Result<EchConfig> {
    EchConfig::new(
        EchConfigListBytes::from(bytes.to_vec()),
        pod_tls::crypto::hpke::SUITES,
    )
    .map_err(|e| anyhow::anyhow!("ECHConfigList is not supported by pure-Rust HPKE: {e}"))
}

/// Target hops selected for ECH configuration and doctor probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EchHops {
    pub api: bool,
    pub edge: bool,
    pub masque: bool,
}

impl Default for EchHops {
    fn default() -> Self {
        Self::all()
    }
}

impl EchHops {
    /// All hops enabled (`api`, `edge`, `masque`).
    pub const fn all() -> Self {
        Self {
            api: true,
            edge: true,
            masque: true,
        }
    }

    /// No hops enabled.
    pub const fn none() -> Self {
        Self {
            api: false,
            edge: false,
            masque: false,
        }
    }

    /// Parse a comma-separated list of hops: `api`, `edge`, `masque`, `all`, or
    /// `none`. An empty string returns [`Self::all`].
    pub fn parse(spec: &str) -> Result<Self> {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return Ok(Self::all());
        }
        let mut hops = Self::none();
        let mut any = false;
        for raw in trimmed.split(',') {
            let word = raw.trim().to_ascii_lowercase();
            if word.is_empty() {
                continue;
            }
            any = true;
            match word.as_str() {
                "all" => hops = Self::all(),
                "api" => hops.api = true,
                "edge" => hops.edge = true,
                "masque" => hops.masque = true,
                "none" => hops = Self::none(),
                other => bail!("unknown ECH hop {other:?}: give api, edge, masque, or all"),
            }
        }
        if !any {
            return Ok(Self::all());
        }
        Ok(hops)
    }

    /// Hostnames of the selected podnet hops, in stable order.
    pub fn target_hosts(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if self.api {
            let quick = crate::services::endpoint(crate::services::QUICK, "api");
            out.push(("api", quick.host().to_string()));
            let warp = crate::services::endpoint(crate::services::WARP, "api");
            out.push(("warp-api", warp.host().to_string()));
        }
        if self.edge {
            out.push(("edge", crate::services::edge_region_name().to_string()));
        }
        if self.masque {
            out.push(("masque", crate::services::warp::masque_sni().to_string()));
        }
        out
    }
}

/// Complete ECH options for the process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EchOptions {
    pub setting: EchSetting,
    pub front: Option<String>,
    pub hops: EchHops,
    pub grease: bool,
}

static OPTIONS: OnceLock<RwLock<EchOptions>> = OnceLock::new();

fn state() -> &'static RwLock<EchOptions> {
    OPTIONS.get_or_init(|| RwLock::new(EchOptions::default()))
}

/// Current ECH options of the process.
pub fn options() -> EchOptions {
    state()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// True when ECH or GREASE ECH is enabled for this process.
pub fn is_active() -> bool {
    let opts = options();
    opts.setting != EchSetting::Off || opts.grease
}

/// Apply `opts` to the process TLS context (`pod-tls`) and cache any explicit
/// `ECHConfigList` for the selected target hops.
pub fn configure(opts: EchOptions) -> Result<()> {
    let ctx = crate::trust::context();
    if let EchSetting::ConfigList(ref bytes) = opts.setting {
        parse_ech_config_list(bytes).context("invalid ECHConfigList")?;
        validate_ech_config_list(bytes)?;
        for (_, host) in opts.hops.target_hosts() {
            ctx.ech()
                .cache()
                .remember(&host, bytes.clone(), pod_tls::ech::cache::CEILING);
        }
        if let Some(front) = opts
            .front
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            ctx.ech()
                .cache()
                .remember(front, bytes.clone(), pod_tls::ech::cache::CEILING);
        }
    }
    ctx.ech().set_mode(opts.setting.mode());
    *state().write().unwrap_or_else(PoisonError::into_inner) = opts;
    Ok(())
}

/// Parse a boolean environment variable (`1`/`true`/`yes`/`on` vs
/// `0`/`false`/`no`/`off`).
fn parse_bool(value: &str, name: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        other => bail!("{name}: invalid boolean {other:?}: give 1, 0, true, or false"),
    }
}

fn env_front() -> Option<String> {
    std::env::var(ECH_FRONT_VARIABLE)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_hops() -> Result<EchHops> {
    match std::env::var(ECH_HOPS_VARIABLE) {
        Ok(val) => EchHops::parse(&val).context(ECH_HOPS_VARIABLE),
        Err(_) => Ok(EchHops::all()),
    }
}

/// Read ECH options from the environment (`PODNET_ECH`, `PODNET_ECH_FRONT`,
/// `PODNET_ECH_HOPS`, `PODNET_ECH_GREASE`) without modifying process state.
pub fn options_from_env() -> Result<EchOptions> {
    let setting = match std::env::var(ECH_VARIABLE) {
        Ok(val) => EchSetting::parse(&val).context(ECH_VARIABLE)?,
        Err(_) => EchSetting::Off,
    };
    let grease = match std::env::var(ECH_GREASE_VARIABLE) {
        Ok(val) => parse_bool(&val, ECH_GREASE_VARIABLE)?,
        Err(_) => false,
    };
    Ok(EchOptions {
        setting,
        front: env_front(),
        hops: env_hops()?,
        grease,
    })
}

/// Validate ECH environment variables (configuration check before work).
pub fn validate_env() -> Result<()> {
    let _ = options_from_env()?;
    Ok(())
}

/// Read ECH options from the environment and configure the process TLS context.
pub fn configure_from_env() -> Result<()> {
    configure(options_from_env()?)
}

/// Configure ECH from CLI flags (`--ech`, `--ech-grease`) combined with
/// `PODNET_ECH_FRONT` and `PODNET_ECH_HOPS`.
pub fn configure_cli(ech: Option<&str>, grease: bool) -> Result<()> {
    let setting = match ech {
        Some(value) => EchSetting::parse(value).context("--ech")?,
        None => EchSetting::Off,
    };
    let hops = env_hops().map_err(|e| crate::exit::fault(crate::exit::Fault::Config, e))?;
    configure(EchOptions {
        setting,
        front: env_front(),
        hops,
        grease,
    })
}

/// Construct a GREASE ECH configuration using `pod-tls` pure-Rust HPKE.
pub fn grease_config() -> Result<EchGreaseConfig> {
    let suite = pod_tls::crypto::hpke::SUITES[0];
    let (placeholder_key, _) = suite
        .generate_key_pair()
        .map_err(|e| anyhow::anyhow!("generating GREASE ECH placeholder key: {e}"))?;
    Ok(EchGreaseConfig::new(suite, placeholder_key))
}

/// Build a TLS 1.3 `ClientConfig` with GREASE ECH enabled, using the process
/// roots (plus optional `extra` PEM roots) and `alpn` protocols.
pub fn grease_client_config(extra: &[&[u8]], alpn: &[&[u8]]) -> Result<rustls::ClientConfig> {
    client_config_with_ech(EchMode::Grease(grease_config()?), extra, alpn)
}

/// Build a TLS 1.3 `ClientConfig` sealed with `list` (an `ECHConfigList`),
/// using the process roots (plus optional `extra` PEM roots) and `alpn`.
pub fn sealed_client_config(
    list: &[u8],
    extra: &[&[u8]],
    alpn: &[&[u8]],
) -> Result<rustls::ClientConfig> {
    let ech = validate_ech_config_list(list)?;
    client_config_with_ech(EchMode::Enable(ech), extra, alpn)
}

fn client_config_with_ech(
    mode: EchMode,
    extra: &[&[u8]],
    alpn: &[&[u8]],
) -> Result<rustls::ClientConfig> {
    let ctx = crate::trust::context();
    let store = ctx.roots()?.with_extra(extra)?;
    let provider = ctx.provider().clone();
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_ech(mode)
        .map_err(|e| anyhow::anyhow!("building ECH client config: {e}"))?
        .with_root_certificates(store)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(config)
}

/// ECH probe result for one podnet hop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopProbe {
    pub hop: &'static str,
    pub host: String,
    pub published: bool,
    pub public_name: Option<String>,
    pub max_name_length: Option<u8>,
    pub name_fits: Option<bool>,
    pub detail: String,
}

impl HopProbe {
    fn without_info(hop: &'static str, host: String, published: bool, detail: String) -> Self {
        Self {
            hop,
            host,
            published,
            public_name: None,
            max_name_length: None,
            name_fits: None,
            detail,
        }
    }
}

/// ECH status row for `podnet doctor`.
#[derive(Debug, Clone)]
pub struct DoctorRow {
    pub mode: String,
    pub grease: bool,
    pub hops: Vec<HopProbe>,
    pub note: Option<String>,
}

impl DoctorRow {
    /// One-line summary when ECH is off.
    pub fn off_line() -> String {
        "ech    off (pure-Rust HPKE ready; pass --ech auto|strict)".to_string()
    }

    /// Fallback row when `--quick` hits its deadline before probes finish.
    pub fn late(detail: &str) -> Self {
        let opts = options();
        Self {
            mode: opts.setting.word().to_string(),
            grease: opts.grease,
            hops: Vec::new(),
            note: Some(detail.to_string()),
        }
    }

    /// Render the doctor row as plain text lines.
    pub fn text(&self) -> String {
        let grease = if self.grease { "on" } else { "off" };
        let mut out = format!("ech    mode={} grease={grease}", self.mode);
        if let Some(note) = &self.note {
            out.push_str(&format!(" ({note})"));
        }
        out.push('\n');
        for h in &self.hops {
            let status = if h.published {
                "published"
            } else {
                "not-published"
            };
            out.push_str(&format!(
                "  {:<10} {:<38} {:<13} {}\n",
                h.hop, h.host, status, h.detail
            ));
        }
        out
    }

    /// Render the doctor row as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "mode": self.mode,
            "grease": self.grease,
            "note": self.note,
            "hops": self.hops.iter().map(|h| json!({
                "hop": h.hop,
                "host": h.host,
                "published": h.published,
                "public_name": h.public_name,
                "max_name_length": h.max_name_length,
                "name_fits": h.name_fits,
                "detail": h.detail,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Probe HTTPS (`ech=`) records for `hops` using the process `pod-dns` resolver.
pub async fn doctor_row(hops: EchHops, timeout: Duration) -> DoctorRow {
    probe_hops_with(&*crate::egress::resolver(), hops, timeout).await
}

/// Run the doctor ECH probe only when ECH or GREASE ECH is active, respecting
/// an optional `--quick` deadline.
pub async fn doctor_probe_if_active(
    timeout: Duration,
    deadline: Option<Instant>,
    late_detail: &str,
) -> Option<DoctorRow> {
    if !is_active() {
        return None;
    }
    let work = doctor_row(options().hops, timeout);
    Some(match deadline {
        Some(end) => tokio::time::timeout_at(end, work)
            .await
            .unwrap_or_else(|_| DoctorRow::late(late_detail)),
        None => work.await,
    })
}

/// Probe HTTPS (`ech=`) records for `hops` using `records`.
pub async fn probe_hops_with(records: &dyn Records, hops: EchHops, timeout: Duration) -> DoctorRow {
    let opts = options();
    let deadline = Instant::now() + timeout;
    let probed = futures::future::join_all(
        hops.target_hosts()
            .into_iter()
            .map(|(hop, host)| probe_one(records, hop, host, deadline)),
    )
    .await;
    DoctorRow {
        mode: opts.setting.word().to_string(),
        grease: opts.grease,
        hops: probed,
        note: None,
    }
}

async fn probe_one(
    records: &dyn Records,
    hop: &'static str,
    host: String,
    deadline: Instant,
) -> HopProbe {
    match records.ech_list(&host, deadline).await {
        Ok(Some(published)) => match parse_ech_config_list(&published.list) {
            Ok(infos) => {
                let first = &infos[0];
                let (name_fits, detail) = match check_name_length(&host, first) {
                    Ok(()) => (
                        Some(true),
                        format!(
                            "public_name={} max_name_length={} fits=yes",
                            first.public_name, first.max_name_length
                        ),
                    ),
                    Err(err) => (
                        Some(false),
                        format!(
                            "public_name={} max_name_length={} fits=no ({err})",
                            first.public_name, first.max_name_length
                        ),
                    ),
                };
                HopProbe {
                    hop,
                    host,
                    published: true,
                    public_name: Some(first.public_name.clone()),
                    max_name_length: Some(first.max_name_length),
                    name_fits,
                    detail,
                }
            }
            Err(err) => {
                HopProbe::without_info(hop, host, true, format!("invalid ECHConfigList: {err}"))
            }
        },
        Ok(None) => {
            HopProbe::without_info(hop, host, false, "no ech parameter in HTTPS record".into())
        }
        Err(err) => HopProbe::without_info(
            hop,
            host,
            false,
            format!("HTTPS record query failed: {err}"),
        ),
    }
}

#[cfg(test)]
mod tests;
