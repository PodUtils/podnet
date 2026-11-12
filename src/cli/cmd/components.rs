//! `podnet components`: the inventory of the carriers and services of this
//! build (T-310).

use anyhow::{anyhow, Result};
use clap::Args;

use podnet::carriers::Options;
use podnet::components;

#[derive(Args, Debug)]
pub(crate) struct ComponentsArgs {
    /// Print one JSON document: {"components": [...], "features": [...],
    /// "excluded": [...], "portable": BOOL}. Each row has the fields name,
    /// kind, framing, duplex, needs_privilege, available, reason, cost,
    /// feature, built, about and address.
    #[arg(long)]
    pub(crate) json: bool,
}

/// `podnet components`.
pub(crate) fn components(args: ComponentsArgs) -> Result<()> {
    let rows = components::rows(&Options::default());
    components::check_limits(&rows).map_err(|e| anyhow!(e))?;
    if args.json {
        println!("{}", podnet::status::stamped(components::document(&rows)));
        return Ok(());
    }
    for row in &rows {
        println!("{}", row.line());
    }
    for (name, reason) in components::excluded() {
        println!("feature {name}: {reason}");
    }
    Ok(())
}
