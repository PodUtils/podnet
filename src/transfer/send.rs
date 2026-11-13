//! The sender: a code at the first mailbox that answers, then a text in the
//! offer itself, or a file or a directory (as a zip) over the transit.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pod_wormhole::mailbox::Mood;
use pod_wormhole::meet::or_close;
use pod_wormhole::{Code, Failed, Wormhole};
use sha2::{Digest, Sha256};

use super::messages::{self, Answer, Hint, Message, Offer, Transit};
use super::podnet_relay;
use super::transit::{Role, CHUNK};
use super::{connect, failed, podnet_peer, unavailable, versions, Settings, NOT_PODNET};

/// What a sender sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum What {
    Text(String),
    /// A file or a directory.
    Path(PathBuf),
}

/// A code that waits for its receiver.
pub struct Opened {
    hole: Wormhole,
    code: Code,
}

/// What a sender sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// The bytes that went over the transit (a zip for a directory); 0 for a
    /// text.
    pub bytes: u64,
    /// Their SHA-256 in hex, which the receiver acked; none for a text.
    pub sha256: Option<String>,
    /// The way of the data: `mailbox`, or the transit connection.
    pub via: String,
}

/// A new code at the first mailbox of `settings` that answers.
pub async fn open(settings: &Settings) -> Result<Opened> {
    let (hole, code) = pod_wormhole::opened(&settings.ends, versions())
        .await
        .map_err(failed)?;
    Ok(Opened { hole, code })
}

/// The payload of a file or a directory: its offer and its bytes.
struct Payload {
    offer: Offer,
    file: File,
    size: u64,
}

impl Opened {
    /// The code, to show to a person.
    pub fn code(&self) -> &str {
        self.code.for_terminal()
    }

    /// The URL of the mailbox, as lines show it.
    pub fn mailbox(&self) -> String {
        self.hole.shown().to_string()
    }

    /// Wait for the receiver, then send `what`. A file or a directory is
    /// read before the receiver comes, so that its offer is ready.
    pub async fn send(self, settings: &Settings, what: &What) -> Result<Sent> {
        let Opened { mut hole, .. } = self;
        let payload = match what {
            What::Text(_) => None,
            What::Path(path) => match payload(path) {
                Ok(payload) => Some(payload),
                Err(err) => {
                    hole.close(Mood::Errory).await;
                    return Err(err);
                }
            },
        };
        if !hole.agree(settings.wait).await.map_err(failed)? {
            hole.close(Mood::Lonely).await;
            return Err(failed(Failed::Expired(settings.wait)));
        }
        let theirs = hole.confirm(settings.step).await.map_err(failed)?;
        match (what, payload) {
            (What::Text(text), _) => send_text(&mut hole, settings, text).await,
            (What::Path(_), Some(payload)) => {
                if !podnet_peer(&theirs) {
                    let _ = hole.send(&messages::error(NOT_PODNET)).await;
                    hole.close(Mood::Errory).await;
                    return Err(anyhow::anyhow!(NOT_PODNET));
                }
                send_payload(hole, settings, payload).await
            }
            (What::Path(_), None) => unreachable!("a path has its payload"),
        }
    }
}

async fn send_text(hole: &mut Wormhole, settings: &Settings, text: &str) -> Result<Sent> {
    let sent = hole.send(&Offer::Message(text.to_string()).encode()).await;
    or_close(hole, sent).await.map_err(failed)?;
    loop {
        let got = hole.receive(settings.wait).await.map_err(failed)?;
        match messages::parse(&got) {
            Ok(Message::Answer(Answer::MessageAck)) => break,
            Ok(Message::Error(words)) => return Err(peer_error(hole, &words).await),
            Ok(Message::Answer(Answer::Other(words))) => return Err(peer_error(hole, &words).await),
            Ok(_) => continue,
            Err(why) => return Err(failed(hole.fail(Mood::Errory, Failed::Peer(why)).await)),
        }
    }
    hole.close(Mood::Happy).await;
    Ok(Sent {
        bytes: 0,
        sha256: None,
        via: "mailbox".into(),
    })
}

async fn peer_error(hole: &mut Wormhole, words: &str) -> anyhow::Error {
    hole.close(Mood::Errory).await;
    anyhow::anyhow!("the receiver refused the transfer: {words}")
}

async fn send_payload(
    mut hole: Wormhole,
    settings: &Settings,
    mut payload: Payload,
) -> Result<Sent> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|_| failed(Failed::NoRandom))?;
    let mut ours =
        match connect::prepare(&settings.listen, &settings.relays, &settings.piping).await {
            Ok(ours) => ours,
            Err(err) => {
                hole.close(Mood::Errory).await;
                return Err(unavailable(format!("no listener for the transit: {err}")));
            }
        };
    let mut abilities = vec![messages::DIRECT.to_string(), messages::RELAY.to_string()];
    if !settings.piping.is_empty() {
        abilities.push(messages::PIPING.to_string());
    }
    // The node of the podnet relay runs until the end of this function.
    let node = match settings.relay_pair.as_ref().map(podnet_relay::start) {
        Some(Ok((node, sessions))) => {
            abilities.push(messages::PODNET_RELAY.to_string());
            ours.hints.push(node.hint.clone());
            if let Hint::PodnetRelay { name, .. } = &node.hint {
                ours.node = Some((name.clone(), sessions));
            }
            Some(node)
        }
        Some(Err(err)) => {
            let words = format!("the sender cannot run the node of its relay pair: {err:#}");
            let _ = hole.send(&messages::error(&words)).await;
            hole.close(Mood::Errory).await;
            return Err(err);
        }
        None => None,
    };
    let transit = Transit {
        abilities,
        hints: ours.hints.clone(),
        key: Some(key),
        relay_token: node.as_ref().map(|n| n.token.clone()),
    };
    let sent = hole.send(&transit.encode()).await;
    or_close(&mut hole, sent).await.map_err(failed)?;
    let sent = hole.send(&payload.offer.encode()).await;
    or_close(&mut hole, sent).await.map_err(failed)?;
    let mut theirs: Vec<Hint> = Vec::new();
    loop {
        let got = hole.receive(settings.wait).await.map_err(failed)?;
        match messages::parse(&got) {
            Ok(Message::Transit(transit)) => theirs = transit.hints,
            Ok(Message::Answer(Answer::FileAck)) => break,
            Ok(Message::Answer(Answer::Other(words))) | Ok(Message::Error(words)) => {
                return Err(peer_error(&mut hole, &words).await);
            }
            Ok(_) => continue,
            Err(why) => return Err(failed(hole.fail(Mood::Errory, Failed::Peer(why)).await)),
        }
    }
    hole.close(Mood::Happy).await;
    let way = settings.transit_way();
    let (mut records, via) = connect::connect(
        Role::Sender,
        &key,
        ours,
        &theirs,
        &way,
        settings.transit_wait,
    )
    .await
    .map_err(unavailable)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut left = payload.size;
    payload.file.seek(SeekFrom::Start(0))?;
    while left > 0 {
        let want = (left.min(CHUNK as u64)) as usize;
        payload
            .file
            .read_exact(&mut buf[..want])
            .context("the file changed while it was sent")?;
        hash.update(&buf[..want]);
        records
            .send(&buf[..want])
            .await
            .map_err(|e| unavailable(format!("the transit ({via}) ended: {e}")))?;
        left -= want as u64;
    }
    let sha256 = pod_wormhole::hex(&hash.finalize());
    let ack = records
        .receive()
        .await
        .map_err(|e| unavailable(format!("no ack came over the transit ({via}): {e}")))?;
    records.close().await;
    if node.is_some() {
        // The end of this side must pass the node before it stops: wait for
        // the end of the receiver, then for the layer of the node.
        records.drain(podnet_relay::GRACE).await;
        tokio::time::sleep(podnet_relay::GRACE / 4).await;
    }
    drop(node);
    match messages::read_ack(&ack) {
        Ok(Some(theirs)) if theirs != sha256 => Err(super::refused(format!(
            "the receiver got other data: SHA-256 {theirs}, sent {sha256}"
        ))),
        Ok(_) => Ok(Sent {
            bytes: payload.size,
            sha256: Some(sha256),
            via,
        }),
        Err(why) => Err(anyhow::anyhow!(why)),
    }
}

/// The offer and the bytes of the file or the directory at `path`.
fn payload(path: &Path) -> Result<Payload> {
    let meta = std::fs::metadata(path).with_context(|| format!("{}", path.display()))?;
    let name = path
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .ok_or_else(|| super::refused(format!("{}: no name to send", path.display())))?;
    if meta.is_file() {
        let file = File::open(path).with_context(|| format!("{}", path.display()))?;
        return Ok(Payload {
            offer: Offer::File {
                name,
                size: meta.len(),
            },
            file,
            size: meta.len(),
        });
    }
    if !meta.is_dir() {
        return Err(super::refused(format!(
            "{}: podnet sends a file or a directory",
            path.display()
        )));
    }
    let mut file = anonymous_file()?;
    let written = super::zip::write(path, &mut file)
        .map_err(|e| super::refused(format!("{}: {e}", path.display())))?;
    let offer = Offer::Directory {
        mode: messages::ZIP_MODE.into(),
        name,
        zipsize: written.zipsize,
        numbytes: written.numbytes,
        numfiles: written.numfiles,
    };
    Ok(Payload {
        offer,
        file,
        size: written.zipsize,
    })
}

/// A new file in the directory of temporary files that no name holds: it
/// goes when it is closed.
pub(super) fn anonymous_file() -> Result<File> {
    let dir = std::env::temp_dir();
    anonymous_in(&dir)
}

/// [`anonymous_file`] in `dir`.
pub(super) fn anonymous_in(dir: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut tag = [0u8; 8];
    getrandom::fill(&mut tag).map_err(|_| failed(Failed::NoRandom))?;
    let path = dir.join(format!(".podnet-transfer-{}", pod_wormhole::hex(&tag)));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .with_context(|| format!("a file for the zip in {}", dir.display()))?;
    std::fs::remove_file(&path).with_context(|| format!("{}", path.display()))?;
    Ok(file)
}
