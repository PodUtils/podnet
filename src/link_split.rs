//! The split session of the link (T-302): one session over an up carrier
//! and a down carrier (`--up-carrier` and `--down-carrier`).
//!
//! `pod_link::split` of PodUtils/podcrates does the work: the bytes of the
//! dialer go up on one carrier, the bytes of the listener come down on the
//! other, and the link above sees one byte stream. The credits of each
//! direction ride on the other half, so the slower half sets the
//! throughput of its direction. Each direction sends a beat after 10 s of
//! silence. A half that is silent for 30 s, fails, or ends early ends the
//! session on each side with `up-carrier lost` or `down-carrier lost`
//! (`pod_link::split::Lost`). No half takes over the bytes of the other
//! (the ruling of 2026-11-14 in `docs/decisions.md`, "The split session: a
//! lost half ends the session"). A session that must outlive the loss of a
//! carrier runs over the resumable layer ([`crate::session`]).
//!
//! This module is the glue:
//!
//! * [`half`] makes the carrier of one half by its name, with the key of
//!   this install and the allow list for a carrier that pins keys.
//! * [`dial`] opens a session at `UP|DOWN` (the address of each half in the
//!   form of its carrier), then runs the hello of [`crate::hello`] above
//!   it: each end proves its key on the session, whichever the halves are.
//! * [`listen`] listens at `UP|DOWN`. Its door gives no key, so the
//!   listener runs the hello too.
//! * [`ended`] gives the exit of a session that a lost half ended: 69, and
//!   the text names the half.

use std::sync::Arc;

use anyhow::anyhow;
use pod_identity::PublicKey;
use pod_link::split::{self, Split};
use pod_link::{Accepted, BoxIo, Carrier, Listener, Pending, Role, Settings};

use crate::carriers::{self, Options};
use crate::exit::{self, Fault};
use crate::identity::{AllowList, Identity};
use crate::link::{self, carrier_fault, Door, Link};
use crate::link_keyed::{self, Tune};

/// The carrier name of a split session, as `link listen` and `link dial`
/// show it.
pub const NAME: &str = split::NAME;

/// The carrier `name` of one half: a carrier of the registry, a carrier of
/// [`link_keyed`], or `tcp`, `ws` or `h2`. A carrier that this build leaves
/// out exits 69; an unknown name exits 64.
pub fn half(
    name: &str,
    tune: &Tune,
    me: &Identity,
    allow: &AllowList,
    options: &Options,
) -> anyhow::Result<Arc<dyn Carrier>> {
    let _ = (&tune, &me, &allow);
    match name {
        #[cfg(feature = "tcp")]
        "tcp" => Ok(Arc::new(crate::link_tcp::carrier(me, allow))),
        #[cfg(feature = "ws")]
        "ws" => Ok(Arc::new(crate::link_ws::carrier(me, allow))),
        #[cfg(feature = "h2")]
        "h2" => Ok(Arc::new(crate::link_h2::carrier(me, allow))),
        _ if ["tcp", "ws", "h2"].contains(&name) => {
            crate::build_features::require(name).map(|()| unreachable!())
        }
        _ if link_keyed::NAMES.contains(&name) => link_keyed::carrier(name, tune, me, allow),
        _ => Ok(Arc::from(carriers::open(name, options)?)),
    }
}

/// The error of a split session: a lost half exits 69 and keeps the text
/// of the loss.
fn fault(error: pod_link::CarrierError) -> anyhow::Error {
    if split::lost_in(&error.to_string()).is_some() {
        return exit::fault(Fault::Unavailable, anyhow::Error::new(error));
    }
    carrier_fault(error)
}

/// Open a session over `split` at `address` (`UP|DOWN`) as the dialer,
/// and prove the keys with the hello: the listener must prove a key of
/// `allow` (else exit 77). It gives the link and that key.
pub async fn dial(
    split: &Split,
    me: &Identity,
    allow: AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (up, down) = Split::addresses(address).map_err(carrier_fault)?;
    split.available().map_err(fault)?;
    let io = split.connect(up, down).await.map_err(fault)?;
    let link = link::from_io(io, Role::Dialer, settings)?;
    let peer = crate::hello::dial(&link, me, allow).await?;
    Ok((link, peer))
}

/// Listen at `address` (`UP|DOWN`) through `split`. The door gives no key:
/// the listener proves each peer with the hello.
pub async fn listen(split: &Split, address: &str) -> anyhow::Result<Door> {
    let (up, down) = Split::addresses(address).map_err(carrier_fault)?;
    split.available().map_err(fault)?;
    let listener = split.bind(up, down).await.map_err(fault)?;
    Ok(Door::from_listener(Box::new(Hello(Box::new(listener)))))
}

/// A listener whose peers prove their key with the hello: the key that a
/// half proved is not given.
struct Hello(Box<dyn Listener>);

impl Listener for Hello {
    fn address(&self) -> String {
        self.0.address()
    }

    fn accept(&mut self) -> Pending<'_, BoxIo> {
        self.0.accept()
    }

    fn accept_peer(&mut self) -> Pending<'_, Accepted> {
        Box::pin(async move {
            let io = self.0.accept().await?;
            Ok(Accepted { io, peer: None })
        })
    }
}

/// The loss of a half that ended `link`, as its text (`up-carrier lost:
/// ...` or `down-carrier lost: ...`), when a loss ended it.
pub fn lost(link: &Link) -> Option<String> {
    let text = link.ended()?.to_string();
    split::lost_in(&text).map(|_| text)
}

/// The exit of a read or a write of a stream that failed: a lost half of a
/// split session exits 69 and names the half; another error keeps its own
/// class.
pub fn ended(error: std::io::Error) -> anyhow::Error {
    let text = error.to_string();
    let lost = split::lost(&error).is_some() || split::lost_in(&text).is_some();
    let error = anyhow!(error);
    if lost {
        return exit::fault(Fault::Unavailable, error);
    }
    error
}

/// True when the read error `error` comes from a lost half.
pub fn is_lost(error: &std::io::Error) -> bool {
    split::lost(error).is_some() || split::lost_in(&error.to_string()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_half_exits_69_and_names_the_half() {
        let lost = split::Lost {
            half: split::Half::Up,
            why: "silent for 30 s".into(),
        };
        let e = std::io::Error::new(std::io::ErrorKind::ConnectionAborted, lost);
        assert!(is_lost(&e));
        let e = ended(e);
        assert_eq!(exit::code_of(&e), 69, "{e:#}");
        assert!(format!("{e:#}").contains("up-carrier lost"), "{e:#}");
        let other = std::io::Error::other("the stream was reset");
        assert!(!is_lost(&other));
        assert_ne!(exit::code_of(&ended(other)), 69);
    }

    #[test]
    fn an_address_with_no_bar_exits_64() {
        let up = carriers::open("memory", &Options::default()).unwrap();
        let down = carriers::open("memory", &Options::default()).unwrap();
        let split = Split::new(Arc::from(up), Arc::from(down));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let Err(e) = rt.block_on(listen(&split, "a")) else {
            panic!("an address with no | is refused")
        };
        assert_eq!(exit::code_of(&e), 64, "{e:#}");
    }

    #[test]
    fn an_unknown_half_exits_64() {
        let dir = tempfile::tempdir().unwrap();
        let me = crate::identity::create(dir.path()).unwrap();
        let tune = Tune::default();
        let Err(e) = half(
            "pigeon",
            &tune,
            &me,
            &AllowList::default(),
            &Options::default(),
        ) else {
            panic!("no carrier is named pigeon")
        };
        assert_eq!(exit::code_of(&e), 64, "{e:#}");
    }
}
