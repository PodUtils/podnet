//! The transit of magic-wormhole (magic-wormhole/magic-wormhole-protocols:
//! transit.md, as transit.py of the Python client speaks it): the keys of
//! the transit key, the handshakes and the records.
//!
//! * Each key is HKDF-SHA256 of the transit key with no salt, its context a
//!   word: `transit_sender`, `transit_receiver`, `transit_relay_token`,
//!   `transit_record_sender_key`, `transit_record_receiver_key`.
//! * The relay handshake: `please relay TOKEN for side SIDE\n`, then `ok\n`.
//! * The handshake of the ends: `transit sender HEX ready\n\n` and
//!   `transit receiver HEX ready\n\n`; the sender then writes `go\n` on the
//!   first connection that passes, and `nevermind\n` on the others.
//! * A record: 4 bytes of length (big-endian), then a nonce of 24 bytes and
//!   the SecretBox of the data. The nonce of each direction counts from 0,
//!   big-endian; a record with another nonce ends the connection.

use std::io;

use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Key, Nonce, XSalsa20Poly1305};
use hkdf::Hkdf;
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The bytes of a nonce.
pub const NONCE: usize = 24;
/// The bytes that a record adds to its data: the length, the nonce, the tag.
pub const OVERHEAD: usize = 4 + NONCE + 16;
/// The largest record that this end reads: the default bound of transit.md.
pub const RECORD_MOST: usize = 64 << 20;
/// The data of each record that this end sends.
pub const CHUNK: usize = 256 << 10;

/// The two roles of a transit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Sender,
    Receiver,
}

/// The key of `context`, of the transit key.
pub fn derive(key: &[u8; 32], context: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    // 32 bytes is far below the bound of HKDF-SHA256.
    let _ = Hkdf::<Sha256>::new(None, key).expand(context, &mut out);
    out
}

/// The handshake that `role` writes.
pub fn handshake(key: &[u8; 32], role: Role) -> Vec<u8> {
    let (word, context): (&str, &[u8]) = match role {
        Role::Sender => ("sender", b"transit_sender"),
        Role::Receiver => ("receiver", b"transit_receiver"),
    };
    format!(
        "transit {word} {} ready\n\n",
        pod_wormhole::hex(&derive(key, context))
    )
    .into_bytes()
}

/// The handshake that `role` writes to a relay, as `side` (16 hex digits).
pub fn relay_handshake(key: &[u8; 32], side: &str) -> Vec<u8> {
    let token = pod_wormhole::hex(&derive(key, b"transit_relay_token"));
    format!("please relay {token} for side {side}\n").into_bytes()
}

/// Read exactly `expected` from `io`; false at the first byte that differs
/// or at the end of the stream.
pub async fn expect<S: AsyncRead + Unpin>(io: &mut S, expected: &[u8]) -> io::Result<bool> {
    let mut got = vec![0u8; expected.len()];
    let mut have = 0;
    while have < got.len() {
        let n = io.read(&mut got[have..]).await?;
        if n == 0 || got[have..have + n] != expected[have..have + n] {
            return Ok(false);
        }
        have += n;
    }
    Ok(true)
}

/// The records of one transit connection, after its handshake.
pub struct Records<S> {
    io: S,
    seal: XSalsa20Poly1305,
    open: XSalsa20Poly1305,
    sent: u64,
    read: u64,
    most: usize,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Records<S> {
    /// The records of `role` over `io`, under the transit key `key`.
    pub fn new(io: S, key: &[u8; 32], role: Role) -> Records<S> {
        let sender = derive(key, b"transit_record_sender_key");
        let receiver = derive(key, b"transit_record_receiver_key");
        let (mine, theirs) = match role {
            Role::Sender => (sender, receiver),
            Role::Receiver => (receiver, sender),
        };
        Records {
            io,
            seal: XSalsa20Poly1305::new(Key::from_slice(&mine)),
            open: XSalsa20Poly1305::new(Key::from_slice(&theirs)),
            sent: 0,
            read: 0,
            most: RECORD_MOST,
        }
    }

    /// Send `data` as one record.
    pub async fn send(&mut self, data: &[u8]) -> io::Result<()> {
        let nonce = nonce(self.sent);
        self.sent += 1;
        let sealed = self
            .seal
            .encrypt(Nonce::from_slice(&nonce), data)
            .map_err(|_| broken("a record could not be sealed"))?;
        let length =
            u32::try_from(NONCE + sealed.len()).map_err(|_| broken("a record over 4 GiB"))?;
        let mut out = Vec::with_capacity(4 + NONCE + sealed.len());
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        self.io.write_all(&out).await?;
        self.io.flush().await
    }

    /// The next record. A record that does not open, a nonce out of order
    /// or a record over the bound ends the transit with an error.
    pub async fn receive(&mut self) -> io::Result<Vec<u8>> {
        let mut length = [0u8; 4];
        self.io.read_exact(&mut length).await?;
        let length = u32::from_be_bytes(length) as usize;
        if length > self.most || length < NONCE + 16 {
            return Err(broken(&format!(
                "a record of {length} bytes: the bound is {} bytes",
                self.most
            )));
        }
        // The buffer grows with the bytes that come: a length of 64 MiB
        // with no bytes after it does not take 64 MiB.
        let mut body = Vec::with_capacity(length.min(CHUNK + OVERHEAD));
        (&mut self.io)
            .take(length as u64)
            .read_to_end(&mut body)
            .await?;
        if body.len() != length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the transit ended in a record",
            ));
        }
        let (got, sealed) = body.split_at(NONCE);
        if got != nonce(self.read) {
            return Err(broken("a record came out of order: the transit changed it"));
        }
        self.read += 1;
        self.open
            .decrypt(Nonce::from_slice(got), sealed)
            .map_err(|_| broken("a record did not open: the transit changed it"))
    }

    /// The stream under the records.
    pub fn into_inner(self) -> S {
        self.io
    }

    /// Close the stream of this end.
    pub async fn close(&mut self) {
        let _ = self.io.shutdown().await;
    }

    /// Read and drop the bytes of the other end until its end, for at most
    /// `limit`.
    pub async fn drain(&mut self, limit: std::time::Duration) {
        let mut buf = [0u8; 512];
        let _ = tokio::time::timeout(limit, async {
            while let Ok(n) = self.io.read(&mut buf).await {
                if n == 0 {
                    break;
                }
            }
        })
        .await;
    }
}

/// The nonce of record `n`: big-endian in 24 bytes, as transit.py writes it.
fn nonce(n: u64) -> [u8; NONCE] {
    let mut out = [0u8; NONCE];
    out[NONCE - 8..].copy_from_slice(&n.to_be_bytes());
    out
}

fn broken(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0x11; 32];

    // Computed with the hmac and hashlib of Python (HKDF of RFC 5869 written
    // out, with no salt), software that podnet did not write.
    #[test]
    fn wormhole_transfer_the_keys_are_hkdf_of_the_transit_key() {
        let hex = |context: &[u8]| pod_wormhole::hex(&derive(&KEY, context));
        assert_eq!(
            hex(b"transit_sender"),
            "327d27094efbf70fd91dbea98ba76becbc78b588140ab48918c3bd0dc572e282"
        );
        assert_eq!(
            hex(b"transit_receiver"),
            "67a8212561515ca6fdaaf20e16ea06d6a873c51d2cf8b3fb3f226d68f4ab63d8"
        );
        assert_eq!(
            hex(b"transit_relay_token"),
            "e6fcc6138a8d9f9dffbcf20818cbda08ba950c7d671bef08c2f4bb63c020ed56"
        );
        assert_eq!(
            hex(b"transit_record_sender_key"),
            "2a2a392bb3ece1ca987f42640ba9923ed69df831a7e9cb78ab29e40f880cbbdb"
        );
    }

    #[test]
    fn wormhole_transfer_the_handshakes_have_the_words_of_transit_py() {
        let sender = String::from_utf8(handshake(&KEY, Role::Sender)).unwrap();
        let receiver = String::from_utf8(handshake(&KEY, Role::Receiver)).unwrap();
        assert!(
            sender.starts_with("transit sender ") && sender.ends_with(" ready\n\n"),
            "{sender}"
        );
        assert!(
            receiver.starts_with("transit receiver ") && receiver.ends_with(" ready\n\n"),
            "{receiver}"
        );
        assert_eq!(
            sender.len(),
            "transit sender ".len() + 64 + " ready\n\n".len()
        );
        let relay = String::from_utf8(relay_handshake(&KEY, "0123456789abcdef")).unwrap();
        assert!(
            relay.starts_with("please relay ") && relay.ends_with(" for side 0123456789abcdef\n"),
            "{relay}"
        );
        assert_ne!(
            derive(&KEY, b"transit_sender"),
            derive(&KEY, b"transit_receiver")
        );
    }

    #[test]
    fn wormhole_transfer_the_nonce_counts_big_endian() {
        assert_eq!(nonce(0), [0u8; 24]);
        assert_eq!(nonce(1)[23], 1);
        assert_eq!(nonce(256)[22..], [1, 0]);
    }

    #[tokio::test]
    async fn wormhole_transfer_records_go_both_ways_and_a_changed_one_fails() {
        let (a, b) = tokio::io::duplex(1 << 20);
        let (mut sender, mut receiver) = (
            Records::new(a, &KEY, Role::Sender),
            Records::new(b, &KEY, Role::Receiver),
        );
        sender.send(b"one").await.unwrap();
        sender.send(&vec![9u8; CHUNK]).await.unwrap();
        assert_eq!(receiver.receive().await.unwrap(), b"one");
        assert_eq!(receiver.receive().await.unwrap().len(), CHUNK);
        receiver.send(b"ack").await.unwrap();
        assert_eq!(sender.receive().await.unwrap(), b"ack");

        // A record with one bit changed on the wire.
        let (mut wire, b) = tokio::io::duplex(1 << 16);
        let mut receiver = Records::new(b, &KEY, Role::Receiver);
        let (c, mut tap) = tokio::io::duplex(1 << 16);
        let mut sender = Records::new(c, &KEY, Role::Sender);
        sender.send(b"the data").await.unwrap();
        let mut bytes = vec![0u8; 4 + NONCE + 16 + 8];
        tap.read_exact(&mut bytes).await.unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        wire.write_all(&bytes).await.unwrap();
        let err = receiver.receive().await.unwrap_err();
        assert!(err.to_string().contains("did not open"), "{err}");
    }

    #[tokio::test]
    async fn wormhole_transfer_a_record_out_of_order_fails() {
        let (c, mut tap) = tokio::io::duplex(1 << 16);
        let mut sender = Records::new(c, &KEY, Role::Sender);
        sender.send(b"zero").await.unwrap();
        sender.send(b"one").await.unwrap();
        let mut first = vec![0u8; OVERHEAD + 4];
        tap.read_exact(&mut first).await.unwrap();
        let mut second = vec![0u8; OVERHEAD + 3];
        tap.read_exact(&mut second).await.unwrap();
        let (mut wire, b) = tokio::io::duplex(1 << 16);
        let mut receiver = Records::new(b, &KEY, Role::Receiver);
        wire.write_all(&second).await.unwrap();
        let err = receiver.receive().await.unwrap_err();
        assert!(err.to_string().contains("out of order"), "{err}");
    }
}
