//! `podnet link reconnect` and `podnet link rotate` (T-307): ask the control
//! socket of a link instance that dialed to make a new link to its peer.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use podnet::exit::{self, Fault};
use podnet::instances;
use podnet::link_ctl;

/// The time that the verb can take: a dial, and the end of the old link.
const LIMIT: Duration = Duration::from_secs(60);

/// Call `verb` for each link of the instance `name` that is up, or for the
/// link `only`. One line for each result: `VERB link OLD -> NEW  PEER`.
pub(crate) async fn swap(verb: &str, name: &str, only: Option<u64>) -> Result<()> {
    instances::check_name(name).map_err(|e| exit::fault(Fault::Usage, e))?;
    let socket = instances::read(&instances::lookup_dir(), name)
        .and_then(|r| r.ready)
        .and_then(|d| d["control"].as_str().map(std::path::PathBuf::from))
        .ok_or_else(|| {
            exit::fault(
                Fault::Unavailable,
                anyhow!("{name} is no link instance with a control socket"),
            )
        })?;
    let work = async {
        let token = link_ctl::read_token(&link_ctl::token_path(&socket))?;
        let mut client = link_ctl::Client::connect(&socket).await?;
        let status = client.call(&token, "status", json!({})).await?;
        let rows: Vec<&Value> = status["links"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| only.map_or(r["state"] == "up", |id| r["id"] == id))
            .collect();
        if rows.is_empty() {
            return Err(exit::fault(
                Fault::Unavailable,
                anyhow!("{name} holds no link to {verb}"),
            ));
        }
        for row in rows {
            let done = client
                .call(&token, verb, json!({"link": row["id"]}))
                .await?;
            println!(
                "{verb} link {} -> {}  {}",
                done["replaced"],
                done["link"],
                row["peer"].as_str().unwrap_or("-")
            );
        }
        Ok(())
    };
    match tokio::time::timeout(LIMIT, work).await {
        Ok(done) => done,
        Err(_) => Err(exit::fault(
            Fault::Timeout,
            anyhow!(
                "the control socket of {name} did not answer in {} s",
                LIMIT.as_secs()
            ),
        )),
    }
}
