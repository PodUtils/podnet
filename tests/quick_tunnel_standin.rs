//! The quick tunnel against a stand-in of the service and of the edge
//! (T-316): no call to Cloudflare. `tests/standin/` has the stand-in; the
//! steps of `live.yml` that test podnet and not Cloudflare run here on each
//! push.

mod standin;
mod test_host;

use std::time::{Duration, Instant};

use standin::{Answer, Process, Reply, Standin};

const LONG: Duration = Duration::from_secs(60);

fn hostname_of(ready: &serde_json::Value) -> String {
    let url = ready["url"].as_str().expect("a url");
    url.strip_prefix("https://").expect("an https url").into()
}

#[test]
fn quick_tunnel_standin_serves_a_visitor_over_http2() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let (port, seen) = standin::origin("origin says hello\n");
    let url = format!("http://127.0.0.1:{port}");
    let mut tunnel = Process::spawn(standin.tunnel(&["--url", &url, "--json"]));
    let ready = tunnel.ready();
    let host = hostname_of(&ready);
    assert!(host.ends_with(".trycloudflare.com"), "{host}");
    assert_eq!(ready["leg"], "http2");
    assert!(standin.block_on(standin.edge.wait_registered(&host, Duration::from_secs(20))));

    let issued = standin.api.issued.lock().unwrap().clone();
    assert_eq!(issued.len(), 1);
    assert_eq!(ready["tunnel_id"], issued[0].id.to_string());
    assert_eq!(host, issued[0].hostname);

    let (status, headers, body) = standin.visit(&host, "/some/path?q=1", &[("X-Visitor", "yes")]);
    assert_eq!((status, body.as_str()), (200, "origin says hello\n"));
    assert!(
        headers
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case("x-origin") && v == "standin"),
        "{headers:?}"
    );
    let head = seen
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the origin got a request");
    assert!(head.starts_with("GET /some/path?q=1 "), "{head}");
    assert!(head.to_lowercase().contains("x-visitor: yes"), "{head}");

    // The edge got the tunnel secret on the wire, verbatim.
    let registrations = standin.edge.registrations.lock().unwrap().clone();
    assert_eq!(registrations.len(), 1, "{registrations:?}");
    assert!(registrations[0].accepted);
    assert_eq!(registrations[0].account_tag, issued[0].account_tag);
    assert_eq!(registrations[0].secret, issued[0].secret);
    assert_eq!(registrations[0].tunnel_id, issued[0].id);

    // A stop signal ends the process with 0 and the connection is
    // unregistered.
    tunnel.signal(libc::SIGTERM);
    let status = tunnel.wait(LONG).expect("podnet stops");
    assert_eq!(status.code(), Some(0), "{}", tunnel.stderr());
    assert!(
        tunnel.stderr().contains("SIGTERM: stopping"),
        "{}",
        tunnel.stderr()
    );
    assert_eq!(standin.edge.connected(&host), 0);
    assert!(
        standin
            .edge
            .unregistrations
            .load(std::sync::atomic::Ordering::SeqCst)
            >= 1
    );
    let (status, _, body) = standin.visit(&host, "/", &[]);
    assert_eq!(status, 530, "{body}");
}

#[test]
fn quick_tunnel_standin_retries_after_429() {
    test_host::need_loopback!();
    let standin = Standin::start();
    standin.api.script([Answer::TooMany {
        retry_after: Some(1),
    }]);
    let started = Instant::now();
    let mut tunnel = Process::spawn(standin.tunnel(&["--hello-world", "--json"]));
    let ready = tunnel.ready();
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert_eq!(standin.api.hits(), 2, "the first request got 429");
    assert!(
        tunnel.stderr().contains("provisioning failed"),
        "{}",
        tunnel.stderr()
    );
    let host = hostname_of(&ready);
    assert!(standin.block_on(standin.edge.wait_registered(&host, Duration::from_secs(20))));
    tunnel.signal(libc::SIGTERM);
    assert_eq!(tunnel.wait(LONG).and_then(|s| s.code()), Some(0));
}

#[test]
fn quick_tunnel_standin_exits_75_when_retry_after_exceeds_the_budget() {
    test_host::need_loopback!();
    let standin = Standin::start();
    standin.api.always(Answer::TooMany {
        retry_after: Some(3600),
    });
    let mut tunnel = Process::spawn(standin.tunnel(&["--hello-world"]));
    let status = tunnel.wait(LONG).expect("podnet gives up at once");
    assert_eq!(status.code(), Some(75), "{}", tunnel.stderr());
    let err = tunnel.stderr();
    assert!(err.contains("429") && err.contains("3600"), "{err}");
    assert_eq!(standin.api.hits(), 1, "no second request");
    assert!(standin.edge.registrations.lock().unwrap().is_empty());
}

#[test]
fn quick_tunnel_standin_does_not_retry_a_403() {
    test_host::need_loopback!();
    let standin = Standin::start();
    standin.api.always(Answer::Status(403));
    let mut tunnel = Process::spawn(standin.tunnel(&["--hello-world"]));
    let status = tunnel.wait(LONG).expect("podnet stops");
    assert_eq!(status.code(), Some(69), "{}", tunnel.stderr());
    assert_eq!(standin.api.hits(), 1);
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

#[test]
fn quick_tunnel_standin_checks_the_auth_user() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let pass = standin.dir.path().join("pass");
    std::fs::write(&pass, "standin-secret\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&pass, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut tunnel = Process::spawn(standin.tunnel(&[
        "--hello-world",
        "--json",
        "--auth-user",
        "live",
        "--auth-password-file",
        pass.to_str().unwrap(),
    ]));
    let host = hostname_of(&tunnel.ready());
    assert!(standin.block_on(standin.edge.wait_registered(&host, Duration::from_secs(20))));
    let basic = |user: &str, password: &str| {
        let pair = format!("{user}:{password}");
        // HTTP Basic uses the padded form of base64.
        let mut text = podnet::cloudflare::http2::base64_encode(pair.as_bytes());
        while !text.len().is_multiple_of(4) {
            text.push('=');
        }
        format!("Basic {text}")
    };
    let none = standin.visit(&host, "/", &[]);
    assert_eq!(none.0, 401, "{none:?}");
    let wrong = standin.visit(&host, "/", &[("Authorization", &basic("live", "wrong!"))]);
    assert_eq!(wrong.0, 401, "{wrong:?}");
    let right = standin.visit(
        &host,
        "/",
        &[("Authorization", &basic("live", "standin-secret"))],
    );
    assert_eq!(right.0, 200, "{right:?}");
    assert!(right.2.contains("Hello, world"), "{}", right.2);
    tunnel.signal(libc::SIGTERM);
    tunnel.wait(LONG).expect("podnet stops");
}

#[test]
fn quick_tunnel_standin_goes_through_a_connect_proxy() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let (proxy, log) = standin.proxy();
    let mut command = standin.podnet(&[
        "tunnel",
        "--service-url",
        &standin.api_url,
        "--transport",
        "direct",
        "--no-verify",
        "--hello-world",
        "--json",
    ]);
    command.env("HTTPS_PROXY", format!("http://{proxy}"));
    command.env("HTTP_PROXY", format!("http://{proxy}"));
    let mut tunnel = Process::spawn(command);
    let host = hostname_of(&tunnel.ready());
    assert!(standin.block_on(standin.edge.wait_registered(&host, Duration::from_secs(20))));
    // The edge leg went through the proxy, and no QUIC leg was tried.
    let seen = log.lock().unwrap().clone();
    assert!(seen.contains(&standin.edge_addr.to_string()), "{seen:?}");
    assert!(
        tunnel.stderr().contains("no proxy carries UDP"),
        "{}",
        tunnel.stderr()
    );
    tunnel.signal(libc::SIGTERM);
    tunnel.wait(LONG).expect("podnet stops");
}

#[test]
fn quick_tunnel_standin_exits_when_the_edge_refuses_the_registration() {
    test_host::need_loopback!();
    let standin = Standin::start();
    standin
        .edge
        .answer(Reply::Refuse("the tunnel is deleted".into()));
    let mut tunnel = Process::spawn(standin.tunnel(&["--hello-world"]));
    let status = tunnel.wait(LONG).expect("podnet gives up");
    assert!(!status.success(), "{}", tunnel.stderr());
    assert!(
        tunnel.stderr().contains("the tunnel is deleted"),
        "{}",
        tunnel.stderr()
    );
    let registrations = standin.edge.registrations.lock().unwrap().clone();
    assert!(!registrations.is_empty() && registrations.iter().all(|r| !r.accepted));
}

#[test]
fn quick_tunnel_standin_front_answers_530_for_an_unknown_host() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let (status, _, body) = standin.visit("nobody.trycloudflare.com", "/", &[]);
    assert_eq!(status, 530);
    assert!(body.contains("1033"), "{body}");
}

#[test]
fn quick_tunnel_standin_falls_back_to_http2_when_quic_has_no_listener() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let mut command = standin.podnet(&[
        "tunnel",
        "--service-url",
        &standin.api_url,
        "--transport",
        "direct",
        "--no-verify",
        "--hello-world",
        "--json",
    ]);
    // The stand-in listens on TCP only: the QUIC leg gets no answer.
    command.env("PODNET_UDP", "on");
    let mut tunnel = Process::spawn(command);
    let ready = tunnel.ready();
    assert_eq!(ready["leg"], "http2", "{}", tunnel.stderr());
    // The log line of the race can come a little after the ready line.
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !tunnel.stderr().contains("the leg won the race") && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    let err = tunnel.stderr();
    assert!(err.contains("the leg won the race"), "{err}");
    assert!(err.contains("leg=http2"), "{err}");
    tunnel.signal(libc::SIGTERM);
    tunnel.wait(LONG).expect("podnet stops");
}

#[test]
fn quick_tunnel_standin_edge_certificate_needs_its_ca() {
    test_host::need_loopback!();
    let standin = Standin::start();
    // Without the roots of the test, the certificate of the stand-in edge is
    // not valid: podnet adds no root for a test, and the registration fails.
    let mut command = standin.tunnel(&["--hello-world"]);
    command.env_remove("SSL_CERT_FILE");
    let mut tunnel = Process::spawn(command);
    let status = tunnel.wait(LONG).expect("podnet gives up");
    assert!(!status.success(), "{}", tunnel.stderr());
    assert!(
        standin.edge.registrations.lock().unwrap().is_empty(),
        "the edge got a registration"
    );
}

/// UDP 7844 dropped with no answer (the step "Quick tunnel with UDP 7844
/// dropped" of `live.yml`, T-316): a UDP socket on the port of the edge
/// takes each QUIC packet and answers none. `--protocol auto` registers
/// over HTTP/2 after the stagger, and the log names the leg that won.
#[test]
fn quick_tunnel_standin_falls_back_to_http2_when_udp_is_dropped() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let silent =
        std::net::UdpSocket::bind(standin.edge_addr).expect("a UDP socket on the edge port");
    let mut command = standin.podnet(&[
        "tunnel",
        "--service-url",
        &standin.api_url,
        "--transport",
        "direct",
        "--no-verify",
        "--hello-world",
        "--json",
    ]);
    command.env("PODNET_UDP", "on");
    let mut tunnel = Process::spawn(command);
    let ready = tunnel.ready();
    assert_eq!(ready["leg"], "http2", "{}", tunnel.stderr());
    let until = Instant::now() + Duration::from_secs(10);
    while !tunnel.stderr().contains("the leg won the race") && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    let err = tunnel.stderr();
    assert!(err.contains("the leg won the race"), "{err}");
    assert!(err.contains("leg=http2"), "{err}");
    // The QUIC leg sent its packets to the silent socket.
    silent
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut buf = [0u8; 2048];
    assert!(silent.recv_from(&mut buf).is_ok(), "no QUIC packet came");
    tunnel.signal(libc::SIGTERM);
    tunnel.wait(LONG).expect("podnet stops");
}

/// A proxy at the standard input and output of a command (issue 129; the
/// step "Quick tunnel through a local CONNECT proxy" of `live.yml`, T-316):
/// `--proxy 'http+exec:COMMAND'` with one command for each connection. The
/// command is a pipe in Python from its standard streams to the CONNECT
/// proxy of the stand-in. The service and the edge go through it.
#[test]
fn quick_tunnel_standin_goes_through_an_exec_proxy() {
    test_host::need_loopback!();
    let standin = Standin::start();
    let (proxy, log) = standin.proxy();
    let pipe = standin.dir.path().join("pipe.py");
    std::fs::write(
        &pipe,
        "import socket, sys, threading\n\
         s = socket.create_connection((sys.argv[1], int(sys.argv[2])))\n\
         def up():\n\
         \x20   while True:\n\
         \x20       b = sys.stdin.buffer.raw.read(65536)\n\
         \x20       if not b:\n\
         \x20           s.shutdown(socket.SHUT_WR)\n\
         \x20           return\n\
         \x20       s.sendall(b)\n\
         threading.Thread(target=up, daemon=True).start()\n\
         while True:\n\
         \x20   b = s.recv(65536)\n\
         \x20   if not b:\n\
         \x20       break\n\
         \x20   sys.stdout.buffer.write(b)\n\
         \x20   sys.stdout.buffer.flush()\n",
    )
    .unwrap();
    let spec = format!(
        "http+exec:python3 -I {} {} {}",
        pipe.display(),
        proxy.ip(),
        proxy.port()
    );
    let mut command = standin.podnet(&[
        "--proxy",
        &spec,
        "tunnel",
        "--service-url",
        &standin.api_url,
        "--transport",
        "direct",
        "--no-verify",
        "--hello-world",
        "--json",
    ]);
    command.env("PODNET_UDP", "on");
    let mut tunnel = Process::spawn(command);
    let host = hostname_of(&tunnel.ready());
    assert!(standin.block_on(standin.edge.wait_registered(&host, Duration::from_secs(20))));
    let seen = log.lock().unwrap().clone();
    let api = standin.api_url.trim_start_matches("http://").to_string();
    assert!(seen.contains(&api), "{seen:?}");
    assert!(seen.contains(&standin.edge_addr.to_string()), "{seen:?}");
    tunnel.signal(libc::SIGTERM);
    tunnel.wait(LONG).expect("podnet stops");
}
