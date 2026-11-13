//! The TCP carrier of the link (T-268, feature `tcp`).
//!
//! `pod_link::tcp` of PodUtils/podcrates does the work: TLS 1.3 only, an
//! Ed25519 certificate only, the ALPN `podlink/1`, and a pin of the key of
//! the peer on each side. The pinned key is in the TLS handshake, so the
//! hello of [`crate::hello`] is not used on this carrier. This module is
//! the glue:
//!
//! * [`carrier`] makes the carrier from the key of this install and the
//!   allow list. The list feeds the client keys of the listener
//!   (`with_clients`). The dial goes through the way out of the process
//!   ([`crate::egress`]).
//! * [`listen`] binds `IP:PORT`. [`crate::link::Door::accept_peer`] gives
//!   the key that each client proved.
//! * [`dial`] connects to `KEY@HOST:PORT`. It pins `KEY`, and it checks
//!   the key that the listener proved against the allow list.
//! * A refused key exits 77 ([`crate::link::carrier_fault`]).
//!
//! There is no plaintext mode: the carrier always runs TLS, on a loopback
//! bind too. So the rule "a public bind with `--plaintext` exits 68" has
//! no case here.

use std::sync::Arc;

use pod_identity::PublicKey;
use pod_link::tcp::{Keys, Tcp};
use pod_link::{
    BoxIo, Carrier, CarrierError, CarrierErrorKind, Facts, Link, Listener, Role, Settings,
};
use rustls::crypto::CryptoProvider;

use crate::identity::{AllowList, Identity};
use crate::link::{self, carrier_fault, Door};

/// The TLS provider of the carrier: ring, as the rest of podnet.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The key of this install, as the carrier takes it.
pub(crate) fn key_of(me: &Identity) -> Arc<pod_identity::Identity> {
    Arc::new(pod_identity::Identity::from_seed(&me.key().seed()))
}

/// The client keys of a listener: the entries of `allow`.
pub fn keys(allow: &AllowList) -> Keys {
    Keys::new(allow.names())
}

/// The carrier of this install: it presents the key of `me`, a listener
/// takes the clients of `allow`, and a dial goes through the way out of
/// the process.
pub fn carrier(me: &Identity, allow: &AllowList) -> Tcp {
    Tcp::new(provider(), key_of(me), crate::egress::egress(None)).with_clients(keys(allow))
}

/// [`carrier`] with the dial `via` in place of the way out of the process:
/// the chain of `--egress` elements (T-303, [`crate::link_chain`]).
pub fn carrier_via(me: &Identity, allow: &AllowList, via: Arc<dyn pod_core::Dial>) -> Tcp {
    Tcp::new(provider(), key_of(me), via).with_clients(keys(allow))
}

/// Listen at `address` (`IP:PORT`) through `tcp`.
pub async fn listen(tcp: &Tcp, address: &str) -> anyhow::Result<Door> {
    tcp.available().map_err(carrier_fault)?;
    let listener = tcp.listen(address).await.map_err(carrier_fault)?;
    Ok(Door::from_listener(listener))
}

/// Connect to `address` (`KEY@HOST:PORT`) through `tcp` as the dialer.
/// The handshake pins `KEY`. Then the key that the listener proved must be
/// in `allow` (else exit 77). It gives the link and that key.
pub async fn dial(
    tcp: &Tcp,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, target) = Tcp::address(address).map_err(carrier_fault)?;
    tcp.available().map_err(carrier_fault)?;
    let (io, proved) = tcp.connect(&target, &name).await.map_err(carrier_fault)?;
    allow.check(&proved)?;
    Ok((link::from_io(io, Role::Dialer, settings)?, proved))
}

/// A carrier for the registry. It has the facts and the host check of the
/// TCP carrier, and no key of this install: a dial or a listen through it
/// is a usage error.
pub struct Probe(Tcp);

impl Probe {
    pub fn new() -> Probe {
        // The key is for the probe only: it never signs a handshake.
        let key = Arc::new(pod_identity::Identity::from_seed(&[0u8; 32]));
        Probe(Tcp::new(provider(), key, crate::egress::egress(None)))
    }

    fn usage() -> CarrierError {
        CarrierError::new(
            "tcp",
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
