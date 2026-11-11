//! The roads to the visitors (T-062). A service is an HTTP router; a road
//! publishes it: a local listener, a Cloudflare tunnel, the node of a relay
//! pair or a listener of the virtual network. This crate has the trait, the
//! table of services and the local road. The roads that need the tunnel or
//! the relay are in the root crate (`podnet::roads`): each serves the router
//! on a private unix socket ([`private_socket`]) and points its carrier at
//! it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use futures::future::BoxFuture;
use tower::ServiceExt as _;

pub use crate::files::{guard, Access, Role, Users};
use crate::ingress::PathPattern;

/// A service that a road published.
pub struct Published {
    /// The URL for a visitor, when the road has one (`http://HOST:PORT/`,
    /// `unix:PATH`, `https://NAME.trycloudflare.com`).
    pub url: Option<String>,
    /// The part of the road in the ready document.
    pub ready: serde_json::Value,
    run: BoxFuture<'static, Result<()>>,
}

impl Published {
    /// A road that runs `run` until it ends; a drop of the value ends it.
    pub fn new(
        url: Option<String>,
        ready: serde_json::Value,
        run: BoxFuture<'static, Result<()>>,
    ) -> Published {
        Published { url, ready, run }
    }

    /// Serve until the road ends (an error of its listener or its carrier).
    pub async fn run(self) -> Result<()> {
        self.run.await
    }
}

impl std::fmt::Debug for Published {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Published")
            .field("url", &self.url)
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

/// A road to the visitors.
pub trait Road: Send + Sync {
    /// `local`, `tunnel`, `relay` or `vnet`.
    fn name(&self) -> &'static str;

    /// Make `service` reachable on the road. The road serves when the
    /// result runs ([`Published::run`]).
    fn publish(&self, service: axum::Router) -> BoxFuture<'_, Result<Published>>;
}

/// One rule of a table of services: a host (any host with `None`), a path
/// pattern and the router of the service.
#[derive(Clone)]
pub struct Rule {
    pub host: Option<String>,
    pub path: PathPattern,
    pub router: axum::Router,
}

/// The services of one road, in order: the first rule whose host and path
/// match a request takes it; a request that no rule matches gets 404.
#[derive(Clone, Default)]
pub struct Services {
    rules: Vec<Rule>,
}

impl Services {
    pub fn new() -> Services {
        Services::default()
    }

    /// Add a rule after the rules that exist.
    pub fn rule(mut self, host: Option<&str>, path: PathPattern, router: axum::Router) -> Services {
        self.rules.push(Rule {
            host: host.map(|h| h.to_ascii_lowercase()),
            path,
            router,
        });
        self
    }

    /// The index of the rule for `host` (with no port) and `path`.
    pub fn find(&self, host: &str, path: &str) -> Option<usize> {
        let host = host.to_ascii_lowercase();
        self.rules
            .iter()
            .position(|r| r.host.as_deref().is_none_or(|h| h == host) && r.path.is_match(path))
    }

    /// One router that sends each request to the router of its rule.
    pub fn router(self) -> axum::Router {
        let services = Arc::new(self);
        axum::Router::new().fallback(move |request: Request<Body>| {
            let services = services.clone();
            async move {
                let host = request
                    .headers()
                    .get(axum::http::header::HOST)
                    .and_then(|h| h.to_str().ok())
                    .map(|h| match h.rsplit_once(':') {
                        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
                        _ => h,
                    })
                    .unwrap_or("")
                    .to_string();
                match services.find(&host, request.uri().path()) {
                    Some(i) => {
                        let router = services.rules[i].router.clone();
                        match router.oneshot(request).await {
                            Ok(response) => response,
                            Err(never) => match never {},
                        }
                    }
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }
        })
    }
}

/// The local road: a TCP listener (`HOST:PORT`, `tcp://HOST:PORT`) or a
/// unix socket (`unix:PATH`, mode 0600). A unix socket that answers is
/// refused unless `replace`; a stale socket is removed
/// (`podnet_core::net::claim_socket_path`).
#[derive(Clone, Debug)]
pub struct Local {
    pub listen: String,
    pub replace: bool,
}

impl Road for Local {
    fn name(&self) -> &'static str {
        "local"
    }

    fn publish(&self, service: axum::Router) -> BoxFuture<'_, Result<Published>> {
        Box::pin(async move {
            if let Some(path) = self.listen.strip_prefix("unix:") {
                let path = PathBuf::from(path);
                let listener = bind_unix(&path, self.replace)?;
                let url = format!("unix:{}", path.display());
                let ready = serde_json::json!({ "road": "local", "url": url });
                let run = async move {
                    let _file = Removed(path);
                    axum::serve(listener, service)
                        .await
                        .context("the local listener")
                };
                return Ok(Published::new(Some(url), ready, Box::pin(run)));
            }
            let addr = self.listen.strip_prefix("tcp://").unwrap_or(&self.listen);
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("--local {addr}"))
                .map_err(podnet_core::net::bind_fault)?;
            let bound = listener.local_addr()?;
            let url = format!("http://{bound}/");
            let ready = serde_json::json!({ "road": "local", "url": url });
            let run = async move {
                axum::serve(listener, service)
                    .await
                    .context("the local listener")
            };
            Ok(Published::new(Some(url), ready, Box::pin(run)))
        })
    }
}

/// A unix listener at `path`, mode 0600.
fn bind_unix(path: &Path, replace: bool) -> Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt as _;
    podnet_core::net::claim_socket_path(path, replace)?;
    let listener = tokio::net::UnixListener::bind(path)
        .with_context(|| format!("unix:{}", path.display()))
        .map_err(podnet_core::net::bind_fault)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// A socket file that is removed at the drop.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Serve `service` on a new private unix socket in `dir` (mode 0600, a
/// random name with `prefix`). The path, and the server that removes the
/// socket at its end.
pub fn private_socket(
    dir: &Path,
    prefix: &str,
    service: axum::Router,
) -> Result<(PathBuf, BoxFuture<'static, Result<()>>)> {
    let path = dir.join(format!(
        "{prefix}-{}.sock",
        &podnet_core::util::random_token()[..16]
    ));
    let listener = bind_unix(&path, false)?;
    let file = Removed(path.clone());
    let run = async move {
        let _file = file;
        axum::serve(listener, service)
            .await
            .context("the private socket of the road")
    };
    Ok((path, Box::pin(run)))
}

#[cfg(test)]
mod tests;
