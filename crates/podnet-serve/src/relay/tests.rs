//! The tests of the SSH relay leg and of its supervisor.

use super::*;

/// #230: the default origin socket is in a private directory of this
/// user, not a fixed path of the shared /tmp.
#[test]
fn the_default_origin_socket_is_private() {
    use std::os::unix::fs::MetadataExt as _;
    let path = RelayConfig::default().unix_socket;
    assert_ne!(path, std::path::PathBuf::from("/tmp/podnet-origin.sock"));
    if let Some(dir) = path
        .parent()
        .filter(|d| d.is_dir() && *d != std::env::temp_dir())
    {
        let meta = std::fs::metadata(dir).unwrap();
        assert_eq!(meta.uid(), unsafe { libc::geteuid() });
        assert_eq!(meta.mode() & 0o077, 0, "{}", dir.display());
    }
}

#[test]
fn splits_a_valid_host_port() {
    let (host, port) = split_address("free.pinggy.io:443").expect("should split");
    assert_eq!(host, "free.pinggy.io");
    assert_eq!(port, 443);
}

#[test]
fn rejects_addresses_without_a_port() {
    for bad in ["free.pinggy.io", "", "host:not-a-port", ":443"] {
        assert!(
            split_address(bad).is_err(),
            "{bad:?} should be rejected as an address"
        );
    }
}

#[test]
fn default_config_targets_port_443() {
    // 443, not 22: some egress proxies permit only 443.
    let cfg = RelayConfig::default();
    assert!(
        cfg.address.ends_with(":443"),
        "default relay must be on 443"
    );
}

#[test]
fn picks_the_tunnel_url_and_not_the_dashboard() {
    // The shape of the banner of free.pinggy.io (read on 2026-10-09). The
    // facts of the relay hold the tunnel domain; the rule takes the URL of it.
    let line = "  You are not authenticated.\n  Your tunnel will expire in 60 minutes. \
Upgrade to Pinggy Pro to get unrestricted tunnels. https://dashboard.pinggy.io\n  \
http://jcydy-2400-1a00-5b2f-821c-869e-56ff-fe03-2b71.run.pinggy-free.link\n  \
https://jcydy-2400-1a00-5b2f-821c-869e-56ff-fe03-2b71.run.pinggy-free.link\n";
    let url = UrlRule::pick(&facts::default_rule(), line).expect("a tunnel url");
    assert_eq!(
        url,
        "https://jcydy-2400-1a00-5b2f-821c-869e-56ff-fe03-2b71.run.pinggy-free.link"
    );
    assert!(
        !url.contains("dashboard"),
        "dashboard must never be chosen: {url}"
    );
}

#[test]
fn trailing_punctuation_is_stripped_from_a_url() {
    let line = "visit https://x.run.pinggy-free.link.";
    assert_eq!(
        facts::default_rule().pick(line).as_deref(),
        Some("https://x.run.pinggy-free.link")
    );
}

#[test]
fn a_dashboard_only_line_yields_no_tunnel_url() {
    // Guards against silently returning the dashboard as if it were a tunnel.
    let line = "Upgrade for more at https://dashboard.pinggy.io\n";
    assert_eq!(facts::default_rule().pick(line), None);
}

fn data(text: &str) -> ChannelMsg {
    ChannelMsg::Data {
        data: text.as_bytes().to_vec().into(),
    }
}

/// The URL that the banner reader gives after the frames, or the error.
fn read_frames(rule: &UrlRule, frames: Vec<ChannelMsg>) -> Result<String, RelayError> {
    let mut banner = Banner::new(rule.clone());
    for frame in frames {
        match feed(&mut banner, Some(frame)) {
            Step::More => {}
            Step::Url(url) => return Ok(url),
            Step::End => return Err(RelayError::NoUrl),
        }
    }
    Err(RelayError::NoUrl)
}

#[test]
fn url_across_two_frames() {
    let rule = facts::default_rule();
    let frames = vec![
        data("pitch https://dashboard.pinggy.io\n  https://jcydy-1.run.ping"),
        data("gy-free.link\n"),
    ];
    assert_eq!(
        read_frames(&rule, frames).unwrap(),
        "https://jcydy-1.run.pinggy-free.link"
    );
    // A frame that ends inside the scheme.
    let frames = vec![
        data("go to ht"),
        data("tps://a.run.pinggy-free.link "),
        data("x"),
    ];
    assert_eq!(
        read_frames(&rule, frames).unwrap(),
        "https://a.run.pinggy-free.link"
    );
}

#[test]
fn a_banner_with_no_end_of_line_is_whole_when_the_channel_closes() {
    let rule = facts::default_rule();
    let frames = vec![
        data("https://a.run.pinggy-"),
        data("free.link"),
        ChannelMsg::Eof,
    ];
    assert_eq!(
        read_frames(&rule, frames).unwrap(),
        "https://a.run.pinggy-free.link"
    );
}

#[test]
fn a_banner_with_only_a_dashboard_ends_with_no_url() {
    let rule = facts::default_rule();
    let frames = vec![
        data("https://dash"),
        data("board.pinggy.io\n"),
        ChannelMsg::Close,
    ];
    assert!(matches!(read_frames(&rule, frames), Err(RelayError::NoUrl)));
}

#[tokio::test]
async fn copy_bidirectional_carries_request_and_response_on_one_origin_connection() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    // podnet_channel: the SSH channel the relay opened (relay <-> podnet).
    let (mut relay, mut podnet_channel) = tokio::io::duplex(4096);
    // podnet_origin: the unix-socket origin (origin <-> podnet).
    let (mut origin, mut podnet_origin) = tokio::io::duplex(4096);

    // The relay writes the visitor's request, then reads the reply back.
    let relay_task = tokio::spawn(async move {
        relay
            .write_all(b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        relay.shutdown().await.unwrap();
        let mut back = Vec::new();
        relay.read_to_end(&mut back).await.unwrap();
        back
    });

    // The origin reads to end-of-request, replies, then closes, like an
    // HTTP/1.1 server that answers one request and closes the connection.
    let origin_task = tokio::spawn(async move {
        let mut request = Vec::new();
        origin.read_to_end(&mut request).await.unwrap();
        origin
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
            .await
            .unwrap();
        origin.shutdown().await.unwrap();
        request
    });

    // podnet splices the channel into the origin, both directions, bounded so
    // a regression fails the test instead of hanging the suite.
    let (mut cr, mut cw) = tokio::io::split(&mut podnet_channel);
    let (mut or, mut ow) = tokio::io::split(&mut podnet_origin);
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        copy_bidirectional(&mut cr, &mut cw, &mut or, &mut ow),
    )
    .await
    .expect("splice must not hang")
    .expect("splice completes");

    let request = origin_task.await.unwrap();
    assert!(
        String::from_utf8_lossy(&request).contains("GET /hello"),
        "origin should have received the request, got {:?}",
        String::from_utf8_lossy(&request)
    );

    let to_relay = relay_task.await.unwrap();
    let response = String::from_utf8_lossy(&to_relay);
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "relay should have received the status line, got {response:?}"
    );
    assert!(
        response.ends_with("OK"),
        "relay should have received the response body, got {response:?}"
    );
}

#[test]
fn a_url_rule_skips_and_holds_texts() {
    let banner = "pitch https://dash.example.test and then https://a.other.test \
                  https://b.tunnel.test\n";
    let rule = UrlRule {
        skips: vec!["dash.example.test".into()],
        matches: vec![],
    };
    assert_eq!(rule.pick(banner).as_deref(), Some("https://a.other.test"));
    let rule = UrlRule {
        skips: vec!["dash.example.test".into()],
        matches: vec![".tunnel.test".into()],
    };
    assert_eq!(rule.pick(banner).as_deref(), Some("https://b.tunnel.test"));
    let rule = UrlRule {
        skips: vec![],
        matches: vec!["nothing".into()],
    };
    assert_eq!(rule.pick(banner), None);
}

#[test]
fn the_facts_of_the_default_relay_come_from_pod_services() {
    let f = facts::default_facts();
    assert_eq!(f.name, facts::DEFAULT);
    assert_eq!(f.addresses, [default_relay()]);
    assert!(f.user.is_some());
    assert!(f.keepalive.is_some());
    assert!(!f.rule.matches.is_empty());
}

#[test]
fn the_known_relays_have_facts_and_an_unknown_name_lists_them() {
    let names = facts::names();
    assert!(
        names.contains(&"pinggy") && names.contains(&"serveo"),
        "{names:?}"
    );
    for name in &names {
        let f = facts::lookup(name).unwrap();
        assert!(!f.addresses.is_empty(), "{name}");
        assert!(
            f.addresses.iter().all(|a| check_address(a).is_ok()),
            "{name}"
        );
    }
    let err = facts::lookup("no-such-relay").unwrap_err();
    assert!(
        err.contains("no-such-relay") && err.contains("pinggy"),
        "{err}"
    );
}

#[test]
fn the_rule_of_each_relay_skips_every_known_dashboard() {
    // Approach 2: the dashboard of Pinggy is skipped on another relay too.
    let dashboards = facts::dashboards();
    assert!(dashboards.iter().any(|d| d.contains("dashboard.pinggy.io")));
    for name in facts::names() {
        let rule = facts::lookup(name).unwrap().rule;
        for d in &dashboards {
            assert!(rule.skips.contains(d), "{name} does not skip {d}");
            assert!(!rule.accepts(&format!("https://{d}")), "{name}");
        }
    }
    let serveo = facts::lookup("serveo").unwrap().rule;
    let banner = "https://dashboard.pinggy.io https://abc.serveo.net\n";
    assert_eq!(
        serveo.pick(banner).as_deref(),
        Some("https://abc.serveo.net")
    );
}

#[test]
fn a_relay_with_more_than_one_endpoint_gives_the_address_list_in_order() {
    let f = facts::lookup("serveo").unwrap();
    assert!(f.addresses.len() > 1, "{:?}", f.addresses);
    assert!(f.addresses[0].ends_with(":443"), "{:?}", f.addresses);
}

#[tokio::test]
async fn the_transport_tries_the_address_list_in_order() {
    pod_testkit::need_loopback!();
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_address = dead.local_addr().unwrap().to_string();
    drop(dead);
    let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let live_address = live.local_addr().unwrap().to_string();
    let accept = tokio::spawn(async move { live.accept().await.is_ok() });
    let config = RelayConfig {
        address: dead_address,
        fallbacks: vec![live_address],
        connect_timeout: Duration::from_secs(5),
        ..RelayConfig::default()
    };
    connect_transport(&config)
        .await
        .expect("the second address answers");
    assert!(accept.await.unwrap());
    // No address answers: the error is the one of the last address.
    let config = RelayConfig {
        fallbacks: vec![],
        ..config
    };
    assert!(matches!(
        connect_transport(&config).await,
        Err(RelayError::Transport(_))
    ));
}

#[test]
fn the_backoff_doubles_to_its_limit() {
    use supervise::Backoff;
    let backoff = Backoff::default();
    let mut delay = backoff.initial;
    let mut seen = Vec::new();
    for _ in 0..7 {
        seen.push(delay.as_secs());
        delay = backoff.next(delay);
    }
    assert_eq!(seen, [1, 2, 4, 8, 16, 30, 30]);
}

mod fake {
    //! A relay on loopback: it takes any key, allows one remote forward,
    //! and writes a banner with a URL that holds the number of the
    //! connection.

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use russh::keys::ssh_key::rand_core::UnwrapErr;
    use russh::keys::ssh_key::Algorithm;
    use russh::keys::PrivateKey;
    use russh::server::{Auth, ChannelOpenHandle, Msg, Session, VerifiedPublicKey};
    use russh::{Channel, ChannelId, Disconnect};
    use ssh_key::getrandom::SysRng;

    /// How the first connection ends after its banner.
    #[derive(Clone, Copy, Debug)]
    pub enum End {
        /// The relay closes the banner channel.
        Channel,
        /// The relay ends the connection.
        Connection,
    }

    struct Conn {
        number: usize,
        end: End,
        reject: bool,
        channels: Vec<Channel<Msg>>,
    }

    impl russh::server::Handler for Conn {
        type Error = anyhow::Error;

        async fn auth_publickey(
            &mut self,
            _: &str,
            _: VerifiedPublicKey<'_>,
        ) -> anyhow::Result<Auth> {
            if self.reject {
                return Ok(Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                });
            }
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
                match self.end {
                    End::Channel => session.close(channel)?,
                    End::Connection => {
                        session.disconnect(Disconnect::ByApplication, "bye", "")?;
                    }
                }
            }
            Ok(())
        }
    }

    /// Start the relay. Give its address and the count of connections.
    pub async fn start(end: End) -> (String, Arc<AtomicUsize>) {
        start_with(end, false).await
    }

    /// As [`start`]; with `reject` the relay refuses each key.
    pub async fn start_with(end: End, reject: bool) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let key = PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).unwrap();
        let config = Arc::new(russh::server::Config {
            keys: vec![key],
            auth_rejection_time: std::time::Duration::ZERO,
            auth_rejection_time_initial: Some(std::time::Duration::ZERO),
            ..Default::default()
        });
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let number = counter.fetch_add(1, Ordering::SeqCst) + 1;
                let conn = Conn {
                    number,
                    end,
                    reject,
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
        (address, count)
    }
}

use std::time::Duration;
use supervise::{supervise, Backoff, Event};

fn test_config(address: String) -> RelayConfig {
    RelayConfig {
        address,
        username: "test".into(),
        url_rule: UrlRule {
            skips: vec!["dash.example.test".into()],
            matches: vec![],
        },
        inactivity: None,
        ..RelayConfig::default()
    }
}

const FAST: Backoff = Backoff {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(40),
    stable: Duration::from_secs(60),
};

async fn next_event(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("an event within 20 s")
        .expect("the supervisor still runs")
}

#[tokio::test]
async fn the_supervisor_reconnects_and_announces_the_new_url() {
    pod_testkit::need_loopback!();
    let (address, count) = fake::start(fake::End::Channel).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(supervise(test_config(address), FAST, tx));

    let Event::Up(first) = next_event(&mut rx).await else {
        panic!("the first event is Up");
    };
    assert_eq!(first.url, "https://tunnel1.example.test");
    assert_eq!(first.remote_port, 40001);
    match next_event(&mut rx).await {
        Event::Down { reason } => assert!(reason.contains("banner channel"), "{reason}"),
        other => panic!("the relay closed the channel: {other:?}"),
    }
    let Event::Up(second) = next_event(&mut rx).await else {
        panic!("the third event is Up");
    };
    assert_eq!(second.url, "https://tunnel2.example.test");
    assert_eq!(second.remote_port, 40002);
    assert_ne!(first.url, second.url);

    // The second session stays up: no event, no third connection.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(rx.try_recv().is_err());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    task.abort();
}

#[tokio::test]
async fn the_supervisor_sees_a_connection_that_the_relay_ends() {
    pod_testkit::need_loopback!();
    let (address, _) = fake::start(fake::End::Connection).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(supervise(test_config(address), FAST, tx));

    assert!(matches!(next_event(&mut rx).await, Event::Up(_)));
    assert!(matches!(next_event(&mut rx).await, Event::Down { .. }));
    let Event::Up(second) = next_event(&mut rx).await else {
        panic!("the third event is Up");
    };
    assert_eq!(second.url, "https://tunnel2.example.test");
    task.abort();
}

#[tokio::test]
async fn the_supervisor_tries_again_when_the_relay_is_not_there() {
    pod_testkit::need_loopback!();
    // A port with no listener: each connect is refused.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = free.local_addr().unwrap().to_string();
    drop(free);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(supervise(test_config(address), FAST, tx));
    for _ in 0..3 {
        match next_event(&mut rx).await {
            Event::Down { reason } => assert!(reason.contains("transport"), "{reason}"),
            other => panic!("no relay, no Up: {other:?}"),
        }
    }
    task.abort();
}

#[tokio::test]
async fn the_supervisor_ends_when_nobody_listens_to_its_events() {
    pod_testkit::need_loopback!();
    let (address, _) = fake::start(fake::End::Channel).await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    drop(rx);
    tokio::time::timeout(
        Duration::from_secs(20),
        supervise(test_config(address), FAST, tx),
    )
    .await
    .expect("the supervisor returns");
}

mod probe_tests {
    use super::*;
    use probe::{probe, Stage};

    const LIMIT: Duration = Duration::from_secs(10);

    fn config(address: String) -> RelayConfig {
        test_config(address)
    }

    /// A listener that runs `serve` for each connection. Gives its address.
    async fn listener<F, Fut>(serve: F) -> String
    where
        F: Fn(tokio::net::TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream));
            }
        });
        address
    }

    #[tokio::test]
    async fn the_probe_passes_each_stage_of_a_good_relay() {
        pod_testkit::need_loopback!();
        let (address, count) = fake::start(fake::End::Channel).await;
        let got = probe(&config(address), LIMIT).await;
        assert_eq!(got.failure, None);
        assert!(got.connect.is_some());
        assert!(got.banner.as_deref().unwrap().starts_with("SSH-2.0-"));
        assert_eq!(got.auth, Some(true));
        assert!(got.total >= got.connect.unwrap());
        // One connection, and no session: the fake would have set a banner
        // for a shell request, and the probe asks for none.
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_probe_names_the_connect_stage_when_nobody_listens() {
        pod_testkit::need_loopback!();
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = free.local_addr().unwrap().to_string();
        drop(free);
        let got = probe(&config(address), LIMIT).await;
        let failure = got.failure.expect("a failure");
        assert_eq!(failure.stage, Stage::Connect);
        assert!(!failure.timed_out, "{failure:?}");
        assert_eq!(got.connect, None);
        assert_eq!(got.banner, None);
    }

    #[tokio::test]
    async fn the_probe_names_the_banner_stage_for_a_server_that_is_not_ssh() {
        pod_testkit::need_loopback!();
        let address = listener(|mut stream| async move {
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        })
        .await;
        let got = probe(&config(address), LIMIT).await;
        let failure = got.failure.expect("a failure");
        assert_eq!(failure.stage, Stage::Banner);
        assert!(!failure.timed_out, "{failure:?}");
        assert!(got.connect.is_some());
        assert_eq!(got.banner, None);
    }

    #[tokio::test]
    async fn the_probe_names_the_auth_stage_when_the_relay_rejects_the_user() {
        pod_testkit::need_loopback!();
        let (address, _) = fake::start_with(fake::End::Channel, true).await;
        let got = probe(&config(address), LIMIT).await;
        let failure = got.failure.expect("a failure");
        assert_eq!(failure.stage, Stage::Auth);
        assert!(!failure.timed_out, "{failure:?}");
        assert!(failure.reason.contains("test"), "{failure:?}");
        assert!(got.banner.is_some());
        assert_eq!(got.auth, Some(false));
    }

    #[tokio::test]
    async fn the_probe_times_out_on_a_relay_that_sends_nothing() {
        pod_testkit::need_loopback!();
        let address = listener(|stream| async move {
            // Hold the connection open and send no byte.
            let _stream = stream;
            tokio::time::sleep(Duration::from_secs(30)).await;
        })
        .await;
        let got = probe(&config(address), Duration::from_millis(300)).await;
        let failure = got.failure.expect("a failure");
        assert_eq!(failure.stage, Stage::Banner);
        assert!(failure.timed_out, "{failure:?}");
        assert!(got.total < Duration::from_secs(5), "{:?}", got.total);
    }
}
