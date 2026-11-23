use std::net::TcpListener;
use std::sync::mpsc;

use super::*;

/// A fake proxy on loopback for one connection: it reads the head of the
/// request, sends it on `seen`, writes `answer`, then closes, or echoes one
/// read when the answer is a whole head of a 200.
fn fake_proxy(answer: Vec<u8>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (tx, seen) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut sock, _)) = listener.accept() else {
            return;
        };
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match sock.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => return,
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&head).into_owned());
        let _ = sock.write_all(&answer);
        let whole = answer.windows(4).any(|w| w == b"\r\n\r\n");
        if answer.starts_with(b"HTTP/1.1 200") && whole {
            let mut buf = [0u8; 64];
            if let Ok(n) = sock.read(&mut buf) {
                let _ = sock.write_all(&buf[..n]);
            }
        }
    });
    (address, seen)
}

/// T-473: the CONNECT client is the dial of `pod-proxy`. One table for the
/// class "the answer of the proxy": the bytes of the tunnel after the head
/// survive, a refusal is `ConnectionRefused` with the status and the words
/// of the proxy, a head with no end and a closed connection are errors,
/// and credentials go as Basic. A call from a task of a tokio runtime
/// works too (the dial has a thread of its own).
#[test]
fn connect_tunnel_takes_each_answer_of_the_proxy() {
    pod_testkit::need_loopback!();
    type Want = Result<&'static [u8], (io::ErrorKind, &'static str)>;
    let big = vec![b'A'; 16 * 1024 + 64];
    // The answer, the credentials, the result, a header of the request.
    type Case<'a> = (&'a [u8], Option<(&'a str, &'a str)>, Want, Option<&'a str>);
    let cases: Vec<Case> = vec![
        (
            b"HTTP/1.1 200 Connection Established\r\nX: y\r\n\r\nP",
            None,
            Ok(b"Pping"),
            None,
        ),
        (
            b"HTTP/1.1 403 not on the egress allowlist\r\nContent-Length: 0\r\n\r\n",
            None,
            Err((
                io::ErrorKind::ConnectionRefused,
                "403 not on the egress allowlist",
            )),
            None,
        ),
        (
            b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
            None,
            Err((io::ErrorKind::ConnectionRefused, "407")),
            None,
        ),
        (
            &big,
            None,
            Err((io::ErrorKind::Other, "no end of headers")),
            None,
        ),
        (
            b"HTTP/1.1 200 Con",
            None,
            Err((io::ErrorKind::Other, "closed before")),
            None,
        ),
        (
            b"HTTP/1.1 200 OK\r\n\r\n",
            Some(("user", "p@ss:w")),
            Ok(b"ping"),
            // base64("user:p@ss:w")
            Some("Proxy-Authorization: Basic dXNlcjpwQHNzOnc=\r\n"),
        ),
    ];
    for (in_runtime, (answer, credentials, want, header)) in [false, true]
        .into_iter()
        .flat_map(|r| cases.iter().map(move |c| (r, c)))
    {
        let (address, seen) = fake_proxy(answer.to_vec());
        let dial = || {
            connect_tunnel_with_auth(
                &address,
                "example.com",
                443,
                Duration::from_secs(5),
                *credentials,
            )
        };
        let got = if in_runtime {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async { dial() })
        } else {
            dial()
        };
        let request = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            request.starts_with("CONNECT example.com:443 HTTP/1.1\r\n"),
            "{request}"
        );
        if let Some(header) = header {
            assert!(request.contains(header), "{request}");
        }
        match (got, want) {
            (Ok(mut tunnel), Ok(data)) => {
                tunnel.write_all(b"ping").unwrap();
                let mut read = vec![0u8; data.len()];
                tunnel.read_exact(&mut read).unwrap();
                assert_eq!(read, *data, "the bytes after the head survive");
            }
            (Err(e), Err((kind, words))) => {
                assert!(e.to_string().contains(words), "{e}");
                if *kind != io::ErrorKind::Other {
                    assert_eq!(e.kind(), *kind, "{e}");
                }
            }
            (Ok(_), want) => panic!("a tunnel for {want:?}"),
            (Err(e), want) => panic!("{e} for {want:?}"),
        }
    }
}

/// The bytes that a caller read past a head come first, then the stream.
#[test]
fn a_tunnel_stream_gives_its_leftover_first() {
    let mut tunnel = TunnelStream::new(&b"world"[..], b"HELLO".to_vec());
    let mut all = Vec::new();
    tunnel.read_to_end(&mut all).unwrap();
    assert_eq!(all, b"HELLOworld");
}

/// A proxy address that `pod-proxy` cannot read is `InvalidInput`.
#[test]
fn a_bad_proxy_address_is_invalid_input() {
    let err = proxy_of("http://[::1", None).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
    let proxy = proxy_of("127.0.0.1:3128", Some(("u", "p"))).unwrap();
    assert!(proxy.has_credentials());
    assert_eq!(proxy.to_string(), "127.0.0.1:3128");
}
