//! `podnet exit --stdio` (T-329): one SOCKS5 session on one stream.
//!
//! The stream is stdin and stdout of the process, or any stream of a test.
//! The server side is the front door ([`podnet_netns::door::Door`], the code
//! of `podnet pod --serve`), with no listener. `CONNECT` is the stream
//! itself. `UDP ASSOCIATE` needs a UDP socket next to the control stream,
//! which a pipe does not have, so the door takes the command 0x83 of
//! `pod-proxy` (UDP in the stream): each datagram is a frame on the stream
//! (see `docs/proxies.md`, "UDP in the stream"). The command 3 is refused
//! with the reply 0x07: its UDP socket would be on this host, and the
//! client cannot reach it.
//!
//! The way out is the way of the process (`--proxy`, `--no-proxy`, the
//! variables, `--egress`). TCP follows it. UDP leaves from this host when
//! the process has no proxy; with a proxy it is refused (reply 0x07), as a
//! SOCKS5 or HTTP proxy of the process carries no UDP here.

use std::sync::Arc;

use anyhow::Result;
use pod_core::{BoxFuture, Connected, Datagrams, Dial, Failure, Stage, Target};
use podnet_netns::door::{Door, Duplex, Extension};
use tokio::io::AsyncWriteExt as _;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;

use crate::egress::ProxyChoice;

/// The way out of the exit: TCP through the way of the process, UDP from
/// this host when the process has no proxy.
pub struct ExitDial {
    tcp: Arc<dyn Dial>,
    udp: Option<Arc<dyn Dial>>,
}

impl ExitDial {
    /// The exit over the way of the process.
    pub fn process() -> ExitDial {
        let proxied = match crate::egress::choice() {
            ProxyChoice::Via(_) => true,
            ProxyChoice::Direct => false,
            ProxyChoice::FromEnvironment => crate::egress::proxy_for("udp.invalid", 53).is_some(),
        };
        let udp = (!proxied).then(|| Arc::new(crate::dialers::Direct) as Arc<dyn Dial>);
        ExitDial {
            tcp: crate::dialers::process(),
            udp,
        }
    }

    /// An exit over `tcp`, with the datagrams of `udp` (a test).
    pub fn with(tcp: Arc<dyn Dial>, udp: Option<Arc<dyn Dial>>) -> ExitDial {
        ExitDial { tcp, udp }
    }
}

impl Dial for ExitDial {
    fn label(&self) -> String {
        format!("exit {}", self.tcp.label())
    }

    fn names_inside(&self) -> bool {
        self.tcp.names_inside()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.tcp.dial(target, deadline)
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        match &self.udp {
            Some(udp) => udp.datagrams(),
            None => Box::pin(async move {
                Err(Failure::new(
                    Stage::Connect,
                    pod_core::Class::Unavailable,
                    "the way out of this process carries no UDP",
                ))
            }),
        }
    }
}

/// The refusal of `UDP ASSOCIATE` (command 3) on a stream.
pub(crate) struct NoAssociate;

impl Extension for NoAssociate {
    fn command<'a>(
        &'a self,
        command: u8,
        control: &'a mut dyn Duplex,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            if command != 3 {
                return Ok(false);
            }
            control
                .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            control.flush().await?;
            Ok(true)
        })
    }
}

/// How long `exit --stdio` waits for the target to send data or close after
/// the client closes stdin (T-621).
pub const DRAIN_LIMIT: std::time::Duration = std::time::Duration::from_secs(1);

/// Serve one SOCKS5 session on `stream` over `dial` until it ends.
pub async fn serve<S>(stream: S, dial: Arc<dyn Dial>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let door = Door::new(dial)
        .connect_only()
        .with_extension(Arc::new(NoAssociate))
        .with_udp_in_stream()
        .with_client_eof_limit(DRAIN_LIMIT);
    door.handle(stream).await
}

#[cfg(test)]
#[path = "exit_stdio_tests.rs"]
mod tests;

/// T-621 (GitHub #787): after stdin closes, `exit --stdio` shuts the write
/// side of the target and ends cleanly even when the target never writes or
/// closes; a target that answers after stdin closes still delivers its full
/// answer.
#[cfg(test)]
#[tokio::test]
async fn the_run_ends_when_stdin_closes_and_the_target_stays_open() {
    crate::test_host::need_loopback!();
    use std::time::Duration;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    // Case 1: target accepts and never writes or closes.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    let (hold_tx, hold_rx) = tokio::sync::oneshot::channel::<()>();
    let (saw_eof_tx, saw_eof_rx) = tokio::sync::oneshot::channel::<bool>();
    tokio::spawn(async move {
        let (mut s, _) = silent.accept().await.unwrap();
        let mut buf = [0u8; 16];
        let eof = matches!(s.read(&mut buf).await, Ok(0));
        let _ = saw_eof_tx.send(eof);
        let _ = hold_rx.await;
        drop(s);
    });

    let (client, stream) = tokio::io::duplex(4096);
    let (mut cr, mut cw) = tokio::io::split(client);
    let dial = Arc::new(ExitDial::with(Arc::new(crate::dialers::Direct), None));
    let served = tokio::spawn(serve(stream, dial.clone()));

    cw.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    cr.read_exact(&mut method).await.unwrap();
    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend_from_slice(&silent_port.to_be_bytes());
    cw.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    cr.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "{reply:?}");

    // Client closes stdin while stdout stays open.
    cw.shutdown().await.unwrap();

    let saw_eof = tokio::time::timeout(Duration::from_secs(3), saw_eof_rx)
        .await
        .expect("target did not observe write shutdown")
        .unwrap();
    assert!(saw_eof, "target must see EOF after client closes stdin");

    let finished = tokio::time::timeout(Duration::from_secs(4), served)
        .await
        .expect("exit --stdio did not end after stdin closed against a silent target")
        .unwrap();
    assert!(finished.is_ok(), "expected Ok(()), got {finished:?}");
    let _ = hold_tx.send(());

    // Case 2: normal half-close — client sends request and closes stdin,
    // target answers after seeing EOF and then closes.
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = echo.accept().await.unwrap();
        let mut req_data = Vec::new();
        s.read_to_end(&mut req_data).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        s.write_all(b"reply:").await.unwrap();
        s.write_all(&req_data).await.unwrap();
        s.shutdown().await.unwrap();
    });

    let (client, stream) = tokio::io::duplex(4096);
    let (mut cr, mut cw) = tokio::io::split(client);
    let served = tokio::spawn(serve(stream, dial));

    cw.write_all(&[5, 1, 0]).await.unwrap();
    cr.read_exact(&mut method).await.unwrap();
    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend_from_slice(&echo_port.to_be_bytes());
    cw.write_all(&req).await.unwrap();
    cr.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "{reply:?}");

    cw.write_all(b"hello").await.unwrap();
    cw.shutdown().await.unwrap();

    let mut out = Vec::new();
    cr.read_to_end(&mut out).await.unwrap();
    assert_eq!(&out, b"reply:hello");
    let finished = tokio::time::timeout(Duration::from_secs(4), served)
        .await
        .expect("exit --stdio did not end after half-closed response")
        .unwrap();
    assert!(finished.is_ok(), "expected Ok(()), got {finished:?}");
}
