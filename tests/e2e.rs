//! The session end to end at the podnet surface (GitHub #496): the sealed
//! destination, the record keys from the handshake, the KEY-2 header, the
//! suite check, the rekey, the splice, the tokens and the relay's view.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pod_e2e::record::{self, Opened, Suite, HEADER};
use pod_e2e::{initiate, respond, Channel, Destination, EnvelopeError, Pair};
use pod_identity::{Identity, PublicKey};
use tokio::io::{duplex, split, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use podnet::e2e::{self, Config};

fn identity(seed: u8) -> Identity {
    Identity::from_seed(&[seed; 32])
}

fn config(destination: (String, u16), allowed: [PublicKey; 1]) -> Config {
    Config {
        relay: "relay-a".into(),
        pair: "pair-1".into(),
        suites: vec![Suite::ChaCha20Poly1305],
        host: destination.0,
        port: destination.1,
        service: "secret-service".into(),
        allowed: allowed.to_vec(),
        compat_destination: false,
    }
}

/// Copy both directions of one stream while recording every byte.
async fn relay(near: DuplexStream, far: DuplexStream, dump: Arc<Mutex<Vec<u8>>>) {
    let (mut near_read, mut near_write) = split(near);
    let (mut far_read, mut far_write) = split(far);
    let forward = async {
        let mut buf = vec![0u8; 4096];
        loop {
            let n = near_read.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
                let _ = far_write.shutdown().await;
                break;
            }
            dump.lock().await.extend_from_slice(&buf[..n]);
            far_write
                .write_all(&buf[..n])
                .await
                .expect("the relay wrote");
        }
    };
    let back = async {
        let mut buf = vec![0u8; 4096];
        loop {
            let n = far_read.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
                let _ = near_write.shutdown().await;
                break;
            }
            dump.lock().await.extend_from_slice(&buf[..n]);
            near_write
                .write_all(&buf[..n])
                .await
                .expect("the relay wrote");
        }
    };
    tokio::join!(forward, back);
}

/// Copy one direction while recording every byte.
async fn relay_one(near: DuplexStream, far: DuplexStream, dump: Arc<Mutex<Vec<u8>>>) {
    let (mut near_read, _) = split(near);
    let (_, mut far_write) = split(far);
    let mut buf = vec![0u8; 4096];
    loop {
        let n = near_read.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            let _ = far_write.shutdown().await;
            break;
        }
        dump.lock().await.extend_from_slice(&buf[..n]);
        far_write
            .write_all(&buf[..n])
            .await
            .expect("the relay wrote");
    }
}

/// A destination that echoes each byte and ends its write when the peer does.
async fn echo() -> (String, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let addr = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("a destination");
        let (mut read, mut write) = stream.split();
        tokio::io::copy(&mut read, &mut write).await.ok();
        write.shutdown().await.ok();
    });
    (addr.ip().to_string(), addr.port())
}

/// The relay carries every byte, and the dump holds no plaintext, no
/// destination, no service and no key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_relay_dump_holds_no_plaintext_destination_service_or_key() {
    let client_id = identity(11);
    let server_id = identity(12);
    let client_hex = client_id.public().hex();
    let server_hex = server_id.public().hex();
    let destination = echo().await;
    let client_cfg = config(destination.clone(), [server_id.public()]);
    let server_cfg = config(destination.clone(), [client_id.public()]);

    let (client_data, relay_client_data) = duplex(64 * 1024);
    let (relay_server_data, server_data) = duplex(64 * 1024);
    let (client_control, relay_client_control) = duplex(1024);
    let (relay_server_control, server_control) = duplex(1024);
    let dump = Arc::new(Mutex::new(Vec::new()));
    let data_relay = tokio::spawn(relay(relay_client_data, relay_server_data, dump.clone()));
    let control_relay = tokio::spawn(relay_one(
        relay_client_control,
        relay_server_control,
        dump.clone(),
    ));

    let (app_client, mut app) = duplex(64 * 1024);
    let client_task = tokio::spawn(async move {
        let mut control = client_control;
        e2e::client_with(
            client_data,
            &mut control,
            app_client,
            &client_cfg,
            &client_id,
        )
        .await
    });
    let server_task = tokio::spawn(async move {
        let mut control = server_control;
        e2e::server_with(server_data, &mut control, &server_cfg, &server_id).await
    });
    let marker = b"plaintext marker 42";

    // The application marker crosses the relay and comes back from the echo.
    app.write_all(marker).await.expect("the marker");
    app.shutdown().await.expect("the end of the message");
    let mut echo = Vec::new();
    app.read_to_end(&mut echo).await.expect("the echo");

    let client = client_task.await.expect("the client").expect("the client");
    let server = server_task.await.expect("the server").expect("the server");
    assert_eq!(
        client.in_bytes,
        marker.len() as u64,
        "client stats {client:?}"
    );
    assert_eq!(server.out, marker.len() as u64, "server stats {server:?}");
    assert_eq!(echo, marker);
    assert_eq!(client.suite, "chacha20-poly1305");
    assert_eq!(client.epochs.0, 0, "a small transfer rekeyed");

    tokio::time::timeout(Duration::from_secs(5), data_relay)
        .await
        .expect("the data relay ended")
        .expect("the data relay");
    tokio::time::timeout(Duration::from_secs(5), control_relay)
        .await
        .expect("the control relay ended")
        .expect("the control relay");
    let dump = dump.lock().await;
    let needles: [&[u8]; 5] = [
        marker,
        destination.0.as_bytes(),
        b"secret-service",
        client_hex.as_bytes(),
        server_hex.as_bytes(),
    ];
    for needle in needles {
        assert!(
            !dump.windows(needle.len()).any(|window| window == needle),
            "the relay dump holds {:?}",
            String::from_utf8_lossy(needle)
        );
    }
    assert!(dump.len() > marker.len(), "the relay carried no bytes");
    assert!(
        dump.windows(pod_e2e::MAGIC.len())
            .any(|window| window == pod_e2e::MAGIC),
        "the dump carries no channel magic"
    );
}

/// A session is bound to the relay's pair: an envelope of one session does
/// not open in another, so a relay cannot splice two sessions (E2E-2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_splice_between_two_sessions_fails() {
    let pair = Pair::named("relay-a", "pair-1");
    let (first, _) = session(pair).await;
    let (_, second) = session(pair).await;
    let sealed = Destination::seal(&first, "example.com", 443, "ssh").expect("the envelope");
    let error = Destination::open(&second, sealed.bytes()).expect_err("the splice opens");
    assert!(
        matches!(error, EnvelopeError::NotForThisSession),
        "the splice gave {error}"
    );

    // A relay that advertises another pair to one end changes the
    // transcript of that end, so the envelope of the other pair does not
    // open (E2E-2).
    let (first, _) = session(Pair::named("relay-a", "pair-2")).await;
    let sealed = Destination::seal(&first, "example.com", 443, "ssh").expect("the envelope");
    let error = Destination::open(&second, sealed.bytes()).expect_err("the other pair opens");
    assert!(
        matches!(error, EnvelopeError::NotForThisSession),
        "the other pair gave {error}"
    );

    // The records of one session do not open in the other.
    let suite = Suite::ChaCha20Poly1305;
    let mut records = first
        .exporter()
        .expect("the exporter")
        .records(suite, record::Settings::DEFAULT, Instant::now())
        .expect("the records");
    let wire = records
        .seal(Instant::now(), e2e::DATA, 0, 0, b"secret")
        .expect("the record");
    let mut other = second
        .exporter()
        .expect("the exporter")
        .records(suite, record::Settings::DEFAULT, Instant::now())
        .expect("the records");
    assert!(other.open(Instant::now(), &wire).is_err());
}

/// One near end and one far end of a handshake, with the same pair.
async fn session(pair: Pair) -> (Channel<DuplexStream>, Channel<DuplexStream>) {
    let (a, b) = duplex(64 * 1024);
    let near = async {
        let offered = initiate(a, &identity(21)).await.expect("the offer");
        let mut channel = offered.accept(None).await.expect("the channel");
        channel.bind(pair).expect("the pair");
        channel
    };
    let far = async {
        let asked = respond(b, &identity(22)).await.expect("the ask");
        let mut channel = asked.admit().await.expect("the channel");
        channel.bind(pair).expect("the pair");
        channel
    };
    tokio::join!(near, far)
}

/// A suite mismatch is exit 78, before any record is sealed.
#[test]
fn a_suite_mismatch_exits_78_before_a_record() {
    let error = e2e::agree_suite(&[Suite::ChaCha20Poly1305], &[Suite::Aes256Gcm])
        .expect_err("the mismatch");
    assert_eq!(podnet::exit::code_of(&error), 78, "{error:#}");
    assert!(format!("{error:#}").contains("suite"));
    let error = e2e::suite_of("no-such-suite").expect_err("the name");
    assert_eq!(podnet::exit::code_of(&error), 78, "{error:#}");
}

/// A transfer that reaches 1 GiB over the loopback rekeys and completes.
/// The test prints the duration that it measured, and it does not lower the
/// byte count. One record past the ceiling makes the rekey visible: the
/// layer takes the new key before it seals the record after the ceiling.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_gib_loopback_transfer_rekeys_and_completes() {
    let pair = Pair::named("relay-a", "pair-1");
    // Either suite rekeys on the same byte count. The hardware-accelerated
    // suite keeps this 1 GiB run practical in a debug build.
    let suite = Suite::Aes256Gcm;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let address = listener.local_addr().expect("an address");
    let near_id = identity(31);
    let far_id = identity(32);
    let near = tokio::spawn(async move {
        let stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("the loopback");
        let offered = initiate(stream, &near_id).await.expect("the offer");
        let mut channel = offered.accept(None).await.expect("the channel");
        channel.bind(pair).expect("the pair");
        let suite = channel.agree(&[suite]).await.expect("the suite");
        let records = channel
            .exporter()
            .expect("the exporter")
            .records(suite, record::Settings::DEFAULT, Instant::now())
            .expect("the records");
        (records, channel.into_transport())
    });
    let far = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("the loopback");
        let asked = respond(stream, &far_id).await.expect("the ask");
        let mut channel = asked.admit().await.expect("the channel");
        channel.bind(pair).expect("the pair");
        let suite = channel.agree(&[suite]).await.expect("the suite");
        let records = channel
            .exporter()
            .expect("the exporter")
            .records(suite, record::Settings::DEFAULT, Instant::now())
            .expect("the records");
        (records, channel.into_transport())
    });
    let (near, far) = tokio::join!(near, far);
    let (mut near_records, mut near_stream) = near.expect("the near task");
    let (mut far_records, mut far_stream) = far.expect("the far task");
    let total: u64 = 1 << 30;
    let limit = total + e2e::CHUNK as u64;
    let chunk = vec![0x5au8; e2e::CHUNK];
    let mut sent = 0u64;
    let mut opened = 0u64;
    let started = Instant::now();
    while sent < limit {
        let n = (limit - sent).min(chunk.len() as u64) as usize;
        let wire = near_records
            .seal(Instant::now(), e2e::DATA, 0, 0, &chunk[..n])
            .expect("the record");
        near_stream
            .write_all(&wire)
            .await
            .expect("the loopback write");
        let mut header = [0u8; HEADER];
        far_stream
            .read_exact(&mut header)
            .await
            .expect("the header of the record");
        let len = record::record_len(&header).expect("the record length");
        let mut rest = vec![0u8; len - HEADER];
        far_stream
            .read_exact(&mut rest)
            .await
            .expect("the body of the record");
        let mut whole = header.to_vec();
        whole.extend_from_slice(&rest);
        match far_records
            .open(Instant::now(), &whole)
            .expect("the record opens")
        {
            Opened::Record(record) => {
                opened += record.payload.len() as u64;
                sent += n as u64;
            }
            Opened::Resume { next } => panic!("a resume at {next} with no lost carrier"),
        }
    }
    let elapsed = started.elapsed();
    eprintln!(
        "loopback: {} bytes sent, {} opened, far epoch {}, in {:?}",
        sent,
        opened,
        far_records.epochs().1,
        elapsed
    );
    assert_eq!(sent, limit);
    assert_eq!(opened, sent);
    // The key schedule of `record::Settings::DEFAULT`: a new key before the
    // record at 1 GiB of bytes or 2^31 records, or at 3600 s of age. The
    // transfer is `1 GiB + CHUNK` bytes of 16 KiB records, so exactly one
    // byte ceiling is crossed and the epoch is 1, not "more than 0".
    let rekey_bytes = record::Settings::DEFAULT.rekey_bytes;
    assert_eq!(rekey_bytes, 1 << 30, "the byte ceiling is 1 GiB");
    assert_eq!(record::Settings::DEFAULT.rekey_records, 1 << 31);
    let expected_epoch = (limit - 1) / rekey_bytes;
    assert_eq!(expected_epoch, 1, "one byte ceiling in {limit} bytes");
    assert_eq!(
        far_records.epochs().1,
        expected_epoch,
        "the far end rekeys once at the byte ceiling: {:?}",
        far_records.epochs()
    );
    assert_eq!(
        near_records.epochs().0,
        expected_epoch,
        "the near end rekeys once at the byte ceiling: {:?}",
        near_records.epochs()
    );
    assert!(
        elapsed < Duration::from_secs(1800),
        "the transfer took {elapsed:?}"
    );
}

/// The KEY-2 header is exactly 24 bytes with the specified fields, and the
/// tag is 16 bytes.
#[tokio::test]
async fn the_record_header_is_key2() {
    assert_eq!(HEADER, 24);
    assert_eq!(pod_e2e::TAG, 16);
    let (mut near, mut far) = e2e::records_with(
        &identity(41),
        &identity(42),
        Pair::named("relay-a", "pair-1"),
        Suite::ChaCha20Poly1305,
    )
    .await
    .expect("the records");
    let wire = near
        .seal(Instant::now(), e2e::DATA, 0, 7, b"hello")
        .expect("the record");
    assert_eq!(wire.len(), HEADER + 5 + pod_e2e::TAG);
    assert_eq!(wire[0], record::VERSION);
    assert_eq!(wire[1], e2e::DATA);
    assert_eq!(u16::from_be_bytes([wire[2], wire[3]]), 0);
    assert_eq!(u32::from_be_bytes([wire[4], wire[5], wire[6], wire[7]]), 7);
    assert_eq!(
        u64::from_be_bytes(wire[8..16].try_into().expect("a counter")),
        0
    );
    assert_eq!(
        u32::from_be_bytes(wire[16..20].try_into().expect("a length")),
        5
    );
    assert_eq!(u16::from_be_bytes([wire[20], wire[21]]), 0);
    assert_eq!(u16::from_be_bytes([wire[22], wire[23]]), 0);
    match far.open(Instant::now(), &wire).expect("the record") {
        Opened::Record(record) => {
            assert_eq!(record.kind, e2e::DATA);
            assert_eq!(record.stream, 7);
            assert_eq!(record.payload, b"hello");
        }
        Opened::Resume { next } => panic!("a resume at {next}"),
    }
}

/// The 2^31-record ceiling takes a new key at the count, not only at 1 GiB.
/// A reduced `rekey_records` drives the count-only path without a large
/// payload; the boundary is the fifth record for a ceiling of four, and the
/// record at the boundary carries the layer's PHASE bit of the new key.
#[tokio::test]
async fn the_record_count_ceiling_takes_a_new_key_at_the_count() {
    assert_eq!(record::Settings::DEFAULT.rekey_records, 1 << 31);
    assert_eq!(record::Settings::DEFAULT.rekey_bytes, 1 << 30);
    let settings = record::Settings {
        rekey_records: 4,
        rekey_bytes: 1 << 40,
        ..record::Settings::DEFAULT
    };
    let (mut near, mut far) = e2e::records_with_settings(
        &identity(61),
        &identity(62),
        Pair::named("relay-a", "pair-1"),
        Suite::Aes256Gcm,
        settings,
    )
    .await
    .expect("the records");
    let mut flags = Vec::new();
    for at in 0..4u8 {
        let wire = near
            .seal(Instant::now(), e2e::DATA, 0, 0, &[at; 32])
            .expect("the record");
        flags.push(u16::from_be_bytes([wire[2], wire[3]]));
        match far.open(Instant::now(), &wire).expect("the record opens") {
            Opened::Record(record) => assert_eq!(record.payload, vec![at; 32]),
            Opened::Resume { next } => panic!("a resume at {next}"),
        }
    }
    assert_eq!(flags, [0, 0, 0, 0], "no phase bit before the count");
    assert_eq!(near.epochs().0, 0, "four records are one key");
    assert_eq!(far.epochs().1, 0, "the far end opened one key");
    let wire = near
        .seal(Instant::now(), e2e::DATA, 0, 0, &[4u8; 32])
        .expect("the record at the count");
    let phase = u16::from_be_bytes([wire[2], wire[3]]);
    assert_eq!(
        phase & record::PHASE,
        record::PHASE,
        "the record at the count opens the next key: flags {phase:#06x}"
    );
    match far.open(Instant::now(), &wire).expect("the record opens") {
        Opened::Record(record) => assert_eq!(record.payload, vec![4u8; 32]),
        Opened::Resume { next } => panic!("a resume at {next}"),
    }
    assert_eq!(near.epochs().0, 1, "the near end took a new key");
    assert_eq!(far.epochs().1, 1, "the far end moved to the new key");
}

/// A token for one relay and one service does not open another (LEARN-2).
#[test]
fn a_token_does_not_cross_a_relay_or_a_service() {
    let secret = [7u8; 32];
    let token = e2e::mint_token(&secret, "relay-a", "ssh");
    assert!(e2e::verify_token(&secret, "relay-a", "ssh", &token));
    assert!(!e2e::verify_token(&secret, "relay-b", "ssh", &token));
    assert!(!e2e::verify_token(&secret, "relay-a", "http", &token));
    assert!(!e2e::verify_token(&secret, "relay-a", "ssh", "00"));
}

/// The compatibility path prints the warning that names the leak (OPAQUE-2).
#[test]
fn the_clear_destination_prints_a_leak_warning() {
    let text = e2e::compat_warning("secret.example", 5432);
    assert!(text.contains("warning"), "{text}");
    assert!(text.contains("/connect/secret.example/5432"), "{text}");
    assert!(text.contains("in clear"), "{text}");
    assert!(text.contains("every hop learns the destination"), "{text}");
}

/// The compatibility path is refused unless the far end takes it, and the
/// sealed path is the default.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_clear_destination_is_refused_without_the_flag() {
    let client_id = identity(51);
    let server_id = identity(52);
    let mut client_cfg = config(("127.0.0.1".into(), 9), [server_id.public()]);
    client_cfg.compat_destination = true;
    let server_cfg = config(("127.0.0.1".into(), 9), [client_id.public()]);
    let (client_data, server_data) = duplex(64 * 1024);
    let (mut client_control, mut server_control) = duplex(1024);
    let (app, mut peer) = duplex(1024);
    let client = tokio::spawn(async move {
        e2e::client_with(
            client_data,
            &mut client_control,
            app,
            &client_cfg,
            &client_id,
        )
        .await
    });
    let server = tokio::spawn(async move {
        e2e::server_with(server_data, &mut server_control, &server_cfg, &server_id).await
    });
    let error = server
        .await
        .expect("the server task")
        .expect_err("the far end took a clear destination");
    assert_eq!(podnet::exit::code_of(&error), 78, "{error:#}");
    // End the application, so the near end can finish and the test cannot
    // wait on a stream that no end closes.
    peer.shutdown().await.expect("the application end");
    let _ = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("the near end ended");
}

/// `inspect` prints the hop table and the relay's view, and the view holds
/// no destination.
#[tokio::test]
async fn inspect_prints_the_hop_table_and_the_relay_view() {
    let cfg = config(("secret.example".into(), 443), [identity(61).public()]);
    let text = e2e::inspect(&cfg, &identity(62).public())
        .await
        .expect("the view");
    assert!(text.contains("first hop"), "{text}");
    assert!(text.contains("| relay |"), "{text}");
    assert!(text.contains("far hop"), "{text}");
    let view = text
        .split("tunnel --inspect, relay:")
        .nth(1)
        .expect("the relay view");
    assert!(!view.contains("secret.example"), "{view}");
    assert!(!view.contains("secret-service"), "{view}");
    assert!(
        text.contains("secret.example:443"),
        "the table lacks the destination: {text}"
    );
}
