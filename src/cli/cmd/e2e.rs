//! `podnet tunnel --e2e`: the session end to end. The handshake runs on the
//! data stream before any application byte, the sealed destination travels
//! on the control stream, and the records come from the handshake exporter.
//! A relay forwards both streams and holds no key.

use anyhow::{anyhow, Context, Result};
use tokio::net::{TcpListener, TcpStream};

use podnet::e2e::{self, legs, Config};
use podnet::exit::{Fault, ResultExt};

use super::tunnel::TunnelArgs;

/// The e2e flags of `tunnel`. None of the Cloudflare flags apply.
pub(crate) async fn tunnel_e2e(args: &TunnelArgs) -> Result<()> {
    let inspect = args.inspect;
    let (relay, pair_name) = match args.pair.as_deref() {
        Some(pair) => pair.split_once('/').ok_or_else(|| {
            usage("--pair is RELAY/PAIR, the two names that the relay advertises")
        })?,
        // The table and the relay view are a local model: they need no
        // relay of the caller, so `podnet tunnel --inspect` alone works.
        None if inspect => ("relay", "pair"),
        None => return Err(usage("--e2e needs --pair RELAY/PAIR")),
    };
    if relay.is_empty() || pair_name.is_empty() {
        return Err(usage("--pair is RELAY/PAIR with both names"));
    }
    let suites = if args.suite.is_empty() {
        vec![e2e::suite_of(e2e::DEFAULT_SUITE)?]
    } else {
        args.suite
            .iter()
            .map(|name| e2e::suite_of(name))
            .collect::<Result<Vec<_>>>()?
    };
    let (host, port) = match &args.destination {
        Some(text) => e2e::parse_destination(text)?,
        None if inspect => ("example.invalid".to_string(), 0),
        None => return Err(usage("--e2e needs --destination HOST:PORT")),
    };
    let peers = args
        .peer
        .iter()
        .map(|text| e2e::public_key(text))
        .collect::<Result<Vec<_>>>()?;
    let config = Config {
        relay: relay.to_string(),
        pair: pair_name.to_string(),
        suites,
        host,
        port,
        service: args.service.clone(),
        allowed: peers.clone(),
        compat_destination: args.compat_destination,
    };
    if inspect {
        let peer = match peers.first() {
            Some(peer) => *peer,
            None => podnet::identity::create(&podnet::identity::default_dir()?)
                .context("the identity of this end")?
                .public(),
        };
        print!("{}", e2e::inspect(&config, &peer).await?);
        return Ok(());
    }
    check_token(args, relay, &args.service)?;

    if args.listen.is_some() || args.control_listen.is_some() {
        let data = args
            .listen
            .as_deref()
            .ok_or_else(|| usage("the far end needs --listen IP:PORT"))?;
        let control = args
            .control_listen
            .as_deref()
            .ok_or_else(|| usage("the far end needs --control-listen IP:PORT"))?;
        let data = TcpListener::bind(e2e::socket_address(data)?)
            .await
            .with_context(|| format!("--listen {data}"))?;
        let control = TcpListener::bind(e2e::socket_address(control)?)
            .await
            .with_context(|| format!("--control-listen {control}"))?;
        let (data, control) = tokio::join!(data.accept(), control.accept());
        let (data, from) = data.context("the data stream")?;
        let (control, control_from) = control.context("the control stream")?;
        eprintln!("e2e: data {from}, control {control_from}");
        let stats = e2e::server(data, control, &config).await?;
        print_stats(&stats);
        return Ok(());
    }

    let data = args
        .connect
        .as_deref()
        .ok_or_else(|| usage("the near end needs --connect IP:PORT, or --listen IP:PORT"))?;
    let control = args
        .control
        .as_deref()
        .ok_or_else(|| usage("the near end needs --control IP:PORT"))?;
    // EGRESS-3 and EGRESS-4: the two peers of the relay are destinations of
    // the deployment's policy, checked before either socket opens.
    for text in [data, control] {
        let (host, port) = e2e::parse_destination(text)?;
        podnet::policy::permit_fault(&host, port)?;
    }
    let (data, control) = tokio::join!(
        TcpStream::connect(e2e::socket_address(data)?),
        TcpStream::connect(e2e::socket_address(control)?)
    );
    let data = data.context("--connect")?;
    let control = control.context("--control")?;
    let app = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    let stats = e2e::client(data, control, app, &config).await?;
    print_stats(&stats);
    Ok(())
}

fn print_stats(stats: &e2e::Stats) {
    eprintln!(
        "e2e: suite {}, out {} bytes, in {} bytes, {} records, epochs {} and {}",
        stats.suite, stats.out, stats.in_bytes, stats.records, stats.epochs.0, stats.epochs.1
    );
}

/// A token for one relay and one service does not open another (LEARN-2).
fn check_token(args: &TunnelArgs, relay: &str, service: &str) -> Result<()> {
    let Some(token) = &args.e2e_token else {
        return Ok(());
    };
    let path = args
        .e2e_token_seed_file
        .as_deref()
        .ok_or_else(|| usage("--e2e-token needs --e2e-token-seed-file PATH"))?;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("--e2e-token-seed-file {}", path.display()))
        .fault(Fault::NoInput)?;
    let seed = legs::unhex(text.trim())
        .ok_or_else(|| usage("the token seed file holds 64 hex digits on its first line"))?;
    if !e2e::verify_token(&seed, relay, service, token) {
        return Err(podnet::exit::fault(
            Fault::NoPermission,
            anyhow!("the token does not open relay {relay:?} for service {service:?}"),
        ));
    }
    Ok(())
}

fn usage(text: &str) -> anyhow::Error {
    podnet::exit::fault(Fault::Usage, anyhow!("{text}"))
}
