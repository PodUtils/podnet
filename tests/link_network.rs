//! `peer:FINGERPRINT/SERVICE` as a network of the registry (T-266,
//! GitHub #466 row E0-5): it resolves to one stream of a link that a
//! running instance holds, and a peer that is not the named fingerprint is
//! refused with exit 77 before the service opens. The tests bind no TCP
//! port: the link, the control socket and the far origin are unix sockets.
//!
//! The process variable `PODNET_LINK_CONTROL` names the one control socket
//! that the network asks. One test sets it, so the tests do not race.

use std::sync::Arc;
use std::time::Duration;

use pod_identity::PublicKey;
use podnet::exit::code_of;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings};
use podnet::link_ctl::{Control, Hub, Slot, CONTROL_VARIABLE};
use podnet::link_service as service;
use podnet::network::{Addressing, BoxedIo, Ctx, Network as _};
use podnet::networks;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Instant};

const LIMIT: Duration = Duration::from_secs(20);

fn install(dir: &std::path::Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

fn list(key: &Identity) -> AllowList {
    AllowList::parse(&key.fingerprint()).unwrap()
}

#[test]
fn the_registry_has_a_row_for_the_scheme_peer() {
    let row = networks::networks()
        .iter()
        .find(|r| r.scheme == "peer")
        .expect("a row for peer");
    assert_eq!(row.forms, &["peer:FINGERPRINT/SERVICE"]);
}

#[test]
fn a_peer_spec_names_the_fingerprint_and_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let key = install(dir.path(), "k");
    let fp = key.fingerprint();
    let spec = format!("peer:{fp}/http");
    let network = networks::parse(&spec).unwrap();
    assert_eq!(network.scheme(), "peer");
    assert_eq!(network.label(), spec);
    assert_eq!(network.row().scheme, "peer");
    // The SSH form of a fingerprint is a fingerprint too.
    let ssh = key.report().ssh_fingerprint;
    assert!(networks::parse(&format!("peer:{ssh}/a.b_c-1")).is_ok());
}

#[test]
fn a_bad_peer_spec_is_a_usage_failure() {
    let dir = tempfile::tempdir().unwrap();
    let fp = install(dir.path(), "k").fingerprint();
    for spec in [
        "peer:".to_string(),
        "peer:/http".to_string(),
        format!("peer:{fp}"),
        format!("peer:{fp}/"),
        format!("peer:{fp}/a b"),
        format!("peer:{fp}/{}", "x".repeat(33)),
        "peer:not-a-fingerprint/http".to_string(),
        format!("peer:{fp}/http/more"),
    ] {
        let err = networks::parse(&spec)
            .err()
            .unwrap_or_else(|| panic!("{spec} parsed"));
        assert_eq!(err.class, podnet::network::Class::Usage, "{spec}: {err}");
    }
    // A host named peer with a port is the SOCKS hop that it was.
    assert_eq!(networks::parse("peer:1080").unwrap().scheme(), "proxy");
}

/// The far side of a link: it admits streams and serves the service
/// `echo` from a unix origin that echoes, `gone` from an origin that does
/// not exist.
async fn far(link: Arc<Link>, origin: std::path::PathBuf) {
    while let Ok(mut stream) = link.accept().await {
        let origin = origin.clone();
        tokio::spawn(async move {
            let Ok(name) = service::accept(&mut stream).await else {
                return;
            };
            match name.as_str() {
                "echo" => {
                    let mut io = tokio::net::UnixStream::connect(&origin).await.unwrap();
                    service::reply(&mut stream, service::OPEN).await.unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut io).await;
                }
                "gone" => {
                    let _ = service::reply(&mut stream, service::NO_ORIGIN).await;
                }
                _ => {
                    let _ = service::reply(&mut stream, service::NO_SERVICE).await;
                }
            }
        });
    }
}

fn echo(path: &std::path::Path) {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = std::io::Read::read(&mut conn, &mut buf) {
                    if n == 0 || std::io::Write::write_all(&mut conn, &buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
}

struct World {
    _dir: tempfile::TempDir,
    near: Identity,
    far_key: Identity,
    stranger: Identity,
    hub: Arc<Hub>,
    _control: Control,
    _slot: Slot,
    _near_link: Arc<Link>,
}

/// A near instance that holds a link to a far peer; the far peer serves
/// `echo`, `gone` and nothing else. `PODNET_LINK_CONTROL` names the socket.
async fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let (near, far_key, stranger) = (
        install(dir.path(), "near"),
        install(dir.path(), "far"),
        install(dir.path(), "stranger"),
    );
    let origin = dir.path().join("origin.sock");
    echo(&origin);
    let (dialer, listener) = link::memory_pair(1 << 20, Settings::default()).unwrap();
    let (d, l): (_, anyhow::Result<PublicKey>) = tokio::join!(
        hello::dial(&dialer, &near, list(&far_key)),
        hello::accept(&listener, &far_key, list(&near))
    );
    let far_public = d.unwrap();
    l.unwrap();
    tokio::spawn(far(Arc::new(listener), origin));
    let near_link = Arc::new(dialer);
    let hub = Hub::new(&near.fingerprint());
    let slot = hub.hold(near_link.clone(), far_public, "memory", "pair", "dialer");
    let socket = dir.path().join("ctl.sock");
    let control = Control::start(&socket, hub.clone()).unwrap();
    std::env::set_var(CONTROL_VARIABLE, &socket);
    World {
        _dir: dir,
        near,
        far_key,
        stranger,
        hub,
        _control: control,
        _slot: slot,
        _near_link: near_link,
    }
}

async fn dial_service(spec: &str) -> anyhow::Result<BoxedIo> {
    let network = networks::parse(spec).unwrap();
    let up = network.up(&Ctx::default()).await?;
    let way = up.way.expect("a way");
    assert_eq!(way.addressing, Addressing::Peer);
    // The target is not read by a peer way: the service is in the spec.
    let target = podnet::network::peer("unused");
    let conn = way
        .dial
        .dial(&target, Instant::now() + LIMIT)
        .await
        .map_err(anyhow::Error::from)?;
    Ok(conn.io)
}

#[tokio::test]
async fn a_peer_network_reaches_a_far_unix_origin_and_refuses_a_stranger_with_77() {
    let w = world().await;
    let fp = w.far_key.fingerprint();

    // One stream to the service of the named peer: the bytes reach the far
    // origin and come back.
    let mut s = dial_service(&format!("peer:{fp}/echo")).await.unwrap();
    s.write_all(b"through a peer").await.unwrap();
    let mut got = [0u8; 14];
    timeout(LIMIT, s.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got, b"through a peer");
    assert_eq!(w.hub.status()["links"][0]["opened"], 1);

    // A peer that is not the named fingerprint: 77 at the `up`, and no
    // stream opens (the counter stays).
    let stranger = w.stranger.fingerprint();
    let err = dial_service(&format!("peer:{stranger}/echo"))
        .await
        .err()
        .expect("fails");
    assert_eq!(code_of(&err), 77, "{err:#}");
    let err = dial_service(&format!("peer:{}/echo", w.near.fingerprint()))
        .await
        .err()
        .expect("fails");
    assert_eq!(code_of(&err), 77, "this install is not the peer: {err:#}");
    assert_eq!(w.hub.status()["links"][0]["opened"], 1, "no stream opened");

    // A service that the far side does not have, and one whose origin
    // does not answer: the dial fails (69), and the link stays up.
    for service in ["nope", "gone"] {
        let err = dial_service(&format!("peer:{fp}/{service}"))
            .await
            .err()
            .expect("fails");
        let failure = err.downcast_ref::<podnet::network::Failure>().unwrap();
        assert_eq!(
            failure.class,
            podnet::network::Class::Unavailable,
            "{service}: {err:#}"
        );
    }
    let mut s = dial_service(&format!("peer:{fp}/echo")).await.unwrap();
    s.write_all(b"again").await.unwrap();
    let mut got = [0u8; 5];
    timeout(LIMIT, s.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got, b"again");

    // The way is a peer way: a consumer of destinations refuses it.
    let network = networks::parse(&format!("peer:{fp}/echo")).unwrap();
    let up = network.up(&Ctx::default()).await.unwrap();
    let way = up.way.unwrap();
    assert!(Addressing::Destination
        .admit(std::slice::from_ref(&way))
        .is_err());

    // No instance that runs (a socket that nobody serves): 69.
    let missing = w._dir.path().join("none.sock");
    std::env::set_var(CONTROL_VARIABLE, &missing);
    let network = networks::parse(&format!("peer:{fp}/echo")).unwrap();
    let err = network.up(&Ctx::default()).await.err().unwrap();
    assert_eq!(code_of(&err), 69, "{err:#}");
}
