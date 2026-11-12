//! The door over a group of `podnet_core::network` (T-178). The door is in
//! `podnet_netns::door`; the group is in `podnet-core`, which `podnet-netns`
//! does not use.

use std::sync::{Arc, Mutex};

use podnet_core::network::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use podnet_netns::door::Door;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

/// A dialer that answers each flow with the target in text.
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
