//! T-097, items 3, 7, 9 and 10: the probe of the router renews the member of
//! a network that renews (WARP), with a gap, and only when the control says
//! that the host is up. The networks here are fakes that give a link when
//! `up` runs; the WARP network itself runs against the stand-in edge in
//! `src/networks/warp_tests.rs`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use super::*;
use crate::network::{
    BoxFuture, Caps, Class, Connected, EgressCheck, Failure, Link, Network, ProbeResult, Stage,
};

/// What the dials of a network do.
#[derive(Clone, Copy)]
enum Mode {
    Pass,
    Fail(Class),
}

type Shared = Arc<Mutex<Mode>>;

struct Script {
    name: String,
    mode: Shared,
}

impl Dial for Script {
    fn label(&self) -> String {
        self.name.clone()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let mode = *self.mode.lock().unwrap();
            match mode {
                Mode::Pass => {
                    // The far end is an echo endpoint of the operator: it
                    // writes the address that it saw, and closes.
                    use tokio::io::AsyncWriteExt as _;
                    let (a, mut b) = tokio::io::duplex(64);
                    tokio::spawn(async move {
                        let _ = b.write_all(b"127.0.0.1\n").await;
                        let _ = b.shutdown().await;
                    });
                    Ok(Connected::new(Box::new(a)))
                }
                Mode::Fail(class) => Err(Failure::new(Stage::Connect, class, "scripted")),
            }
        })
    }
}

/// A network that renews: each `up` gives a link with a new label.
struct Renewing {
    mode: Shared,
    ups: Arc<AtomicU32>,
    check: Option<Arc<Mutex<ProbeResult>>>,
    renews: bool,
}

impl Network for Renewing {
    fn scheme(&self) -> &'static str {
        "fake"
    }

    fn caps(&self) -> Caps {
        Caps::EGRESS_TCP
    }

    fn up<'a>(&'a self, _ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move {
            let n = self.ups.fetch_add(1, Ordering::SeqCst) + 1;
            let dial = Arc::new(Script {
                name: format!("renewed{n}"),
                mode: self.mode.clone(),
            });
            Ok(Link::new(Some(Way::destination(dial))))
        })
    }

    fn renews(&self) -> bool {
        self.renews
    }

    fn egress_check(&self) -> Option<EgressCheck> {
        let result = self.check.clone()?;
        Some(Arc::new(move |_way, _timeout| {
            let result = *result.lock().unwrap();
            Box::pin(async move { result })
        }))
    }
}

struct Harness {
    router: Router,
    mode: Shared,
    ups: Arc<AtomicU32>,
}

fn file(extra: &str) -> String {
    format!(
        "networks:\n  w: warp\n  d: direct\ngroups:\n  g: w*1, d*1\nhealth:\n  target: 192.0.2.1:443\n{extra}doors:\n  - listen: unix:/tmp/r.sock\n    via: g\n"
    )
}

/// A router over `w` (scripted by `mode`) and `d` (always passes).
fn harness(
    text: &str,
    renews: bool,
    check: Option<Arc<Mutex<ProbeResult>>>,
    first: Mode,
    second: Mode,
) -> Harness {
    let config = parse(text).unwrap();
    let mode: Shared = Arc::new(Mutex::new(first));
    let pass: Shared = Arc::new(Mutex::new(second));
    let mut ways = HashMap::new();
    for (name, shared) in [("w", mode.clone()), ("d", pass)] {
        let dial = Arc::new(Script {
            name: name.into(),
            mode: shared,
        });
        ways.insert(name.to_string(), Way::destination(dial));
    }
    let router = Router::build(&config, ways).unwrap();
    let ups = Arc::new(AtomicU32::new(0));
    let (m, u, c) = (mode.clone(), ups.clone(), check);
    router.control.set_connector(Arc::new(move |spec| {
        // Only `w` is WARP; `d` is a network that renews nothing.
        let warp = spec == "warp";
        Ok(Arc::new(Renewing {
            mode: m.clone(),
            ups: u.clone(),
            check: c.clone().filter(|_| warp),
            renews: renews && warp,
        }) as Arc<dyn Network>)
    }));
    router.control.set_drain(Duration::ZERO);
    Harness { router, mode, ups }
}

fn health_of(router: &Router, member: &str) -> String {
    let members = router.status()["groups"][0]["members"].clone();
    let m = members
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == member)
        .unwrap()
        .clone();
    m["health"].as_str().unwrap().to_string()
}

fn way_of(router: &Router, member: &str) -> String {
    let members = router.status()["groups"][0]["members"].clone();
    let m = members
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == member)
        .unwrap()
        .clone();
    m["way"].as_str().unwrap().to_string()
}

/// Items 3 and 10: K clean misses with a passed control take the member to
/// `Down`, and its network gives it a new link that the new flows take.
#[tokio::test]
async fn router_health_renews_a_member_that_goes_down() {
    let h = harness(
        &file(""),
        true,
        None,
        Mode::Fail(Class::Unavailable),
        Mode::Pass,
    );
    for round in 1..=3 {
        h.router.probe_round().await;
        assert_eq!(health_of(&h.router, "w"), "cooldown", "round {round}");
        assert_eq!(h.ups.load(Ordering::SeqCst), 0, "round {round}");
    }
    h.router.probe_round().await;
    assert_eq!(health_of(&h.router, "w"), "down");
    assert_eq!(h.ups.load(Ordering::SeqCst), 1, "one renew at the step");
    assert_eq!(way_of(&h.router, "w"), "renewed1");
    // The link works now: the member comes back through probation.
    *h.mode.lock().unwrap() = Mode::Pass;
    h.router.probe_round().await;
    assert_eq!(health_of(&h.router, "w"), "probation");
    assert_eq!(h.ups.load(Ordering::SeqCst), 1, "a pass renews nothing");
}

/// Item 7: when each member fails, the control fails: the host is out, so
/// no member goes down and nothing renews.
#[tokio::test]
async fn router_health_outage_of_each_member_renews_nothing() {
    let down = Mode::Fail(Class::Unavailable);
    let h = harness(&file(""), true, None, down, down);
    for _ in 0..8 {
        h.router.probe_round().await;
    }
    assert_eq!(health_of(&h.router, "w"), "cooldown");
    assert_eq!(health_of(&h.router, "d"), "cooldown");
    assert_eq!(h.ups.load(Ordering::SeqCst), 0);
}

/// Item 10: a refused credential is `AuthExpired`: the member renews at the
/// first miss, and a second renew waits for the gap.
#[tokio::test]
async fn router_health_auth_expired_renews_at_once_and_keeps_a_gap() {
    let h = harness(
        &file(""),
        true,
        None,
        Mode::Fail(Class::Refused),
        Mode::Pass,
    );
    h.router.probe_round().await;
    assert_eq!(h.ups.load(Ordering::SeqCst), 1, "the first miss renews");
    h.router.probe_round().await;
    h.router.probe_round().await;
    assert_eq!(h.ups.load(Ordering::SeqCst), 1, "the gap holds the next");
    h.router.control.set_renew_gap(Duration::ZERO);
    h.router.probe_round().await;
    assert_eq!(h.ups.load(Ordering::SeqCst), 2, "after the gap");
}

/// A network that does not ask for a renew (each network but WARP) is not
/// renewed, whatever its health.
#[tokio::test]
async fn router_health_does_not_renew_a_network_that_does_not_renew() {
    let h = harness(
        &file(""),
        false,
        None,
        Mode::Fail(Class::Unavailable),
        Mode::Pass,
    );
    for _ in 0..6 {
        h.router.probe_round().await;
    }
    assert_eq!(health_of(&h.router, "w"), "down");
    assert_eq!(h.ups.load(Ordering::SeqCst), 0);
    assert_eq!(way_of(&h.router, "w"), "w");
}

/// Item 9: the egress check of the network runs when the connect passed;
/// `WrongEgress` takes the member down at once, and it renews.
#[tokio::test]
async fn router_health_runs_the_egress_check_of_the_network() {
    let check = Arc::new(Mutex::new(ProbeResult::WrongEgress));
    let h = harness(&file(""), true, Some(check.clone()), Mode::Pass, Mode::Pass);
    h.router.probe_round().await;
    assert_eq!(health_of(&h.router, "w"), "down", "the wrong egress");
    assert_eq!(health_of(&h.router, "d"), "up");
    assert_eq!(h.ups.load(Ordering::SeqCst), 1);
    // The member that has no check of its own is not asked.
    *check.lock().unwrap() = ProbeResult::Ok(Duration::from_millis(3));
    h.router.probe_round().await;
    assert_eq!(health_of(&h.router, "w"), "probation");
}

/// Item 9: the check of the operator (`health.egress`) goes before the check
/// of the network.
#[tokio::test]
async fn router_health_egress_of_the_operator_goes_before_the_network() {
    let echo = "192.0.2.7:7";
    let text = file(&format!("  egress:\n    w: {echo}=127.0.0.1\n"));
    let check = Arc::new(Mutex::new(ProbeResult::WrongEgress));
    let h = harness(&text, true, Some(check), Mode::Pass, Mode::Pass);
    h.router.probe_round().await;
    assert_eq!(health_of(&h.router, "w"), "up", "the operator check passed");
    assert_eq!(h.ups.load(Ordering::SeqCst), 0);
}
