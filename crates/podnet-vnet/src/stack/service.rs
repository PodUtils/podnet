use super::*;

impl Core {
    /// Move data between smoltcp and the application, adopt inbound
    /// connections, and reap closed sockets.
    pub(super) fn service(&mut self) {
        self.accept_inbound();
        self.service_conns();
        self.service_udp();
        self.service_dns();
    }

    /// Answer datagrams that arrive at the gateway's port 53 from the virtual
    /// resolver. An unknown name with no upstream is answered `NXDOMAIN` by the
    /// resolver; a name configured for upstream forwarding is dropped here (the
    /// host process owns the real forwarding path).
    pub(super) fn service_dns(&mut self) {
        let Some(service) = self.dns.as_ref() else {
            return;
        };
        let handle = service.handle;
        let resolver = service.resolver.clone();
        loop {
            let socket = self.sockets.get_mut::<udp::Socket>(handle);
            if !socket.can_recv() {
                break;
            }
            let mut buffer = [0u8; 1232];
            let (n, metadata) = match socket.recv_slice(&mut buffer) {
                Ok(received) => received,
                Err(_) => break,
            };
            let Some(response) = resolver.handle(&buffer[..n]) else {
                continue;
            };
            let socket = self.sockets.get_mut::<udp::Socket>(handle);
            let _ = socket.send_slice(&response, metadata.endpoint);
        }
    }

    pub(super) fn accept_inbound(&mut self) {
        let events = self.events.clone();
        let mut adopted: Vec<(usize, SocketHandle, VirtAddr, VirtAddr)> = Vec::new();
        for (index, listener) in self.listeners.iter().enumerate() {
            for handle in &listener.handles {
                let socket = self.sockets.get::<tcp::Socket>(*handle);
                if socket.may_recv() {
                    if let Some(remote) = socket.remote_endpoint() {
                        adopted.push((index, *handle, listener.addr, endpoint_to_virt(remote)));
                    }
                }
            }
        }
        for (index, handle, local, remote) in adopted {
            if let Some(listener) = self.listeners.get_mut(index) {
                listener.handles.retain(|candidate| *candidate != handle);
            }
            let (to_app_tx, to_app_rx) = mpsc::channel(64);
            let (from_app_tx, from_app_rx) = mpsc::channel(64);
            let (ctl_tx, ctl_rx) = mpsc::channel(16);
            let write_waker = Arc::new(AtomicWaker::new());
            let id = self.next_id;
            self.next_id += 1;
            let stream = TcpStream {
                id,
                local,
                remote,
                read_rx: to_app_rx,
                write_tx: from_app_tx,
                write_waker: write_waker.clone(),
                ctl_tx,
                kick: self.kick.clone(),
                read_buf: Vec::new(),
                read_pos: 0,
                eof: false,
                nodelay: false,
            };
            self.conns.insert(
                id,
                Conn {
                    handle,
                    to_app: to_app_tx,
                    from_app: from_app_rx,
                    write_waker,
                    ctl: ctl_rx,
                    closing: false,
                    recv_eof_sent: false,
                    announced: false,
                    pending: Vec::new(),
                },
            );
            let addr = self
                .listeners
                .get(index)
                .map(|listener| listener.addr)
                .unwrap_or(local);
            match self.spawn_listener_socket(addr) {
                Ok(replacement) => {
                    if let Some(listener) = self.listeners.get_mut(index) {
                        listener.handles.push(replacement);
                    }
                }
                Err(err) => tracing::warn!(error = %err, "could not replenish the listener pool"),
            }
            let queued = self
                .listeners
                .get_mut(index)
                .map(|listener| listener.accept_tx.try_send(stream).is_ok());
            if queued != Some(true) {
                tracing::warn!("inbound connection dropped: accept queue is full");
                // Free the socket too. The connection is already established,
                // so leaving it in the SocketSet leaks its buffers and holds a
                // half-open connection from the peer's point of view, since
                // nothing will ever service it.
                if let Some(dropped) = self.conns.remove(&id) {
                    self.sockets.remove(dropped.handle);
                    self.socket_count.remove();
                }
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let _ = events.send(StackEvent::Accepted { id, local, remote });
            self.stats.accepted.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn service_conns(&mut self) {
        let ids: Vec<u32> = self.conns.keys().copied().collect();
        let mut reap: Vec<(u32, Option<String>)> = Vec::new();
        for id in ids {
            let Some(conn) = self.conns.get_mut(&id) else {
                continue;
            };
            let mut reset_reason: Option<String> = None;
            let mut closed = false;

            // Control commands.
            loop {
                match conn.ctl.try_recv() {
                    // A close request marks the connection for finishing.
                    // Whether the socket closes on this pass or waits is
                    // decided after the queues are drained below, because a
                    // handle dropped while a reply is still in flight must not
                    // cut the exchange short.
                    //
                    // This is the only path by which a dropped handle is noticed
                    // at all: `Drop` on `TcpStream` sends this, and the reactor
                    // watches `to_app.is_closed()`, which runs the other way and
                    // stays open after the application goes.
                    Ok(ConnCmd::Close) => {
                        conn.closing = true;
                    }
                    Ok(ConnCmd::Abort) => {
                        self.sockets.get_mut::<tcp::Socket>(conn.handle).abort();
                        reset_reason = Some("connection aborted".into());
                        closed = true;
                    }
                    Ok(ConnCmd::SetNodelay(enabled)) => {
                        self.sockets
                            .get_mut::<tcp::Socket>(conn.handle)
                            .set_nagle_enabled(!enabled);
                    }
                    Err(_) => break,
                }
            }
            if closed {
                reap.push((id, reset_reason));
                continue;
            }

            // App -> stack.
            let mut wrote = false;
            loop {
                let socket = self.sockets.get_mut::<tcp::Socket>(conn.handle);
                if !socket.may_send() {
                    break;
                }
                if conn.pending.is_empty() {
                    match conn.from_app.try_recv() {
                        Ok(data) => conn.pending = data,
                        Err(_) => break,
                    }
                }
                match socket.send_slice(&conn.pending) {
                    Ok(0) => break,
                    Ok(n) => {
                        conn.pending.drain(..n);
                        wrote = true;
                    }
                    Err(_) => break,
                }
            }
            // stack -> app, but only while the app is reading.
            let socket = self.sockets.get_mut::<tcp::Socket>(conn.handle);
            while socket.may_recv() {
                let mut buffer = [0u8; 4096];
                match socket.recv_slice(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let packet = buffer[..n].to_vec();
                        if conn.to_app.try_send(AppEvent::Data(packet)).is_err() {
                            // Backpressure or a gone reader. A gone reader is
                            // detected below; a full one just waits.
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // A reader that has gone away closes the socket.
            if conn.to_app.is_closed() {
                self.sockets.get_mut::<tcp::Socket>(conn.handle).close();
                conn.closing = true;
            }
            let (state, send_queue, recv_queue, open) = {
                let socket = self.sockets.get::<tcp::Socket>(conn.handle);
                (
                    socket.state(),
                    socket.send_queue(),
                    socket.recv_queue(),
                    socket.is_open(),
                )
            };
            if !conn.announced
                && matches!(
                    state,
                    tcp::State::Established
                        | tcp::State::CloseWait
                        | tcp::State::FinWait1
                        | tcp::State::FinWait2
                )
            {
                conn.announced = true;
                let _ = conn.to_app.try_send(AppEvent::Connected);
            }
            // The remote half-closed: tell the app once, but keep the write
            // side open until the app closes it.
            if state == tcp::State::CloseWait && recv_queue == 0 && !conn.recv_eof_sent {
                conn.recv_eof_sent = true;
                let _ = conn.to_app.try_send(AppEvent::Eof);
            }
            // A close request closes the socket only once nothing is still owed in
            // either direction, so a handle dropped mid-exchange does not
            // discard the reply that is on its way. This is the decision
            // `TcpStream::drop` cannot make for itself: it does not know whether
            // the reactor is holding data it still owes.
            //
            // The flag is persistent, not per-poll: a close that arrives while
            // the send queue still holds the reply is deferred, and the check
            // runs again on the next pass. A per-poll local forgets the request
            // after one pass, no FIN is ever sent, and a peer reading until EOF
            // waits forever.
            //
            // Before the handshake completes, a close must wait too: smoltcp
            // closes a SYN-SENT socket at once, which resets the connection
            // and loses the data that the app wrote first.
            let handshake_done = !matches!(
                state,
                tcp::State::SynSent | tcp::State::SynReceived | tcp::State::Listen
            );
            if conn.closing
                && handshake_done
                && send_queue == 0
                && recv_queue == 0
                && conn.pending.is_empty()
                && conn.from_app.is_empty()
            {
                self.sockets.get_mut::<tcp::Socket>(conn.handle).close();
            }
            // A socket in TIME_WAIT is finished for this stack's purposes: it
            // holds no buffers of anything and smoltcp will never move it again,
            // because the segment that ends a TIME_WAIT is sent by the socket
            // that closes last, which is the peer. Keeping it resident means a
            // short-lived connection leaves a corpse behind, and a workload that
            // makes many of them grows without bound. `TimeWait` is therefore
            // treated as closed here even though it is not `Closed`.
            let closed_now = !open
                || matches!(
                    state,
                    tcp::State::Closed | tcp::State::TimeWait | tcp::State::Closing
                );
            if wrote {
                // The app had data waiting on a full channel; it can try again.
                conn.write_waker.wake();
            }
            if closed_now {
                reap.push((id, None));
            }
        }
        for (id, reason) in reap {
            if let Some(conn) = self.conns.remove(&id) {
                if let Some(reason) = reason {
                    let _ = conn.to_app.try_send(AppEvent::Reset(reason));
                } else {
                    let _ = conn.to_app.try_send(AppEvent::Eof);
                }
                // Free the smoltcp socket. A SocketSet does not free a slot
                // itself: without `remove`, each closed connection keeps its two
                // buffers for the life of the process.
                self.sockets.remove(conn.handle);
                self.socket_count.remove();
                self.emit(StackEvent::Closed { id });
                self.stats.closed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn service_udp(&mut self) {
        let ids: Vec<u32> = self.udp.keys().copied().collect();
        let events = self.events.clone();
        for id in ids {
            let Some(conn) = self.udp.get_mut(&id) else {
                continue;
            };
            // App -> stack.
            while let Ok((data, remote)) = conn.from_app.try_recv() {
                let in_subnet = self.config.subnet.contains(&remote.ip);
                let allowed = self
                    .config
                    .acl
                    .as_ref()
                    .map(|acl| acl.allows(&remote.ip, remote.port))
                    .unwrap_or(true);
                if !in_subnet || !allowed {
                    self.stats.refused.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let socket = self.sockets.get_mut::<udp::Socket>(conn.handle);
                if socket
                    .send_slice(&data, IpEndpoint::new(to_smoltcp(remote.ip), remote.port))
                    .is_err()
                {
                    break;
                }
                let _ = events.send(StackEvent::Datagram {
                    from: conn.local,
                    to: remote,
                    len: data.len(),
                });
                self.stats.datagrams.fetch_add(1, Ordering::Relaxed);
            }
            // stack -> app.
            loop {
                let socket = self.sockets.get_mut::<udp::Socket>(conn.handle);
                if !socket.can_recv() {
                    break;
                }
                let mut buffer = [0u8; 65535];
                match socket.recv_slice(&mut buffer) {
                    Ok((n, metadata)) => {
                        let from = endpoint_to_virt(metadata.endpoint);
                        let data = buffer[..n].to_vec();
                        if conn.to_app.try_send((data, from)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
}
