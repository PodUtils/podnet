//! The hello of a link (T-265): each end proves its Ed25519 key to the
//! other end before the link offers any service.
//!
//! A carrier of `pod-link` moves bytes and knows no peer key. A unix
//! socket checks the uid of the peer only. The hello runs on the first
//! stream of the link, which the dialer opens. It has four steps:
//!
//! 1. Dialer to listener: the tag, the public key of the dialer, a nonce.
//! 2. Listener to dialer: its public key, a nonce, and its signature over
//!    the transcript. The dialer checks the signature, and checks the key
//!    against its allow list. A key that the list does not hold ends the
//!    hello at the dialer (exit 77).
//! 3. Dialer to listener: its signature over the transcript.
//! 4. Listener to dialer: one byte, 0 for admitted, 1 for refused. The
//!    listener checks the signature and its allow list before it sends the
//!    byte (exit 77 for a key that the list does not hold).
//!
//! The transcript holds both public keys and both nonces, and the role of
//! the signer, so a signature of one role is no proof for the other role
//! and a signature of another hello is no proof for this one.
//!
//! Limit: the hello proves the keys of the two ends, but it makes no key
//! for the link and does not bind the streams after it. It is for the
//! carriers whose channel the kernel keeps on this host (unix, memory). A
//! carrier that crosses a network must not rely on it: a party between the
//! ends could pass the hello on and then take the link. Such a carrier
//! takes the end-to-end handshake of `pod-e2e` (T-298) or its own TLS
//! handshake with the pinned key (T-268).

use std::time::Duration;

use anyhow::{anyhow, Context};
use pod_core::entropy::{Entropy, OsEntropy};
use pod_identity::PublicKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

use crate::exit::{self, Fault};
use crate::identity::{AllowList, Identity};
use crate::link::{Link, Stream};

/// The first bytes of a hello.
const TAG: &[u8; 8] = b"PNLINK01";

/// The time that a hello can take.
pub const LIMIT: Duration = Duration::from_secs(10);

const ADMITTED: u8 = 0;
const REFUSED: u8 = 1;

fn transcript(
    role: &str,
    dialer: &PublicKey,
    listener: &PublicKey,
    nd: &[u8],
    nl: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(160);
    out.extend_from_slice(b"podnet link hello 1 ");
    out.extend_from_slice(role.as_bytes());
    out.extend_from_slice(&dialer.0);
    out.extend_from_slice(&listener.0);
    out.extend_from_slice(nd);
    out.extend_from_slice(nl);
    out
}

fn nonce() -> anyhow::Result<[u8; 32]> {
    let mut n = [0u8; 32];
    OsEntropy.fill(&mut n).map_err(|e| {
        exit::fault(
            Fault::OsError,
            anyhow!("no random bytes for the hello: {e}"),
        )
    })?;
    Ok(n)
}

fn refused(key: &PublicKey, why: &str) -> anyhow::Error {
    exit::fault(
        Fault::NoPermission,
        anyhow!("the peer {}: {why}", key.display_fingerprint()),
    )
}

async fn read<const N: usize>(stream: &mut Stream) -> anyhow::Result<[u8; N]> {
    let mut buf = [0u8; N];
    stream
        .read_exact(&mut buf)
        .await
        .context("the peer ended the hello")
        .map_err(|e| exit::fault(Fault::Unavailable, e))?;
    Ok(buf)
}

async fn write(stream: &mut Stream, bytes: &[u8]) -> anyhow::Result<()> {
    stream
        .write_all(bytes)
        .await
        .context("the peer ended the hello")
        .map_err(|e| exit::fault(Fault::Unavailable, e))
}

async fn within<T>(f: impl std::future::Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    match timeout(LIMIT, f).await {
        Ok(r) => r,
        Err(_) => Err(exit::fault(
            Fault::Timeout,
            anyhow!("the hello did not end in {} s", LIMIT.as_secs()),
        )),
    }
}

/// Run the hello as the dialer of `link`: open its first stream and prove
/// `me`. The listener must prove a key that `allow` holds. The result is
/// the key of the listener.
pub async fn dial(link: &Link, me: &Identity, allow: AllowList) -> anyhow::Result<PublicKey> {
    within(async {
        let mut s = link.open().map_err(crate::link::fault)?;
        let (mine, nd) = (me.public(), nonce()?);
        let mut hello = Vec::with_capacity(72);
        hello.extend_from_slice(TAG);
        hello.extend_from_slice(&mine.0);
        hello.extend_from_slice(&nd);
        write(&mut s, &hello).await?;
        let reply = read::<128>(&mut s).await?;
        let theirs = PublicKey(reply[..32].try_into()?);
        let nl = &reply[32..64];
        let sig_l: [u8; 64] = reply[64..].try_into()?;
        if !theirs.verifies(&transcript("listener", &mine, &theirs, &nd, nl), &sig_l) {
            return Err(refused(&theirs, "its signature is wrong"));
        }
        allow.check(&theirs)?;
        let sig = me
            .key()
            .sign(&transcript("dialer", &mine, &theirs, &nd, nl));
        write(&mut s, &sig).await?;
        match read::<1>(&mut s).await?[0] {
            ADMITTED => Ok(theirs),
            _ => Err(exit::fault(
                Fault::NoPermission,
                anyhow!(
                    "the peer {} does not admit this install ({}): add it to the allow list of the peer",
                    theirs.display_fingerprint(),
                    me.fingerprint()
                ),
            )),
        }
    })
    .await
}

/// Run the hello as the listener of `link`: wait for its first stream and
/// prove `me`. The dialer must prove a key that `allow` holds. The result
/// is the key of the dialer. A dialer that `allow` does not hold gets the
/// byte 1, and the result is an error (exit 77).
pub async fn accept(link: &Link, me: &Identity, allow: AllowList) -> anyhow::Result<PublicKey> {
    within(async {
        let mut s = link.accept().await.map_err(crate::link::fault)?;
        let head = read::<72>(&mut s).await?;
        if &head[..8] != TAG {
            return Err(exit::fault(
                Fault::Unavailable,
                anyhow!("the peer does not speak the hello of podnet link"),
            ));
        }
        let theirs = PublicKey(head[8..40].try_into()?);
        let nd = &head[40..72];
        let (mine, nl) = (me.public(), nonce()?);
        let sig = me
            .key()
            .sign(&transcript("listener", &theirs, &mine, nd, &nl));
        let mut reply = Vec::with_capacity(128);
        reply.extend_from_slice(&mine.0);
        reply.extend_from_slice(&nl);
        reply.extend_from_slice(&sig);
        write(&mut s, &reply).await?;
        let sig_d: [u8; 64] = read::<64>(&mut s).await?;
        let proved = theirs.verifies(&transcript("dialer", &theirs, &mine, nd, &nl), &sig_d);
        let verdict = if proved {
            allow.check(&theirs)
        } else {
            Err(refused(&theirs, "its signature is wrong"))
        };
        let byte = if verdict.is_ok() { ADMITTED } else { REFUSED };
        write(&mut s, &[byte]).await?;
        verdict.map(|()| theirs)
    })
    .await
}
