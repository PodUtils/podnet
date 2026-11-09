//! `podnet warp run`: a program in a pod whose traffic goes through WARP.

use std::sync::Arc;

use anyhow::{Context, Result};

use podnet::exit::{Fault, ResultExt};

use super::{WarpArgs, WarpCommand};

/// `podnet warp run`.
pub(crate) async fn run(args: WarpArgs, client: podnet::warp::WarpClient) -> Result<()> {
    match args.command {
        WarpCommand::Run {
            backend,
            rules,
            keep_proxy_env,
            inbound_dir,
            program,
        } => {
            // netns-raw (issue 79): the TUN of a namespace goes to WARP as
            // raw packets: TCP, UDP, ICMP and IPv6 with no root. auto takes
            // it when this host can make the namespace.
            let raw = backend == "netns-raw"
                || (backend == "auto" && rules.is_empty() && podnet_netns::raw::available());
            if raw {
                match warp_run_raw(&client, &program, keep_proxy_env).await {
                    Ok(code) => std::process::exit(code),
                    // Issue 251: the program cannot run. The proxy pod
                    // cannot run it either: exit 3, as the netns backend.
                    Err(err) if err.downcast_ref::<podnet_netns::raw::ExecError>().is_some() => {
                        eprintln!("podnet: warp run: {err:#}");
                        std::process::exit(podnet::exit::NO_BACKEND);
                    }
                    Err(err) if backend == "auto" => {
                        tracing::info!(error = %format!("{err:#}"), "the netns-raw backend cannot run here; the pod with a proxy runs");
                    }
                    Err(err) => return Err(podnet::exit::fault(Fault::Unavailable, err)),
                }
            }
            let backend = if backend == "netns-raw" {
                "auto".to_string()
            } else {
                backend
            };
            let socket =
                std::env::temp_dir().join(format!("podnet-warp-{}.sock", std::process::id()));
            let live = Arc::new(client)
                .connect_live()
                .await
                .fault(Fault::Unavailable)?;
            let proxy = podnet::warp::proxy::serve(
                live,
                podnet::vnet::proxy::ProxyListen::Unix(socket.clone()),
            )
            .await?;
            let mut argv = vec![
                "--backend".to_string(),
                backend.clone(),
                "-x".to_string(),
                format!("socks5://unix:{}", socket.display()),
            ];
            if backend == "auto" {
                argv.push("--prefer-udp".to_string());
            }
            for rule in rules {
                argv.push("-r".to_string());
                argv.push(rule);
            }
            if keep_proxy_env {
                argv.push("--keep-proxy-env".to_string());
            }
            if let Some(dir) = inbound_dir {
                argv.push("--inbound-dir".to_string());
                argv.push(dir.display().to_string());
            }
            argv.push("--".to_string());
            argv.extend(program);
            // podnet-netns makes its own runtime, so run it on a plain thread.
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let _ = done_tx.send(podnet_netns::main_with_args(&argv));
            });
            let code = done_rx.await.unwrap_or(1);
            proxy.abort();
            let _ = std::fs::remove_file(&socket);
            std::process::exit(code);
        }
        _ => unreachable!("warp() gives this function its commands only"),
    }
}

/// `podnet warp run --backend netns-raw`: the program in a raw pod whose TUN
/// packets go through WARP; a session that ends is opened again. Gives the
/// exit code of the program.
async fn warp_run_raw(
    client: &podnet::warp::WarpClient,
    program: &[String],
    keep_proxy_env: bool,
) -> Result<i32> {
    let v4 = client
        .device()
        .ipv4()
        .context("the device has no IPv4 address")?;
    let mut session = Some(client.connect_raw().await?);
    let pod = podnet_netns::raw::spawn(&podnet_netns::raw::RawOptions {
        program: program.to_vec(),
        keep_proxy_env,
        v4,
        v6: client.device().ipv6(),
        mtu: podnet::warp::ip::TUNNEL_MTU,
        nameserver: podnet::warp::route::nameservers_of_service()[0],
    })?;
    let pid = pod.pid;
    let tun = podnet::warp::tun::Tun::from_fd(pod.tun, "eth0")?;
    tracing::info!(pid, "the program runs in a raw pod (netns-raw)");
    let carry = async {
        let mut delay = std::time::Duration::from_secs(1);
        loop {
            let current = match session.take() {
                Some(s) => s,
                None => match client.connect_raw().await {
                    Ok(s) => {
                        tracing::info!("a new WARP session is open");
                        delay = std::time::Duration::from_secs(1);
                        s
                    }
                    Err(err) => {
                        tracing::warn!(error = %format!("{err:#}"), "a new WARP session failed");
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(std::time::Duration::from_secs(30));
                        continue;
                    }
                },
            };
            if let Err(err) = podnet::warp::tun::run(&tun, current).await {
                tracing::warn!(error = %format!("{err:#}"), "the WARP session ended; opening a new one");
            }
        }
    };
    let exited = async {
        loop {
            if let Some(code) = podnet_netns::raw::try_wait(pid) {
                return code;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        code = exited => Ok(code),
        _ = carry => unreachable!("the loop does not end"),
    }
}
