//! The shared helpers of the tests of the record layer: a stream over a
//! unix link, a chain A - B - C with a relay that reads whole records at B,
//! and the sealed echo.
#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use pod_e2e::record::{Header, HEADER};
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings as LinkSettings, Stream};
use podnet::link_record::{self as record, Options, Sealed};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

pub fn allow(keys: &[&Identity]) -> AllowList {
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    AllowList::parse(&text.join("\n")).unwrap()
}

/// A stream that the opener made and the acceptor got, over a unix link.
pub async fn streams() -> (Stream, Stream, (Link, Link)) {
    let (a, b) = link::unix_pair(LinkSettings::default()).unwrap();
    let opened = a.open().unwrap();
    let mut got = b.accept().await.unwrap();
    // The opener's first bytes open the stream at the acceptor.
    let mut opened = opened;
    opened.write_all(b"x").await.unwrap();
    let mut one = [0u8; 1];
    got.read_exact(&mut one).await.unwrap();
    (opened, got, (a, b))
}

pub async fn pair(
    dialer: Options,
    listener: Options,
) -> (Result<Sealed, anyhow::Error>, Result<Sealed, anyhow::Error>) {
    let dir = tempfile::tempdir().unwrap();
    let (a, c) = (install(dir.path(), "a"), install(dir.path(), "c"));
    let (s, t, _links) = streams().await;
    let (cp, list) = (c.public(), allow(&[&a]));
    let (d, l) = tokio::join!(
        record::dial(s, &a, Some(&cp), dialer),
        record::accept(t, &c, &list, listener)
    );
    std::mem::forget(_links);
    (d, l)
}

pub async fn echo(mut s: Sealed) {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = s.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        s.write_all(&buf[..n]).await.unwrap();
    }
    let _ = s.shutdown().await;
}

pub async fn round(mut s: Sealed, data: Vec<u8>) -> (Vec<u8>, Sealed) {
    let (mut r, mut w) = tokio::io::split(&mut s);
    let send = async {
        w.write_all(&data).await.unwrap();
        w.shutdown().await.unwrap();
    };
    let mut back = Vec::new();
    let (_, got) = tokio::join!(send, r.read_to_end(&mut back));
    got.unwrap();
    (back, s)
}

pub fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

// ---- the relay of a chain: it reads whole records and may change them ----

pub type Edit = Box<dyn FnMut(usize, Vec<u8>) -> Vec<Vec<u8>> + Send>;

#[derive(Default, Clone)]
pub struct Seen {
    /// The records A sent, as the middle hop saw them.
    pub to_c: Arc<Mutex<Vec<Vec<u8>>>>,
    pub to_a: Arc<Mutex<Vec<Vec<u8>>>>,
}

pub async fn skip_handshake<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(r: &mut R, w: &mut W) {
    let mut magic = [0u8; 13];
    r.read_exact(&mut magic).await.unwrap();
    w.write_all(&magic).await.unwrap();
    // The Noise message, the proof or the verdict, and the suites.
    for _ in 0..3 {
        let mut len = [0u8; 2];
        r.read_exact(&mut len).await.unwrap();
        let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
        r.read_exact(&mut body).await.unwrap();
        w.write_all(&len).await.unwrap();
        w.write_all(&body).await.unwrap();
    }
}

pub async fn relay_one<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut r: R,
    mut w: W,
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
    mut edit: Edit,
) {
    skip_handshake(&mut r, &mut w).await;
    let mut index = 0;
    loop {
        let mut rec = vec![0u8; HEADER];
        if r.read_exact(&mut rec).await.is_err() {
            break;
        }
        rec.resize(pod_e2e::record::record_len(&rec).unwrap(), 0);
        if r.read_exact(&mut rec[HEADER..]).await.is_err() {
            break;
        }
        seen.lock().unwrap().push(rec.clone());
        for out in edit(index, rec) {
            if w.write_all(&out).await.is_err() {
                return;
            }
        }
        index += 1;
    }
    let _ = w.shutdown().await;
}

/// A chain A - B - C of two unix links. B copies the records of one stream
/// and applies `to_c` to those of A and `to_a` to those of C.
pub async fn chain(
    opts_a: Options,
    opts_c: Options,
    to_c: Edit,
    to_a: Edit,
) -> (Sealed, Sealed, Seen) {
    let dir = tempfile::tempdir().unwrap();
    let (a, c) = (install(dir.path(), "a"), install(dir.path(), "c"));
    let (ab_a, ab_b) = link::unix_pair(LinkSettings::default()).unwrap();
    let (bc_b, bc_c) = link::unix_pair(LinkSettings::default()).unwrap();
    let mut sa = ab_a.open().unwrap();
    sa.write_all(b"x").await.unwrap();
    let mut sb_in = ab_b.accept().await.unwrap();
    let mut one = [0u8; 1];
    sb_in.read_exact(&mut one).await.unwrap();
    let mut sb_out = bc_b.open().unwrap();
    sb_out.write_all(b"x").await.unwrap();
    let mut sc = bc_c.accept().await.unwrap();
    sc.read_exact(&mut one).await.unwrap();
    let seen = Seen::default();
    let (in_r, in_w) = tokio::io::split(sb_in);
    let (out_r, out_w) = tokio::io::split(sb_out);
    tokio::spawn(relay_one(in_r, out_w, seen.to_c.clone(), to_c));
    tokio::spawn(relay_one(out_r, in_w, seen.to_a.clone(), to_a));
    let (cp, list) = (c.public(), allow(&[&a]));
    let (d, l) = tokio::join!(
        record::dial(sa, &a, Some(&cp), opts_a),
        record::accept(sc, &c, &list, opts_c)
    );
    std::mem::forget((ab_a, ab_b, bc_b, bc_c));
    (d.unwrap(), l.unwrap(), seen)
}

pub fn pass() -> Edit {
    Box::new(|_, r| vec![r])
}

pub fn header(record: &[u8]) -> Header {
    Header::parse(record).unwrap()
}
