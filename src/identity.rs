//! The identity of an install in the link (T-261): one Ed25519 key, its
//! fingerprint and an allow list of peers.
//!
//! `pod-identity` of PodUtils/podcrates does the work (call C118): the key
//! file, the key, the fingerprints and the display form. This module is the
//! glue. It keeps the key of the install in the state directory
//! ([`key_path`]), gives each failure the exit code of podnet, and wraps the
//! allow list: default deny, 4096 entries at most ([`MAX_ALLOWED`]), and a
//! line in the display form ([`Identity::fingerprint`]).
//!
//! The seed is in the key file only. It is not in [`Identity`]'s `Debug`
//! output, in [`Report`], in a log line or in an error.

use std::collections::BTreeSet;
use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use pod_core::entropy::OsEntropy;
use pod_core::files::{read_own, Others};
use pod_identity::file::{self, Place};
use pod_identity::{DisplayFingerprint, KeyName, PublicKey};
use serde::Serialize;

use crate::exit::{self, Fault};

/// The name of the key file in the state directory.
pub const KEY_FILE: &str = "link-identity.key";

/// The most entries of an allow list.
pub const MAX_ALLOWED: usize = 4096;

/// The path of the key file in `dir`.
pub fn key_path(dir: &Path) -> PathBuf {
    dir.join(KEY_FILE)
}

/// The state directory of this process, where the key is kept. A host with
/// `PODNET_STATE_DIR=none` keeps no file, so it has no key (exit 78).
pub fn default_dir() -> anyhow::Result<PathBuf> {
    match crate::state::state_dir()? {
        Some(dir) => Ok(dir),
        None => Err(exit::fault(
            Fault::Config,
            anyhow!("the link key needs a state directory: PODNET_STATE_DIR is none"),
        )),
    }
}

/// The key of an install.
pub struct Identity {
    inner: pod_identity::Identity,
    path: PathBuf,
    made: bool,
}

impl Identity {
    /// The key file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether this call made the key file.
    pub fn made(&self) -> bool {
        self.made
    }

    /// The public key.
    pub fn public(&self) -> PublicKey {
        self.inner.public()
    }

    /// The fingerprint to show: base32 in five groups of eight characters
    /// with a checksum.
    pub fn fingerprint(&self) -> String {
        self.public().display_fingerprint()
    }

    /// The `pod-identity` key, for the handshake of the link.
    pub fn key(&self) -> &pod_identity::Identity {
        &self.inner
    }

    /// What a command prints for `--json`: no secret.
    pub fn report(&self) -> Report {
        let public = self.public();
        Report {
            fingerprint: self.fingerprint(),
            ssh_fingerprint: public.fingerprint(),
            public_key: public.hex(),
            path: self.path.display().to_string(),
            made: self.made,
        }
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint())
            .field("path", &self.path)
            .field("made", &self.made)
            .finish()
    }
}

/// The public facts of an [`Identity`], for `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The display form.
    pub fingerprint: String,
    /// The form of OpenSSH (`SHA256:`).
    pub ssh_fingerprint: String,
    /// The public key in hex.
    pub public_key: String,
    /// The key file.
    pub path: String,
    /// Whether this call made the key.
    pub made: bool,
}

/// The key of the install in `dir`, made with the mode 0600 when there is
/// none. A key file with a wide mode exits 71, and it is not replaced.
pub fn create(dir: &Path) -> anyhow::Result<Identity> {
    load(dir, true)
}

/// The key of the install in `dir`, never made. A missing key exits 66, and
/// a key file with a wide mode exits 71.
pub fn open(dir: &Path) -> anyhow::Result<Identity> {
    load(dir, false)
}

fn load(dir: &Path, make: bool) -> anyhow::Result<Identity> {
    let path = key_path(dir);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) => {
            let mode = meta.mode() & 0o7777;
            if mode & 0o077 != 0 {
                return Err(exit::fault(
                    Fault::OsError,
                    anyhow!(
                        "{}: the mode is {mode:04o}, and the key file is for this user only: run chmod 600 on it",
                        path.display()
                    ),
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !make {
                return Err(exit::fault(
                    Fault::NoInput,
                    anyhow!("{}: there is no key of this install yet", path.display()),
                ));
            }
        }
        Err(e) => {
            return Err(exit::fault(
                Fault::OsError,
                anyhow!("{}: {e}", path.display()),
            ))
        }
    }
    let dirs = [dir.to_path_buf()];
    let place = Place::File(path.clone());
    let os = |why: String| exit::fault(Fault::OsError, anyhow!(why));
    let key = if make {
        file::load_in(&dirs, &place, &mut OsEntropy).map_err(os)?
    } else {
        match file::existing_in(&dirs, &place).map_err(os)? {
            Some(inner) => file::Key {
                identity: inner,
                path: Some(path.clone()),
                made: false,
            },
            None => {
                return Err(exit::fault(
                    Fault::NoInput,
                    anyhow!("{}: there is no key of this install yet", path.display()),
                ))
            }
        }
    };
    hide_key_file(&path);
    Ok(Identity {
        inner: key.identity,
        path,
        made: key.made,
    })
}

/// Tell the reports of this process to hide the text of the key file, so
/// that no report, dump or log line shows the seed whatever its form.
fn hide_key_file(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines().map(str::trim).filter(|l| l.len() >= 16) {
        crate::report::hide(line);
    }
}

/// The most services in the `services=` list of one peer.
pub const MAX_SERVICES: usize = 64;

/// What one peer may do (T-290).
///
/// A bare key in the allow list has the open grant: every service, and
/// forwarding. A key with the option `services=` or `forward=` has a
/// restricted grant: only what the options name. `services=*` names every
/// service. An option that is absent grants nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// `None`: every service.
    services: Option<BTreeSet<String>>,
    forward: bool,
}

impl Default for Grant {
    fn default() -> Grant {
        Grant::open()
    }
}

impl Grant {
    /// Every service, and forwarding: the grant of a bare key.
    pub fn open() -> Grant {
        Grant {
            services: None,
            forward: true,
        }
    }

    /// No service and no forwarding. The peer may only hold the link.
    pub fn closed() -> Grant {
        Grant {
            services: Some(BTreeSet::new()),
            forward: false,
        }
    }

    /// Whether the peer may open the service `name`.
    pub fn allows_service(&self, name: &str) -> bool {
        self.services.as_ref().is_none_or(|s| s.contains(name))
    }

    /// Whether the peer may have this node relay a stream to another node.
    pub fn may_forward(&self) -> bool {
        self.forward
    }
}

/// One line of an allow list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    key: Key,
    grant: Grant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    /// The display form: a head of the fingerprint and a checksum.
    Display(DisplayFingerprint),
    /// A fingerprint of OpenSSH, or a key in hex or base32.
    Key(KeyName),
}

impl Entry {
    fn admits(&self, key: &PublicKey) -> bool {
        match &self.key {
            Key::Display(d) => d.matches(key),
            Key::Key(k) => k.matches(key),
        }
    }
}

/// The options after the key of a line: `services=a,b` and `forward=yes`.
/// Any other word is a comment, as before T-290.
fn grant_of(n: usize, words: &[&str]) -> anyhow::Result<Grant> {
    let mut services = None;
    let mut forward = None;
    for word in words {
        if word.starts_with('#') {
            break;
        }
        if let Some(list) = word.strip_prefix("services=") {
            let mut set = BTreeSet::new();
            for name in list.split(',') {
                if name == "*" {
                    set.insert("*".to_string());
                } else if let Err(e) = crate::link_service::check_name(name) {
                    return Err(bad_line(n, &format!("services: {e:#}")));
                } else {
                    set.insert(name.to_string());
                }
            }
            if set.len() > MAX_SERVICES {
                return Err(bad_line(
                    n,
                    &format!("a peer has {MAX_SERVICES} services at most"),
                ));
            }
            services = Some(if set.contains("*") { None } else { Some(set) });
        } else if let Some(value) = word.strip_prefix("forward=") {
            forward = Some(match value {
                "yes" => true,
                "no" => false,
                _ => return Err(bad_line(n, "forward= takes yes or no")),
            });
        }
    }
    if services.is_none() && forward.is_none() {
        return Ok(Grant::open());
    }
    Ok(Grant {
        services: services.unwrap_or_else(|| Some(BTreeSet::new())),
        forward: forward.unwrap_or(false),
    })
}

/// The peers that may use this install. The list is default deny: a list
/// with no entry admits no peer.
///
/// A file holds one peer on each line: the display fingerprint, the
/// `SHA256:` fingerprint, or the key in hex or base32. The options
/// `services=NAME,NAME` and `forward=yes` follow the key ([`Grant`]). Any
/// other word after the key is a comment, and a line that starts with `#`
/// is a comment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowList {
    entries: Vec<Entry>,
}

impl AllowList {
    /// The list in `text`. A line that is no key exits 78, and so does
    /// the 4097th entry.
    pub fn parse(text: &str) -> anyhow::Result<AllowList> {
        let mut entries = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let word = words.next().unwrap_or_default();
            let rest: Vec<&str> = words.collect();
            let grant = grant_of(n + 1, &rest)?;
            let key = match DisplayFingerprint::parse(word) {
                Ok(d) => Key::Display(d),
                Err(e) if word.chars().filter(|c| *c != '-').count() == 40 => {
                    return Err(bad_line(n + 1, &e.to_string()));
                }
                Err(_) => match KeyName::parse(word) {
                    Some(k) => Key::Key(k),
                    None => {
                        return Err(bad_line(
                            n + 1,
                            "it is no fingerprint (display form or SHA256:) and no key",
                        ))
                    }
                },
            };
            if entries.len() == MAX_ALLOWED {
                return Err(exit::fault(
                    Fault::Config,
                    anyhow!(
                        "line {}: the allow list holds {MAX_ALLOWED} entries at most",
                        n + 1
                    ),
                ));
            }
            entries.push(Entry { key, grant });
        }
        Ok(AllowList { entries })
    }

    /// The list in the file at `path`: a file of this user that others can
    /// read and cannot change. A missing file exits 66, and a file that
    /// others can change exits 71.
    pub fn read(path: &Path) -> anyhow::Result<AllowList> {
        match read_own(path, Others::Read) {
            Ok(Some(text)) => AllowList::parse(&text)
                .map_err(|e| e.context(format!("the allow list {}", path.display()))),
            Ok(None) => Err(exit::fault(
                Fault::NoInput,
                anyhow!("{}: there is no allow list", path.display()),
            )),
            Err(why) => Err(exit::fault(Fault::OsError, anyhow!(why))),
        }
    }

    /// The entries as key names, for a carrier that pins keys in its
    /// handshake (`pod_tls::peer::Keys`).
    pub fn names(&self) -> Vec<KeyName> {
        self.entries
            .iter()
            .map(|e| match &e.key {
                Key::Display(d) => KeyName::Display(*d),
                Key::Key(k) => k.clone(),
            })
            .collect()
    }

    /// Whether `key` is in the list.
    pub fn admits(&self, key: &PublicKey) -> bool {
        self.entries.iter().any(|e| e.admits(key))
    }

    /// What `key` may do: the grant of the first entry that admits it, or
    /// `None` for a key that is not in the list.
    pub fn grant(&self, key: &PublicKey) -> Option<Grant> {
        self.entries
            .iter()
            .find(|e| e.admits(key))
            .map(|e| e.grant.clone())
    }

    /// The check of the handshake, before any service: `Ok` for a peer in
    /// the list, else a refusal that exits 77 and names the peer.
    pub fn check(&self, key: &PublicKey) -> anyhow::Result<()> {
        if self.admits(key) {
            return Ok(());
        }
        Err(exit::fault(
            Fault::NoPermission,
            anyhow!(
                "the peer {} is not in the allow list ({} entries)",
                key.display_fingerprint(),
                self.entries.len()
            ),
        ))
    }

    /// How many entries the list holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list holds no entry (it admits no peer).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn bad_line(n: usize, why: &str) -> anyhow::Error {
    exit::fault(Fault::Config, anyhow!("line {n}: {why}"))
}
