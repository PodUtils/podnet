//! T-128: the DERP road of podnet (`podnet::derp`, `podnet derp`) through
//! the stand-in DERP server in memory of `pod-derp` (the feature
//! `standin`, in the dev-dependencies only).
//!
//! * `derp_each_session_ends_with_its_code`: a node serves a TCP echo
//!   through the stand-in, and a client of its ticket carries bytes through
//!   the resumable layer and the end-to-end channel. Each fault of the
//!   family (a client that the node does not admit, a node that does not
//!   prove the key of the ticket, a target that refuses, a ticket of no
//!   node) ends the client with its documented exit code. A cut of the
//!   connection of the node in the middle of the bytes resumes the session.
//! * `derp_the_command_line_refuses_each_bad_value`: the binary exits 64 for
//!   each bad value before the network, and 69 when no server answers.

use std::process::Command;
use std::time::Duration;

use pod_derp::standin::StandIn;
use pod_derp::{Key, Server};
use pod_identity::Identity;
use podnet::derp::{self, Keys, Road};
use podnet::exit;
use podnet::relay_reverse::Target;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const LIMIT: Duration = Duration::from_secs(60);

/// A TCP echo on the loopback: each connection gets its bytes back, then
/// the end.
async fn echo() -> Target {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = socket.into_split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
                let _ = write.shutdown().await;
            });
        }
    });
    Target::Tcp {
        host: "127.0.0.1".into(),
        port,
    }
}

/// A loopback port where nothing listens.
async fn closed() -> Target {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    Target::Tcp {
        host: "127.0.0.1".into(),
        port,
    }
}

/// One case: what the client does wrong, and the code that it must give.
struct Case {
    name: &'static str,
    /// The node admits the key of the client.
    admitted: bool,
    /// The ticket names the key of the node.
    true_node: bool,
    /// The ticket names the key of the node on the server.
    true_derp: bool,
    /// The target answers.
    target_up: bool,
    /// The stand-in cuts the connection of the node in the middle of the
    /// bytes: the session resumes on a new link.
    cut: bool,
    /// `None`: the bytes come back.
    code: Option<i32>,
}

async fn run(case: &Case) {
    let stand_in = StandIn::new(0);
    let road = Road {
        dial: stand_in.dial(),
        servers: vec![StandIn::server()],
        events: derp::log_events(),
    };
    let node_key = Identity::from_seed(&[7; 32]);
    let client_key = Identity::from_seed(&[9; 32]);
    let stranger = Identity::from_seed(&[11; 32]).public();
    let admit = if case.admitted {
        client_key.public()
    } else {
        stranger
    };
    let keys = Keys::new(&node_key, move |k| *k == admit);
    let target = if case.target_up {
        echo().await
    } else {
        closed().await
    };
    let probe = |_: &Server| async { Ok::<(), String>(()) };
    let online = derp::online(&node_key, &road, probe)
        .await
        .expect("online at the stand-in");
    let mut ticket = online.ticket.clone();
    assert_eq!(ticket.server, StandIn::server(), "{}", case.name);
    if !case.true_node {
        ticket.node = stranger;
    }
    if !case.true_derp {
        ticket.derp = Key([0x42; 32]);
    }
    let cut = case.cut.then(|| (stand_in.clone(), online.ticket.derp.0));
    let serving = tokio::spawn(derp::serve(online, road.clone(), target, keys, |_| {}));
    let (mine, app) = tokio::io::duplex(64 * 1024);
    let sent: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let user = {
        let sent = sent.clone();
        tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(mine);
            let writing = async {
                let half = sent.len() / 2;
                let _ = write.write_all(&sent[..half]).await;
                if let Some((stand_in, node)) = &cut {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    stand_in.cut(node);
                }
                let _ = write.write_all(&sent[half..]).await;
                let _ = write.shutdown().await;
            };
            let mut got = Vec::new();
            let reading = read.read_to_end(&mut got);
            let (_, _) = tokio::join!(writing, reading);
            got
        })
    };
    let ended = tokio::time::timeout(LIMIT, derp::connect(&ticket, &road, app, &client_key))
        .await
        .unwrap_or_else(|_| panic!("{}: the session ended in time", case.name));
    match case.code {
        None => {
            ended.unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
            let got = user.await.unwrap();
            assert_eq!(got.len(), sent.len(), "{}", case.name);
            assert!(got == sent, "{}: the bytes came back changed", case.name);
            // The node logged in again after the cut.
            let logins = stand_in.logins.load(std::sync::atomic::Ordering::SeqCst);
            assert!(!case.cut || logins >= 3, "{}: {logins} logins", case.name);
        }
        Some(code) => {
            let e = ended
                .err()
                .unwrap_or_else(|| panic!("{}: no failure", case.name));
            assert_eq!(exit::code_of(&e), code, "{}: {e:#}", case.name);
            let line = exit::text(&e);
            assert!(!line.contains('\n'), "{}: one line: {line}", case.name);
            user.abort();
        }
    }
    serving.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn derp_each_session_ends_with_its_code() {
    let cases = [
        Case {
            name: "an admitted client gets its bytes back",
            admitted: true,
            true_node: true,
            true_derp: true,
            target_up: true,
            cut: false,
            code: None,
        },
        Case {
            name: "a cut of the connection of the node: the session resumes",
            admitted: true,
            true_node: true,
            true_derp: true,
            target_up: true,
            cut: true,
            code: None,
        },
        Case {
            name: "a client that the node does not admit",
            admitted: false,
            true_node: true,
            true_derp: true,
            target_up: true,
            cut: false,
            code: Some(exit::NO_PERM),
        },
        Case {
            name: "a node that does not prove the key of the ticket",
            admitted: true,
            true_node: false,
            true_derp: true,
            target_up: true,
            cut: false,
            code: Some(exit::NO_PERM),
        },
        Case {
            name: "a target that refuses",
            admitted: true,
            true_node: true,
            true_derp: true,
            target_up: false,
            cut: false,
            code: Some(exit::UNAVAILABLE),
        },
        Case {
            name: "a ticket of no node at the server",
            admitted: true,
            true_node: true,
            true_derp: false,
            target_up: true,
            cut: false,
            code: Some(exit::UNAVAILABLE),
        },
    ];
    for case in &cases {
        run(case).await;
    }
}

#[test]
fn derp_the_command_line_refuses_each_bad_value() {
    let dir = tempfile::tempdir().unwrap();
    // The ticket of a node of the stand-in, as a client gets it.
    let ticket = pod_derp::Ticket {
        node: Identity::from_seed(&[7; 32]).public(),
        derp: Key([0x42; 32]),
        server: StandIn::server(),
    }
    .to_string();
    let peer = Identity::from_seed(&[9; 32]).public().display_fingerprint();
    let to = ["--to", "127.0.0.1:22"];
    let cases: [(&str, Vec<&str>, i32); 7] = [
        ("no ticket", vec!["connect", "nope"], exit::USAGE),
        (
            "a ticket of another road",
            vec!["connect", "iroh:abc"],
            exit::USAGE,
        ),
        (
            "a bad server",
            vec!["connect", &ticket, "--server", "a b"],
            exit::USAGE,
        ),
        (
            "a bad main fleet",
            vec!["connect", &ticket, "--main-fleet", "maybe"],
            exit::USAGE,
        ),
        (
            "no --peer and no --allow",
            vec!["node", to[0], to[1]],
            exit::USAGE,
        ),
        (
            "a bad target",
            vec!["node", "--to", "nowhere", "--peer", &peer],
            exit::USAGE,
        ),
        (
            "no server answers",
            vec![
                "node",
                to[0],
                to[1],
                "--peer",
                &peer,
                "--server",
                "127.0.0.1:1",
            ],
            exit::UNAVAILABLE,
        ),
    ];
    for (name, args, code) in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .arg("derp")
            .args(&args)
            .env("PODNET_STATE_DIR", dir.path())
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env("NO_PROXY", "*")
            .output()
            .expect("podnet runs");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(code), "{name}: {stderr}");
        assert!(!stderr.trim().is_empty(), "{name}: no line on stderr");
    }
}
