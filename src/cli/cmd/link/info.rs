//! `podnet link status`, `close` and `carriers`.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use podnet::carriers::{self, Options, Report};
use podnet::exit::{self, Fault};
use podnet::identity;
use podnet::instances::{self, Entry};
use podnet::link_ctl;

use crate::cli::cmd::instances::instances_stop;

/// The word that starts the command of a link instance: `link listen`.
const COMMAND: &str = "link ";

fn is_link(entry: &Entry) -> bool {
    entry.record.command.starts_with(COMMAND)
}

/// The identity of this install, as a JSON value: `null` when it has no
/// key yet (`create` makes the key). A key that cannot be used exits 71.
fn identity_doc(create: bool) -> Result<Value> {
    let dir = identity::default_dir()?;
    if create {
        return Ok(serde_json::to_value(identity::create(&dir)?.report())?);
    }
    match identity::open(&dir) {
        Ok(key) => Ok(serde_json::to_value(key.report())?),
        Err(err) if exit::code_of(&err) == podnet::exit::NO_INPUT => Ok(Value::Null),
        Err(err) => Err(err),
    }
}

fn instance_doc(entry: &Entry, live: Option<&Result<Value>>) -> Value {
    let r = &entry.record;
    let mut doc = json!({
        "name": r.name,
        "state": r.state,
        "pid": r.pid,
        "command": r.command,
        "started": r.started,
        "restarts": r.restarts,
        "log": r.log,
        "ready": r.ready,
    });
    match live {
        Some(Ok(live)) => doc["live"] = live.clone(),
        Some(Err(err)) => {
            doc["live"] = Value::Null;
            doc["live_error"] = format!("{err:#}").into();
        }
        None => doc["live"] = Value::Null,
    }
    doc
}

/// The time that a status waits for a control socket.
const LIVE_LIMIT: Duration = Duration::from_secs(3);

/// Ask the control socket `socket` for its status. The token is in
/// `token_file`, else in `SOCKET.token`.
async fn live(socket: &Path, token_file: Option<&Path>) -> Result<Value> {
    let token = match token_file {
        Some(file) => link_ctl::read_token(file)?,
        None => link_ctl::read_token(&link_ctl::token_path(socket))?,
    };
    let exchange = async {
        link_ctl::Client::connect(socket)
            .await?
            .call(&token, "status", json!({}))
            .await
    };
    match tokio::time::timeout(LIVE_LIMIT, exchange).await {
        Ok(doc) => doc,
        Err(_) => Err(exit::fault(
            Fault::Timeout,
            anyhow!("the control socket {} did not answer", socket.display()),
        )),
    }
}

/// The links of a live document, one text line each.
fn live_lines(live: &Value) -> Vec<String> {
    live["links"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            format!(
                "          peer {}  {}  {}  streams {}  opened {}  accepted {}",
                l["peer"].as_str().unwrap_or("-"),
                l["role"].as_str().unwrap_or("-"),
                l["state"].as_str().unwrap_or("-"),
                l["streams"],
                l["opened"],
                l["accepted"]
            )
        })
        .collect()
}

/// What `podnet link status` was asked.
pub(crate) struct Ask<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) json: bool,
    pub(crate) create: bool,
    pub(crate) control: Option<&'a Path>,
    pub(crate) token_file: Option<&'a Path>,
}

/// `podnet link status`.
pub(crate) async fn status(ask: Ask<'_>) -> Result<()> {
    let Ask {
        name,
        json,
        create,
        control,
        token_file,
    } = ask;
    let identity = identity_doc(create)?;
    if let Some(socket) = control {
        // One socket, asked by name: its errors are the errors of the
        // command (a wrong token exits 77).
        let live = live(socket, token_file).await?;
        if json {
            println!(
                "{}",
                podnet::status::stamped(json!({"identity": identity, "live": live}))
            );
        } else {
            println!(
                "control   {}  {}",
                socket.display(),
                live["fingerprint"].as_str().unwrap_or("")
            );
            for line in live_lines(&live) {
                println!("{line}");
            }
        }
        return Ok(());
    }
    let dir = instances::lookup_dir();
    instances::prune(&dir);
    let mut links: Vec<Entry> = instances::list(&dir).into_iter().filter(is_link).collect();
    if let Some(name) = name {
        links.retain(|e| e.record.name == name);
        if links.is_empty() {
            return Err(exit::fault(
                Fault::Unavailable,
                anyhow!("no link instance {name} runs (podnet link status)"),
            ));
        }
    }
    let mut lives = Vec::new();
    for entry in &links {
        let control = entry
            .record
            .ready
            .as_ref()
            .and_then(|d| d["control"].as_str());
        lives.push(match control {
            Some(path) => Some(live(Path::new(path), None).await),
            None => None,
        });
    }
    if json {
        let rows: Vec<Value> = links
            .iter()
            .zip(&lives)
            .map(|(e, l)| instance_doc(e, l.as_ref()))
            .collect();
        println!(
            "{}",
            podnet::status::stamped(json!({"identity": identity, "links": rows}))
        );
        return Ok(());
    }
    match &identity {
        Value::Null => println!("identity  none yet (podnet link listen or dial makes the key)"),
        key => println!(
            "identity  {}  ({})",
            key["fingerprint"].as_str().unwrap_or(""),
            key["path"].as_str().unwrap_or("")
        ),
    }
    if links.is_empty() {
        println!("links     none run in the background");
    }
    for (entry, live) in links.iter().zip(&lives) {
        let r = &entry.record;
        let listen = r
            .ready
            .as_ref()
            .and_then(|d| d["listen"].as_str().or_else(|| d["address"].as_str()))
            .unwrap_or("-");
        println!(
            "link      {}  {}  pid {}  {}  {listen}",
            r.name, r.state, r.pid, r.command
        );
        if let Some(Ok(live)) = live {
            for line in live_lines(live) {
                println!("{line}");
            }
        }
    }
    Ok(())
}

/// The time that `close` gives to the control socket to end the links.
const CLOSE_LIMIT: Duration = Duration::from_secs(15);

/// End the links of the instance `name` with the verb `close-link` of its
/// control socket, so that the peers get a GOAWAY and not a cut carrier.
/// An instance with no socket, or a socket that does not answer, is left to
/// the stop by signal: the failure is a line on stderr, not an error.
async fn close_links(name: &str) {
    let dir = instances::lookup_dir();
    let control = instances::read(&dir, name)
        .and_then(|r| r.ready)
        .and_then(|d| d["control"].as_str().map(std::path::PathBuf::from));
    let Some(socket) = control else { return };
    let work = async {
        let token = link_ctl::read_token(&link_ctl::token_path(&socket))?;
        let mut client = link_ctl::Client::connect(&socket).await?;
        let status = client.call(&token, "status", json!({})).await?;
        for row in status["links"].as_array().into_iter().flatten() {
            if row["state"] != "up" {
                continue;
            }
            client
                .call(&token, "close-link", json!({"link": row["id"]}))
                .await?;
            println!(
                "closed link {}  {}",
                row["id"],
                row["peer"].as_str().unwrap_or("-")
            );
        }
        Ok::<(), anyhow::Error>(())
    };
    let why = match tokio::time::timeout(CLOSE_LIMIT, work).await {
        Ok(Ok(())) => return,
        Ok(Err(err)) => format!("{err:#}"),
        Err(_) => format!("no answer in {} s", CLOSE_LIMIT.as_secs()),
    };
    eprintln!(
        "podnet: link close: the control socket of {name}: {why}; the instance stops by signal"
    );
}

/// `podnet link close`: end each link of the instance through its control
/// socket when the socket answers, then stop the instance.
pub(crate) async fn close(name: &str, timeout: u64) -> Result<()> {
    instances::check_name(name).map_err(|e| exit::fault(Fault::Usage, e))?;
    let dir = instances::lookup_dir();
    if let Some(record) = instances::read(&dir, name) {
        if !record.command.starts_with(COMMAND) {
            return Err(exit::fault(
                Fault::Usage,
                anyhow!(
                    "{name} is no link instance ({}): podnet stop {name} ends it",
                    record.command
                ),
            ));
        }
    }
    close_links(name).await;
    instances_stop(Some(name.to_string()), false, timeout)
}

fn report_doc(r: &Report) -> Value {
    json!({
        "name": r.name,
        "about": r.about,
        "address": r.address,
        "addressing": r.addressing.name(),
        "framing": r.framing(),
        "duplex": r.facts.duplex,
        "ordered": r.facts.ordered,
        "mtu": r.facts.mtu,
        "cost": r.cost,
        "needs_privilege": r.needs_privilege,
        "feature": r.feature,
        "built": r.built,
        "available": r.available.is_ok(),
        "reason": r.available.as_ref().err(),
    })
}

/// `podnet link carriers`.
pub(crate) fn carriers(options: &Options, json: bool) -> Result<()> {
    let reports = carriers::report(options);
    if json {
        println!(
            "{}",
            podnet::status::stamped(json!({
                "carriers": reports.iter().map(report_doc).collect::<Vec<_>>(),
            }))
        );
        return Ok(());
    }
    for r in &reports {
        println!("{}", r.line());
    }
    Ok(())
}
