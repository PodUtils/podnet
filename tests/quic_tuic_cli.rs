//! The CLI part of the TUIC gate of PodUtils/podnet#486 (QUIC-8):
//! `podnet warp --tuic URL nc HOST PORT` runs the road of `src/roads/tuic.rs`
//! against a server built from the specification.
//!
//! The server is a stand-in: it answers the authentication and echoes one
//! TCP stream. It proves that the command line reaches the road, that the
//! link and the token are used, and that the bytes of stdin come back on
//! stdout. It does not prove interoperability with `tuic-server`, and it
//! exercises no UDP: the UDP path of the road is the test `quic_tuic`.

use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;

use pod_quic::noq;
use pod_quic::noq::crypto::rustls::QuicServerConfig;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PASSWORD: &str = "cli-password";
const ALPN: &[u8] = b"h3";

fn crypto() -> Arc<QuicServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let key = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    Arc::new(QuicServerConfig::try_from(Arc::new(tls)).unwrap())
}

/// The authentication, then one echo of the bytes after the proxy header.
async fn serve(conn: noq::Connection) {
    let Ok(mut auth) = conn.accept_uni().await else {
        return;
    };
    let mut bytes = vec![0u8; 2 + 16 + 32];
    if auth.read_exact(&mut bytes).await.is_err() {
        return;
    }
    let uuid = uuid::Uuid::parse_str(UUID).unwrap();
    let mut token = [0u8; 32];
    if conn
        .export_keying_material(&mut token, uuid.as_bytes(), PASSWORD.as_bytes())
        .is_err()
    {
        return;
    }
    if bytes[..2] != [5, 0] || bytes[2..18] != *uuid.as_bytes() || bytes[18..] != token {
        return;
    }
    let _auth = auth;
    let Ok((mut send, mut recv)) = conn.accept_bi().await else {
        return;
    };
    // The Connect command: the version, the command, the address type, the
    // address, the port.
    let mut head = [0u8; 2];
    if recv.read_exact(&mut head).await.is_err() || head != [5, 1] {
        return;
    }
    let mut kind = [0u8; 1];
    if recv.read_exact(&mut kind).await.is_err() {
        return;
    }
    let extra = match kind[0] {
        0x00 => {
            let mut len = [0u8; 1];
            if recv.read_exact(&mut len).await.is_err() {
                return;
            }
            usize::from(len[0])
        }
        0x01 => 4,
        0x02 => 16,
        _ => return,
    };
    let mut rest = vec![0u8; extra + 2];
    if recv.read_exact(&mut rest).await.is_err() {
        return;
    }
    let mut buf = [0u8; 1024];
    let Ok(Some(n)) = recv.read(&mut buf).await else {
        return;
    };
    let _ = send.write_all(&buf[..n]).await;
    let _ = send.finish();
    // Keep the connection open until the client closes it: a dropped
    // connection would reach the client as "connection lost", not EOF.
    let _ = conn.closed().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tuic_option_of_warp_nc_carries_stdin_to_stdout() {
    let endpoint = pod_quic::endpoint::server(
        pod_quic::endpoint::server_config(crypto()).unwrap(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let server: SocketAddr = endpoint.local_addr().unwrap();
    let serving = tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let Ok(conn) = incoming.await else { continue };
            tokio::spawn(serve(conn));
        }
    });
    let link = format!("tuic://{UUID}:{PASSWORD}@{server}?sni=localhost&allow_insecure=1");
    let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "warp",
            "--tuic",
            &link,
            "nc",
            "--shutdown",
            "echo.test",
            "80",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the podnet binary");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(b"podnet tuic nc").await.unwrap();
    drop(stdin);
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("podnet warp --tuic nc ends")
        .expect("the status of podnet");
    assert!(
        out.status.success(),
        "status={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout,
        b"podnet tuic nc",
        "stdout={}",
        String::from_utf8_lossy(&out.stdout)
    );
    println!(
        "tuic cli: status={} stdout={:?}",
        out.status,
        String::from_utf8_lossy(&out.stdout)
    );
    serving.abort();
}

/// `--tuic` and `--via-relay` both pick the transport of `nc`, so the
/// command line must not take both. The check is in the dispatch, not in
/// `clap`: `--tuic` is global and `--via-relay` belongs to `nc`.
#[tokio::test]
async fn the_tuic_option_refuses_the_relay() {
    let link = format!("tuic://{UUID}:{PASSWORD}@127.0.0.1:1");
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "warp",
            "--tuic",
            &link,
            "nc",
            "--via-relay",
            "echo.test",
            "80",
        ])
        .output()
        .await
        .expect("the podnet binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(64),
        "status={:?} stderr={stderr}",
        out.status
    );
    assert!(
        stderr.contains("--tuic and --via-relay are exclusive"),
        "stderr={stderr}"
    );
    println!(
        "tuic cli conflict: status={:?} stderr={:?}",
        out.status,
        stderr.trim()
    );
}
