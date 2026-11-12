//! The tailnet as a network of the registry (T-115): the login state of
//! `tailscaled` for the rows of the checks and for the link.

use std::collections::BTreeMap;

use podnet_netns::tailscale::Dialer;

/// The state of `tailscaled` on `socket`: a line when it is `Running`, else
/// the reason in one line (it does not answer, or it needs a login).
pub async fn state(socket: &str) -> Result<String, String> {
    let dialer = Dialer::new(socket);
    let status = dialer
        .status()
        .await
        .map_err(|e| format!("socket={socket} does not answer ({e})"))?;
    let tailnet = status.tailnet().unwrap_or("unknown").to_string();
    if status.backend_state == "Running" {
        Ok(format!(
            "socket={socket} BackendState=Running tailnet={tailnet}"
        ))
    } else {
        Err(format!(
            "socket={socket} BackendState={} tailnet={tailnet}: each dial fails until it is logged in (tailscale up; tailscale status)",
            status.backend_state
        ))
    }
}

/// The state as a short word for a status document: the `BackendState`,
/// or `unreachable`.
pub(super) async fn word(socket: &str) -> String {
    match Dialer::new(socket).status().await {
        Ok(status) => status.backend_state,
        Err(_) => "unreachable".into(),
    }
}

/// One line for each network of `networks` that is a tailnet: the name of
/// the network, then the line or the reason of [`state`].
pub async fn rows(networks: &BTreeMap<String, String>) -> Vec<String> {
    let mut rows = Vec::new();
    for (name, spec) in networks {
        let Some(socket) = super::parse(spec).ok().and_then(|n| n.tailscale_socket()) else {
            continue;
        };
        rows.push(match state(&socket).await {
            Ok(line) => format!("{name}: {line}"),
            Err(why) => format!("{name}: {why}"),
        });
    }
    rows
}
