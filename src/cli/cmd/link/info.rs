//! `podnet link status`, `close` and `carriers`.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use podnet::carriers::{self, Options, Report};
use podnet::exit::{self, Fault};
use podnet::identity;
use podnet::instances::{self, Entry};

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

fn instance_doc(entry: &Entry) -> Value {
    let r = &entry.record;
    json!({
        "name": r.name,
        "state": r.state,
        "pid": r.pid,
        "command": r.command,
        "started": r.started,
        "restarts": r.restarts,
        "log": r.log,
        "ready": r.ready,
    })
}

/// `podnet link status`.
pub(crate) fn status(name: Option<&str>, json: bool, create: bool) -> Result<()> {
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
    let identity = identity_doc(create)?;
    if json {
        println!(
            "{}",
            podnet::status::stamped(json!({
                "identity": identity,
                "links": links.iter().map(instance_doc).collect::<Vec<_>>(),
            }))
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
    for entry in &links {
        let r = &entry.record;
        let listen = r
            .ready
            .as_ref()
            .and_then(|d| d["listen"].as_str())
            .unwrap_or("-");
        println!(
            "link      {}  {}  pid {}  {}  {listen}",
            r.name, r.state, r.pid, r.command
        );
    }
    Ok(())
}

/// `podnet link close`.
pub(crate) fn close(name: &str, timeout: u64) -> Result<()> {
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
