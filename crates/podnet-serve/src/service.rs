//! Prebuilt service runtimes: built-in origins are axum routers created once
//! at startup; forward services keep their rule + options.

use std::sync::Arc;

use anyhow::Result;
use axum::Router;

use crate::gate::Gate;
use crate::ingress::{ForwardTarget, Ingress, OriginOptions, Service};
use crate::share::{ShareApp, ShareControl, ShareMode};
use podnet_core::metrics::Metrics;

pub use crate::files::{static_router, static_router_with};

/// A service prepared for dispatch.
pub enum Built {
    /// HTTP / HTTPS / unix / unix+tls origin, proxied raw.
    Forward(Service, OriginOptions),
    /// Raw TCP origin.
    Tcp(Service, OriginOptions),
    /// WebSocket endpoint bridging to a raw stream target.
    ForwardWs(ForwardTarget),
    HelloWorld,
    Status(u16),
    Static(Router),
    Metrics,
    Share(ShareApp),
    /// quickbridge proxy mode: gated forwarding to a local HTTP port.
    ShareProxy {
        target: String,
        gate: Gate,
        control: Arc<ShareControl>,
        /// Stop the session after the first request that the origin
        /// answered (`--stop-after`).
        stop_after: bool,
    },
}

pub struct ServiceRuntime {
    built: Vec<Built>,
    metrics: Metrics,
}

impl ServiceRuntime {
    /// Build every rule's runtime. Fails fast on a bad origin (missing
    /// directory, missing file) so the tunnel never starts half-configured.
    pub fn build(ingress: &Ingress, metrics: Metrics) -> Result<Self> {
        let mut built = Vec::with_capacity(ingress.rules.len());
        for rule in &ingress.rules {
            built.push(match &rule.service {
                Service::Http(_) | Service::Unix(_) | Service::UnixTls(_) => {
                    Built::Forward(rule.service.clone(), rule.origin.clone())
                }
                Service::Tcp(_) => Built::Tcp(rule.service.clone(), rule.origin.clone()),
                Service::Forward(target) => Built::ForwardWs(target.clone()),
                Service::HelloWorld => Built::HelloWorld,
                Service::Status(code) => Built::Status(*code),
                Service::Static { root, spa, tar } => {
                    Built::Static(static_router_with(root, *spa, *tar)?)
                }
                Service::Files(files) => Built::Static(crate::files::router(files)?),
                Service::Git(git) => Built::Static(crate::git::http::router(git.clone())?),
                Service::Metrics => Built::Metrics,
                Service::Share(config) => match config.mode {
                    ShareMode::Proxy => Built::ShareProxy {
                        target: config
                            .target
                            .clone()
                            .unwrap_or_else(|| "127.0.0.1:80".to_string()),
                        gate: config.gate.clone(),
                        control: Arc::new(ShareControl::default()),
                        stop_after: config.stop_after,
                    },
                    _ => Built::Share(ShareApp::new(config.clone())?),
                },
            });
        }
        Ok(Self { built, metrics })
    }

    pub fn get(&self, index: usize) -> Option<&Built> {
        self.built.get(index)
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn len(&self) -> usize {
        self.built.len()
    }

    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }

    /// Session controls for every share rule, so a host can watch for
    /// `stop_after` and idle timeouts.
    pub fn share_controls(&self) -> Vec<Arc<ShareControl>> {
        self.built
            .iter()
            .filter_map(|built| match built {
                Built::Share(app) => Some(app.control()),
                Built::ShareProxy { control, .. } => Some(control.clone()),
                _ => None,
            })
            .collect()
    }
}
