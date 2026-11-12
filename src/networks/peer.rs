//! `peer:FINGERPRINT/SERVICE` as a network (T-266).
//!
//! The network is a peer way (`Addressing::Peer`): it dials no destination.
//! A dial opens one stream of a link that a running `podnet link listen`
//! or `podnet link connect` instance holds, to the peer with the
//! fingerprint, and asks that peer for the service. The instance is found
//! through its control socket ([`crate::link_ctl`]). The target of the dial
//! is not read: the service is in the spec.

use tokio::time::Instant;

use super::{usage, Kind};
use crate::network::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};

/// The kind of `peer:REST`. A bad fingerprint or service is a usage
/// failure.
pub(super) fn parse(spec: &str, rest: &str) -> Result<Kind, Failure> {
    let (fingerprint, service) = rest.split_once('/').ok_or_else(|| {
        usage(format!(
            "{spec}: give peer:FINGERPRINT/SERVICE (the fingerprint that podnet link status shows)"
        ))
    })?;
    crate::link_ctl::peer_list(fingerprint).map_err(|e| usage(format!("{spec}: {e:#}")))?;
    crate::link_service::check_name(service).map_err(|e| usage(format!("{spec}: {e:#}")))?;
    Ok(Kind::Peer {
        fingerprint: fingerprint.to_string(),
        service: service.to_string(),
    })
}

/// A dialer of one service of one peer.
pub(super) struct PeerDial {
    fingerprint: String,
    service: String,
}

impl PeerDial {
    pub(super) fn new(fingerprint: &str, service: &str) -> PeerDial {
        PeerDial {
            fingerprint: fingerprint.to_string(),
            service: service.to_string(),
        }
    }
}

/// The class of a failure by its exit code.
fn class_of(err: &anyhow::Error) -> Class {
    match crate::exit::code_of(err) {
        crate::exit::USAGE => Class::Usage,
        crate::exit::NO_PERM => Class::Policy,
        crate::exit::TIMEOUT => Class::Timeout,
        crate::exit::CONFIG => Class::Config,
        _ => Class::Unavailable,
    }
}

impl Dial for PeerDial {
    fn label(&self) -> String {
        format!("peer:{}/{}", self.fingerprint, self.service)
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let open = crate::link_ctl::open_peer(&self.fingerprint, &self.service);
            let stream = match tokio::time::timeout_at(deadline, open).await {
                Ok(Ok(stream)) => stream,
                Ok(Err(err)) => {
                    return Err(Failure::new(
                        Stage::Connect,
                        class_of(&err),
                        format!("{err:#}"),
                    ))
                }
                Err(_) => {
                    return Err(Failure::new(
                        Stage::Connect,
                        Class::Timeout,
                        format!("{}: no stream before the deadline", self.label()),
                    ))
                }
            };
            Ok(Connected::new(Box::new(stream)))
        })
    }
}
