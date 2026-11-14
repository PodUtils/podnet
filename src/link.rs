//! The link of podnet (T-262): many streams over one carrier.
//!
//! `pod-link` of PodUtils/podcrates does the work: the frames, the credit
//! window of each stream and of the link, the keepalive, and the carriers
//! (podcrates calls C120 to C124). This module is the glue. It opens a
//! link over a [`Carrier`] ([`dial`], [`listen`] and [`Door::accept`]),
//! and it gives each failure the exit code of podnet ([`fault`]).
//! [`crate::carriers`] is the registry of the carriers.
//!
//! The resume of a link after the loss of its carrier is `pod-session`
//! (T-263): run the link over the application side of a session with
//! [`from_io`].

use pod_core::Class;
use pod_identity::PublicKey;
pub use pod_link::{
    BoxIo, Carrier, CarrierError, CarrierErrorKind, Error, Facts, Link, Listener, ResetCode, Role,
    Settings, Stream,
};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::exit::{self, Fault};

/// The exit class of a failure of pod-link, when podnet has one. A bad
/// value is 64, a missing or held or refused path is 69.
fn fault_of(class: Class) -> Option<Fault> {
    match class {
        Class::Usage => Some(Fault::Usage),
        Class::Config => Some(Fault::Config),
        Class::TempFail => Some(Fault::TempFail),
        Class::Timeout => Some(Fault::Timeout),
        Class::Unavailable | Class::NotFound | Class::Refused => Some(Fault::Unavailable),
        _ => None,
    }
}

/// A failure of a link with the exit class of podnet.
pub fn fault(error: Error) -> anyhow::Error {
    let class = error.class();
    let error = anyhow::Error::new(error);
    match fault_of(class) {
        Some(f) => exit::fault(f, error),
        None => error,
    }
}

/// True when a handshake refused a key: the pin of this end refused the
/// key of the peer (`Refused` with a `pod_tls::peer::Refusal`), or the peer
/// refused the key of this end (`KeyRefused` of `pod-link`, on the dial and
/// on the accept of each carrier since v0.45.0). A refusal of
/// the host (a bind that `EACCES` denies) is not one.
fn key_refused(error: &CarrierError) -> bool {
    match error.kind {
        CarrierErrorKind::KeyRefused => true,
        CarrierErrorKind::Refused => pod_tls::peer::refusal(error).is_some(),
        _ => false,
    }
}

/// A failure of a carrier with the exit class of podnet: 64 for an
/// address that cannot be used, 77 for a key that a handshake refused,
/// 69 for the others.
pub fn carrier_fault(error: CarrierError) -> anyhow::Error {
    if key_refused(&error) {
        return exit::fault(Fault::NoPermission, anyhow::Error::new(error));
    }
    let class = error.class();
    let error = anyhow::Error::new(error);
    match fault_of(class) {
        Some(f) => exit::fault(f, error),
        None => error,
    }
}

/// Start a link over a byte stream that already exists (a socket pair, the
/// application side of a session) as `role`.
pub fn from_io<C>(io: C, role: Role, settings: Settings) -> anyhow::Result<Link>
where
    C: AsyncRead + AsyncWrite + Send + 'static,
{
    Link::new(io, role, settings).map_err(fault)
}

/// Connect to `address` through `carrier` and run a link as the dialer.
/// The carrier says first whether the host can use it.
pub async fn dial(
    carrier: &dyn Carrier,
    address: &str,
    settings: Settings,
) -> anyhow::Result<Link> {
    carrier.available().map_err(carrier_fault)?;
    let io = carrier.dial(address).await.map_err(carrier_fault)?;
    from_io(io, Role::Dialer, settings)
}

/// Listen at `address` through `carrier`. The carrier says first whether
/// the host can use it.
pub async fn listen(carrier: &dyn Carrier, address: &str) -> anyhow::Result<Door> {
    carrier.available().map_err(carrier_fault)?;
    let listener = carrier.listen(address).await.map_err(carrier_fault)?;
    Ok(Door { listener })
}

/// A listening carrier address. Its drop frees the address.
pub struct Door {
    listener: Box<dyn Listener>,
}

impl Door {
    /// A door over a listener that a carrier made.
    pub fn from_listener(listener: Box<dyn Listener>) -> Door {
        Door { listener }
    }

    /// The address where peers dial, in the form of the carrier.
    pub fn address(&self) -> String {
        self.listener.address()
    }

    /// Wait for the next peer and run a link with it as the listener.
    pub async fn accept(&mut self, settings: Settings) -> anyhow::Result<Link> {
        Ok(self.accept_peer(settings).await?.0)
    }

    /// Like [`Door::accept`], with the key that the carrier proved for the
    /// peer. The `tcp` carrier proves one in its TLS handshake; the unix
    /// and memory carriers prove none (`None`), and the hello does it.
    pub async fn accept_peer(
        &mut self,
        settings: Settings,
    ) -> anyhow::Result<(Link, Option<PublicKey>)> {
        let accepted = self.listener.accept_peer().await.map_err(carrier_fault)?;
        Ok((
            from_io(accepted.io, Role::Listener, settings)?,
            accepted.peer,
        ))
    }
}

/// Two links in one process over a unix socket pair: (dialer, listener).
/// It needs `AF_UNIX`, and no path.
pub fn unix_pair(settings: Settings) -> anyhow::Result<(Link, Link)> {
    let (a, b) = pod_link::unix::pair()
        .map_err(|e| carrier_fault(CarrierError::io("unix", "a socket pair", e)))?;
    Ok((
        from_io(a, Role::Dialer, settings)?,
        from_io(b, Role::Listener, settings)?,
    ))
}

/// Two links in one process over a memory pair with a ring of `ring`
/// bytes for each direction: (dialer, listener). The ring is 64 KiB to
/// 64 MiB.
pub fn memory_pair(ring: usize, settings: Settings) -> anyhow::Result<(Link, Link)> {
    let ring = pod_link::memory::Memory::new(ring)
        .map_err(carrier_fault)?
        .ring();
    let (a, b) = pod_link::memory::pair(ring);
    Ok((
        from_io(a, Role::Dialer, settings)?,
        from_io(b, Role::Listener, settings)?,
    ))
}
