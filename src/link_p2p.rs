//! The path of a peer-to-peer TCP connection (GitHub #485, TURN-7): the
//! direct path first, and the TURN relay when the direct path is refused or
//! times out.
//!
//! A relay adds a hop that sees the volume of the flow, so it is the second
//! choice. [`open`] tries the direct target, and only a refused or a timed
//! out direct path moves to the relay. Any other failure of the direct path
//! is the answer: a name that does not resolve, or a policy that refuses the
//! flow, is not a case for a relay.
//!
//! This module is the choice alone; the command `podnet link p2p` makes the
//! two ways and carries the bytes.

use std::sync::Arc;
use std::time::Duration;

use pod_core::{BoxedIo, Class, Failure, Stage, Target};
use tokio::time::Instant;

use crate::network::Dial;

/// The path that carried the flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// The host connected to the peer itself.
    Direct,
    /// The flow went through a relay.
    Relay,
}

impl Path {
    pub fn name(self) -> &'static str {
        match self {
            Path::Direct => "direct",
            Path::Relay => "relay",
        }
    }
}

/// What carried the flow, and the stream.
pub struct Opened {
    pub path: Path,
    pub io: BoxedIo,
    /// The class of the failure of the direct path, when the relay carried
    /// the flow.
    pub direct_failure: Option<Class>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("path", &self.path)
            .field("direct_failure", &self.direct_failure)
            .finish_non_exhaustive()
    }
}

/// True when a failure of the direct path is a reason to try the relay:
/// the peer refused the flow, or it did not answer in time (TURN-7).
pub fn falls_back(class: Class) -> bool {
    matches!(class, Class::Refused | Class::Timeout)
}

/// True when a failure of the direct path is a TCP connect that the peer
/// refused. The class of `ConnectionRefused` alone is `Unavailable`
/// (`pod_core::Class::of_io`), so the io cause names the refusal.
fn connect_refused(failure: &Failure) -> bool {
    failure
        .io_cause()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused)
}

/// Open a TCP stream to `peer`: `direct_target` at first, then
/// `relay_target` through `relay` when the direct path is refused or does
/// not answer before `timeout`.
///
/// A relay that is absent, or a direct failure that is not a refusal or a
/// timeout, is a named error. A relay that refuses UDP names the host error
/// of the network (the failure keeps the words of the way).
pub async fn open(
    direct: &Arc<dyn Dial>,
    direct_target: &Target,
    relay: Option<&Arc<dyn Dial>>,
    peer: &Target,
    timeout: Duration,
) -> Result<Opened, Failure> {
    let deadline = Instant::now() + timeout;
    match tokio::time::timeout_at(deadline, direct.dial(direct_target, deadline)).await {
        Ok(Ok(connected)) => Ok(Opened {
            path: Path::Direct,
            io: connected.io,
            direct_failure: None,
        }),
        Ok(Err(failure)) if !falls_back(failure.class) && !connect_refused(&failure) => {
            Err(failure)
        }
        Ok(Err(failure)) => {
            let class = if connect_refused(&failure) {
                Class::Refused
            } else {
                failure.class
            };
            relayed(relay, peer, timeout, class).await
        }
        Err(_) => relayed(relay, peer, timeout, Class::Timeout).await,
    }
}

async fn relayed(
    relay: Option<&Arc<dyn Dial>>,
    peer: &Target,
    timeout: Duration,
    class: Class,
) -> Result<Opened, Failure> {
    let Some(relay) = relay else {
        return Err(Failure::new(
            Stage::Relay,
            class,
            format!(
                "{peer}: the direct path failed and no relay is given: give --relay turn://HOST:PORT"
            ),
        ));
    };
    let deadline = Instant::now() + timeout;
    let connected = relay.dial(peer, deadline).await?;
    Ok(Opened {
        path: Path::Relay,
        io: connected.io,
        direct_failure: Some(class),
    })
}

/// True when a failure of a TURN network names a host that refuses UDP.
/// The command prints that failure as a named skip, with the host's words.
pub fn refuses_udp(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("udp") || text.contains("sock_dgram")
}

/// The line of a relay that the host refused before any allocation: the
/// named skip, with the words of the host.
pub fn skip_line(reason: &str) -> String {
    format!("path relay skipped: the host refuses UDP: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{BoxFuture, Connected};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A way that fails or gives a stream, and counts its dials.
    struct Way {
        name: &'static str,
        failure: Option<Class>,
        io: Option<std::io::ErrorKind>,
        dials: AtomicUsize,
    }

    impl Way {
        fn new(name: &'static str, failure: Option<Class>) -> Arc<Way> {
            Arc::new(Way {
                name,
                failure,
                io: None,
                dials: AtomicUsize::new(0),
            })
        }

        /// A TCP connect that the peer refused: `Unavailable` with the io
        /// cause, as the direct dialer gives it.
        fn refused() -> Arc<Way> {
            Arc::new(Way {
                name: "direct",
                failure: Some(Class::Unavailable),
                io: Some(std::io::ErrorKind::ConnectionRefused),
                dials: AtomicUsize::new(0),
            })
        }
    }

    impl Dial for Way {
        fn label(&self) -> String {
            self.name.into()
        }

        fn dial<'a>(
            &'a self,
            _: &'a Target,
            _: Instant,
        ) -> BoxFuture<'a, Result<Connected, Failure>> {
            self.dials.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                match self.failure {
                    Some(class) => {
                        let failure = match self.io {
                            Some(kind) => Failure::caused(
                                Stage::Connect,
                                class,
                                self.name,
                                std::io::Error::from(kind),
                            ),
                            None => Failure::new(Stage::Connect, class, self.name),
                        };
                        Err(failure)
                    }
                    None => {
                        let (mine, _theirs) = tokio::io::duplex(64);
                        Ok(Connected::new(Box::new(mine)))
                    }
                }
            })
        }
    }

    fn target() -> Target {
        "203.0.113.9:443".parse().unwrap()
    }

    /// The direct path is used when it answers: the relay is not tried.
    #[tokio::test]
    async fn a_direct_path_is_preferred() {
        let (direct, relay) = (Way::new("direct", None), Way::new("relay", None));
        let opened = open(
            &(direct.clone() as Arc<dyn Dial>),
            &target(),
            Some(&(relay.clone() as Arc<dyn Dial>)),
            &target(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(opened.path, Path::Direct);
        assert_eq!(direct.dials.load(Ordering::SeqCst), 1);
        assert_eq!(relay.dials.load(Ordering::SeqCst), 0, "TURN-7");
    }

    /// A refusal of the direct path moves to the relay, and the class of the
    /// refusal is kept.
    #[tokio::test]
    async fn a_refused_direct_path_falls_back_to_the_relay() {
        for class in [Class::Refused, Class::Timeout] {
            let direct = Way::new("direct", Some(class));
            let relay = Way::new("relay", None);
            let opened = open(
                &(direct.clone() as Arc<dyn Dial>),
                &target(),
                Some(&(relay.clone() as Arc<dyn Dial>)),
                &target(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
            assert_eq!(opened.path, Path::Relay, "{class:?}");
            assert_eq!(opened.direct_failure, Some(class));
            assert_eq!(relay.dials.load(Ordering::SeqCst), 1);
        }
    }

    /// A TCP connect that the peer refused is `Unavailable` at
    /// `pod_core::Class::of_io`, but it is a refusal for the choice of the
    /// path: the relay is tried, and the line says refused.
    #[tokio::test]
    async fn a_connect_that_the_peer_refused_falls_back_to_the_relay() {
        let direct = Way::refused();
        let relay = Way::new("relay", None);
        let opened = open(
            &(direct.clone() as Arc<dyn Dial>),
            &target(),
            Some(&(relay.clone() as Arc<dyn Dial>)),
            &target(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(opened.path, Path::Relay);
        assert_eq!(opened.direct_failure, Some(Class::Refused));
        assert_eq!(relay.dials.load(Ordering::SeqCst), 1);
    }

    /// An unavailable direct path that is not a refusal is the answer.
    #[tokio::test]
    async fn an_unavailable_direct_path_that_is_not_a_refusal_does_not_fall_back() {
        let direct = Way::new("direct", Some(Class::Unavailable));
        let relay = Way::new("relay", None);
        let failure = open(
            &(direct.clone() as Arc<dyn Dial>),
            &target(),
            Some(&(relay.clone() as Arc<dyn Dial>)),
            &target(),
            Duration::from_secs(5),
        )
        .await
        .expect_err("the direct failure is the answer");
        assert_eq!(failure.class, Class::Unavailable);
        assert_eq!(relay.dials.load(Ordering::SeqCst), 0);
    }

    /// Another failure of the direct path is the answer: a name that does
    /// not resolve is not a case for a relay.
    #[tokio::test]
    async fn a_direct_path_that_fails_for_another_reason_does_not_fall_back() {
        for class in [Class::NotFound, Class::Policy, Class::Usage] {
            let direct = Way::new("direct", Some(class));
            let relay = Way::new("relay", None);
            let failure = open(
                &(direct.clone() as Arc<dyn Dial>),
                &target(),
                Some(&(relay.clone() as Arc<dyn Dial>)),
                &target(),
                Duration::from_secs(5),
            )
            .await
            .expect_err("the direct failure is the answer");
            assert_eq!(failure.class, class);
            assert_eq!(relay.dials.load(Ordering::SeqCst), 0, "{class:?}");
        }
    }

    /// A refused direct path with no relay is a named error that asks for
    /// one.
    #[tokio::test]
    async fn a_refusal_with_no_relay_names_the_flag() {
        let direct = Way::new("direct", Some(Class::Refused));
        let failure = open(
            &(direct as Arc<dyn Dial>),
            &target(),
            None,
            &target(),
            Duration::from_secs(5),
        )
        .await
        .expect_err("no relay");
        assert_eq!(failure.stage, Stage::Relay);
        assert!(failure.text().contains("--relay"), "{failure}");
    }

    #[test]
    fn a_host_error_that_names_udp_is_a_skip() {
        assert!(refuses_udp(
            "no UDP socket: the seccomp filter denies socket(AF_INET, SOCK_DGRAM)"
        ));
        assert!(refuses_udp("a TURN allocation needs a UDP socket"));
        assert!(!refuses_udp("the server refused the allocation (HTTP 403)"));
    }
}
