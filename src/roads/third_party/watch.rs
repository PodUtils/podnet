//! Watch probes and non-fatal degradation for third-party roads (T-368).
//!
//! A failed probe transitions the road to [`WatchState::Degraded`] naming the road.
//! It never panics and never aborts an active session.

use std::time::Duration;

use tokio::net::TcpStream;

use super::irc::split_host_port;
use super::{ThirdPartyKind, ThirdPartyRegistry, ThirdPartyRoad, VerifiedDate, WatchState};

/// Default timeout for a third-party road endpoint probe.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Probe one third-party road endpoint without panicking or aborting active sessions.
///
/// A missing, invalid, refused, or timed-out endpoint sets `road.state` to
/// [`WatchState::Degraded`] with a reason naming `road.name`. A reachable endpoint
/// updates `road.state` to [`WatchState::Ok`] or [`WatchState::Stale`] relative to `today`.
pub async fn probe_road(
    road: &mut ThirdPartyRoad,
    timeout: Duration,
    today: VerifiedDate,
) -> WatchState {
    let Some(endpoint) = road.endpoint.as_deref() else {
        let state = WatchState::degraded(format!("road {}: no endpoint configured", road.name));
        road.state = state.clone();
        return state;
    };

    let target = match extract_tcp_target(road.kind, endpoint) {
        Ok(addr) => addr,
        Err(reason) => {
            let state = WatchState::degraded(format!("road {}: {reason}", road.name));
            road.state = state.clone();
            return state;
        }
    };

    match tokio::time::timeout(timeout, TcpStream::connect(&target)).await {
        Ok(Ok(_stream)) => {
            road.state = WatchState::Ok;
            let evaluated = road.evaluate_state(today);
            road.state = evaluated.clone();
            evaluated
        }
        Ok(Err(err)) => {
            let state = WatchState::degraded(format!(
                "road {}: endpoint {target} failed: {err}",
                road.name
            ));
            road.state = state.clone();
            state
        }
        Err(_) => {
            let state = WatchState::degraded(format!(
                "road {}: endpoint {target} timed out after {} ms",
                road.name,
                timeout.as_millis()
            ));
            road.state = state.clone();
            state
        }
    }
}

/// Probe every road in `registry` that has an endpoint configured.
pub async fn probe_registry(
    registry: &mut ThirdPartyRegistry,
    timeout: Duration,
    today: VerifiedDate,
) -> Vec<(String, WatchState)> {
    let mut results = Vec::new();
    for road in registry.roads_mut() {
        if road.endpoint.is_some() {
            let state = probe_road(road, timeout, today).await;
            results.push((road.name.clone(), state));
        }
    }
    results
}

fn extract_tcp_target(kind: ThirdPartyKind, endpoint: &str) -> Result<String, String> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return Err("empty endpoint".to_string());
    }
    let default_port = match kind {
        ThirdPartyKind::Irc => super::irc::DEFAULT_IRC_PORT,
        ThirdPartyKind::Mqtt => super::mqtt::DEFAULT_MQTT_PORT,
        ThirdPartyKind::Syncthing => super::syncthing::DEFAULT_SYNCTHING_PORT,
        ThirdPartyKind::Piping => {
            if trimmed.starts_with("http://") {
                80
            } else {
                443
            }
        }
        ThirdPartyKind::WormholeTransit => 4001,
    };
    let rest = trimmed
        .split_once("://")
        .map_or(trimmed, |(_, after)| after)
        .strip_prefix("tcp:")
        .unwrap_or_else(|| {
            trimmed
                .split_once("://")
                .map_or(trimmed, |(_, after)| after)
        });
    let authority = rest.split(['/', '?']).next().unwrap_or("").trim();
    let (host, port) = split_host_port(authority, default_port).map_err(|e| e.to_string())?;
    if host.contains(':') {
        Ok(format!("[{host}]:{port}"))
    } else {
        Ok(format!("{host}:{port}"))
    }
}
