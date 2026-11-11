use std::collections::VecDeque;
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use super::super::balance::{Balance, Health, Member};
use super::super::fakes::{log, soon, Fake};
use super::super::{
    recv_from, send_to, BoxFuture, Class, Connected, Datagrams, Dial, Failure, RecvMeta, Stage,
    Target, Transmit, Way,
};
use super::*;

/// A datagram socket in memory: it keeps what it sent, and it receives
/// what a test put in its inbox.
#[derive(Debug, Default)]
struct Pipe {
    sent: Mutex<Vec<Vec<u8>>>,
    inbox: Mutex<VecDeque<Vec<u8>>>,
}

impl Datagrams for Pipe {
    fn poll_writable(&self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn try_send(&self, t: &Transmit<'_>) -> io::Result<()> {
        self.sent.lock().unwrap().push(t.contents.to_vec());
        Ok(())
    }

    fn poll_recv(
        &self,
        _cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let Some(data) = self.inbox.lock().unwrap().pop_front() else {
            return Poll::Pending;
        };
        bufs[0][..data.len()].copy_from_slice(&data);
        meta[0] = RecvMeta {
            addr: SocketAddr::from(([192, 0, 2, 9], 53)),
            len: data.len(),
            stride: data.len(),
            dst_ip: None,
        };
        Poll::Ready(Ok(1))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::from(([127, 0, 0, 1], 0)))
    }
}

/// A way that carries datagrams over one [`Pipe`] (TCP is a fake stream).
struct UdpWay {
    tcp: Fake,
    pipe: Option<Arc<Pipe>>,
}

impl Dial for UdpWay {
    fn label(&self) -> String {
        self.tcp.label.to_string()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.tcp.dial(target, deadline)
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            match &self.pipe {
                Some(p) => Ok(p.clone() as Arc<dyn Datagrams>),
                None => Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    "no UDP".to_string(),
                )),
            }
        })
    }
}

fn way(label: &'static str, pipe: Option<Arc<Pipe>>) -> Way {
    let tcp = Fake::up(label, &log());
    Way::destination(Arc::new(UdpWay { tcp, pipe }))
}

fn pair() -> (Arc<Pipe>, Arc<Pipe>) {
    (Arc::new(Pipe::default()), Arc::new(Pipe::default()))
}

fn group(a: &Arc<Pipe>, b: &Arc<Pipe>) -> Balance {
    Balance::new(
        "g",
        vec![
            Member::new("a", way("a", Some(a.clone())), 1),
            Member::new("b", way("b", Some(b.clone())), 1),
        ],
    )
    .unwrap()
    .with_seed(3)
}

fn dest() -> SocketAddr {
    SocketAddr::from(([192, 0, 2, 9], 53))
}

/// T-179: with a split, the datagrams of a socket leave by `up`, the
/// answers are received on `down`, and an answer on `up` is not lost.
#[tokio::test]
async fn group_split_sends_on_up_and_receives_on_down() {
    let (a, b) = pair();
    let g = group(&a, &b).with_split("a", "b").unwrap();
    let socket = g.datagrams().await.unwrap();
    for i in 0..5u8 {
        send_to(&*socket, &[i], dest()).await.unwrap();
    }
    assert_eq!(a.sent.lock().unwrap().len(), 5, "all sends leave by up");
    assert!(b.sent.lock().unwrap().is_empty(), "none leaves by down");
    b.inbox.lock().unwrap().push_back(b"from-down".to_vec());
    a.inbox.lock().unwrap().push_back(b"from-up".to_vec());
    let mut buf = [0u8; 32];
    let (n, _) = recv_from(&*socket, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"from-down", "down is read first");
    let (n, _) = recv_from(&*socket, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"from-up", "an answer on up is delivered");
    let status = g.status();
    assert_eq!(status["split"]["mode"], "udp");
    assert_eq!(status["split"]["up"], "a");
    assert_eq!(status["split"]["down"], "b");
    assert_eq!(status["split"]["sockets"], 1);
    assert_eq!(status["split"]["fallbacks"], 0);
}

/// T-179: the split is off by default. A socket takes one member and uses
/// it in both directions.
#[tokio::test]
async fn group_split_is_off_by_default() {
    let (a, b) = pair();
    let g = group(&a, &b);
    assert_eq!(g.split(), None);
    let socket = g.datagrams().await.unwrap();
    send_to(&*socket, b"x", dest()).await.unwrap();
    let used = if a.sent.lock().unwrap().len() == 1 {
        &a
    } else {
        &b
    };
    used.inbox.lock().unwrap().push_back(b"y".to_vec());
    let other = if Arc::ptr_eq(used, &a) { &b } else { &a };
    assert!(other.sent.lock().unwrap().is_empty());
    let mut buf = [0u8; 8];
    let (n, _) = recv_from(&*socket, &mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"y");
    let split = &g.status()["split"];
    assert_eq!(split["mode"], "off");
    assert_eq!(split["up"], serde_json::Value::Null);
    assert_eq!(split["sockets"], 0);
}

/// T-179: when a member of the split is down, the socket falls back to
/// one member of the pick; with no member up the kill switch refuses it.
#[tokio::test]
async fn group_split_falls_back_to_one_member_and_keeps_the_kill_switch() {
    let (a, b) = pair();
    let g = group(&a, &b).with_split("a", "b").unwrap();
    g.set_health(1, Health::Down);
    let socket = g.datagrams().await.unwrap();
    send_to(&*socket, b"x", dest()).await.unwrap();
    assert_eq!(a.sent.lock().unwrap().len(), 1, "the member that is up");
    assert_eq!(g.status()["split"]["fallbacks"], 1);
    assert_eq!(g.status()["split"]["sockets"], 0);
    g.set_health(0, Health::Down);
    let err = g.datagrams().await.expect_err("the kill switch");
    assert_eq!(err.class, Class::Policy);
}

/// T-179: a `down` member with no datagrams leaves the socket on `up`.
#[tokio::test]
async fn group_split_keeps_up_when_down_has_no_datagrams() {
    let a = Arc::new(Pipe::default());
    let g = Balance::new(
        "g",
        vec![
            Member::new("a", way("a", Some(a.clone())), 1),
            Member::new("b", way("b", None), 1),
        ],
    )
    .unwrap()
    .with_split("a", "b")
    .unwrap();
    let socket = g.datagrams().await.unwrap();
    send_to(&*socket, b"x", dest()).await.unwrap();
    assert_eq!(a.sent.lock().unwrap().len(), 1);
    assert_eq!(g.status()["split"]["fallbacks"], 1);
}

/// T-179: TCP keeps one member, also with a split. The option refuses a
/// split of TCP with a clear error, and refuses members that cannot split.
#[tokio::test]
async fn group_split_keeps_tcp_on_one_member_and_refuses_bad_values() {
    let (a, b) = pair();
    let g = group(&a, &b).with_split("a", "b").unwrap();
    let conn = g
        .dial(&Target::new("svc.example", 443), soon())
        .await
        .unwrap();
    drop(conn);
    let flows: u64 = g.members().iter().map(|m| m.flows()).sum();
    assert_eq!(flows, 1, "one flow, one member");
    let members = ["a".to_string(), "b".to_string()];
    assert_eq!(parse("off", &members), Ok(None));
    assert_eq!(
        parse("up=a down=b", &members),
        Ok(Some(("a".into(), "b".into())))
    );
    assert_eq!(parse("tcp", &members), Err(TCP_REFUSAL.to_string()));
    assert!(parse("up=a down=a", &members)
        .unwrap_err()
        .contains("needs two"));
    assert!(parse("up=a down=z", &members)
        .unwrap_err()
        .contains("not a member"));
    assert!(parse("up=a", &members).is_err());
    assert!(parse("", &members).is_err());
    assert!(group(&a, &b).with_split("a", "a").is_err());
    assert!(group(&a, &b).with_split("a", "z").is_err());
}
