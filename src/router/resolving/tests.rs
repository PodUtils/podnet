use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use podnet_netns::dns::{build_response, ParsedQuery, TYPE_A, TYPE_AAAA};

use super::*;
use crate::network::{BoxFuture, Class, Connected, Failure, Stage};

const UPSTREAM: &str = "10.0.0.53:53";

/// The far end of a network: it dials a DNS server at `UPSTREAM`, and
/// answers each other flow with `ok:` and the target. It logs each dial.
struct Far {
    log: Mutex<Vec<String>>,
    names: HashMap<&'static str, Vec<IpAddr>>,
    /// The addresses whose connect fails.
    dead: Vec<IpAddr>,
    inside: bool,
}

impl Far {
    fn new(names: &[(&'static str, &[&str])]) -> Arc<Far> {
        Arc::new(Far {
            log: Mutex::default(),
            names: names
                .iter()
                .map(|(n, ips)| (*n, ips.iter().map(|i| i.parse().unwrap()).collect()))
                .collect(),
            dead: Vec::new(),
            inside: false,
        })
    }

    fn seen(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Dial for Far {
    fn label(&self) -> String {
        "far".into()
    }

    fn names_inside(&self) -> bool {
        self.inside
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(target.to_string());
            if let Target::Addr(a) = target {
                if self.dead.contains(&a.ip()) {
                    return Err(Failure::new(Stage::Connect, Class::Refused, "refused"));
                }
            }
            let (near, mut far) = tokio::io::duplex(4096);
            if target.to_string() == UPSTREAM {
                let names = self.names.clone();
                tokio::spawn(async move {
                    // DNS over TCP: a 2-byte length, then the message.
                    while let Ok(len) = far.read_u16().await {
                        let mut msg = vec![0u8; len as usize];
                        if far.read_exact(&mut msg).await.is_err() {
                            return;
                        }
                        let q = ParsedQuery::parse(&msg).unwrap();
                        let all = names.get(q.name.as_str()).cloned().unwrap_or_default();
                        let want: Vec<IpAddr> = all
                            .into_iter()
                            .filter(|ip| match q.qtype {
                                TYPE_A => ip.is_ipv4(),
                                TYPE_AAAA => ip.is_ipv6(),
                                _ => false,
                            })
                            .collect();
                        let rcode = if names.contains_key(q.name.as_str()) {
                            0
                        } else {
                            3
                        };
                        let answer = build_response(&q, &want, rcode);
                        let _ = far.write_u16(answer.len() as u16).await;
                        let _ = far.write_all(&answer).await;
                    }
                });
            } else {
                let text = format!("ok:{target}");
                tokio::spawn(async move {
                    let _ = far.write_all(text.as_bytes()).await;
                    let _ = far.shutdown().await;
                });
            }
            Ok(Connected::new(Box::new(near)))
        })
    }
}

fn soon() -> Instant {
    Instant::now() + std::time::Duration::from_secs(10)
}

async fn text_of(mut conn: Connected) -> String {
    let mut text = String::new();
    conn.io.read_to_string(&mut text).await.unwrap();
    text
}

fn resolving(far: &Arc<Far>) -> Resolving {
    Resolving::new(far.clone(), UPSTREAM.parse().unwrap())
}

/// T-098 item 3: a name resolves through the network of the member, over
/// its own dial (DNS over TCP to the upstream at the far end), and then the
/// flow connects to the address: IPv4 first.
#[tokio::test]
async fn route_resolver_name_resolves_through_the_member() {
    let far = Far::new(&[("svc.example", &["2001:db8::7", "192.0.2.7"])]);
    let way = resolving(&far);
    let target: Target = "svc.example:443".parse().unwrap();
    let conn = way.dial(&target, soon()).await.unwrap();
    assert_eq!(text_of(conn).await, "ok:192.0.2.7:443");
    assert_eq!(
        far.seen(),
        [UPSTREAM, "192.0.2.7:443"],
        "one DNS connection, then the address"
    );
    assert!(way.names_inside(), "the name resolves inside the network");
}

/// T-098 item 3: an address needs no query; a way whose far end resolves
/// names (a SOCKS5h proxy) gets the name as it is.
#[tokio::test]
async fn route_resolver_leaves_addresses_and_far_end_names_alone() {
    let far = Far::new(&[]);
    let way = resolving(&far);
    let conn = way
        .dial(&"192.0.2.9:80".parse().unwrap(), soon())
        .await
        .unwrap();
    assert_eq!(text_of(conn).await, "ok:192.0.2.9:80");
    assert_eq!(far.seen(), ["192.0.2.9:80"]);

    let inside = Arc::new(Far {
        inside: true,
        ..Arc::into_inner(Far::new(&[])).unwrap()
    });
    let way = Resolving::new(inside.clone(), UPSTREAM.parse().unwrap());
    let conn = way
        .dial(&"svc.example:443".parse().unwrap(), soon())
        .await
        .unwrap();
    assert_eq!(text_of(conn).await, "ok:svc.example:443");
    assert_eq!(
        inside.seen(),
        ["svc.example:443"],
        "no query: the far end resolves"
    );
}

/// T-098 item 3: a name that the network cannot resolve is an error. The
/// flow does not fall back to the resolver of this host.
#[tokio::test]
async fn route_resolver_unknown_name_is_an_error_with_no_fallback() {
    let far = Far::new(&[("svc.example", &["192.0.2.7"])]);
    let way = resolving(&far);
    let err = way
        .dial(&"nowhere.example:443".parse().unwrap(), soon())
        .await
        .map(|_| ())
        .unwrap_err();
    assert!(err.text().contains("nowhere.example"), "{}", err.text());
    assert_eq!(far.seen(), [UPSTREAM], "only the query was dialed");
}

/// T-098 item 3: the next address is tried when one fails to connect.
#[tokio::test]
async fn route_resolver_tries_the_next_address() {
    let far = Arc::new(Far {
        dead: vec!["192.0.2.7".parse().unwrap()],
        ..Arc::into_inner(Far::new(&[("svc.example", &["192.0.2.7", "192.0.2.8"])])).unwrap()
    });
    let way = Resolving::new(far.clone(), UPSTREAM.parse().unwrap());
    let conn = way
        .dial(&"svc.example:443".parse().unwrap(), soon())
        .await
        .unwrap();
    assert_eq!(text_of(conn).await, "ok:192.0.2.8:443");
    assert_eq!(far.seen(), [UPSTREAM, "192.0.2.7:443", "192.0.2.8:443"]);
}
