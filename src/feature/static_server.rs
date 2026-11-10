//! Static serving glue: buffered and streamed responses and the static server.

use super::*;

// ---------------------------------------------------------------------------
// Static serving glue
// ---------------------------------------------------------------------------

/// An in-memory HTTP response, ready to be written to a socket.
///
/// This exists because the origin layer's [`crate::util::http::ResponseWriter`]
/// trait is streaming: it is built for writing straight to a socket as bytes
/// arrive. A tunnel that wants to inspect a response, record it, or decide
/// whether to forward it needs the whole thing first, and buffering is the
/// cheapest way to get that without a second response path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BufferedResponse {
    /// The status code the response was begun with.
    pub status: u16,
    /// Header name to value, in the order they were added.
    pub headers: Vec<(String, String)>,
    /// The body bytes.
    pub body: Vec<u8>,
}

impl BufferedResponse {
    /// A fresh buffer. The status starts at 0 so a response that was never
    /// begun is distinguishable from one that deliberately sent 0.
    pub fn new() -> Self {
        Self {
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Case-insensitive header lookup.
    ///
    /// Named `header_value` rather than `header` because
    /// [`ResponseWriter::header`] is the setter, and an inherent `header` would
    /// shadow it: every `buf.header("X", "y")` in this file would become a
    /// compile error the first time a getter and a setter share a name.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        let lower = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == lower)
            .map(|(_, v)| v.as_str())
    }

    /// Render as a complete HTTP/1.1 response.
    pub fn to_bytes(&self) -> Vec<u8> {
        crate::util::http::build_response(
            self.status,
            crate::origin::static_files::http_phrase(self.status),
            &self
                .headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect::<Vec<_>>(),
            Some(&self.body),
        )
    }

    /// Whether a status was ever set on this buffer.
    pub fn is_started(&self) -> bool {
        self.status != 0
    }
}

impl crate::util::http::ResponseWriter for BufferedResponse {
    fn status(&mut self, code: u16) -> &mut Self {
        self.status = code;
        self
    }

    fn header(&mut self, name: &str, value: &str) -> &mut Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    fn body(&mut self, body: &[u8]) -> io::Result<()> {
        self.body.extend_from_slice(body);
        Ok(())
    }

    /// Keep the body in the buffer, up to [`MAX_BUFFERED_BODY`] bytes. A
    /// larger body is an error of kind `FileTooLarge`, and nothing of it is
    /// read.
    fn body_reader(&mut self, len: u64, reader: &mut dyn io::Read) -> io::Result<()> {
        if len > MAX_BUFFERED_BODY {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                format!(
                    "a body of {len} bytes is over the buffer limit of {MAX_BUFFERED_BODY} bytes"
                ),
            ));
        }
        let start = self.body.len();
        io::Read::read_to_end(&mut io::Read::take(reader, len), &mut self.body)?;
        if (self.body.len() - start) as u64 != len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(())
    }
}

/// The largest body that a [`BufferedResponse`] keeps (issue 276). A larger
/// body given to [`ResponseWriter::body_reader`] is an error.
pub const MAX_BUFFERED_BODY: u64 = 64 * 1024 * 1024;

/// The body of a [`StaticResponse`]: bytes in memory (a short text), or an
/// open file that is read when the body is read (issue 296).
///
/// A file body keeps no copy of the file in memory: each
/// [`io::Read::read`] reads the file. A file of any size is sent with a
/// buffer of the size that the reader gives.
#[derive(Debug)]
pub struct StaticBody {
    inner: BodySource,
    len: u64,
}

#[derive(Debug)]
pub(super) enum BodySource {
    Bytes(io::Cursor<Vec<u8>>),
    File(io::Take<std::fs::File>),
}

impl StaticBody {
    /// A body of bytes in memory.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        Self {
            len: bytes.len() as u64,
            inner: BodySource::Bytes(io::Cursor::new(bytes)),
        }
    }

    /// A body of `len` bytes of `file`, from its current offset.
    pub fn from_file(file: std::fs::File, len: u64) -> Self {
        Self {
            inner: BodySource::File(io::Read::take(file, len)),
            len,
        }
    }

    /// The length of the whole body (the value of `Content-Length`), also
    /// after a part of it was read.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// True when the body has no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when the body is read from a file.
    pub fn is_file(&self) -> bool {
        matches!(self.inner, BodySource::File(_))
    }

    /// Add bytes to a body in memory. A file body is replaced.
    fn push(&mut self, bytes: &[u8]) {
        match &mut self.inner {
            BodySource::Bytes(cursor) => cursor.get_mut().extend_from_slice(bytes),
            BodySource::File(_) => {
                self.inner = BodySource::Bytes(io::Cursor::new(bytes.to_vec()));
                self.len = 0;
            }
        }
        self.len += bytes.len() as u64;
    }
}

impl Default for StaticBody {
    fn default() -> Self {
        Self::from_bytes(Vec::new())
    }
}

impl io::Read for StaticBody {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            BodySource::Bytes(cursor) => io::Read::read(cursor, buf),
            BodySource::File(file) => {
                let n = io::Read::read(file, buf)?;
                // The file became shorter after the open: the declared
                // length cannot be sent.
                if n == 0 && !buf.is_empty() && file.limit() > 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                Ok(n)
            }
        }
    }
}

/// A response of a [`StaticServer`]: the status, the headers and a body
/// reader (issue 296).
///
/// The body of a file is not in memory: read it from
/// [`StaticResponse::body`] (it implements [`io::Read`]), or write the
/// whole response with [`StaticResponse::write_to`].
#[derive(Debug, Default)]
pub struct StaticResponse {
    /// The status code.
    pub status: u16,
    /// Header name to value, in the order they were added. A
    /// `Content-Length` here is not sent: [`StaticResponse::head_bytes`]
    /// writes the length of the body.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: StaticBody,
}

impl StaticResponse {
    /// An empty response. The status starts at 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Case-insensitive header lookup.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The HTTP/1.1 head: the status line, the headers, `Content-Length`
    /// (the length of the body) and the empty line.
    pub fn head_bytes(&self) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\n",
            self.status,
            crate::origin::static_files::http_phrase(self.status)
        );
        for (k, v) in &self.headers {
            if !k.eq_ignore_ascii_case("content-length") {
                out.push_str(&format!("{k}: {v}\r\n"));
            }
        }
        out.push_str(&format!("Content-Length: {}\r\n\r\n", self.body.len()));
        out.into_bytes()
    }

    /// Write the head and the body to `out`, the body in chunks of at most
    /// [`crate::util::http::BODY_CHUNK`] bytes. Gives the number of body
    /// bytes. A body that ends before its length is an `UnexpectedEof`
    /// error.
    pub fn write_to<W: io::Write>(mut self, out: &mut W) -> io::Result<u64> {
        out.write_all(&self.head_bytes())?;
        let len = self.body.len();
        let mut buf = vec![0u8; crate::util::http::BODY_CHUNK.min(len as usize)];
        let mut sent = 0u64;
        while sent < len {
            let n = match io::Read::read(&mut self.body, &mut buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            out.write_all(&buf[..n])?;
            sent += n as u64;
        }
        out.flush()?;
        Ok(sent)
    }
}

impl crate::util::http::ResponseWriter for StaticResponse {
    fn status(&mut self, code: u16) -> &mut Self {
        self.status = code;
        self
    }

    fn header(&mut self, name: &str, value: &str) -> &mut Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    fn body(&mut self, body: &[u8]) -> io::Result<()> {
        self.body.push(body);
        Ok(())
    }

    /// Keep the open file: the body is read from it when the body is read.
    fn body_file(&mut self, len: u64, file: std::fs::File) -> io::Result<()> {
        self.body = StaticBody::from_file(file, len);
        Ok(())
    }

    /// A head with no body: the status and the headers.
    fn raw_head(&mut self, code: u16, _phrase: &str, extra: &[(String, String)]) -> io::Result<()> {
        self.status = code;
        self.headers.extend(extra.iter().cloned());
        self.body = StaticBody::default();
        Ok(())
    }
}

/// Serves a `static:` or `spa:` origin in process, with the headers a
/// tunnelled site needs.
///
/// [`crate::origin::StaticDir`] already resolves paths and refuses traversal.
/// What it does not do is answer the questions a file behind a public tunnel
/// raises: is the response cacheable, is the content being framed by someone
/// else, may a browser sniff the type.
///
/// The security headers are the reason this wrapper exists rather than calling
/// `StaticDir` directly. They are set on every response, including error ones,
/// because a 404 body is still an HTML document a browser will render.
#[derive(Debug, Clone)]
pub struct StaticServer {
    inner: crate::origin::StaticDir,
    cache_control: String,
}

impl StaticServer {
    /// Serve `dir`, with the index file and SPA behaviour of the origin.
    pub fn new(inner: crate::origin::StaticDir) -> Self {
        Self {
            inner,
            // Short by default: a tunnelled file may be replaced under the
            // same URL, and a visitor holding a stale copy is a bug report
            // that cannot be reproduced.
            cache_control: "no-cache".to_string(),
        }
    }

    /// Build from a parsed `static:`/`spa:` origin.
    pub fn from_origin(origin: &crate::origin::Origin) -> Result<Self, FeatureError> {
        match origin {
            crate::origin::Origin::Static { dir, spa, index } => {
                Ok(Self::new(crate::origin::StaticDir {
                    dir: dir.clone(),
                    spa: *spa,
                    index: index.clone(),
                }))
            }
            other => Err(FeatureError::Invalid(format!(
                "{} is not a static origin; expected static: or spa:",
                other.kind()
            ))),
        }
    }

    /// Override the `Cache-Control` sent with file responses.
    pub fn cache_control(mut self, value: &str) -> Self {
        self.cache_control = value.to_string();
        self
    }

    /// The headers added to every response.
    ///
    /// `X-Content-Type-Options: nosniff` matters most: without it a browser
    /// may re-interpret a served file as HTML, which turns a user-supplied
    /// upload into script execution on the tunnel's own origin.
    fn security_headers() -> [(&'static str, &'static str); 5] {
        [
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "SAMEORIGIN"),
            ("Referrer-Policy", "no-referrer"),
            ("Cross-Origin-Opener-Policy", "same-origin"),
            (
                "Content-Security-Policy",
                "default-src 'self'; frame-ancestors 'self'",
            ),
        ]
    }

    /// Answer one request.
    ///
    /// The body of a file is not read here: the response keeps the open
    /// file, and [`StaticResponse::body`] reads it (issue 296). A file of
    /// any size is served with a bounded memory.
    pub fn respond(&self, req: &RequestHead) -> io::Result<StaticResponse> {
        let mut out = StaticResponse::new();
        for (name, value) in Self::security_headers() {
            out.header(name, value);
        }

        // A tunnelled site is reached over a connection this process owns, so
        // the only methods worth serving are the ones a browser sends for a
        // document. Rejecting the rest here is cheaper and clearer than letting
        // a file server answer them.
        match req.method.as_str() {
            "GET" | "HEAD" => {}
            other => {
                out.status(405);
                out.header("Content-Type", "text/plain; charset=utf-8");
                out.header("Allow", "GET, HEAD");
                out.body = StaticBody::from_bytes(format!("405 method not allowed: {other}\n"));
                return Ok(out);
            }
        }

        if self.inner.spa {
            out.header("Cache-Control", "no-store");
        } else {
            out.header("Cache-Control", &self.cache_control.clone());
        }

        // `StaticDir::respond` writes through the ResponseWriter trait, so it
        // fills the same response. Its own status and headers take
        // precedence because it knows which file it found; the headers added
        // above stay. A file is given with `body_file`: the response keeps
        // the open handle.
        self.inner.respond(req, &mut out)?;
        Ok(out)
    }
}
