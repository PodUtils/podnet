//! `podnet warp proxy` and `podnet warp forward`: the local front ends of
//! the WARP session.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use podnet::exit::{Fault, ResultExt};

use super::client::{identity_pairs, masque_options};
use super::{WarpArgs, WarpCommand};
use crate::cli::cmd::run::listen_doc;
use crate::cli::signals::sleep_or_signal;

/// The forms of `warp proxy` and `warp forward` that need no client. True
/// when the command is done.
pub(crate) fn before_client(args: &WarpArgs) -> Result<bool> {
    match &args.command {
        WarpCommand::Forward { list: true, .. } => {
            let list = podnet::instances::list(&podnet::instances::lookup_dir());
            print!("{}", podnet::instances::text::forward_list(&list));
            return Ok(true);
        }
        // Issues 132 and 136: check the rules with no device and no session.
        WarpCommand::Proxy {
            rules,
            rules_file,
            rule_default,
            check: true,
            ..
        } => {
            let default = podnet::warp::rules::Action::parse(rule_default).fault(Fault::Usage)?;
            let table = podnet::warp::rules::build(rules, rules_file.as_deref(), default)
                .fault(Fault::Usage)?;
            for rule in &table.rules {
                println!("rule        {}", rule.spec);
            }
            println!("default     {}", table.default.name());
            return Ok(true);
        }
        // Issue 150: a bad --listen-identity is a usage error before any
        // registration.
        WarpCommand::Proxy {
            listen_identity,
            listen,
            ..
        } if !listen_identity.is_empty() => {
            let main = match &args.identity {
                Some(path) => Some(path.clone()),
                None if args.ephemeral => None,
                None => podnet::warp::WarpClient::default_identity_path().fault(Fault::Config)?,
            };
            identity_pairs(listen_identity, main.as_deref(), listen).fault(Fault::Usage)?;
        }
        _ => {}
    }
    Ok(false)
}

/// `podnet warp proxy` and `podnet warp forward` with a client.
pub(crate) async fn run(
    args: WarpArgs,
    client: podnet::warp::WarpClient,
    path: Option<PathBuf>,
) -> Result<()> {
    let masque = masque_options(&args);
    match args.command {
        WarpCommand::Proxy {
            listen,
            run_for,
            strict,
            max_connections,
            dns,
            proxy_user,
            proxy_password_file,
            rules,
            rules_file,
            rule_default,
            listen_identity,
            l4,
            l4_sni,
            ..
        } => {
            // Issue 150: the listeners of the other identities.
            let pairs =
                identity_pairs(&listen_identity, path.as_deref(), &listen).fault(Fault::Usage)?;
            podnet::warp::proxy::set_max_connections(max_connections);
            // Issues 132 and 136: the rules of the destinations.
            let default = podnet::warp::rules::Action::parse(&rule_default).fault(Fault::Usage)?;
            let table = podnet::warp::rules::build(&rules, rules_file.as_deref(), default)
                .fault(Fault::Usage)?;
            podnet::warp::rules::set(table);
            if let Some(file) = rules_file {
                podnet::warp::rules::watch(rules, file, default);
            }
            // Issue 146: the credentials of the clients.
            if let (Some(user), Some(file)) = (&proxy_user, &proxy_password_file) {
                let credentials = podnet::ingress::credentials(user, file).fault(Fault::Config)?;
                podnet::warp::proxy::set_credentials(credentials);
            }
            // Issue 117: the DNS listener is bound before the session, as
            // the proxy listeners.
            let dns_socket = match dns {
                Some(addr) => Some(
                    tokio::net::UdpSocket::bind(addr)
                        .await
                        .with_context(|| format!("binding the DNS address {addr}"))
                        .fault(Fault::Unavailable)?,
                ),
                None => None,
            };
            let listens = listen
                .iter()
                .map(|l| podnet::vnet::proxy::ProxyListen::parse(l))
                .collect::<Result<Vec<_>>>()
                .fault(Fault::Usage)?;
            // Issue 97: each listener is bound on its own, before the WARP
            // session. A listener that the host refuses (EACCES on a TCP
            // bind) is reported; the proxy starts with the others.
            let (bound, mut failed) = podnet::warp::proxy::bind_all(&listens).await;
            let mut extra = Vec::new();
            for (listen, identity) in pairs {
                match podnet::warp::proxy::bind(&listen).await {
                    Ok(b) => extra.push((b, identity)),
                    Err(err) => failed.push((listen, err)),
                }
            }
            for (listen, err) in &failed {
                eprintln!(
                    "podnet: warp proxy: {} is not available: {err:#}",
                    listen_doc(listen)["listen"].as_str().unwrap_or("?")
                );
            }
            if bound.is_empty() || (strict && !failed.is_empty()) {
                let list = failed
                    .iter()
                    .map(|(_, err)| format!("{err:#}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(podnet::exit::fault(
                    Fault::Unavailable,
                    anyhow::anyhow!("warp proxy: a listener did not start: {list}"),
                ));
            }
            // Issue 140: one HTTP/3 CONNECT for each client, no stack.
            if l4 {
                let door = Arc::new(client.l4(&l4_sni).fault(Fault::Config)?);
                door.connection().await.fault(Fault::Unavailable)?;
                let proxies: Vec<_> = bound
                    .into_iter()
                    .map(|b| podnet::warp::proxy::run_l4(door.clone(), b))
                    .collect();
                for proxy in &proxies {
                    println!(
                        "podnet: warp proxy  {}  (L4: {l4_sni}, HTTP/3 on UDP; TCP only)",
                        listen_doc(&proxy.listen)["listen"].as_str().unwrap_or("?")
                    );
                }
                let mut doc = listen_doc(&proxies[0].listen);
                doc["listens"] = proxies
                    .iter()
                    .map(|p| listen_doc(&p.listen)["listen"].clone())
                    .collect::<Vec<_>>()
                    .into();
                doc["l4"] = true.into();
                podnet::detach::ready(doc);
                sleep_or_signal(run_for).await;
                for proxy in &proxies {
                    proxy.abort();
                }
                return Ok(());
            }
            let live = Arc::new(client)
                .connect_live()
                .await
                .fault(Fault::Unavailable)?;
            podnet::warp::proxy::health_log(live.clone());
            if let Some(socket) = dns_socket {
                podnet::warp::proxy::serve_dns(live.clone(), socket);
            }
            let mut proxies: Vec<_> = bound
                .into_iter()
                .map(|b| podnet::warp::proxy::run(live.clone(), b))
                .collect();
            let mut identities = Vec::new();
            for (b, identity) in extra {
                let other = podnet::warp::WarpClient::register_or_load(&identity)
                    .await
                    .fault(Fault::Unavailable)?
                    .with_masque(masque.clone());
                let device = other.device().id.clone();
                let other = Arc::new(other)
                    .connect_live()
                    .await
                    .fault(Fault::Unavailable)?;
                let proxy = podnet::warp::proxy::run(other, b);
                identities.push(serde_json::json!({
                    "listen": listen_doc(&proxy.listen)["listen"],
                    "identity": identity.display().to_string(),
                    "device": device,
                }));
                proxies.push(proxy);
            }
            for proxy in &proxies {
                match &proxy.listen {
                    podnet::vnet::proxy::ProxyListen::Tcp(addr) => {
                        println!("podnet: warp proxy  socks5h://{addr}  (and HTTP CONNECT)");
                        println!("  export ALL_PROXY=socks5h://{addr} HTTPS_PROXY=http://{addr}");
                    }
                    podnet::vnet::proxy::ProxyListen::Unix(path) => {
                        println!("podnet: warp proxy  unix:{}", path.display());
                        println!(
                            "  podnet pod -x socks5://unix:{} -- PROGRAM",
                            path.display()
                        );
                    }
                }
            }
            let mut doc = listen_doc(&proxies[0].listen);
            doc["listens"] = proxies
                .iter()
                .map(|p| listen_doc(&p.listen)["listen"].clone())
                .collect::<Vec<_>>()
                .into();
            doc["failed"] = failed
                .iter()
                .map(|(listen, err)| {
                    serde_json::json!({
                        "listen": listen_doc(listen)["listen"],
                        "error": format!("{err:#}"),
                    })
                })
                .collect::<Vec<_>>()
                .into();
            doc["identities"] = identities.into();
            podnet::detach::ready(podnet::warp::live::instance::with_data_path(doc));
            sleep_or_signal(run_for).await;
            for proxy in &proxies {
                proxy.abort();
            }
        }
        WarpCommand::Forward {
            local,
            strict,
            max_connections,
            run_for,
            ..
        } => {
            // Issue 143: check each rule before any bind or session (exit 64).
            let rules = local
                .iter()
                .map(|spec| podnet::warp::forward::Rule::parse(spec))
                .collect::<Result<Vec<_>>>()
                .fault(Fault::Usage)?;
            let mut bound = Vec::new();
            let mut failed = Vec::new();
            for rule in rules {
                match podnet::warp::proxy::bind(&rule.listen).await {
                    Ok(b) => bound.push((b, rule)),
                    Err(err) => {
                        eprintln!(
                            "podnet: warp forward: {} is not available: {err:#}",
                            rule.target()
                        );
                        failed.push((rule, err));
                    }
                }
            }
            if bound.is_empty() || (strict && !failed.is_empty()) {
                let list = failed
                    .iter()
                    .map(|(_, err)| format!("{err:#}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(podnet::exit::fault(
                    Fault::Unavailable,
                    anyhow::anyhow!("warp forward: a listener did not start: {list}"),
                ));
            }
            let live = Arc::new(client)
                .connect_live()
                .await
                .fault(Fault::Unavailable)?;
            podnet::warp::proxy::health_log(live.clone());
            let forwards: Vec<_> = bound
                .into_iter()
                .map(|(b, rule)| podnet::warp::forward::run(live.clone(), b, rule, max_connections))
                .collect();
            let mut docs = Vec::new();
            for forward in &forwards {
                let listen = listen_doc(&forward.listen)["listen"].clone();
                println!(
                    "podnet: warp forward  {}  ->  {} (through WARP)",
                    listen.as_str().unwrap_or("?"),
                    forward.rule.target()
                );
                docs.push(serde_json::json!({ "listen": listen, "to": forward.rule.target() }));
            }
            let mut doc = listen_doc(&forwards[0].listen);
            doc["forward"] = docs.into();
            doc["failed"] = failed
                .iter()
                .map(|(rule, err)| {
                    serde_json::json!({
                        "listen": listen_doc(&rule.listen)["listen"],
                        "to": rule.target(),
                        "error": format!("{err:#}"),
                    })
                })
                .collect::<Vec<_>>()
                .into();
            podnet::detach::ready(doc);
            sleep_or_signal(run_for).await;
            for forward in &forwards {
                forward.abort();
            }
        }
        _ => unreachable!("warp() gives this function its commands only"),
    }
    Ok(())
}
