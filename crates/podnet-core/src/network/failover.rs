//! `Failover`: the next way after an error (T-058). The ways are tried
//! one after the other, in their order, until one connects. Each attempt
//! has the deadline of the dial.

use std::sync::Arc;

use tokio::time::Instant;

use super::race::{admit, first, first_datagrams, labels};
use super::{BoxFuture, Connected, Datagrams, Dial, Failure, Target, Way};
use crate::race::Failover as Mode;

/// The ways in order: the next one after an error.
pub struct Failover {
    ways: Vec<Way>,
}

impl Failover {
    /// Each way must dial a destination; the list needs one way or more.
    pub fn new(ways: Vec<Way>) -> Result<Failover, Failure> {
        admit(&ways, "a failover")?;
        Ok(Failover { ways })
    }

    /// The ways, in their order.
    pub fn ways(&self) -> &[Way] {
        &self.ways
    }
}

impl Dial for Failover {
    fn label(&self) -> String {
        format!("failover({})", labels(&self.ways))
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(first(&self.ways, Mode::Serial, target, deadline, false))
    }

    fn dial_http<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(first(&self.ways, Mode::Serial, target, deadline, true))
    }

    /// True when one way or more sends the name: the caller then sends the
    /// name, and a way that does not send it resolves the name itself.
    fn names_inside(&self) -> bool {
        self.ways.iter().any(|w| w.dial.names_inside())
    }

    /// The datagrams of the first way that gives them.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(first_datagrams(&self.ways))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::fakes::{carrier, log, seen, soon, Fake};
    use super::super::{Class, Stage};
    use super::*;

    fn target() -> Target {
        Target::new("example.test", 443)
    }

    #[tokio::test(start_paused = true)]
    async fn the_next_way_starts_only_after_an_error() {
        let log = log();
        let failover = Failover::new(vec![
            Fake::down("a", &log).after(Duration::from_secs(5)).way(),
            Fake::up("b", &log).after(Duration::from_secs(1)).way(),
            Fake::up("c", &log).way(),
        ])
        .unwrap();
        let start = Instant::now();
        let conn = failover.dial(&target(), soon()).await.unwrap();
        assert_eq!(carrier(conn).await, "b");
        assert_eq!(
            start.elapsed(),
            Duration::from_secs(6),
            "no stagger: a, then b"
        );
        assert_eq!(seen(&log), ["a", "b"], "c never started");
        assert_eq!(failover.label(), "failover(a, b, c)");
    }

    #[tokio::test]
    async fn the_error_names_each_way_when_each_fails() {
        let log = log();
        let failover = Failover::new(vec![
            Fake::down("a", &log).way(),
            Fake::down("b", &log).way(),
        ])
        .unwrap();
        let err = failover
            .dial(&target(), soon())
            .await
            .expect_err("each way fails");
        assert_eq!((err.stage, err.class), (Stage::Connect, Class::Unavailable));
        assert_eq!(
            err.text(),
            "no way reached example.test:443: a: a failed; b: b failed"
        );
        let err = failover.datagrams().await.expect_err("no UDP");
        assert_eq!(
            err.text(),
            "no way carries UDP: a: a carries no UDP; b: b carries no UDP"
        );
        assert_eq!(seen(&log), ["a", "b", "a udp", "b udp"]);
    }

    #[test]
    fn a_failover_refuses_a_peer_way_and_an_empty_list() {
        let log = log();
        let peer = Way::peer(Arc::new(Fake::up("wormhole", &log)));
        let err = Failover::new(vec![peer]).err().expect("a peer way");
        assert_eq!(err.class, Class::Usage);
        let err = Failover::new(Vec::new()).err().expect("no way");
        assert_eq!(err.text(), "a failover needs one way or more");
    }
}
