//! The form of an address: the `unix` prefix, and the hint of another
//! carrier (GitHub #622 item 8, GitHub #634).

use anyhow::anyhow;

use crate::exit::{self, Fault};

/// The carrier that a URL scheme names, for the hint of [`check_address`].
fn carrier_of_scheme(scheme: &str) -> Option<&'static str> {
    Some(match scheme {
        "tcp" | "tls" => "tcp",
        "ws" | "wss" => "ws",
        "http" | "https" | "h2" => "h2",
        "quic" => "quic",
        "udp" | "kcp" => "udp",
        "dns" => "dns",
        "icmp" => "icmp",
        "s3" => "s3",
        "poll" | "sse" | "chunked" | "httppoll" => "poll",
        _ => return None,
    })
}

/// The scheme of `text` when it starts with `SCHEME://`.
fn scheme_of(text: &str) -> Option<&str> {
    let (scheme, _) = text.split_once("://")?;
    let mut bytes = scheme.bytes();
    let first = bytes.next()?;
    let ok = first.is_ascii_alphabetic()
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
    ok.then_some(scheme)
}

/// True when `text` is `IP:PORT`, `[IPV6]:PORT` or `localhost:PORT`.
fn is_host_port(text: &str) -> bool {
    if text.parse::<std::net::SocketAddr>().is_ok() {
        return true;
    }
    match text.rsplit_once(':') {
        Some((host, port)) => host.eq_ignore_ascii_case("localhost") && port.parse::<u16>().is_ok(),
        None => false,
    }
}

/// The address of the carrier `name` with no `unix:` prefix (GitHub #634).
/// The unix carrier takes `unix:PATH` and `unix://PATH` as the other unix
/// socket options of podnet do, also after the key of a dial
/// (`KEY@unix:PATH`). The other carriers get `address` unchanged.
pub fn unix_path(name: &str, address: &str) -> String {
    if name != "unix" {
        return address.to_string();
    }
    let (key, target) = match address.split_once('@') {
        Some((key, rest)) if !key.contains('/') => (Some(key), rest),
        _ => (None, address),
    };
    let path = target
        .strip_prefix("unix://")
        .or_else(|| target.strip_prefix("unix:"))
        .unwrap_or(target);
    match key {
        Some(key) => format!("{key}@{path}"),
        None => path.to_string(),
    }
}

/// Check that `address` has the form of the carrier `name` (GitHub #622
/// item 8). The `unix` carrier (the default) takes a path. An address that
/// starts with a URL scheme (`tcp://`, `wss://`, `quic://`), or that is
/// `IP:PORT`, is for a network carrier: it exits 64, and the message names
/// that carrier. The message does not show the address. The other
/// carriers check their addresses when they open.
pub fn check_address(name: &str, address: &str) -> anyhow::Result<()> {
    if name != "unix" {
        return Ok(());
    }
    // A dial address can hold the key of the listener: `KEY@HOST:PORT`.
    let target = match address.split_once('@') {
        Some((key, rest)) if !key.contains('/') => rest,
        _ => address,
    };
    let usage = |text: String| Err(exit::fault(Fault::Usage, anyhow!(text)));
    if let Some(scheme) = scheme_of(target) {
        let hint = match carrier_of_scheme(&scheme.to_ascii_lowercase()) {
            Some(carrier) => format!("give --carrier {carrier}"),
            None => "give --carrier NAME (podnet link carriers lists them)".to_string(),
        };
        return usage(format!(
            "the unix carrier takes a path, and the address starts with {scheme}://: \
             {hint}, or give a path"
        ));
    }
    if is_host_port(target) {
        return usage(
            "the unix carrier takes a path, and the address is HOST:PORT: give --carrier tcp \
             (or another network carrier: podnet link carriers lists them), or give a path \
             such as ./NAME.sock"
                .to_string(),
        );
    }
    Ok(())
}
