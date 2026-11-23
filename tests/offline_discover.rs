//! The signed mDNS announcement of PodUtils/podnet#499: `discover announce`
//! sends the record of a key to one address, and `discover listen` reports
//! the peers whose signature verified and counts the records that did not.
//!
//! The listener is a real `podnet offline discover listen` on the loopback,
//! and the packets of the test are a valid announcement and the same
//! announcement with its port changed after the signature.

use std::net::UdpSocket;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use podnet::offline::discovery::Announcement;
use podnet::offline::mdns;

fn podnet(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("RUST_LOG");
    cmd
}

fn key(dir: &Path) -> podnet::identity::Identity {
    std::fs::create_dir_all(dir).expect("a state directory");
    podnet::identity::create(dir).expect("a key")
}

/// A UDP port of the loopback that nothing holds now.
fn free_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("a socket");
    socket.local_addr().expect("an address").port()
}

#[test]
fn an_announcement_goes_to_one_address_as_one_txt_record() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let me = key(&dir.path().join("key"));
    let receiver = UdpSocket::bind("127.0.0.1:0").expect("a socket");
    receiver
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let port = receiver.local_addr().expect("an address").port();
    let to = format!("127.0.0.1:{port}");

    let out = podnet(
        dir.path(),
        &[
            "offline",
            "discover",
            "announce",
            "--name",
            "node-a",
            "--address",
            "127.0.0.1",
            "--port",
            "4242",
            "--key",
            dir.path().join("key").to_str().expect("a path"),
            "--to",
            &to,
            "--repeat",
            "2",
        ],
    )
    .output()
    .expect("discover announce");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("node-a 127.0.0.1:4242"), "{said:?}");
    assert!(said.contains(&me.public().hex()), "{said:?}");

    let mut packet = vec![0u8; 9000];
    let mut announcements = Vec::new();
    while announcements.len() < 2 {
        let (count, _) = receiver.recv_from(&mut packet).expect("a packet");
        let records = mdns::txt_records(&packet[..count]);
        assert_eq!(records.len(), 1, "one TXT record in one answer");
        let announcement = Announcement::from_txt(&records[0]).expect("an announcement");
        assert!(announcement.verify(), "the signature of the record");
        announcements.push(announcement);
    }
    assert_eq!(announcements[0], announcements[1], "the repeat differs");
    assert_eq!(announcements[0].name, "node-a");
    assert_eq!(announcements[0].address, "127.0.0.1");
    assert_eq!(announcements[0].port, 4242);
    assert_eq!(announcements[0].key, me.public());
}

#[test]
fn a_listener_takes_the_signed_record_and_ignores_a_changed_one() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let me = key(&dir.path().join("key"));
    let good =
        Announcement::signed(me.key(), "node-a", "127.0.0.1", 4242).expect("an announcement");
    let good_packet = mdns::answer(
        &mdns::service_name("node-a"),
        &good.txt().expect("a record"),
    )
    .expect("a packet");

    // The port changes after the signature, so the message of the key is
    // not the message of the record.
    let mut bad = good.clone();
    bad.port = 9;
    let bad_packet = mdns::answer(&mdns::service_name("node-a"), &bad.txt().expect("a record"))
        .expect("a packet");
    assert!(!bad.verify(), "the changed record still verifies");

    let port = free_port();
    let bind = format!("127.0.0.1:{port}");
    let mut child = podnet(
        dir.path(),
        &[
            "offline",
            "discover",
            "listen",
            "--bind",
            &bind,
            "--timeout",
            "1",
            "--json",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .expect("discover listen");

    let sender = UdpSocket::bind("127.0.0.1:0").expect("a socket");
    let to: std::net::SocketAddr = bind.parse().expect("an address");
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        sender.send_to(&good_packet, to).expect("the good packet");
        sender.send_to(&bad_packet, to).expect("the bad packet");
        std::thread::sleep(Duration::from_millis(100));
        if child.try_wait().expect("a check").is_some() {
            break;
        }
    }
    let out = child.wait_with_output().expect("the listener");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "the listener printed no JSON ({e}): {:?}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    let peers = doc["peers"].as_array().expect("the peers");
    assert_eq!(peers.len(), 1, "{doc}");
    assert_eq!(peers[0]["name"], "node-a");
    assert_eq!(peers[0]["address"], "127.0.0.1");
    assert_eq!(peers[0]["port"].as_u64(), Some(4242));
    assert_eq!(peers[0]["key"], me.public().hex());
    assert!(
        doc["ignored"].as_u64().expect("a count") >= 1,
        "no changed record was ignored: {doc}"
    );
    assert!(
        doc["records"].as_u64().expect("a count") >= 2,
        "the listener heard too few records: {doc}"
    );
}
