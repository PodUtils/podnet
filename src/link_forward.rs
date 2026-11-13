//! The forwarding of a session stream through the mesh (T-287).
//!
//! A stream for a node that is not a neighbour goes to the next hop of the
//! best route ([`crate::mesh_table`]) and is opened again there, hop by hop.
//! The opener sends the first line `PNSVC1 fwd` of the service stream
//! ([`crate::link_service`]) and then a header line:
//!
//! ```text
//! PNFWD1 DEST HOPS SERVICE VISITED\n
//! ```
//!
//! `DEST` is the key of the destination in hex (64 characters). `HOPS` is the
//! number of links that the stream may still cross (0 to 16). `SERVICE` is
//! the service that the destination must serve. `VISITED` is the keys of the
//! nodes that the stream passed, in hex, separated by commas (at most 16).
//!
//! A node that gets the header:
//!
//! * is the destination: the stream is a request for `SERVICE` at this node
//!   ([`Arrived::Local`]); the caller answers it as any request.
//! * has `HOPS` 0: it answers [`service::HOPS_EXCEEDED`].
//! * is in `VISITED` (a loop): it answers [`service::NO_ROUTE`].
//! * else it tries the usable routes to `DEST`, best first, skipping a next
//!   hop that is in `VISITED`. For each it opens a stream to the next hop with
//!   `HOPS - 1` and itself added to `VISITED`, and waits [`OPEN_LIMIT`] for the
//!   answer. The answers [`service::RESOURCE_LIMIT`] and
//!   [`service::NO_ROUTE`], a stream that cannot be opened and the timeout
//!   make it try the next route. Any other answer goes back to the previous
//!   hop as it is. When it has no route left it answers the last error. At
//!   the answer 0 it answers 0 and copies the bytes both ways. The credit
//!   windows of the links give the back-pressure hop by hop.
//!
//! The bytes between the ends are sealed end to end when the ends run the
//! record layer ([`crate::link_record`], T-298): a hop sees records only.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use pod_identity::PublicKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::link::Stream;
use crate::link_announce::{now_ms, Mesh};
use crate::link_service as service;

/// The service name of a forwarded stream.
pub const NAME: &str = "fwd";

/// The default number of links that a stream may cross.
pub const DEFAULT_HOPS: u8 = 8;

/// The most links that a stream may cross.
pub const MAX_HOPS: u8 = 16;

/// The time that a hop waits for the answer of the next hop.
pub const OPEN_LIMIT: Duration = Duration::from_secs(30);

const TAG: &str = "PNFWD1 ";

/// The most bytes of the header line.
const MAX_LINE: usize = TAG.len() + 64 + 1 + 2 + 1 + service::NAME_LIMIT + 1 + 16 * 65;

/// The header of a forwarded stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The node that serves the stream.
    pub dest: PublicKey,
    /// The links that the stream may still cross.
    pub hops: u8,
    /// The service at the destination.
    pub service: String,
    /// The nodes that the stream passed.
    pub visited: Vec<PublicKey>,
}

fn hex(key: &PublicKey) -> String {
    key.0.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<PublicKey> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(PublicKey(out))
}

impl Header {
    /// The header line, with the line feed.
    pub fn line(&self) -> String {
        let visited: Vec<String> = self.visited.iter().map(hex).collect();
        format!(
            "{TAG}{} {} {} {}\n",
            hex(&self.dest),
            self.hops,
            self.service,
            visited.join(",")
        )
    }

    /// Read a header line (without the line feed). `None` for any other
    /// text.
    pub fn parse(line: &str) -> Option<Header> {
        let rest = line.strip_prefix(TAG)?;
        let mut parts = rest.split(' ');
        let dest = unhex(parts.next()?)?;
        let hops: u8 = parts.next()?.parse().ok()?;
        let name = parts.next()?;
        let visited = parts.next()?;
        if parts.next().is_some() || hops > MAX_HOPS || service::check_name(name).is_err() {
            return None;
        }
        let visited: Vec<PublicKey> = if visited.is_empty() {
            Vec::new()
        } else {
            visited.split(',').map(unhex).collect::<Option<_>>()?
        };
        if visited.len() > usize::from(MAX_HOPS) {
            return None;
        }
        Some(Header {
            dest,
            hops,
            service: name.to_string(),
            visited,
        })
    }
}

/// Why a forwarded stream was not opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardError {
    /// The hop limit is used up before the destination.
    HopsExceeded,
    /// A node has no stream to give, or did not answer in time.
    ResourceLimit,
    /// No usable route leads to the destination, or the route is a loop.
    NoRoute,
    /// The destination has no such service.
    NoService,
    /// The origin of the service did not answer.
    NoOrigin,
    /// A hop does not grant the service or the forwarding (T-290).
    Forbidden,
    /// A hop sent an answer that is not known.
    Refused(u8),
}

impl ForwardError {
    fn from_answer(byte: u8) -> ForwardError {
        match byte {
            service::NO_SERVICE => ForwardError::NoService,
            service::NO_ORIGIN => ForwardError::NoOrigin,
            service::HOPS_EXCEEDED => ForwardError::HopsExceeded,
            service::RESOURCE_LIMIT => ForwardError::ResourceLimit,
            service::NO_ROUTE => ForwardError::NoRoute,
            service::FORBIDDEN => ForwardError::Forbidden,
            other => ForwardError::Refused(other),
        }
    }

    /// The answer byte for the previous hop.
    pub fn answer(self) -> u8 {
        match self {
            ForwardError::HopsExceeded => service::HOPS_EXCEEDED,
            ForwardError::ResourceLimit => service::RESOURCE_LIMIT,
            ForwardError::NoService => service::NO_SERVICE,
            ForwardError::NoOrigin => service::NO_ORIGIN,
            ForwardError::Forbidden => service::FORBIDDEN,
            ForwardError::NoRoute | ForwardError::Refused(_) => service::NO_ROUTE,
        }
    }

    /// Whether the next route may work.
    fn retry(self) -> bool {
        matches!(self, ForwardError::ResourceLimit | ForwardError::NoRoute)
    }
}

impl fmt::Display for ForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForwardError::HopsExceeded => f.write_str("the hop limit is used up"),
            ForwardError::ResourceLimit => f.write_str("a hop has no stream to give"),
            ForwardError::NoRoute => f.write_str("no route leads to the destination"),
            ForwardError::NoService => f.write_str("the destination has no such service"),
            ForwardError::NoOrigin => f.write_str("the origin of the service did not answer"),
            ForwardError::Forbidden => f.write_str("a hop does not grant it"),
            ForwardError::Refused(b) => write!(f, "a hop answered {b}"),
        }
    }
}

impl std::error::Error for ForwardError {}

/// Open a stream to the next hop of `way`, send the request, read the
/// answer.
async fn open_one(
    link: &crate::link::Link,
    direct: bool,
    next: &Header,
) -> Result<Stream, ForwardError> {
    let mut stream = link.open().map_err(|_| ForwardError::ResourceLimit)?;
    let request = if direct {
        format!("{}{}\n", service::TAG, next.service)
    } else {
        format!("{}{NAME}\n{}", service::TAG, next.line())
    };
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| ForwardError::ResourceLimit)?;
    let mut answer = [0u8; 1];
    stream
        .read_exact(&mut answer)
        .await
        .map_err(|_| ForwardError::ResourceLimit)?;
    match answer[0] {
        service::OPEN => Ok(stream),
        other => Err(ForwardError::from_answer(other)),
    }
}

/// Open the stream of `header` through the best route and return it, open.
/// `header.visited` lists the nodes before this one. This node is added for
/// the next hop. A direct neighbour gets the plain request for the service.
pub async fn forward_open(mesh: &Arc<Mesh>, header: &Header) -> Result<Stream, ForwardError> {
    let me = mesh.public();
    if header.hops == 0 {
        return Err(ForwardError::HopsExceeded);
    }
    if header.visited.contains(&me) {
        return Err(ForwardError::NoRoute);
    }
    let ways = mesh.ways(&header.dest, &header.visited, now_ms());
    let mut next = header.clone();
    next.hops -= 1;
    next.visited.push(me);
    let mut last = ForwardError::NoRoute;
    for (peer, link) in ways {
        let direct = peer == header.dest;
        match tokio::time::timeout(OPEN_LIMIT, open_one(&link, direct, &next)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(why)) if why.retry() => last = why,
            Ok(Err(why)) => return Err(why),
            Err(_) => last = ForwardError::ResourceLimit,
        }
    }
    Err(last)
}

/// Open a stream from this node to the service `service` of the node
/// `dest`, over at most `hops` links (at most [`MAX_HOPS`]).
pub async fn open(
    mesh: &Arc<Mesh>,
    dest: &PublicKey,
    service: &str,
    hops: u8,
) -> Result<Stream, ForwardError> {
    let header = Header {
        dest: *dest,
        hops: hops.min(MAX_HOPS),
        service: service.to_string(),
        visited: Vec::new(),
    };
    forward_open(mesh, &header).await
}

/// What [`accept`] did with a stream.
#[derive(Debug, PartialEq, Eq)]
pub enum Arrived {
    /// This node is the destination. The request for the service is read and
    /// not answered yet: the caller answers it as it answers a request that
    /// came direct.
    Local(String),
    /// The stream was relayed to the next hop and has ended, or it was
    /// refused and answered.
    Done,
}

async fn read_line(stream: &mut Stream) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.ok()?;
        if byte[0] == b'\n' {
            return String::from_utf8(line).ok();
        }
        line.push(byte[0]);
        if line.len() > MAX_LINE {
            return None;
        }
    }
}

/// Take a stream that opened the service [`NAME`] (the first line is read):
/// read the header, then deliver the stream here or relay it.
pub async fn accept(mesh: &Arc<Mesh>, stream: &mut Stream) -> anyhow::Result<Arrived> {
    accept_with(mesh, stream, true, || {}).await
}

/// [`accept`] for a peer that may forward only when `may_forward` is true.
/// A stream for another node that the peer may not have relayed is answered
/// [`service::FORBIDDEN`], `refused` runs once, and the result is
/// [`Arrived::Done`]. The link stays up (T-290).
pub async fn accept_with(
    mesh: &Arc<Mesh>,
    stream: &mut Stream,
    may_forward: bool,
    refused: impl FnOnce(),
) -> anyhow::Result<Arrived> {
    let Some(header) = read_line(stream)
        .await
        .and_then(|line| Header::parse(&line))
    else {
        let _ = service::reply(stream, service::NO_ROUTE).await;
        anyhow::bail!("a forwarded stream has a bad header");
    };
    if header.dest == mesh.public() {
        return Ok(Arrived::Local(header.service));
    }
    if !may_forward {
        refused();
        tracing::warn!("forward: a peer that may not forward is refused");
        service::reply(stream, service::FORBIDDEN).await?;
        return Ok(Arrived::Done);
    }
    match forward_open(mesh, &header).await {
        Ok(mut down) => {
            service::reply(stream, service::OPEN).await?;
            let _ = tokio::io::copy_bidirectional(stream, &mut down).await;
        }
        Err(why) => {
            tracing::warn!(error = %why, "forward: a stream is refused");
            service::reply(stream, why.answer()).await?;
        }
    }
    Ok(Arrived::Done)
}
