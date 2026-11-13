//! The local SOCKS5 door at the opener of a link (T-291): `podnet link
//! dial --service socks5 --listen ADDRESS`.
//!
//! Local clients speak SOCKS5 (and HTTP `CONNECT`) to the door
//! ([`podnet_netns::door::Door`], the code of the front door). The door
//! has a way out ([`Way`]) that opens one stream of the link for each
//! request and speaks SOCKS5 to the service `socks5` of the peer
//! ([`crate::link_socks`]):
//!
//! * `CONNECT`: a stream with a `CONNECT` to the target. A name goes to the
//!   peer as a name (`--resolve peer`), or resolves here and goes as an
//!   address (`--resolve local`).
//! * `BIND` (command 2): the request goes to the peer on a new stream and
//!   its two replies and the bytes of the incoming connection pass on.
//! * `UDP ASSOCIATE` (command 3): the door has a UDP socket for the client.
//!   Its datagrams go as the frames of the command 0x83 over one stream
//!   ([`Frames`]). A name in a datagram resolves here.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use pod_core::datagrams::{Datagrams, RecvMeta, Transmit};
use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use podnet_netns::door::{Door, Duplex, Extension};
use podnet_netns::socks::{socks5_udp_unwrap, ProxyHost};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::link::{Link, Stream};
use crate::link_service as service;

/// Where a name is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolve {
    /// At the peer: the name goes in the request (the default).
    Peer,
    /// At this host: the request has an address.
    Local,
}

/// The most datagrams that wait for the writer of the stream.
const QUEUE: usize = 256;

/// The time that the peer has to answer a request of the door.
const ANSWER: Duration = Duration::from_secs(30);

/// The way out of the door: streams of one link.
pub struct Way {
    link: Arc<Link>,
    resolve: Resolve,
}

impl Way {
    pub fn new(link: Arc<Link>, resolve: Resolve) -> Way {
        Way { link, resolve }
    }

    /// The door over this way.
    pub fn door(self) -> Door {
        let link = self.link.clone();
        Door::new(Arc::new(self)).with_extension(Arc::new(Bind { link }))
    }
}

/// A stream with the service `socks5` of the peer, after the greeting.
async fn session(link: &Link) -> Result<Stream> {
    let mut stream = link.open().map_err(crate::link::fault)?;
    service::request(&mut stream, crate::link_socks::NAME).await?;
    stream.write_all(&[5, 1, 0]).await?;
    let mut method = [0u8; 2];
    stream.read_exact(&mut method).await?;
    if method != [5, 0] {
        bail!("the peer answered {method:?} to the greeting of SOCKS5");
    }
    Ok(stream)
}

/// Read one reply of SOCKS5: its code and its address.
async fn reply(stream: &mut Stream) -> Result<(u8, Option<SocketAddr>)> {
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    let addr = match head[3] {
        1 => {
            let mut b = [0u8; 6];
            stream.read_exact(&mut b).await?;
            Some(SocketAddr::from((
                [b[0], b[1], b[2], b[3]],
                u16::from_be_bytes([b[4], b[5]]),
            )))
        }
        4 => {
            let mut b = [0u8; 18];
            stream.read_exact(&mut b).await?;
            let ip: [u8; 16] = b[..16].try_into()?;
            Some(SocketAddr::from((ip, u16::from_be_bytes([b[16], b[17]]))))
        }
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut rest = vec![0u8; usize::from(len[0]) + 2];
            stream.read_exact(&mut rest).await?;
            None
        }
        other => bail!("the peer sent the address type {other} in a reply"),
    };
    Ok((head[1], addr))
}

/// The request of `command` to an address or a name.
fn request(command: u8, host: &ProxyHost, port: u16) -> Result<Vec<u8>> {
    let mut out = vec![5, command, 0];
    match host {
        ProxyHost::Ip(IpAddr::V4(ip)) => {
            out.push(1);
            out.extend_from_slice(&ip.octets());
        }
        ProxyHost::Ip(IpAddr::V6(ip)) => {
            out.push(4);
            out.extend_from_slice(&ip.octets());
        }
        ProxyHost::Domain(name) => {
            let len = u8::try_from(name.len()).map_err(|_| anyhow!("the name is too long"))?;
            out.push(3);
            out.push(len);
            out.extend_from_slice(name.as_bytes());
        }
    }
    out.extend_from_slice(&port.to_be_bytes());
    Ok(out)
}

fn failure(stage: Stage, class: Class, text: impl Into<String>) -> Failure {
    Failure::new(stage, class, text)
}

/// The failure for a reply code of the peer. The door maps a text with
/// "refused" or "denied" to a host failure, so the text says it.
fn refusal(code: u8, target: &Target) -> Failure {
    let (class, what) = match code {
        2 => (Class::Policy, "denied by the peer"),
        6 => (Class::Timeout, "timed out at the peer"),
        5 => (Class::Refused, "refused at the peer"),
        3 | 4 => (Class::Unavailable, "unreachable from the peer"),
        7 => (Class::Unavailable, "not supported by the peer"),
        _ => (Class::Unavailable, "failed at the peer"),
    };
    failure(
        Stage::Connect,
        class,
        format!("the peer answered {code} for {target}: {what}"),
    )
}

impl Way {
    async fn connect(&self, target: &Target) -> Result<Connected, Failure> {
        let unavailable =
            |e: anyhow::Error| failure(Stage::Connect, Class::Unavailable, format!("{e:#}"));
        let (host, port) = match target {
            Target::Addr(addr) => (ProxyHost::Ip(addr.ip()), addr.port()),
            Target::Name { host, port } => match (self.resolve, host.parse::<IpAddr>()) {
                (_, Ok(ip)) => (ProxyHost::Ip(ip), *port),
                (Resolve::Peer, Err(_)) => (ProxyHost::Domain(host.clone()), *port),
                (Resolve::Local, Err(_)) => {
                    let found = tokio::net::lookup_host((host.as_str(), *port))
                        .await
                        .ok()
                        .and_then(|mut a| a.next())
                        .ok_or_else(|| {
                            failure(
                                Stage::Dns,
                                Class::NotFound,
                                format!("{host} has no address"),
                            )
                        })?;
                    (ProxyHost::Ip(found.ip()), *port)
                }
            },
            #[allow(unreachable_patterns)]
            _ => return Err(failure(Stage::Connect, Class::Usage, "no target")),
        };
        let mut stream = session(&self.link).await.map_err(unavailable)?;
        let wire = request(1, &host, port).map_err(unavailable)?;
        stream
            .write_all(&wire)
            .await
            .map_err(|e| unavailable(e.into()))?;
        let (code, _) = reply(&mut stream).await.map_err(unavailable)?;
        if code != 0 {
            return Err(refusal(code, target));
        }
        Ok(Connected::new(Box::new(stream)))
    }
}

impl Dial for Way {
    fn label(&self) -> String {
        "link socks5".to_string()
    }

    fn names_inside(&self) -> bool {
        self.resolve == Resolve::Peer
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let limit = deadline
                .saturating_duration_since(Instant::now())
                .min(ANSWER);
            tokio::time::timeout(limit, self.connect(target))
                .await
                .unwrap_or_else(|_| {
                    Err(failure(
                        Stage::Connect,
                        Class::Timeout,
                        format!("no answer for {target} in time"),
                    ))
                })
        })
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            let bad =
                |e: anyhow::Error| failure(Stage::Connect, Class::Unavailable, format!("{e:#}"));
            let open = async {
                let mut stream = session(&self.link).await?;
                stream.write_all(&[5, 0x83, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                let (code, _) = reply(&mut stream).await?;
                if code != 0 {
                    bail!("the peer answered {code} to UDP in the stream");
                }
                Ok::<Stream, anyhow::Error>(stream)
            };
            let stream = tokio::time::timeout(ANSWER, open)
                .await
                .map_err(|_| failure(Stage::Connect, Class::Timeout, "no answer for UDP in time"))?
                .map_err(bad)?;
            Ok(Arc::new(Frames::start(stream)) as Arc<dyn Datagrams>)
        })
    }
}

/// `BIND` (command 2) of a local client: the request goes to the peer and
/// everything after it passes both ways.
struct Bind {
    link: Arc<Link>,
}

impl Extension for Bind {
    fn command<'a>(
        &'a self,
        command: u8,
        control: &'a mut dyn Duplex,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            if command != 2 {
                return Ok(false);
            }
            let mut stream = match session(&self.link).await {
                Ok(stream) => stream,
                Err(_) => {
                    control.write_all(&[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                    return Ok(true);
                }
            };
            stream.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            tokio::io::copy_bidirectional(control, &mut stream).await?;
            Ok(true)
        })
    }
}

/// The datagrams of the command 0x83: each one is a frame on a stream.
pub struct Frames {
    send: mpsc::Sender<(SocketAddr, Vec<u8>)>,
    inbox: Mutex<mpsc::Receiver<(SocketAddr, Vec<u8>)>>,
}

impl std::fmt::Debug for Frames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Frames")
    }
}

impl Frames {
    /// Run the two tasks of the stream. Each ends when the stream ends or
    /// when this value drops.
    fn start(stream: Stream) -> Frames {
        let (mut read, mut write) = tokio::io::split(stream);
        let (send, mut out) = mpsc::channel::<(SocketAddr, Vec<u8>)>(QUEUE);
        let (deliver, inbox) = mpsc::channel::<(SocketAddr, Vec<u8>)>(QUEUE);
        tokio::spawn(async move {
            while let Some((to, payload)) = out.recv().await {
                let datagram = pod_proxy::udp::wrap(&payload, to.ip(), to.port());
                if pod_proxy::udp::write_frame(&mut write, &datagram)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            let _ = write.shutdown().await;
        });
        tokio::spawn(async move {
            while let Ok(Some(datagram)) = pod_proxy::udp::read_frame(&mut read).await {
                let Some((ProxyHost::Ip(ip), port, payload)) = socks5_udp_unwrap(&datagram) else {
                    continue;
                };
                // A full inbox drops the datagram, as a UDP socket does.
                let _ = deliver.try_send((SocketAddr::new(ip, port), payload.to_vec()));
            }
        });
        Frames {
            send,
            inbox: Mutex::new(inbox),
        }
    }
}

impl Datagrams for Frames {
    fn poll_writable(&self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.send.is_closed() {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        Poll::Ready(Ok(()))
    }

    fn try_send(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        match self
            .send
            .try_send((transmit.destination, transmit.contents.to_vec()))
        {
            Ok(()) => Ok(()),
            // A full queue drops the datagram.
            Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let (Some(buf), Some(m)) = (bufs.first_mut(), meta.first_mut()) else {
            return Poll::Ready(Ok(0));
        };
        let mut inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        match inbox.poll_recv(cx) {
            Poll::Ready(Some((addr, data))) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                *m = RecvMeta {
                    addr,
                    len,
                    stride: len,
                    dst_ip: None,
                };
                Poll::Ready(Ok(1))
            }
            Poll::Ready(None) => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
            Poll::Pending => Poll::Pending,
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::from(([0, 0, 0, 0], 0)))
    }
}
