use std::sync::Mutex;

use podnet_core::network::{BoxFuture, Class, Connected, Failure, Stage};

use super::*;

/// A dialer that answers each flow with the target in text, or refuses
/// the name `refused.example`.
struct Named {
    seen: Mutex<Vec<String>>,
}

impl Dial for Named {
    fn label(&self) -> String {
        "named".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(target.to_string());
            if target.host() == "refused.example" {
                return Err(Failure::new(
                    Stage::Connect,
                    Class::Refused,
                    "connection refused",
                ));
            }
            let (near, mut far) = tokio::io::duplex(1024);
            let text = target.to_string();
            tokio::spawn(async move {
                let _ = far.write_all(text.as_bytes()).await;
                let _ = far.shutdown().await;
            });
            Ok(Connected::new(Box::new(near)))
        })
    }
}

fn door() -> (Arc<Door>, Arc<Named>) {
    let named = Arc::new(Named {
        seen: Mutex::new(Vec::new()),
    });
    (Arc::new(Door::new(named.clone())), named)
}

async fn socks5(door: &Arc<Door>, host: &str, port: u16) -> Vec<u8> {
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut reply = [0u8; 2];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [5, 0]);
    let mut req = vec![5, 1, 0, 3, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    client.write_all(&req).await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    out
}

#[tokio::test]
async fn front_door_carries_socks5_over_its_dialer() {
    let (door, named) = door();
    let out = socks5(&door, "svc.example", 443).await;
    assert_eq!(&out[..2], &[5, 0], "{out:?}");
    assert!(out.ends_with(b"svc.example:443"), "{out:?}");
    assert_eq!(
        *named.seen.lock().unwrap(),
        ["svc.example:443"],
        "the name goes to the dialer"
    );
    let out = socks5(&door, "refused.example", 22).await;
    assert_ne!(out[1], 0, "a refused dial is a SOCKS5 failure: {out:?}");
}

#[tokio::test]
async fn front_door_carries_http_connect_and_refuses_a_bad_user() {
    let (door, _) = door();
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client
        .write_all(b"CONNECT svc.example:8443 HTTP/1.1\r\nHost: svc.example:8443\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    client.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(out.ends_with("svc.example:8443"), "{out}");

    let named = Arc::new(Named {
        seen: Mutex::new(Vec::new()),
    });
    let users: Users = Arc::new(|u: &str, p: &str| u == "ann" && p == "pw");
    let guarded = Arc::new(Door::new(named.clone()).with_users(users));
    let (mut client, server) = tokio::io::duplex(4096);
    guarded.clone().spawn(server);
    client
        .write_all(b"CONNECT svc.example:443 HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    client.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 407"), "{out}");
    assert!(
        named.seen.lock().unwrap().is_empty(),
        "no dial for a refused user"
    );
    let (mut client, server) = tokio::io::duplex(4096);
    guarded.clone().spawn(server);
    let auth = base64::engine::general_purpose::STANDARD.encode("ann:pw");
    let head =
        format!("CONNECT svc.example:443 HTTP/1.1\r\nProxy-Authorization: Basic {auth}\r\n\r\n");
    client.write_all(head.as_bytes()).await.unwrap();
    let mut out = String::new();
    client.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
}

#[tokio::test(start_paused = true)]
async fn front_door_ends_a_client_with_no_request_at_its_deadline() {
    let named = Arc::new(Named {
        seen: Mutex::new(Vec::new()),
    });
    let door = Door::new(named).with_request_limit(Duration::from_secs(30));
    let (_client, server) = tokio::io::duplex(64);
    let start = Instant::now();
    let err = door.handle(server).await.expect_err("no request");
    assert!(err.to_string().contains("no request in 30s"), "{err}");
    assert_eq!(start.elapsed(), Duration::from_secs(30));
}

#[tokio::test]
async fn front_door_turns_a_client_away_over_its_cap() {
    let named = Arc::new(Named {
        seen: Mutex::new(Vec::new()),
    });
    let door = Arc::new(Door::new(named).with_client_limit(1));
    let (_hold, first) = tokio::io::duplex(64);
    door.clone().spawn(first);
    let (mut second_client, second) = tokio::io::duplex(64);
    door.clone().spawn(second);
    assert_eq!(door.turned_away(), 1);
    let mut out = Vec::new();
    second_client.read_to_end(&mut out).await.unwrap();
    assert!(out.is_empty(), "the second client is closed at once");
}

#[tokio::test]
async fn front_door_refuses_udp_associate_with_command_unsupported() {
    let (door, _) = door();
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut reply = [0u8; 2];
    client.read_exact(&mut reply).await.unwrap();
    client
        .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    assert_eq!(out[1], 0x07, "{out:?}");
}

/// T-178: a door over a group counts the bytes of each flow for the member
/// that carried it, in both directions, and the status shows them.
#[tokio::test]
async fn front_door_gives_the_bytes_of_a_flow_to_its_member() {
    use podnet_core::network::{Balance, Member, Way};
    let named = Arc::new(Named {
        seen: Mutex::new(Vec::new()),
    });
    let group = Arc::new(
        Balance::new(
            "g",
            vec![Member::new("only", Way::destination(named.clone()), 1)],
        )
        .unwrap(),
    );
    let door = Arc::new(Door::new(group.clone()));
    let out = socks5(&door, "svc.example", 443).await;
    assert!(out.ends_with(b"svc.example:443"), "{out:?}");
    let member = &group.members()[0];
    assert_eq!(member.bytes(), "svc.example:443".len() as u64);
    assert_eq!(member.opens(), 0, "the first byte ended the open dial");
    assert_eq!(
        group.status()["members"][0]["bytes"],
        "svc.example:443".len()
    );
}
