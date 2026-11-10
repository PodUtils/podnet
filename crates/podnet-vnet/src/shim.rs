//! Build, locate and configure the `LD_PRELOAD` shim.
//!
//! The shim is C, not Rust, because it has to interpose libc symbols before
//! the program's own constructors run. Its source is embedded in this crate
//! so a `podnet` binary can drop it in a writable directory, compile it with
//! the host's `cc`, and use it without shipping a second file.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// The shim source, compiled into the binary.
pub const SHIM_SOURCE: &str = include_str!("../shim/podnet-vnet.c");
/// The shim's file name on a unix host.
pub const SHIM_FILE: &str = "libpodnet-vnet.so";

/// The Tailscale-socksifying shim source, compiled into the binary.
///
/// Separate from [`SHIM_SOURCE`]: this one rewrites `connect(2)` to an
/// `AF_UNIX` SOCKS5 front door for a tailnet address, and leaves every other
/// socket alone. The abstract-name shim and this one are alternative front
/// ends, not layers, so they are built and preloaded separately.
pub const SOCKS_SHIM_SOURCE: &str = include_str!("../shim/podnet-socks.c");
/// The socksifying shim's file name on a unix host.
pub const SOCKS_SHIM_FILE: &str = "libpodnet-socks.so";

/// Configuration passed to the shim through the environment.
#[derive(Clone, Debug)]
pub struct ShimOptions {
    /// The abstract-name prefix (default `podnet-vnet`).
    pub prefix: String,
    /// Log every translation to stderr.
    pub log: bool,
    /// Map `127.0.0.1`/`::1` into the virtual subnet.
    pub map_loopback: bool,
    /// A `hosts`-style table for `getaddrinfo` interception.
    pub hosts: Vec<(String, std::net::IpAddr)>,
    /// Enter the stack's control protocol instead of direct mode. `None`
    /// keeps direct mode: two interposed programs talk over abstract names
    /// with no host process.
    pub control: Option<String>,
    /// Size of the shim's per-fd table, used to size the shim's arrays.
    pub max_fds: usize,
    /// The routes of [`parse_socks_route`] whose stream connects go to the
    /// SOCKS5 front door `socks_socket` (T-167), not to an abstract name.
    pub socks_routes: Vec<String>,
    /// The unix socket of the front door of `socks_routes`.
    pub socks_socket: String,
}

/// The default unix socket of the SOCKS5 front door (`net tailscale`).
pub const DEFAULT_SOCKS_SOCKET: &str = "/tmp/podnet-socks.sock";

/// The most routes that the C shims keep (`MAX_ROUTES`).
pub const MAX_SOCKS_ROUTES: usize = 32;

/// Check one route of `PODNET_SOCKS_CIDRS` and `PODNET_VNET_SOCKS_ROUTES`
/// (T-167): `tailnet` (100.64.0.0/10 and fd7a:115c:a1e0::/48), `ADDR`,
/// `ADDR/BITS`, `IPV4:PORT` or `[IPV6]:PORT`. Gives the route as the shims
/// read it.
pub fn parse_socks_route(text: &str) -> Result<String> {
    use std::net::IpAddr;
    let text = text.trim();
    let forms = "give tailnet, ADDR, ADDR/BITS, IPV4:PORT or [IPV6]:PORT";
    let port = |p: &str| match p.parse::<u16>() {
        Ok(n) if n > 0 => Ok(()),
        _ => bail!("the route {text:?} has a bad port: {forms}"),
    };
    if text == "tailnet" {
        return Ok(text.to_string());
    }
    if let Some(rest) = text.strip_prefix('[') {
        let Some((addr, tail)) = rest.split_once(']') else {
            bail!("the route {text:?} has no ]: {forms}");
        };
        if addr.parse::<std::net::Ipv6Addr>().is_err() {
            bail!("the route {text:?} has no IPv6 address in []: {forms}");
        }
        if !tail.is_empty() {
            port(tail.strip_prefix(':').unwrap_or("-"))?;
        }
        return Ok(text.to_string());
    }
    if let Some((addr, bits)) = text.split_once('/') {
        let max = match addr.parse::<IpAddr>() {
            Ok(IpAddr::V4(_)) => 32,
            Ok(IpAddr::V6(_)) => 128,
            Err(_) => bail!("the route {text:?} has no address before /: {forms}"),
        };
        match bits.parse::<u8>() {
            Ok(n) if n <= max => return Ok(text.to_string()),
            _ => bail!("the route {text:?} has a prefix length that is not 0 to {max}"),
        }
    }
    if text.parse::<IpAddr>().is_ok() {
        return Ok(text.to_string());
    }
    if let Some((addr, p)) = text.rsplit_once(':') {
        if addr.parse::<std::net::Ipv4Addr>().is_ok() {
            port(p)?;
            return Ok(text.to_string());
        }
    }
    bail!("the route {text:?} is not an address: {forms}")
}

/// Check each route and join them for the environment. More than
/// [`MAX_SOCKS_ROUTES`] is an error.
pub fn socks_route_list(routes: &[String]) -> Result<String> {
    if routes.len() > MAX_SOCKS_ROUTES {
        bail!(
            "{} routes: the shim keeps {MAX_SOCKS_ROUTES} at most",
            routes.len()
        );
    }
    let checked = routes
        .iter()
        .map(|r| parse_socks_route(r))
        .collect::<Result<Vec<_>>>()?;
    Ok(checked.join(","))
}

impl Default for ShimOptions {
    fn default() -> Self {
        Self {
            prefix: "podnet-vnet".into(),
            log: false,
            map_loopback: false,
            hosts: Vec::new(),
            control: None,
            max_fds: 65536,
            socks_routes: Vec::new(),
            socks_socket: DEFAULT_SOCKS_SOCKET.into(),
        }
    }
}

impl ShimOptions {
    /// Build the environment a preloaded program needs.
    pub fn environment(&self, shim_path: &Path) -> Vec<(String, String)> {
        let mut env = vec![(
            "LD_PRELOAD".to_string(),
            match std::env::var("LD_PRELOAD") {
                Ok(existing) if !existing.is_empty() => {
                    format!("{}:{existing}", shim_path.display())
                }
                _ => shim_path.display().to_string(),
            },
        )];
        env.push(("PODNET_VNET_PREFIX".into(), self.prefix.clone()));
        if self.log {
            env.push(("PODNET_VNET_LOG".into(), "1".into()));
        }
        if self.map_loopback {
            env.push(("PODNET_VNET_MAP_LOOPBACK".into(), "1".into()));
        }
        if let Some(control) = &self.control {
            env.push(("PODNET_VNET_CONTROL".into(), control.clone()));
        }
        if !self.hosts.is_empty() {
            let table = self
                .hosts
                .iter()
                .map(|(name, ip)| format!("{name}={ip}"))
                .collect::<Vec<_>>()
                .join(",");
            env.push(("PODNET_VNET_HOSTS".into(), table));
        }
        env.push(("PODNET_VNET_MAX_FDS".into(), self.max_fds.to_string()));
        if !self.socks_routes.is_empty() {
            env.push(("PODNET_VNET_SOCKS".into(), self.socks_socket.clone()));
            env.push((
                "PODNET_VNET_SOCKS_ROUTES".into(),
                self.socks_routes.join(","),
            ));
        }
        env
    }
}

/// How the shim was obtained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShimOrigin {
    /// `PODNET_VNET_SHIM` named it.
    Environment,
    /// A previously compiled copy beside the build tree or binary.
    Cached,
    /// It was compiled during this call.
    Compiled,
    /// The crate's own `target/` directory has one.
    BuildTree,
}

/// A located or compiled shim.
#[derive(Clone, Debug)]
pub struct Shim {
    pub path: PathBuf,
    pub origin: ShimOrigin,
}

/// Names searched, in order, when looking for an existing shim.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(value) = std::env::var("PODNET_VNET_SHIM") {
        if !value.is_empty() {
            out.push(PathBuf::from(value));
        }
    }
    // Beside the running executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join(SHIM_FILE));
            out.push(dir.join("shim").join(SHIM_FILE));
        }
    }
    // The crate's target directory, when running from a checkout.
    out.push(PathBuf::from("target/release").join(SHIM_FILE));
    out.push(PathBuf::from("target/debug").join(SHIM_FILE));
    out.push(PathBuf::from("shim").join(SHIM_FILE));
    out
}

/// Find an existing shim, or `None`.
///
/// Candidates are tried in the order [`candidate_paths`] returns them, so an
/// explicit `PODNET_VNET_SHIM` wins, then the running binary's own directory, then
/// a checkout's build tree.
pub fn locate() -> Option<Shim> {
    let env_shim = std::env::var("PODNET_VNET_SHIM")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    for path in candidate_paths() {
        if path.is_file() {
            let origin = if env_shim.as_deref() == Some(path.as_path()) {
                ShimOrigin::Environment
            } else if path.starts_with("target") {
                ShimOrigin::BuildTree
            } else {
                ShimOrigin::Cached
            };
            return Some(Shim { path, origin });
        }
    }
    None
}

/// Find the shim, compiling it if necessary.
///
/// `directory` is where a compiled copy is written when none exists.
pub fn locate_or_build(directory: &Path) -> Result<Shim> {
    if let Some(shim) = locate() {
        return Ok(shim);
    }
    build(directory)
}

/// Find an existing socksifying shim, or `None`.
pub fn locate_socks() -> Option<Shim> {
    let env_shim = std::env::var("PODNET_SOCKS_SHIM")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut candidates = Vec::new();
    if let Some(path) = env_shim.clone() {
        candidates.push(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(SOCKS_SHIM_FILE));
            candidates.push(dir.join("shim").join(SOCKS_SHIM_FILE));
        }
    }
    candidates.push(PathBuf::from("target/release").join(SOCKS_SHIM_FILE));
    candidates.push(PathBuf::from("target/debug").join(SOCKS_SHIM_FILE));
    candidates.push(PathBuf::from("shim").join(SOCKS_SHIM_FILE));
    for path in candidates {
        if path.is_file() {
            let origin = if env_shim.as_deref() == Some(path.as_path()) {
                ShimOrigin::Environment
            } else if path.starts_with("target") {
                ShimOrigin::BuildTree
            } else {
                ShimOrigin::Cached
            };
            return Some(Shim { path, origin });
        }
    }
    None
}

/// Compile the embedded shim source into `directory/libpodnet-vnet.so`.
pub fn build(directory: &Path) -> Result<Shim> {
    compile_source(SHIM_SOURCE, SHIM_FILE, directory)
}

/// Compile the embedded socksifying shim into `directory/libpodnet-socks.so`.
pub fn build_socks(directory: &Path) -> Result<Shim> {
    compile_source(SOCKS_SHIM_SOURCE, SOCKS_SHIM_FILE, directory)
}

pub(crate) fn compile_source(source_text: &str, file_name: &str, directory: &Path) -> Result<Shim> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("creating shim directory {}", directory.display()))?;
    let source = directory.join(file_name.replace(".so", ".c"));
    // Only rewrite when the content differs, so a rebuild loop is cheap and an
    // editor's mtime is not fought.
    // Issue 247: a source of another user, or a symlink, is replaced.
    let needs_write = !podnet_core::state::own_file(&source)
        || std::fs::read(&source)
            .map(|existing| existing != source_text.as_bytes())
            .unwrap_or(true);
    if needs_write {
        podnet_core::state::replace_file(&source, source_text.as_bytes(), 0o644)
            .context("writing the shim source")?;
    }
    let output = directory.join(file_name);
    const FLAGS: &[&str] = &["-O2", "-shared", "-fPIC", "-ldl", "-lpthread"];
    // Issue 181: a stamp of the source and the flags only. With no
    // compiler on this host, a library that a compiler made from the same
    // source is used.
    let source_stamp_path = directory.join(format!("{file_name}.source-stamp"));
    let source_stamp = {
        let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
        ctx.update(source_text.as_bytes());
        for flag in FLAGS {
            ctx.update(b"\0");
            ctx.update(flag.as_bytes());
        }
        ctx.finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let cc = match compiler() {
        Ok(cc) => cc,
        Err(err) => {
            if cached(&output, &source_stamp_path, &source_stamp) {
                return Ok(Shim {
                    path: output,
                    origin: ShimOrigin::Cached,
                });
            }
            return Err(err);
        }
    };
    // A stamp of the source, the compiler and the flags: when it matches
    // and the library exists, cc does not run again (issue 64).
    let stamp_path = directory.join(format!("{file_name}.stamp"));
    let stamp = {
        let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
        ctx.update(source_text.as_bytes());
        ctx.update(b"\0");
        ctx.update(cc.as_bytes());
        for flag in FLAGS {
            ctx.update(b"\0");
            ctx.update(flag.as_bytes());
        }
        ctx.finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    if cached(&output, &stamp_path, &stamp) {
        return Ok(Shim {
            path: output,
            origin: ShimOrigin::Cached,
        });
    }
    // The compiler writes a new file; a rename puts it at `output`, so a
    // symlink at `output` is replaced and not followed.
    let built = podnet_core::state::temp_name(&output);
    let mut command = Command::new(&cc);
    command
        .arg("-O2")
        .arg("-shared")
        .arg("-fPIC")
        .arg("-o")
        .arg(&built)
        .arg(&source)
        .arg("-ldl")
        .arg("-lpthread");
    let result = command.output();
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            let _ = std::fs::remove_file(&built);
            return Err(err).with_context(|| format!("running {cc} to build the shim"));
        }
    };
    if !result.status.success() {
        let _ = std::fs::remove_file(&built);
        bail!(
            "compiling the shim failed ({}):\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    if !built.is_file() {
        bail!(
            "{cc} reported success but {} does not exist",
            built.display()
        );
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let made = std::fs::set_permissions(&built, std::fs::Permissions::from_mode(0o755))
            .and_then(|()| std::fs::rename(&built, &output));
        if let Err(err) = made {
            let _ = std::fs::remove_file(&built);
            return Err(err).with_context(|| format!("writing {}", output.display()));
        }
    }
    let _ = podnet_core::state::replace_file(&stamp_path, format!("{stamp}\n").as_bytes(), 0o644);
    let _ = podnet_core::state::replace_file(
        &source_stamp_path,
        format!("{source_stamp}\n").as_bytes(),
        0o644,
    );
    Ok(Shim {
        path: output,
        origin: ShimOrigin::Compiled,
    })
}

/// True when the library `output` and its stamp `stamp_path` are files of
/// this user that other users cannot write, and the stamp is `stamp`.
/// Issue 247: the stamp is a hash of public text, so a library that another
/// user put in the directory, or a symlink, is not used.
fn cached(output: &Path, stamp_path: &Path, stamp: &str) -> bool {
    podnet_core::state::own_file(output)
        && podnet_core::state::own_file(stamp_path)
        && std::fs::read_to_string(stamp_path).is_ok_and(|s| s.trim() == stamp)
}

/// The C compiler to use: `$CC`, else `cc`, else `gcc`/`clang`.
pub fn compiler() -> Result<String> {
    if let Ok(cc) = std::env::var("CC") {
        if !cc.trim().is_empty() {
            return Ok(cc);
        }
    }
    for candidate in ["cc", "gcc", "clang"] {
        if which(candidate).is_some() {
            return Ok(candidate.to_string());
        }
    }
    bail!("no C compiler found (set CC, or install cc/gcc/clang)")
}

/// Search `PATH` for an executable.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Write the embedded source to `path` without compiling, for auditing or
/// for a user who wants to build it themselves.
pub fn write_source(path: &Path) -> Result<()> {
    std::fs::write(path, SHIM_SOURCE)
        .with_context(|| format!("writing shim source {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_is_built() {
        let options = ShimOptions {
            log: true,
            map_loopback: true,
            hosts: vec![("web".into(), "10.66.0.2".parse().unwrap())],
            ..ShimOptions::default()
        };
        let env: std::collections::HashMap<_, _> = options
            .environment(Path::new("/tmp/libpodnet-vnet.so"))
            .into_iter()
            .collect();
        assert!(env["LD_PRELOAD"].contains("libpodnet-vnet.so"));
        assert_eq!(env["PODNET_VNET_LOG"], "1");
        assert_eq!(env["PODNET_VNET_MAP_LOOPBACK"], "1");
        assert_eq!(env["PODNET_VNET_HOSTS"], "web=10.66.0.2");
    }

    // T-167: the forms of a route of the socks shims.
    #[test]
    fn socks_routes_are_checked() {
        for good in [
            "tailnet",
            "127.0.0.1",
            "127.0.0.1:3000",
            "10.0.0.0/8",
            "0.0.0.0/0",
            "fd00::/8",
            "::1",
            "[::1]:8080",
            "[fd7a::1]",
        ] {
            assert_eq!(parse_socks_route(good).unwrap(), good);
        }
        for bad in [
            "",
            "web:80",
            "10.0.0.0/33",
            "::/129",
            "127.0.0.1:0",
            "127.0.0.1:x",
            "[::1",
            "[::1]80",
            "a,b",
            "10.0.0.1/8:80",
        ] {
            assert!(parse_socks_route(bad).is_err(), "{bad:?}");
        }
        let many: Vec<String> = (0..33).map(|i| format!("10.0.0.{i}")).collect();
        assert!(socks_route_list(&many).is_err());
        assert_eq!(
            socks_route_list(&["tailnet".into(), "127.0.0.1:3000".into()]).unwrap(),
            "tailnet,127.0.0.1:3000"
        );
        let options = ShimOptions {
            socks_routes: vec!["127.0.0.1:3000".into()],
            ..ShimOptions::default()
        };
        let env: std::collections::HashMap<_, _> = options
            .environment(Path::new("/tmp/libpodnet-vnet.so"))
            .into_iter()
            .collect();
        assert_eq!(env["PODNET_VNET_SOCKS"], DEFAULT_SOCKS_SOCKET);
        assert_eq!(env["PODNET_VNET_SOCKS_ROUTES"], "127.0.0.1:3000");
    }

    #[test]
    fn embedded_source_looks_like_the_shim() {
        assert!(SHIM_SOURCE.contains("podnet-vnet"));
        assert!(SHIM_SOURCE.contains("dlsym"));
        assert!(SHIM_SOURCE.contains("AF_UNIX"));
    }

    // Issue 247: a cached library that is a symlink, or that other users
    // can write, is built again; the stamp alone is not trusted.
    #[test]
    fn a_planted_library_is_not_used_from_the_cache() {
        use std::os::unix::fs::PermissionsExt;
        if compiler().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let first = build(dir.path()).unwrap();
        assert_eq!(first.origin, ShimOrigin::Compiled);
        assert_eq!(build(dir.path()).unwrap().origin, ShimOrigin::Cached);
        // A symlink with the correct stamp.
        let planted = dir.path().join("planted.so");
        std::fs::copy(&first.path, &planted).unwrap();
        std::fs::remove_file(&first.path).unwrap();
        std::os::unix::fs::symlink(&planted, &first.path).unwrap();
        let again = build(dir.path()).unwrap();
        assert_eq!(again.origin, ShimOrigin::Compiled);
        assert!(!std::fs::symlink_metadata(&again.path)
            .unwrap()
            .file_type()
            .is_symlink());
        // A library that other users can write.
        std::fs::set_permissions(&again.path, std::fs::Permissions::from_mode(0o666)).unwrap();
        let again = build(dir.path()).unwrap();
        assert_eq!(again.origin, ShimOrigin::Compiled);
        let mode = std::fs::metadata(&again.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        // No temporary file is left.
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(!name.to_string_lossy().ends_with(".tmp"), "{name:?}");
        }
    }

    #[test]
    fn finds_a_compiler() {
        // cc or gcc are present on every host this is developed on.
        assert!(compiler().is_ok());
        assert!(which("sh").is_some() || which("cc").is_some());
    }
}
