//! A SOCKS5 / HTTP `CONNECT` front door into the virtual network.
//!
//! Programs that cannot be interposed — a statically linked binary, a language
//! runtime that resolves its own network stack — still honour `ALL_PROXY`.
//! [`handle_connection`] speaks the client side of SOCKS5 and HTTP `CONNECT`
//! and dials the requested endpoint through a [`NetStack`].
//!
//! A virtual endpoint only exists while something listens on it. [`forward`]
//! supplies that: it binds a virtual listener and splices every accepted
//! connection to a real destination reached through the host, so a fixed
//! virtual address can name a service that actually exists:
//!
//! ```text
//! ALL_PROXY=socks5h:///tmp/podnet-vnet.socks
//! curl http://10.66.0.2:8080/        # -> 127.0.0.1:3000
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use podnet_core::network::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use podnet_netns::door::Door;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::addr::VirtAddr;
use crate::socks::{self, ProxyHost};
use crate::stack::NetStack;

pub use podnet_netns::listen::ProxyListen;

/// A destination table for names the virtual network cannot resolve.
pub type Resolver = HashMap<String, IpAddr>;

/// Turn a parsed request into a virtual endpoint.
pub fn resolve_target(
    request: &socks::ProxyRequest,
    resolver: &Resolver,
    default_port: u16,
) -> Result<VirtAddr> {
    let port = if request.port == 0 {
        default_port
    } else {
        request.port
    };
    match &request.host {
        ProxyHost::Ip(ip) => Ok(VirtAddr::new(*ip, port)),
        ProxyHost::Domain(name) => {
            let key = name.to_ascii_lowercase();
            if let Some(ip) = resolver.get(&key) {
                return Ok(VirtAddr::new(*ip, port));
            }
            // SOCKS5' domain form is also how some clients send an address
            // literal, so a name that parses as one is used directly rather
            // than reported as unresolvable.
            if let Ok(ip) = key.trim_start_matches('[').trim_end_matches(']').parse() {
                return Ok(VirtAddr::new(ip, port));
            }
            Err(anyhow::anyhow!("no virtual address for {name:?}"))
        }
    }
}

pub use crate::stack::TcpStream;

/// How long the copy of one proxied client connection may take. It does not
/// bound the opening: `REQUEST_LIMIT` bounds that.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(300);

/// The time limit of the handshake with a virtual address. A SYN to an
/// address that nothing owns gets no answer; the client gets 504 after this.
pub const CONNECT_LIMIT: Duration = Duration::from_secs(5);

/// The way out of `net proxy`: a TCP flow to an address of the virtual
/// network, with the names of the table.
struct StackDial {
    net: Arc<NetStack>,
    resolver: Resolver,
}

impl Dial for StackDial {
    fn label(&self) -> String {
        "vnet".into()
    }

    /// The table resolves the name, so the door sends it.
    fn names_inside(&self) -> bool {
        true
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let request = match target {
                Target::Addr(addr) => socks::ProxyRequest {
                    host: ProxyHost::Ip(addr.ip()),
                    port: addr.port(),
                },
                Target::Name { host, port } => socks::ProxyRequest {
                    host: ProxyHost::Domain(host.clone()),
                    port: *port,
                },
            };
            let to = resolve_target(&request, &self.resolver, 0)
                .map_err(|e| Failure::new(Stage::Dns, Class::NotFound, e.to_string()))?;
            // `net.connect` returns as soon as the socket exists and the SYN is queued,
            // which is not the same as the connection being established. The client has
            // already sent its request and is waiting, so if nothing is listening it
            // waits for data that will never arrive. Refusing instead means the client
            // gets a SOCKS5 error or a 502, which it can act on.
            let mut remote = self
                .net
                .connect(to)
                .await
                .map_err(|e| Failure::new(Stage::Connect, Class::Unavailable, e.to_string()))?;
            wait_established(&mut remote).await?;
            Ok(Connected::new(Box::new(remote)))
        })
    }
}

/// Wait for the virtual connection to be established, or report why it
/// failed: a refusal (502) or no answer (504, a timeout).
async fn wait_established(stream: &mut TcpStream) -> Result<(), Failure> {
    let peer = stream.peer_addr();
    match tokio::time::timeout(CONNECT_LIMIT, stream.established()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(Failure::new(
            Stage::Connect,
            Class::Unavailable,
            format!("{peer} refused the connection: {err}"),
        )),
        Err(_) => Err(Failure::new(
            Stage::Connect,
            Class::Timeout,
            format!(
                "no answer from {peer} in {} s: no virtual host has this address",
                CONNECT_LIMIT.as_secs()
            ),
        )),
    }
}

/// The front door of `net proxy` over `net`: SOCKS5 and HTTP `CONNECT` on
/// one listener (the protocol is detected from the first byte), each flow
/// dialed through the stack. A plain `http://` request gets 405. A port 0
/// takes `default_port`. A flow ends after [`CLIENT_TIMEOUT`]: without it a
/// client that opens a connection and then stops reading holds a task and
/// both sockets indefinitely.
pub fn door(net: Arc<NetStack>, resolver: Resolver, default_port: u16) -> Door {
    Door::new(Arc::new(StackDial { net, resolver }))
        .with_default_port(default_port)
        .with_flow_limit(CLIENT_TIMEOUT)
        .connect_only()
}

/// Serve one proxy client: negotiate, dial `net`, and splice both directions.
pub async fn handle_connection<S>(
    stream: S,
    net: Arc<NetStack>,
    resolver: Resolver,
    default_port: u16,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    if let Err(err) = door(net, resolver, default_port).handle(stream).await {
        tracing::debug!("a client of the proxy: {err:#}");
    }
}

/// Bind a virtual listener and splice each accepted connection to a real
/// destination. `dial` is called once per connection, so it can open a fresh
/// real socket; the test suite passes a duplex instead.
///
/// The bind is done when this returns. A refusal of the stack (an address
/// outside the subnet: `AddrNotAvailable`; a second bind: `AddrInUse`; the
/// ACL: `PermissionDenied`) is the error.
pub async fn forward<F, Fut, S>(
    net: Arc<NetStack>,
    virt: VirtAddr,
    dial: F,
) -> std::io::Result<JoinHandle<()>>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = std::io::Result<S>> + Send + 'static,
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut listener = net.listen(virt).await?;
    tracing::info!(%virt, "virtual forward listening");
    Ok(tokio::spawn(async move {
        while let Ok(mut virtual_stream) = listener.accept().await {
            let dialed = dial();
            tokio::spawn(async move {
                match dialed.await {
                    Ok(mut real) => {
                        let _ = tokio::io::copy_bidirectional(&mut virtual_stream, &mut real).await;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "virtual forward could not reach the real service");
                        // A reset is closer to the truth than a silent close.
                        virtual_stream.abort();
                    }
                }
            });
        }
    }))
}

/// The running proxy: the accept loop plus every forward task.
pub struct Proxy {
    pub listen: ProxyListen,
    accepts: JoinHandle<()>,
    forwards: Vec<JoinHandle<()>>,
}

impl Proxy {
    /// Start forwarding the configured virtual endpoints.
    pub async fn spawn_forwards<F, Fut, S>(
        &mut self,
        net: Arc<NetStack>,
        virt: VirtAddr,
        dial: F,
    ) -> std::io::Result<()>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::io::Result<S>> + Send + 'static,
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.forwards.push(forward(net, virt, dial).await?);
        Ok(())
    }

    /// Stop the accept loop and every forward.
    pub fn abort(self) {
        self.accepts.abort();
        for task in self.forwards {
            task.abort();
        }
    }
}

/// Bind the client-facing listener and start accepting.
pub async fn serve(
    net: Arc<NetStack>,
    listen: ProxyListen,
    resolver: Resolver,
    default_port: u16,
) -> Result<Proxy> {
    let mut listen = listen;
    let door = Arc::new(door(net, resolver, default_port));
    let accepts = match &listen {
        ProxyListen::Tcp(bind) => {
            let listener = podnet_core::net::bind_tcp(*bind).await?;
            // Issue 300: keep the bound address (the port that the kernel
            // chose for port 0), not the requested one.
            listen = ProxyListen::Tcp(listener.local_addr()?);
            tokio::spawn(async move {
                let _ = door.serve_tcp(listener).await;
            })
        }
        ProxyListen::Unix(path) => {
            #[cfg(unix)]
            {
                let listener = bind_unix(path)?;
                tokio::spawn(async move {
                    let _ = door.serve_unix(listener).await;
                })
            }
            #[cfg(not(unix))]
            {
                let _ = (door, path);
                bail!("unix sockets are not supported on this platform");
            }
        }
    };
    Ok(Proxy {
        listen,
        accepts,
        forwards: Vec::new(),
    })
}

#[cfg(unix)]
fn bind_unix(path: &std::path::Path) -> Result<tokio::net::UnixListener> {
    podnet_core::net::claim_socket_path(path, false)?;
    podnet_netns::listen::bind_private(path)
        .with_context(|| format!("binding unix:{}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// An echo server on the far end of a duplex, returned as the `dial`
    /// closure shape.
    async fn echo(stream: tokio::io::DuplexStream) {
        let (mut read, mut write) = tokio::io::split(stream);
        let _ = tokio::io::copy(&mut read, &mut write).await;
    }

    #[test]
    fn a_domain_form_literal_resolves_to_itself() {
        // Some SOCKS5 clients send an address literal as a domain name. A name
        // that parses as an address is used directly instead of being reported
        // as unresolvable, which is what a `socks5h` client can produce.
        let request = socks::ProxyRequest {
            host: ProxyHost::Domain("10.66.0.2".into()),
            port: 8080,
        };
        let target = resolve_target(&request, &Resolver::new(), 80).unwrap();
        assert_eq!(target, VirtAddr::new("10.66.0.2".parse().unwrap(), 8080));

        let named = socks::ProxyRequest {
            host: ProxyHost::Domain("web".into()),
            port: 0,
        };
        let mut resolver = Resolver::new();
        resolver.insert("web".into(), "10.66.0.9".parse().unwrap());
        let target = resolve_target(&named, &resolver, 80).unwrap();
        assert_eq!(target, VirtAddr::new("10.66.0.9".parse().unwrap(), 80));

        let unknown = socks::ProxyRequest {
            host: ProxyHost::Domain("nowhere".into()),
            port: 80,
        };
        assert!(resolve_target(&unknown, &Resolver::new(), 80).is_err());
    }

    /// One HTTP request through the proxy: the whole answer and its time.
    async fn ask(net: Arc<NetStack>, request: &[u8]) -> (String, std::time::Duration) {
        let (mut client, server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            handle_connection(server, net, Resolver::new(), 80).await;
        });
        let start = std::time::Instant::now();
        client.write_all(request).await.unwrap();
        let mut answer = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.read_to_end(&mut answer),
        )
        .await
        .expect("the proxy did not end the answer in 10 s")
        .unwrap();
        (
            String::from_utf8_lossy(&answer).into_owned(),
            start.elapsed(),
        )
    }

    /// Issue 41: a closed virtual port gets 502 with a body at once, an
    /// address that nothing owns gets 504 after CONNECT_LIMIT (not a hang),
    /// and a plain GET gets 405 with a body that names CONNECT.
    #[tokio::test]
    async fn refusals_have_a_status_and_a_body() {
        let net = Arc::new(NetStack::loopback());
        let (answer, took) = ask(net.clone(), b"CONNECT 10.66.0.2:81 HTTP/1.1\r\n\r\n").await;
        assert!(
            answer.starts_with("HTTP/1.1 502 Bad Gateway\r\n"),
            "{answer}"
        );
        assert!(
            answer.contains("10.66.0.2:81 refused the connection"),
            "{answer}"
        );
        assert!(took < std::time::Duration::from_secs(1), "{took:?}");

        let (answer, took) = ask(net.clone(), b"CONNECT 10.66.0.9:80 HTTP/1.1\r\n\r\n").await;
        assert!(
            answer.starts_with("HTTP/1.1 504 Gateway Timeout\r\n"),
            "{answer}"
        );
        assert!(answer.contains("no answer from 10.66.0.9:80"), "{answer}");
        assert!(
            took >= CONNECT_LIMIT && took < CONNECT_LIMIT * 2,
            "{took:?}"
        );

        let (answer, _) = ask(
            net.clone(),
            b"GET http://10.66.0.2:8080/ HTTP/1.1\r\nHost: 10.66.0.2\r\n\r\n",
        )
        .await;
        assert!(
            answer.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
            "{answer}"
        );
        assert!(answer.contains("HTTP CONNECT only"), "{answer}");
    }

    /// T-061: the proxy is the front door over the stack. A client that
    /// asks for UDP ASSOCIATE (the stack of the proxy carries no datagrams
    /// for it) gets "command not supported", not a closed connection; and
    /// a request with port 0 takes the port of the proxy.
    #[tokio::test]
    async fn the_proxy_is_the_door_over_the_stack() {
        let net = Arc::new(NetStack::loopback());
        let (mut client, server) = tokio::io::duplex(4096);
        let n = net.clone();
        tokio::spawn(async move { handle_connection(server, n, Resolver::new(), 80).await });
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        client
            .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0x07, "{reply:?}");

        let virt: VirtAddr = "10.66.0.2:8080".parse().unwrap();
        let handle = forward(net.clone(), virt, || async {
            let (real, far) = tokio::io::duplex(4096);
            tokio::spawn(echo(far));
            Ok(real)
        })
        .await
        .unwrap();
        let (mut client, server) = tokio::io::duplex(4096);
        tokio::spawn(async move { handle_connection(server, net, Resolver::new(), 8080).await });
        client.write_all(&[5, 1, 0]).await.unwrap();
        client.read_exact(&mut method).await.unwrap();
        client
            .write_all(&[5, 1, 0, 1, 10, 66, 0, 2, 0, 0])
            .await
            .unwrap();
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0, "port 0 takes the default port: {reply:?}");
        handle.abort();
    }

    #[tokio::test]
    async fn socks5_to_virtual_echo() {
        let net = Arc::new(NetStack::loopback());
        let virt: VirtAddr = "10.66.0.2:8080".parse().unwrap();
        let handle = forward(net.clone(), virt, || async {
            let (real, far) = tokio::io::duplex(4096);
            tokio::spawn(echo(far));
            Ok(real)
        })
        .await
        .unwrap();

        let (mut client, server) = tokio::io::duplex(4096);
        let proxy = {
            let net = net.clone();
            tokio::spawn(async move {
                handle_connection(server, net, Resolver::new(), 80).await;
            })
        };
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
        client
            .write_all(&[5, 1, 0, 1, 10, 66, 0, 2, 0x1f, 0x90])
            .await
            .unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0, "SOCKS5 CONNECT should succeed");
        client.write_all(b"ping").await.unwrap();
        let mut back = [0u8; 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.read_exact(&mut back),
        )
        .await
        .expect("echo timed out")
        .unwrap();
        assert_eq!(&back, b"ping");
        drop(client);
        let _ = proxy.await;
        handle.abort();
    }

    #[tokio::test]
    async fn names_resolve_through_the_table() {
        let net = Arc::new(NetStack::loopback());
        let virt: VirtAddr = "10.66.0.2:8080".parse().unwrap();
        let handle = forward(net.clone(), virt, || async {
            let (real, far) = tokio::io::duplex(4096);
            tokio::spawn(echo(far));
            Ok(real)
        })
        .await
        .unwrap();

        let mut resolver = Resolver::new();
        resolver.insert("api".into(), "10.66.0.2".parse().unwrap());

        let (mut client, server) = tokio::io::duplex(4096);
        let proxy = {
            let net = net.clone();
            tokio::spawn(async move { handle_connection(server, net, resolver, 80).await })
        };
        // HTTP CONNECT to the name on the forwarded port.
        client
            .write_all(b"CONNECT api:8080 HTTP/1.1\r\nHost: api\r\n\r\n")
            .await
            .unwrap();
        let mut response = [0u8; 39];
        client.read_exact(&mut response).await.unwrap();
        assert!(
            response.starts_with(b"HTTP/1.1 200"),
            "{}",
            String::from_utf8_lossy(&response)
        );
        client.write_all(b"hello").await.unwrap();
        let mut back = [0u8; 5];
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.read_exact(&mut back),
        )
        .await
        .expect("echo timed out")
        .unwrap();
        assert_eq!(&back, b"hello");
        drop(client);
        let _ = proxy.await;
        handle.abort();
    }

    async fn duplex_dial() -> std::io::Result<tokio::io::DuplexStream> {
        let (real, far) = tokio::io::duplex(64);
        tokio::spawn(echo(far));
        Ok(real)
    }

    #[tokio::test]
    async fn a_forward_outside_the_subnet_returns_the_error() {
        let net = Arc::new(NetStack::loopback());
        let virt: VirtAddr = "192.168.9.9:80".parse().unwrap();
        let err = forward(net, virt, duplex_dial).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrNotAvailable);
    }

    #[tokio::test]
    async fn a_second_forward_of_one_address_returns_addr_in_use() {
        let net = Arc::new(NetStack::loopback());
        let virt: VirtAddr = "10.66.0.2:8080".parse().unwrap();
        let first = forward(net.clone(), virt, duplex_dial).await.unwrap();
        let err = forward(net, virt, duplex_dial).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        first.abort();
    }
}
