//! `podnet update`: replace this binary with the binary of a release, after
//! these checks:
//!
//! 1. `SHA256SUMS.sigstore.json` (the cosign bundle) holds a certificate
//!    that chains to the Fulcio roots of Sigstore (`src/sigstore-fulcio.pem`)
//!    at the time of its log entry, for code signing.
//! 2. The certificate names `release.yml` of this repository at the tag of
//!    the release (`refs/tags/vX`), and the issuer of GitHub Actions.
//! 3. The signature of the bundle is valid over `SHA256SUMS`, with the key
//!    of the certificate, and the digest of the bundle is the SHA-256 of
//!    `SHA256SUMS`.
//! 4. The SHA-256 of the downloaded binary is its line in `SHA256SUMS`.
//! 5. The new binary runs and prints the version of the release.
//!
//! podnet does not check the inclusion proof of the transparency log (Rekor).
//! All downloads use the TLS roots of [`crate::trust`] and the proxy of
//! [`crate::egress`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use base64::Engine;
use serde::Deserialize;

/// The repository of the releases.
pub const REPOSITORY: &str = "PodUtils/podnet";
/// The issuer of the GitHub Actions tokens.
pub const ISSUER: &str = "https://token.actions.githubusercontent.com";
/// The Fulcio intermediate and root certificates of Sigstore (the public
/// instance), from the trusted root of sigstore/root-signing.
pub const FULCIO_PEM: &[u8] = include_bytes!("sigstore-fulcio.pem");
/// The target triple of this binary.
pub const TARGET: &str = env!("PODNET_BUILD_TARGET");

/// The release asset for the target `target`: the static musl binary of the
/// same architecture (a release ships no glibc binary, and a static binary
/// runs on a glibc host too).
pub fn asset_target(target: &str) -> String {
    target
        .replace("-gnueabihf", "-musleabihf")
        .replace("-gnueabi", "-musleabi")
        .replace("-gnu", "-musl")
}

/// One release of the GitHub API.
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

impl Release {
    /// The release of `tag` with no request to the GitHub API (issue 166):
    /// the URLs of its files follow from the tag. The checks of the
    /// signature and of SHA-256 do not change.
    pub fn pinned(tag: &str) -> Release {
        let asset = |name: String| Asset {
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/{tag}/{name}"
            ),
            name,
        };
        Release {
            tag_name: tag.to_string(),
            prerelease: tag.contains('-'),
            draft: false,
            assets: vec![
                asset(format!("podnet-{}", asset_target(TARGET))),
                asset("SHA256SUMS".into()),
                asset("SHA256SUMS.sigstore.json".into()),
            ],
        }
    }

    fn asset(&self, name: &str) -> Result<&Asset> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .with_context(|| format!("the release {} has no file {name}", self.tag_name))
    }
}

/// The identity that the certificate of `tag` must name.
pub fn identity(tag: &str) -> String {
    identity_in(REPOSITORY, tag)
}

/// The identity of `tag` of the repository `repository`.
pub fn identity_in(repository: &str, tag: &str) -> String {
    format!("https://github.com/{repository}/.github/workflows/release.yml@refs/tags/{tag}")
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
/// redirects.
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
                    .context(format!("GET {}", url.split('?').next().unwrap_or(""))))
            }
        }
    }
    bail!("too many redirects")
}

/// The releases, newest first.
pub async fn releases() -> Result<Vec<Release>> {
    let body = get(
        &format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=30"),
        Duration::from_secs(30),
        crate::fetch::MAX_BODY,
    )
    .await
    .context("reading the list of releases")?;
    serde_json::from_slice(&body).context("decoding the list of releases")
}

/// True when `tag` can be a release tag: 1 to 64 characters of
/// `0-9 A-Z a-z . + -`, the first `v` or a digit, and no `..`. Issue 227: a
/// tag goes into the URLs of the files and into the identity of the
/// signature, so a `/` or a `..` must not pass.
pub fn valid_tag(tag: &str) -> bool {
    (1..=64).contains(&tag.len())
        && tag.starts_with(|c: char| c == 'v' || c.is_ascii_digit())
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
        && !tag.contains("..")
}

/// The tags of a releases.atom feed, newest first. A text that is not a
/// [`valid_tag`] is skipped.
pub fn atom_tags(feed: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for part in feed.split("/releases/tag/").skip(1) {
        let tag: String = part
            .chars()
            .take_while(|c| !matches!(c, '"' | '<' | '\'' | ' '))
            .collect();
        if valid_tag(&tag) && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

/// The newest tag of releases.atom that `current` takes: a prerelease
/// (a tag with `-`) only when this binary is a prerelease, as [`choose`].
pub fn newest_in_atom(feed: &str, current: &str) -> Option<String> {
    let pre = current.contains('-');
    atom_tags(feed)
        .into_iter()
        .filter(|tag| pre || !tag.contains('-'))
        .max_by(|a, b| version_order(a, b))
}

/// The order of two tags as versions (`v0.3.0-beta.10` after
/// `v0.3.0-beta.9`; a release after its prereleases). The GitHub API and
/// the feed do not give the newest first: the API lists by tag name, so
/// `v0.3.0-beta.10` came after `v0.3.0-beta.7`, and an update took
/// `v0.3.0-beta.9` as the newest.
pub fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    fn key(tag: &str) -> (Vec<u64>, bool, Vec<(u64, String)>) {
        let tag = tag.trim_start_matches('v');
        let (core, pre) = match tag.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (tag, None),
        };
        let core = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        // A numeric part compares as a number, a word as text.
        let parts = pre
            .map(|p| {
                p.split('.')
                    .map(|x| match x.parse::<u64>() {
                        Ok(n) => (n, String::new()),
                        Err(_) => (0, x.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        (core, pre.is_none(), parts)
    }
    let (ca, sa, pa) = key(a);
    let (cb, sb, pb) = key(b);
    ca.cmp(&cb).then(sa.cmp(&sb)).then_with(|| {
        let word = |v: &Vec<(u64, String)>| v.iter().map(|(_, w)| w.clone()).collect::<Vec<_>>();
        word(&pa).cmp(&word(&pb)).then(pa.cmp(&pb))
    })
}

/// The newest release: the GitHub API, else releases.atom (issue 166: the
/// API answers 403 on a shared address; the feed has no quota).
async fn newest(current: &str) -> Result<Release> {
    let api = match releases().await {
        Ok(list) => return Ok(choose(&list, None, current)?.clone()),
        Err(err) => err,
    };
    tracing::info!(error = %format!("{api:#}"), "the GitHub API did not answer; reading releases.atom");
    let feed = get(
        &format!("https://github.com/{REPOSITORY}/releases.atom"),
        Duration::from_secs(30),
        crate::fetch::MAX_BODY,
    )
    .await
    .map_err(|atom| anyhow::anyhow!("{api:#}; releases.atom: {atom:#}"))
    .context("reading the list of releases")?;
    let tag = newest_in_atom(&String::from_utf8_lossy(&feed), current)
        .context("releases.atom has no release")?;
    Ok(Release::pinned(&tag))
}

/// The release to install: `tag`, or the newest release. A prerelease is
/// taken only with `tag`, or when this binary is a prerelease.
pub fn choose<'a>(
    releases: &'a [Release],
    tag: Option<&str>,
    current: &str,
) -> Result<&'a Release> {
    if let Some(tag) = tag {
        return releases
            .iter()
            .find(|r| r.tag_name == tag && !r.draft)
            .with_context(|| format!("no release has the tag {tag}"));
    }
    let pre = current.contains('-');
    releases
        .iter()
        .filter(|r| !r.draft && (pre || !r.prerelease) && valid_tag(&r.tag_name))
        .max_by(|a, b| version_order(&a.tag_name, &b.tag_name))
        .context("no release found")
}

mod cosign;
pub use cosign::{sum_of, verify_bundle, verify_bundle_for};

/// What `podnet update` did.
#[derive(Debug)]
pub enum Outcome {
    UpToDate(String),
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

/// The step before a download: nothing to do, only a report, or an install.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    UpToDate,
    Same,
    Available,
    Install,
}

/// What to do with the release `tag` (issue 197: `--version` of the
/// version of this binary does nothing, unless `force`; issue 275: also
/// with no `--version`, `force` installs the version of this binary again).
pub fn plan(tag: &str, current: &str, pinned: bool, check: bool, force: bool) -> Plan {
    if tag == current && !force {
        return if pinned { Plan::Same } else { Plan::UpToDate };
    }
    // With no --version, an older release is not an update: podnet never
    // goes back on its own (--version goes back on purpose). `force` does
    // not change this.
    if !pinned
        && !(force && tag == current)
        && version_order(tag, current) != std::cmp::Ordering::Greater
    {
        return Plan::UpToDate;
    }
    if check {
        Plan::Available
    } else {
        Plan::Install
    }
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

/// Update this binary (`check`: only say what is available; `force`:
/// install the version of this binary again).
pub async fn run(tag: Option<&str>, check: bool, force: bool) -> Result<Outcome> {
    if let Some(warning) = ignored_repo_warning(std::env::var_os("PODNET_REPO")) {
        tracing::warn!("{warning}");
    }
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    if let Some(tag) = tag.filter(|tag| !valid_tag(tag)) {
        bail!("not a release tag: {tag:?}");
    }
    // Issue 166: a pinned tag needs no list of the releases.
    let release = match tag {
        Some(tag) => Release::pinned(tag),
        None => newest(&current).await?,
    };
    let step = plan(&release.tag_name, &current, tag.is_some(), check, force);
    let name = format!("podnet-{}", asset_target(TARGET));
    let sums_url = release.asset("SHA256SUMS")?.browser_download_url.clone();
    let sums = match step {
        Plan::UpToDate => return Ok(Outcome::UpToDate(current)),
        Plan::Same => return Ok(Outcome::Same(current)),
        // A pinned tag: SHA256SUMS shows that the release exists.
        Plan::Available if tag.is_none() => None,
        Plan::Available | Plan::Install => Some(
            get(&sums_url, Duration::from_secs(60), crate::fetch::MAX_BODY)
                .await
                .map_err(|err| {
                    let missing = err
                        .chain()
                        .any(|e| matches!(e.downcast_ref::<HttpStatus>(), Some(HttpStatus(404))));
                    if missing {
                        anyhow::anyhow!("no release has the tag {}", release.tag_name)
                    } else {
                        err.context("SHA256SUMS")
                    }
                })?,
        ),
    };
    if step == Plan::Available {
        return Ok(Outcome::Available {
            current,
            tag: release.tag_name.clone(),
        });
    }
    let sums = sums.expect("fetched for an install");
    let binary_url = release.asset(&name)?.browser_download_url.clone();
    let bundle_url = release
        .asset("SHA256SUMS.sigstore.json")?
        .browser_download_url
        .clone();
    let bundle = get(&bundle_url, Duration::from_secs(60), crate::fetch::MAX_BODY)
        .await
        .context("SHA256SUMS.sigstore.json")?;
    verify_bundle(&bundle, &sums, &release.tag_name)
        .with_context(|| format!("checking the signature of {}", release.tag_name))?;
    let want = sum_of(&sums, &name)?;
    let binary = get(&binary_url, Duration::from_secs(300), MAX_BINARY)
        .await
        .context(name.clone())?;
    let got = ring::digest::digest(&ring::digest::SHA256, &binary);
    if got.as_ref() != want.as_slice() {
        bail!("the SHA-256 of {name} is not its line in SHA256SUMS");
    }
    let path = std::env::current_exe()
        .context("finding this binary")?
        .canonicalize()
        .context("finding this binary")?;
    install(&path, &binary, &release.tag_name)?;
    Ok(Outcome::Updated {
        from: current,
        to: release.tag_name.clone(),
        path,
    })
}

/// How long the new binary may take to print its version.
pub const TRIAL_LIMIT: Duration = Duration::from_secs(10);

pub use crate::detach::previous_of;

/// Run `path --version` with no stdin, for at most `limit`. It must exit 0,
/// and its first line must start with `want`. A text file that is busy
/// (ETXTBSY, just after the write) is tried again.
pub fn trial(path: &Path, want: &str, limit: Duration) -> Result<()> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    const TEXT_BUSY: i32 = 26;
    let mut tries = 0;
    let mut child = loop {
        match Command::new(path)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => break child,
            Err(err) if err.raw_os_error() == Some(TEXT_BUSY) && tries < 10 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err).with_context(|| format!("running {}", path.display())),
        }
    };
    // Read stdout in a thread, so a long answer cannot block the child.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.by_ref().take(64 * 1024).read_to_end(&mut out);
        out
    });
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} did not answer --version in {} s",
                path.display(),
                limit.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = reader.join().unwrap_or_default();
    if !status.success() {
        bail!("{} --version exits with {status}", path.display());
    }
    let text = String::from_utf8_lossy(&out);
    let first = text.lines().next().unwrap_or("");
    if !first.starts_with(want) {
        bail!("{} prints {:?}, not {want}…", path.display(), first.trim());
    }
    Ok(())
}

/// Sync the directory, so a rename stays after a power loss.
fn sync_dir(dir: &Path) {
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
}

/// Keep a copy of `path` as `previous`: a hard link, or a copy when a link
/// is not possible, put in place with a rename.
fn keep(path: &Path, previous: &Path) -> Result<()> {
    let tmp = previous.with_file_name(format!(".podnet-prev-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    if std::fs::hard_link(path, &tmp).is_err() {
        std::fs::copy(path, &tmp)
            .with_context(|| format!("keeping a copy of {}", path.display()))?;
    }
    std::fs::rename(&tmp, previous).with_context(|| format!("writing {}", previous.display()))
}

/// Put `binary` in place of `path`: a new file beside it (mode 0755), which
/// must pass [`trial`] for `tag`; then the binary before is kept as
/// [`previous_of`], and a rename puts the new file in place.
pub fn install(path: &Path, binary: &[u8], tag: &str) -> Result<()> {
    install_with(path, binary, tag, TRIAL_LIMIT)
}

/// The identity of a file: its device and its inode.
type FileId = (u64, u64);

/// Check that `path` is still the file `id` of the trial: a regular file
/// (not a symlink) of this user that other users cannot write (issue 236).
/// A check by the path leaves a short window before the rename; a process
/// that can write the directory can also rename its own file over the
/// binary, so the check is for a file of this user in a shared directory.
fn same_file(path: &Path, id: FileId) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let uid = unsafe { libc::geteuid() };
    if !meta.file_type().is_file()
        || (meta.dev(), meta.ino()) != id
        || meta.uid() != uid
        || meta.mode() & 0o022 != 0
    {
        bail!(
            "{} changed after its trial run; it is not installed",
            path.display()
        );
    }
    Ok(())
}

/// The identity of the file at `path` (not of a symlink target).
fn file_id(path: &Path) -> Result<FileId> {
    use std::os::unix::fs::MetadataExt;
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    Ok((meta.dev(), meta.ino()))
}

#[cfg(test)]
thread_local! {
    /// A test hook: it runs after the trial and before the rename.
    static AFTER_TRIAL: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn after_trial(path: &Path) {
    if let Some(hook) = AFTER_TRIAL.with(|h| h.get()) {
        hook(path);
    }
}

#[cfg(not(test))]
fn after_trial(_path: &Path) {}

fn install_with(path: &Path, binary: &[u8], tag: &str, limit: Duration) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let dir = path.parent().context("the binary has no directory")?;
    let tmp = dir.join(format!(".podnet-update-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(&tmp)
            .with_context(|| format!("writing in {} (no write permission?)", dir.display()))?;
        file.write_all(binary)?;
        // Issue 270: the umask masks the mode of the open; 0755 also with
        // a restrictive umask.
        file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        file.sync_all()?;
        let meta = file.metadata()?;
        let id = (meta.dev(), meta.ino());
        drop(file);
        let want = format!("podnet {} ", tag.trim_start_matches('v'));
        trial(&tmp, &want, limit).context("the new binary")?;
        after_trial(&tmp);
        same_file(&tmp, id).context("the new binary")?;
        keep(path, &previous_of(path))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        sync_dir(dir);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Put the kept binary ([`previous_of`]) back in place of `path`, with no
/// network. The kept binary must pass [`trial`] first. The binary that was
/// in place becomes the kept binary, so a second rollback goes forward
/// again. Gives the first line of `--version` of the binary now in place.
pub fn rollback(path: &Path) -> Result<String> {
    rollback_with(path, TRIAL_LIMIT)
}

fn rollback_with(path: &Path, limit: Duration) -> Result<String> {
    let previous = previous_of(path);
    if !previous.exists() {
        bail!(
            "no kept binary at {}: podnet update keeps one from 0.3.0-beta.5 on",
            previous.display()
        );
    }
    let id = file_id(&previous)?;
    trial(&previous, "podnet ", limit).context("the kept binary")?;
    after_trial(&previous);
    same_file(&previous, id).context("the kept binary")?;
    let dir = path.parent().context("the binary has no directory")?;
    let held = dir.join(format!(".podnet-rollback-{}", std::process::id()));
    let _ = std::fs::remove_file(&held);
    if std::fs::hard_link(path, &held).is_err() {
        std::fs::copy(path, &held).with_context(|| format!("keeping {}", path.display()))?;
    }
    std::fs::rename(&previous, path).with_context(|| format!("replacing {}", path.display()))?;
    std::fs::rename(&held, &previous).with_context(|| format!("writing {}", previous.display()))?;
    sync_dir(dir);
    let out = std::process::Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    Ok(out)
}

#[cfg(test)]
mod tests;
