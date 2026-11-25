//! `podnet warp up` and the ways that later shells join it (issue 78).
//!
//! `podnet warp up` starts `podnet warp proxy` detached (instance [`INSTANCE`])
//! with a TCP listener on 127.0.0.1 and a unix socket in the run directory,
//! and writes `warp.env` there: the proxy variables for the tools that read
//! them. Later shells join it in one of three ways:
//!
//! * `eval "$(podnet warp env)"`: the proxy variables;
//! * `podnet warp exec -- CMD` and `podnet warp shell`: the command runs under
//!   the pod of the socket (TCP, DNS and UDP, also static binaries);
//! * `podnet warp up --persist`: a marked block in `~/.bashrc` and
//!   `~/.profile` loads the env file, and an interactive shell runs under
//!   the pod. `podnet warp down` removes it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};

mod fish;

use fish::{fish_block, fish_file};

/// The name of the detached instance of `podnet warp up`.
pub const INSTANCE: &str = "warp";
/// The name of the detached instance of `podnet warp up --tun`.
pub const TUN_INSTANCE: &str = "warp-tun";
/// Set in a shell that `podnet warp shell` started, so that the hook does not
/// start it again.
pub const SHELL_VARIABLE: &str = "PODNET_WARP_SHELL";
pub(super) const BEGIN: &str = "# >>> podnet warp up >>>";
pub(super) const END: &str = "# <<< podnet warp up <<<";

/// The unix socket of the proxy of `podnet warp up`.
pub fn socket_path() -> Result<PathBuf> {
    Ok(crate::instances::run_dir()?.join("warp.sock"))
}

/// The env file of `podnet warp up`.
pub fn env_path() -> Result<PathBuf> {
    Ok(crate::instances::run_dir()?.join("warp.env"))
}

pub(super) fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run `podnet ARGV` (the command `label`, for example `podnet warp proxy
/// --detach`), print its lines, and give its ready document (the last line).
/// The lines of stderr go to stderr at once and are kept for the text of a
/// failure ([`failure_text`]). The error lines of the child (`podnet: ...`)
/// wait: a failure says them once, in the line of this process (GitHub
/// #738).
pub fn start(label: &str, argv: &[String]) -> Result<serde_json::Value> {
    use std::io::{BufRead as _, BufReader};
    let exe = std::env::current_exe().context("the path of this binary")?;
    let mut command = std::process::Command::new(&exe);
    // A shell that read the env file of an earlier warp up has proxy
    // variables that point at the listeners of this proxy: the proxy must
    // not send its own session through them.
    for key in own_proxy_variables(argv, std::env::vars()) {
        command.env_remove(key);
    }
    let mut child = command
        .args(argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {label}"))?;
    let stderr = child.stderr.take();
    let tee = std::thread::spawn(move || {
        let mut tee = Tee::default();
        if let Some(stderr) = stderr {
            for line in BufReader::new(stderr).split(b'\n').map_while(Result::ok) {
                for shown in tee.line(&String::from_utf8_lossy(&line)) {
                    eprintln!("{shown}");
                }
            }
        }
        tee
    });
    let out = child
        .wait_with_output()
        .with_context(|| format!("waiting for {label}"))?;
    let tee = tee.join().unwrap_or_default();
    let stderr = tee.kept.clone();
    let text = String::from_utf8_lossy(&out.stdout);
    for shown in tee.end(out.status.success()) {
        eprintln!("{shown}");
    }
    if !out.status.success() {
        let code = out.status.code().unwrap_or(1);
        // SAFETY: geteuid() has no precondition.
        let root = unsafe { libc::geteuid() } == 0;
        return Err(crate::exit::fault(
            match code {
                crate::exit::USAGE => crate::exit::Fault::Usage,
                crate::exit::CONFIG => crate::exit::Fault::Config,
                _ => crate::exit::Fault::Unavailable,
            },
            anyhow::anyhow!(failure_text(label, code, &stderr, root)),
        ));
    }
    let last = text.lines().last().unwrap_or("");
    serde_json::from_str(last).with_context(|| format!("the ready document: {last:?}"))
}

/// Why `podnet warp tun` could not run, when stderr says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TunFault {
    /// No right to make the TUN (CAP_NET_ADMIN, or the mode of /dev/net/tun).
    Privilege,
    /// The host has no /dev/net/tun.
    NoDevice,
    /// `--default-route` needs an IPv4 default route.
    NoDefaultRoute,
}

fn tun_fault(stderr: &str) -> Option<TunFault> {
    if stderr.contains("no IPv4 default route") {
        return Some(TunFault::NoDefaultRoute);
    }
    if !(stderr.contains("TUNSETIFF") || stderr.contains("/dev/net/tun")) {
        return None;
    }
    if stderr.contains("No such file or directory") || stderr.contains("No such device") {
        Some(TunFault::NoDevice)
    } else if stderr.contains("Operation not permitted") || stderr.contains("Permission denied") {
        Some(TunFault::Privilege)
    } else {
        None
    }
}

/// The lines of stderr of a child, as [`start`] shows them: a line
/// `podnet: ...` waits until a log line comes after it. The lines that wait
/// at the end are the error of the child.
#[derive(Default)]
pub(crate) struct Tee {
    /// Each line, for [`failure_text`].
    pub(crate) kept: String,
    /// The lines `podnet: ...` that wait.
    held: Vec<String>,
}

impl Tee {
    /// Keep `line`, and give the lines to show now.
    pub(crate) fn line(&mut self, line: &str) -> Vec<String> {
        self.kept.push_str(line);
        self.kept.push('\n');
        if line.starts_with("podnet: ") {
            self.held.push(line.to_string());
            return Vec::new();
        }
        let mut shown = std::mem::take(&mut self.held);
        shown.push(line.to_string());
        shown
    }

    /// The lines to show at the end. After a failure, the error of the
    /// child is in the error of this process ([`failure_text`]): only the
    /// other lines are shown.
    pub(crate) fn end(self, success: bool) -> Vec<String> {
        if success {
            return self.held;
        }
        let error = error_of(&self.kept);
        self.held
            .into_iter()
            .filter(|l| {
                !l.strip_prefix("podnet: ")
                    .is_some_and(|l| error.contains(&l))
            })
            .collect()
    }
}

/// The error of the child: the last lines `podnet: TEXT`, without the
/// prefix, and with no line of `--detach`. The first is the cause; the
/// others say what to do. The log lines before them have no such prefix.
fn error_of(stderr: &str) -> Vec<&str> {
    let mut error: Vec<&str> = stderr
        .lines()
        .rev()
        .take_while(|l| l.starts_with("podnet: ") || l.is_empty())
        .filter_map(|l| l.strip_prefix("podnet: "))
        .filter(|l| !l.starts_with("--detach:"))
        .collect();
    error.reverse();
    if error.is_empty() {
        // A log line after the error: the last error line alone.
        error.extend(
            stderr
                .lines()
                .rev()
                .filter_map(|l| l.strip_prefix("podnet: "))
                .find(|l| !l.starts_with("--detach:")),
        );
    }
    error
}

/// The text of the error when the command `label` exited with `code` and
/// wrote `stderr`. The first line names the command and the cause. When the
/// command is `warp tun` and stderr shows a refusal of the TUN, the next
/// lines say what happened and the last lines name the recovery. `root` is
/// whether this process runs as root (sudo does not help then).
pub fn failure_text(label: &str, code: i32, stderr: &str, root: bool) -> String {
    let mut text = format!("{label} exited {code}");
    let error = error_of(stderr);
    if let Some((cause, steps)) = error.split_first() {
        text.push_str(": ");
        text.push_str(cause);
        for step in steps {
            text.push_str("\npodnet: ");
            text.push_str(step);
        }
    }
    if !label.contains("warp tun") {
        return text;
    }
    let (why, sudo) = match tun_fault(stderr) {
        Some(TunFault::Privilege) => (
            "the host refused the TUN interface: this process has no CAP_NET_ADMIN",
            !root,
        ),
        Some(TunFault::NoDevice) => ("the host has no /dev/net/tun", false),
        Some(TunFault::NoDefaultRoute) => (
            "warp up --tun needs an IPv4 default route; sudo does not add one",
            false,
        ),
        None => return text,
    };
    text.push_str(&format!("\npodnet: warp up --tun: {why}"));
    if sudo {
        text.push_str("\npodnet: with root: sudo podnet warp up --tun");
    }
    text.push_str("\npodnet: with no root: podnet warp up (proxy mode)");
    text.push_str("\npodnet: one command with no root: podnet warp run -- CMD");
    text
}

// Issue 116 and T-140: the instance of warp up in `warp status`.
pub use super::live::instance::{instance_line, instance_state};

/// The variables of `vars` whose value is a listener of `argv` (a
/// `--listen tcp://ADDR` or `unix:PATH`): the env file of warp up wrote
/// them.
fn own_proxy_variables(
    argv: &[String],
    vars: impl Iterator<Item = (String, String)>,
) -> Vec<String> {
    let own: Vec<String> = argv
        .iter()
        .filter_map(|a| {
            a.strip_prefix("tcp://")
                .or_else(|| a.strip_prefix("unix:"))
                .map(str::to_string)
        })
        .collect();
    vars.filter(|(key, value)| {
        key != "PODNET_WARP_SOCKET" && own.iter().any(|o| value.ends_with(o.as_str()))
    })
    .map(|(key, _)| key)
    .collect()
}

/// The text of the env file for a proxy on `127.0.0.1:port` and `socket`.
/// With no port (issue 97: the host refused the TCP listener), the file has
/// no proxy variables for other tools: a program goes through WARP with
/// `podnet warp exec`. podnet itself gets PODNET_PROXY on the unix socket (issue
/// 116).
pub fn env_file_text(port: Option<u16>, socket: &Path) -> String {
    let mut out = String::from("# podnet warp up: the proxy variables of the WARP proxy\n");
    let Some(port) = port else {
        out.push_str(
            "# The host refused the TCP listener: there are no proxy variables.\n\
             # Run a program through WARP with: podnet warp exec -- CMD\n\
             # podnet itself uses the unix socket (PODNET_PROXY).\n",
        );
        out.push_str(&format!(
            "export PODNET_WARP_SOCKET={}\n",
            quote(&socket.display().to_string())
        ));
        out.push_str(&format!(
            "export PODNET_PROXY={}\n",
            quote(&format!("socks5h+unix:{}", socket.display()))
        ));
        return out;
    };
    let socks = format!("socks5h://127.0.0.1:{port}");
    let http = format!("http://127.0.0.1:{port}");
    let no_proxy = "localhost,127.0.0.1,::1";
    for (k, v) in [
        ("ALL_PROXY", socks.as_str()),
        ("all_proxy", socks.as_str()),
        ("HTTPS_PROXY", http.as_str()),
        ("https_proxy", http.as_str()),
        ("HTTP_PROXY", http.as_str()),
        ("http_proxy", http.as_str()),
        ("NO_PROXY", no_proxy),
        ("no_proxy", no_proxy),
        ("CARGO_HTTP_PROXY", http.as_str()),
        ("npm_config_proxy", http.as_str()),
        ("npm_config_https_proxy", http.as_str()),
        ("PODNET_WARP_SOCKET", &socket.display().to_string()),
    ] {
        out.push_str(&format!("export {k}={}\n", quote(v)));
    }
    out
}

/// The TCP port and whether the unix socket is up, from the ready document
/// of the `warp proxy` of `podnet warp up` (its `listens`).
pub fn listeners_of(doc: &serde_json::Value) -> (Option<u16>, bool) {
    let listens: Vec<&str> = doc["listens"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let port = listens.iter().find_map(|l| {
        l.strip_prefix("tcp://")
            .and_then(|a| a.parse::<std::net::SocketAddr>().ok())
            .map(|a| a.port())
    });
    let socket = listens.iter().any(|l| l.starts_with("unix:"));
    (port, socket)
}

/// The variables that name a bundle of TLS roots (issue 86).
pub const CA_VARIABLES: [&str; 4] = [
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "GIT_SSL_CAINFO",
];
/// A bundle with fewer certificates than this is taken as the roots of a
/// proxy that terminates TLS, not a public bundle.
pub const SMALL_BUNDLE: usize = 10;
/// The system bundles, in their order: the first that exists is used. They
/// are the bundles of `pod_tls`, as the manual names them (T-475: an own list
/// of 4 paths missed 2 of them).
pub use pod_tls::SYSTEM_BUNDLES;

fn certificates_in(pem: &str) -> usize {
    pem.matches("-----BEGIN CERTIFICATE-----").count()
}

/// The fix of the CA bundle for WARP: when a variable of [`CA_VARIABLES`]
/// names a small bundle (the roots of a proxy that terminates TLS), the
/// tools that use it fail through WARP, which shows the real chains. The
/// fix is one bundle of that bundle and the system bundle, so that the
/// proxy and WARP both work. Gives the merged text and the variables to
/// set to it, or `None` when no variable needs it. `system` is the system
/// bundle, when there is one.
pub fn ca_fix(
    env: &dyn Fn(&str) -> Option<String>,
    system: Option<&Path>,
) -> Option<(String, Vec<&'static str>)> {
    let mut small: Vec<(&'static str, String)> = Vec::new();
    for var in CA_VARIABLES {
        let Some(path) = env(var).filter(|p| !p.is_empty()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let n = certificates_in(&text);
        if n > 0 && n < SMALL_BUNDLE {
            small.push((var, text));
        }
    }
    if small.is_empty() {
        return None;
    }
    let Some(system) = system.and_then(|p| std::fs::read_to_string(p).ok()) else {
        tracing::warn!(
            variables = ?small.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
            "a CA bundle has only the roots of a proxy, and this host has no system bundle: through WARP, the tools that use it fail; unset the variable in a shell that uses WARP"
        );
        return None;
    };
    let mut merged =
        String::from("# podnet warp up: the bundle of the proxy and the system bundle\n");
    let mut seen = std::collections::BTreeSet::new();
    for (_, text) in &small {
        if seen.insert(text.clone()) {
            merged.push_str(text);
            if !text.ends_with('\n') {
                merged.push('\n');
            }
        }
    }
    merged.push_str(&system);
    Some((merged, small.into_iter().map(|(v, _)| v).collect()))
}

/// The CA bundle of `podnet warp up` in the run directory.
pub fn ca_path() -> Result<PathBuf> {
    Ok(crate::instances::run_dir()?.join("warp-ca.pem"))
}

/// Write the env file (mode 0600), with the CA fix when one is needed.
pub fn write_env(port: Option<u16>, socket: &Path) -> Result<PathBuf> {
    let path = env_path()?;
    let mut text = env_file_text(port, socket);
    let system = SYSTEM_BUNDLES.iter().map(Path::new).find(|p| p.exists());
    if let Some((bundle, vars)) = ca_fix(&|k| std::env::var(k).ok(), system) {
        let ca = ca_path()?;
        crate::warp::write_private(&ca, bundle.as_bytes())
            .with_context(|| format!("writing {}", ca.display()))?;
        for var in &vars {
            text.push_str(&format!(
                "export {var}={}\n",
                quote(&ca.display().to_string())
            ));
        }
        tracing::info!(variables = ?vars, bundle = %ca.display(), "a CA bundle had only the roots of a proxy; the env file names a bundle with the system roots too");
    }
    crate::warp::write_private(&path, text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    super::find::write_pointer(socket).context("writing the pointer file of warp up")?;
    Ok(path)
}

/// The env file, when `podnet warp up` runs.
pub fn env_text() -> Result<String> {
    let dir = crate::instances::run_dir()?;
    if !crate::instances::is_alive(&dir, INSTANCE) {
        return Err(crate::exit::fault(
            crate::exit::Fault::Unavailable,
            anyhow::anyhow!("podnet warp up is not running; start it with: podnet warp up"),
        ));
    }
    let path = env_path()?;
    std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
}

/// The block of `--persist` for `env`, `socket` and the binary `exe`.
pub fn hook_block(env: &Path, socket: &Path, exe: &Path) -> String {
    let env = quote(&env.display().to_string());
    let socket = quote(&socket.display().to_string());
    let exe = quote(&exe.display().to_string());
    format!(
        "{BEGIN}\n\
[ -f {env} ] && . {env}\n\
case \"$-\" in *i*) if [ -z \"${{{SHELL_VARIABLE}:-}}\" ] && [ -S {socket} ] && [ -x {exe} ]; then exec {exe} warp shell; fi ;; esac\n\
{END}\n"
    )
}

/// `text` with no block of podnet warp up.
pub fn without_block(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        if bare == BEGIN {
            inside = true;
            continue;
        }
        if bare == END && inside {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
        }
    }
    out
}

fn shell_files() -> Result<Vec<PathBuf>> {
    let home = PathBuf::from(std::env::var_os("HOME").context("no HOME")?);
    // podssh #118: no file of root in the home of another user.
    crate::state::check_own(&home)?;
    let shell = std::env::var("SHELL").unwrap_or_default();
    Ok(shell_files_in(&home, &shell))
}

/// The files of the sh block (issue 115): `~/.bashrc`, `~/.profile`, and
/// `~/.zshrc` when it exists or the shell is zsh (the block is valid zsh).
fn shell_files_in(home: &Path, shell: &str) -> Vec<PathBuf> {
    let mut files = vec![home.join(".bashrc"), home.join(".profile")];
    let zshrc = home.join(".zshrc");
    if shell.ends_with("zsh") || zshrc.exists() {
        files.push(zshrc);
    }
    files
}

/// Add the block to `~/.bashrc` and `~/.profile` (an old block is
/// replaced; the rest of each file stays). Gives the files.
pub fn persist(env: &Path) -> Result<Vec<PathBuf>> {
    let socket = socket_path()?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let block = hook_block(env, &socket, &exe);
    // T-211: a fish file that podnet did not write stays; check it before
    // any write.
    let fish = fish_file();
    if let Some(file) = &fish {
        fish::check(file)?;
    }
    let mut done = Vec::new();
    for file in shell_files()? {
        let old = std::fs::read_to_string(&file).unwrap_or_default();
        let mut text = without_block(&old);
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&block);
        std::fs::write(&file, text).with_context(|| format!("writing {}", file.display()))?;
        done.push(file);
    }
    if let Some(file) = fish {
        fish::write(&file, &fish_block(env, &socket, &exe))?;
        done.push(file);
    }
    Ok(done)
}

/// The first line of the environment.d file of podnet (issue 138): only a
/// file with it is changed or removed.
pub const ENV_D_MARK: &str = "# podnet warp up --env-d: the proxy of the WARP proxy";

/// `$XDG_CONFIG_HOME/environment.d/90-podnet-warp.conf`, or
/// `~/.config/environment.d/90-podnet-warp.conf`.
pub fn env_d_path() -> Result<PathBuf> {
    let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".config"),
    };
    let path = config.join("environment.d/90-podnet-warp.conf");
    crate::state::check_own(&path)?;
    Ok(path)
}

/// The text of the environment.d file for the proxy on 127.0.0.1:`port`
/// (`KEY=VALUE` lines, as environment.d(5)).
pub fn env_d_text(port: u16) -> String {
    let socks = format!("socks5h://127.0.0.1:{port}");
    let http = format!("http://127.0.0.1:{port}");
    let no_proxy = "localhost,127.0.0.1,::1";
    let mut out = format!("{ENV_D_MARK}\n# podnet warp down removes this file.\n");
    for (k, v) in [
        ("ALL_PROXY", socks.as_str()),
        ("all_proxy", socks.as_str()),
        ("HTTPS_PROXY", http.as_str()),
        ("https_proxy", http.as_str()),
        ("HTTP_PROXY", http.as_str()),
        ("http_proxy", http.as_str()),
        ("NO_PROXY", no_proxy),
        ("no_proxy", no_proxy),
    ] {
        out.push_str(&format!("{k}={v}\n"));
    }
    out
}

/// True when `text` is a file of podnet (or there is no text).
fn env_d_is_ours(text: Option<&str>) -> bool {
    text.is_none_or(|t| t.lines().next() == Some(ENV_D_MARK))
}

/// The path of the environment.d file; an error when a file of that name
/// is not a file of podnet. `warp up` calls it before it starts the proxy.
pub fn check_env_d() -> Result<PathBuf> {
    let path = env_d_path()?;
    let old = std::fs::read_to_string(&path).ok();
    if !env_d_is_ours(old.as_deref()) {
        bail!(
            "{} exists and podnet did not write it; remove it, or do not give --env-d",
            path.display()
        );
    }
    Ok(path)
}

/// Write the environment.d file for the proxy on 127.0.0.1:`port`. A file
/// of that name that podnet did not write is an error, and stays.
pub fn write_env_d(port: u16) -> Result<PathBuf> {
    let path = check_env_d()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    std::fs::write(&path, env_d_text(port))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// `podnet warp down`: stop the instance, remove the env file, the socket and
/// the blocks.
pub fn down() -> Result<()> {
    let dir = crate::instances::run_dir()?;
    for name in [INSTANCE, TUN_INSTANCE] {
        if crate::instances::read(&dir, name).is_some() {
            crate::detach::stop(&dir, name, std::time::Duration::from_secs(10))?;
            println!("podnet: warp down: stopped the instance {name}");
        }
    }
    super::find::remove_pointer(&socket_path()?);
    for path in [env_path()?, socket_path()?, ca_path()?] {
        if std::fs::remove_file(&path).is_ok() {
            println!("podnet: warp down: removed {}", path.display());
        }
    }
    for file in shell_files()? {
        let Ok(old) = std::fs::read_to_string(&file) else {
            continue;
        };
        let new = without_block(&old);
        if new != old {
            std::fs::write(&file, new).with_context(|| format!("writing {}", file.display()))?;
            println!("podnet: warp down: removed the block in {}", file.display());
        }
    }
    if let Some(file) = fish_file() {
        fish::remove(&file);
    }
    // Issue 138: the environment.d file, when podnet wrote it.
    if let Ok(file) = env_d_path() {
        if let Ok(text) = std::fs::read_to_string(&file) {
            if env_d_is_ours(Some(&text)) {
                if std::fs::remove_file(&file).is_ok() {
                    println!(
                        "podnet: warp down: removed {} (the programs of this login keep the variables until the next login)",
                        file.display()
                    );
                }
            } else {
                println!(
                    "podnet: warp down: {} is not a file of podnet; it stays",
                    file.display()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
