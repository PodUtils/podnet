//! Fake dialers of the tests of the combinators: no network. A dial
//! writes the label of its way into the stream, so that a test reads which
//! way carried the flow.

use std::collections::VecDeque;
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

use super::{
    BoxFuture, Class, Connected, Datagrams, Dial, Failure, RecvMeta, Stage, Target, Transmit, Way,
};

/// The labels of the dials, in the order that they started.
pub type Log = Arc<Mutex<Vec<String>>>;

/// A fake way.
pub struct Fake {
    pub label: &'static str,
    /// The time before the dial ends.
    pub delay: Duration,
    /// True: the dial fails after its delay.
    pub fails: bool,
    pub names_inside: bool,
    pub log: Log,
}

impl Fake {
    /// A way that connects at once.
    pub fn up(label: &'static str, log: &Log) -> Fake {
        Fake {
            label,
            delay: Duration::ZERO,
            fails: false,
            names_inside: false,
            log: log.clone(),
        }
    }

    /// A way that fails at once.
    pub fn down(label: &'static str, log: &Log) -> Fake {
        Fake {
            fails: true,
            ..Fake::up(label, log)
        }
    }

    /// The same way, with `delay` before its end.
    pub fn after(mut self, delay: Duration) -> Fake {
        self.delay = delay;
        self
    }

    pub fn way(self) -> Way {
        Way::destination(Arc::new(self))
    }
}

impl Dial for Fake {
    fn label(&self) -> String {
        self.label.to_string()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(self.label.to_string());
            tokio::time::sleep(self.delay).await;
            if self.fails {
                return Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    format!("{} failed", self.label),
                ));
            }
            let (near, mut far) = tokio::io::duplex(256);
            let label = self.label.as_bytes().to_vec();
            tokio::spawn(async move {
                let _ = far.write_all(&label).await;
                let _ = far.shutdown().await;
                let mut rest = Vec::new();
                let _ = far.read_to_end(&mut rest).await;
            });
            Ok(Connected::new(Box::new(near)))
        })
    }

    fn names_inside(&self) -> bool {
        self.names_inside
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(format!("{} udp", self.label));
            Err(Failure::new(
                Stage::Connect,
                Class::Unavailable,
                format!("{} carries no UDP", self.label),
            ))
        })
    }
}

/// A new empty log.
pub fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

/// The labels in `log`.
pub fn seen(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// The label that the far end of `conn` wrote: the way that carried it.
pub async fn carrier(mut conn: Connected) -> String {
    let mut text = String::new();
    conn.io.read_to_string(&mut text).await.unwrap();
    text
}

/// A deadline in 30 s.
pub fn soon() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

/// A datagram socket in memory: it keeps what it sent, and it receives
/// what a test put in its inbox.
#[derive(Debug, Default)]
pub struct Pipe {
    pub sent: Mutex<Vec<Vec<u8>>>,
    pub inbox: Mutex<VecDeque<Vec<u8>>>,
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
pub struct UdpWay {
    pub tcp: Fake,
    pub pipe: Option<Arc<Pipe>>,
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

/// A way that carries datagrams over `pipe`, or none.
pub fn udp_way(label: &'static str, pipe: Option<Arc<Pipe>>) -> Way {
    let tcp = Fake::up(label, &log());
    Way::destination(Arc::new(UdpWay { tcp, pipe }))
}
