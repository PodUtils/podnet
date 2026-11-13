//! The chain of a link (T-303): each `--egress` of `podnet link dial` and
//! `podnet link connect` is one element before the carrier. The tests run
//! on loopback: two SOCKS5 proxies in the test process, and a `link listen`
//! carrier. A test that needs TCP loopback prints `skipping, REASON` on a
//! host that denies it, and does not fail.
#![cfg(feature = "tcp")]

#[path = "test_host/mod.rs"]
mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use podnet::identity::{self, AllowList, Identity};
use podnet::link::Settings;
use podnet::link_chain::Plan;
use podnet::link_tcp;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(20);

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

fn write_allow(dir: &Path, name: &str, keys: &[&Identity]) -> PathBuf {
    let file = dir.join(name);
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    std::fs::write(&file, text.join("\n") + "\n").unwrap();
    file
}

fn podnet(dir: &Path, state: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join(state))
        .env("PODNET_RUN_DIR", dir.join(format!("run-{state}")))
        .env_remove("RUST_LOG")
        .env_remove("PODNET_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY");
    cmd
}

struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A SOCKS5 proxy. With `auth`, it asks for that user and password. It keeps
/// each target that it is asked for.
fn socks5(auth: Option<(&'static str, &'static str)>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let targets = Arc::new(Mutex::new(Vec::new()));
    let seen = targets.clone();
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let _ = serve_socks5(client, auth, &seen);
            });
        }
    });
    (port, targets)
}

fn serve_socks5(
    mut c: TcpStream,
    auth: Option<(&str, &str)>,
    seen: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    c.read_exact(&mut head)?;
    let mut methods = vec![0u8; head[1] as usize];
    c.read_exact(&mut methods)?;
    match auth {
        None => c.write_all(&[5, 0])?,
        Some((user, pass)) => {
            if !methods.contains(&2) {
                c.write_all(&[5, 0xff])?;
                return Ok(());
            }
            c.write_all(&[5, 2])?;
            let mut v = [0u8; 2];
            c.read_exact(&mut v)?;
            let mut u = vec![0u8; v[1] as usize];
            c.read_exact(&mut u)?;
            let mut l = [0u8; 1];
            c.read_exact(&mut l)?;
            let mut p = vec![0u8; l[0] as usize];
            c.read_exact(&mut p)?;
            if u != user.as_bytes() || p != pass.as_bytes() {
                c.write_all(&[1, 1])?;
                return Ok(());
            }
            c.write_all(&[1, 0])?;
        }
    }
    let mut req = [0u8; 4];
    c.read_exact(&mut req)?;
    let host = match req[3] {
        1 => {
            let mut ip = [0u8; 4];
            c.read_exact(&mut ip)?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut l = [0u8; 1];
            c.read_exact(&mut l)?;
            let mut name = vec![0u8; l[0] as usize];
            c.read_exact(&mut name)?;
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => return Ok(()),
    };
    let mut port = [0u8; 2];
    c.read_exact(&mut port)?;
    let port = u16::from_be_bytes(port);
    seen.lock().unwrap().push(format!("{host}:{port}"));
    let target = match TcpStream::connect((
        if host == "localhost" {
            "127.0.0.1"
        } else {
            host.as_str()
        },
        port,
    )) {
        Ok(t) => t,
        Err(_) => {
            c.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0])?;
            return Ok(());
        }
    };
    c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
    let (mut a, mut b) = (c.try_clone()?, target.try_clone()?);
    let (mut c2, mut t2) = (c, target);
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut a, &mut t2);
        let _ = t2.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut b, &mut c2);
    let _ = c2.shutdown(std::net::Shutdown::Write);
    Ok(())
}

/// The carrier: `podnet link listen` over tcp. It gives the dial address.
fn carrier(dir: &Path, listener: &Identity, allow: &Path) -> (Guard, String) {
    let mut child = podnet(
        dir,
        "b",
        &[
            "link",
            "listen",
            "127.0.0.1:0",
            "--carrier",
            "tcp",
            "--allow",
            allow.to_str().unwrap(),
            "--json",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&line).expect(&line);
    let to = format!(
        "{}@{}",
        listener.fingerprint(),
        doc["listen"].as_str().unwrap()
    );
    (Guard(child), to)
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_elements_reach_a_local_proxy_then_a_local_carrier() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let list = |k: &Identity| AllowList::parse(&k.fingerprint()).unwrap();
    let listening = link_tcp::carrier(&b, &list(&a));
    let mut door = link_tcp::listen(&listening, "127.0.0.1:0").await.unwrap();
    let to = format!("{}@{}", b.fingerprint(), door.address());
    let (first, seen_first) = socks5(None);
    let (second, seen_second) = socks5(None);
    let specs = vec![
        format!("socks5://127.0.0.1:{first}"),
        format!("socks5://127.0.0.1:{second}"),
    ];
    let plan = Plan::parse(&specs, &to).unwrap();
    let opened = plan.open().await.unwrap();
    let dialing = link_tcp::carrier_via(&a, &AllowList::default(), opened.dial());
    let allow = list(&b);
    let (dialed, accepted) = tokio::join!(
        link_tcp::dial(&dialing, &allow, &to, Settings::default()),
        door.accept_peer(Settings::default())
    );
    let ((near, proved), (far, _)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(proved, b.public());
    let mut stream = near.open().unwrap();
    stream.write_all(b"through two elements").await.unwrap();
    let mut peer = timeout(LIMIT, far.accept()).await.unwrap().unwrap();
    let mut got = vec![0; 20];
    timeout(LIMIT, peer.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, b"through two elements");
    let carrier_place = to.rsplit_once('@').unwrap().1.to_string();
    assert_eq!(
        *seen_first.lock().unwrap(),
        vec![format!("127.0.0.1:{second}")],
        "the first element reaches the second element"
    );
    assert_eq!(
        *seen_second.lock().unwrap(),
        vec![carrier_place],
        "the last element reaches the carrier address"
    );
}

#[test]
fn a_chain_of_two_elements_dials_through_the_cli_and_check_prints_each() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (allow_b, allow_a) = (
        write_allow(dir.path(), "allow-b", &[&a]),
        write_allow(dir.path(), "allow-a", &[&b]),
    );
    let (_listener, to) = carrier(dir.path(), &b, &allow_b);
    let (first, seen_first) = socks5(None);
    let (second, seen_second) = socks5(None);
    let (e1, e2) = (
        format!("socks5://127.0.0.1:{first}"),
        format!("socks5://127.0.0.1:{second}"),
    );
    let out = podnet(
        dir.path(),
        "a",
        &[
            "--egress",
            &e1,
            "link",
            "dial",
            &to,
            "--carrier",
            "tcp",
            "--allow",
            allow_a.to_str().unwrap(),
            "--egress",
            &e2,
            "--check",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let shown = text(&out);
    assert!(shown.contains("element 1 of 2"), "{shown}");
    assert!(shown.contains("element 2 of 2"), "{shown}");
    assert!(shown.contains(&e1) && shown.contains(&e2), "{shown}");
    assert!(shown.contains("peer admitted"), "{shown}");
    assert!(seen_first
        .lock()
        .unwrap()
        .contains(&format!("127.0.0.1:{second}")));
    let place = to.rsplit_once('@').unwrap().1;
    assert!(seen_second.lock().unwrap().iter().any(|t| t == place));
}

#[test]
fn check_of_connect_prints_each_element_and_leaves_no_listener() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (allow_b, allow_a) = (
        write_allow(dir.path(), "allow-b", &[&a]),
        write_allow(dir.path(), "allow-a", &[&b]),
    );
    let (_listener, to) = carrier(dir.path(), &b, &allow_b);
    let (first, _) = socks5(None);
    let (second, _) = socks5(None);
    let (e1, e2) = (
        format!("socks5://127.0.0.1:{first}"),
        format!("socks5h://127.0.0.1:{second}"),
    );
    let out = podnet(
        dir.path(),
        "a",
        &[
            "link",
            "connect",
            &to,
            "--carrier",
            "tcp",
            "--allow",
            allow_a.to_str().unwrap(),
            "--egress",
            &e1,
            "--egress",
            &e2,
            "--check",
            "--json",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{}", text(&out)));
    let rows = doc["elements"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{doc}");
    assert_eq!(rows[0]["element"], e1.as_str());
    assert_eq!(rows[1]["n"], 2);
    assert!(rows[1]["resolved"]
        .as_str()
        .unwrap()
        .contains(&second.to_string()));
    assert_eq!(doc["peer"], b.fingerprint().as_str());
    // The command ended, and it left no control socket and no record.
    let run = dir.path().join("run-a");
    let left: Vec<_> = std::fs::read_dir(&run)
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        !left.iter().any(|n| n.to_string_lossy().contains("ctl")),
        "left in the run directory: {left:?}"
    );
}

#[test]
fn a_missing_element_exits_69_names_it_and_opens_no_carrier() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let allow_a = write_allow(dir.path(), "allow-a", &[&b]);
    // The carrier address is a listener that counts its connections.
    let counter = TcpListener::bind("127.0.0.1:0").unwrap();
    let hits = Arc::new(Mutex::new(0usize));
    let seen = hits.clone();
    let place = counter.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in counter.incoming().flatten() {
            *seen.lock().unwrap() += 1;
            drop(s);
        }
    });
    let to = format!("{}@{place}", b.fingerprint());
    let missing = dir.path().join("none.sock");
    let spec = format!("unix:{}", missing.display());
    let peer = format!("peer:{}/web", a.fingerprint());
    for (spec, name) in [(spec.as_str(), "unix:"), (peer.as_str(), "peer:")] {
        let out = podnet(
            dir.path(),
            "a",
            &[
                "link",
                "dial",
                &to,
                "--carrier",
                "tcp",
                "--allow",
                allow_a.to_str().unwrap(),
                "--egress",
                spec,
            ],
        )
        .output()
        .unwrap();
        let shown = text(&out);
        assert_eq!(out.status.code(), Some(69), "{shown}");
        assert!(shown.contains("element 1"), "{shown}");
        assert!(shown.contains(name), "{shown}");
    }
    assert_eq!(*hits.lock().unwrap(), 0, "no carrier opened");
}

#[test]
fn a_cycle_or_a_duplicate_exits_78_and_too_many_elements_exit_64() {
    let dir = tempfile::tempdir().unwrap();
    let b = install(dir.path(), "b");
    let allow = write_allow(dir.path(), "allow", &[&b]);
    let to = format!("{}@127.0.0.1:7777", b.fingerprint());
    let run = |egress: &[&str]| {
        let mut args = vec![
            "link",
            "dial",
            &to,
            "--carrier",
            "tcp",
            "--allow",
            allow.to_str().unwrap(),
        ];
        for e in egress {
            args.push("--egress");
            args.push(e);
        }
        let out = podnet(dir.path(), "a", &args).output().unwrap();
        (out.status.code(), text(&out))
    };
    // The same element twice.
    let (code, shown) = run(&["socks5://127.0.0.1:1", "socks5://127.0.0.1:1"]);
    assert_eq!(code, Some(78), "{shown}");
    assert!(shown.contains("element 2"), "{shown}");
    // Two spellings of one place: a cycle.
    let (code, shown) = run(&["socks5://127.0.0.1:1", "socks5h://127.0.0.1:1"]);
    assert_eq!(code, Some(78), "{shown}");
    assert!(shown.contains("cycle"), "{shown}");
    // An element at the carrier address: a cycle.
    let (code, shown) = run(&["socks5://127.0.0.1:2", "socks5://127.0.0.1:7777"]);
    assert_eq!(code, Some(78), "{shown}");
    assert!(shown.contains("carrier address"), "{shown}");
    // Nine elements.
    let many: Vec<String> = (1..=9).map(|n| format!("socks5://127.0.0.1:{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let (code, shown) = run(&many);
    assert_eq!(code, Some(64), "{shown}");
    assert!(shown.contains("at most 8"), "{shown}");
    // A network that is not an element.
    let (code, shown) = run(&["relay"]);
    assert_eq!(code, Some(64), "{shown}");
    assert!(shown.contains("not an element"), "{shown}");
}

#[test]
fn a_chain_with_a_carrier_that_opens_no_connection_exits_64() {
    let dir = tempfile::tempdir().unwrap();
    let b = install(dir.path(), "b");
    let allow = write_allow(dir.path(), "allow", &[&b]);
    let out = podnet(
        dir.path(),
        "a",
        &[
            "link",
            "dial",
            "/tmp/none.sock",
            "--allow",
            allow.to_str().unwrap(),
            "--egress",
            "socks5://127.0.0.1:1",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", text(&out));
}
