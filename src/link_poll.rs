//! The poll carriers of the link (T-271, T-272, T-280, feature `poll`).
//!
//! `pod_link::poll` of PodUtils/podcrates does the work: a link over
//! requests of HTTP/1.1 with TLS 1.3 (ALPN `http/1.1`), each side pinning
//! the Ed25519 key of the other, as the `tcp` carrier. No request lives as
//! long as the link, so a proxy that closes idle connections or a front of
//! HTTP/1.1 alone can carry it. The bytes up go in POSTs; the dialer
//! chooses the form of the bytes down, and each form is a carrier name:
//!
//! * `poll`: a POST that the listener holds up to the hold (T-271).
//! * `sse`: one answer of events of SSE, base64 (T-272).
//! * `chunked`: one chunked answer of raw bytes (T-272).
//! * `httppoll`: a GET at once, 204 when the listener has nothing (T-280).
//!
//! A listener serves each form; a listener that refuses a push makes the
//! dialer of `sse` or `chunked` take the long poll. This module is the
//! glue, as [`crate::link_tcp`]: [`carrier`], [`listen`] (`IP:PORT`) and
//! [`dial`] (`KEY@https://HOST[:PORT]/PATH`). The dial goes through the way
//! out of the process ([`crate::egress`]). A refused key exits 77.

use std::time::Duration;

use pod_identity::PublicKey;
use pod_link::poll::{Form, Poll};
use pod_link::{Link, Settings};

use crate::identity::{AllowList, Identity};
use crate::link::{carrier_fault, Door};
use crate::link_keyed::{dialed, listen_on, Tune};
use crate::link_probe::{self, Probe};
use crate::link_tcp::{key_of, keys, provider};

/// The carrier names of the forms, in the order of [`form_of`].
pub const NAMES: [&str; 4] = ["poll", "sse", "chunked", "httppoll"];

/// The form of the carrier name `name`.
pub fn form_of(name: &str) -> Option<Form> {
    match name {
        "poll" => Some(Form::LongPoll),
        "sse" => Some(Form::Sse),
        "chunked" => Some(Form::Chunked),
        "httppoll" => Some(Form::Simplex),
        _ => None,
    }
}

/// The carrier of this install for the form `form`: it presents the key
/// of `me`, a listener takes the clients of `allow`, a dial goes through
/// the way out of the process. The query key `poll` of `tune` sets the
/// hold of a long poll (1 to 60 s; default 25 s).
pub fn carrier(me: &Identity, allow: &AllowList, form: Form, tune: &Tune) -> Poll {
    let mut poll = Poll::new(provider(), key_of(me), crate::egress::egress(None))
        .with_clients(keys(allow))
        .with_form(form);
    if let Some(hold) = tune.hold {
        poll.hold = Duration::from_secs(u64::from(hold));
        // The idle time of a lane must be longer than the hold.
        if poll.idle <= poll.hold {
            poll.idle = poll.hold * 2;
        }
    }
    poll
}

/// Listen at `address` (`IP:PORT`) through `poll`. The listener serves
/// each form.
pub async fn listen(poll: &Poll, address: &str) -> anyhow::Result<Door> {
    listen_on(poll, address).await
}

/// Connect to `address` (`KEY@https://HOST[:PORT]/PATH`) through `poll` as
/// the dialer. The handshake pins `KEY`. Then the key that the listener
/// proved must be in `allow` (else exit 77). It gives the link and that
/// key.
pub async fn dial(
    poll: &Poll,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, url) = poll.address(address).map_err(carrier_fault)?;
    pod_link::Carrier::available(poll).map_err(carrier_fault)?;
    dialed(poll.connect(&url, &name).await, allow, settings)
}

/// The carrier of the form `form` for the registry: the facts (its cost
/// class) and the host check, no key of this install.
pub fn probe(form: Form) -> Probe<Poll> {
    let poll = Poll::new(provider(), link_probe::key(), crate::egress::egress(None));
    Probe::new(poll.with_form(form))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_name_is_the_name_of_its_form() {
        for name in NAMES {
            assert_eq!(form_of(name).unwrap().name(), name);
        }
        assert!(form_of("ws").is_none());
    }
}
