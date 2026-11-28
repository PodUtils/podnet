//! Offline integration tests for the hermetic client capture rig (`scripts/capture-rig/`).
//!
//! Verifies that the redactor scrubs secrets and rejects raw keylogs, that the
//! pure-Python PCAP fallback dissector extracts TLS ClientHello SNI without
//! external tools, and that `mitm_proxy.py` captures HTTP and passthrough CONNECT
//! ClientHello metadata on loopback.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn rig_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("capture-rig")
}

/// Build a minimal valid TLS ClientHello carrying SNI `host` and ALPN `h2`.
fn build_client_hello(host: &str) -> Vec<u8> {
    let host_bytes = host.as_bytes();
    // SNI extension (0x0000)
    let mut sni_ext = Vec::new();
    let list_len = (host_bytes.len() + 3) as u16;
    let ext_len = list_len + 2;
    sni_ext.extend_from_slice(&0x0000u16.to_be_bytes());
    sni_ext.extend_from_slice(&ext_len.to_be_bytes());
    sni_ext.extend_from_slice(&list_len.to_be_bytes());
    sni_ext.push(0x00); // host_name
    sni_ext.extend_from_slice(&(host_bytes.len() as u16).to_be_bytes());
    sni_ext.extend_from_slice(host_bytes);

    // ALPN extension (0x0010) with "h2"
    let alpn_ext: [u8; 9] = [0x00, 0x10, 0x00, 0x05, 0x00, 0x03, 0x02, b'h', b'2'];

    let mut exts = Vec::new();
    exts.extend_from_slice(&sni_ext);
    exts.extend_from_slice(&alpn_ext);

    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x03]); // TLS 1.2 legacy version
    body.extend_from_slice(&[0xAA; 32]); // random
    body.push(0x00); // session id len
    body.extend_from_slice(&[0x00, 0x02, 0x13, 0x01]); // cipher suites
    body.extend_from_slice(&[0x01, 0x00]); // compression
    body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
    body.extend_from_slice(&exts);

    let mut hs = Vec::new();
    hs.push(0x01); // ClientHello
    let blen = body.len() as u32;
    hs.extend_from_slice(&blen.to_be_bytes()[1..4]);
    hs.extend_from_slice(&body);

    let mut rec = Vec::new();
    rec.extend_from_slice(&[0x16, 0x03, 0x01]);
    rec.extend_from_slice(&(hs.len() as u16).to_be_bytes());
    rec.extend_from_slice(&hs);
    rec
}

/// Wrap `payload` into a minimal valid classic libpcap file.
fn build_pcap(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // Global header (little-endian): magic, v2.4, tz, sigfigs, snaplen, linktype RAW
    out.extend_from_slice(&0xa1b2c3d4u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&65535u32.to_le_bytes());
    out.extend_from_slice(&101u32.to_le_bytes());
    // Packet header
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[test]
fn capture_rig_redactor_scrubs_secrets_and_drops_raw_keylogs() {
    let tmp = tempfile::tempdir().unwrap();
    let raw = tmp.path().join("raw");
    let out = tmp.path().join("redacted");
    std::fs::create_dir_all(&raw).unwrap();

    std::fs::write(
        raw.join("exchange.jsonl"),
        concat!(
            r#"{"request_headers":{"Authorization":"Bearer topsecretvalue123456","CF-Access-Jwt-Assertion":"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3OCJ9.signature12345678","User-Agent":"warp-test"},"token":"my-device-secret-token","account_type":"free"}"#,
            "\n"
        ),
    )
    .unwrap();
    std::fs::write(
        raw.join("tls_keylog.txt"),
        "CLIENT_RANDOM 0123456789abcdef 0123456789abcdef\n",
    )
    .unwrap();
    std::fs::write(
        raw.join("ca.key"),
        "-----BEGIN PRIVATE KEY-----\nSECRET\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    std::fs::write(raw.join("capture.pcap"), build_pcap(b"raw")).unwrap();

    let status = Command::new("python3")
        .arg(rig_dir().join("redact.py"))
        .arg("--in-dir")
        .arg(&raw)
        .arg("--out-dir")
        .arg(&out)
        .status()
        .expect("run redact.py");
    assert!(status.success(), "redact.py must pass audit");

    assert!(
        !out.join("tls_keylog.txt").exists(),
        "keylog must be dropped"
    );
    assert!(!out.join("ca.key").exists(), "CA key must be dropped");
    assert!(
        !out.join("capture.pcap").exists(),
        "raw pcap must be dropped"
    );

    let redacted = std::fs::read_to_string(out.join("exchange.jsonl")).unwrap();
    assert!(!redacted.contains("topsecretvalue123456"));
    assert!(!redacted.contains("signature12345678"));
    assert!(!redacted.contains("my-device-secret-token"));
    assert!(redacted.contains("free"));
    assert!(redacted.contains("[REDACTED]"));

    let audit: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("redaction_audit.json")).unwrap())
            .unwrap();
    assert_eq!(audit["leaks_found"], 0);
    assert!(audit["redactions_applied"].as_u64().unwrap() >= 3);

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["clean"], true);
    assert_eq!(manifest["schema"], 1);
}

#[test]
fn capture_rig_dissector_extracts_sni_via_python_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let pcap = tmp.path().join("sample.pcap");
    let summary_path = tmp.path().join("wire_summary.json");
    let hello = build_client_hello("consumer-masque-proxy.cloudflareclient.com");
    std::fs::write(&pcap, build_pcap(&hello)).unwrap();

    let status = Command::new("python3")
        .arg(rig_dir().join("quic_dissect.py"))
        .arg("--pcap")
        .arg(&pcap)
        .arg("--out")
        .arg(&summary_path)
        .status()
        .expect("run quic_dissect.py");
    assert!(status.success());

    let summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&summary_path).unwrap()).unwrap();
    assert_eq!(summary["fallback_scan"]["packets"], 1);
    let hints = summary["fallback_scan"]["sni_hints"].as_array().unwrap();
    assert!(
        hints
            .iter()
            .any(|v| v.as_str() == Some("consumer-masque-proxy.cloudflareclient.com")),
        "expected SNI in fallback_scan: {summary}"
    );
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn capture_rig_proxy_records_passthrough_connect_sni_and_alpn() {
    pod_testkit::need_loopback!();

    // Dummy upstream sink that accepts one connection and reads the ClientHello bytes.
    let sink = TcpListener::bind("127.0.0.1:0").unwrap();
    let sink_port = sink.local_addr().unwrap().port();
    let sink_thread = std::thread::spawn(move || {
        if let Ok((mut s, _)) = sink.accept() {
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf);
        }
    });

    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("proxy_out");
    std::fs::create_dir_all(&out).unwrap();

    let child = Command::new("python3")
        .arg(rig_dir().join("mitm_proxy.py"))
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .arg("--out")
        .arg(&out)
        .arg("--mode")
        .arg("passthrough")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mitm_proxy.py");
    let _guard = KillOnDrop(child);

    let ready_path = out.join("proxy.ready");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let ready: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&ready_path).unwrap()).unwrap();
    let proxy_port = ready["port"].as_u64().unwrap() as u16;

    let mut conn = TcpStream::connect(("127.0.0.1", proxy_port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        conn,
        "CONNECT 127.0.0.1:{sink_port} HTTP/1.1\r\nHost: 127.0.0.1:{sink_port}\r\n\r\n"
    )
    .unwrap();
    let mut resp = [0u8; 64];
    let n = conn.read(&mut resp).unwrap();
    assert!(std::str::from_utf8(&resp[..n]).unwrap().contains("200"));

    let hello = build_client_hello("example-client.test");
    conn.write_all(&hello).unwrap();
    drop(conn);
    let _ = sink_thread.join();

    // Give proxy a moment to flush connect_log.jsonl.
    let log_path = out.join("connect_log.jsonl");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut text = String::new();
    while Instant::now() < deadline {
        if let Ok(t) = std::fs::read_to_string(&log_path) {
            if t.contains("example-client.test") {
                text = t;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        text.contains("example-client.test") && text.contains("\"h2\""),
        "connect_log missing SNI/ALPN: {text}"
    );
}
