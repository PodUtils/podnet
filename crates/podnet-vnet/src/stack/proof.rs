use super::*;

/// The steps and verdict of a userspace TCP handshake + transfer, run
/// entirely in-process.
///
/// The `kernel_af_inet_*` fields are **not** measurements. A userspace stack has
/// no way to observe its own syscalls, so the report says which link layer it
/// ran over rather than asserting a kernel call count it cannot see. The
/// original set the count to a literal zero and printed "kernel AF_INET
/// bind/connect used: none", which reads as a syscall trace and is not one: a
/// direct `AF_INET` connect added anywhere on this path would not change it.
#[derive(Clone, Debug)]
pub struct ProofReport {
    pub steps: Vec<String>,
    /// The states when the response arrived.
    pub server_exchange_state: String,
    pub client_exchange_state: String,
    /// The states after the teardown (the client closes first).
    pub server_state: String,
    pub client_state: String,
    /// The link layer the stack ran over: `"in-process loopback"` or `"a
    /// tunnel"`. The second would mean packets could reach a kernel socket.
    pub link_layer: String,
    pub bytes_received: usize,
}

impl ProofReport {
    /// The handshake and the exchange completed with both sockets
    /// ESTABLISHED, and the teardown ended them: the server (the passive
    /// closer) CLOSED, the client TIME-WAIT or CLOSED.
    pub fn passed(&self) -> bool {
        self.server_exchange_state == "ESTABLISHED"
            && self.client_exchange_state == "ESTABLISHED"
            && self.bytes_received > 0
            && self.server_state == "CLOSED"
            && matches!(self.client_state.as_str(), "TIME-WAIT" | "CLOSED")
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            out.push_str("  ");
            out.push_str(step);
            out.push('\n');
        }
        out.push_str("\n--- verdict ---\n");
        out.push_str(&format!("link layer: {}\n", self.link_layer));
        out.push_str(
            "note: this proves the stack ran in-process. It cannot observe its own\n\
             \x20     syscalls; tests/af_inet_kernel.rs measures the kernel side by socket inode.\n",
        );
        out.push_str(&format!(
            "states at the exchange: server {}, client {}\n",
            self.server_exchange_state, self.client_exchange_state
        ));
        out.push_str(&format!(
            "states after the teardown: server {}, client {}\n",
            self.server_state, self.client_state
        ));
        if self.passed() {
            out.push_str(
                "PROOF: userspace TCP handshake, request/response and teardown complete\n",
            );
        } else {
            out.push_str("FAIL: the handshake, the exchange or the teardown did not complete\n");
        }
        out
    }
}

/// Run the §9.1 proof with the crate's own device and no external link.
pub fn loopback_proof() -> ProofReport {
    use smoltcp::wire::Ipv4Address;

    let mut device = LinkDevice::loopback(1500);
    let mut config = IfaceConfig::new(HardwareAddress::Ip);
    config.random_seed = 0x1234_5678;
    let mut iface = Interface::new(config, &mut device, SmolInstant::ZERO);
    let ip = Ipv4Address::new(10, 66, 0, 2);
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(IpAddress::Ipv4(ip), 24));
    });
    let mk = || {
        tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; 8192]),
            tcp::SocketBuffer::new(vec![0u8; 8192]),
        )
    };
    let mut sockets: SocketSet<'static> = SocketSet::new(Vec::new());
    let server = sockets.add(mk());
    let client = sockets.add(mk());
    sockets
        .get_mut::<tcp::Socket>(server)
        .listen(IpListenEndpoint {
            addr: Some(IpAddress::Ipv4(ip)),
            port: 8080,
        })
        .unwrap();
    sockets
        .get_mut::<tcp::Socket>(client)
        .connect(
            iface.context(),
            IpEndpoint::new(IpAddress::Ipv4(ip), 8080),
            IpListenEndpoint {
                addr: Some(IpAddress::Ipv4(ip)),
                port: 49152,
            },
        )
        .unwrap();

    let mut steps = Vec::new();
    let mut request_sent = false;
    let mut response = Vec::new();
    let mut accepted = false;
    for tick in 1..=20_000i64 {
        let now = SmolInstant::from_millis(tick);
        let _ = iface.poll(now, &mut device, &mut sockets);

        {
            let socket = sockets.get_mut::<tcp::Socket>(server);
            if !accepted && socket.may_recv() {
                accepted = true;
                if let Some(remote) = socket.remote_endpoint() {
                    steps.push(format!(
                        "server ACCEPTED from {}:{}",
                        from_smoltcp(remote.addr),
                        remote.port
                    ));
                }
            }
            if socket.may_recv() {
                let mut buffer = [0u8; 512];
                if let Ok(n) = socket.recv_slice(&mut buffer) {
                    if n > 0 {
                        let got = String::from_utf8_lossy(&buffer[..n]).to_string();
                        steps.push(format!("server READ {n}B: {}", got.trim()));
                        let reply =
                            format!("HTTP/1.0 200 OK\r\n\r\nuserspace tcp says: {}", got.trim());
                        let _ = socket.send_slice(reply.as_bytes());
                        steps.push("server WROTE response".into());
                    }
                }
            }
        }
        {
            let socket = sockets.get_mut::<tcp::Socket>(client);
            if !request_sent && socket.may_send() {
                request_sent = true;
                let request = b"GET /hello HTTP/1.0\r\n\r\n";
                let n = socket.send_slice(request).unwrap();
                steps.push(format!(
                    "client WROTE {n}B request (state={})",
                    socket.state()
                ));
            }
            if socket.may_recv() {
                let mut buffer = [0u8; 512];
                if let Ok(n) = socket.recv_slice(&mut buffer) {
                    if n > 0 {
                        response.extend_from_slice(&buffer[..n]);
                        steps.push(format!("client READ {n}B"));
                    }
                }
            }
        }
        if response.windows(4).any(|window| window == b"\r\n\r\n") && response.len() > 20 {
            break;
        }
    }
    let server_exchange_state = sockets.get::<tcp::Socket>(server).state().to_string();
    let client_exchange_state = sockets.get::<tcp::Socket>(client).state().to_string();

    // The teardown: the client closes (FIN); the server closes when it sees
    // the end of the client's data. Before, the proof stopped here, and the
    // final states were ESTABLISHED (issue 53).
    sockets.get_mut::<tcp::Socket>(client).close();
    steps.push("client CLOSE (FIN)".into());
    let mut server_closed = false;
    let start = 20_001i64;
    for tick in start..start + 30_000 {
        let now = SmolInstant::from_millis(tick);
        let _ = iface.poll(now, &mut device, &mut sockets);
        let socket = sockets.get_mut::<tcp::Socket>(server);
        if !server_closed && !socket.may_recv() {
            server_closed = true;
            socket.close();
            steps.push(format!("server saw EOF; CLOSE (state={})", socket.state()));
        }
        let server_state = sockets.get::<tcp::Socket>(server).state();
        let client_state = sockets.get::<tcp::Socket>(client).state();
        if server_state == tcp::State::Closed
            && matches!(client_state, tcp::State::TimeWait | tcp::State::Closed)
        {
            steps.push(format!(
                "teardown done after {} ms: server {server_state}, client {client_state}",
                tick - start + 1
            ));
            break;
        }
    }
    let server_state = sockets.get::<tcp::Socket>(server).state().to_string();
    let client_state = sockets.get::<tcp::Socket>(client).state().to_string();
    ProofReport {
        steps,
        server_exchange_state,
        client_exchange_state,
        server_state,
        client_state,
        link_layer: "in-process loopback".to_string(),
        bytes_received: response.len(),
    }
}

/// A deterministic integration test of the full async stack: a listener and a
/// client in one loopback stack, with a real request/response.
pub async fn loopback_round_trip() -> io::Result<(usize, Vec<u8>)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut net = NetStack::loopback();
    let addr = VirtAddr::new(net::LOCAL_IP, 8080);
    let mut listener = net.listen(addr).await?;
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 1024];
        let n = stream.read(&mut buffer).await.unwrap();
        let body = format!(
            "HTTP/1.0 200 OK\r\n\r\nuserspace tcp says: {}",
            String::from_utf8_lossy(&buffer[..n]).trim()
        );
        stream.write_all(body.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        n
    });
    let mut client = net.connect(addr).await?;
    client.write_all(b"GET /hello HTTP/1.0\r\n\r\n").await?;
    client.flush().await?;
    let mut response = Vec::new();
    client.read_to_end(&mut response).await?;
    let request_len = server.await.unwrap();
    net.shutdown().await;
    Ok((request_len, response))
}
