//! A DERP server as a way of the transit (T-128): two ends of podnet meet
//! at a stand-in mailbox, and the sealed records go through the stand-in
//! DERP server in memory of `pod-derp` (the feature `standin`), to the
//! node that the sender runs for the time of the transfer. One table holds
//! the rules of the way: it carries a file when no other way exists, it
//! needs a receiver that takes it, and a direct hint wins before it.

#[path = "transfer_harness/mod.rs"]
mod harness;

mod test_host;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use harness::Mailbox;
use pod_derp::standin::StandIn;
use pod_wormhole::Code;
use pod_ws::url::WsUrl;
use podnet::derp::{self, Road};
use podnet::exit;
use podnet::transfer::land::Caps;
use podnet::transfer::{self, Listen, Receive, Received, Settings, What};
use sha2::{Digest, Sha256};

fn way() -> pod_ws::Way {
    podnet::net::init_crypto();
    let settings = pod_proxy::Settings::new("podnet-test", pod_proxy::ProxyChoice::Direct);
    let dial = pod_proxy::Dialer::new(settings, Arc::new(pod_proxy::resolve::System::new()));
    pod_ws::Way::new(
        podnet::trust::context().clone(),
        Arc::new(dial),
        "podnet/test",
    )
}

fn settings(mailbox: &str, listen: Listen, derp: Option<Road>) -> Settings {
    let mut settings = Settings::new(vec![WsUrl::parse(mailbox).unwrap()], Vec::new(), way());
    settings.listen = listen;
    settings.derp = derp;
    settings.wait = Duration::from_secs(30);
    settings.step = Duration::from_secs(5);
    settings.transit_wait = Duration::from_secs(30);
    settings
}

fn sha256(path: &Path) -> String {
    pod_wormhole::hex(&Sha256::digest(std::fs::read(path).unwrap()))
}

/// One case: how the receiver can be reached, whether it takes the DERP
/// way, and how the transfer goes.
struct Case {
    name: &'static str,
    /// The receiver listens on the loopback.
    direct: bool,
    /// The receiver has a DERP road.
    takes_derp: bool,
    /// The start of the `via` of the transfer at each end, or `None`: both ends
    /// exit 69.
    via: Option<&'static str>,
    /// The logins at the stand-in: the node, and each client.
    logins: usize,
}

async fn run(case: &Case) {
    let mailbox = Mailbox::start().await;
    let stand_in = StandIn::new(0);
    let road = Road {
        dial: stand_in.dial(),
        servers: vec![StandIn::server()],
        events: derp::log_events(),
        probed: false,
    };
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("file.bin");
    let data: Vec<u8> = (0..300_001u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    std::fs::write(&file, &data).unwrap();
    let mut sender = settings(&mailbox.url, Listen::No, Some(road.clone()));
    let listen = if case.direct {
        Listen::At(SocketAddr::from(([127, 0, 0, 1], 0)))
    } else {
        Listen::No
    };
    let mut receiver = settings(&mailbox.url, listen, case.takes_derp.then_some(road));
    if case.via.is_none() {
        sender.transit_wait = Duration::from_secs(3);
        receiver.transit_wait = Duration::from_secs(3);
    }
    let opened = transfer::open(&sender).await.expect("a code");
    let code = Code::parse(opened.code()).expect("a code that parses");
    let what = What::Path(file.clone());
    let sending = tokio::spawn(async move { opened.send(&sender, &what).await });
    let how = Receive {
        out: out.path().to_path_buf(),
        caps: Caps::default(),
    };
    let mut ask = |_: &transfer::Offered| Ok(true);
    let received = transfer::receive(&receiver, &code, &how, &mut ask).await;
    let sent = sending.await.unwrap();
    match case.via {
        Some(want) => {
            let sent = sent.unwrap_or_else(|e| panic!("{}: the sender: {e:#}", case.name));
            let Ok(Received::Path { path, via, .. }) = received else {
                panic!("{}: the receiver: {:?}", case.name, received.err());
            };
            assert!(sent.via.starts_with(want), "{}: {}", case.name, sent.via);
            assert!(via.starts_with(want), "{}: {via}", case.name);
            assert_eq!(sha256(&path), sha256(&file), "{}", case.name);
        }
        None => {
            let (sent, received) = (sent.unwrap_err(), received.unwrap_err());
            assert_eq!(
                exit::code_of(&sent),
                exit::UNAVAILABLE,
                "{}: {sent:#}",
                case.name
            );
            assert_eq!(
                exit::code_of(&received),
                exit::UNAVAILABLE,
                "{}: {received:#}",
                case.name
            );
            assert_eq!(
                std::fs::read_dir(out.path()).unwrap().count(),
                0,
                "{}",
                case.name
            );
        }
    }
    let logins = stand_in.logins.load(Ordering::SeqCst);
    assert_eq!(
        logins, case.logins,
        "{}: the logins at the stand-in",
        case.name
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn derp_a_transfer_takes_the_derp_way_by_its_rules() {
    test_host::need_loopback!();
    let cases = [
        Case {
            name: "no other way: the file goes through the DERP server",
            direct: false,
            takes_derp: true,
            via: Some("DERP derp.test"),
            logins: 2,
        },
        Case {
            name: "a receiver with no DERP road: no way",
            direct: false,
            takes_derp: false,
            via: None,
            logins: 1,
        },
        Case {
            name: "a direct hint wins before the DERP way",
            direct: true,
            takes_derp: true,
            via: Some("direct "),
            logins: 1,
        },
    ];
    for case in &cases {
        run(case).await;
    }
}
