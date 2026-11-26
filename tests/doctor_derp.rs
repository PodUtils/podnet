//! The row `derp` of `podnet doctor` (T-128): derper's probe of the servers
//! of `--derp-server`, through the way out of podnet and with the roots of
//! `--ca-file`. The servers are stand-ins on the loopback that answer
//! `/derp/probe` as derper does. No test uses the network.

mod test_host;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;

/// The answer of a stand-in server to `GET /derp/probe`.
#[derive(Clone, Copy)]
enum Probe {
    /// 200 with no body, as derper.
    Answers,
    /// 404: a web server that is not derper.
    NotDerp,
}

/// A CA in a PEM file, and the TLS of a server for 127.0.0.1 that it
/// signs. The CA file goes to `--ca-file`.
fn tls(dir: &std::path::Path) -> (std::path::PathBuf, Arc<rustls::ServerConfig>) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "podnet doctor derp test CA");
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["127.0.0.1".to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let path = dir.join("ca.pem");
    std::fs::write(&path, ca.pem()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
    (path, Arc::new(config))
}

/// A stand-in server on a loopback port: HTTPS, one answer for each
/// request. Gives the port.
fn server(config: Arc<rustls::ServerConfig>, probe: Probe) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let config = config.clone();
            std::thread::spawn(move || {
                let conn = rustls::ServerConnection::new(config).unwrap();
                let mut tls = rustls::StreamOwned::new(conn, stream);
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match tls.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => return,
                    }
                }
                let probe_path = String::from_utf8_lossy(&head).starts_with("GET /derp/probe ");
                let status = match probe {
                    Probe::Answers if probe_path => "200 OK",
                    _ => "404 Not Found",
                };
                let _ = write!(tls, "HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n");
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    port
}

/// `podnet --ca-file CA doctor --quick --json --derp-server LIST`, with a
/// process proxy that refuses each connection and `NO_PROXY` for the
/// loopback, so only the probes of the row reach a server.
fn doctor(ca: &std::path::Path, list: &str) -> (Option<i32>, serde_json::Value, String) {
    let (_guard, dead) = pod_testkit::refused::refused();
    let ca = ca.to_str().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["--ca-file", ca, "--proxy", &format!("http://{dead}")])
        .args(["doctor", "--quick", "--json", "--timeout", "1"])
        .args(["--derp-server", list])
        .env("PODNET_RUN_DIR", "/nonexistent/podnet-run")
        .env("PODNET_EDGE", "192.0.2.1:7844")
        .env("NO_PROXY", "127.0.0.1")
        .env_remove("no_proxy")
        .env_remove("SSL_CERT_FILE")
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code(), doc, all)
}

fn keys(v: &serde_json::Value) -> Vec<&str> {
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

/// T-128: the row of each kind of server: derper's answer is `ok` with the
/// server and its round trip; the first server of the list that answers is
/// the one that the row names; a closed port and a server that is not
/// derper are `fail` with one line that names the first server and its
/// reason. The row does not change the exit code of the doctor.
#[test]
fn the_derp_row_names_the_server_that_answers_or_why_none_does() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (ca, config) = tls(dir.path());
    let derper = format!("127.0.0.1:{}", server(config.clone(), Probe::Answers));
    let web = format!("127.0.0.1:{}", server(config, Probe::NotDerp));
    let (_guard, closed) = pod_testkit::refused::refused();
    let closed = closed.to_string();
    // (list, status, server, answered, words of the detail)
    let table = [
        (
            derper.clone(),
            "ok",
            Some(&derper),
            1,
            "1 of 1 servers answered",
        ),
        (
            format!("{closed},{derper}"),
            "ok",
            Some(&derper),
            1,
            "1 of 2 servers answered",
        ),
        (
            closed.clone(),
            "fail",
            None,
            0,
            "no DERP server of 1 answered /derp/probe",
        ),
        (web.clone(), "fail", None, 0, "HTTP 404"),
        (
            format!("{web},{closed}"),
            "fail",
            None,
            0,
            "; and 1 more that did not",
        ),
    ];
    for (list, status, server, answered, words) in table {
        let (code, doc, all) = doctor(&ca, &list);
        let row = &doc["derp"];
        let case = format!("{list}: {row}\n{all}");
        assert_eq!(code, Some(1), "no leg works behind the dead proxy: {case}");
        assert_eq!(
            keys(row),
            ["answered", "detail", "fleet", "ms", "probed", "server", "status", "verdict"],
            "{case}"
        );
        assert_eq!(row["status"], status, "{case}");
        assert_eq!(row["server"].as_str(), server.map(String::as_str), "{case}");
        assert_eq!(row["answered"], answered, "{case}");
        assert_eq!(row["ms"].is_number(), status == "ok", "{case}");
        assert!(row["fleet"].is_null(), "a server of the user's own: {case}");
        assert_eq!(
            row["probed"].as_array().unwrap().len(),
            list.split(',').count(),
            "{case}"
        );
        let detail = row["detail"].as_str().unwrap();
        assert!(detail.contains(words), "{case}");
        assert!(!detail.contains('\n'), "one line: {case}");
    }
}

/// T-128: the text report has the row, and a list that is not a list of
/// servers exits 64 before the network.
#[test]
fn the_text_has_the_row_and_a_bad_list_exits_64() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (ca, config) = tls(dir.path());
    let derper = format!("127.0.0.1:{}", server(config, Probe::Answers));
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "--ca-file",
            ca.to_str().unwrap(),
            "--proxy",
            "http://127.0.0.1:1",
        ])
        .args([
            "doctor",
            "--quick",
            "--timeout",
            "1",
            "--derp-server",
            &derper,
        ])
        .env("PODNET_RUN_DIR", "/nonexistent/podnet-run")
        .env("PODNET_EDGE", "192.0.2.1:7844")
        .env("NO_PROXY", "127.0.0.1")
        .env_remove("no_proxy")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.starts_with("derp ")).unwrap_or("");
    assert!(line.starts_with("derp   ok: 127.0.0.1:"), "{text}");
    assert!(line.contains("answered /derp/probe in"), "{text}");
    let (code, _, all) = doctor(&ca, "bad host,");
    assert_eq!(code, Some(64), "{all}");
    assert!(all.contains("bad host"), "{all}");
}
