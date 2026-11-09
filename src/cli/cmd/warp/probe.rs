//! `podnet warp scan`, `exit` and `probe`: the front doors, the exit and the
//! targets through WARP.

use anyhow::{Context, Result};

use podnet::exit::{Fault, ResultExt};

use super::{WarpArgs, WarpCommand};
use crate::cli::cmd::tunnel::split_host_port;

/// The probe subcommands of `podnet warp`.
pub(crate) async fn run(args: WarpArgs, client: podnet::warp::WarpClient) -> Result<()> {
    match args.command {
        WarpCommand::Scan {
            candidate,
            timeout_ms,
            json,
        } => {
            let candidates = candidate
                .iter()
                .map(|c| parse_door(c))
                .collect::<Result<Vec<_>>>()
                .fault(Fault::Usage)?;
            let timeout = std::time::Duration::from_millis(timeout_ms);
            let results = client
                .scan(&candidates, timeout)
                .await
                .fault(Fault::Config)?;
            let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
            if json {
                let doors: Vec<_> = results
                    .iter()
                    .map(|(a, r)| match r {
                        Ok(t) => serde_json::json!({
                            "endpoint": a.to_string(),
                            "ok": true,
                            "tcp_ms": t.tcp.as_millis() as u64,
                            "tls_ms": t.tls.as_millis() as u64,
                        }),
                        Err(err) => serde_json::json!({
                            "endpoint": a.to_string(),
                            "ok": false,
                            "error": format!("{err:#}"),
                        }),
                    })
                    .collect();
                println!("{}", serde_json::json!({ "doors": doors }));
            } else {
                for (a, r) in &results {
                    match r {
                        Ok(t) => println!(
                            "OK          {a:<28} tcp {:>5} ms  tls {:>5} ms",
                            t.tcp.as_millis(),
                            t.tls.as_millis()
                        ),
                        Err(err) => println!("FAIL        {a:<28} {err:#}"),
                    }
                }
                if let Some((best, _)) = results.iter().find(|(_, r)| r.is_ok()) {
                    println!("use         podnet warp --masque-endpoint {best} ...");
                }
            }
            if ok == 0 {
                return Err(podnet::exit::fault(
                    Fault::Unavailable,
                    anyhow::anyhow!("no front door answered"),
                ));
            }
        }
        WarpCommand::Exit { json, no_ipv6 } => {
            let stack = client.connect().await.fault(Fault::Unavailable)?;
            let mut exit = podnet::warp::exit::trace(&stack)
                .await
                .fault(Fault::Unavailable)?;
            // Issue 184: the IPv6 egress address changes with each session.
            if !no_ipv6 {
                if client.device().ipv6().is_none() {
                    exit.ipv6_error = Some("the device has no IPv6 address".into());
                } else {
                    match podnet::warp::exit::egress_v6(&stack).await {
                        Ok(ip) => exit.ipv6 = Some(ip),
                        Err(err) => exit.ipv6_error = Some(format!("{err:#}")),
                    }
                }
            }
            if json {
                println!("{}", exit.to_json());
            } else {
                println!("colo        {}", exit.colo);
                println!("country     {}", exit.loc.as_deref().unwrap_or("-"));
                println!("ip          {}", exit.ip.as_deref().unwrap_or("-"));
                if !no_ipv6 {
                    match (&exit.ipv6, &exit.ipv6_error) {
                        (Some(ip), _) => println!("ipv6        {ip}"),
                        (None, Some(why)) => println!("ipv6        - ({why})"),
                        _ => println!("ipv6        -"),
                    }
                }
                println!("warp        {}", exit.warp.as_deref().unwrap_or("-"));
                println!("rtt         {} ms", exit.rtt.as_millis());
            }
        }
        WarpCommand::Probe { target } => {
            let targets = if target.is_empty() {
                podnet::services::probe_targets()
            } else {
                target
            };
            // Check each target before any output or connection: a bad
            // value is a usage error (exit 64).
            let targets = targets
                .iter()
                .map(|spec| split_host_port(spec).map(|(h, p)| (spec.clone(), h, p)))
                .collect::<Result<Vec<_>>>()
                .fault(Fault::Usage)?;
            println!("device      {}", client.device().id);
            println!(
                "masque      {}",
                client
                    .device()
                    .masque_endpoint()
                    .map_or_else(|| "-".to_string(), |a| a.to_string())
            );
            let stack = client.connect().await.fault(Fault::Unavailable)?;
            println!(
                "tunnel      up ({})",
                if client.is_wireguard() {
                    "WireGuard over UDP"
                } else if args.masque_transport.as_deref() == Some("h3") {
                    "MASQUE over HTTP/3, UDP 443"
                } else {
                    "MASQUE over TCP 443"
                }
            );
            // Issue 147: the exit of the session. It does not change the
            // exit code.
            match podnet::warp::exit::trace(&stack).await {
                Ok(exit) => println!("exit        {}", exit_line(&exit)),
                Err(err) => println!("exit        - ({err:#})"),
            }
            let total = targets.len() + 1;
            let mut failed = 0;
            // Issue 178: the summary names the failed checks.
            let mut names: Vec<String> = Vec::new();
            // Issue 115: the UDP data path: a DNS query through the tunnel.
            match podnet::warp::proxy::check_data_path(&stack, true).await {
                Ok(took) => println!("OK          udp 1.1.1.1:53 (DNS) in {took:?}"),
                Err(err) => {
                    failed += 1;
                    names.push("udp 1.1.1.1:53".into());
                    println!("FAIL        udp 1.1.1.1:53 (DNS): {err:#}");
                }
            }
            for (spec, host, port) in targets {
                let started = std::time::Instant::now();
                let attempt = async {
                    // A name is resolved by DNS through the tunnel.
                    let ip = podnet::warp::proxy::resolve(&stack, &host).await?;
                    let addr = std::net::SocketAddr::new(ip, port);
                    let conn = stack.tcp_connect(addr).await?;
                    Ok::<_, anyhow::Error>((addr, conn))
                };
                match tokio::time::timeout(std::time::Duration::from_secs(12), attempt).await {
                    Ok(Ok((addr, conn))) => {
                        println!("OK          {spec} ({addr}) in {:?}", started.elapsed());
                        conn.close().await;
                    }
                    Ok(Err(err)) => {
                        failed += 1;
                        names.push(spec.clone());
                        println!("FAIL        {spec}: {err:#}");
                    }
                    Err(_) => {
                        failed += 1;
                        names.push(spec.clone());
                        println!("TIMEOUT     {spec}");
                    }
                }
            }
            // Issue 95: a target that fails is a failure of the probe.
            if failed > 0 {
                return Err(podnet::exit::fault(
                    Fault::Unavailable,
                    anyhow::anyhow!(
                        "{failed} of {total} targets failed through WARP: {}",
                        names.join(", ")
                    ),
                ));
            }
        }
        _ => unreachable!("warp() gives this function its commands only"),
    }
    Ok(())
}

/// A front door of `warp scan` (issue 139): IP or IP:PORT, port 443.
pub(crate) fn parse_door(spec: &str) -> Result<std::net::SocketAddr> {
    let spec = spec.trim();
    if let Ok(addr) = spec.parse::<std::net::SocketAddr>() {
        return Ok(addr);
    }
    let ip: std::net::IpAddr = spec
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .with_context(|| format!("{spec:?} is not IP or IP:PORT"))?;
    Ok(std::net::SocketAddr::new(ip, 443))
}

/// One line of an exit (issue 147): `AMS NL 104.28.1.2 warp=on 42 ms`.
fn exit_line(exit: &podnet::warp::exit::Exit) -> String {
    format!(
        "{} {} {} warp={} {} ms",
        exit.colo,
        exit.loc.as_deref().unwrap_or("-"),
        exit.ip.as_deref().unwrap_or("-"),
        exit.warp.as_deref().unwrap_or("-"),
        exit.rtt.as_millis()
    )
}
