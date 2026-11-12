#[allow(unused_imports)]
use crate::prelude::*;

use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

use crate::door::{Door, Duplex, Extension};

// ── front door: podnet-netns as a proxy for others ─────────────────────────────

pub(crate) enum FrontListener {
    Unix(tokio::net::UnixListener),
    Tcp(tokio::net::TcpListener),
}

/// Serve SOCKS5 and HTTP `CONNECT`, forwarding each request through the
/// configured upstream chain. This makes an instance a hop for another, and a
/// drop-in replacement for a shim's front door. The door is the one of
/// [`crate::door`] (T-061) over [`PodDial`].
pub(crate) async fn serve_front_door(
    listen: crate::listen::ProxyListen,
    chain: Vec<Proxy>,
    rules: Vec<Rule>,
    verbose: bool,
) -> Result<()> {
    let (listener, bound) = bind_front_door(&listen).await?;
    // Issue 300: the bound address, so a TCP port 0 shows the port that
    // the kernel chose.
    eprintln!(
        "{}: serving {bound} ({} upstream)",
        crate::cli::program_name(),
        chain.len()
    );
    let door = Arc::new(front_door(chain, rules, verbose));
    match listener {
        FrontListener::Unix(listener) => door.serve_unix(listener).await,
        FrontListener::Tcp(listener) => door.serve_tcp(listener).await,
    }
}

/// The door of `pod --serve`: the rules choose the chain of each flow, the
/// first upstream carries UDP, and a plain `http://` request gets 405.
pub(crate) fn front_door(chain: Vec<Proxy>, rules: Vec<Rule>, verbose: bool) -> Door {
    let udp = Arc::new(PodUdp {
        chain: chain.clone(),
        verbose,
    });
    let dial = Arc::new(PodDial {
        chain,
        rules,
        verbose,
    });
    let door = Door::new(dial).with_extension(udp).connect_only();
    door.on_error(Arc::new(move |e: &anyhow::Error| {
        if verbose {
            eprintln!("{}: front door: {e:#}", crate::cli::program_name());
        }
    }))
}

/// Bind the listener of the front door. Return it with the bound address:
/// for TCP, the port that the kernel chose for port 0.
pub(crate) async fn bind_front_door(
    listen: &crate::listen::ProxyListen,
) -> Result<(FrontListener, crate::listen::ProxyListen)> {
    use crate::listen::ProxyListen;
    Ok(match listen {
        ProxyListen::Unix(path) => (
            FrontListener::Unix(crate::listen::bind_claimed(path)?),
            listen.clone(),
        ),
        ProxyListen::Tcp(addr) => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            let actual = listener.local_addr()?;
            (FrontListener::Tcp(listener), ProxyListen::Tcp(actual))
        }
    })
}

/// The way out of the pod door: the rules choose a chain (or a refusal),
/// the chain carries the flow, and the flow log (T-157) gets the decision
/// and the byte counts.
struct PodDial {
    chain: Vec<Proxy>,
    rules: Vec<Rule>,
    verbose: bool,
}

impl Dial for PodDial {
    fn label(&self) -> String {
        format!("pod:{} upstream", self.chain.len())
    }

    /// The upstream resolves a name: the door sends it.
    fn names_inside(&self) -> bool {
        true
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let text = target.to_string();
            let picked = select_chain(&self.rules, &self.chain, &text);
            let flow = crate::flowlog::decided("tcp", &text, &picked);
            let Some(selected) = picked.chain() else {
                crate::rules::note_refusal("front door", &text, picked.rule_name(), self.verbose);
                return Err(Failure::new(
                    Stage::Proxy,
                    Class::Policy,
                    format!("refused by the rule {}", picked.rule_name()),
                ));
            };
            if self.verbose {
                eprintln!(
                    "{}: front door {text} via {}",
                    crate::cli::program_name(),
                    selected[0].endpoint()
                );
            }
            let upstream = within_timeout(
                "front door connect",
                connect_upstream_async(selected, &text),
            )
            .await;
            match upstream {
                Ok(upstream) => Ok(Connected::new(crate::flowlog::counted(
                    upstream, flow, &text,
                ))),
                Err(e) => {
                    // The reply code follows the words of the outer text.
                    if self.verbose {
                        eprintln!("{}: front door: {e:#}", crate::cli::program_name());
                    }
                    Err(Failure::new(
                        Stage::Proxy,
                        Class::Unavailable,
                        e.to_string(),
                    ))
                }
            }
        })
    }
}

/// The SOCKS5 commands that the pod carries through its first upstream, and
/// the dialer cannot: UDP ASSOCIATE and UDP in the stream.
struct PodUdp {
    chain: Vec<Proxy>,
    verbose: bool,
}

impl Extension for PodUdp {
    fn command<'a>(
        &'a self,
        command: u8,
        control: &'a mut dyn Duplex,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            match command {
                0x03 => serve_udp_associate(control, &self.chain, self.verbose).await?,
                pod_proxy::udp::CMD_UDP_IN_STREAM => {
                    serve_udp_in_stream(control, &self.chain).await?
                }
                _ => return Ok(false),
            }
            Ok(true)
        })
    }
}

/// Answer a UDP association in the stream (issue 84) with the same
/// association of the first upstream: the frames are the same on both
/// connections, so the bytes are copied. An upstream that does not know the
/// command gives 0x07 to the client, which then can use `UDP ASSOCIATE`.
async fn serve_udp_in_stream<S>(control: &mut S, chain: &[Proxy]) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let refuse = [0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
    let proxy = match chain.first() {
        Some(proxy) if proxy.kind == ProxyKind::Socks5 => proxy,
        _ => {
            control.write_all(&refuse).await?;
            bail!("UDP in the stream needs a SOCKS5 upstream");
        }
    };
    let upstream = async {
        let up = dial_proxy(proxy).await?;
        crate::upstream::socks5_udp_in_stream(up, proxy).await
    };
    let mut up = match within_timeout("udp in the stream", upstream).await {
        Ok(up) => up,
        Err(err) => {
            let _ = control.write_all(&refuse).await;
            return Err(err);
        }
    };
    control
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;
    control.flush().await?;
    let _ = tokio::io::copy_bidirectional(control, &mut up).await;
    Ok(())
}

/// Answer `UDP ASSOCIATE` by relaying through the first upstream proxy's own
/// association, so a UDP relay can chain through this instance.
async fn serve_udp_associate<S>(control: &mut S, chain: &[Proxy], verbose: bool) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let proxy = chain.first().context("no upstream for UDP ASSOCIATE")?;
    if proxy.kind != ProxyKind::Socks5 {
        let _ = control
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await;
        bail!("UDP ASSOCIATE needs a SOCKS5 upstream");
    }
    // Our own relay socket, advertised to the client.
    let relay = Arc::new(tokio::net::UdpSocket::bind("0.0.0.0:0").await?);
    let addr = relay.local_addr()?;
    let mut bnd = vec![0x05u8, 0x00, 0x00];
    match addr {
        SocketAddr::V4(a) => {
            bnd.push(0x01);
            bnd.extend_from_slice(&a.ip().octets());
            bnd.extend_from_slice(&a.port().to_be_bytes());
        }
        SocketAddr::V6(a) => {
            bnd.push(0x04);
            bnd.extend_from_slice(&a.ip().octets());
            bnd.extend_from_slice(&a.port().to_be_bytes());
        }
    }
    control.write_all(&bnd).await?;
    control.flush().await?;

    // The upstream's relay, reached over its own association.
    let up_control = dial_proxy(proxy).await?;
    let (_up_control, up_addr) = socks5_udp_associate(up_control, proxy).await?;
    let up = Arc::new(tokio::net::UdpSocket::bind("0.0.0.0:0").await?);
    if verbose {
        eprintln!(
            "{}: front door UDP relay {addr} -> {up_addr}",
            crate::cli::program_name()
        );
    }

    let client: Arc<tokio::sync::Mutex<Option<SocketAddr>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    // client -> upstream
    let to_upstream = {
        let relay = relay.clone();
        let up = up.clone();
        let client = client.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while let Ok((n, from)) = relay.recv_from(&mut buf).await {
                *client.lock().await = Some(from);
                if n < 4 {
                    continue;
                }
                let _ = up.send_to(&buf[..n], up_addr).await;
            }
        })
    };
    // upstream -> client
    let to_client = {
        let relay = relay.clone();
        let up = up.clone();
        let client = client.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while let Ok((n, _)) = up.recv_from(&mut buf).await {
                if let Some(to) = *client.lock().await {
                    let _ = relay.send_to(&buf[..n], to).await;
                }
            }
        })
    };
    // Hold the control connection until the client closes it, then stop the
    // relay tasks rather than leaving them holding the sockets.
    let mut byte = [0u8; 1];
    while control.read(&mut byte).await.unwrap_or(0) == 1 {}
    to_upstream.abort();
    to_client.abort();
    Ok(())
}

#[cfg(test)]
#[path = "frontdoor_tests.rs"]
mod tests;
