//! GitHub #746: the UDP forwards and the DNS door of `podnet forward` over
//! the way of the process.

use std::process::{Command, Output};

mod test_host;

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// GitHub #746: with no proxy the way of the process carries UDP: a UDP
/// forward gives the answer of its target, and the DNS door reaches its
/// upstream (the echo answers with the query, so the door answers it).
#[test]
fn forward_carries_udp_over_the_way_of_the_process() {
    test_host::need_loopback!();
    let echo = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let upstream = echo.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        while let Ok((n, from)) = echo.recv_from(&mut buf) {
            let _ = echo.send_to(&buf[..n], from);
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let udp_port = || {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    };
    let (fwd, door) = (udp_port(), udp_port());
    let forward = format!("udp:{fwd}={upstream}");
    let door_addr = format!("127.0.0.1:{door}");
    let upstream_text = upstream.to_string();
    let cases: [(Vec<&str>, u16); 2] = [
        (
            vec!["--no-proxy", "forward", "-L", &forward, "--run-for", "5"],
            fwd,
        ),
        (
            vec![
                "--no-proxy",
                "forward",
                "--dns",
                &door_addr,
                "--dns-upstream",
                &upstream_text,
                "--run-for",
                "5",
            ],
            door,
        ),
    ];
    // A DNS query of example.com (id 0x4321, RD).
    let query: Vec<u8> = [
        &[0x43, 0x21, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0][..],
        b"\x07example\x03com\x00",
        &[0, 1, 0, 1],
    ]
    .concat();
    for (args, port) in cases {
        let child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(&args)
            .env("PODNET_RUN_DIR", dir.path().join("run"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .env_remove("RUST_LOG")
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_millis(300)))
            .unwrap();
        let mut got = None;
        for _ in 0..12 {
            let _ = client.send_to(&query, ("127.0.0.1", port));
            let mut buf = [0u8; 1500];
            if let Ok(n) = client.recv(&mut buf) {
                got = Some(buf[..n].to_vec());
                break;
            }
        }
        let out = child.wait_with_output().unwrap();
        let got = got.unwrap_or_else(|| panic!("{args:?}: no answer: {}", text(&out)));
        assert_eq!(&got[..2], &[0x43, 0x21], "{args:?}: {got:?}");
        assert!(
            !text(&out).contains("carries no UDP"),
            "{args:?}: {}",
            text(&out)
        );
    }
}
