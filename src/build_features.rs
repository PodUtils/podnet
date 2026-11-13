//! The build features of podnet (T-311, GitHub #476, row E10-6).
//!
//! The default build has no optional feature. An optional carrier or
//! service sits behind a Cargo feature ([`OPTIONAL`]). The feature `full`
//! turns on all of them but `portable` ([`FULL`]); CI builds and tests it.
//! No flag at run time turns a feature on: a binary cannot run code that
//! it was not built with.
//!
//! * [`built`] says whether this binary holds the feature.
//! * [`require`] is the gate of a command: a feature that is off exits 69,
//!   and the text names the feature.
//! * [`enabled`] lists the optional features of this build, for the dump
//!   of the link (T-309) and for the inventory (T-310).

use anyhow::anyhow;

use crate::exit::{self, Fault};

/// The optional features, in the order of the issue.
pub const OPTIONAL: [&str; 12] = [
    "tcp", "ws", "h2", "quic", "udp", "poll", "s3", "icmp", "ss", "trojan", "c-abi", "portable",
];

/// The features that `full` turns on: each optional feature but
/// `portable` (a subset takes code away; `full` adds it).
pub const FULL: [&str; 11] = [
    "tcp", "ws", "h2", "quic", "udp", "poll", "s3", "icmp", "ss", "trojan", "c-abi",
];

/// True when this binary holds the optional feature `name`. An unknown
/// name is false.
pub fn built(name: &str) -> bool {
    match name {
        "tcp" => cfg!(feature = "tcp"),
        "ws" => cfg!(feature = "ws"),
        "h2" => cfg!(feature = "h2"),
        "quic" => cfg!(feature = "quic"),
        "udp" => cfg!(feature = "udp"),
        "poll" => cfg!(feature = "poll"),
        "s3" => cfg!(feature = "s3"),
        "icmp" => cfg!(feature = "icmp"),
        "ss" => cfg!(feature = "ss"),
        "trojan" => cfg!(feature = "trojan"),
        "c-abi" => cfg!(feature = "c-abi"),
        "portable" => cfg!(feature = "portable"),
        _ => false,
    }
}

/// The optional features that this binary holds, in the order of
/// [`OPTIONAL`].
pub fn enabled() -> Vec<&'static str> {
    OPTIONAL.into_iter().filter(|f| built(f)).collect()
}

/// The reason that a feature is off, in one line. It names the feature and
/// the way to build it.
pub fn reason(name: &str) -> String {
    format!("not built: the feature {name} is off (build with --features {name})")
}

/// The gate of a command that needs the feature `name`. A feature that is
/// off exits 69 and names the feature. A name that is no optional feature
/// exits 64.
pub fn require(name: &str) -> anyhow::Result<()> {
    if !OPTIONAL.contains(&name) {
        return Err(exit::fault(
            Fault::Usage,
            anyhow!(
                "no build feature is named {name:?}: {}",
                OPTIONAL.join(", ")
            ),
        ));
    }
    if built(name) {
        return Ok(());
    }
    Err(exit::fault(Fault::Unavailable, anyhow!("{}", reason(name))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_agree_with_the_gates() {
        // `built` has an arm for each optional feature: an unknown name is
        // the only false in a `full` build.
        assert!(FULL.iter().all(|f| OPTIONAL.contains(f)));
        assert!(!built("warp-drive"));
        assert!(enabled().iter().all(|f| built(f)));
    }

    #[test]
    fn the_reason_names_the_feature() {
        assert!(reason("quic").contains("not built"));
        assert!(reason("quic").contains("--features quic"));
    }
}
