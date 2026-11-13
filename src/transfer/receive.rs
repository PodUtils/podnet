//! The receiver: the giver of a code, its offer, the question to a person,
//! then the text, or the bytes of the transit written by the rules of
//! [`super::land`].

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use pod_wormhole::mailbox::Mood;
use pod_wormhole::meet::or_close;
use pod_wormhole::{Code, Failed, Wormhole};
use sha2::{Digest, Sha256};

use super::land::{self, Caps, Landing};
use super::messages::{self, Answer, Message, Offer, Transit};
use super::transit::{Records, Role};
use super::{connect, failed, refused, unavailable, versions, Settings, NOT_PODNET};

/// How a receiver writes.
#[derive(Debug, Clone)]
pub struct Receive {
    /// The directory where the file or the directory lands. It must exist.
    pub out: PathBuf,
    pub caps: Caps,
}

/// What the sender offers, for the question to a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    /// `file` or `directory`.
    pub kind: &'static str,
    pub name: String,
    /// The bytes of the file, or of the files of the directory.
    pub bytes: u64,
    /// The entries of a directory; 1 for a file.
    pub files: u64,
}

/// What a receiver got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    Text(String),
    /// A file or a directory at `path`.
    Path {
        path: PathBuf,
        /// The bytes that came over the transit (a zip for a directory).
        bytes: u64,
        sha256: String,
        /// The files and directories written.
        files: u64,
        via: String,
    },
}

/// Take `code`, read its offer, ask `ask` for a file or a directory, and
/// receive it under `how.out`. `ask` gives false to refuse the offer.
pub async fn receive(
    settings: &Settings,
    code: &Code,
    how: &Receive,
    ask: &mut (dyn FnMut(&Offered) -> Result<bool> + Send),
) -> Result<Received> {
    let mut hole = pod_wormhole::joined(&settings.ends, code, versions(), settings.giver_wait)
        .await
        .map_err(failed)?;
    let theirs = hole.confirm(settings.step).await.map_err(failed)?;
    let mut transit: Option<Transit> = None;
    loop {
        let got = hole.receive(settings.wait).await.map_err(failed)?;
        let offer = match messages::parse(&got) {
            Ok(Message::Transit(t)) => {
                transit = Some(t);
                continue;
            }
            Ok(Message::Offer(offer)) => offer,
            Ok(Message::Error(words)) => {
                hole.close(Mood::Errory).await;
                return Err(anyhow::anyhow!("the sender ended the transfer: {words}"));
            }
            Ok(_) => continue,
            Err(why) => return Err(failed(hole.fail(Mood::Errory, Failed::Peer(why)).await)),
        };
        if let Offer::Message(text) = offer {
            let sent = hole.send(&messages::answer(&Answer::MessageAck)).await;
            or_close(&mut hole, sent).await.map_err(failed)?;
            hole.close(Mood::Happy).await;
            return Ok(Received::Text(text));
        }
        let offered = match check(&offer, how) {
            Ok(offered) => offered,
            Err(err) => return Err(decline(&mut hole, err).await),
        };
        let key = match transit.as_ref().and_then(|t| t.key) {
            Some(key) if super::podnet_peer(&theirs) => key,
            _ => return Err(decline(&mut hole, anyhow::anyhow!(NOT_PODNET)).await),
        };
        match ask(&offered) {
            Ok(true) => {}
            Ok(false) => return Err(decline(&mut hole, anyhow::anyhow!("transfer rejected")).await),
            Err(err) => return Err(decline(&mut hole, err).await),
        }
        let theirs = transit.unwrap_or_default();
        return land_offer(hole, settings, how, offer, offered, key, theirs).await;
    }
}

/// Send `err` as the error of this end, close, and give it back.
async fn decline(hole: &mut Wormhole, err: anyhow::Error) -> anyhow::Error {
    let _ = hole.send(&messages::error(&format!("{err:#}"))).await;
    hole.close(Mood::Errory).await;
    err
}

/// The offer of a file or a directory, checked by the rules of the top
/// name and the caps.
fn check(offer: &Offer, how: &Receive) -> Result<Offered> {
    let (kind, name, bytes, files, wire) = match offer {
        Offer::File { name, size } => ("file", name, *size, 1, *size),
        Offer::Directory {
            mode,
            name,
            zipsize,
            numbytes,
            numfiles,
        } => {
            if !mode.starts_with("zipfile") {
                return Err(refused(format!(
                    "a directory in the mode {mode:?}: podnet reads a zip"
                )));
            }
            ("directory", name, *numbytes, *numfiles, *zipsize)
        }
        Offer::Message(_) => unreachable!("a text has no name"),
    };
    let top = land::top(name).map_err(refused)?;
    if bytes > how.caps.max_bytes || wire > how.caps.max_bytes {
        return Err(refused(format!(
            "{bytes} bytes: the cap is {} (--max-bytes)",
            how.caps.max_bytes
        )));
    }
    if files > how.caps.max_files {
        return Err(refused(format!(
            "{files} entries: the cap is {} (--max-files)",
            how.caps.max_files
        )));
    }
    if std::fs::symlink_metadata(how.out.join(&top)).is_ok() {
        return Err(refused(format!(
            "{top:?} exists (EEXIST): the receiver never overwrites"
        )));
    }
    Ok(Offered {
        kind,
        name: top,
        bytes,
        files,
    })
}

#[allow(clippy::too_many_arguments)]
async fn land_offer(
    mut hole: Wormhole,
    settings: &Settings,
    how: &Receive,
    offer: Offer,
    offered: Offered,
    key: [u8; 32],
    theirs: Transit,
) -> Result<Received> {
    let hints = theirs.hints;
    let mut ours = match connect::prepare(&settings.listen, &settings.relays).await {
        Ok(ours) => ours,
        Err(err) => {
            return Err(decline(
                &mut hole,
                unavailable(format!("no listener for the transit: {err}")),
            )
            .await)
        }
    };
    ours.token = theirs.relay_token;
    ours.resume = settings.relay_resume;
    let abilities = vec![
        messages::DIRECT.to_string(),
        messages::RELAY.to_string(),
        messages::PODNET_RELAY.to_string(),
    ];
    let mine = Transit {
        abilities,
        hints: ours.hints.clone(),
        key: None,
        relay_token: None,
    };
    let sent = hole.send(&mine.encode()).await;
    or_close(&mut hole, sent).await.map_err(failed)?;
    let sent = hole.send(&messages::answer(&Answer::FileAck)).await;
    or_close(&mut hole, sent).await.map_err(failed)?;
    hole.close(Mood::Happy).await;
    let way = settings.transit_way();
    let (mut records, via) = connect::connect(
        Role::Receiver,
        &key,
        ours,
        &hints,
        &way,
        settings.transit_wait,
    )
    .await
    .map_err(unavailable)?;
    let (bytes, sha256, files) = match offer {
        Offer::File { size, .. } => {
            let mut landing = Landing::open(&how.out).map_err(refused)?;
            let mut file = landing
                .file(std::slice::from_ref(&offered.name))
                .map_err(refused)?;
            let sha256 = take(&mut records, &mut file, size, &via).await?;
            file.sync_all()?;
            landing.keep();
            (size, sha256, 1)
        }
        Offer::Directory { zipsize, .. } => {
            let mut zip = super::send::anonymous_in(&how.out)?;
            let sha256 = take(&mut records, &mut zip, zipsize, &via).await?;
            match land::unzip(&mut zip, &how.out, &offered.name, &how.caps) {
                Ok(files) => (zipsize, sha256, files),
                Err(err) => {
                    let words =
                        serde_json::json!({ "ack": format!("refused: {err:#}") }).to_string();
                    let _ = records.send(words.as_bytes()).await;
                    return Err(err);
                }
            }
        }
        Offer::Message(_) => unreachable!("a text has no transit"),
    };
    let ack = pod_wormhole::unhex(&sha256).unwrap_or_default();
    records
        .send(&messages::ack(&ack))
        .await
        .map_err(|e| unavailable(format!("the ack did not go ({via}): {e}")))?;
    records.close().await;
    Ok(Received::Path {
        path: how.out.join(&offered.name),
        bytes,
        sha256,
        files,
        via,
    })
}

/// Read `size` bytes of records into `out`: their SHA-256 in hex. More or
/// fewer bytes than the offer said is an error.
async fn take(
    records: &mut Records<pod_core::BoxedIo>,
    out: &mut File,
    size: u64,
    via: &str,
) -> Result<String> {
    let mut hash = Sha256::new();
    let mut got = 0u64;
    while got < size {
        let record = records.receive().await.map_err(|e| {
            unavailable(format!(
                "the transit ({via}) ended after {got} of {size} bytes: {e}"
            ))
        })?;
        got += record.len() as u64;
        if got > size {
            return Err(refused(format!(
                "the sender sent more than the {size} bytes of its offer"
            )));
        }
        hash.update(&record);
        out.write_all(&record)?;
    }
    Ok(pod_wormhole::hex(&hash.finalize()))
}
