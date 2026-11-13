//! Local forwards into WARP (issue 143): `podnet warp forward -L
//! LISTEN=HOST:PORT`: `pod-warp`'s forward, on podnet's listeners. Each
//! connection to LISTEN goes through the WARP session to HOST:PORT. A name
//! is resolved inside the tunnel.

use std::sync::Arc;

use anyhow::Result;
use podnet_netns::listen::ProxyListen;

use super::ip::WarpStack;
use super::proxy::Bound;
use super::Warped as _;

/// One rule of `-L`: podnet's listener and the rule of `pod-warp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub listen: ProxyListen,
    pub rule: pod_warp::forward::Rule,
}

impl Rule {
    /// `LISTEN=HOST:PORT`. LISTEN is `PORT` (127.0.0.1), `ADDR:PORT`,
    /// `tcp://ADDR:PORT` or `unix:/PATH`. HOST:PORT can start with
    /// `tunnel:` or `tcp://`; an IPv6 address is in brackets.
    pub fn parse(spec: &str) -> Result<Self> {
        let rule = pod_warp::forward::Rule::parse(spec).warped()?;
        let listen = match &rule.listen {
            pod_warp::forward::Listen::Tcp(addr) => ProxyListen::Tcp(*addr),
            pod_warp::forward::Listen::Unix(path) => ProxyListen::Unix(path.clone()),
        };
        Ok(Rule { listen, rule })
    }

    /// The target as text: `HOST:PORT`, `[IPV6]:PORT`.
    pub fn target(&self) -> String {
        self.rule.target.to_string()
    }
}

/// A running forward: its listener and its accept task.
pub struct Forward {
    pub listen: ProxyListen,
    pub rule: Rule,
    task: tokio::task::JoinHandle<()>,
}

impl Forward {
    /// Stop the forward.
    pub fn abort(&self) {
        self.task.abort();
    }
}

/// Accept connections on `bound` and send each one through the WARP session
/// of `live` to the target of `rule`. At most `max` connections at the same
/// time (0: no limit); a connection over it is closed at once.
pub fn run(live: Arc<super::Live<WarpStack>>, bound: Bound, rule: Rule, max: usize) -> Forward {
    let listen = bound.listen().clone();
    let mut forward = pod_warp::forward::Forward::new(
        rule.rule.clone(),
        live as Arc<dyn pod_warp::proxy::Tunnel>,
        super::proxy::names(),
        max,
    );
    forward.report = super::report();
    let forward = Arc::new(forward);
    let task = match bound {
        Bound::Tcp(listener, _) => forward.run_tcp(listener),
        Bound::Unix(listener, _) => forward.run_unix(listener),
    };
    tracing::info!(%listen, target = %rule.target(), "WARP forward is listening");
    Forward { listen, rule, task }
}
