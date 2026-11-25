//! The WebSocket relay of podnet as the relay leg of a WARP session (issue
//! 76).
//!
//! pod-warp names the door and the way in its own words ("TCP to the MASQUE
//! front door ADDR through the relay URL"), so the failure of this dial
//! gives the cause alone (GitHub #738): no address and no relay again. Its
//! stage is the relay: the step of a failed connect hop (check the host,
//! the port, --proxy) is not true for it.

use std::sync::Arc;

use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};

use super::live;

/// The relay leg: TCP to an IPv4 address through the WebSocket relay.
pub(super) struct RelayDial(pub(super) Arc<crate::ws_relay::Relay>);

impl Dial for RelayDial {
    fn label(&self) -> String {
        format!("the relay {}", self.0.url())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let Target::Addr(addr) = target else {
                let text = format!("the relay takes an address, not {target}");
                return Err(Failure::new(Stage::Connect, Class::Usage, text));
            };
            if addr.is_ipv6() {
                let text = "the relay has no IPv6 egress";
                return Err(Failure::new(Stage::Relay, Class::Unavailable, text));
            }
            let limit = deadline.saturating_duration_since(tokio::time::Instant::now());
            let io =
                self.0.connect(*addr, limit).await.map_err(|e| {
                    Failure::new(Stage::Relay, Class::Unavailable, cause(&self.0, &e))
                })?;
            let io = pod_tls::fragment::wrap(Box::new(io), live::split());
            Ok(Connected::new(io))
        })
    }
}

/// The text of a failed connect through `relay`, with no lead
/// "the relay URL: ": the label of the dial names the relay.
fn cause(relay: &crate::ws_relay::Relay, err: &anyhow::Error) -> String {
    let text = crate::exit::text(err);
    let lead = format!("the relay {}", relay.url());
    pod_core::failure::strip_lead(&text, &lead).to_string()
}

#[cfg(test)]
#[path = "relay_dial_tests.rs"]
mod tests;
