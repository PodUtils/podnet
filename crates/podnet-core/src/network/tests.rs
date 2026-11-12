use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

use super::*;

/// A dialer with no network: each dial gives one end of a pipe, and a
/// task echoes the bytes of the other end.
struct FakeDial {
    label: &'static str,
}

impl Dial for FakeDial {
    fn label(&self) -> String {
        self.label.to_string()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let (near, mut far) = tokio::io::duplex(1024);
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                while let Ok(n) = far.read(&mut buf).await {
                    if n == 0 || far.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
            Ok(Connected::new(Box::new(near)))
        })
    }
}

fn fake(label: &'static str) -> Arc<dyn Dial> {
    Arc::new(FakeDial { label })
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[tokio::test]
async fn a_fake_dialer_carries_bytes_and_gives_no_udp() {
    let dial = fake("fake");
    let target = Target::new("example.test", 80);
    let mut conn = dial.dial(&target, soon()).await.unwrap();
    conn.io.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    conn.io.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"ping");
    assert!(!dial.names_inside());
    let err = dial.datagrams().await.expect_err("no UDP");
    assert_eq!((err.stage, err.class), (Stage::Connect, Class::Unavailable));
    assert!(err.to_string().contains("fake carries no UDP"), "{err}");
}

#[test]
fn a_peer_dialer_in_a_destination_chain_is_refused() {
    let chain = [Way::destination(fake("direct")), Way::peer(fake("gsr"))];
    let err = Addressing::Destination
        .admit(&chain)
        .expect_err("a destination consumer refuses a peer way");
    assert_eq!((err.stage, err.class), (Stage::Connect, Class::Usage));
    let line = err.to_string();
    assert!(line.contains("gsr meets a peer"), "{line}");
    assert!(!line.contains('\n'), "one line: {line}");
}

#[test]
fn a_destination_chain_of_destination_dialers_is_admitted() {
    let chain = [
        Way::destination(fake("direct")),
        Way::destination(fake("socks5h://p:1080")),
    ];
    Addressing::Destination.admit(&chain).unwrap();
    Addressing::Destination.admit(&[]).unwrap();
}

#[test]
fn a_peer_consumer_takes_each_kind_of_way() {
    let chain = [Way::peer(fake("gsr")), Way::destination(fake("direct"))];
    Addressing::Peer.admit(&chain).unwrap();
    assert_eq!(chain[0].label(), "gsr");
    assert_eq!(
        format!("{:?}", chain[0]),
        r#"Way { dial: "gsr", addressing: Peer }"#
    );
    assert_eq!(Addressing::Peer.to_string(), "peer");
    assert_eq!(Addressing::Destination.name(), "destination");
}

#[test]
fn a_peer_target_has_a_name_and_no_port() {
    let target = peer("office");
    assert_eq!(target.host(), "office");
    assert_eq!(target.port(), 0);
    assert_eq!(target.to_string(), "office:0");
}

#[test]
fn caps_combine_and_name_each_bit() {
    let caps = Caps::EGRESS_TCP | Caps::EGRESS_UDP | Caps::EXPORT;
    assert!(caps.contains(Caps::EGRESS_TCP));
    assert!(caps.contains(Caps::EGRESS_TCP | Caps::EXPORT));
    assert!(!caps.contains(Caps::INGRESS));
    assert!(caps.contains(Caps::NONE));
    assert_eq!(caps.names(), ["egress-tcp", "egress-udp", "export"]);
    assert!(Caps::default().names().is_empty());
    assert_eq!(Caps::KERNEL_IFACE.names(), ["kernel-iface"]);
}

/// A network that gives the fake dialer, one export, and a close function
/// that sets a flag.
struct FakeNetwork {
    closed: Arc<AtomicBool>,
}

impl Network for FakeNetwork {
    fn scheme(&self) -> &'static str {
        "fake"
    }

    fn caps(&self) -> Caps {
        Caps::EGRESS_TCP | Caps::EXPORT
    }

    fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move {
            let below = ctx.below.as_ref().map(|d| d.label());
            let closed = self.closed.clone();
            Ok(Link::new(Some(Way::destination(fake("fake"))))
                .with_export(Export {
                    kind: "env",
                    name: "fake.env".into(),
                    text: "ALL_PROXY=socks5h://127.0.0.1:1080\n".into(),
                })
                .with_status(serde_json::json!({ "below": below }))
                .on_close(Box::new(move || {
                    Box::pin(async move { closed.store(true, Ordering::SeqCst) })
                })))
        })
    }
}

#[tokio::test]
async fn a_network_comes_up_and_its_link_closes() {
    let closed = Arc::new(AtomicBool::new(false));
    let net: Box<dyn Network> = Box::new(FakeNetwork {
        closed: closed.clone(),
    });
    assert_eq!(net.scheme(), "fake");
    assert!(net.caps().contains(Caps::EXPORT));
    assert!(net.checks().is_empty());
    let ctx = Ctx {
        below: Some(fake("socks5h://p:1080")),
        ..Ctx::default()
    };
    assert_eq!(
        format!("{ctx:?}"),
        r#"Ctx { below: Some("socks5h://p:1080") }"#
    );
    let link = net.up(&ctx).await.unwrap();
    assert_eq!(link.status["below"], "socks5h://p:1080");
    assert_eq!(link.exports[0].kind, "env");
    let way = link.way.clone().expect("a way out");
    assert_eq!(way.addressing, Addressing::Destination);
    let mut conn = way
        .dial
        .dial(&Target::new("10.0.0.1", 22), soon())
        .await
        .unwrap();
    conn.io.write_all(b"x").await.unwrap();
    assert!(format!("{link:?}").contains("close: true"));
    link.close().await;
    assert!(closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_link_with_no_close_function_closes() {
    let link = Link::new(None);
    assert!(link.way.is_none());
    assert!(link.status.is_null());
    link.close().await;
}
