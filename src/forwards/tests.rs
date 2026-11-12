use std::sync::Mutex;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;
use crate::network::{BoxFuture, Connected, Failure};

/// A network that answers each flow with the target in text.
struct Named(Mutex<Vec<String>>);

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
            self.0.lock().unwrap().push(target.to_string());
            let (near, mut far) = tokio::io::duplex(1024);
            let text = format!("via named to {target}");
            tokio::spawn(async move {
                let _ = far.write_all(text.as_bytes()).await;
                let _ = far.shutdown().await;
            });
            Ok(Connected::new(Box::new(near)))
        })
    }
}

#[test]
fn forward_rule_parses_each_form() {
    let rule = Rule::parse("8080=db.internal:5432").unwrap();
    assert_eq!(
        rule.listen,
        ProxyListen::Tcp("127.0.0.1:8080".parse().unwrap())
    );
    assert_eq!(rule.target, Target::new("db.internal", 5432));
    let rule = Rule::parse("unix:/run/f.sock=[2001:db8::1]:22").unwrap();
    assert!(matches!(rule.listen, ProxyListen::Unix(_)));
    assert_eq!(rule.target.port(), 22);
    let rule = Rule::parse("udp:5353=1.1.1.1:53").unwrap();
    assert!(rule.udp);
    assert!(!Rule::parse("5353=1.1.1.1:53").unwrap().udp);
    for bad in [
        "udp:unix:/x=h:1",
        "8080",
        "0=h:1",
        "8080=h",
        "8080=h:0",
        "8080=:22",
    ] {
        assert!(Rule::parse(bad).is_err(), "{bad}");
    }
}

/// T-099, the Prove: a connection to the listener goes through the
/// network to the target of the rule.
#[tokio::test]
async fn forward_over_a_network() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("forward.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let named = Arc::new(Named(Mutex::new(Vec::new())));
    let target = Target::new("db.internal", 5432);
    tokio::spawn(serve_unix(listener, named.clone(), target));
    for _ in 0..2 {
        let mut client = tokio::net::UnixStream::connect(&path).await.unwrap();
        let mut back = String::new();
        tokio::time::timeout(Duration::from_secs(10), client.read_to_string(&mut back))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(back, "via named to db.internal:5432");
    }
    assert_eq!(named.0.lock().unwrap().len(), 2);
}

// T-266: `-L LISTEN=peer:FINGERPRINT/SERVICE`.

fn fingerprint() -> String {
    let dir = tempfile::tempdir().unwrap();
    crate::identity::create(dir.path()).unwrap().fingerprint()
}

#[test]
fn a_rule_to_a_peer_keeps_the_spec_and_the_port_0_target() {
    let fp = fingerprint();
    let rule = Rule::parse(&format!("8080=peer:{fp}/http")).unwrap();
    assert_eq!(
        rule.via.as_deref(),
        Some(format!("peer:{fp}/http").as_str())
    );
    assert_eq!(rule.target.port(), 0);
    assert!(!rule.udp);
    let plain = Rule::parse("8080=example.com:80").unwrap();
    assert_eq!(plain.via, None);
}

#[test]
fn a_rule_to_a_peer_refuses_udp_and_a_bad_spec() {
    let fp = fingerprint();
    for spec in [
        format!("udp:5353=peer:{fp}/dns"),
        format!("8080=peer:{fp}"),
        "8080=peer:nope/http".to_string(),
    ] {
        assert!(Rule::parse(&spec).is_err(), "{spec}");
    }
}
