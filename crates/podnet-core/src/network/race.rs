//! `Race`: staggered attempts over several ways (T-058). The first way
//! that connects wins; each other connection is closed. The race of the
//! tunnel legs uses the same function ([`crate::race::race`]).

use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use super::{
    Addressing, BoxFuture, Class, Connected, Datagrams, Dial, Failure, Stage, Target, Way,
};
use crate::race::{race, Failover};

/// Staggered attempts over several ways.
pub struct Race {
    ways: Vec<Way>,
    stagger: Duration,
}

impl Race {
    /// Start the attempt of `ways[0]`; start the next one when the one
    /// before has not connected after `stagger`, or at once when it fails.
    /// Each way must dial a destination; a race needs one way or more.
    pub fn new(ways: Vec<Way>, stagger: Duration) -> Result<Race, Failure> {
        admit(&ways, "a race")?;
        Ok(Race { ways, stagger })
    }

    /// The ways, in their order.
    pub fn ways(&self) -> &[Way] {
        &self.ways
    }
}

impl Dial for Race {
    fn label(&self) -> String {
        format!("race({})", labels(&self.ways))
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(first(
            &self.ways,
            Failover::Staggered(self.stagger),
            target,
            deadline,
            false,
        ))
    }

    fn dial_http<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(first(
            &self.ways,
            Failover::Staggered(self.stagger),
            target,
            deadline,
            true,
        ))
    }

    /// True when one way or more sends the name: the caller then sends the
    /// name, and a way that does not send it resolves the name itself.
    fn names_inside(&self) -> bool {
        self.ways.iter().any(|w| w.dial.names_inside())
    }

    /// The datagrams of the first way that gives them, in the order of the
    /// ways.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(first_datagrams(&self.ways))
    }
}

/// Refuse an empty list and a way that meets a peer: `what` dials a
/// destination.
pub(super) fn admit(ways: &[Way], what: &str) -> Result<(), Failure> {
    if ways.is_empty() {
        return Err(Failure::new(
            Stage::Connect,
            Class::Usage,
            format!("{what} needs one way or more"),
        ));
    }
    Addressing::Destination.admit(ways)
}

/// The labels of `ways`, with `, ` between them.
pub(super) fn labels(ways: &[Way]) -> String {
    ways.iter().map(Way::label).collect::<Vec<_>>().join(", ")
}

/// The first connection of `ways` in a race of the `mode`; the other
/// connections are closed. With `http`, each way dials with
/// [`Dial::dial_http`].
pub(super) async fn first(
    ways: &[Way],
    mode: Failover,
    target: &Target,
    deadline: Instant,
    http: bool,
) -> Result<Connected, Failure> {
    let attempts: Vec<BoxFuture<'_, anyhow::Result<Connected>>> = ways
        .iter()
        .map(|way| {
            let attempt: BoxFuture<'_, anyhow::Result<Connected>> = Box::pin(async move {
                let conn = if http {
                    way.dial.dial_http(target, deadline).await
                } else {
                    way.dial.dial(target, deadline).await
                };
                conn.map_err(anyhow::Error::from)
            });
            attempt
        })
        .collect();
    let close =
        |_, conn: Connected| -> BoxFuture<'static, ()> { Box::pin(async move { drop(conn) }) };
    match race(attempts, mode, close).await {
        Ok(won) => {
            tracing::debug!(way = %ways[won.index].label(), after = ?won.after, %target, "a way won");
            Ok(won.value)
        }
        Err(failures) => Err(combine(ways, target, failures)),
    }
}

/// One failure for the failures of each way: the class of each, when they
/// agree, else `unavailable`; the stage of the first one; the text of
/// each, with the label of its way.
pub(super) fn combine(
    ways: &[Way],
    target: &Target,
    failures: Vec<(usize, anyhow::Error)>,
) -> Failure {
    let parts: Vec<(Stage, Class, String)> = failures
        .iter()
        .map(|(i, e)| {
            let label = ways.get(*i).map(Way::label).unwrap_or_default();
            match e.downcast_ref::<Failure>() {
                Some(f) => (f.stage, f.class, format!("{label}: {}", f.text())),
                None => (
                    Stage::Connect,
                    Class::Unavailable,
                    format!("{label}: {e:#}"),
                ),
            }
        })
        .collect();
    let stage = parts.first().map_or(Stage::Connect, |p| p.0);
    let class = match parts.first() {
        Some(p) if parts.iter().all(|q| q.1 == p.1) => p.1,
        _ => Class::Unavailable,
    };
    let texts: Vec<String> = parts.into_iter().map(|p| p.2).collect();
    Failure::new(
        stage,
        class,
        format!("no way reached {target}: {}", texts.join("; ")),
    )
}

/// The datagrams of the first way of `ways` that gives them.
pub(super) async fn first_datagrams(ways: &[Way]) -> Result<Arc<dyn Datagrams>, Failure> {
    let mut texts = Vec::new();
    let mut class = None;
    for way in ways {
        match way.dial.datagrams().await {
            Ok(d) => return Ok(d),
            Err(e) => {
                class = match class {
                    None => Some(e.class),
                    Some(c) if c == e.class => Some(c),
                    Some(_) => Some(Class::Unavailable),
                };
                texts.push(format!("{}: {}", way.label(), e.text()));
            }
        }
    }
    Err(Failure::new(
        Stage::Connect,
        class.unwrap_or(Class::Unavailable),
        format!("no way carries UDP: {}", texts.join("; ")),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::fakes::{carrier, log, seen, soon, Fake};
    use super::*;

    fn target() -> Target {
        Target::new("example.test", 443)
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_way_to_connect_wins_and_the_stagger_starts_the_next() {
        let log = log();
        let race = Race::new(
            vec![
                Fake::up("slow", &log).after(Duration::from_secs(10)).way(),
                Fake::up("fast", &log).after(Duration::from_secs(1)).way(),
            ],
            Duration::from_secs(2),
        )
        .unwrap();
        let start = Instant::now();
        let conn = race.dial(&target(), soon()).await.unwrap();
        assert_eq!(carrier(conn).await, "fast");
        assert_eq!(
            start.elapsed(),
            Duration::from_secs(3),
            "the stagger, then the dial"
        );
        assert_eq!(seen(&log), ["slow", "fast"]);
        assert_eq!(race.label(), "race(slow, fast)");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_starts_the_next_way_at_once() {
        let log = log();
        let race = Race::new(
            vec![Fake::down("down", &log).way(), Fake::up("up", &log).way()],
            Duration::from_secs(2),
        )
        .unwrap();
        let start = Instant::now();
        let conn = race.dial(&target(), soon()).await.unwrap();
        assert_eq!(carrier(conn).await, "up");
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn the_way_before_wins_when_it_connects_inside_the_stagger() {
        let log = log();
        let race = Race::new(
            vec![
                Fake::up("first", &log).after(Duration::from_secs(1)).way(),
                Fake::up("second", &log).way(),
            ],
            Duration::from_secs(2),
        )
        .unwrap();
        let conn = race.dial(&target(), soon()).await.unwrap();
        assert_eq!(carrier(conn).await, "first");
        assert_eq!(seen(&log), ["first"], "the second way never started");
    }

    #[tokio::test]
    async fn each_failure_is_in_the_one_error_line() {
        let log = log();
        let race = Race::new(
            vec![Fake::down("a", &log).way(), Fake::down("b", &log).way()],
            Duration::from_secs(2),
        )
        .unwrap();
        let err = race
            .dial(&target(), soon())
            .await
            .expect_err("each way fails");
        assert_eq!((err.stage, err.class), (Stage::Connect, Class::Unavailable));
        assert_eq!(
            err.text(),
            "no way reached example.test:443: a: a failed; b: b failed"
        );
        let err = race.datagrams().await.expect_err("no UDP");
        assert!(err.text().starts_with("no way carries UDP: a: "), "{err}");
    }

    #[test]
    fn a_race_refuses_a_peer_way_and_an_empty_list() {
        let log = log();
        let peer = Way::peer(Arc::new(Fake::up("wormhole", &log)));
        let err = Race::new(vec![Fake::up("direct", &log).way(), peer], Duration::ZERO)
            .err()
            .expect("a peer way");
        assert_eq!(err.class, Class::Usage);
        assert!(err.text().starts_with("wormhole meets a peer"), "{err}");
        let err = Race::new(Vec::new(), Duration::ZERO).err().expect("no way");
        assert_eq!(err.text(), "a race needs one way or more");
    }

    #[test]
    fn a_race_sends_the_name_when_one_way_sends_it() {
        let log = log();
        let mut socks = Fake::up("socks5h", &log);
        socks.names_inside = true;
        let race = Race::new(
            vec![Fake::up("direct", &log).way(), socks.way()],
            Duration::ZERO,
        )
        .unwrap();
        assert!(race.names_inside());
    }
}
