//! Integration tests for TLS Encrypted ClientHello (ECH, GitHub #151 / T-004).

use std::net::SocketAddr;
use std::pin::Pin;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use base64::Engine as _;
use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use pod_tls::ech::{self, Cache, Mode, Outcome, Published, Records, Route};
use pod_tls::{Context as TlsContext, Settings, Trust, Words};
use podnet::ech::{
    check_name_length, grease_client_config, grease_config, parse_ech_config_list,
    sealed_client_config, validate_ech_config_list, EchHops, EchSetting,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

const PUBLIC_NAME: &str = "public.ech.test";
const INNER_NAME: &str = "inner.ech.test";
const LIMIT: Duration = Duration::from_secs(10);

fn build_ech_config_list(public_name: &str, max_name_length: u8) -> Vec<u8> {
    let suite = pod_tls::crypto::hpke::SUITES[0];
    let (pk, _) = suite.generate_key_pair().expect("hpke key pair");
    let mut body = vec![0x0b, 0x00, 0x20];
    body.extend_from_slice(&(pk.0.len() as u16).to_be_bytes());
    body.extend_from_slice(&pk.0);
    body.extend_from_slice(&[0x00, 0x04, 0x00, 0x01, 0x00, 0x01]);
    body.push(max_name_length);
    body.push(public_name.len() as u8);
    body.extend_from_slice(public_name.as_bytes());
    body.extend_from_slice(&[0x00, 0x00]);

    let mut cfg = vec![0xfe, 0x0d];
    cfg.extend_from_slice(&(body.len() as u16).to_be_bytes());
    cfg.extend_from_slice(&body);

    let mut list = (cfg.len() as u16).to_be_bytes().to_vec();
    list.extend_from_slice(&cfg);
    list
}

struct LoopbackDial;

impl Dial for LoopbackDial {
    fn label(&self) -> String {
        "loopback".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let Target::Addr(addr) = target else {
                return Err(Failure::new(
                    Stage::Connect,
                    Class::Usage,
                    "loopback dialer requires a socket address",
                ));
            };
            let tcp = tokio::time::timeout_at(deadline, TcpStream::connect(addr))
                .await
                .map_err(|_| Failure::new(Stage::Connect, Class::Timeout, "connect timeout"))?
                .map_err(|e| Failure::io(Stage::Connect, "connect", e))?;
            let (peer, local) = (tcp.peer_addr().ok(), tcp.local_addr().ok());
            Ok(Connected {
                io: Box::new(tcp),
                peer,
                local,
            })
        })
    }
}

struct EmptyRecords;

impl Records for EmptyRecords {
    fn ech_list<'a>(
        &'a self,
        _name: &'a str,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Option<Published>, String>> {
        Box::pin(async { Ok(None) })
    }
}

struct Tap {
    inner: TcpStream,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl AsyncRead for Tap {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        let before = buf.filled().len();
        let polled = Pin::new(&mut me.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &polled {
            me.seen
                .lock()
                .unwrap()
                .extend_from_slice(&buf.filled()[before..]);
        }
        polled
    }
}

impl AsyncWrite for Tap {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

type SeenHellos = Arc<Mutex<Vec<(Option<String>, Vec<u8>)>>>;

async fn spawn_tls_server(names: &[&str]) -> (SocketAddr, SeenHellos, tempfile::TempDir, Trust) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "podnet ech test CA");
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);

    let dir = tempfile::tempdir().unwrap();
    let ca_path = dir.path().join("ca.pem");
    std::fs::write(&ca_path, ca_cert.pem()).unwrap();

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut configs: Vec<(String, Arc<rustls::ServerConfig>)> = Vec::new();
    for name in names {
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf_params = rcgen::CertificateParams::new(vec![(*name).to_string()]).unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
        let chain = vec![CertificateDer::from(leaf.der().to_vec())];
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let cfg = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        configs.push(((*name).to_string(), Arc::new(cfg)));
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hellos: SeenHellos = Arc::default();
    let log = hellos.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let tap = Tap {
                inner: tcp,
                seen: seen.clone(),
            };
            let (configs, log) = (configs.clone(), log.clone());
            tokio::spawn(async move {
                let acceptor =
                    tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(), tap);
                let Ok(start) = acceptor.await else { return };
                let sni = start.client_hello().server_name().map(str::to_string);
                log.lock()
                    .unwrap()
                    .push((sni.clone(), seen.lock().unwrap().clone()));
                let found = configs
                    .iter()
                    .find(|(n, _)| Some(n.as_str()) == sni.as_deref());
                let Some((_, cfg)) = found else { return };
                if let Ok(mut tls) = start.into_stream(cfg.clone()).await {
                    let mut buf = Vec::new();
                    let _ = tokio::time::timeout(LIMIT, tls.read_to_end(&mut buf)).await;
                }
            });
        }
    });
    (addr, hellos, dir, Trust::File(ca_path))
}

fn test_tls_context() -> TlsContext {
    let mut words = Words::new("podnet");
    words.ech_strict = "--ech strict or PODNET_ECH=strict".into();
    TlsContext::new(
        Arc::new(rustls::crypto::ring::default_provider()),
        Settings::new(words),
    )
}

#[test]
fn parses_ech_settings_name_length_and_builds_tls13_configs() {
    podnet::init_crypto();
    assert_eq!(EchSetting::parse("off").unwrap(), EchSetting::Off);
    assert_eq!(EchSetting::parse("auto").unwrap(), EchSetting::On);
    assert_eq!(EchSetting::parse("on").unwrap(), EchSetting::On);
    assert_eq!(EchSetting::parse("strict").unwrap(), EchSetting::Strict);
    assert!(EchSetting::parse("not-valid-ech").is_err());

    let raw = build_ech_config_list("cloudflare-ech.com", 18);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    assert_eq!(
        EchSetting::parse(&b64).unwrap(),
        EchSetting::ConfigList(raw.clone())
    );

    let infos = parse_ech_config_list(&raw).unwrap();
    assert_eq!(infos[0].public_name, "cloudflare-ech.com");
    assert_eq!(infos[0].max_name_length, 18);

    // consumer-masque.cloudflareclient.com (len 36) > 18 names both 36 and 18.
    let err = check_name_length("consumer-masque.cloudflareclient.com", &infos[0]).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("36") && msg.contains("18"), "{msg}");
    assert!(check_name_length("api.example.com", &infos[0]).is_ok());

    assert!(validate_ech_config_list(&raw).is_ok());
    assert!(grease_config().is_ok());
    assert!(grease_client_config(&[], &[b"http/1.1"]).is_ok());
    assert!(sealed_client_config(&raw, &[], &[b"http/1.1"]).is_ok());
    assert_eq!(EchHops::parse("api,edge,masque").unwrap(), EchHops::all());
}

#[tokio::test]
async fn loopback_ech_off_on_strict_and_sealed_outer_sni() {
    pod_testkit::need_loopback!();
    podnet::init_crypto();

    let (addr, hellos, _dir, trust) = spawn_tls_server(&[PUBLIC_NAME, INNER_NAME]).await;
    let ctx = test_tls_context();
    let target = Target::Addr(addr);

    // (a) ECH off sends a plain ClientHello naming INNER_NAME.
    let cache = Cache::new();
    let route = Route {
        target: &target,
        server_name: INNER_NAME,
        trust: &trust,
        dial: &LoopbackDial,
        records: &EmptyRecords,
        limit: LIMIT,
        alpn: &[],
    };
    let (stream, _) = ech::open_with(&ctx, &cache, Mode::Off, &route)
        .await
        .expect("ECH off connects with plain ClientHello");
    drop(stream);
    {
        let seen = hellos.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.as_deref(), Some(INNER_NAME));
    }

    // (b) Mode::Strict with no published config refuses without sending a plain ClientHello.
    let count_before = hellos.lock().unwrap().len();
    let err = ech::open_with(&ctx, &cache, Mode::Strict, &route)
        .await
        .err()
        .expect("strict mode must fail without a published ECH config");
    assert!(
        err.to_string()
            .contains("--ech strict or PODNET_ECH=strict"),
        "{err}"
    );
    assert_eq!(
        hellos.lock().unwrap().len(),
        count_before,
        "strict must not send a plain ClientHello when no ECH config is published"
    );

    // (c) Mode::On with no published config falls back to plain and records Outcome::NotPublished.
    let (stream, _) = ech::open_with(&ctx, &cache, Mode::On, &route)
        .await
        .expect("Mode::On falls back to plain when not published");
    drop(stream);
    assert_eq!(ctx.ech().outcome(INNER_NAME), Some(Outcome::NotPublished));
    assert_eq!(hellos.lock().unwrap().len(), count_before + 1);

    // (d) Mode::Strict with a cached ECHConfigList seals the ClientHello with PUBLIC_NAME
    // (never leaking INNER_NAME in the outer ClientHello) and does not fall back to plain.
    let sealed_cache = Cache::new();
    let cfg_list = build_ech_config_list(PUBLIC_NAME, 64);
    sealed_cache.remember(INNER_NAME, cfg_list, Duration::from_secs(300));
    let before_sealed = hellos.lock().unwrap().len();
    let err = ech::open_with(&ctx, &sealed_cache, Mode::Strict, &route)
        .await
        .err()
        .expect("strict mode must fail when server does not accept ECH");
    assert!(
        err.to_string()
            .contains("--ech strict or PODNET_ECH=strict"),
        "{err}"
    );
    let seen = hellos.lock().unwrap();
    assert_eq!(seen.len(), before_sealed + 1, "only the sealed hello went");
    let (ref outer_sni, ref raw_hello) = seen[before_sealed];
    assert_eq!(outer_sni.as_deref(), Some(PUBLIC_NAME));
    assert!(
        !raw_hello
            .windows(INNER_NAME.len())
            .any(|w| w == INNER_NAME.as_bytes()),
        "sealed ClientHello must not expose inner server name in the clear"
    );
}

#[test]
fn cli_accepts_ech_flags_and_rejects_bad_value() {
    let bin = env!("CARGO_BIN_EXE_podnet");
    for args in [
        &["--ech", "off", "status"][..],
        &["--ech", "auto", "status"][..],
        &["--ech", "strict", "status"][..],
        &["--ech-grease", "status"][..],
    ] {
        let out = Command::new(bin).args(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let bad = Command::new(bin)
        .args(["--ech", "bogus-mode", "status"])
        .output()
        .unwrap();
    assert_eq!(
        bad.status.code(),
        Some(podnet::exit::USAGE),
        "bad --ech exits 64: {}",
        String::from_utf8_lossy(&bad.stderr)
    );
}
