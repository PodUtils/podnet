//! The service `socks5` of a stream of a link (T-291).
//!
//! The peer that opened the stream speaks SOCKS5 on it. This end runs the
//! server side of the front door ([`podnet_netns::door::Door`], the same
//! code as `podnet pod --serve`) with a direct dial: a `CONNECT` goes from
//! this host, and a name resolves with the resolver of this host. The
//! allow list of the listener has admitted the peer before the stream
//! opened. The door takes SOCKS5 only here: a first byte that is not 5
//! ends the stream. UDP is the command 0x83 of `pod-proxy` (UDP in the
//! stream): each datagram is a frame on the stream, and the datagrams leave
//! from a UDP socket of this host. `UDP ASSOCIATE` (command 3) gets the
//! reply 0x07. `BIND` (command 2) listens on a TCP port of this host (the
//! loopback, or the address of `--socks-bind-ip`, T-291): reply 1 gives the
//! address, one incoming connection ends the wait (30 s), reply 2 gives its
//! source, and the connection is copied to the stream. The opener needs no
//! second stream: RFC 1928 puts the incoming connection on the control
//! connection, which here is the stream. The local door and `--resolve
//! local` are in [`crate::link_door`].

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use pod_core::{BoxFuture, Class, Connected, Datagrams, Dial, Failure, Stage, Target};
use podnet_netns::door::{Door, Duplex, Extension};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::time::Instant;

/// The name of the service.
pub const NAME: &str = "socks5";

/// The way out of the service: a TCP connection from this host.
struct Direct;

impl Dial for Direct {
    fn label(&self) -> String {
        "link socks5".to_string()
    }

    /// The door sends the name: this host resolves it.
    fn names_inside(&self) -> bool {
        true
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let limit = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(30));
            let host = target.host();
            let work = tokio::net::TcpStream::connect((host.as_str(), target.port()));
            let fail = |text: String| Failure::new(Stage::Connect, Class::Unavailable, text);
            match tokio::time::timeout(limit, work).await {
                Err(_) => Err(fail(format!("no answer from {target} in time"))),
                Ok(Err(e)) => Err(fail(format!("connecting to {target}: {e}"))),
                Ok(Ok(tcp)) => {
                    let _ = tcp.set_nodelay(true);
                    let (local, peer) = (tcp.local_addr().ok(), tcp.peer_addr().ok());
                    let mut done = Connected::new(Box::new(tcp));
                    done.local = local;
                    done.peer = peer;
                    Ok(done)
                }
            }
        })
    }

    /// One UDP socket of this host (IPv4, any port).
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            let any = std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 0));
            let socket = crate::network::SystemDatagrams::bind(any)
                .await
                .map_err(|e| Failure::io(Stage::Connect, e))?;
            Ok(Arc::new(socket) as Arc<dyn Datagrams>)
        })
    }
}

/// The time that `BIND` waits for the incoming connection.
const BIND_WAIT: Duration = Duration::from_secs(30);

/// The commands of the service that the door does not take: `BIND` and the
/// refusal of command 3.
struct Commands {
    bind_ip: IpAddr,
}

impl Extension for Commands {
    fn command<'a>(
        &'a self,
        command: u8,
        control: &'a mut dyn Duplex,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            if command != 2 {
                return crate::exit_stdio::NoAssociate
                    .command(command, control)
                    .await;
            }
            bind(self.bind_ip, control).await?;
            Ok(true)
        })
    }
}

/// `BIND`: reply 1, wait for one connection, reply 2, then copy.
async fn bind(ip: IpAddr, mut control: &mut dyn Duplex) -> Result<()> {
    use podnet_netns::socks::socks5_reply;
    let listener = match tokio::net::TcpListener::bind((ip, 0)).await {
        Ok(listener) => listener,
        Err(_) => return socks5_reply(&mut control, 0x01, None).await,
    };
    socks5_reply(&mut control, 0x00, Some(listener.local_addr()?)).await?;
    let Ok(Ok((mut incoming, from))) = tokio::time::timeout(BIND_WAIT, listener.accept()).await
    else {
        // TTL expired: nobody connected in time.
        return socks5_reply(&mut control, 0x06, None).await;
    };
    drop(listener);
    let _ = incoming.set_nodelay(true);
    socks5_reply(&mut control, 0x00, Some(from)).await?;
    tokio::io::copy_bidirectional(control, &mut incoming).await?;
    Ok(())
}

/// Serve SOCKS5 on `stream` until the flow ends. The first byte must be 5.
/// `BIND` listens on the loopback.
pub async fn serve<S>(stream: S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    serve_with(stream, IpAddr::from([127, 0, 0, 1])).await
}

/// [`serve`], with `BIND` listening on `bind_ip`.
pub async fn serve_with<S>(stream: S, bind_ip: IpAddr) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut read, write) = tokio::io::split(stream);
    let mut first = [0u8; 1];
    read.read_exact(&mut first).await?;
    if !podnet_netns::socks::looks_like_socks5(first[0]) {
        bail!("the stream of the service {NAME} does not start with SOCKS5");
    }
    // The door reads the byte again: give it back in front of the stream.
    let again = tokio::io::join(std::io::Cursor::new(first).chain(read), write);
    // Command 3 would open a UDP socket here that the opener cannot reach.
    let door = Door::new(Arc::new(Direct))
        .connect_only()
        .with_extension(Arc::new(Commands { bind_ip }))
        .with_udp_in_stream();
    door.handle(again).await
}
