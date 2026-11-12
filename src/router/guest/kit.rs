//! The parts of the tests of a guest wire (T-248): fake ways that dial nothing
//! on the network, a router over them, and a guest that is a stack of
//! `pod-netstack` at the other end of a socket pair.

use std::collections::VecDeque;
use std::io;
use std::io::IoSliceMut;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use pod_netstack::{NetStack, StackConfig, Subnet};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::Notify;
use tokio::time::Instant;

use super::wire::{seqpacket_pair, Opened, PacketFd, PACKET_MAX};
use crate::network::{
    BoxFuture, Class, Connected, Datagrams, Dial, Failure, RecvMeta, Stage, Target, Transmit, Way,
};
use crate::router::{parse, Router};

pub const GUEST: &str = "10.66.0.4";

pub fn at(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse::<IpAddr>().unwrap(), port)
}

pub async fn within<T>(secs: u64, what: &str, future: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(Duration::from_secs(secs), future).await {
        Ok(value) => value,
        Err(_) => panic!("{what}: no result in {secs} s"),
    }
}

/// A datagram socket in memory. A datagram that it receives goes to `sent`;
/// with `echo` it also comes back from its destination. While `blocked`, the
/// socket takes no datagram.
#[derive(Debug, Default)]
pub struct Loop {
    inner: Mutex<LoopInner>,
}

#[derive(Debug, Default)]
struct LoopInner {
    sent: Vec<(SocketAddr, Vec<u8>)>,
    inbox: VecDeque<(SocketAddr, Vec<u8>)>,
    rx: Option<Waker>,
    tx: Option<Waker>,
    blocked: bool,
    echo: bool,
}

impl Loop {
    pub fn new(echo: bool) -> Arc<Loop> {
        let l = Loop::default();
        l.inner.lock().unwrap().echo = echo;
        Arc::new(l)
    }

    pub fn block(&self, on: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.blocked = on;
        if !on {
            if let Some(w) = inner.tx.take() {
                w.wake();
            }
        }
    }

    /// The datagrams that the socket took, in order.
    pub fn sent(&self) -> Vec<(SocketAddr, Vec<u8>)> {
        self.inner.lock().unwrap().sent.clone()
    }

    /// Put a datagram in the inbox, as if `from` sent it.
    pub fn arrive(&self, from: SocketAddr, data: Vec<u8>) {
        let mut inner = self.inner.lock().unwrap();
        inner.inbox.push_back((from, data));
        if let Some(w) = inner.rx.take() {
            w.wake();
        }
    }
}

impl Datagrams for Loop {
    fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut inner = self.inner.lock().unwrap();
        if inner.blocked {
            inner.tx = Some(cx.waker().clone());
            return Poll::Pending;
        }
        Poll::Ready(Ok(()))
    }

    fn try_send(&self, t: &Transmit<'_>) -> io::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.blocked {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        inner.sent.push((t.destination, t.contents.to_vec()));
        if inner.echo {
            inner.inbox.push_back((t.destination, t.contents.to_vec()));
            if let Some(w) = inner.rx.take() {
                w.wake();
            }
        }
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let mut inner = self.inner.lock().unwrap();
        let Some((from, data)) = inner.inbox.pop_front() else {
            inner.rx = Some(cx.waker().clone());
            return Poll::Pending;
        };
        let n = data.len().min(bufs[0].len());
        bufs[0][..n].copy_from_slice(&data[..n]);
        meta[0] = RecvMeta {
            addr: from,
            len: n,
            stride: n,
            dst_ip: None,
        };
        Poll::Ready(Ok(1))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::from(([127, 0, 0, 1], 0)))
    }
}

/// A fake way. A TCP dial waits for the gate, then gives a stream whose far
/// end writes `LABEL|` and echoes each byte it reads. The datagrams are one
/// [`Loop`].
pub struct Fake {
    pub label: &'static str,
    pub fails: bool,
    pub gate: Option<Arc<Notify>>,
    pub dialed: Mutex<Vec<String>>,
    pub udp: Arc<Loop>,
}

impl Fake {
    pub fn new(label: &'static str) -> Arc<Fake> {
        Arc::new(Fake {
            label,
            fails: false,
            gate: None,
            dialed: Mutex::new(Vec::new()),
            udp: Loop::new(true),
        })
    }

    pub fn with(label: &'static str, fails: bool, gate: Option<Arc<Notify>>) -> Arc<Fake> {
        Arc::new(Fake {
            label,
            fails,
            gate,
            dialed: Mutex::new(Vec::new()),
            udp: Loop::new(true),
        })
    }

    pub fn dialed(&self) -> Vec<String> {
        self.dialed.lock().unwrap().clone()
    }

    pub fn way(self: &Arc<Self>) -> Way {
        Way::destination(self.clone())
    }
}

impl Dial for Fake {
    fn label(&self) -> String {
        self.label.to_string()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.dialed.lock().unwrap().push(target.to_string());
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            if self.fails {
                return Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    format!("{} is down", self.label),
                ));
            }
            let (near, mut far) = tokio::io::duplex(64 * 1024);
            let label = format!("{}|", self.label);
            tokio::spawn(async move {
                let _ = far.write_all(label.as_bytes()).await;
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    match far.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if far.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                let _ = far.shutdown().await;
            });
            Ok(Connected::new(Box::new(near)))
        })
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move { Ok(self.udp.clone() as Arc<dyn Datagrams>) })
    }
}

/// The router over fake ways, the served guest wire, and the guest stack.
pub struct Rig {
    pub router: Router,
    pub a: Arc<Fake>,
    pub b: Arc<Fake>,
    pub guest: NetStack,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    pub served: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

pub fn guest_config() -> StackConfig {
    StackConfig {
        subnet: Subnet {
            v4_prefix: 0,
            v6_prefix: 0,
            ..Subnet::default()
        },
        local_v4: GUEST.parse().unwrap(),
        mtu: 1500,
        ..StackConfig::default()
    }
}

/// Move the packets of `guest` over `fd`, and the packets of `fd` into it.
pub fn join(guest: &NetStack, fd: OwnedFd) -> Vec<tokio::task::JoinHandle<()>> {
    let io = Arc::new(PacketFd::new(fd).unwrap());
    let out = guest.link_out();
    let into = guest.link_in();
    let send = io.clone();
    let up = tokio::spawn(async move {
        loop {
            let packet = out.recv().await;
            if send.send(&packet).await.is_err() {
                return;
            }
        }
    });
    let down = tokio::spawn(async move {
        let mut buf = vec![0u8; PACKET_MAX];
        while let Ok(n) = io.recv(&mut buf).await {
            if n == 0 {
                return;
            }
            into.push(buf[..n].to_vec());
        }
    });
    vec![up, down]
}

impl Rig {
    /// A router of the file `text` (networks `a` and `b` are the fake ways;
    /// the first guest has the wire `unix:/unused`), served over a socket
    /// pair.
    pub async fn new(text: &str, a: Arc<Fake>, b: Arc<Fake>) -> Rig {
        let config = parse(text).unwrap();
        let mut ways = std::collections::HashMap::new();
        ways.insert("a".to_string(), a.way());
        ways.insert("b".to_string(), b.way());
        let router = Router::build(&config, ways).unwrap();
        let (near, far) = seqpacket_pair().unwrap();
        let served = tokio::spawn(router.guests[0].clone().serve(Opened::Packets(near)));
        let guest = NetStack::over_link(guest_config());
        let tasks = join(&guest, far);
        Rig {
            router,
            a,
            b,
            guest,
            tasks,
            served: Some(served),
        }
    }

    pub fn stats(&self) -> &super::GuestStats {
        &self.router.guests[0].stats
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        if let Some(served) = &self.served {
            served.abort();
        }
    }
}

/// The file of the tests: the networks `a` and `b`, a group `g` over `b`, and
/// one guest with the rules file `rules`.
pub fn file(rules: &std::path::Path) -> String {
    format!(
        "networks:\n  a: direct\n  b: direct\ngroups:\n  g: b*1\nguests:\n  - wire: unix:/unused\n    net: 10.66.0.0/24\n    via: g\n    rules: {}\n",
        rules.display()
    )
}

/// The rules of the tests: `a` takes 93.184.216.0/24 and `reject` takes
/// 198.51.100.0/24.
pub const RULES: &str = "cidr:93.184.216.0/24=a\ncidr:198.51.100.0/24=reject\n";

/// A tempdir with the rules file of the tests, and the file that names it.
pub fn rules_file() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.rules");
    std::fs::write(&path, RULES).unwrap();
    let text = file(&path);
    (dir, text)
}
