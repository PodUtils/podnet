//! The service of a stream of a link (T-266).
//!
//! The hello admits a peer for the whole link. It binds no service. The
//! opener of a stream names the service it wants in the first line of the
//! stream, and the other end answers with one byte before any data flows:
//!
//! ```text
//! opener -> PNSVC1 NAME\n
//! other  -> one byte: 0 the stream is open, 1 no such service,
//!                     2 the origin of the service did not answer
//! ```
//!
//! A name is 1 to 32 characters from `A-Z a-z 0-9 . _ -`. The default
//! service is [`RAW`]. The listener maps a name to a target (`--service
//! NAME=TARGET`; `--to TARGET` is the service `raw`).

use anyhow::{anyhow, Context};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::exit::{self, Fault};
use crate::link::Stream;

/// The tag at the start of the first line of a stream.
const TAG: &str = "PNSVC1 ";

/// The most characters of a service name.
pub const NAME_LIMIT: usize = 32;

/// The service that carries bytes to the target of `--to`.
pub const RAW: &str = "raw";

/// The answer: the stream is open.
pub const OPEN: u8 = 0;
/// The answer: the other end has no service of this name.
pub const NO_SERVICE: u8 = 1;
/// The answer: the origin of the service did not answer.
pub const NO_ORIGIN: u8 = 2;

/// Check a service name. A bad name exits 64.
pub fn check_name(name: &str) -> anyhow::Result<()> {
    let good = !name.is_empty()
        && name.len() <= NAME_LIMIT
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if good {
        return Ok(());
    }
    Err(exit::fault(
        Fault::Usage,
        anyhow!(
            "{name:?} is no service name: use 1 to {NAME_LIMIT} characters from A-Z a-z 0-9 . _ -"
        ),
    ))
}

/// Ask the other end of `stream` for the service `name` and wait for its
/// answer. A service that does not exist, or whose origin does not answer,
/// exits 69.
pub async fn request(stream: &mut Stream, name: &str) -> anyhow::Result<()> {
    check_name(name)?;
    let unavailable = |e: anyhow::Error| exit::fault(Fault::Unavailable, e);
    stream
        .write_all(format!("{TAG}{name}\n").as_bytes())
        .await
        .context("the peer ended the stream")
        .map_err(unavailable)?;
    let mut answer = [0u8; 1];
    stream
        .read_exact(&mut answer)
        .await
        .context("the peer ended the stream before it answered")
        .map_err(unavailable)?;
    match answer[0] {
        OPEN => Ok(()),
        NO_SERVICE => Err(exit::fault(
            Fault::Unavailable,
            anyhow!("the peer has no service {name:?}"),
        )),
        NO_ORIGIN => Err(exit::fault(
            Fault::Unavailable,
            anyhow!("the origin of the service {name:?} did not answer"),
        )),
        other => Err(exit::fault(
            Fault::Unavailable,
            anyhow!("the peer answered {other} to the service {name:?}"),
        )),
    }
}

/// Read the first line of a stream that a peer opened: the service name.
/// A line that is no request exits 69 (the peer does not speak it).
pub async fn accept(stream: &mut Stream) -> anyhow::Result<String> {
    let mut line = Vec::with_capacity(TAG.len() + NAME_LIMIT + 1);
    let mut byte = [0u8; 1];
    loop {
        stream
            .read_exact(&mut byte)
            .await
            .context("the peer ended the stream before it named a service")
            .map_err(|e| exit::fault(Fault::Unavailable, e))?;
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        if line.len() > TAG.len() + NAME_LIMIT {
            return Err(exit::fault(
                Fault::Unavailable,
                anyhow!("the first line of the stream is too long"),
            ));
        }
    }
    let line = String::from_utf8_lossy(&line).into_owned();
    let name = line.strip_prefix(TAG).ok_or_else(|| {
        exit::fault(
            Fault::Unavailable,
            anyhow!("the stream does not start with a service request"),
        )
    })?;
    check_name(name).map_err(|e| exit::fault(Fault::Unavailable, e))?;
    Ok(name.to_string())
}

/// Send the answer to a request.
pub async fn reply(stream: &mut Stream, answer: u8) -> anyhow::Result<()> {
    stream
        .write_all(&[answer])
        .await
        .context("the peer ended the stream")
        .map_err(|e| exit::fault(Fault::Unavailable, e))
}
