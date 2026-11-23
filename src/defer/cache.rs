//! The response cache of the relay (DEFER-1 to DEFER-4).
//!
//! A [`Defer`] holds one origin and one prepared response. [`Defer::serve`]
//! answers a request:
//!
//! * a prepared response of the same key is served after half of the
//!   origin's delay: the copy was made while the last client was served, so
//!   the timing is plausible and never a replacement in line (DEFER-1);
//! * any other request is served live, with the origin's own timing, and its
//!   answer is then prepared for the next one (DEFER-4).
//!
//! The chunk size bounds the bytes of one prepared response. A response that
//! is larger is not prepared, so the bandwidth that the cache adds stays
//! within the chunk (DEFER-2).

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::carrier::Carrier;

/// The origin of a request: a service that gives the bytes of one response.
pub trait Origin {
    /// The response of `request`. The error ends the request, never the
    /// process.
    fn fetch<'a>(
        &'a mut self,
        request: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<u8>>> + Send + 'a>>;
}

/// An HTTP/1.1 origin on the loopback or on the network.
pub struct HttpOrigin {
    address: String,
}

impl HttpOrigin {
    /// An origin at `address`, `HOST:PORT`.
    pub fn new(address: impl Into<String>) -> HttpOrigin {
        HttpOrigin {
            address: address.into(),
        }
    }
}

impl Origin for HttpOrigin {
    fn fetch<'a>(
        &'a mut self,
        request: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<u8>>> + Send + 'a>> {
        Box::pin(async move {
            // EGRESS-3 and EGRESS-4: the origin of a deferred response is a
            // destination of the deployment's policy before the socket opens.
            let (host, port) = self.address.rsplit_once(':').ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "no port in HOST:PORT")
            })?;
            let port: u16 = port.parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "the port is not 0 to 65535")
            })?;
            crate::policy::permit_io(host.trim_matches(['[', ']']), port)?;
            let mut tcp = tokio::net::TcpStream::connect(&self.address).await?;
            tcp.write_all(request).await?;
            tcp.flush().await?;
            let mut out = Vec::new();
            tcp.read_to_end(&mut out).await?;
            Ok(out)
        })
    }
}

/// How a response was served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    /// Live from the origin, with its own delay.
    Live(Duration),
    /// From the prepared copy, after the plausible delay.
    Replay(Duration),
}

impl Timing {
    /// The delay that the client waited.
    pub fn delay(self) -> Duration {
        match self {
            Timing::Live(delay) | Timing::Replay(delay) => delay,
        }
    }

    /// True for a prepared copy.
    pub fn replayed(self) -> bool {
        matches!(self, Timing::Replay(_))
    }
}

/// One response and how it was served.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Served {
    pub bytes: Vec<u8>,
    pub timing: Timing,
}

/// The prepared copy of one key.
struct Prepared {
    key: Vec<u8>,
    bytes: Vec<u8>,
    delay: Duration,
}

/// The response cache of one relay.
pub struct Defer<O: Origin> {
    origin: O,
    chunk: usize,
    prepared: Option<Prepared>,
    /// The control stream inside the responses of this cache (T-371).
    carrier: Option<Carrier>,
    /// The bytes of prepared copies that a later prepare replaced.
    dropped: u64,
    live: u64,
    replayed: u64,
    misses: u64,
}

impl<O: Origin> Defer<O> {
    /// A cache of `origin` whose prepared responses hold `chunk` bytes.
    pub fn new(origin: O, chunk: usize) -> Defer<O> {
        Defer {
            origin,
            chunk,
            prepared: None,
            carrier: None,
            dropped: 0,
            live: 0,
            replayed: 0,
            misses: 0,
        }
    }

    /// The chunk size: the most bytes of one prepared response.
    pub fn chunk(&self) -> usize {
        self.chunk
    }

    /// Carry `carrier` inside the responses of this cache (#494): one frame
    /// per response, in the `X-Pod-Carrier` header, when the response with
    /// it fits the chunk. The body stays the origin's bytes.
    pub fn carrying(mut self, carrier: Carrier) -> Defer<O> {
        self.carrier = Some(carrier);
        self
    }

    /// The carrier of this cache, to queue frames.
    pub fn carrier_mut(&mut self) -> Option<&mut Carrier> {
        self.carrier.as_mut()
    }

    /// The responses that the origin served.
    pub fn live(&self) -> u64 {
        self.live
    }

    /// The responses that the prepared copy served.
    pub fn replayed(&self) -> u64 {
        self.replayed
    }

    /// The requests that a live answer served because no copy was ready.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// The bytes of the copy that the cache holds now. This is the storage
    /// of the prepared copy, not a count of the bytes on the wire: a
    /// bandwidth test measures what the server writes (`tests/defer.rs`).
    pub fn overhead(&self) -> u64 {
        self.prepared_bytes() as u64
    }

    /// The bytes of prepared copies that a later prepare replaced. The
    /// count is for a report; the cache holds one copy at a time.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The bytes of the prepared copy that is held now.
    pub fn prepared_bytes(&self) -> usize {
        self.prepared.as_ref().map_or(0, |p| p.bytes.len())
    }

    /// Serve `request`: from the prepared copy when its key matches, else
    /// live. A live answer prepares the next response of the same key.
    pub async fn serve(&mut self, request: &[u8]) -> io::Result<Served> {
        let key = key_of(request);
        if self.prepared.as_ref().is_some_and(|p| p.key == key) {
            let prepared = self.prepared.take().expect("the key matched");
            let delay = prepared.delay;
            tokio::time::sleep(delay).await;
            self.replayed += 1;
            return Ok(Served {
                bytes: self.decorate(prepared.bytes),
                timing: Timing::Replay(delay),
            });
        }
        self.misses += 1;
        let start = Instant::now();
        let bytes = self.origin.fetch(request).await?;
        let delay = start.elapsed();
        self.live += 1;
        // A GET or a HEAD is safe to answer again; a POST or a PUT is not.
        if is_cacheable(request) {
            self.prepare(key, &bytes, delay);
        } else {
            self.drop_prepared();
        }
        Ok(Served {
            bytes: self.decorate(bytes),
            timing: Timing::Live(delay),
        })
    }

    /// Add the next carrier frame to `bytes`, within the chunk. The prepared
    /// copy stays clean, so the overhead of the cache does not grow.
    fn decorate(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        match &mut self.carrier {
            Some(carrier) => carrier.attach(bytes, self.chunk),
            None => bytes,
        }
    }

    /// Prepare `bytes` for the next request of `key`. A response larger than
    /// the chunk is not prepared: it is served live each time.
    fn prepare(&mut self, key: Vec<u8>, bytes: &[u8], delay: Duration) {
        if bytes.len() > self.chunk {
            self.drop_prepared();
            return;
        }
        self.drop_prepared();
        self.prepared = Some(Prepared {
            key,
            bytes: bytes.to_vec(),
            // Half of the origin's delay: a copy that answers at once would
            // be the detector's easiest case (DEFER-1).
            delay: delay / 2,
        });
    }

    fn drop_prepared(&mut self) {
        if let Some(old) = self.prepared.take() {
            self.dropped += old.bytes.len() as u64;
        }
    }
}

/// True for a request that a prepared copy may answer again.
pub fn is_cacheable(request: &[u8]) -> bool {
    let line = request.split(|b| *b == b'\r').next().unwrap_or(request);
    line.starts_with(b"GET ") || line.starts_with(b"HEAD ")
}

/// The key of a request: its method and its target, without the headers.
fn key_of(request: &[u8]) -> Vec<u8> {
    let end = request
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(request.len(), |at| at + 4);
    let head = &request[..end.min(request.len())];
    let line = head.split(|b| *b == b'\r').next().unwrap_or(head);
    line.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_get_or_a_head_is_cached() {
        assert!(is_cacheable(b"GET /a HTTP/1.1\r\n"));
        assert!(is_cacheable(b"HEAD /a HTTP/1.1\r\n"));
        assert!(!is_cacheable(b"POST /a HTTP/1.1\r\n"));
        assert!(!is_cacheable(b"PUT /a HTTP/1.1\r\n"));
    }

    #[test]
    fn the_key_is_the_request_line() {
        assert_eq!(
            key_of(b"GET /a HTTP/1.1\r\nHost: x\r\n\r\n"),
            b"GET /a HTTP/1.1".to_vec()
        );
        assert_eq!(key_of(b"GET /"), b"GET /".to_vec());
    }
}
