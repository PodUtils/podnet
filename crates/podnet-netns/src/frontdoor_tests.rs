use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A SOCKS5 upstream on loopback that records each target and echoes the
/// bytes of each flow.
async fn fake_upstream(seen: Arc<std::sync::Mutex<Vec<String>>>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut head = [0u8; 3];
                stream.read_exact(&mut head).await.unwrap();
                stream.write_all(&[5, 0]).await.unwrap();
                let mut request = [0u8; 4];
                stream.read_exact(&mut request).await.unwrap();
                let host = match request[3] {
                    1 => {
                        let mut ip = [0u8; 4];
                        stream.read_exact(&mut ip).await.unwrap();
                        std::net::Ipv4Addr::from(ip).to_string()
                    }
                    _ => {
                        let len = stream.read_u8().await.unwrap() as usize;
                        let mut name = vec![0u8; len];
                        stream.read_exact(&mut name).await.unwrap();
                        String::from_utf8(name).unwrap()
                    }
                };
                let port = stream.read_u16().await.unwrap();
                seen.lock().unwrap().push(format!("{host}:{port}"));
                stream
                    .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    port
}

async fn socks5(door: &Arc<Door>, request: &[u8]) -> (tokio::io::DuplexStream, Vec<u8>) {
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    client.write_all(request).await.unwrap();
    let mut reply = vec![0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    (client, reply)
}

fn name_request(command: u8, host: &str, port: u16) -> Vec<u8> {
    let mut request = vec![5, command, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    request
}

/// A rule that refuses a target gives SOCKS5 2 and HTTP 403 with no dial;
/// BIND is "command not supported" (the old door took it for CONNECT).
#[tokio::test]
async fn the_pod_door_refuses_a_rejected_target_and_bind() {
    pod_testkit::need_loopback!();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let port = fake_upstream(seen.clone()).await;
    let chain = vec![Proxy::parse(&format!("socks5://127.0.0.1:{port}")).unwrap()];
    let rules = vec![Rule::parse("domain:.ads.example=reject").unwrap()];
    let door = Arc::new(front_door(chain, rules, false));

    let (_, reply) = socks5(&door, &name_request(1, "x.ads.example", 443)).await;
    assert_eq!(reply[1], 0x02, "{reply:?}");
    let (_, reply) = socks5(&door, &name_request(2, "ok.example", 443)).await;
    assert_eq!(reply[1], 0x07, "{reply:?}");

    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client
        .write_all(b"CONNECT x.ads.example:443 HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut text = String::new();
    client.read_to_string(&mut text).await.unwrap();
    assert!(text.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{text}");
    assert!(seen.lock().unwrap().is_empty(), "no dial for a refusal");
}

/// A flow goes through the first upstream with the name kept; a plain GET
/// gets 405; with `--flow-log` the flow has a dial line and a close line
/// with the byte counts.
#[tokio::test]
async fn the_pod_door_carries_a_flow_and_writes_its_flow_record() {
    pod_testkit::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("flows.jsonl");
    crate::flowlog::open(&log).unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let port = fake_upstream(seen.clone()).await;
    let chain = vec![Proxy::parse(&format!("socks5://127.0.0.1:{port}")).unwrap()];
    let door = Arc::new(front_door(chain, Vec::new(), false));

    let (mut client, reply) = socks5(&door, &name_request(1, "flow.example", 80)).await;
    assert_eq!(reply[1], 0, "{reply:?}");
    client.write_all(b"ping").await.unwrap();
    let mut echo = [0u8; 4];
    client.read_exact(&mut echo).await.unwrap();
    assert_eq!(&echo, b"ping");
    drop(client);
    assert_eq!(*seen.lock().unwrap(), ["flow.example:80"]);

    let mut lines = Vec::new();
    for _ in 0..100 {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        lines = text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|v| v["target"] == "flow.example:80")
            .collect();
        if lines.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0]["event"], "dial");
    assert_eq!(lines[0]["verdict"], "allow");
    assert_eq!(lines[1]["event"], "close");
    assert_eq!(lines[1]["bytes_out"], 4);
    assert_eq!(lines[1]["bytes_in"], 4);
    assert_eq!(lines[0]["flow"], lines[1]["flow"]);

    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client
        .write_all(b"GET http://flow.example/ HTTP/1.1\r\nHost: flow.example\r\n\r\n")
        .await
        .unwrap();
    let mut text = String::new();
    client.read_to_string(&mut text).await.unwrap();
    assert!(
        text.starts_with("HTTP/1.1 405 Method Not Allowed"),
        "{text}"
    );
}
