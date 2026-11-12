//! `Chain`: an outer network carries an inner one (T-058). MASQUE over the
//! relay over a proxy is the chain `[proxy, relay, warp]`: each network
//! comes up with the way of the network below it ([`Ctx::below`]), and
//! the way of the chain is the way of the top network.

use std::sync::Arc;

use super::{Addressing, BoxFuture, Caps, Class, Ctx, Failure, Link, Network, Stage};

/// Networks in layers, the lowest first.
pub struct Chain {
    layers: Vec<Arc<dyn Network>>,
}

impl Chain {
    /// The layers, the lowest first. A chain needs one layer or more.
    pub fn new(layers: Vec<Arc<dyn Network>>) -> Result<Chain, Failure> {
        if layers.is_empty() {
            return Err(Failure::new(
                Stage::Connect,
                Class::Usage,
                "a chain needs one network or more",
            ));
        }
        Ok(Chain { layers })
    }

    /// The schemes of the layers, the top first, with ` over ` between
    /// them (`warp over relay over proxy`).
    pub fn label(&self) -> String {
        let names: Vec<&str> = self.layers.iter().rev().map(|n| n.scheme()).collect();
        names.join(" over ")
    }
}

impl Network for Chain {
    fn scheme(&self) -> &'static str {
        "chain"
    }

    /// What the top network gives.
    fn caps(&self) -> Caps {
        self.layers.last().map_or(Caps::NONE, |n| n.caps())
    }

    /// Bring up each layer, the lowest first, each over the way of the
    /// layer below (the lowest over `ctx.below`). A layer below another
    /// must give a way that dials a destination. On an error, each layer
    /// that came up closes, the highest first.
    fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move {
            let mut links: Vec<Link> = Vec::new();
            let mut below = ctx.below.clone();
            let count = self.layers.len();
            for (i, layer) in self.layers.iter().enumerate() {
                let up = layer
                    .up(&Ctx {
                        below: below.clone(),
                        state_dir: ctx.state_dir.clone(),
                    })
                    .await;
                let step = up.map_err(|e| {
                    e.context(format!(
                        "the layer {} of {count} ({}) of the chain {}",
                        i + 1,
                        layer.scheme(),
                        self.label()
                    ))
                });
                let link = match step {
                    Ok(link) => link,
                    Err(e) => {
                        close_all(links).await;
                        return Err(e);
                    }
                };
                if let Err(e) = carries(&link, i + 1 < count, layer.scheme()) {
                    links.push(link);
                    close_all(links).await;
                    return Err(e);
                }
                if i + 1 < count {
                    below = link.way.as_ref().map(|w| w.dial.clone());
                }
                links.push(link);
            }
            let top = links.last().expect("a chain has a layer");
            let way = top.way.clone();
            let exports = links.iter().flat_map(|l| l.exports.clone()).collect();
            let status = serde_json::Value::Array(
                self.layers
                    .iter()
                    .zip(&links)
                    .map(|(n, l)| serde_json::json!({"scheme": n.scheme(), "status": l.status}))
                    .collect(),
            );
            let mut link = Link::new(way)
                .with_status(status)
                .on_close(Box::new(move || Box::pin(close_all(links))));
            link.exports = exports;
            Ok(link)
        })
    }

    /// The rows of each layer, the lowest first.
    fn checks(&self) -> Vec<super::Check> {
        self.layers.iter().flat_map(|n| n.checks()).collect()
    }
}

/// Check that `link` can carry the layer above it, when it has one: it
/// gives a way, and the way dials a destination.
fn carries(link: &Link, above: bool, scheme: &str) -> anyhow::Result<()> {
    if !above {
        return Ok(());
    }
    match &link.way {
        None => Err(anyhow::anyhow!(
            "{scheme} gives no way out, so it cannot carry the network above it in the chain"
        )),
        Some(way) => Ok(Addressing::Destination.admit(std::slice::from_ref(way))?),
    }
}

/// Close each link, the highest first.
async fn close_all(links: Vec<Link>) {
    for link in links.into_iter().rev() {
        link.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::fakes::{carrier as carried_by, log, seen, soon, Fake, Log};
    use super::super::{Dial, Target, Way};
    use super::*;

    /// A network whose way is the way below it with its name in front of
    /// the label, or a fake way when it is the lowest.
    struct Layer {
        scheme: &'static str,
        fails: bool,
        no_way: bool,
        peer: bool,
        log: Log,
    }

    impl Layer {
        fn new(scheme: &'static str, log: &Log) -> Layer {
            Layer {
                scheme,
                fails: false,
                no_way: false,
                peer: false,
                log: log.clone(),
            }
        }
    }

    struct Over {
        name: &'static str,
        below: Arc<dyn Dial>,
    }

    impl Dial for Over {
        fn label(&self) -> String {
            format!("{} over {}", self.name, self.below.label())
        }

        fn dial<'a>(
            &'a self,
            target: &'a Target,
            deadline: tokio::time::Instant,
        ) -> BoxFuture<'a, Result<super::super::Connected, Failure>> {
            self.below.dial(target, deadline)
        }
    }

    impl Network for Layer {
        fn scheme(&self) -> &'static str {
            self.scheme
        }

        fn caps(&self) -> Caps {
            Caps::EGRESS_TCP
        }

        fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
            Box::pin(async move {
                let below = ctx.below.as_ref().map_or("none".to_string(), |d| d.label());
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("up {} over {below}", self.scheme));
                if self.fails {
                    anyhow::bail!("{} failed", self.scheme);
                }
                let dial: Arc<dyn Dial> = match &ctx.below {
                    Some(below) => Arc::new(Over {
                        name: self.scheme,
                        below: below.clone(),
                    }),
                    None => Arc::new(Fake::up(self.scheme, &self.log)),
                };
                let way = match (self.no_way, self.peer) {
                    (true, _) => None,
                    (false, true) => Some(Way::peer(dial)),
                    (false, false) => Some(Way::destination(dial)),
                };
                let log = self.log.clone();
                let scheme = self.scheme;
                Ok(Link::new(way)
                    .with_status(serde_json::json!(scheme))
                    .on_close(Box::new(move || {
                        Box::pin(async move { log.lock().unwrap().push(format!("close {scheme}")) })
                    })))
            })
        }
    }

    fn chain(layers: Vec<Layer>) -> Chain {
        Chain::new(
            layers
                .into_iter()
                .map(|l| Arc::new(l) as Arc<dyn Network>)
                .collect(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn each_layer_comes_up_over_the_way_below_it() {
        let log = log();
        let chain = chain(vec![
            Layer::new("proxy", &log),
            Layer::new("relay", &log),
            Layer::new("warp", &log),
        ]);
        assert_eq!(chain.label(), "warp over relay over proxy");
        let link = chain.up(&Ctx::default()).await.unwrap();
        let way = link.way.clone().expect("a way");
        assert_eq!(way.label(), "warp over relay over proxy");
        let conn = way
            .dial
            .dial(&Target::new("example.test", 443), soon())
            .await
            .unwrap();
        assert_eq!(
            carried_by(conn).await,
            "proxy",
            "the lowest layer carries the bytes"
        );
        assert_eq!(
            link.status,
            serde_json::json!([
                {"scheme": "proxy", "status": "proxy"},
                {"scheme": "relay", "status": "relay"},
                {"scheme": "warp", "status": "warp"},
            ])
        );
        link.close().await;
        assert_eq!(
            seen(&log),
            [
                "up proxy over none",
                "up relay over proxy",
                "up warp over relay over proxy",
                "proxy",
                "close warp",
                "close relay",
                "close proxy",
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_layer_closes_the_layers_below_it() {
        let log = log();
        let mut warp = Layer::new("warp", &log);
        warp.fails = true;
        let chain = chain(vec![
            Layer::new("proxy", &log),
            Layer::new("relay", &log),
            warp,
        ]);
        let err = chain.up(&Ctx::default()).await.expect_err("warp fails");
        assert_eq!(
            format!("{err:#}"),
            "the layer 3 of 3 (warp) of the chain warp over relay over proxy: warp failed"
        );
        assert_eq!(
            seen(&log)[3..],
            ["close relay".to_string(), "close proxy".to_string()]
        );
    }

    #[tokio::test]
    async fn a_lower_layer_must_give_a_way_that_dials_a_destination() {
        let log = log();
        let mut peer = Layer::new("wormhole", &log);
        peer.peer = true;
        let err = chain(vec![peer, Layer::new("warp", &log)])
            .up(&Ctx::default())
            .await
            .expect_err("a peer below");
        assert!(
            format!("{err:#}").starts_with("wormhole meets a peer"),
            "{err:#}"
        );
        let mut silent = Layer::new("export", &log);
        silent.no_way = true;
        let err = chain(vec![silent, Layer::new("warp", &log)])
            .up(&Ctx::default())
            .await
            .expect_err("no way below");
        assert_eq!(
            format!("{err:#}"),
            "export gives no way out, so it cannot carry the network above it in the chain"
        );
        assert!(seen(&log).contains(&"close wormhole".to_string()));
        assert!(seen(&log).contains(&"close export".to_string()));
    }

    #[tokio::test]
    async fn the_lowest_layer_goes_over_the_way_of_the_context() {
        let log = log();
        let below: Arc<dyn Dial> = Arc::new(Fake::up("socks5h", &log));
        let chain = chain(vec![Layer::new("relay", &log)]);
        let link = chain
            .up(&Ctx {
                below: Some(below),
                ..Ctx::default()
            })
            .await
            .unwrap();
        assert_eq!(link.way.as_ref().unwrap().label(), "relay over socks5h");
        assert_eq!(chain.caps(), Caps::EGRESS_TCP);
        assert!(Chain::new(Vec::new()).is_err());
    }
}
