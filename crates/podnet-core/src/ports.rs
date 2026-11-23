//! Discover local TCP ports that are already listening, so a session can be
//! pointed at one. Linux reads `/proc/net/tcp{,6}`; other platforms return an
//! empty list.

use std::collections::BTreeSet;

/// Listening TCP ports on loopback (and wildcard binds).
pub fn listening_ports() -> Vec<u16> {
    #[cfg(target_os = "linux")]
    {
        let mut ports = BTreeSet::new();
        for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
            if let Ok(text) = std::fs::read_to_string(path) {
                parse_proc_net(&text, &mut ports);
            }
        }
        ports.into_iter().collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn parse_proc_net(text: &str, ports: &mut BTreeSet<u16>) {
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            continue;
        }
        // state 0A == LISTEN
        if fields[3] != "0A" {
            continue;
        }
        let Some((_, port_hex)) = fields[1].rsplit_once(':') else {
            continue;
        };
        if let Ok(port) = u16::from_str_radix(port_hex, 16) {
            if port != 0 {
                ports.insert(port);
            }
        }
    }
}

/// True when a TCP connect to 127.0.0.1:`port` works in `timeout` (issue
/// 119: a port in the LISTEN table can refuse each connection, for example
/// under a seccomp policy that denies connect).
pub async fn reachable(port: u16, timeout: std::time::Duration) -> bool {
    origin(port, timeout).await.is_ok()
}

/// [`reachable`] with the cause: `Err` is one line with the cause and the
/// next step (T-523: a connect that the host refuses is not "nothing
/// accepts a connection").
pub async fn origin(port: u16, timeout: std::time::Duration) -> Result<(), String> {
    let connect = tokio::net::TcpStream::connect(("127.0.0.1", port));
    match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(origin_problem(port, Some(err.kind()), &err.to_string())),
        Err(_) => Err(origin_problem(port, None, "")),
    }
}

/// The line of a failed connect to the origin 127.0.0.1:`port`: `kind` is
/// the class of the error, `None` for a time limit.
pub fn origin_problem(port: u16, kind: Option<std::io::ErrorKind>, error: &str) -> String {
    use std::io::ErrorKind::{ConnectionRefused, PermissionDenied};
    match kind {
        Some(ConnectionRefused) => format!("nothing accepts a connection on 127.0.0.1:{port}"),
        Some(PermissionDenied) => format!(
            "the host refuses a TCP connection to 127.0.0.1:{port}: use unix sockets (podnet tunnel --unix, podnet net)"
        ),
        None => format!("127.0.0.1:{port} does not answer a TCP connect in time"),
        Some(_) => format!("127.0.0.1:{port}: {error}"),
    }
}

/// The ports of `ports` that accept a connection, and the others.
pub async fn probe(ports: &[u16]) -> (Vec<u16>, Vec<u16>) {
    let checks = futures::future::join_all(
        ports
            .iter()
            .map(|p| reachable(*p, std::time::Duration::from_millis(500))),
    )
    .await;
    let mut open = Vec::new();
    let mut closed = Vec::new();
    for (port, ok) in ports.iter().zip(checks) {
        if ok {
            open.push(*port);
        } else {
            closed.push(*port);
        }
    }
    (open, closed)
}

/// Probe a loopback port with a minimal HTTP request; returns the port when
/// something HTTP-ish answers.
pub async fn confirm_local_http(port: u16) -> anyhow::Result<u16> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map_err(|_| anyhow::anyhow!("port {port} did not accept a connection"))?
    .map_err(|e| anyhow::anyhow!("cannot connect to 127.0.0.1:{port}: {e}"))?;
    stream
        .write_all(b"HEAD / HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await?;
    let mut buf = [0u8; 64];
    let n = tokio::time::timeout(std::time::Duration::from_secs(3), stream.read(&mut buf))
        .await
        .map_err(|_| anyhow::anyhow!("port {port} did not answer HTTP"))?
        .map_err(|e| anyhow::anyhow!("port {port} read failed: {e}"))?;
    if n == 0 || !buf[..n].starts_with(b"HTTP/") {
        anyhow::bail!("port {port} does not look like an HTTP server");
    }
    Ok(port)
}

/// T-524: open a TCP listener on 127.0.0.1 and connect to it. A bind and a
/// listen can work on a host where a connect to the loopback fails (no
/// address on `lo`). `check` sees the address of the listener before the
/// connect (the policy of `podnet agent`). The error is one line:
/// `bind: ...`, `connect: ...`, or the text of `check`.
pub fn loopback_connect(
    check: impl FnOnce(std::net::SocketAddr) -> Result<(), String>,
) -> Result<std::net::SocketAddr, String> {
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("bind: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("no local address: {e}"))?;
    check(addr)?;
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2))
        .map_err(|e| format!("connect: {e}"))?;
    Ok(addr)
}

/// T-524: bind a unix socket at `path`, connect to it, and remove the file.
pub fn unix_connect(path: &std::path::Path) -> Result<(), String> {
    let _ = std::fs::remove_file(path);
    let result = std::os::unix::net::UnixListener::bind(path)
        .map_err(|e| format!("bind: {e}"))
        .and_then(|listener| {
            std::os::unix::net::UnixStream::connect(path)
                .map(drop)
                .map_err(|e| format!("connect: {e}"))?;
            drop(listener);
            Ok(())
        });
    let _ = std::fs::remove_file(path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_connect_names_the_failed_step() {
        let dir = std::env::temp_dir().join(format!("podnet-unix-connect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unix_connect(&dir.join("s.sock")), Ok(()));
        let none = unix_connect(&dir.join("no/such/s.sock")).unwrap_err();
        assert!(none.starts_with("bind: "), "{none}");
        let _ = std::fs::remove_dir_all(&dir);
        let refused = loopback_connect(|_| Err("refused by the policy".into()));
        // A host with no loopback refuses the bind: the line names it.
        match refused {
            Err(line) => assert!(
                line == "refused by the policy" || line.starts_with("bind: "),
                "{line}"
            ),
            Ok(addr) => panic!("the check was not called: {addr}"),
        }
    }

    /// T-523: the line of a failed origin check names its cause. A connect
    /// that the host refuses (EACCES) names the unix sockets; a refusal is
    /// "nothing accepts"; a time limit and another error are not either.
    #[test]
    fn the_origin_line_follows_the_cause_of_the_connect() {
        use std::io::ErrorKind::{ConnectionRefused, NetworkUnreachable, PermissionDenied};
        let cases = [
            (
                Some(ConnectionRefused),
                "nothing accepts a connection on 127.0.0.1:8080",
            ),
            (
                Some(PermissionDenied),
                "the host refuses a TCP connection to 127.0.0.1:8080: use unix sockets",
            ),
            (None, "127.0.0.1:8080 does not answer a TCP connect in time"),
            (Some(NetworkUnreachable), "127.0.0.1:8080: unreachable"),
        ];
        for (kind, want) in cases {
            let line = origin_problem(8080, kind, "unreachable");
            assert!(line.starts_with(want), "{kind:?}: {line}");
            assert!(!line.contains('\n'), "{kind:?}: {line}");
        }
    }

    #[tokio::test]
    async fn the_probe_splits_open_and_closed_ports() {
        pod_testkit::need_loopback!();
        let open = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (a, b) = (
            open.local_addr().unwrap().port(),
            closed.local_addr().unwrap().port(),
        );
        drop(closed);
        assert_eq!(probe(&[a, b]).await, (vec![a], vec![b]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_listen_rows() {
        let sample = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
                      0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000 0 1 1\n\
                      1: 00000000:0050 00000000:0000 01 00000000:00000000 00:00000000 00000000  0 0 2 1\n";
        let mut ports = BTreeSet::new();
        parse_proc_net(sample, &mut ports);
        assert_eq!(ports.into_iter().collect::<Vec<_>>(), vec![8080]);
    }
}
