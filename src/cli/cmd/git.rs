//! `podnet git`: git hosting with the installed `git` (T-094). `serve`
//! serves the repositories of a root over smart HTTP and SSH; `init`
//! makes a bare repository; `urls` prints the clone URLs and the
//! fingerprint of the host key.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use podnet::exit::{Fault, ResultExt};

pub(crate) use crate::cli::cmd::git_serve::GitServeArgs;

#[derive(Args, Debug)]
pub(crate) struct GitArgs {
    #[command(subcommand)]
    pub(crate) command: GitCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum GitCommand {
    /// Serve the git repositories under a root: smart HTTP on --local,
    /// through a quick tunnel or both, and SSH on --ssh or through a relay
    /// pair. It runs the installed git (git http-backend, git
    /// upload-pack, git receive-pack).
    ///
    /// A fetch needs the read role (--users, or --anon-read), a push needs
    /// the write role (--push-user, a write user of --users, or a key of
    /// --authorized-keys). Limits: the quick tunnel takes at most 100 MB
    /// in one request body, so a larger push over HTTPS fails (a clone has
    /// no limit); a session through the relay ends at 64 MiB, so a larger
    /// clone or push through the relay fails.
    Serve(Box<GitServeArgs>),
    /// Make a bare repository NAME under the root (NAME.git when NAME has
    /// no .git at its end).
    Init {
        /// The directory that holds the repositories.
        #[arg(long, value_name = "DIR")]
        root: PathBuf,
        /// The path of the new repository under the root (a/b makes
        /// ROOT/a/b.git).
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Print the clone URLs of each repository under the root, from the
    /// last git serve of the root, and the fingerprint of the SSH host
    /// key.
    Urls {
        /// The directory that holds the repositories.
        #[arg(long, value_name = "DIR")]
        root: PathBuf,
        /// The SSH host key (default: git/ssh_host_ed25519_key in the
        /// state directory).
        #[arg(long, value_name = "FILE")]
        host_key: Option<PathBuf>,
        /// Print one JSON document.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) async fn git(args: GitArgs) -> Result<()> {
    match args.command {
        GitCommand::Serve(args) => crate::cli::cmd::git_serve::serve(*args).await,
        GitCommand::Init { root, name } => init(&root, &name).await,
        GitCommand::Urls {
            root,
            host_key,
            json,
        } => urls(&root, host_key, json),
    }
}

fn usage(message: String) -> anyhow::Error {
    podnet::exit::fault(Fault::Usage, anyhow::anyhow!(message))
}

/// A root that is a directory, as its real path.
pub(crate) fn real_root(root: &Path) -> Result<PathBuf> {
    if !root.is_dir() {
        return Err(usage(format!("--root {}: not a directory", root.display())));
    }
    root.canonicalize()
        .with_context(|| format!("--root {}", root.display()))
}

/// `podnet git init --root DIR NAME`.
async fn init(root: &Path, name: &str) -> Result<()> {
    let root = real_root(root)?;
    let dir = podnet::git::create_repo(&root, name, std::ffi::OsStr::new("git"))
        .await
        .map_err(|err| usage(format!("git init {name}: {err}")))?;
    println!("{}", dir.display());
    Ok(())
}

/// The default host key: `git/ssh_host_ed25519_key` in the state
/// directory.
pub(crate) fn default_host_key() -> Result<PathBuf> {
    let dir = podnet::state::state_dir()?
        .context("no state directory (set XDG_STATE_HOME or HOME)")?
        .join("git");
    Ok(dir.join("ssh_host_ed25519_key"))
}

/// The file of the last `git serve` of `root` (its real path) in the
/// state directory.
pub(crate) fn record_path(root: &Path) -> Result<PathBuf> {
    // FNV-1a of the path: one file for each root.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let dir = podnet::state::state_dir()?
        .context("no state directory (set XDG_STATE_HOME or HOME)")?
        .join("git");
    Ok(dir.join(format!("serve-{hash:016x}.json")))
}

/// The bases of the clone URLs of a record of `git serve`: each one with
/// the form of its URL.
pub(crate) fn clone_urls(record: &serde_json::Value, repo: &str) -> Vec<String> {
    let mut out = Vec::new();
    let list = |key: &str| -> Vec<String> {
        record[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    for base in list("http") {
        out.push(format!("{}/{repo}", base.trim_end_matches('/')));
    }
    if let Some(url) = record["tunnel"].as_str() {
        out.push(format!("{}/{repo}", url.trim_end_matches('/')));
    }
    for base in list("ssh") {
        out.push(format!("{}/{repo}", base.trim_end_matches('/')));
    }
    if let Some(pair) = record["pair"].as_str() {
        out.push(format!(
            "GIT_SSH_COMMAND=\"ssh -o ProxyCommand='podnet relay connect {pair}'\" git clone ssh://git@{pair}/{repo}"
        ));
    }
    out
}

/// True when the process `pid` runs.
fn runs(pid: u64) -> bool {
    pid > 0 && Path::new(&format!("/proc/{pid}")).exists()
}

/// `podnet git urls --root DIR`.
fn urls(root: &Path, host_key: Option<PathBuf>, json: bool) -> Result<()> {
    let root = real_root(root)?;
    let record = match std::fs::read(record_path(&root)?) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::Null,
    };
    let running = record["pid"].as_u64().is_some_and(runs);
    let key_path = match host_key {
        Some(path) => path,
        None => default_host_key()?,
    };
    let key = if key_path.exists() {
        Some(podnet::git::ssh::load_or_create_host_key(&key_path).fault(Fault::Config)?)
    } else {
        None
    };
    let fingerprint = key.as_ref().map(podnet::git::ssh::fingerprint);
    let public = key.as_ref().map(podnet::git::ssh::public_line);
    let repos: Vec<serde_json::Value> = podnet::git::repos(&root)
        .into_iter()
        .map(|repo| serde_json::json!({ "repo": repo, "urls": clone_urls(&record, &repo) }))
        .collect();
    if json {
        let doc = serde_json::json!({
            "root": root.display().to_string(),
            "running": running,
            "repos": repos,
            "fingerprint": fingerprint,
            "host_key": public,
        });
        println!("{}", podnet::status::stamped(doc));
        return Ok(());
    }
    if record.is_null() {
        println!("podnet: no git serve of {} is recorded", root.display());
    } else if !running {
        println!(
            "podnet: the last git serve of {} does not run now",
            root.display()
        );
    }
    for repo in &repos {
        println!("{}", repo["repo"].as_str().unwrap_or_default());
        for url in repo["urls"].as_array().into_iter().flatten() {
            println!("  {}", url.as_str().unwrap_or_default());
        }
    }
    match (&fingerprint, &public) {
        (Some(fingerprint), Some(public)) => {
            println!("host key  {fingerprint}");
            println!("known_hosts  HOST {public}");
        }
        _ => println!("host key  none yet: git serve --ssh makes it"),
    }
    Ok(())
}
