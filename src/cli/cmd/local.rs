//! A local listener of a router: `serve --local` and `share --local`.

use anyhow::Result;

use podnet::exit::Fault;

use crate::cli::cmd::connect::{unix_listener, SocketFile};

/// A router on a local listener.
pub(crate) struct Local {
    /// `http://HOST:PORT/` or `unix:PATH`.
    pub(crate) url: String,
    /// `tcp://HOST:PORT` or `unix:PATH`.
    pub(crate) listen: String,
    pub(crate) server: tokio::task::JoinHandle<std::io::Result<()>>,
    /// The socket file, removed on drop.
    _file: Option<SocketFile>,
}

impl Local {
    /// The server task and the socket file (removed on drop).
    pub(crate) fn split(
        self,
    ) -> (
        tokio::task::JoinHandle<std::io::Result<()>>,
        Option<SocketFile>,
    ) {
        (self.server, self._file)
    }
}

/// Serve `router` on the local listener `local` (issue 307): HOST:PORT,
/// tcp://HOST:PORT or unix:PATH.
pub(crate) async fn bind_local(local: &str, replace: bool, router: axum::Router) -> Result<Local> {
    let usage = |msg: String| podnet::exit::fault(Fault::Usage, anyhow::anyhow!(msg));
    if local.starts_with("unix:") {
        let path = match podnet::vnet::proxy::ProxyListen::parse(local) {
            Ok(podnet::vnet::proxy::ProxyListen::Unix(path)) => path,
            Ok(_) => unreachable!("a unix: address is a unix socket"),
            Err(err) => return Err(usage(format!("--local {local}: {err:#}"))),
        };
        let (listener, file) = unix_listener(&path, replace)?;
        let name = format!("unix:{}", path.display());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        return Ok(Local {
            url: name.clone(),
            listen: name,
            server,
            _file: Some(file),
        });
    }
    let addr = local.strip_prefix("tcp://").unwrap_or(local);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(err) if !addr.contains(':') || err.kind() == std::io::ErrorKind::InvalidInput => {
            return Err(usage(format!(
                "--local {local}: give HOST:PORT (for example 127.0.0.1:8080) or unix:PATH ({err})"
            )));
        }
        // An address in use, or an address of no interface: exit 69.
        Err(err) => {
            return Err(podnet::exit::fault(
                Fault::Unavailable,
                anyhow::anyhow!("--local {local}: binding {addr}: {err}"),
            ));
        }
    };
    let bound = listener.local_addr()?;
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    Ok(Local {
        url: format!("http://{bound}/"),
        listen: format!("tcp://{bound}"),
        server,
        _file: None,
    })
}
