//! `podnet ssh-relay` (T-165): the CLI of the SSH relay leg. A fake relay on
//! loopback ends the first session; the command opens the next one, and
//! `podnet ps` shows the new URL. A missing flag is a usage error.

mod test_host;

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::keys::ssh_key::rand_core::UnwrapErr;
use russh::keys::ssh_key::Algorithm;
use russh::keys::PrivateKey;
use russh::server::{Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId};
use ssh_key::getrandom::SysRng;

/// One connection of the fake relay. The first connection closes its
/// banner channel at once; the others stay.
struct Conn {
    number: usize,
    channels: Vec<Channel<Msg>>,
}

impl russh::server::Handler for Conn {
    type Error = anyhow::Error;

    async fn auth_publickey(&mut self, _: &str, _: VerifiedPublicKey<'_>) -> anyhow::Result<Auth> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> anyhow::Result<()> {
        self.channels.push(channel);
        reply.accept().await;
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        _: &str,
        port: &mut u32,
        _: &mut Session,
    ) -> anyhow::Result<bool> {
        *port = 40000 + self.number as u32;
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> anyhow::Result<()> {
        session.channel_success(channel)?;
        let banner = format!(
            "  see https://dash.example.test\n  https://tunnel{}.example.test\n",
            self.number
        );
        session.data(channel, banner.into_bytes())?;
        if self.number == 1 {
            session.close(channel)?;
        }
        Ok(())
    }
}

/// Start the fake relay on its own thread. Give its address and the count
/// of the connections.
fn fake_relay() -> (String, Arc<AtomicUsize>) {
    let (address_tx, address_rx) = std::sync::mpsc::channel();
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            address_tx
                .send(listener.local_addr().unwrap().to_string())
                .unwrap();
            let key = PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).unwrap();
            let config = Arc::new(russh::server::Config {
                keys: vec![key],
                auth_rejection_time: Duration::ZERO,
                auth_rejection_time_initial: Some(Duration::ZERO),
                ..Default::default()
            });
            while let Ok((stream, _)) = listener.accept().await {
                let number = counter.fetch_add(1, Ordering::SeqCst) + 1;
                let conn = Conn {
                    number,
                    channels: Vec::new(),
                };
                let config = config.clone();
                tokio::spawn(async move {
                    if let Ok(running) = russh::server::run_stream(config, stream, conn).await {
                        let _ = running.await;
                    }
                });
            }
        });
    });
    (address_rx.recv().unwrap(), count)
}

fn podnet(run: &std::path::Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_RUN_DIR", run)
        .env_remove("RUST_LOG")
        .env_remove("PODNET_RELAY")
        .env_remove("PODNET_ERROR_PROGRAM");
    cmd
}

fn ps(run: &std::path::Path) -> serde_json::Value {
    let out = podnet(run, &["ps", "--json"]).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn ssh_relay_detach_reconnects_and_ps_shows_the_new_url() {
    test_host::need_loopback!();
    let (address, count) = fake_relay();
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let out = podnet(
        &run,
        &[
            "ssh-relay",
            "--server",
            &address,
            "--user",
            "test",
            "--url-skip",
            "dash.example.test",
            "--socket",
            &tmp.path().join("origin.sock").display().to_string(),
            "--detach",
            // The process supervisor does not start the command again:
            // the new URL comes from the session supervisor alone.
            "--no-restart",
            "--name",
            "leg",
        ],
    )
    .output()
    .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    // The ready document names the URL of the first session.
    let ready = String::from_utf8_lossy(&out.stdout);
    assert!(ready.contains("tunnel1.example.test"), "{ready}");
    assert!(ready.contains("\"remote_port\":40001"), "{ready}");

    // The fake relay ended the first session. The command opens the next
    // one, and the record of `podnet ps` has the new URL.
    let until = Instant::now() + Duration::from_secs(20);
    let url = loop {
        let doc = ps(&run);
        let url = doc["instances"][0]["ready"]["url"]
            .as_str()
            .map(String::from);
        if url.as_deref() == Some("https://tunnel2.example.test") {
            break url.unwrap();
        }
        assert!(Instant::now() < until, "ps never showed the new URL: {doc}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(url, "https://tunnel2.example.test");
    let doc = ps(&run);
    assert_eq!(doc["instances"][0]["name"], "leg", "{doc}");
    assert_eq!(doc["instances"][0]["ready"]["remote_port"], 40002, "{doc}");
    assert_eq!(count.load(Ordering::SeqCst), 2);

    let stop = podnet(&run, &["stop", "leg"]).output().unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
}

#[test]
fn ssh_relay_with_a_missing_flag_is_a_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let out = podnet(
        &run,
        &[
            "ssh-relay",
            "--server",
            "relay.example.test:443",
            "--url-skip",
            "dash.example.test",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--user"), "{err}");

    let out = podnet(
        &run,
        &[
            "ssh-relay",
            "--server",
            "relay.example.test:443",
            "--user",
            "podnet",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--url-match") && err.contains("--url-skip"),
        "{err}"
    );
}
