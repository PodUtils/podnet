//! The WARP network against the stand-in API and the stand-in edge of
//! `pod-warp` on the loopback (T-097, T-099, T-180, T-061). No test calls
//! WARP.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pod_testkit::stand;
use pod_warp::{Api, IdentityFile, Settings, Warp, Way as WarpWay};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::warp::{Device, Source, Spec, WarpNetwork};
use crate::network::{
    BoxFuture, Class, Connected, Ctx, Datagrams, Dial, Failure, Network, ProbeResult, Stage,
    Target, Way,
};
use crate::warp::exit::Exit;
use crate::warp::WarpStack;

#[allow(dead_code)]
pub(crate) mod edge;

use edge::{Edge, Route};

const LIMIT: Duration = Duration::from_secs(20);
/// The TCP port of the echo service inside the tunnel.
const ECHO_TCP: u16 = 80;
pub(crate) const TRACE_OK: &str =
    "fl=1\nh=one.one.one.one\nip=104.28.0.9\ncolo=AMS\nloc=NL\nwarp=on\n";
pub(crate) const TRACE_OFF: &str =
    "fl=1\nh=one.one.one.one\nip=104.28.0.9\ncolo=AMS\nloc=NL\nwarp=off\n";

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A dial that sends each address of its map to its loopback address, and
/// each other target to the loopback address of the same port.
pub(crate) struct Moved(pub Vec<(SocketAddr, SocketAddr)>);

impl Dial for Moved {
    fn label(&self) -> String {
        "the moved dial of the test".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let to = match target {
                Target::Addr(addr) => self
                    .0
                    .iter()
                    .find(|(from, _)| from == addr)
                    .map(|(_, to)| *to),
                Target::Name { .. } => None,
            };
            let to = to
                .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), target.port()));
            let tcp = tokio::net::TcpStream::connect(to)
                .await
                .map_err(|e| Failure::io(Stage::Connect, e))?;
            Ok(Connected {
                io: Box::new(tcp),
                peer: Some(to),
                local: None,
            })
        })
    }
}

fn http_client() -> pod_proxy::Client {
    let dialer = pod_proxy::Dialer::new(
        pod_proxy::Settings::new("podnet", pod_proxy::ProxyChoice::Direct),
        Arc::new(pod_proxy::resolve::System::new()),
    );
    let context = Arc::new(pod_tls::Context::new(
        provider(),
        pod_tls::Settings::new(pod_tls::Words::new("podnet")),
    ));
    pod_proxy::Client::new(context, Arc::new(dialer)).with_user_agent("podnet/0.0.0-test")
}

/// One certificate for each handshake.
#[derive(Debug)]
struct One(Arc<rustls::sign::CertifiedKey>);

impl rustls::server::ResolvesServerCert for One {
    fn resolve(
        &self,
        _hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// An HTTPS server of the trace on the loopback with a key of its own, the
/// TLS client that pins that key, and the body that it serves.
async fn trace_server() -> (
    SocketAddr,
    Arc<rustls::ClientConfig>,
    Arc<Mutex<&'static str>>,
) {
    let key = pod_warp::key::DeviceKey::generate().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(One(Arc::new(key.certified(now - 60)))));
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = Arc::new(Mutex::new(TRACE_OK));
    let served = body.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, body) = (acceptor.clone(), served.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && tls.read_exact(&mut byte).await.is_ok() {
                    head.push(byte[0]);
                }
                let text = *body.lock().unwrap();
                let answer = format!("HTTP/1.0 200 OK\r\ncontent-type: text/plain\r\n\r\n{text}");
                let _ = tls.write_all(answer.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    let identity = pod_warp::Identity::generate().unwrap();
    let client = pod_warp::tls::client_config(provider(), &identity, key.spki(), b"h2").unwrap();
    (addr, client, body)
}

/// The world of a test: the API, the edge with an echo route and the trace
/// on port 443, and the source that gives the network its devices.
pub(crate) struct World {
    pub api: stand::warp::Api,
    pub edge: Edge,
    pub source: Arc<Standin>,
    pub trace: Arc<Mutex<&'static str>>,
    _served: stand::Stand,
}

pub(crate) struct Standin {
    base: String,
    dial: Arc<Moved>,
    tls: Arc<rustls::ClientConfig>,
    pub opened: AtomicUsize,
}

struct Standing(Warp);

impl Device for Standing {
    fn connect(&self) -> BoxFuture<'_, anyhow::Result<WarpStack>> {
        Box::pin(async move { self.0.connect().await.map_err(anyhow::Error::new) })
    }

    fn rotate_key(&mut self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move { self.0.rotate_key().await.map_err(anyhow::Error::new) })
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        self.0.save(path).map_err(anyhow::Error::new)
    }
}

impl Standin {
    pub fn settings(&self) -> Settings {
        let api = Api::new(http_client()).with_bases(vec![self.base.clone()]);
        let mut way = WarpWay::new(self.dial.clone(), provider());
        way.timeout = Duration::from_secs(10);
        Settings::new(api, way)
    }
}

impl Source for Standin {
    fn open<'a>(
        &'a self,
        path: Option<&'a Path>,
    ) -> BoxFuture<'a, anyhow::Result<Box<dyn Device>>> {
        Box::pin(async move {
            self.opened.fetch_add(1, Ordering::SeqCst);
            let warp = match path {
                Some(path) => {
                    Warp::register_or_load(self.settings(), &IdentityFile::new(path)).await
                }
                None => Warp::register(self.settings()).await,
            }
            .map_err(anyhow::Error::new)?;
            Ok(Box::new(Standing(warp)) as Box<dyn Device>)
        })
    }

    fn trace<'a>(&'a self, net: &'a dyn Dial) -> BoxFuture<'a, Result<Exit, Failure>> {
        Box::pin(async move { pod_warp::exit::trace(net, &self.tls).await })
    }
}

pub(crate) async fn world() -> World {
    let echo = edge::echo_server().await;
    let (trace_addr, tls, trace) = trace_server().await;
    let api = stand::warp::Api::new("", "192.0.2.1", "2001:db8::1");
    let routes = vec![
        Route {
            port: ECHO_TCP,
            outside: echo,
        },
        Route {
            port: 443,
            outside: trace_addr,
        },
    ];
    let edge = Edge::start(api.clone(), routes).await;
    api.serve_key(&edge.pem());
    let served = stand::http(api.handler()).unwrap();
    let dial = Arc::new(Moved(vec![
        (edge::door_v4(), edge.addr),
        (edge::door_v6(), edge.addr),
    ]));
    let source = Arc::new(Standin {
        base: format!("http://{}", served.addr()),
        dial,
        tls,
        opened: AtomicUsize::new(0),
    });
    World {
        api,
        edge,
        source,
        trace,
        _served: served,
    }
}

impl World {
    /// The network over the stand-ins, with a quick backoff for a session
    /// that comes back.
    pub fn network(&self) -> WarpNetwork {
        let live = pod_warp::LiveSettings {
            base: Duration::from_millis(50),
            probe: None,
            ..pod_warp::LiveSettings::default()
        };
        WarpNetwork::with_source(Spec { identity: None }, self.source.clone(), live)
    }
}

impl World {
    /// A function that makes the network, for the connector of a router.
    pub fn maker(&self) -> Arc<dyn Fn() -> WarpNetwork + Send + Sync> {
        let source = self.source.clone();
        Arc::new(move || {
            let live = pod_warp::LiveSettings {
                base: Duration::from_millis(50),
                probe: None,
                ..pod_warp::LiveSettings::default()
            };
            WarpNetwork::with_source(Spec { identity: None }, source.clone(), live)
        })
    }
}

pub(crate) fn ctx_of(dir: &Path) -> Ctx {
    Ctx {
        below: None,
        state_dir: Some(dir.to_path_buf()),
    }
}

fn target() -> Target {
    Target::Addr("203.0.113.9:80".parse().unwrap())
}

/// Send `data` through `dial` to the echo route and read it back.
pub(crate) async fn echo(dial: &dyn Dial, data: &[u8]) -> Vec<u8> {
    let connected = dial
        .dial(&target(), tokio::time::Instant::now() + LIMIT)
        .await
        .unwrap();
    let (mut read, mut write) = tokio::io::split(connected.io);
    let sent = data.to_vec();
    let writer = tokio::spawn(async move {
        write.write_all(&sent).await.unwrap();
        write
    });
    let mut got = vec![0u8; data.len()];
    tokio::time::timeout(LIMIT, read.read_exact(&mut got))
        .await
        .expect("the echo in the limit")
        .unwrap();
    let _write = writer.await.unwrap();
    got
}

async fn up(world: &World, dir: &Path) -> crate::network::Link {
    world
        .network()
        .up(&ctx_of(dir))
        .await
        .expect("the network is up")
}

fn way_of(link: &crate::network::Link) -> Way {
    link.way.clone().expect("the link has a way")
}

// T-099: WARP is a way out for TCP; a name is resolved inside the tunnel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warp_network_carries_tcp_and_a_name_inside_the_tunnel() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let dir = tempfile::tempdir().unwrap();
    let link = up(&world, dir.path()).await;
    let way = way_of(&link);
    assert_eq!(way.label(), "WARP");
    assert!(way.dial.names_inside());
    assert_eq!(
        echo(&*way.dial, b"through the tunnel").await,
        b"through the tunnel"
    );
    // The far end of the stand-in answers each name with 198.51.100.7.
    let named = Target::new("echo.test", ECHO_TCP);
    let connected = way
        .dial
        .dial(&named, tokio::time::Instant::now() + LIMIT)
        .await
        .expect("a name resolved inside the tunnel");
    drop(connected);
    link.close().await;
}

// T-099: WARP is a datagram carrier: a UDP datagram goes out through the
// session and its answer comes back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warp_network_carries_udp_datagrams() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let dir = tempfile::tempdir().unwrap();
    let link = up(&world, dir.path()).await;
    let datagrams: Arc<dyn Datagrams> = way_of(&link).dial.datagrams().await.expect("UDP");
    let to: SocketAddr = format!("203.0.113.9:{}", edge::ECHO_PORT).parse().unwrap();
    crate::network::send_to(&*datagrams, b"datagram", to)
        .await
        .unwrap();
    let mut buf = [0u8; 64];
    let (n, from) = tokio::time::timeout(LIMIT, crate::network::recv_from(&*datagrams, &mut buf))
        .await
        .expect("an answer")
        .unwrap();
    assert_eq!(&buf[..n], b"datagram");
    assert_eq!(from, to);
    link.close().await;
}

// T-180 (the Prove of the real device): a reconnect and two rotates give the
// same state directory, and the service sees one device.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warp_network_rotate_reuses_the_stored_device() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx_of(dir.path());
    let network = world.network();
    let first = network.up(&ctx).await.unwrap();
    assert_eq!(world.api.devices(), 1);
    let posts = world.api.asked("POST");
    let keys_before = world.api.enrolled();
    let identity = dir.path().join("warp.json");
    let stored = std::fs::read_to_string(&identity).unwrap();
    // A reconnect reads the file: no new device, no new key.
    let again = network.up(&ctx).await.unwrap();
    assert_eq!(world.api.devices(), 1);
    assert_eq!(world.api.enrolled(), keys_before);
    // Two rotates: a new key each time for the same device, and a link that
    // carries data.
    let second = network.rotate(&ctx).await.unwrap();
    assert_eq!(
        echo(&*way_of(&second).dial, b"after one rotate").await,
        b"after one rotate"
    );
    let third = network.rotate(&ctx).await.unwrap();
    assert_eq!(
        echo(&*way_of(&third).dial, b"after two").await,
        b"after two"
    );
    assert_eq!(world.api.devices(), 1, "a rotate registers no new device");
    assert_eq!(
        world.api.asked("POST"),
        posts,
        "no registration after the first"
    );
    assert_ne!(
        world.api.enrolled(),
        keys_before,
        "the key of the device is new"
    );
    assert_ne!(std::fs::read_to_string(&identity).unwrap(), stored);
    // Make before break: the first link still carries data.
    assert_eq!(
        echo(&*way_of(&first).dial, b"the old link").await,
        b"the old link"
    );
    for link in [first, again, second, third] {
        link.close().await;
    }
}

// T-097 items 3 and 10: a session that the edge ends comes back (`Live`),
// and a dial waits for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warp_network_session_comes_back_after_the_edge_ends_it() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let dir = tempfile::tempdir().unwrap();
    let link = up(&world, dir.path()).await;
    let way = way_of(&link);
    assert_eq!(echo(&*way.dial, b"one").await, b"one");
    world.edge.end_sessions();
    // A dial after the end waits for the new session.
    let mut again = Vec::new();
    for _ in 0..50 {
        let tried = tokio::time::timeout(
            Duration::from_secs(5),
            way.dial.dial(
                &target(),
                tokio::time::Instant::now() + Duration::from_secs(5),
            ),
        )
        .await;
        if matches!(tried, Ok(Ok(_))) {
            again = echo(&*way.dial, b"two").await;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(again, b"two");
    assert_eq!(world.edge.tunnels(), 2);
    assert_eq!(world.api.devices(), 1);
    link.close().await;
}

// T-097 item 9: the exit trace says WARP is on, or the member is on the
// wrong egress.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warp_network_egress_check_reads_the_trace() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let dir = tempfile::tempdir().unwrap();
    let link = up(&world, dir.path()).await;
    let check = world.network().egress_check().expect("a check");
    let way = way_of(&link);
    let result = check(way.clone(), Duration::from_secs(10)).await;
    assert!(matches!(result, ProbeResult::Ok(_)), "{result:?}");
    *world.trace.lock().unwrap() = TRACE_OFF;
    assert_eq!(
        check(way.clone(), Duration::from_secs(10)).await,
        ProbeResult::WrongEgress
    );
    // A trace that does not answer in the time is a miss that is not clean.
    let late = check(way, Duration::from_millis(1)).await;
    assert!(
        matches!(late, ProbeResult::ConnectFailed { clean: false }),
        "{late:?}"
    );
    link.close().await;
}

// T-097 item 10: a stored key that the edge refuses is enrolled again for
// the same device, once. No new device.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warp_network_refused_key_is_enrolled_again_for_the_same_device() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let dir = tempfile::tempdir().unwrap();
    // A device whose stored key the service never enrolled.
    let device = Warp::register(world.source.settings()).await.unwrap();
    let loose = Warp::from_parts(
        world.source.settings(),
        device.device().clone(),
        pod_warp::Identity::generate().unwrap(),
    );
    loose.save(&dir.path().join("warp.json")).unwrap();
    assert_eq!(world.api.devices(), 1);
    let link = up(&world, dir.path()).await;
    assert_eq!(
        echo(&*way_of(&link).dial, b"enrolled again").await,
        b"enrolled again"
    );
    assert_eq!(world.api.devices(), 1, "the same device");
    link.close().await;
}

// A way below WARP is refused, not ignored.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warp_network_below_is_refused() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let below: Arc<dyn Dial> = Arc::new(Moved(Vec::new()));
    let ctx = Ctx {
        below: Some(below),
        state_dir: None,
    };
    for result in [
        world.network().up(&ctx).await,
        world.network().rotate(&ctx).await,
    ] {
        let err = result.expect_err("a way below is refused");
        let failure = err.downcast_ref::<Failure>().expect("a failure");
        assert_eq!(failure.class, Class::Unsupported);
    }
}

/// T-061: the one front door runs over a WARP dialer: a SOCKS5 CONNECT
/// reaches the echo route inside the tunnel, and a UDP ASSOCIATE carries a
/// datagram to the UDP echo of the far end and brings the answer back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn front_door_over_the_warp_network() {
    pod_testkit::need_loopback!();
    let world = world().await;
    let dir = tempfile::tempdir().unwrap();
    let link = up(&world, dir.path()).await;
    let door = Arc::new(podnet_netns::door::Door::new(way_of(&link).dial));
    // CONNECT 203.0.113.9:80.
    let (mut client, stream) = tokio::io::duplex(4096);
    door.clone().spawn(stream);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    assert_eq!(method, [5, 0]);
    client
        .write_all(&[5, 1, 0, 1, 203, 0, 113, 9, 0, 80])
        .await
        .unwrap();
    let mut reply = [0u8; 10];
    tokio::time::timeout(LIMIT, client.read_exact(&mut reply))
        .await
        .expect("a reply")
        .unwrap();
    assert_eq!(reply[..2], [5, 0], "{reply:?}");
    client.write_all(b"socks through warp").await.unwrap();
    let mut got = [0u8; 18];
    tokio::time::timeout(LIMIT, client.read_exact(&mut got))
        .await
        .expect("an echo")
        .unwrap();
    assert_eq!(&got, b"socks through warp");
    // UDP ASSOCIATE; a datagram to 203.0.113.9:7.
    let (mut control, stream) = tokio::io::duplex(4096);
    door.clone().spawn(stream);
    control.write_all(&[5, 1, 0]).await.unwrap();
    control.read_exact(&mut method).await.unwrap();
    control
        .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    tokio::time::timeout(LIMIT, control.read_exact(&mut reply))
        .await
        .expect("a reply")
        .unwrap();
    assert_eq!(reply[..4], [5, 0, 0, 1], "{reply:?}");
    let bound = SocketAddr::from((
        [reply[4], reply[5], reply[6], reply[7]],
        u16::from_be_bytes([reply[8], reply[9]]),
    ));
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut wrapped = vec![0, 0, 0, 1, 203, 0, 113, 9, 0, edge::ECHO_PORT as u8];
    wrapped.extend_from_slice(b"udp through warp");
    udp.send_to(&wrapped, bound).await.unwrap();
    let mut buf = [0u8; 256];
    let (n, _) = tokio::time::timeout(LIMIT, udp.recv_from(&mut buf))
        .await
        .expect("an answer")
        .unwrap();
    assert_eq!(&buf[10..n], b"udp through warp");
    drop(control);
    link.close().await;
}
