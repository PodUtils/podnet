use std::os::unix::fs::PermissionsExt as _;
use std::sync::atomic::AtomicU32;

use super::*;
use crate::network::{BoxFuture, Caps, Class, Connected, Dial, Stage, Target};
use crate::router::{parse, Router};

const FILE: &str = "networks:\n  home: direct\n  other: direct\ngroups:\n  egress: home*2, other*1\nhealth:\n  target: 192.0.2.1:443\n  interval: 5\ndoors:\n  - listen: unix:/tmp/r.sock\n    via: egress\n";

type Log = Arc<Mutex<Vec<String>>>;

/// A way that only has a label: the generation of its network.
struct Gen(String);

impl Dial for Gen {
    fn label(&self) -> String {
        self.0.clone()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            Err(Failure::new(
                Stage::Connect,
                Class::Usage,
                format!("{}: no dial to {target}", self.0),
            ))
        })
    }
}

/// A network with an identity: each `up` and `rotate` writes a line with the
/// state directory that it got, and each close writes a line.
struct Identity {
    log: Log,
    count: Arc<AtomicU32>,
    fail: bool,
}

impl Identity {
    fn link(&self, what: &str, ctx: &Ctx) -> anyhow::Result<Link> {
        if self.fail {
            anyhow::bail!("the service refused");
        }
        let n = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        let dir = ctx.state_dir.as_ref().map(|d| d.display().to_string());
        self.log
            .lock()
            .unwrap()
            .push(format!("{what} gen{n} {}", dir.unwrap_or_default()));
        let log = self.log.clone();
        Ok(
            Link::new(Some(crate::network::Way::destination(Arc::new(Gen(
                format!("gen{n}"),
            )))))
            .on_close(Box::new(move || {
                Box::pin(async move { log.lock().unwrap().push(format!("close gen{n}")) })
            })),
        )
    }
}

impl Network for Identity {
    fn scheme(&self) -> &'static str {
        "identity"
    }

    fn caps(&self) -> Caps {
        Caps::EGRESS_TCP
    }

    fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move { self.link("up", ctx) })
    }

    fn rotate<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move { self.link("rotate", ctx) })
    }
}

fn identity_router(dir: &Path, fail: bool) -> (Arc<Router>, Log) {
    let text = format!("{FILE}state_dir: {}\n", dir.display());
    let router = Router::plan(&parse(&text).unwrap()).unwrap();
    let log: Log = Default::default();
    let (l, count) = (log.clone(), Arc::new(AtomicU32::new(0)));
    router.control.set_connector(Arc::new(move |_| {
        Ok(Arc::new(Identity {
            log: l.clone(),
            count: count.clone(),
            fail,
        }) as Arc<dyn Network>)
    }));
    router.control.set_drain(Duration::ZERO);
    (Arc::new(router), log)
}

fn way_of(router: &Router, member: &str) -> String {
    let status = router.status();
    let members = status["groups"][0]["members"].as_array().unwrap().clone();
    let m = members.iter().find(|m| m["name"] == member).unwrap();
    m["way"].as_str().unwrap().to_string()
}

async fn serve(router: &Arc<Router>, dir: &Path) -> PathBuf {
    let path = dir.join("r.ctl");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(router.clone().serve_control(listener));
    path
}

fn answer_of(err: &anyhow::Error) -> &Answer {
    err.downcast_ref::<Answer>()
        .unwrap_or_else(|| panic!("not an answer of the socket: {err:#}"))
}

/// T-180, the Prove: a rotate makes the new link before it ends the old one,
/// gives the network the same state directory both times (so a WARP network
/// registers no new device), logs each rotation, and keeps a gap between two
/// rotations of one network.
#[tokio::test]
async fn route_control_rotate_reuses_the_state_dir() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let (router, log) = identity_router(&state, false);
    let path = serve(&router, dir.path()).await;
    let body = serde_json::json!({"member": "home"});
    request(&path, "POST", "/route/reconnect", Some(&body))
        .await
        .unwrap();
    let status = request(&path, "POST", "/route/rotate", Some(&body))
        .await
        .unwrap();
    assert_eq!(status["groups"][0]["group"], "egress");
    // The new flows take the new way; the other member is as it was.
    assert_eq!(way_of(&router, "home"), "gen2");
    assert_eq!(way_of(&router, "other"), "other");
    // The gap: a second rotate at once is refused and makes no link.
    let err = request(&path, "POST", "/route/rotate", Some(&body))
        .await
        .unwrap_err();
    assert_eq!(answer_of(&err).code, 429, "{err:#}");
    router.control.set_rotate_gap(Duration::ZERO);
    request(&path, "POST", "/route/rotate", Some(&body))
        .await
        .unwrap();
    for _ in 0..200 {
        if log.lock().unwrap().len() >= 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let want = state.join("home").display().to_string();
    // Make before break: the close of a link follows the up of the next.
    assert_eq!(
        *log.lock().unwrap(),
        [
            format!("up gen1 {want}"),
            format!("rotate gen2 {want}"),
            "close gen1".to_string(),
            format!("rotate gen3 {want}"),
            "close gen2".to_string(),
        ]
    );
    assert!(state.join("home").is_dir());
    // A network that does not come up leaves the way as it was.
    let (router, log) = identity_router(&state, true);
    let err = router
        .handle("POST", "/route/rotate", body.to_string().as_bytes())
        .await
        .unwrap_err();
    assert_eq!(err.code, 502, "{err:?}");
    assert_eq!(way_of(&router, "home"), "home");
    assert!(log.lock().unwrap().is_empty());
    let err = router
        .handle("POST", "/route/rotate", br#"{"member":"nowhere"}"#)
        .await
        .unwrap_err();
    assert_eq!(err.code, 404);
}

/// T-180, the Prove: the control socket has the gate of the status socket (a
/// mode of 0600 and a peer of the same user, or root). A peer that the gate
/// refuses gets 403 for a read and for each write, and nothing changes.
#[tokio::test]
async fn route_control_needs_the_gate() {
    assert!(peer_allowed(1000, 1000));
    assert!(peer_allowed(0, 1000));
    assert!(!peer_allowed(1001, 1000));
    let dir = tempfile::tempdir().unwrap();
    let (router, log) = identity_router(&dir.path().join("state"), false);
    let path = dir.path().join("gate.ctl");
    let listener = podnet_netns::listen::bind_private(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    tokio::spawn(
        router
            .clone()
            .serve_control_gated(listener, Arc::new(|_| false)),
    );
    let select = serde_json::json!({"group": "egress", "member": "other"});
    for (method, route, body) in [
        ("GET", "/route/status", None),
        ("POST", "/route/select", Some(&select)),
        (
            "POST",
            "/route/rotate",
            Some(&serde_json::json!({"member": "home"})),
        ),
        (
            "POST",
            "/route/reconnect",
            Some(&serde_json::json!({"member": "home"})),
        ),
        (
            "PATCH",
            "/route/interval",
            Some(&serde_json::json!({"seconds": 1})),
        ),
    ] {
        let err = request(&path, method, route, body).await.unwrap_err();
        assert_eq!(answer_of(&err).code, 403, "{route}: {err:#}");
    }
    assert!(log.lock().unwrap().is_empty());
    assert_eq!(router.groups[0].1.pinned(), None);
    assert_eq!(router.control.interval(), Some(Duration::from_secs(5)));
    // The gate of the router: the same peer gets through when it is allowed.
    let open = dir.path().join("open.ctl");
    let listener = tokio::net::UnixListener::bind(&open).unwrap();
    tokio::spawn(router.clone().serve_control(listener));
    request(&open, "POST", "/route/select", Some(&select))
        .await
        .unwrap();
    assert_eq!(router.groups[0].1.pinned(), Some("other".to_string()));
}

/// T-180: select pins a member and `auto` frees it; the interval changes
/// the probe; a bad request gets 400, an unknown name 404, a wrong method 405.
#[tokio::test]
async fn route_control_select_and_interval() {
    let dir = tempfile::tempdir().unwrap();
    let (router, _) = identity_router(&dir.path().join("state"), false);
    let path = serve(&router, dir.path()).await;
    let pin = |m: serde_json::Value| serde_json::json!({"group": "egress", "member": m});
    let status = request(&path, "POST", "/route/select", Some(&pin("other".into())))
        .await
        .unwrap();
    assert_eq!(status["groups"][0]["pinned"], "other");
    let target: Target = "example.com:443".parse().unwrap();
    for _ in 0..30 {
        assert_eq!(router.decide(0, &target).member.unwrap().0, "other");
    }
    let status = request(&path, "POST", "/route/select", Some(&pin("auto".into())))
        .await
        .unwrap();
    assert_eq!(status["groups"][0]["pinned"], serde_json::Value::Null);
    let status = request(
        &path,
        "PATCH",
        "/route/interval",
        Some(&serde_json::json!({"seconds": 2})),
    )
    .await
    .unwrap();
    assert_eq!(status["interval"], 2);
    assert_eq!(router.control.interval(), Some(Duration::from_secs(2)));
    // A change of the interval ends the wait of the probe at once.
    let waiting = router.clone();
    let pause = tokio::spawn(async move { waiting.control.pause().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    request(
        &path,
        "PATCH",
        "/route/interval",
        Some(&serde_json::json!({"seconds": 9})),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), pause)
        .await
        .unwrap()
        .unwrap();
    for (method, route, body, code) in [
        (
            "POST",
            "/route/select",
            br#"{"group":"nope","member":null}"#.as_slice(),
            404,
        ),
        (
            "POST",
            "/route/select",
            br#"{"group":"egress","member":"nope"}"#,
            404,
        ),
        ("POST", "/route/select", b"", 400),
        ("POST", "/route/select", br#"{"member":"home"}"#, 400),
        ("PATCH", "/route/interval", br#"{"seconds":0}"#, 400),
        ("PATCH", "/route/interval", br#"{"seconds":"x"}"#, 400),
        ("GET", "/route/rotate", b"", 405),
        ("POST", "/route/status", b"", 405),
        ("POST", "/route/nothing", b"", 404),
    ] {
        let err = router.handle(method, route, body).await.unwrap_err();
        assert_eq!(err.code, code, "{method} {route}: {err:?}");
    }
    let plain = Router::plan(
        &parse(&FILE.replace("health:\n  target: 192.0.2.1:443\n  interval: 5\n", "")).unwrap(),
    )
    .unwrap();
    let err = plain
        .handle("PATCH", "/route/interval", br#"{"seconds":3}"#)
        .await
        .unwrap_err();
    assert_eq!(err.code, 409);
}
