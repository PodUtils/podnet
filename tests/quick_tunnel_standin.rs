//! The binary of podnet against the stand-in of the service and of the edge
//! of pod-cloudflare: no call to Cloudflare. The tests of the quick tunnel
//! itself are in pod-cloudflare; these test the command line: its JSON
//! lines, its signals and its detached instances.

mod standin;
mod test_host;

use std::time::Duration;

use standin::{Cli as _, Process, Standin};

const LONG: Duration = Duration::from_secs(60);

fn hostname_of(ready: &serde_json::Value) -> String {
    let url = ready["url"].as_str().expect("a url");
    url.strip_prefix("https://").expect("an https url").into()
}

#[test]
fn quick_tunnel_standin_prints_json_lines() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let mut tunnel = Process::spawn(standin.tunnel(&["--hello-world", "--json"]));
    let ready = tunnel.ready();
    let url = ready["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://") && url.ends_with(".trycloudflare.com"),
        "{url}"
    );
    assert!(["quic", "http2", "quic+warp", "http2+relay"].contains(&ready["leg"].as_str().unwrap()));
    assert!(ready["tunnel_id"].is_string());
    assert!(ready["locations"].is_array(), "{ready}");
    assert_eq!(ready["locations"][0], "standin1");
    // Each line of standard output is one JSON object.
    tunnel.signal(libc::SIGTERM);
    tunnel.wait(LONG).expect("podnet stops");
    for line in tunnel.out.lock().unwrap().iter() {
        let value: serde_json::Value = serde_json::from_str(line).expect(line);
        assert!(value["event"].is_string(), "{line}");
    }
}

#[test]
fn quick_tunnel_standin_stops_clean_on_sigterm_after_ready() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let mut tunnel =
        Process::spawn(standin.tunnel(&["--hello-world", "--run-for", "60", "--json"]));
    tunnel.ready();
    tunnel.signal(libc::SIGTERM);
    let status = tunnel.wait(LONG).expect("podnet stops");
    assert_eq!(status.code(), Some(0), "{}", tunnel.stderr());
    assert!(tunnel.stderr().contains("SIGTERM: stopping"));
}

#[test]
fn quick_tunnel_standin_detach_ps_logs_and_stop() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let mut start = standin.tunnel(&["--hello-world", "--detach", "--name", "standin"]);
    let output = start.output().expect("podnet --detach");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let doc: serde_json::Value =
        serde_json::from_str(text.lines().last().expect("a line")).expect(&text);
    let host = hostname_of(&doc);
    let pid = doc["pid"].as_i64().expect("a pid") as i32;
    assert_eq!(doc["verified"], false);
    assert!(standin.block_on(standin.edge.wait_registered(&host, Duration::from_secs(20))));

    let (status, _, body) = standin.visit(&host, "/", &[]);
    assert_eq!(status, 200);
    assert!(body.contains("Hello, world"), "{body}");

    let ps = standin.podnet(&["ps", "--json"]).output().unwrap();
    let ps: serde_json::Value = serde_json::from_slice(&ps.stdout).unwrap();
    let state = ps["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "standin")
        .map(|i| i["state"].clone());
    assert_eq!(state, Some("ready".into()), "{ps}");
    // SAFETY: signal 0 only tests that the process exists.
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    let logs = standin
        .podnet(&["logs", "standin", "-n", "5"])
        .output()
        .unwrap();
    assert!(logs.status.success());

    let stop = standin.podnet(&["stop", "standin"]).output().unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    // SAFETY: signal 0 only tests that the process exists.
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the instance runs");
    let ps = standin.podnet(&["ps", "--json"]).output().unwrap();
    let ps: serde_json::Value = serde_json::from_slice(&ps.stdout).unwrap();
    assert_eq!(ps["instances"].as_array().unwrap().len(), 0, "{ps}");
}

/// Live 45169011007: the edge refused the registration of a quick tunnel
/// that the service made some seconds before ("Tunnel not found"). With
/// retry: no, podnet makes one new quick tunnel; each other refusal ends
/// the start at once. With retry: yes (PodUtils/podnet#685), pod-cloudflare
/// tries the same tunnel again for 30 s: no new quick tunnel.
#[test]
fn a_quick_tunnel_that_the_edge_does_not_know_gets_one_new_start() {
    test_host::need_loopback!();
    // (the cause, the refusals with retry: yes before an accept, the edge
    // accepts after the first refusal with retry: no, the provisions, the
    // exit code, the second line has)
    let cases = [
        ("Unauthorized: Tunnel not found", 0, true, 2, None, ""),
        (
            "Unauthorized: Tunnel not found",
            0,
            false,
            2,
            Some(75),
            "does not know the new quick tunnel yet",
        ),
        (
            "Unauthorized: Invalid tunnel secret",
            0,
            false,
            1,
            Some(69),
            "start podnet again to get a new quick tunnel",
        ),
        ("Unauthorized: Tunnel not found", 4, true, 1, None, ""),
    ];
    for (cause, retries, accept, provisions, code, hint) in cases {
        let standin = Standin::start();
        if retries == 0 {
            standin.edge.answer(standin::Reply::Refuse(cause.into()));
        } else {
            standin
                .edge
                .script((0..retries).map(|_| standin::Reply::RefuseRetry {
                    cause: cause.into(),
                    after: Duration::from_secs(1),
                }));
        }
        let edge = standin.edge.clone();
        let flip = std::thread::spawn(move || {
            let end = std::time::Instant::now() + LONG;
            while accept && retries == 0 && std::time::Instant::now() < end {
                if !edge.registrations.lock().unwrap().is_empty() {
                    edge.answer(standin::Reply::Accept);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let mut tunnel = Process::spawn(standin.tunnel(&["--hello-world", "--json"]));
        match code {
            None => {
                let ready = tunnel.ready();
                let issued = standin.api.issued.lock().unwrap().clone();
                let last = issued.last().expect("a quick tunnel");
                assert_eq!(hostname_of(&ready), last.hostname, "{cause}");
                tunnel.signal(libc::SIGTERM);
                tunnel.wait(LONG).expect("podnet stops");
                let stderr = tunnel.stderr();
                assert_eq!(
                    stderr.contains("tries a new quick tunnel again"),
                    retries == 0,
                    "{stderr}"
                );
            }
            Some(code) => {
                let status = tunnel.wait(LONG).expect("podnet exits");
                let stderr = tunnel.stderr();
                assert_eq!(status.code(), Some(code), "{cause}: {stderr}");
                let line = stderr
                    .lines()
                    .find(|l| l.starts_with("podnet: the edge-register hop failed"))
                    .unwrap_or_else(|| panic!("{cause}: no second line: {stderr}"));
                assert!(line.contains(hint), "{cause}: {line}");
                assert!(!line.contains("use a quick tunnel"), "{cause}: {line}");
            }
        }
        flip.join().unwrap();
        assert_eq!(
            standin.api.hits(),
            provisions,
            "{cause}: {}",
            tunnel.stderr()
        );
        // One refused registration for each refused start; after the
        // accept, the other edge connections register too.
        let registrations = standin.edge.registrations.lock().unwrap().clone();
        let refused = registrations.iter().filter(|r| !r.accepted).count();
        let refused_starts = match (retries, accept) {
            (0, true) => 1,
            (0, false) => provisions,
            (n, _) => n,
        };
        assert_eq!(refused, refused_starts, "{cause}: {registrations:?}");
        let accepted = registrations.iter().any(|r| r.accepted);
        assert_eq!(accepted, accept, "{cause}: {registrations:?}");
    }
}
