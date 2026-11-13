//! The userspace stack over a WARP session: `pod-warp`'s `WarpNet`, with
//! podnet's flags and variables of the stack (`--cc`,
//! `PODNET_WARP_TCP_BURST`, `PODNET_WARP_NO_CHECK`) and the counters of the
//! links of the process.

use std::sync::Arc;

use anyhow::Result;

use super::Warped as _;

/// A live handle on the stack of a session: a `pod_core::Dial` with names
/// resolved inside the tunnel, and its UDP. A clone is a handle on the
/// same session.
pub type WarpStack = pod_warp::WarpNet;

pub use pod_warp::net::CongestionControl;

/// The MTU the tunnel carries. 1280 is the IPv6 minimum and what the WARP
/// client uses; anything larger is dropped by the front door.
pub const TUNNEL_MTU: usize = pod_warp::MTU;

/// The variable with the TCP window of the userspace stack, in segments of
/// the MTU, or `none` for no limit (issue 100). For a measurement.
pub const TCP_BURST_VARIABLE: &str = "PODNET_WARP_TCP_BURST";

/// The variable of the congestion controller of the TCP connections of the
/// stack (issue 134): `none` (the default), `reno` or `cubic`.
pub const CC_VARIABLE: &str = "PODNET_WARP_CC";

/// The variable that turns off the check of the data path of a new session
/// (issue 120).
pub const NO_CHECK_VARIABLE: &str = "PODNET_WARP_NO_CHECK";

static CC: std::sync::OnceLock<CongestionControl> = std::sync::OnceLock::new();

/// `none`, `reno` (RFC 5681) or `cubic` (RFC 9438). Another value is a
/// configuration error (exit 78).
pub fn parse_cc(value: &str) -> Result<CongestionControl> {
    pod_warp::net::parse_cc(value).map_err(|failure| {
        crate::exit::fault(
            crate::exit::Fault::Config,
            anyhow::anyhow!("--cc (or {CC_VARIABLE}): {}", failure.chain()),
        )
    })
}

/// Set the congestion controller of this process (`--cc`). A later call
/// does nothing.
pub fn set_cc(cc: CongestionControl) {
    if CC.set(cc).is_ok() {
        tracing::debug!(
            ?cc,
            "the congestion controller of the TCP connections of WARP"
        );
    }
}

/// The congestion controller: `--cc`, else [`CC_VARIABLE`], else none. A
/// bad variable is reported by [`check_cc`] before this is read.
fn cc() -> CongestionControl {
    if let Some(cc) = CC.get() {
        return *cc;
    }
    std::env::var(CC_VARIABLE)
        .ok()
        .and_then(|v| parse_cc(&v).ok())
        .unwrap_or(CongestionControl::None)
}

/// Check [`CC_VARIABLE`] before a command opens a session: a bad value is a
/// configuration error (exit 78).
pub fn check_cc() -> Result<()> {
    match std::env::var(CC_VARIABLE) {
        Ok(v) => parse_cc(&v).map(drop),
        Err(_) => Ok(()),
    }
}

/// The TCP window of [`TCP_BURST_VARIABLE`]: `None` for no limit.
fn tcp_burst() -> Result<Option<usize>> {
    let value = std::env::var(TCP_BURST_VARIABLE).ok();
    pod_warp::net::parse_tcp_burst(value.as_deref()).map_err(|failure| {
        crate::exit::fault(
            crate::exit::Fault::Config,
            anyhow::anyhow!("{TCP_BURST_VARIABLE}: {}", failure.chain()),
        )
    })
}

/// Check [`TCP_BURST_VARIABLE`] before a command opens a session (issue
/// 111): a bad value is a configuration error (exit 78).
pub fn check_tcp_burst() -> Result<()> {
    tcp_burst().map(drop)
}

/// The counters of the links of the WARP stacks of this process (issue
/// 134): each session adds to them.
static LINK_STATS: std::sync::LazyLock<Arc<pod_warp::net::DeviceStats>> =
    std::sync::LazyLock::new(Default::default);

/// The counters of the links of the WARP stacks of this process.
pub fn link_stats() -> pod_warp::net::DeviceStatsSnapshot {
    LINK_STATS.snapshot()
}

/// The check of the data path of a new session, with the resolvers of the
/// tunnel; none with [`NO_CHECK_VARIABLE`].
pub fn data_path_check() -> Option<pod_warp::DataPathCheck> {
    if std::env::var_os(NO_CHECK_VARIABLE).is_some_and(|v| !v.is_empty()) {
        tracing::debug!("{NO_CHECK_VARIABLE} is set: no check of the data path");
        return None;
    }
    Some(pod_warp::DataPathCheck::of(&super::proxy::resolvers()))
}

/// The settings of the stack of a session of this process.
pub fn net_settings() -> pod_warp::NetSettings {
    pod_warp::NetSettings {
        congestion_control: cc(),
        // A bad variable fails the command before (check_tcp_burst).
        tcp_burst: tcp_burst().ok().flatten(),
        link_stats: Some(LINK_STATS.clone()),
        check: data_path_check(),
        ..pod_warp::NetSettings::default()
    }
}

/// The stack over `session` with the addresses `v4` and `v6`, with the
/// settings of this process and no check of the data path.
pub fn start(
    session: pod_warp::MasqueSession,
    v4: Option<std::net::Ipv4Addr>,
    v6: Option<std::net::Ipv6Addr>,
) -> Result<WarpStack> {
    let settings = net_settings();
    WarpStack::start(session, v4, v6, &settings, super::report()).warped()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issues 111 and 134: a bad value is a configuration error (exit 78).
    #[test]
    fn a_bad_value_of_the_stack_is_a_config_error() {
        let err = parse_cc("vegas").unwrap_err();
        assert_eq!(crate::exit::code_of(&err), crate::exit::CONFIG);
        assert!(format!("{err:#}").contains(CC_VARIABLE), "{err:#}");
        assert_eq!(parse_cc("Reno").unwrap(), CongestionControl::Reno);
    }
}
