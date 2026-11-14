//! The object-storage carrier of the link (T-279, feature `s3`).
//!
//! `pod_link::s3` of PodUtils/podcrates does the work: each end writes its
//! bytes as objects of a bucket of the S3 API that both ends reach, and
//! reads the objects of the other end. TLS 1.3 (ALPN `podlink/1`) runs over
//! the objects, each side pinning the Ed25519 key of the other, so a reader
//! of the bucket learns nothing. Objects are 256 KiB, 8 in flight, polled
//! from 100 ms to 5 s, with a budget of 1 GiB for each end of a session.
//! This module is the glue:
//!
//! * [`carrier`] makes the carrier from the key of this install, the allow
//!   list and the flags in [`Tune`]: the endpoint (`--s3-endpoint`, no
//!   default: podnet knows no host of a provider), the region
//!   (`--s3-region`, default `us-east-1`) and the removal of each object
//!   read (`--s3-prune`, off: the bucket can be shared).
//! * The key of the bucket comes from `--s3-credentials FILE` (a profile
//!   file), else from the environment ([`ACCESS_KEY_VARIABLE`],
//!   [`SECRET_KEY_VARIABLE`], [`SESSION_TOKEN_VARIABLE`]), else from the
//!   profile file of [`CREDENTIALS_FILE_VARIABLE`] or
//!   `$HOME/.aws/credentials`. The section is [`PROFILE_VARIABLE`], else
//!   `default`. The key never comes from `argv`. No key exits 77, and the
//!   message names no value.
//! * [`listen`] takes the sessions under `BUCKET[/PREFIX]`; [`dial`] opens
//!   a session at `KEY@BUCKET[/PREFIX]` and pins `KEY`.
//! * A key that the bucket refuses (403) exits 77, as a key that a
//!   handshake refuses ([`crate::link::carrier_fault`]).

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use pod_identity::PublicKey;
use pod_link::s3::{Credentials, Endpoint, S3};
use pod_link::{CarrierError, CarrierErrorKind, Link, Settings};

use crate::exit::{self, Fault};
use crate::identity::{AllowList, Identity};
use crate::link::{carrier_fault, Door};
use crate::link_keyed::{dialed, Tune};
use crate::link_probe::{self, Probe};
use crate::link_tcp::{key_of, keys, provider};

/// The id of the key of the bucket.
pub const ACCESS_KEY_VARIABLE: &str = "AWS_ACCESS_KEY_ID";
/// The secret of the key of the bucket.
pub const SECRET_KEY_VARIABLE: &str = "AWS_SECRET_ACCESS_KEY";
/// The token of a temporary key.
pub const SESSION_TOKEN_VARIABLE: &str = "AWS_SESSION_TOKEN";
/// The section of a profile file.
pub const PROFILE_VARIABLE: &str = "AWS_PROFILE";
/// The profile file, in place of `$HOME/.aws/credentials`.
pub const CREDENTIALS_FILE_VARIABLE: &str = "AWS_SHARED_CREDENTIALS_FILE";

/// The key of section `profile` of the text of a profile file (the INI
/// form: `[NAME]`, then `aws_access_key_id`, `aws_secret_access_key` and
/// `aws_session_token`).
pub fn credentials_in(text: &str, profile: &str) -> Option<Credentials> {
    let (mut section, mut id, mut secret, mut token) = (String::new(), None, None, None);
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if section != profile {
            continue;
        }
        let value = Some(value.trim().to_string()).filter(|v| !v.is_empty());
        match key.trim() {
            "aws_access_key_id" => id = value,
            "aws_secret_access_key" => secret = value,
            "aws_session_token" => token = value,
            _ => {}
        }
    }
    let mut key = Credentials::new(id?, secret?);
    key.session_token = token;
    Some(key)
}

/// The key of the environment that `var` reads, else of the profile file
/// `file`. With `file` given by the operator, the file alone.
pub fn credentials_of(
    var: impl Fn(&str) -> Option<String>,
    file: Option<&Path>,
    given: bool,
) -> Option<Credentials> {
    let var = |name: &str| var(name).filter(|v| !v.is_empty());
    if !given {
        if let (Some(id), Some(secret)) = (var(ACCESS_KEY_VARIABLE), var(SECRET_KEY_VARIABLE)) {
            let mut key = Credentials::new(id, secret);
            key.session_token = var(SESSION_TOKEN_VARIABLE);
            return Some(key);
        }
    }
    let text = std::fs::read_to_string(file?).ok()?;
    let profile = var(PROFILE_VARIABLE).unwrap_or_else(|| "default".into());
    credentials_in(&text, &profile)
}

/// The key of the bucket for `tune`: see the module. No key exits 77.
pub fn credentials(tune: &Tune) -> anyhow::Result<Credentials> {
    let env = |name: &str| std::env::var(name).ok();
    let file = match &tune.s3_credentials {
        Some(file) => Some(file.clone()),
        None => env(CREDENTIALS_FILE_VARIABLE)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| Path::new(&home).join(".aws/credentials"))),
    };
    credentials_of(env, file.as_deref(), tune.s3_credentials.is_some()).ok_or_else(|| {
        exit::fault(
            Fault::NoPermission,
            anyhow!(
                "the s3 carrier: no key of the bucket: set {ACCESS_KEY_VARIABLE} and \
                 {SECRET_KEY_VARIABLE}, or give a profile file (--s3-credentials FILE)"
            ),
        )
    })
}

fn usage(text: String) -> anyhow::Error {
    exit::fault(Fault::Usage, anyhow!("the s3 carrier: {text}"))
}

/// A failure of the carrier: a key that the bucket refused exits 77, the
/// others as [`carrier_fault`].
pub fn fault(error: CarrierError) -> anyhow::Error {
    if error.kind == CarrierErrorKind::Refused {
        return exit::fault(Fault::NoPermission, anyhow::Error::new(error));
    }
    carrier_fault(error)
}

/// The endpoint of `tune`. No endpoint, or one that does not parse, exits
/// 64.
pub fn endpoint_of(tune: &Tune) -> anyhow::Result<Endpoint> {
    let Some(text) = &tune.s3_endpoint else {
        return Err(usage(
            "give the endpoint of the S3 API with --s3-endpoint http[s]://HOST[:PORT][/PATH]"
                .into(),
        ));
    };
    Endpoint::parse(text).map_err(usage)
}

/// The carrier of this install with the key `key` of the bucket: it
/// presents the key of `me`, a listener takes the clients of `allow`, and
/// each request goes through the way out of the process. No endpoint, or
/// one that does not parse, exits 64.
pub fn carrier_with(
    me: &Identity,
    allow: &AllowList,
    tune: &Tune,
    key: Credentials,
) -> anyhow::Result<S3> {
    let endpoint = endpoint_of(tune)?;
    let mut s3 = S3::new(
        provider(),
        key_of(me),
        crate::egress::egress(None),
        endpoint,
        key,
    )
    .with_clients(keys(allow));
    if let Some(region) = &tune.s3_region {
        s3 = s3.with_region(region.clone());
    }
    if s3.endpoint.secure {
        let tls = crate::trust::context()
            .client_config_with(&[], &[b"http/1.1"])
            .map_err(|e| {
                exit::fault(Fault::Config, anyhow!("the s3 carrier: the TLS roots: {e}"))
            })?;
        s3 = s3.with_tls(std::sync::Arc::new(tls));
    }
    s3.prune = tune.s3_prune;
    Ok(s3)
}

/// [`carrier_with`] with the key of [`credentials`]. The endpoint is
/// checked first (exit 64), then the key (exit 77).
pub fn carrier(me: &Identity, allow: &AllowList, tune: &Tune) -> anyhow::Result<S3> {
    endpoint_of(tune)?;
    carrier_with(me, allow, tune, credentials(tune)?)
}

/// Listen at `address` (`BUCKET[/PREFIX]`) through `s3`: the sessions that
/// open after the bind.
pub async fn listen(s3: &S3, address: &str) -> anyhow::Result<Door> {
    pod_link::Carrier::available(s3).map_err(fault)?;
    let listener = pod_link::Carrier::listen(s3, address)
        .await
        .map_err(fault)?;
    Ok(Door::from_listener(listener))
}

/// Connect to `address` (`KEY@BUCKET[/PREFIX]`) through `s3` as the
/// dialer. The handshake pins `KEY`. Then the key that the listener proved
/// must be in `allow` (else exit 77).
pub async fn dial(
    s3: &S3,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, bucket, prefix) = S3::address(address).map_err(fault)?;
    pod_link::Carrier::available(s3).map_err(fault)?;
    match s3.connect(&bucket, &prefix, &name).await {
        Err(e) if e.kind == CarrierErrorKind::Refused => Err(fault(e)),
        connected => dialed(connected, allow, settings),
    }
}

/// The carrier for the registry: the facts (the cost class `highest`) and
/// the host check, no key of this install and no endpoint (it never
/// dials).
pub fn probe() -> Probe<S3> {
    let endpoint = Endpoint::parse("http://127.0.0.1:9").expect("a fixed endpoint");
    let key = Credentials::new("", "");
    let s3 = S3::new(
        provider(),
        link_probe::key(),
        crate::egress::egress(None),
        endpoint,
        key,
    );
    Probe::new(s3)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "[default]\naws_access_key_id = A1\naws_secret_access_key = S1\n\n\
                        [other]\n# a comment\naws_access_key_id=A2\naws_secret_access_key=S2\n\
                        aws_session_token=T2\n";

    #[test]
    fn a_profile_file_gives_the_key_of_its_section() {
        let one = credentials_in(FILE, "default").unwrap();
        assert_eq!(
            (one.access_key.as_str(), one.secret_key.as_str()),
            ("A1", "S1")
        );
        assert_eq!(one.session_token, None);
        let two = credentials_in(FILE, "other").unwrap();
        assert_eq!(two.access_key, "A2");
        assert_eq!(two.session_token.as_deref(), Some("T2"));
        assert!(credentials_in(FILE, "none").is_none());
        assert!(credentials_in("[default]\naws_access_key_id=A\n", "default").is_none());
    }

    #[test]
    fn the_environment_comes_before_the_file_unless_the_file_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("credentials");
        std::fs::write(&file, FILE).unwrap();
        let env = |name: &str| match name {
            ACCESS_KEY_VARIABLE => Some("E1".to_string()),
            SECRET_KEY_VARIABLE => Some("ES".to_string()),
            _ => None,
        };
        assert_eq!(
            credentials_of(env, Some(&file), false).unwrap().access_key,
            "E1"
        );
        assert_eq!(
            credentials_of(env, Some(&file), true).unwrap().access_key,
            "A1"
        );
        let profile = |name: &str| (name == PROFILE_VARIABLE).then(|| "other".to_string());
        assert_eq!(
            credentials_of(profile, Some(&file), false)
                .unwrap()
                .access_key,
            "A2"
        );
        assert!(credentials_of(|_| None, None, false).is_none());
        assert!(credentials_of(|_| None, Some(&dir.path().join("no")), true).is_none());
    }

    #[test]
    fn no_endpoint_exits_64_and_a_refused_key_exits_77() {
        let dir = tempfile::tempdir().unwrap();
        let me = crate::identity::create(dir.path()).unwrap();
        let key = Credentials::new("A", "S");
        let e =
            carrier_with(&me, &AllowList::default(), &Tune::default(), key.clone()).unwrap_err();
        assert_eq!(exit::code_of(&e), 64, "{e:#}");
        let bad = Tune {
            s3_endpoint: Some("ftp://x".into()),
            ..Tune::default()
        };
        let e = carrier_with(&me, &AllowList::default(), &bad, key).unwrap_err();
        assert_eq!(exit::code_of(&e), 64, "{e:#}");
        let refused = CarrierError::new("s3", CarrierErrorKind::Refused, "403 AccessDenied");
        assert_eq!(exit::code_of(&fault(refused)), 77);
        let none = CarrierError::new("s3", CarrierErrorKind::NotFound, "404 NoSuchBucket");
        assert_eq!(exit::code_of(&fault(none)), 69);
    }
}
