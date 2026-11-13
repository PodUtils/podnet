//! The WebSocket carrier of the link (T-269, feature `ws`).
//!
//! `pod_link::ws` of PodUtils/podcrates does the work: a WebSocket over
//! TLS 1.3, the bytes of the link in binary messages, and a pin of the
//! Ed25519 key of the peer on each side (as the `tcp` carrier). The
//! listener runs the accept loop of `pod-ws` (deadlines and caps of the
//! connections). This module is the glue, as [`crate::link_tcp`]:
//!
//! * [`carrier`] makes the carrier from the key of this install and the
//!   allow list. The list feeds the client keys of the listener. The dial
//!   goes through the way out of the process ([`crate::egress`]).
//! * [`listen`] binds `IP:PORT`. It takes any path of the request.
//! * [`dial`] connects to `KEY@wss://HOST[:PORT]/PATH`. It pins `KEY`, and
//!   it checks the key that the listener proved against the allow list.
//! * A refused key exits 77 ([`crate::link::carrier_fault`]).
//!
//! The hello of [`crate::hello`] does not run on this carrier.

use std::sync::Arc;

use pod_identity::PublicKey;
use pod_link::ws::Ws;
use pod_link::{
    BoxIo, Carrier, CarrierError, CarrierErrorKind, Facts, Link, Listener, Role, Settings,
};

use crate::identity::{AllowList, Identity};
use crate::link::{self, carrier_fault, Door};
pub use crate::link_tcp::keys;
use crate::link_tcp::{key_of, provider};

/// The carrier of this install: it presents the key of `me`, a listener
/// takes the clients of `allow`, and a dial goes through the way out of
/// the process.
pub fn carrier(me: &Identity, allow: &AllowList) -> Ws {
    Ws::new(provider(), key_of(me), crate::egress::egress(None)).with_clients(keys(allow))
}

/// [`carrier`] with the dial `via` in place of the way out of the process.
pub fn carrier_via(me: &Identity, allow: &AllowList, via: Arc<dyn pod_core::Dial>) -> Ws {
    Ws::new(provider(), key_of(me), via).with_clients(keys(allow))
}

/// Listen at `address` (`IP:PORT`) through `ws`.
pub async fn listen(ws: &Ws, address: &str) -> anyhow::Result<Door> {
    ws.available().map_err(carrier_fault)?;
    let listener = ws.listen(address).await.map_err(carrier_fault)?;
    Ok(Door::from_listener(listener))
}

/// Connect to `address` (`KEY@wss://HOST[:PORT]/PATH`) through `ws` as the
/// dialer. The handshake pins `KEY`. Then the key that the listener proved
/// must be in `allow` (else exit 77). It gives the link and that key.
pub async fn dial(
    ws: &Ws,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, url) = Ws::address(address).map_err(carrier_fault)?;
    ws.available().map_err(carrier_fault)?;
    let (io, proved) = ws.connect(&url, &name).await.map_err(carrier_fault)?;
    allow.check(&proved)?;
    Ok((link::from_io(io, Role::Dialer, settings)?, proved))
}

/// A carrier for the registry: the facts and the host check of the `ws`
/// carrier, and no key of this install. A dial or a listen through it is a
/// usage error.
pub struct Probe(Ws);

impl Probe {
    pub fn new() -> Probe {
        // The key is for the probe only: it never signs a handshake.
        let key = Arc::new(pod_identity::Identity::from_seed(&[0u8; 32]));
        Probe(Ws::new(provider(), key, crate::egress::egress(None)))
    }

    fn usage() -> CarrierError {
        CarrierError::new(
            "ws",
            CarrierErrorKind::Usage,
            "this carrier has no key of this install: use `podnet link listen` or `podnet link dial`",
        )
    }
}

impl Default for Probe {
    fn default() -> Probe {
        Probe::new()
    }
}

impl Carrier for Probe {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn facts(&self) -> Facts {
        self.0.facts()
    }

    fn available(&self) -> Result<(), CarrierError> {
        self.0.available()
    }

    fn dial<'a>(&'a self, _: &'a str) -> pod_link::Pending<'a, BoxIo> {
        Box::pin(async { Err(Probe::usage()) })
    }

    fn listen<'a>(&'a self, _: &'a str) -> pod_link::Pending<'a, Box<dyn Listener>> {
        Box::pin(async { Err(Probe::usage()) })
    }
}
