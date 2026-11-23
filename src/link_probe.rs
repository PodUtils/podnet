//! A carrier of the registry with no key of this install (T-264).
//!
//! The registry ([`crate::carriers`]) shows the facts and the host check of
//! each carrier. A carrier that pins the Ed25519 key of each end needs the
//! key of this install and the allow list, and the registry has neither.
//! [`Probe`] wraps such a carrier, made with a key for the probe only:
//!
//! * `name`, `facts` and `available` are those of the carrier.
//! * A dial or a listen through it is a usage error (exit 64): `podnet link
//!   listen` and `podnet link dial` make the real carrier.

use std::sync::Arc;

use pod_link::{BoxIo, Carrier, CarrierError, CarrierErrorKind, Facts, Listener};

/// A key for a probe. It never signs a handshake.
pub fn key() -> Arc<pod_identity::Identity> {
    Arc::new(pod_identity::Identity::from_seed(&[0u8; 32]))
}

/// The facts and the host check of a carrier, with no dial and no listen.
pub struct Probe<C> {
    carrier: C,
    rate: Option<pod_link::Rate>,
}

impl<C: Carrier> Probe<C> {
    /// Wrap `carrier`. Make it with [`key`].
    pub fn new(carrier: C) -> Probe<C> {
        Probe {
            carrier,
            rate: None,
        }
    }

    /// Declare the rate of the carrier in the registry, when the carrier
    /// has one that its constructor does not know (the SMTP row of GitHub
    /// #484 measures it only after a submission).
    pub fn with_rate(mut self, rate: pod_link::Rate) -> Probe<C> {
        self.rate = Some(rate);
        self
    }

    fn usage(&self) -> CarrierError {
        CarrierError::new(
            self.carrier.name(),
            CarrierErrorKind::Usage,
            "this carrier has no key of this install: use `podnet link listen` or `podnet link dial`",
        )
    }
}

impl<C: Carrier> Carrier for Probe<C> {
    fn name(&self) -> &'static str {
        self.carrier.name()
    }

    fn facts(&self) -> Facts {
        let mut facts = self.carrier.facts();
        if let Some(rate) = self.rate {
            facts.rate = rate;
        }
        facts
    }

    fn available(&self) -> Result<(), CarrierError> {
        self.carrier.available()
    }

    fn dial<'a>(&'a self, _: &'a str) -> pod_link::Pending<'a, BoxIo> {
        let e = self.usage();
        Box::pin(async move { Err(e) })
    }

    fn listen<'a>(&'a self, _: &'a str) -> pod_link::Pending<'a, Box<dyn Listener>> {
        let e = self.usage();
        Box::pin(async move { Err(e) })
    }
}
