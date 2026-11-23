//! A qlog trace of the HTTP/3 events of [`super::serve`]
//! (PodUtils/podnet#486, QUIC-7).
//!
//! The qlog of the QUIC transport comes from noq (`qlog_from_path`). That
//! trace shows the QUIC frames and the stream ids, but it does not name the
//! HTTP/3 control stream: a stream frame of a QUIC trace is a stream frame.
//! This trace adds the HTTP/3 events, in the JSON-SEQ qlog format, and the
//! two traces share the stream id of the control stream.
//!
//! One writer appends one JSON object per line, so a reader can read the
//! file while the connection runs. The format is the qlog 0.3 draft of
//! JSON-SEQ: a header event, then `time`, `name`, `data`.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

/// The header event of a JSON-SEQ trace (qlog 0.3, section 3.2).
const HEADER: &str = concat!(
    r#"{"qlog_version":"0.3","qlog_format":"JSON-SEQ","title":"podnet h3 shape","#,
    r#""trace":{"vantage_point":{"name":"podnet","type":"server"},"#,
    r#""common_fields":{"protocol_type":["QUIC","HTTP/3"]}}}"#,
    "\n",
);

/// A trace file of the HTTP/3 events.
pub struct Trace {
    start: Instant,
    file: Mutex<BufWriter<File>>,
}

impl Trace {
    /// Create `path` and write its header. Each [`event`](Trace::event)
    /// appends one line and flushes it.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Trace> {
        let mut file = BufWriter::new(File::create(path)?);
        file.write_all(HEADER.as_bytes())?;
        file.flush()?;
        Ok(Trace {
            start: Instant::now(),
            file: Mutex::new(file),
        })
    }

    /// Append one event, with its time in milliseconds since the trace
    /// started.
    pub fn event(&self, name: &str, data: serde_json::Value) {
        let time = self.start.elapsed().as_secs_f64() * 1000.0;
        let line = serde_json::json!({ "time": time, "name": name, "data": data });
        let mut file = match self.file.lock() {
            Ok(file) => file,
            Err(poison) => poison.into_inner(),
        };
        if writeln!(file, "{line}").is_err() {
            return;
        }
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trace has its header, then each event with its name and data, one
    /// line each, and a reader reads them while the writer is open.
    #[test]
    fn the_trace_keeps_each_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h3.qlog");
        let trace = Trace::create(&path).unwrap();
        trace.event(
            "http3:control_stream",
            serde_json::json!({ "stream_id": 3 }),
        );
        trace.event("http3:request", serde_json::json!({ "status": 200 }));
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "the header and two events");
        let header: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(header["qlog_format"], "JSON-SEQ");
        let control: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(control["name"], "http3:control_stream");
        assert_eq!(control["data"]["stream_id"], 3);
        let request: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(request["data"]["status"], 200);
        assert!(request["time"].as_f64().unwrap() >= 0.0);
    }
}
