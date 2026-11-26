//! `podnet forward --network https://...`, `--network wstunnel:...` (T-126),
//! `--network vless://...`, `--network ss://...` and `--network wisp:...`
//! (T-127): a TCP stream goes through a forward proxy over TLS, a wstunnel
//! server, a VLESS server, a Shadowsocks server or a Wisp server. The
//! servers are the stand-ins of `src/networks/road_stand.rs` and
//! `src/networks/servers_stand.rs` on the loopback, with a CA that
//! `--ca-file` gives: no public server and no account. The secrets come from
//! the environment, never from `argv`.

mod test_host;

#[path = "../src/networks/turn_mock.rs"]
mod turn_mock;

#[path = "../src/networks/road_stand.rs"]
mod road_stand;

#[path = "../src/networks/servers_stand.rs"]
mod servers_stand;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use road_stand as stand;

/// The variables of the roads and of the proxy of the host are removed
/// first, so that the host's own do not reach the command.
fn podnet(sub: &[&str], args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(sub).args(args);
    for name in [
        "PODNET_HTTPS_PROXY_AUTH",
        "PODNET_HTTPS_PROXY_AUTH_FILE",
        "PODNET_HTTPS_PROXY_VERSION",
        "PODNET_WSTUNNEL_SECRET",
        "PODNET_WSTUNNEL_SECRET_FILE",
        "PODNET_VLESS_ID",
        "PODNET_VLESS_ID_FILE",
        "PODNET_SHADOWSOCKS_KEY",
        "PODNET_SHADOWSOCKS_KEY_FILE",
        "PODNET_PROXY",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "SSL_CERT_FILE",
        "RUST_LOG",
    ] {
        cmd.env_remove(name);
    }
    cmd
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The CA of the stands and a file of `secret`, in `dir`.
fn files(dir: &tempfile::TempDir, secret: &str) -> (String, String) {
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, stand::ca_pem()).unwrap();
    let file = dir.path().join("secret");
    std::fs::write(&file, format!("{secret}\n")).unwrap();
    let path = |p: std::path::PathBuf| p.to_str().unwrap().to_string();
    (path(ca), path(file))
}

/// A TCP forward through each road: the bytes come back from the echo
/// target, and no secret is in the output.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tcp_forward_goes_through_each_road_of_pod_egress() {
    test_host::need_loopback!();
    let echo = stand::echo().await;
    let (proxy, wst) = (stand::https_proxy().await, stand::wstunnel().await);
    let (vless, ss) = (
        servers_stand::vless().await,
        servers_stand::shadowsocks().await,
    );
    let wisp = servers_stand::wisp().await;
    let line = format!("{}:{}", stand::USER, stand::PASSWORD);
    let key = format!(
        "{}:{}",
        servers_stand::SS_METHOD,
        servers_stand::SS_PASSWORD
    );
    let roads = [
        (
            format!("https://{}", proxy.addr),
            "PODNET_HTTPS_PROXY_AUTH_FILE",
            line.as_str(),
        ),
        (
            format!("wstunnel:wss://{}", wst.addr),
            "PODNET_WSTUNNEL_SECRET_FILE",
            stand::PREFIX,
        ),
        (
            format!("vless://{}?type=ws&security=tls&path=%2Fvless", vless.addr),
            "PODNET_VLESS_ID_FILE",
            servers_stand::VLESS_ID,
        ),
        (
            format!("ss://{}", ss.addr),
            "PODNET_SHADOWSOCKS_KEY_FILE",
            key.as_str(),
        ),
        // A Wisp server takes no secret: the file is not read.
        (
            format!("wisp:wss://{}{}", wisp.stand.addr, servers_stand::WISP_PATH),
            "PODNET_UNUSED_FILE",
            "no-secret-of-wisp",
        ),
    ];
    for (network, variable, secret) in roads {
        let dir = tempfile::tempdir().unwrap();
        let (ca, file) = files(&dir, secret);
        let port = free_tcp_port();
        let listen = format!("127.0.0.1:{port}={echo}");
        let args = [
            "-L",
            &listen,
            "--network",
            &network,
            "--ca-file",
            &ca,
            "--run-for",
            "60",
        ];
        let mut cmd = podnet(&["forward"], &args);
        cmd.env(variable, &file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = Proc(cmd.spawn().unwrap());
        let mut stdout = BufReader::new(child.0.stdout.take().unwrap());
        let mut first = String::new();
        stdout.read_line(&mut first).unwrap();
        assert!(first.starts_with("podnet: forward"), "{network}: {first:?}");
        let label = match network.split_once('?') {
            Some((at, _)) => format!("{at}/vless"),
            None => network.clone(),
        };
        assert!(first.contains(&format!("via {label}")), "{first:?}");
        for text in ["first", "second"] {
            let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            client.write_all(text.as_bytes()).unwrap();
            let mut back = vec![0u8; text.len()];
            client
                .read_exact(&mut back)
                .expect("an echo through the road");
            assert_eq!(back, text.as_bytes());
        }
        let _ = child.0.kill();
        let mut output = first;
        stdout.read_to_string(&mut output).unwrap();
        let mut err = String::new();
        child
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut err)
            .unwrap();
        output.push_str(&err);
        assert!(
            !output.contains(secret),
            "a secret is in the output: {output}"
        );
    }
    for (seen, road) in [
        (&proxy.seen, "https"),
        (&wst.seen, "wstunnel"),
        (&vless.seen, "vless"),
        (&ss.seen, "ss"),
        (&wisp.stand.seen, "wisp"),
    ] {
        assert_eq!(seen.lock().unwrap().targets.len(), 2, "{road}");
    }
}

/// A case of a fault: the spec, the variables, and the text that the line
/// must name.
type Case = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

/// One rule for each bad spec and missing secret of a road at the command:
/// exit 64, a line that names the variable or the form, and no secret.
#[test]
fn a_bad_road_spec_or_secret_is_a_usage_error_with_no_secret() {
    let cases: [Case; 10] = [
        ("https://user:hunter2@127.0.0.1:9", &[], "password"),
        (
            "https://user@127.0.0.1:9",
            &[],
            "PODNET_HTTPS_PROXY_AUTH_FILE",
        ),
        (
            "https://127.0.0.1:9",
            &[("PODNET_HTTPS_PROXY_AUTH", "hunter2")],
            "PODNET_HTTPS_PROXY_AUTH",
        ),
        ("wstunnel:wss://127.0.0.1:9/hunter2", &[], "path prefix"),
        (
            "wstunnel:ws://192.0.2.1:9",
            &[("PODNET_WSTUNNEL_SECRET", "hunter2")],
            "wss://",
        ),
        (
            "vless://hunter2@127.0.0.1:9?type=ws",
            &[],
            "PODNET_VLESS_ID_FILE",
        ),
        ("vless://127.0.0.1:9?type=ws", &[], "PODNET_VLESS_ID_FILE"),
        (
            "ss://127.0.0.1:9",
            &[("PODNET_SHADOWSOCKS_KEY", "hunter2")],
            "PODNET_SHADOWSOCKS_KEY",
        ),
        (
            "ss://hunter2@127.0.0.1:9",
            &[],
            "PODNET_SHADOWSOCKS_KEY_FILE",
        ),
        ("wisp:wss://u:hunter2@127.0.0.1:9/", &[], "no user"),
    ];
    for (network, vars, named) in cases {
        let mut cmd = podnet(
            &["forward"],
            &["-L", "127.0.0.1:5399=127.0.0.1:22", "--network", network],
        );
        cmd.envs(vars.iter().copied());
        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(64), "{network}: {out:?}");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(all.contains(named), "{network}: {all}");
        assert!(!all.contains("hunter2"), "{network}: {all}");
    }
    // --egress names the networks of the registry.
    for network in [
        "https://127.0.0.1:9",
        "wstunnel:wss://127.0.0.1:9",
        "vless://127.0.0.1:9?type=ws",
        "ss://127.0.0.1:9",
        "wisp:wss://127.0.0.1:9/",
    ] {
        let out = podnet(&["--egress", network, "doctor"], &[])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{network}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("podnet forward") && err.contains("--network"),
            "{err}"
        );
    }
}

/// The row of `network` in `net doctor --json --network`.
fn doctor_row(network: &str, ca: &str, variable: &str, file: &str) -> serde_json::Value {
    let out = podnet(
        &["net", "doctor"],
        &["--json", "--network", network, "--ca-file", ca],
    )
    .env(variable, file)
    .output()
    .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let doc: serde_json::Value = serde_json::from_str(&text).expect("one JSON document");
    let name = format!("network {network}");
    doc["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["probe"] == name.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("no row of the network: {text}"))
}

/// `net doctor --network` (T-126): the row of a road is `ok` when its server
/// takes TLS with the CA of `--ca-file`, and `fail` with the hop when no
/// server answers. No secret reaches the document.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn net_doctor_has_a_row_for_each_road_that_names_the_hop() {
    test_host::need_loopback!();
    let (proxy, wst) = (stand::https_proxy().await, stand::wstunnel().await);
    let dir = tempfile::tempdir().unwrap();
    let line = format!("{}:{}", stand::USER, stand::PASSWORD);
    let (ca, auth) = files(&dir, &line);
    let dir2 = tempfile::tempdir().unwrap();
    let (_, prefix) = files(&dir2, stand::PREFIX);
    let closed = free_tcp_port();
    let rows = [
        (
            format!("https://{}", proxy.addr),
            "PODNET_HTTPS_PROXY_AUTH_FILE",
            auth.clone(),
            "ok",
            "",
        ),
        (
            format!("wstunnel:wss://{}", wst.addr),
            "PODNET_WSTUNNEL_SECRET_FILE",
            prefix.clone(),
            "ok",
            "",
        ),
        (
            format!("https://127.0.0.1:{closed}"),
            "PODNET_HTTPS_PROXY_AUTH_FILE",
            auth,
            "fail",
            "the proxy",
        ),
        (
            format!("wstunnel:wss://127.0.0.1:{closed}"),
            "PODNET_WSTUNNEL_SECRET_FILE",
            prefix,
            "fail",
            "the wstunnel server",
        ),
    ];
    for (network, variable, file, status, hop) in rows {
        let ca = ca.clone();
        let n = network.clone();
        let row = tokio::task::spawn_blocking(move || doctor_row(&n, &ca, variable, &file))
            .await
            .unwrap();
        assert_eq!(row["status"], status, "{network}: {row}");
        assert!(
            row["detail"].as_str().unwrap_or_default().contains(hop),
            "{row}"
        );
        let text = row.to_string();
        assert!(
            !text.contains(stand::PASSWORD) && !text.contains(stand::PREFIX),
            "{text}"
        );
    }
}

/// `net doctor --network` (T-127): the row of a VLESS, a Shadowsocks and a
/// Wisp server is `ok` when its server takes the connection (and the opening
/// of Wisp), and `fail` with the hop when no server answers. No secret
/// reaches the document.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn net_doctor_has_a_row_for_each_server_road_that_names_the_hop() {
    test_host::need_loopback!();
    let (vless, ss) = (
        servers_stand::vless().await,
        servers_stand::shadowsocks().await,
    );
    let wisp = servers_stand::wisp().await;
    let key = format!(
        "{}:{}",
        servers_stand::SS_METHOD,
        servers_stand::SS_PASSWORD
    );
    let (dir, dir2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (ca, id) = files(&dir, servers_stand::VLESS_ID);
    let (_, key_file) = files(&dir2, &key);
    let closed = free_tcp_port();
    let vless_at = |at: String| format!("vless://{at}?type=ws&security=tls&path=%2Fvless");
    let (id_var, key_var) = ("PODNET_VLESS_ID_FILE", "PODNET_SHADOWSOCKS_KEY_FILE");
    let wisp_at = |at: String| format!("wisp:wss://{at}{}", servers_stand::WISP_PATH);
    let rows = [
        (vless_at(vless.addr.to_string()), id_var, &id, "ok", ""),
        (format!("ss://{}", ss.addr), key_var, &key_file, "ok", ""),
        (wisp_at(wisp.stand.addr.to_string()), id_var, &id, "ok", ""),
        (
            vless_at(format!("127.0.0.1:{closed}")),
            id_var,
            &id,
            "fail",
            "the VLESS server",
        ),
        (
            format!("ss://127.0.0.1:{closed}"),
            key_var,
            &key_file,
            "fail",
            "the Shadowsocks server",
        ),
        (
            wisp_at(format!("127.0.0.1:{closed}")),
            id_var,
            &id,
            "fail",
            "the Wisp server",
        ),
    ];
    for (network, variable, file, status, hop) in rows {
        let (ca, file, n) = (ca.clone(), file.clone(), network.clone());
        let row = tokio::task::spawn_blocking(move || doctor_row(&n, &ca, variable, &file))
            .await
            .unwrap();
        assert_eq!(row["status"], status, "{network}: {row}");
        assert!(
            row["detail"].as_str().unwrap_or_default().contains(hop),
            "{row}"
        );
        let text = row.to_string();
        assert!(
            !text.contains(servers_stand::VLESS_ID) && !text.contains(servers_stand::SS_PASSWORD),
            "{text}"
        );
    }
}
