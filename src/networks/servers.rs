//! The roads of `pod-egress` through a server of another product (T-127):
//! VLESS (`vless://HOST:PORT?...`), Shadowsocks (`ss://HOST:PORT`) and Wisp
//! (`wisp:wss://HOST[:PORT]/PATH`). Each one gives TCP: the server takes
//! the target of each stream and dials it. podnet has no code of these
//! roads: they are `vless`, `shadowsocks` and `wisp` of `pod-egress`.
//!
//! * A spec holds no secret. The ID of a VLESS user and the key of a
//!   Shadowsocks server come from a file or a variable of the process when
//!   the network comes up ([`super::secret`]). A Wisp server takes no
//!   credential.
//! * The connections to the server use the way out of the process, or the
//!   way below the network. The certificate of a server over TLS is checked
//!   with the roots of podnet (`--ca-file`).
//! * `up` proves the reach of the server (and its certificate over TLS).
//!   A fault starts with the spec and names the hop.

use std::sync::Arc;
use std::time::Duration;

use pod_tls::Trust;

use super::secret::Get;
use crate::network::{Check, Dial, Failure, Link};

mod shadowsocks;
mod vless;
mod wisp;

pub use shadowsocks::{KEY_FILE_VARIABLE as SHADOWSOCKS_KEY_FILE, KEY_VARIABLE as SHADOWSOCKS_KEY};
pub use vless::{ID_FILE_VARIABLE as VLESS_ID_FILE, ID_VARIABLE as VLESS_ID};

/// A road that a spec names.
#[derive(Clone, Debug)]
pub(super) enum Spec {
    Vless(vless::Spec),
    Shadowsocks(shadowsocks::Spec),
    Wisp(wisp::Spec),
}

/// The spec of a road; `None` when `spec` names no road of this module.
pub(super) fn parse(spec: &str) -> Option<Result<Spec, Failure>> {
    if vless::is_vless(spec) {
        return Some(vless::parse(spec).map(Spec::Vless));
    }
    if shadowsocks::is_shadowsocks(spec) {
        return Some(shadowsocks::parse(spec).map(Spec::Shadowsocks));
    }
    if wisp::is_wisp(spec) {
        return Some(wisp::parse(spec).map(Spec::Wisp));
    }
    None
}

impl Spec {
    /// The scheme of the row of the registry.
    pub(super) fn scheme(&self) -> &'static str {
        match self {
            Spec::Vless(_) => "vless",
            Spec::Shadowsocks(_) => "shadowsocks",
            Spec::Wisp(_) => "wisp",
        }
    }

    /// The spec with no secret.
    pub(super) fn label(&self) -> String {
        match self {
            Spec::Vless(spec) => spec.label(),
            Spec::Shadowsocks(spec) => spec.label(),
            Spec::Wisp(spec) => spec.label(),
        }
    }

    /// The server that the network must reach first.
    pub(super) fn check(&self) -> Check {
        let (target, tls) = match self {
            Spec::Vless(spec) => (spec.target(), spec.server.tls),
            Spec::Shadowsocks(spec) => (spec.target(), false),
            Spec::Wisp(spec) => (spec.target(), spec.url.tls),
        };
        Check {
            name: self.scheme().into(),
            target,
            tls,
        }
    }

    /// Read the secret of the road with `get`, for the router network
    /// `name`: now and not at the parse, as a parse reads no variable.
    pub(super) fn settings(&self, get: Get<'_>, name: Option<&str>) -> Result<Settings, Failure> {
        Ok(match self {
            Spec::Vless(spec) => Settings::Vless(vless::settings(get, name, spec)?),
            Spec::Shadowsocks(spec) => {
                Settings::Shadowsocks(shadowsocks::settings(get, name, spec)?)
            }
            Spec::Wisp(_) => Settings::Wisp,
        })
    }

    /// Bring the network up with the roots of podnet.
    pub(super) async fn up(
        &self,
        settings: Settings,
        below: Option<&Arc<dyn Dial>>,
    ) -> Result<Link, Failure> {
        let tls = crate::trust::context().clone();
        self.up_with(settings, below, tls, crate::trust::ws_trust())
            .await
    }

    /// [`Spec::up`] with the TLS context and the trust given, so that a test
    /// can trust the CA of its own stand.
    pub(super) async fn up_with(
        &self,
        settings: Settings,
        below: Option<&Arc<dyn Dial>>,
        tls: Arc<pod_tls::Context>,
        trust: Trust,
    ) -> Result<Link, Failure> {
        let dial = below.cloned().unwrap_or_else(crate::dialers::process);
        let way = pod_ws::Way::new(tls, dial, super::via::agent());
        match (self, settings) {
            (Spec::Vless(spec), Settings::Vless(id)) => vless::up(spec, id, way, trust).await,
            (Spec::Shadowsocks(spec), Settings::Shadowsocks(key)) => {
                shadowsocks::up(spec, key, way, trust).await
            }
            (Spec::Wisp(spec), _) => wisp::up(spec, way, trust).await,
            _ => unreachable!("the settings come from the same spec"),
        }
    }
}

/// The secret of a road, read by [`Spec::settings`].
pub(super) enum Settings {
    Vless(vless::Given),
    Shadowsocks(shadowsocks::Given),
    Wisp,
}

/// The bound of each step of a dial: the TCP, TLS, the upgrade.
const STEP: Duration = super::via::STEP;

#[cfg(test)]
mod tests;
