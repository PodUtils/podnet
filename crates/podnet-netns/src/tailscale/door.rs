//! The tailnet front door (T-061): the door of [`crate::door`] over a dialer
//! of `tailscaled`. The door reads SOCKS5 and HTTP `CONNECT`, gives port 0
//! the port of `--port`, refuses a plain `http://` request, and ends a flow
//! after [`CLIENT_TIMEOUT`]. The dialer adds the guard of the tailnet
//! (T-243), the time limit of the dial (issue 181) and the notes of a
//! failed dial (T-081).

use std::io;
use std::sync::Arc;
use std::time::Duration;

use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use tokio::time::Instant;

use super::status::Gate;
use super::{check_state, Dialer};
use crate::door::Door;

/// How long one proxied client connection may take, end to end.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(300);

/// The way out of the tailnet door: `ts-dial` through the LocalAPI.
struct TailnetDial {
    dialer: Dialer,
    timeout: Duration,
    gate: Option<Arc<Gate>>,
}

impl Dial for TailnetDial {
    fn label(&self) -> String {
        format!("tailscale:{}", self.dialer.socket().display())
    }

    /// The daemon resolves the names of MagicDNS, so the door sends them.
    fn names_inside(&self) -> bool {
        true
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            // T-243: the daemon is in another tailnet.
            if self.gate.as_ref().is_some_and(|gate| gate.is_blocked()) {
                return Err(Failure::new(
                    Stage::Proxy,
                    Class::Policy,
                    "tailscaled is in another tailnet: new dials are refused",
                ));
            }
            let (host, port) = (target.host(), target.port());
            let limit = self
                .timeout
                .min(deadline.saturating_duration_since(Instant::now()));
            let dialed = match tokio::time::timeout(limit, self.dialer.dial(&host, port)).await {
                Ok(result) => result,
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "tailscaled did not answer the dial in {} ms (is it logged in? tailscale status)",
                        self.timeout.as_millis()
                    ),
                )),
            };
            match dialed {
                Ok(remote) => Ok(Connected::new(Box::new(remote))),
                Err(err) => {
                    tokio::spawn(check_state(self.dialer.clone()));
                    if let Some(gate) = &self.gate {
                        let (gate, dialer) = (gate.clone(), self.dialer.clone());
                        tokio::spawn(async move { gate.refresh(&dialer).await });
                    }
                    crate::warn(&format!("tailscale: {host}:{port} failed: {err}"));
                    Err(Failure::io(Stage::Connect, err))
                }
            }
        })
    }
}

/// The door over `dialer`. A dial that takes more than `timeout` is a
/// timeout (SOCKS5 4, HTTP 504).
pub(super) fn door(
    dialer: Dialer,
    default_port: u16,
    timeout: Duration,
    gate: Option<Arc<Gate>>,
) -> Door {
    Door::new(Arc::new(TailnetDial {
        dialer,
        timeout,
        gate,
    }))
    .with_default_port(default_port)
    .with_flow_limit(CLIENT_TIMEOUT)
    .connect_only()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    /// A fake LocalAPI that records the `Dial-Port` of each dial and then
    /// closes the connection after the `101`.
    async fn fake(listener: tokio::net::UnixListener, ports: Arc<std::sync::Mutex<Vec<String>>>) {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while stream.read(&mut byte).await.unwrap_or(0) == 1 {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            if head.starts_with("GET /localapi/v0/status") {
                let _ = stream
                    .write_all(b"HTTP/1.0 200 OK\r\n\r\n{\"BackendState\":\"Running\"}")
                    .await;
                continue;
            }
            for line in head.split("\r\n") {
                if let Some(port) = line.strip_prefix("Dial-Port: ") {
                    ports.lock().unwrap().push(port.to_string());
                }
            }
            let _ = stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: ts-dial\r\nConnection: upgrade\r\n\r\n")
                .await;
        }
    }

    /// T-061: the tailnet door is the door over a dialer. A request with
    /// port 0 takes the port of `--port`, and a plain `http://` request gets
    /// 405 and no dial.
    #[tokio::test]
    async fn the_tailnet_door_gives_port_0_the_port_option_and_takes_connect_only() {
        let dir = tempfile::tempdir().unwrap();
        let api = dir.path().join("tailscaled.sock");
        let listener = tokio::net::UnixListener::bind(&api).unwrap();
        let ports = Arc::new(std::sync::Mutex::new(Vec::new()));
        tokio::spawn(fake(listener, ports.clone()));
        let front = dir.path().join("front.sock");
        let proxy = super::super::serve(
            Dialer::new(&api),
            crate::listen::ProxyListen::Unix(front.clone()),
            8081,
        )
        .await
        .unwrap();

        let mut client = UnixStream::connect(&front).await.unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        client
            .write_all(&[5, 1, 0, 1, 100, 82, 22, 3, 0, 0])
            .await
            .unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 0, "{reply:?}");
        assert_eq!(*ports.lock().unwrap(), ["8081"]);

        let mut client = UnixStream::connect(&front).await.unwrap();
        client
            .write_all(b"GET http://100.82.22.3/ HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut answer = String::new();
        client.read_to_string(&mut answer).await.unwrap();
        assert!(
            answer.starts_with("HTTP/1.1 405 Method Not Allowed"),
            "{answer}"
        );
        assert_eq!(ports.lock().unwrap().len(), 1, "no dial for a GET");
        proxy.abort();
    }
}
