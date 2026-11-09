//! `podnet warp export`: a WireGuard configuration of a device of its own.

use anyhow::Result;

use super::{WarpArgs, WarpCommand};

/// Issue 148: a WireGuard device of its own; warp.json is not used. True
/// when the command is done.
pub(crate) async fn before_client(args: &WarpArgs) -> Result<bool> {
    if let WarpCommand::Export(args) = &args.command {
        podnet::warp::export::command::run(args).await?;
        return Ok(true);
    }
    Ok(false)
}
