//! `podnet warp h3`: the HTTP/3 shape as a real endpoint
//! (PodUtils/podnet#486, QUIC-6). The listener takes the `h3` ALPN, opens a
//! control stream that stays open, answers a GET with the body of the shape,
//! and writes the qlog trace of the control stream and of each request.
//!
//! The endpoint has no identity and no WARP session: it serves the shape
//! itself. A stock HTTP/3 client reaches it on the loopback, and the test
//! `tests/quic_h3_stock.rs` runs one.

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use podnet::exit::{Fault, ResultExt};
use podnet::warp::h3::qlog::Trace;
use podnet::warp::h3::serve::{self, Listener};

/// Serve the shape on `listen` for `run_for` seconds (None: until the
/// process ends), with the trace at `qlog` when it is given.
pub(crate) async fn run(
    listen: SocketAddr,
    qlog: Option<PathBuf>,
    run_for: Option<u64>,
) -> Result<()> {
    let trace = match &qlog {
        Some(path) => Some(Arc::new(
            Trace::create(path)
                .map_err(|e| anyhow::anyhow!("the qlog trace {}: {e}", path.display()))
                .fault(Fault::Unavailable)?,
        )),
        None => None,
    };
    // The transport qlog of noq goes beside the HTTP/3 trace.
    let qlog_dir = qlog.as_deref().and_then(Path::parent);
    let listener = Arc::new(
        Listener::bind(
            listen,
            serve::self_signed("localhost").fault(Fault::Unavailable)?,
            podnet::warp::h3::shape().clone(),
            &pod_quic::Settings::default(),
            trace,
            qlog_dir,
        )
        .fault(Fault::Unavailable)?,
    );
    let addr = listener.local_addr().fault(Fault::Unavailable)?;
    println!("h3 listen {addr}");
    if let Some(path) = &qlog {
        println!("h3 qlog {}", path.display());
    }
    println!("{}", podnet::warp::h3::line());
    // A caller reads the address before the client starts: flush it now.
    let _ = std::io::stdout().flush();
    match run_for {
        Some(secs) => {
            let _ = tokio::time::timeout(Duration::from_secs(secs), listener.serve_forever()).await;
        }
        None => listener.serve_forever().await,
    }
    Ok(())
}
