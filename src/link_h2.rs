//! The HTTP/2 carrier of the link (T-270, feature `h2`).
//!
//! `pod_link::h2` of PodUtils/podcrates does the work: one POST of HTTP/2
//! over TLS 1.3 (ALPN `h2`). The request body carries the bytes of the
//! dialer and the response body the bytes of the listener. Each end pins
//! the Ed25519 key of the other (as the `tcp` carrier). This module is the
//! glue, as [`crate::link_tcp`]:
//!
//! * [`carrier`] makes the carrier from the key of this install and the
//!   allow list. The list feeds the client keys of the listener. The dial
//!   goes through the way out of the process ([`crate::egress`]).
//! * [`listen`] binds `IP:PORT`.
//! * [`dial`] connects to `KEY@https://HOST[:PORT]/PATH`. It pins `KEY`,
//!   and it checks the key that the listener proved against the allow list.
//! * A refused key exits 77 ([`crate::link::carrier_fault`]).
//!
//! The hello of [`crate::hello`] does not run on this carrier.

use std::sync::Arc;

use pod_identity::PublicKey;
use pod_link::h2::H2;
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
pub fn carrier(me: &Identity, allow: &AllowList) -> H2 {
    H2::new(provider(), key_of(me), crate::egress::egress(None)).with_clients(keys(allow))
}

/// [`carrier`] with the dial `via` in place of the way out of the process.
pub fn carrier_via(me: &Identity, allow: &AllowList, via: Arc<dyn pod_core::Dial>) -> H2 {
    H2::new(provider(), key_of(me), via).with_clients(keys(allow))
}

/// Listen at `address` (`IP:PORT`) through `h2`.
pub async fn listen(h2: &H2, address: &str) -> anyhow::Result<Door> {
    h2.available().map_err(carrier_fault)?;
    let listener = h2.listen(address).await.map_err(carrier_fault)?;
    Ok(Door::from_listener(listener))
}

/// Connect to `address` (`KEY@https://HOST[:PORT]/PATH`) through `h2` as the
/// dialer. The handshake pins `KEY`. Then the key that the listener proved
/// must be in `allow` (else exit 77). It gives the link and that key.
pub async fn dial(
    h2: &H2,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, url) = H2::address(address).map_err(carrier_fault)?;
    h2.available().map_err(carrier_fault)?;
    let (io, proved) = h2.connect(&url, &name).await.map_err(carrier_fault)?;
    allow.check(&proved)?;
    Ok((link::from_io(io, Role::Dialer, settings)?, proved))
}

/// A carrier for the registry: the facts and the host check of the `h2`
/// carrier, and no key of this install. A dial or a listen through it is a
/// usage error.
pub struct Probe(H2);

impl Probe {
    pub fn new() -> Probe {
        // The key is for the probe only: it never signs a handshake.
        let key = Arc::new(pod_identity::Identity::from_seed(&[0u8; 32]));
        Probe(H2::new(provider(), key, crate::egress::egress(None)))
    }

    fn usage() -> CarrierError {
        CarrierError::new(
            "h2",
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
