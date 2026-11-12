//! The HTTP/2 edge leg through a WebSocket-to-TCP relay
//! (`--transport relay`), and the relay client of the process, over
//! `pod-relay` of PodUtils/podcrates.
//!
//! Some hosts reach the internet only through an HTTPS proxy on port 443,
//! and the proxy does not carry port 7844 of the edge. A relay on port 443
//! (a Cloudflare Worker, for example `tcp.ssh.relay.ajam.dev`) takes a
//! WebSocket at `/connect/{ip}/{port}` and opens TCP to that address. podnet
//! sends the TLS of the HTTP/2 leg (SNI `h2.cftunnel.com`, the edge roots)
//! in the WebSocket, so the relay sees only TLS bytes. The tunnel token does
//! not go to the relay.
//!
//! The token of the relay comes from [`TOKEN_VARIABLE`], or `pod-relay`
//! mints one with `POST /v1/mint`. It goes only in the `X-Relay-Token`
//! header (not in the URL), and no log or error has it. `pod-relay` keeps a
//! minted token and the pool of the relay in the state directory (none with
//! `PODNET_STATE_DIR=none`), tries the hosts of a list in turn, and mints
//! again once for a refused token.
//!
//! Limits of the relay that apply to an edge connection:
//!
//! * TCP only: no QUIC leg.
//! * IPv4 only: the relay has no IPv6 egress.
//! * The relay can end a session after 180 s with no payload (a limit of
//!   the relay, not of podnet). The PING of the HTTP/2
//!   leg (each 30 s) is payload.
//! * A session ends at 64 MiB (both directions) or after 12 h. podnet
//!   connects a replacement before that: at `SESSION_BYTES` or
//!   `SESSION_AGE` the old connection takes no new stream, and its open
//!   streams go on until they end.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{bail, Result};
use pod_relay::cache::Cache;
use pod_relay::relay::RelayList;
use pod_relay::{Context, Names, Settings};

use crate::net::BoxedIo;

pub use crate::services::default_relay;
pub use pod_relay::RelayBudget;
/// The environment variable with the relay URL.
pub const RELAY_VARIABLE: &str = "PODNET_RELAY";
/// The environment variable with a token for the relay.
pub const TOKEN_VARIABLE: &str = "PODNET_RELAY_TOKEN";
/// The bytes (both directions, TLS included) after which podnet replaces an
/// edge connection through the relay. The relay ends a session at 64 MiB.
pub const SESSION_BYTES: u64 = pod_relay::relay::SESSION_BYTE_BUDGET;
/// The age after which podnet replaces an edge connection through the relay.
/// The relay ends a session after 12 h.
pub const SESSION_AGE: Duration = pod_relay::relay::SESSION_TIME_BUDGET;

static RELAY: OnceLock<Relay> = OnceLock::new();

/// Use `relay` for each HTTP/2 edge connection of this process.
pub fn set(relay: Relay) {
    let _ = RELAY.set(relay);
}

/// The relay of this process, when `--transport relay` is in use.
pub fn active() -> Option<&'static Relay> {
    RELAY.get()
}

/// The relay client of this process: podnet's way out (the egress layer and
/// its TLS context), the state directory as the cache of the tokens and of
/// the pool, and podnet's names. One for each process: the sessions share
/// its tokens, its gates and its record of failures.
pub fn context() -> &'static Context {
    static CONTEXT: OnceLock<Context> = OnceLock::new();
    CONTEXT.get_or_init(|| {
        let way = pod_ws::Way::new(
            crate::trust::context().clone(),
            crate::egress::egress(None),
            format!("podnet/{}", env!("CARGO_PKG_VERSION")),
        );
        let cache = match crate::state::state_dir() {
            Ok(Some(dir)) => Cache::in_dirs(vec![dir]),
            _ => Cache::none(),
        };
        Context::new(way, cache)
            .with_settings(settings(None))
            .with_proxy_of(Arc::new(|host| crate::egress::proxy_for(host, 443)))
    })
}

/// podnet's settings of the relay client: the token that the user gave.
fn settings(token: Option<String>) -> Settings {
    let names = Names {
        app: "podnet".into(),
        token: TOKEN_VARIABLE.into(),
        cache_dir: "PODNET_STATE_DIR".into(),
        ..Names::default()
    };
    Settings {
        token,
        names,
        ..Settings::default()
    }
}

/// Count the bytes of `io` against the budget of one session through the
/// relay in use: spent at its bytes or at its age. A MASQUE session through
/// the relay is replaced when it is spent (the owner ruling of 2026-11-04).
pub fn meter(io: BoxedIo) -> (BoxedIo, RelayBudget) {
    let (io, budget) = RelayBudget::meter_for(context(), io);
    (Box::new(io), budget)
}

/// A WebSocket-to-TCP relay: the hosts of a list, or the default host and
/// the alternates of its pool. A connect tries the first host first, and
/// the next one when it does not open in time or fails (`pod-relay`).
#[derive(Clone)]
pub struct Relay {
    list: RelayList,
    /// Plain `http://` to a relay on the loopback (the tests).
    plain: bool,
    ctx: Context,
}

impl std::fmt::Debug for Relay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relay")
            .field("url", &self.url())
            .field("token", &"<redacted>")
            .finish()
    }
}

/// The hosts of `url` (a comma list), and whether they are plain.
fn parse(url: &str) -> Result<(Vec<pod_relay::Relay>, bool)> {
    let items: Vec<&str> = url
        .split(',')
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .collect();
    let Some(first) = items.first() else {
        bail!("the relay URL is empty; for example {}", default_relay());
    };
    let plain_of = |item: &str| item.starts_with("http://") || item.starts_with("ws://");
    let plain = plain_of(first);
    if items.iter().any(|item| plain_of(item) != plain) {
        bail!("the relay list {url:?} has http:// and https:// hosts: give one scheme");
    }
    let hosts = pod_relay::relay::parse_relay_list(url).map_err(|e| anyhow::anyhow!(e))?;
    Ok((hosts, plain))
}

/// The URL of `relay`: an IPv6 address in brackets, and the port when it is
/// not the default of the scheme.
fn url_of(relay: &pod_relay::Relay, plain: bool) -> String {
    let (scheme, default) = if plain { ("http", 80) } else { ("https", 443) };
    let host = pod_relay::relay::url_host(&relay.host);
    if relay.port == default {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{}", relay.port)
    }
}

/// The context of the process, with `token` when one is given.
fn context_with(token: Option<String>) -> Context {
    match token.filter(|t| !t.trim().is_empty()) {
        Some(token) => context().clone().with_settings(settings(Some(token))),
        None => context().clone(),
    }
}

impl Relay {
    /// A relay at `url` (`https://host[:port]`, `wss://` is the same; a
    /// comma list of hosts). Plain `http://` or `ws://` is taken only for a
    /// loopback host (tests). `token` is used as it is; with no token,
    /// podnet mints one. The default relay takes the alternates of its pool.
    pub fn new(url: &str, token: Option<String>) -> Result<Self> {
        let (hosts, plain) = parse(url)?;
        let ctx = context_with(token);
        let default = pod_relay::Relay {
            host: pod_relay::default_relay_host().to_string(),
            port: 443,
        };
        let list = if !plain && hosts == [default] {
            let pool = pod_relay::pool::alternates(&ctx.cache, pod_relay::default_relay_host());
            pod_relay::relay::select_relays(None, None, &pool).map_err(|e| anyhow::anyhow!(e))?
        } else {
            RelayList {
                hosts,
                explicit: true,
            }
        };
        Ok(Self { list, plain, ctx })
    }

    /// The relay of `url` with only its own host (no alternates): for the
    /// reverse mode and for tests.
    pub fn single(url: &str, token: Option<String>) -> Result<Self> {
        let (hosts, plain) = parse(url)?;
        if hosts.len() != 1 {
            bail!("the relay URL {url:?} names more than one host");
        }
        Ok(Self {
            list: RelayList {
                hosts,
                explicit: true,
            },
            plain,
            ctx: context_with(token),
        })
    }

    /// The relay of `url`, else of [`RELAY_VARIABLE`], else [`default_relay`],
    /// with the token of [`TOKEN_VARIABLE`].
    pub fn from_env(url: Option<&str>) -> Result<Self> {
        let from_env = std::env::var(RELAY_VARIABLE)
            .ok()
            .filter(|v| !v.trim().is_empty());
        let url = url.map(str::to_string).or(from_env);
        Self::new(
            url.as_deref().unwrap_or(default_relay()),
            std::env::var(TOKEN_VARIABLE).ok(),
        )
    }

    /// The URLs of the hosts, the first host first.
    pub fn urls(&self) -> Vec<String> {
        self.list
            .hosts
            .iter()
            .map(|h| url_of(h, self.plain))
            .collect()
    }

    /// The TCP port of the relay.
    pub fn port(&self) -> u16 {
        self.list.primary().port
    }

    /// The host of the relay.
    pub fn host(&self) -> &str {
        &self.list.primary().host
    }

    /// The base URL of the first host, with no token.
    pub fn url(&self) -> String {
        url_of(self.list.primary(), self.plain)
    }

    /// The first host, as `pod-relay` names it.
    pub(crate) fn primary(&self) -> &pod_relay::Relay {
        self.list.primary()
    }

    /// The context of the calls to this relay.
    pub(crate) fn ctx(&self) -> &Context {
        &self.ctx
    }

    /// The wire of a socket of the reverse road to this relay.
    pub(crate) fn wire(&self) -> pod_relay::reverse::Wire {
        if self.plain {
            pod_relay::reverse::Wire::PlainLoopback
        } else {
            pod_relay::reverse::Wire::Tls
        }
    }

    /// A byte stream to `target` through the relay: the hosts in turn
    /// (`pod-relay`). The open takes at most `timeout`, and not less than
    /// the time of one step of `pod-relay` (a mint, an upgrade).
    pub async fn connect(&self, target: SocketAddr, timeout: Duration) -> Result<BoxedIo> {
        if target.is_ipv6() {
            bail!("the relay has no IPv6 egress; {target} is IPv6");
        }
        let path = pod_relay::relay::forward_path(&target.ip().to_string(), target.port())
            .map_err(|e| anyhow::anyhow!(e))?;
        let shown = target.to_string();
        let trust = pod_ws::Trust::Default;
        let request = pod_relay::Request {
            ctx: &self.ctx,
            relays: &self.list,
            path: &path,
            trust: &trust,
            target: &shown,
            rounds: 1,
            failover: pod_relay::failover::Mode::default(),
        };
        let url = self.url();
        let mut notes = |note: &str| tracing::info!(relay = %url, "{note}");
        let limit = timeout.max(pod_relay::open::CONNECT_TIMEOUT);
        let open = async {
            if self.plain {
                pod_relay::open_on_loopback(&request, &mut notes).await
            } else {
                pod_relay::open(&request, &mut notes).await
            }
        };
        let opened = tokio::time::timeout(limit, open)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "the relay {url}: no session to {shown} in {} s",
                    limit.as_secs()
                )
            })?
            .map_err(|failure| {
                anyhow::anyhow!("the relay {url}: {}", failure.lines(&shown).join("; "))
            })?;
        Ok(Box::new(pod_relay::stream::of(opened.session)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_urls_keep_the_scheme_and_the_port() {
        let relay = Relay::new("https://tcp.ssh.relay.ajam.dev/", None).unwrap();
        assert_eq!(relay.url(), "https://tcp.ssh.relay.ajam.dev");
        assert!(relay.urls().len() > 1, "the default relay takes its pool");
        let relay = Relay::new("wss://relay.example:8443", Some("t".into())).unwrap();
        assert_eq!(
            (relay.url(), relay.port()),
            ("https://relay.example:8443".into(), 8443)
        );
        let relay = Relay::new("http://127.0.0.1:9", None).unwrap();
        assert_eq!(relay.url(), "http://127.0.0.1:9");
        assert_eq!(relay.wire(), pod_relay::reverse::Wire::PlainLoopback);
        let relay = Relay::single("https://[2001:db8::1]:8443", None).unwrap();
        assert_eq!(
            (relay.host(), relay.url()),
            ("2001:db8::1", "https://[2001:db8::1]:8443".into())
        );
        for bad in [
            "",
            " , ",
            "http://relay.example",
            "https://a.example,http://127.0.0.1:9",
        ] {
            assert!(Relay::new(bad, None).is_err(), "{bad}");
        }
        assert!(Relay::single("https://a.example,https://b.example", None).is_err());
    }

    #[test]
    fn the_token_is_not_in_debug_or_url() {
        let relay = Relay::new(default_relay(), Some("ephm1.secret".into())).unwrap();
        assert!(!format!("{relay:?}").contains("secret"));
        assert!(!relay.url().contains("secret"));
    }
}
