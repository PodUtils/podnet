//! `podnet warp tun`: a TUN device whose packets go through WARP.

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;

#[cfg(target_os = "linux")]
use podnet::exit::{Fault, ResultExt};

use super::{WarpArgs, WarpCommand};
#[cfg(target_os = "linux")]
use crate::cli::signals::shutdown_signal;

/// `podnet warp tun`.
pub(crate) async fn run(args: WarpArgs, client: podnet::warp::WarpClient) -> Result<()> {
    match args.command {
        #[cfg(target_os = "linux")]
        WarpCommand::Tun {
            interface: name,
            default_route,
            keep_rp_filter,
            ..
        } => {
            podnet::warp::tun::rp_filter::set_keep(keep_rp_filter);
            let address = client
                .device()
                .ipv4()
                .context("the device has no IPv4 address")
                .fault(Fault::Unavailable)?;
            // Issue 102: a TUN that cannot be made, or a session that
            // cannot open, is an unavailable path (69), as for warp up --tun.
            let tun =
                podnet::warp::tun::Tun::create(&name, address, 1280).fault(Fault::Unavailable)?;
            let session = client.connect_raw().await.fault(Fault::Unavailable)?;
            // The IPv6-only kit, 0001: the front door of the session, not
            // the first address of the device record.
            let endpoint = client
                .endpoint_in_use()
                .map(|e| e.ip().to_string())
                .unwrap_or_default();
            println!("podnet: tun      {} {address}/32 mtu 1280", tun.name);
            if let Some(v6) = client.device().ipv6() {
                match tun.add_ipv6(v6) {
                    Ok(()) => println!("podnet: tun      {} {v6}/128", tun.name),
                    Err(err) => {
                        tracing::warn!(error = %format!("{err:#}"), "the TUN has no IPv6 address")
                    }
                }
            }
            println!(
                "podnet: masque   {endpoint} (do not route this address into {})",
                tun.name
            );
            println!(
                "  ip route add 1.1.1.1/32 dev {}        # one destination",
                tun.name
            );
            println!(
                "  ip route add 0.0.0.0/1 dev {0}; ip route add 128.0.0.0/1 dev {0}",
                tun.name
            );
            let host_prefix = if endpoint.contains(':') {
                "-6 route add {}/128"
            } else {
                "route add {}/32"
            };
            println!(
                "  ip {} via <your gateway>   # keep the session outside",
                host_prefix.replace("{}", &endpoint)
            );
            // --default-route (issue 77): the addresses of the MASQUE
            // connection stay outside the tunnel.
            let routes = if default_route {
                // The front door of the session (or its proxy) must stay
                // outside the TUN; the relay of a later session should,
                // where a route to it exists.
                let session_host = match podnet::egress::proxy_for(&endpoint, 443) {
                    Some(proxy) => proxy.host,
                    None => endpoint.clone(),
                };
                let mut relay_host = String::new();
                if !args.no_relay {
                    if let Ok(relay) = podnet::ws_relay::Relay::from_env(None) {
                        // Issue 105: a relay that is reached through the
                        // proxy needs no route of its own.
                        relay_host = match podnet::egress::proxy_for(relay.host(), relay.port()) {
                            Some(proxy) => proxy.host,
                            None => relay.host().to_string(),
                        };
                    }
                }
                let addresses = |host: String| async move {
                    if host.is_empty() {
                        return Vec::new();
                    }
                    podnet::resolve::resolve(&host).await.unwrap_or_default()
                };
                let required = addresses(session_host).await;
                let mut optional = addresses(relay_host).await;
                optional.retain(|ip| !required.contains(ip));
                let routes = podnet::warp::route::Routes::setup(
                    &tun.name,
                    required,
                    optional,
                    client.device().ipv6().is_some(),
                )
                .fault(Fault::Unavailable)?;
                let outside = routes
                    .exempted()
                    .iter()
                    .map(|ip| ip.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "podnet: routes   the default routes go to {}; outside: {}",
                    tun.name,
                    if outside.is_empty() {
                        "none needed (the MASQUE connection goes through loopback or a connected subnet)".to_string()
                    } else {
                        outside
                    }
                );
                Some(std::sync::Mutex::new(routes))
            } else {
                None
            };
            podnet::detach::ready(serde_json::json!({
                "tun": tun.name,
                "address": address.to_string(),
                "default_route": default_route,
                "rp_filter": podnet::warp::tun::rp_filter::apply(&tun.name),
            }));
            // The TUN stays; a session that ends is opened again (issue 75).
            let tun_ref = &tun;
            let client_ref = &client;
            let routes_ref = &routes;
            let keep = async move {
                let mut session = Some(session);
                let mut delay = std::time::Duration::from_secs(1);
                loop {
                    let current = match session.take() {
                        Some(s) => s,
                        None => match client_ref.connect_raw().await {
                            Ok(s) => {
                                tracing::info!("a new WARP session is open");
                                delay = std::time::Duration::from_secs(1);
                                s
                            }
                            Err(err) => {
                                tracing::warn!(error = %format!("{err:#}"), next_try_s = delay.as_secs(), "a new WARP session failed");
                                tokio::time::sleep(delay).await;
                                delay = (delay * 2).min(std::time::Duration::from_secs(30));
                                continue;
                            }
                        },
                    };
                    // The default routes only while a session is up.
                    if let Some(routes) = routes_ref {
                        let up = routes.lock().unwrap_or_else(|e| e.into_inner()).up();
                        if let Err(err) = up {
                            // #278: no session with half of the default
                            // routes; up() removed the half that was in.
                            tracing::error!(error = %format!("{err:#}"), next_try_s = delay.as_secs(), "no default routes on the TUN; podnet tries again");
                            drop(current);
                            tokio::time::sleep(delay).await;
                            delay = (delay * 2).min(std::time::Duration::from_secs(30));
                            continue;
                        }
                    }
                    if let Err(err) = podnet::warp::tun::run(tun_ref, current).await {
                        tracing::warn!(error = %format!("{err:#}"), "the WARP session ended; opening a new one");
                    }
                    if let Some(routes) = routes_ref {
                        routes.lock().unwrap_or_else(|e| e.into_inner()).down();
                    }
                }
            };
            tokio::select! {
                _ = keep => {}
                _ = shutdown_signal() => {}
            }
            // Dropped: the routes and the resolver are put back.
            drop(routes);
        }
        #[cfg(not(target_os = "linux"))]
        WarpCommand::Tun { .. } => anyhow::bail!("podnet warp tun is available on Linux only"),
        _ => unreachable!("warp() gives this function its commands only"),
    }
    Ok(())
}
