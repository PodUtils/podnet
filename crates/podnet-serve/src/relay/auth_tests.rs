//! The login to a relay (T-318): the methods that the server offers, and an
//! RSA host key. The servers here run on loopback.

use std::sync::Arc;
use std::time::Duration;

use russh::keys::PrivateKey;
use russh::server::{Auth, ChannelOpenHandle, Msg, Response, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet};

use super::*;
use crate::relay::probe::{probe, Stage};

/// What the server does at a login.
#[derive(Clone, Copy, Debug)]
enum Login {
    /// It offers keyboard-interactive only, and takes an empty answer.
    KeyboardOnly,
    /// It offers `none`, and accepts it.
    NoneAccepted,
    /// It offers password only, and refuses each try.
    PasswordOnly,
    /// It takes any key.
    AnyKey,
    /// It takes any key, and refuses a forward of port 0 (Sandhole).
    NeedsPort,
}

struct Conn {
    login: Login,
    channels: Vec<Channel<Msg>>,
}

impl russh::server::Handler for Conn {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        _: &str,
        _: russh::server::VerifiedPublicKey<'_>,
    ) -> anyhow::Result<Auth> {
        Ok(match self.login {
            Login::AnyKey | Login::NeedsPort => Auth::Accept,
            _ => Auth::reject(),
        })
    }

    async fn auth_none(&mut self, _: &str) -> anyhow::Result<Auth> {
        Ok(match self.login {
            Login::NoneAccepted => Auth::Accept,
            _ => Auth::reject(),
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> anyhow::Result<Auth> {
        if !matches!(self.login, Login::KeyboardOnly) {
            return Ok(Auth::reject());
        }
        let Some(response) = response else {
            return Ok(Auth::Partial {
                name: "".into(),
                instructions: "".into(),
                prompts: vec![("Name: ".into(), true)].into(),
            });
        };
        // Accept an empty answer only.
        Ok(if response.into_iter().all(|a| a.is_empty()) {
            Auth::Accept
        } else {
            Auth::reject()
        })
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
        if matches!(self.login, Login::NeedsPort) {
            // Port 0 is a refusal; another port is the port of the forward.
            return Ok(*port != 0);
        }
        *port = 40001;
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> anyhow::Result<()> {
        session.channel_success(channel)?;
        session.data(channel, b"  https://tunnel.example.test\n".to_vec())?;
        Ok(())
    }
}

/// Start a server on loopback with `key` as its host key. Gives its address.
async fn serve(login: Login, key: PrivateKey) -> String {
    let methods: &[MethodKind] = match login {
        Login::KeyboardOnly => &[MethodKind::KeyboardInteractive],
        Login::NoneAccepted => &[MethodKind::None],
        Login::PasswordOnly => &[MethodKind::Password],
        Login::AnyKey | Login::NeedsPort => &[MethodKind::PublicKey],
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let config = Arc::new(russh::server::Config {
        keys: vec![key],
        methods: MethodSet::from(methods),
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let conn = Conn {
                login,
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
    address
}

fn ed25519() -> PrivateKey {
    use russh::keys::ssh_key::rand_core::UnwrapErr;
    use russh::keys::ssh_key::Algorithm;
    PrivateKey::random(
        &mut UnwrapErr(ssh_key::getrandom::SysRng),
        Algorithm::Ed25519,
    )
    .unwrap()
}

/// A throwaway RSA key of 4096 bits, made for this test only.
fn rsa4096() -> PrivateKey {
    PrivateKey::from_openssh(include_str!("rsa4096.test-key")).unwrap()
}

fn config(address: String) -> RelayConfig {
    RelayConfig {
        address,
        username: "tester".into(),
        url_rule: UrlRule {
            skips: vec![],
            matches: vec![],
        },
        inactivity: None,
        ..RelayConfig::default()
    }
}

const LIMIT: Duration = Duration::from_secs(20);

#[tokio::test]
async fn a_relay_that_offers_keyboard_interactive_only_takes_the_empty_answer() {
    pod_testkit::need_loopback!();
    let address = serve(Login::KeyboardOnly, ed25519()).await;
    let live = open(&config(address))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(live.tunnel.url, "https://tunnel.example.test");
    assert_eq!(live.tunnel.remote_port, 40001);
}

#[tokio::test]
async fn a_relay_that_accepts_none_takes_no_key() {
    pod_testkit::need_loopback!();
    let address = serve(Login::NoneAccepted, ed25519()).await;
    let live = open(&config(address))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(live.tunnel.remote_port, 40001);
}

#[tokio::test]
async fn a_relay_with_an_rsa_host_key_completes_the_handshake() {
    pod_testkit::need_loopback!();
    let address = serve(Login::AnyKey, rsa4096()).await;
    let live = open(&config(address))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(live.tunnel.url, "https://tunnel.example.test");
}

#[tokio::test]
async fn a_relay_with_no_usable_method_names_the_offered_methods() {
    pod_testkit::need_loopback!();
    let address = serve(Login::PasswordOnly, ed25519()).await;
    let error = match open(&config(address)).await {
        Ok(_) => panic!("the relay took a login"),
        Err(e) => e.to_string(),
    };
    assert!(error.contains("password"), "{error}");
    assert!(error.contains("tester"), "{error}");
}

#[tokio::test]
async fn the_probe_names_the_offered_methods_of_a_relay_that_takes_none() {
    pod_testkit::need_loopback!();
    let address = serve(Login::PasswordOnly, ed25519()).await;
    let got = probe(&config(address), LIMIT).await;
    let failure = got.failure.expect("a failure");
    assert_eq!(failure.stage, Stage::Auth);
    assert_eq!(got.auth, Some(false));
    assert!(failure.reason.contains("password"), "{failure:?}");
}

#[tokio::test]
async fn the_probe_logs_in_with_keyboard_interactive() {
    pod_testkit::need_loopback!();
    let address = serve(Login::KeyboardOnly, ed25519()).await;
    let got = probe(&config(address), LIMIT).await;
    assert_eq!(got.failure, None);
    assert_eq!(got.auth, Some(true));
    assert_eq!(got.method, Some("keyboard-interactive"));
}

#[tokio::test]
async fn the_probe_completes_the_handshake_with_an_rsa_host_key() {
    pod_testkit::need_loopback!();
    let address = serve(Login::AnyKey, rsa4096()).await;
    let got = probe(&config(address), LIMIT).await;
    assert_eq!(got.failure, None);
    assert_eq!(got.method, Some("publickey"));
}

#[tokio::test]
async fn a_relay_that_refuses_port_0_takes_the_remote_port_of_the_config() {
    pod_testkit::need_loopback!();
    let address = serve(Login::NeedsPort, ed25519()).await;
    // No port: the request is port 0, and the relay refuses it.
    let error = match open(&config(address.clone())).await {
        Ok(_) => panic!("the relay took port 0"),
        Err(e) => e.to_string(),
    };
    assert!(error.contains("remote forward refused"), "{error}");
    // Port 80 is the port of the forward.
    let live = open(&RelayConfig {
        remote_port: Some(80),
        ..config(address)
    })
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(live.tunnel.remote_port, 80);
}

/// The fact `remote-port` of the relay is the port of the config, and the
/// relay that refuses port 0 takes it.
#[tokio::test]
async fn a_relay_that_refuses_port_0_takes_the_remote_port_fact() {
    pod_testkit::need_loopback!();
    let address = serve(Login::NeedsPort, ed25519()).await;
    let fact = facts::lookup("sandhole").unwrap().remote_port;
    let live = open(&RelayConfig {
        remote_port: fact,
        ..config(address)
    })
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(live.tunnel.remote_port, 80);
}
