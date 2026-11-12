use std::sync::Mutex;

use pod_core::{BoxFuture, Class, Connected, Failure, Stage};

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

/// A dialer that fails each dial with `class`.
struct Failing(Class);

impl Dial for Failing {
    fn label(&self) -> String {
        "failing".into()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        let class = self.0;
        Box::pin(async move { Err(Failure::new(Stage::Connect, class, "no way today")) })
    }
}

async fn ask(door: &Arc<Door>, request: &[u8]) -> Vec<u8> {
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client.write_all(request).await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    out
}

/// The reply follows the class of the failure: a policy refusal is "not
/// allowed" (SOCKS5 2, HTTP 403), a timeout is "host unreachable" (SOCKS5 4,
/// HTTP 504), any other failure is a general failure or 502 with the text
/// of the failure in the body.
#[tokio::test]
async fn front_door_replies_by_the_class_of_a_failure() {
    let socks = [5, 1, 0, 5, 1, 0, 3, 1, b'a', 0, 80];
    for (class, code) in [
        (Class::Policy, 0x02),
        (Class::Timeout, 0x04),
        (Class::Unavailable, 0x01),
    ] {
        let door = Arc::new(Door::new(Arc::new(Failing(class))));
        let out = ask(&door, &socks).await;
        assert_eq!(out[..2], [5, 0], "{class}");
        assert_eq!(out[3], code, "{class}: {out:?}");
    }
    for (class, status, reason) in [
        (Class::Policy, 403, "Forbidden"),
        (Class::Timeout, 504, "Gateway Timeout"),
        (Class::Unavailable, 502, "Bad Gateway"),
    ] {
        let door = Arc::new(Door::new(Arc::new(Failing(class))));
        let out = ask(&door, b"CONNECT a:80 HTTP/1.1\r\n\r\n").await;
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.starts_with(&format!("HTTP/1.1 {status} {reason}\r\n")),
            "{class}: {text}"
        );
        assert!(text.ends_with("no way today\n"), "{class}: {text}");
    }
}

/// A client that stops in its request gets an answer at the deadline:
/// SOCKS5 4 after its first byte, HTTP 504 before it.
#[tokio::test(start_paused = true)]
async fn front_door_answers_a_slow_client() {
    for (first, check) in [(Some(5u8), "socks"), (None, "http")] {
        let (door, _) = door();
        let (mut client, server) = tokio::io::duplex(64);
        if let Some(byte) = first {
            client.write_all(&[byte]).await.unwrap();
        }
        let task = tokio::spawn(async move { door.handle(server).await });
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        assert!(task.await.unwrap().is_err());
        if check == "socks" {
            assert_eq!(out[..2], [5, 0x04], "{out:?}");
        } else {
            let text = String::from_utf8_lossy(&out);
            assert!(text.starts_with("HTTP/1.1 504 Gateway Timeout"), "{text}");
        }
    }
}

/// `connect_only` refuses a plain `http://` request with 405 and a body
/// that names CONNECT; the dialer sees no flow.
#[tokio::test]
async fn front_door_connect_only_refuses_a_plain_http_request() {
    let named = Arc::new(Named {
        seen: Mutex::new(Vec::new()),
    });
    let door = Arc::new(Door::new(named.clone()).connect_only());
    let out = ask(
        &door,
        b"GET http://a.example/ HTTP/1.1\r\nHost: a.example\r\n\r\n",
    )
    .await;
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
        "{text}"
    );
    assert!(text.contains("HTTP CONNECT only"), "{text}");
    assert!(named.seen.lock().unwrap().is_empty());
}

/// A request with port 0 takes the default port of the door.
#[tokio::test]
async fn front_door_gives_port_0_the_default_port() {
    let named = Arc::new(Named {
        seen: Mutex::new(Vec::new()),
    });
    let door = Arc::new(Door::new(named.clone()).with_default_port(8080));
    let out = socks5(&door, "svc.example", 0).await;
    assert!(out.ends_with(b"svc.example:8080"), "{out:?}");
    let out = socks5(&door, "svc.example", 443).await;
    assert!(out.ends_with(b"svc.example:443"), "{out:?}");
}

/// A flow ends at the flow limit, also when both sides are silent.
#[tokio::test(start_paused = true)]
async fn front_door_ends_a_flow_at_its_flow_limit() {
    struct Silent;
    impl Dial for Silent {
        fn label(&self) -> String {
            "silent".into()
        }
        fn dial<'a>(
            &'a self,
            _target: &'a Target,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Connected, Failure>> {
            Box::pin(async move {
                let (near, far) = tokio::io::duplex(64);
                std::mem::forget(far);
                Ok(Connected::new(Box::new(near)))
            })
        }
    }
    let door = Door::new(Arc::new(Silent)).with_flow_limit(Duration::from_secs(300));
    let (mut client, server) = tokio::io::duplex(4096);
    let start = Instant::now();
    let task = tokio::spawn(async move { door.handle(server).await });
    client
        .write_all(b"CONNECT a.example:80 HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(start.elapsed(), Duration::from_secs(300));
}

/// The error of a client reaches the hook of the door (the `-v` line of
/// `pod --serve`).
#[tokio::test]
async fn front_door_gives_the_error_of_a_client_to_its_hook() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let hook = seen.clone();
    let (door, _) = door();
    let door = Arc::new(Arc::into_inner(door).unwrap().on_error(Arc::new(
        move |e: &anyhow::Error| {
            hook.lock().unwrap().push(format!("{e:#}"));
        },
    )));
    // A client that closes before its request is complete.
    let (mut client, server) = tokio::io::duplex(64);
    door.clone().spawn(server);
    client.write_all(&[5]).await.unwrap();
    drop(client);
    for _ in 0..50 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(seen.lock().unwrap().len(), 1, "{:?}", seen.lock().unwrap());
}

/// An extension takes a SOCKS5 command that the door does not know, and
/// UDP ASSOCIATE before the datagrams of the dialer.
#[tokio::test]
async fn front_door_hands_a_command_to_its_extension() {
    struct Ext(Mutex<Vec<u8>>);
    impl Extension for Ext {
        fn command<'a>(
            &'a self,
            command: u8,
            control: &'a mut dyn Duplex,
        ) -> BoxFuture<'a, anyhow::Result<bool>> {
            Box::pin(async move {
                if command == 0x04 {
                    return Ok(false);
                }
                self.0.lock().unwrap().push(command);
                control.write_all(b"ext").await?;
                Ok(true)
            })
        }
    }
    let ext = Arc::new(Ext(Mutex::new(Vec::new())));
    let (door, _) = door();
    let door = Arc::new(Arc::into_inner(door).unwrap().with_extension(ext.clone()));
    for command in [3u8, 0x83] {
        let out = ask(&door, &[5, 1, 0, 5, command, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        assert_eq!(&out[2..], b"ext", "{out:?}");
    }
    assert_eq!(*ext.0.lock().unwrap(), [3, 0x83]);
    // A command that the extension leaves is "command not supported".
    let out = ask(&door, &[5, 1, 0, 5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).await;
    assert_eq!(out[3], 0x07, "{out:?}");
}
