//! The dialers of the ways out over the egress of the process and over the
//! relay (T-056). Each one is a [`Dial`] of [`crate::network`] over a
//! function that exists, with no change of its behaviour:
//!
//! * [`Direct`]: [`crate::egress::dial_direct`] for TCP, and the UDP
//!   socket of the system;
//! * [`proxy`]: [`crate::egress::egress`] through one proxy (HTTP CONNECT,
//!   SOCKS5, on TCP, on a Unix socket or at a command);
//! * [`process`]: [`crate::egress::egress`] with the proxy choice of the
//!   process (`--proxy`, `--no-proxy`, the variables);
//! * [`RelayDialer`]: [`crate::ws_relay::Relay::connect`], TCP to an IPv4
//!   address.
//!
//! The dialers of the virtual network, of the pod upstream chains and of
//! the tailnet front door are in the module `dialer` of `crates/podnet-vnet`.

use std::sync::Arc;

use crate::hop::{self, Hop};
use crate::network::{Class, Dial, Failure, Stage};

mod direct;
mod relay;

pub use direct::Direct;
pub use relay::RelayDialer;

/// A way out through `proxy` alone (`NO_PROXY` applies), with podnet's
/// words of a failure.
pub fn proxy(proxy: crate::egress::Proxy) -> Arc<dyn Dial> {
    crate::egress::egress(Some(proxy))
}

/// The way out of the process: the proxy of `--proxy`, of the variables or
/// of a tool's file, else direct.
pub fn process() -> Arc<dyn Dial> {
    crate::egress::egress(None)
}

/// A failed dial of podnet as a [`Failure`]: the stage of its hop, else
/// `stage`. The text keeps the chain of the error.
fn failure(e: anyhow::Error, stage: Stage) -> Failure {
    let stage = match hop::of(&e) {
        Some(Hop::Proxy) => Stage::Proxy,
        Some(Hop::Dns) => Stage::Dns,
        _ => stage,
    };
    let class = match e.chain().find_map(|c| c.downcast_ref::<std::io::Error>()) {
        Some(io) => Class::of_io(io.kind()),
        None => Class::Unavailable,
    };
    Failure::caused(stage, class, format!("{e:#}"), e)
}

#[cfg(test)]
mod tests;
