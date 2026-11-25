//! `podnet update`: replace this binary with the binary of a release. The
//! checks are the checks of the crate `pod-release` of PodUtils/podcrates
//! (the ruling of 2026-11-23: podnet keeps no own verifier):
//!
//! 1. The bundle `SHA256SUMS.sigstore.json` signs `SHA256SUMS`
//!    ([`pod_release::verify_release`], with [`Roots::sigstore`]): the
//!    digest and the signature with the key of the leaf certificate; the
//!    entry of Rekor (its signed entry timestamp, or its inclusion proof
//!    with the signed checkpoint) and each RFC 3161 timestamp; the chain of
//!    the leaf to the Fulcio root of Sigstore at each time that the bundle
//!    proves, for code signing; the leaf names `release.yml` of this
//!    repository at the tag of the release (`refs/tags/vX`), and the issuer
//!    of GitHub Actions.
//! 2. Exactly one valid line of `SHA256SUMS` names the binary, and the
//!    SHA-256 of the downloaded binary is that line.
//! 3. The new binary runs beside the old one and prints
//!    `podnet VERSION` of the release ([`pod_release::update::install`]).
//!
//! All downloads use podnet's own fetch: the TLS roots of [`crate::trust`]
//! and the proxy of [`crate::egress`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use pod_release::trial::{self, Expect};
use pod_release::update::Plan;
use pod_release::verify::{BUNDLE, SUMS};
use pod_release::{version, Identity, Roots, Verified, Version};

pub use pod_release::keep::prev_of as previous_of;

/// The repository of the releases.
pub const REPOSITORY: &str = "PodUtils/podnet";
/// The release workflow, which signs `SHA256SUMS`.
pub const WORKFLOW: &str = "release.yml";
/// The name of the program in its `--version` line.
pub const PROGRAM: &str = "podnet";
/// The target triple of this binary.
pub const TARGET: &str = env!("PODNET_BUILD_TARGET");

/// The release asset of this binary: the static musl binary of the same
/// architecture (a release ships no glibc binary, and a static binary runs
/// on a glibc host too).
pub fn asset() -> String {
    version::asset(PROGRAM, &version::release_target(TARGET))
}

/// The identity that the leaf certificate of the release `tag` must name.
pub fn identity(tag: &str) -> Identity {
    Identity::github(REPOSITORY, WORKFLOW, tag)
}

/// The URL of the file `name` of the release `tag`.
pub fn file_url(tag: &str, name: &str) -> String {
    format!("https://github.com/{REPOSITORY}/releases/download/{tag}/{name}")
}

/// The version of the release tag `tag` (issue 227): the form of
/// [`version::valid_tag`] (no `/`, no `..`), and `v` with a version of
/// semver and no build part. A tag goes into the URLs of the files and
/// into the identity of the signature.
pub fn release_version(tag: &str) -> Option<Version> {
    Version::of_tag(tag).filter(|v| version::valid_tag(tag) && v.tag() == tag)
}

/// True when `tag` is a release tag ([`release_version`]).
pub fn valid_tag(tag: &str) -> bool {
    release_version(tag).is_some()
}

fn version_of(tag: &str) -> Result<Version> {
    release_version(tag).with_context(|| format!("not a release tag: {tag:?}"))
}

/// The version of this binary.
pub fn current() -> Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).context("the version of this binary is not semver")
}

/// The largest release binary that `podnet update` downloads. The binaries are
/// about 14 MB; [`crate::fetch::MAX_BODY`] (16 MiB) is too near.
pub const MAX_BINARY: usize = 128 << 20;

/// An HTTP status other than 200 and a redirect.
#[derive(Debug)]
pub struct HttpStatus(pub u16);

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}", self.0)
    }
}

impl std::error::Error for HttpStatus {}

/// GET `url` with a body of at most `max_body` bytes, and follow up to 5
/// redirects, on HTTPS alone.
async fn get(url: &str, timeout: Duration, max_body: usize) -> Result<Vec<u8>> {
    let mut url = url.to_string();
    for _ in 0..6 {
        let request = crate::fetch::Request::new("GET", url.clone())
            .header("Accept", "application/octet-stream, application/json")
            .timeout(timeout)
            .max_body(max_body);
        let response = crate::fetch::send(request).await?;
        match response.status {
            200 => return Ok(response.body),
            301 | 302 | 303 | 307 | 308 => {
                url = response
                    .header("location")
                    .context("a redirect with no Location")?
                    .to_string();
                if !url.starts_with("https://") {
                    bail!("a redirect away from HTTPS");
                }
            }
            status => {
                return Err(anyhow::Error::new(HttpStatus(status))
                    .context(format!("GET {}", pod_release::fetch::shown(&url))))
            }
        }
    }
    bail!("too many redirects")
}

/// True when `err` is an answer of HTTP 404.
fn not_found(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|e| matches!(e.downcast_ref::<HttpStatus>(), Some(HttpStatus(404))))
}

/// The newest release that `current` follows (a prerelease only when
/// `current` is one): the GitHub API, else releases.atom (issue 166: the
/// API answers 403 on a shared address; the feed has no quota).
async fn newest(current: &Version) -> Result<Version> {
    let pre = current.is_prerelease();
    let api = async {
        let body = get(
            &format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=30"),
            Duration::from_secs(30),
            crate::fetch::MAX_BODY,
        )
        .await?;
        version::tags_of_api(&body).map_err(|e| anyhow::anyhow!("decoding the list: {e}"))
    };
    let api = match api.await.context("reading the list of releases") {
        Ok(tags) => return version::newest(&tags, pre).context("no release found"),
        Err(err) => err,
    };
    // GitHub #751: the feed goes through the same proxy. A proxy that does
    // not answer (or refuses) fails the feed too, after its time limit
    // again: the error of the API is the error of the command, with the
    // step of the proxy hop.
    if crate::hop::of(&api) == Some(crate::hop::Hop::Proxy) {
        return Err(api);
    }
    tracing::info!(error = %format!("{api:#}"), "the GitHub API did not answer; reading releases.atom");
    let feed = get(
        &format!("https://github.com/{REPOSITORY}/releases.atom"),
        Duration::from_secs(30),
        crate::fetch::MAX_BODY,
    )
    .await
    // The error of the API starts with "reading the list of releases": the
    // line names it once.
    .map_err(|atom| anyhow::anyhow!("{api:#}; releases.atom: {atom:#}"))?;
    let tags = version::tags_of_atom(&feed).map_err(|e| anyhow::anyhow!("releases.atom: {e}"))?;
    version::newest(&tags, pre).context("releases.atom has no release")
}

/// What `podnet update` did.
#[derive(Debug)]
pub enum Outcome {
    UpToDate(String),
    /// This binary is newer than the newest release: nothing to do.
    Newer {
        current: String,
        tag: String,
    },
    /// `--version` names the version of this binary (issue 197).
    Same(String),
    Available {
        current: String,
        tag: String,
    },
    Updated {
        from: String,
        to: String,
        path: PathBuf,
    },
}

/// The warning when PODNET_REPO is set (issue 168): the repository of
/// `podnet update` is fixed, because it names the signer of the binary.
pub fn ignored_repo_warning(value: Option<std::ffi::OsString>) -> Option<String> {
    let value = value?;
    (value.to_string_lossy() != REPOSITORY).then(|| {
        format!(
            "PODNET_REPO={} is for scripts/install.sh; podnet update uses {REPOSITORY}",
            value.to_string_lossy()
        )
    })
}

/// `SHA256SUMS` of the release `version`; a 404 says that no release has
/// the tag.
async fn sums_of(version: &Version) -> Result<Vec<u8>> {
    let tag = version.tag();
    get(
        &file_url(&tag, SUMS),
        Duration::from_secs(60),
        crate::fetch::MAX_BODY,
    )
    .await
    .map_err(|err| {
        if not_found(&err) {
            anyhow::anyhow!("no release has the tag {tag}")
        } else {
            err.context(SUMS)
        }
    })
}

/// `sums` (the bytes of `SHA256SUMS` of `tag`) with the bundle `bundle`,
/// checked by the identity of `tag` and the roots of Sigstore.
pub fn verify(sums: &[u8], bundle: &[u8], tag: &str) -> Result<Verified> {
    pod_release::verify_release(sums, bundle, &identity(tag), &Roots::sigstore())
        .with_context(|| format!("checking the signature of {tag}"))
}

/// Update this binary (`check`: only say what is available; `force`:
/// install the version of this binary again).
pub async fn run(tag: Option<&str>, check: bool, force: bool) -> Result<Outcome> {
    if let Some(warning) = ignored_repo_warning(std::env::var_os("PODNET_REPO")) {
        tracing::warn!("{warning}");
    }
    let current = current()?;
    let pinned = tag.map(version_of).transpose()?;
    // Issue 166: a pinned tag needs no list of the releases.
    let release = match &pinned {
        Some(version) => version.clone(),
        None => newest(&current).await?,
    };
    let (from, to) = (current.tag(), release.tag());
    match pod_release::update::plan(&release, &current, pinned.is_some(), check, force) {
        Plan::UpToDate => return Ok(Outcome::UpToDate(from)),
        Plan::Newer => {
            return Ok(Outcome::Newer {
                current: from,
                tag: to,
            })
        }
        Plan::Same => return Ok(Outcome::Same(from)),
        // A pinned tag: SHA256SUMS shows that the release exists.
        Plan::Available => {
            if pinned.is_some() {
                sums_of(&release).await?;
            }
            return Ok(Outcome::Available {
                current: from,
                tag: to,
            });
        }
        Plan::Install => {}
    }
    let sums = sums_of(&release).await?;
    let bundle = get(
        &file_url(&to, BUNDLE),
        Duration::from_secs(60),
        crate::fetch::MAX_BODY,
    )
    .await
    .context(BUNDLE)?;
    let verified = verify(&sums, &bundle, &to)?;
    let name = asset();
    verified
        .sums
        .get(&name)
        .map_err(|why| anyhow::anyhow!("the release {to}: {why}"))?;
    let binary = get(&file_url(&to, &name), Duration::from_secs(300), MAX_BINARY)
        .await
        .context(name.clone())?;
    verified.check(&name, &binary)?;
    let path = pod_release::replace::running()
        .map_err(|why| anyhow::anyhow!("finding this binary: {why}"))?;
    install(&path, &binary, &to)?;
    Ok(Outcome::Updated { from, to, path })
}

/// Put `binary`, the release `tag`, in place of `path`: a new file beside
/// it with the mode of `path` (no write for others, no set-user-ID bit),
/// which must run here and print `podnet VERSION` of `tag` within
/// [`trial::LIMIT`] and stay the same file after that run; then the binary
/// before is kept as [`previous_of`], and a rename puts the new file in
/// place.
pub fn install(path: &Path, binary: &[u8], tag: &str) -> Result<()> {
    install_with(path, binary, tag, trial::LIMIT)
}

fn install_with(path: &Path, binary: &[u8], tag: &str, limit: Duration) -> Result<()> {
    let version = version_of(tag)?;
    let replaced = pod_release::update::install(path, binary, PROGRAM, &version, limit)?;
    if let Err(why) = replaced.prev {
        tracing::warn!("{tag} is installed; the binary before it is not kept: {why}");
    }
    Ok(())
}

/// Put the kept binary ([`previous_of`]) back in place of `path`, with no
/// network. The kept binary must run here and print `podnet VERSION` first,
/// and stay the same file after that run. The binary that was in place
/// becomes the kept binary, so a second rollback goes forward again. Gives
/// the first line of `--version` of the binary now in place.
pub fn rollback(path: &Path) -> Result<String> {
    rollback_with(path, trial::LIMIT)
}

fn rollback_with(path: &Path, limit: Duration) -> Result<String> {
    let expect = Expect {
        program: PROGRAM,
        version: None,
        limit,
    };
    let done = pod_release::keep::rollback(path, &expect)?;
    if let Err(why) = done.kept {
        tracing::warn!("the binary that was in place is not kept: {why}");
    }
    Ok(done.said)
}

#[cfg(test)]
mod tests;
