//! The roads to the visitors of the root crate (T-062): a Cloudflare tunnel,
//! the node of a relay pair and a listener of the virtual network. Each road
//! serves the router of the service on a private unix socket in the run
//! directory (`podnet_serve::road::private_socket`) and points its carrier at
//! it. The trait, the table of services and the local road are in
//! `podnet-serve` (`podnet::road`).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::future::BoxFuture;

pub use podnet_serve::road::{
    guard, private_socket, Access, Local, Published, Road, Role, Rule, Services, Users,
};

use crate::config::Config;
use crate::ingress::{Ingress, Service};
use crate::vnet::addr::VirtAddr;
use crate::vnet::stack::NetStack;

/// One row of the table of the roads: a name, its spec forms and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoadRow {
    pub name: &'static str,
    pub forms: &'static [&'static str],
    pub about: &'static str,
}

const ROADS: [RoadRow; 4] = [
    RoadRow {
        name: "local",
        forms: &["local:HOST:PORT", "local:unix:/PATH"],
        about: "a listener of this host, TCP or a unix socket (mode 0600)",
    },
    RoadRow {
        name: "tunnel",
        forms: &["tunnel"],
        about: "a Cloudflare tunnel: a quick tunnel, or the named tunnel of the credentials",
    },
    RoadRow {
        name: "relay",
        forms: &["relay", "relay:PAIR"],
        about: "the node of a relay pair; the operator runs podnet relay connect PAIR",
    },
    RoadRow {
        name: "vnet",
        forms: &["vnet (in the process of a virtual network)"],
        about: "a listener of the virtual network of the process",
    },
];

/// The roads of this build, the second table of the binary beside
/// `crate::networks::networks` (T-062).
pub fn roads() -> &'static [RoadRow] {
    &ROADS
}

/// The road of `spec` (a form of [`roads`]). `config` is the tunnel
/// configuration of the command (credentials, protocol, legs). A vnet road
/// needs a stack of the process: make a [`Vnet`] in code.
pub fn parse(spec: &str, config: &Config) -> Result<Box<dyn Road>> {
    let spec = spec.trim();
    if let Some(listen) = spec.strip_prefix("local:") {
        anyhow::ensure!(
            !listen.is_empty(),
            "the road local: needs HOST:PORT or unix:/PATH"
        );
        return Ok(Box::new(Local {
            listen: listen.to_string(),
            replace: false,
        }));
    }
    match spec.split_once(':') {
        None if spec == "tunnel" => Ok(Box::new(Tunnel {
            config: config.clone(),
        })),
        None if spec == "relay" => Ok(Box::new(Relay::default())),
        Some(("relay", pair)) if !pair.is_empty() => Ok(Box::new(Relay {
            pair: Some(pair.to_string()),
        })),
        None if spec == "vnet" => anyhow::bail!(
            "the road vnet needs the virtual network of the process (podnet net); it has no spec of its own"
        ),
        _ => anyhow::bail!(
            "the road {spec:?} is not known: give local:HOST:PORT, local:unix:/PATH, tunnel, relay or relay:PAIR"
        ),
    }
}

/// The directory of the private sockets of the roads: the run directory.
fn socket_dir() -> Result<PathBuf> {
    crate::instances::run_dir()
}

/// The road of a Cloudflare tunnel: a quick tunnel, or the named tunnel of
/// the credentials of `config`. Its ingress is the private socket of the
/// service: the ingress rules of `config` are not used.
#[derive(Clone, Debug)]
pub struct Tunnel {
    pub config: Config,
}

impl Road for Tunnel {
    fn name(&self) -> &'static str {
        "tunnel"
    }

    fn publish(&self, service: axum::Router) -> BoxFuture<'_, Result<Published>> {
        Box::pin(async move {
            let (path, server) = private_socket(&socket_dir()?, "road-tunnel", service)?;
            let mut config = self.config.clone();
            config.ingress = Ingress::new().catch_all(Service::Unix(path));
            let handle = crate::tunnel::Tunnel::builder()
                .config(config)
                .build()?
                .start()
                .await?;
            let url = handle.url().map(str::to_string);
            let ready = serde_json::json!({ "road": "tunnel", "url": url, "leg": handle.leg() });
            let run = async move {
                let outcome = tokio::select! {
                    served = server => served,
                    reason = handle.stopped() => Err(anyhow::anyhow!("the tunnel stopped: {reason}")),
                };
                handle.shutdown().await;
                outcome
            };
            Ok(Published::new(url, ready, Box::pin(run)))
        })
    }
}

/// The road of the node of a relay pair: the operator reaches the service
/// with `podnet relay connect NAME`. With no pair name, the pair with the
/// most time left, and at its expiry the next stored pair.
#[derive(Clone, Debug, Default)]
pub struct Relay {
    pub pair: Option<String>,
}

impl Road for Relay {
    fn name(&self) -> &'static str {
        "relay"
    }

    fn publish(&self, service: axum::Router) -> BoxFuture<'_, Result<Published>> {
        use crate::relay_reverse as reverse;
        Box::pin(async move {
            let pairs = reverse::pairs_dir()?;
            let pair = reverse::load(&pairs, self.pair.as_deref())?;
            crate::report::hide(pair.node_token());
            crate::ws_relay::Relay::single(&pair.relay, None)?;
            let (path, server) = private_socket(&socket_dir()?, "road-relay", service)?;
            let ready =
                serde_json::json!({ "road": "relay", "pair": pair.name, "relay": pair.relay });
            let follow = self.pair.is_none();
            let run = async move {
                let target = reverse::Target::Unix(path);
                let follow = follow.then_some(pairs.as_path());
                tokio::select! {
                    node = reverse::run_node(follow, pair, &target) => node,
                    served = server => served,
                }
            };
            Ok(Published::new(None, ready, Box::pin(run)))
        })
    }
}

/// The road of a listener of the virtual network: each connection to
/// `addr` on `stack` goes to the service.
#[derive(Clone)]
pub struct Vnet {
    pub stack: Arc<NetStack>,
    pub addr: VirtAddr,
}

impl Road for Vnet {
    fn name(&self) -> &'static str {
        "vnet"
    }

    fn publish(&self, service: axum::Router) -> BoxFuture<'_, Result<Published>> {
        Box::pin(async move {
            let mut listener =
                self.stack.listen(self.addr).await.with_context(|| {
                    format!("a listener of the virtual network on {}", self.addr)
                })?;
            let (path, server) = private_socket(&socket_dir()?, "road-vnet", service)?;
            let url = format!("http://{}/", self.addr);
            let ready = serde_json::json!({ "road": "vnet", "url": url });
            let accept = async move {
                loop {
                    let mut visitor = listener.accept().await.context("the vnet listener")?;
                    let path = path.clone();
                    tokio::spawn(async move {
                        if let Ok(mut local) = tokio::net::UnixStream::connect(&path).await {
                            let _ = tokio::io::copy_bidirectional(&mut visitor, &mut local).await;
                        }
                    });
                }
            };
            let run = async move {
                tokio::select! {
                    served = server => served,
                    accepted = accept => accepted,
                }
            };
            Ok(Published::new(Some(url), ready, Box::pin(run)))
        })
    }
}

#[cfg(test)]
mod tests;
