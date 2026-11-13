//! The transfer of magic-wormhole on the loopback (T-102, T-103, T-251):
//! two ends of podnet meet at a stand-in mailbox through `pod-wormhole`,
//! then move a text in the wormhole, or a file or a directory over the
//! transit, direct or through a stand-in transit relay. The SHA-256 of each
//! byte that lands is the sender's.

#[path = "transfer_harness/mod.rs"]
mod harness;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use harness::{nobody_there, Mailbox, Relay};
use pod_wormhole::Code;
use pod_ws::url::WsUrl;
use podnet::exit;
use podnet::transfer::land::Caps;
use podnet::transfer::{self, Listen, Receive, Received, Settings, What};
use sha2::{Digest, Sha256};

/// The way of the tests: a direct dial, with no proxy of the environment.
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

fn loopback() -> Listen {
    Listen::At(SocketAddr::from(([127, 0, 0, 1], 0)))
}

/// The settings of one end at `mailboxes`, with `relays`.
fn settings(mailboxes: &[&str], relays: &[&str], listen: Listen) -> Settings {
    let mailboxes = mailboxes.iter().map(|u| WsUrl::parse(u).unwrap()).collect();
    let relays = relays
        .iter()
        .map(|r| transfer::Relay::parse(r).unwrap())
        .collect();
    let mut settings = Settings::new(mailboxes, relays, way());
    settings.listen = listen;
    settings.wait = Duration::from_secs(30);
    settings.step = Duration::from_secs(5);
    settings.transit_wait = Duration::from_secs(20);
    settings
}

fn yes() -> impl FnMut(&transfer::Offered) -> anyhow::Result<bool> + Send {
    |_| Ok(true)
}

/// Send `what` from `sender` and receive it with `receiver` into `out`.
async fn run(
    sender: Settings,
    receiver: Settings,
    what: What,
    out: &Path,
) -> (anyhow::Result<transfer::Sent>, anyhow::Result<Received>) {
    let opened = transfer::open(&sender).await.expect("a code");
    let code = Code::parse(opened.code()).expect("a code that parses");
    let sending = tokio::spawn(async move { opened.send(&sender, &what).await });
    let how = Receive {
        out: out.to_path_buf(),
        caps: Caps::default(),
    };
    let mut ask = yes();
    let received = transfer::receive(&receiver, &code, &how, &mut ask).await;
    (sending.await.unwrap(), received)
}

fn sha256(path: &Path) -> String {
    pod_wormhole::hex(&Sha256::digest(std::fs::read(path).unwrap()))
}

#[tokio::test]
async fn wormhole_rendezvous_two_ends_move_a_text() {
    let mailbox = Mailbox::start().await;
    let out = tempfile::tempdir().unwrap();
    let (sender, receiver) = (
        settings(&[&mailbox.url], &[], loopback()),
        settings(&[&mailbox.url], &[], loopback()),
    );
    let (sent, received) = run(sender, receiver, What::Text("the words".into()), out.path()).await;
    assert_eq!(sent.unwrap().via, "mailbox");
    assert_eq!(received.unwrap(), Received::Text("the words".into()));
    assert!(
        mailbox.moods().iter().all(|m| m == "happy"),
        "{:?}",
        mailbox.moods()
    );
}

#[tokio::test]
async fn wormhole_rendezvous_a_wrong_code_fails_at_both_ends() {
    let mailbox = Mailbox::start().await;
    let out = tempfile::tempdir().unwrap();
    let sender = settings(&[&mailbox.url], &[], loopback());
    let receiver = settings(&[&mailbox.url], &[], loopback());
    let opened = transfer::open(&sender).await.expect("a code");
    let shown = opened.code().to_string();
    let (head, last) = shown.rsplit_once('-').unwrap();
    let other = if last == pod_wormhole::words::ODD[0] {
        pod_wormhole::words::ODD[1]
    } else {
        pod_wormhole::words::ODD[0]
    };
    let wrong = Code::parse(&format!("{head}-{other}")).unwrap();
    let sending = tokio::spawn(async move { opened.send(&sender, &What::Text("x".into())).await });
    let how = Receive {
        out: out.path().to_path_buf(),
        caps: Caps::default(),
    };
    let mut ask = yes();
    let received = transfer::receive(&receiver, &wrong, &how, &mut ask)
        .await
        .unwrap_err();
    let sent = sending.await.unwrap().unwrap_err();
    assert_eq!(exit::code_of(&received), exit::NO_PERM, "{received:#}");
    assert_eq!(exit::code_of(&sent), exit::NO_PERM, "{sent:#}");
    assert!(
        format!("{sent:#}").contains("the code is spent"),
        "{sent:#}"
    );
    assert!(
        mailbox.moods().iter().any(|m| m == "scary"),
        "{:?}",
        mailbox.moods()
    );
}

#[tokio::test]
async fn wormhole_transfer_a_file_lands_with_the_sha256_of_the_sender() {
    let mailbox = Mailbox::start().await;
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("data.bin");
    let bytes: Vec<u8> = (0..700_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(&file, &bytes).unwrap();
    let (sender, receiver) = (
        settings(&[&mailbox.url], &[], loopback()),
        settings(&[&mailbox.url], &[], loopback()),
    );
    let (sent, received) = run(sender, receiver, What::Path(file.clone()), out.path()).await;
    let sent = sent.unwrap();
    let Received::Path {
        path,
        bytes: got,
        sha256: acked,
        files,
        via,
    } = received.unwrap()
    else {
        panic!("a path")
    };
    assert_eq!((got, files), (bytes.len() as u64, 1));
    assert!(via.starts_with("direct"), "{via}");
    assert_eq!(path, out.path().join("data.bin"));
    assert_eq!(sha256(&path), sha256(&file));
    assert_eq!(Some(acked), sent.sha256);
}

#[tokio::test]
async fn wormhole_transfer_a_directory_lands_with_the_sha256_of_each_file() {
    let mailbox = Mailbox::start().await;
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let top = src.path().join("tree");
    std::fs::create_dir_all(top.join("a/b")).unwrap();
    std::fs::create_dir_all(top.join("empty")).unwrap();
    std::fs::write(top.join("a/b/one.txt"), b"one\n").unwrap();
    std::fs::write(top.join("two.bin"), vec![3u8; 300_000]).unwrap();
    std::fs::write(top.join("a/zero"), b"").unwrap();
    let (sender, receiver) = (
        settings(&[&mailbox.url], &[], loopback()),
        settings(&[&mailbox.url], &[], loopback()),
    );
    let (sent, received) = run(sender, receiver, What::Path(top.clone()), out.path()).await;
    sent.unwrap();
    let Received::Path { path, files, .. } = received.unwrap() else {
        panic!("a path")
    };
    assert_eq!(path, out.path().join("tree"));
    assert_eq!(
        files, 6,
        "a/, a/b/, a/b/one.txt, a/zero, empty/ and two.bin"
    );
    for name in ["a/b/one.txt", "two.bin", "a/zero"] {
        assert_eq!(sha256(&path.join(name)), sha256(&top.join(name)), "{name}");
    }
    assert!(path.join("empty").is_dir());
}

#[tokio::test]
async fn wormhole_transfer_goes_through_the_relay_when_no_end_listens() {
    let (mailbox, relay) = (Mailbox::start().await, Relay::start().await);
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("f");
    std::fs::write(&file, vec![5u8; 123_457]).unwrap();
    let sender = settings(&[&mailbox.url], &[&relay.url], Listen::No);
    let receiver = settings(&[&mailbox.url], &[], Listen::No);
    let (sent, received) = run(sender, receiver, What::Path(file.clone()), out.path()).await;
    sent.unwrap();
    let Received::Path { via, path, .. } = received.unwrap() else {
        panic!("a path")
    };
    assert!(via.starts_with("relay tcp:127.0.0.1:"), "{via}");
    assert_eq!(relay.joined.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(sha256(&path), sha256(&file));
}

#[tokio::test]
async fn wormhole_transfer_with_no_way_fails_with_69() {
    let mailbox = Mailbox::start().await;
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    let mut sender = settings(&[&mailbox.url], &[], Listen::No);
    let mut receiver = settings(&[&mailbox.url], &[], Listen::No);
    sender.transit_wait = Duration::from_secs(2);
    receiver.transit_wait = Duration::from_secs(2);
    let (sent, received) = run(sender, receiver, What::Path(file), out.path()).await;
    assert_eq!(exit::code_of(&sent.unwrap_err()), exit::UNAVAILABLE);
    assert_eq!(exit::code_of(&received.unwrap_err()), exit::UNAVAILABLE);
    assert_eq!(
        std::fs::read_dir(out.path()).unwrap().count(),
        0,
        "nothing stays"
    );
}

#[tokio::test]
async fn transfer_mailbox_falls_back() {
    let mailbox = Mailbox::start().await;
    let out = tempfile::tempdir().unwrap();
    let dead = nobody_there();
    let sender = settings(&[&dead, &mailbox.url], &[], loopback());
    let receiver = settings(&[&dead, &mailbox.url], &[], loopback());
    let (sent, received) = run(sender, receiver, What::Text("fell back".into()), out.path()).await;
    sent.unwrap();
    assert_eq!(received.unwrap(), Received::Text("fell back".into()));
}

#[tokio::test]
async fn transfer_mailbox_choice_is_found_by_the_receiver() {
    // The sender meets at the second mailbox; the receiver tries the first,
    // where no giver waits, then finds the code at the second one.
    let (first, second) = (Mailbox::start().await, Mailbox::start().await);
    let out = tempfile::tempdir().unwrap();
    let sender = settings(&[&second.url], &[], loopback());
    let mut receiver = settings(&[&first.url, &second.url], &[], loopback());
    receiver.giver_wait = Duration::from_secs(2);
    let (sent, received) = run(
        sender,
        receiver,
        What::Text("at the second".into()),
        out.path(),
    )
    .await;
    sent.unwrap();
    assert_eq!(received.unwrap(), Received::Text("at the second".into()));
    assert_eq!(
        first.nameplates(),
        0,
        "the receiver released the nameplate of the first mailbox"
    );
}

#[tokio::test]
async fn wormhole_transfer_a_refused_offer_writes_nothing() {
    let mailbox = Mailbox::start().await;
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("f");
    std::fs::write(&file, b"new").unwrap();
    std::fs::write(out.path().join("f"), b"old").unwrap();
    let (sender, receiver) = (
        settings(&[&mailbox.url], &[], loopback()),
        settings(&[&mailbox.url], &[], loopback()),
    );
    let (sent, received) = run(sender, receiver, What::Path(file), out.path()).await;
    let received = received.unwrap_err();
    assert_eq!(exit::code_of(&received), exit::DATA_ERROR, "{received:#}");
    assert!(format!("{received:#}").contains("EEXIST"), "{received:#}");
    assert!(format!("{:#}", sent.unwrap_err()).contains("the receiver refused the transfer"));
    assert_eq!(std::fs::read(out.path().join("f")).unwrap(), b"old");
}
