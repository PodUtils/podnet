//! The stored WARP identity and its registration (T-241).
//!
//! A registration is two remote calls: `POST /reg` creates the device, and
//! `PATCH /reg/{id}` enrols the MASQUE key. The service counts the device
//! after the first call. This module does three things for that:
//!
//! * It takes a lock (`PATH.lock`) around the whole sequence, so that two
//!   processes make one device.
//! * It saves the device, its token and its key with the state `created`
//!   as soon as the first call answers. A later run sends the `PATCH` for
//!   the same device and makes no second `POST`.
//! * It saves the complete record (no `state`) after the `PATCH`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::register::{self, Device};
use super::{Identity, WarpClient};

/// The state of a record that holds a created device with no MASQUE key.
const CREATED: &str = "created";
/// How long a process waits for the lock of another process.
pub(crate) const LOCK_WAIT: Duration = Duration::from_secs(60);

#[derive(Serialize)]
struct Write<'a> {
    device: &'a Device,
    cert_pem: String,
    key_pem: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
}

#[derive(Deserialize)]
struct Read {
    device: Device,
    cert_pem: String,
    key_pem: String,
    #[serde(default)]
    state: Option<String>,
}

/// Write a record durably (mode 0600). `state` is `None` for a complete
/// record.
fn write(path: &Path, device: &Device, identity: &Identity, state: Option<&str>) -> Result<()> {
    let (cert_pem, key_pem) = identity.to_pem()?;
    let text = serde_json::to_string_pretty(&Write {
        device,
        cert_pem,
        key_pem,
        state,
    })
    .context("serialising the WARP identity")?;
    super::write_private(path, text.as_bytes())
        .with_context(|| format!("writing the WARP identity to {}", path.display()))
}

/// Write a complete record.
pub(super) fn save(path: &Path, device: &Device, identity: &Identity) -> Result<()> {
    write(path, device, identity, None)
}

/// Read a record: the device, the identity and the state.
fn read(path: &Path) -> Result<(Device, Identity, Option<String>)> {
    super::check_private(path)?;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the WARP identity from {}", path.display()))?;
    let stored: Read = serde_json::from_str(&text)
        .with_context(|| format!("parsing the WARP identity in {}", path.display()))?;
    if let Some(state) = stored.state.as_deref().filter(|s| *s != CREATED) {
        bail!(
            "the WARP identity in {} has the unknown state {state:?}",
            path.display()
        );
    }
    let identity = Identity::from_pem(&stored.cert_pem, &stored.key_pem)
        .context("rebuilding the MASQUE identity")?;
    Ok((stored.device, identity, stored.state))
}

/// Read a complete record. A record with the state `created` is refused.
pub(super) fn load(path: &Path) -> Result<(Device, Identity)> {
    let (device, identity, state) = read(path)?;
    if state.is_some() {
        bail!(
            "the WARP device in {} is created and not enrolled; run `podnet warp` to finish it",
            path.display()
        );
    }
    Ok((device, identity))
}

/// The lock file of `path`.
pub(crate) fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Take the lock of `path` and wait at most `wait` for it. The lock file
/// has mode 0600 and stays after use.
pub(crate) async fn lock(path: &Path, wait: Duration) -> Result<crate::instances::Held> {
    let file = lock_path(path);
    crate::state::check_file_writable(&file)?;
    let end = Instant::now() + wait;
    loop {
        if let Some(held) = crate::instances::try_lock(&file)? {
            return Ok(held);
        }
        if Instant::now() >= end {
            return Err(crate::exit::fault(
                crate::exit::Fault::TempFail,
                anyhow::anyhow!(
                    "another podnet is registering a WARP device (the lock {} is held)",
                    file.display()
                ),
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// [`WarpClient::register_or_load`] with a lock wait of `wait`.
pub(crate) async fn register_or_load(path: &Path, wait: Duration) -> Result<WarpClient> {
    let _held = lock(path, wait).await?;
    // Issue 178: a stat error is an error, not a missing file.
    if crate::state::exists(path)? {
        let (device, identity, state) = read(path)?;
        if state.is_some() {
            tracing::info!("the stored WARP device is not enrolled; finishing it");
            match finish(path, device, identity, false).await {
                Err(err) if gone(&err) => {
                    tracing::info!("the service has no such device; registering a new one");
                    std::fs::remove_file(path)
                        .with_context(|| format!("removing {}", path.display()))?;
                }
                other => return other,
            }
        } else {
            let client = WarpClient::load(path)?;
            if client.device.tunnel_type == "masque" {
                return Ok(client);
            }
            tracing::debug!("the stored device is not enrolled for MASQUE; registering again");
        }
    }
    // A failed save after the registration spends a device: check first.
    crate::state::check_file_writable(path)?;
    let identity = Identity::generate().context("generating the MASQUE identity")?;
    let device = register::create_device(None)
        .await
        .context("creating an anonymous WARP device")?;
    if device.token.is_empty() {
        bail!("the registration response carried no token");
    }
    write(path, &device, &identity, Some(CREATED))?;
    finish(path, device, identity, true).await
}

/// The service answered 401 or 404: it has no such device.
fn gone(err: &anyhow::Error) -> bool {
    matches!(register::api_status(err), Some(401 | 404))
}

/// Send the `PATCH` of a created device and save the complete record.
/// `fresh` is true for a device that this call made.
async fn finish(
    path: &Path,
    device: Device,
    identity: Identity,
    fresh: bool,
) -> Result<WarpClient> {
    let token = device.token.clone();
    let enrolled = register::enrol_masque(&device.id, &token, identity.spki_b64(), None).await;
    let mut enrolled = match enrolled {
        Ok(enrolled) => enrolled,
        Err(err) => {
            let err = err.context("enrolling the MASQUE identity");
            // An answer of the service is final. No answer (a network
            // error) leaves the record, and the next run resumes.
            if fresh && register::api_status(&err).is_some() {
                if let Err(del) = register::delete_device(&device.id, &token, None).await {
                    tracing::warn!(error = %del, "the new WARP device is not deleted");
                } else {
                    let _ = std::fs::remove_file(path);
                }
            }
            return Err(err);
        }
    };
    enrolled.token = token;
    if enrolled.tunnel_type != "masque" {
        bail!(
            "the API kept tunnel_type {:?} instead of masque",
            enrolled.tunnel_type
        );
    }
    save(path, &enrolled, &identity)?;
    Ok(WarpClient::from_parts(enrolled, identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_held_lock_gives_exit_75_after_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("warp.json");
        let _held = lock(&path, Duration::from_secs(1)).await.unwrap();
        let err = lock(&path, Duration::from_millis(200)).await.err().unwrap();
        assert_eq!(crate::exit::code_of(&err), 75, "{err:#}");
        assert!(format!("{err:#}").contains("another podnet"), "{err:#}");
    }
}
