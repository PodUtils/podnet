//! The answer of the file server to `GET DIR/?tar`: the directory as one
//! ustar stream (T-085).

use std::io::Write;
use std::path::PathBuf;

use axum::body::Body;
use axum::response::Response;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::tar::{append_tree, Writer};

/// The size of each chunk of the body.
const CHUNK: usize = 64 * 1024;

/// A `Write` that sends chunks to the body. A closed body (the client went
/// away) is an error, and the walk stops.
struct ChannelWriter {
    tx: mpsc::Sender<std::io::Result<Bytes>>,
    buf: Vec<u8>,
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.flush()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::take(&mut self.buf));
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}

/// The name of the top entry and of the file: the last segment of `dir`,
/// or `files` for a root with no name.
fn top_name(dir: &std::path::Path) -> String {
    dir.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty() && !n.starts_with('.'))
        .unwrap_or("files")
        .to_string()
}

/// Answer `dir` (canonical, in `root`) as a tar stream. HEAD gives the
/// headers only.
pub(crate) fn answer(root: PathBuf, dir: PathBuf, method: &http::Method) -> Response {
    let name = top_name(&dir);
    // A name for the header: no quote or control character.
    let file: String = name
        .chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let builder = Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/x-tar")
        .header(
            http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{file}.tar\""),
        )
        .header(http::header::CACHE_CONTROL, "no-store");
    if method == http::Method::HEAD {
        return builder.body(Body::empty()).unwrap_or_default();
    }
    let (tx, rx) = mpsc::channel::<std::io::Result<Bytes>>(4);
    let fail = tx.clone();
    tokio::task::spawn_blocking(move || {
        let mut w = Writer::new(ChannelWriter {
            tx,
            buf: Vec::with_capacity(CHUNK),
        });
        let result = append_tree(&mut w, &root, &dir, &name).and_then(|()| w.finish().map(drop));
        if let Err(err) = result {
            // An error item cuts the body: the client sees a failed
            // transfer, not a short archive.
            tracing::warn!(dir = %dir.display(), error = %format!("{err:#}"), "tar download stopped");
            let _ = fail.blocking_send(Err(std::io::Error::other("tar download stopped")));
        }
    });
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    builder.body(Body::from_stream(stream)).unwrap_or_default()
}
