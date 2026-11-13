//! `podnet exit --stdio` (T-329): one SOCKS5 session on stdin and stdout.

use anyhow::Result;
use clap::Args;
use podnet::exit::{Fault, ResultExt as _};

#[derive(Args, Debug)]
pub(crate) struct ExitArgs {
    /// Serve one SOCKS5 session on stdin and stdout, with no listener. The
    /// way out is the way of the process (--proxy, --no-proxy, --egress,
    /// the variables). CONNECT: the stream is the session. UDP: a pipe has
    /// no UDP socket, so the client sends the command 0x83 (UDP in the
    /// stream) and each datagram, in each direction, is a frame on the
    /// stream: 2 bytes of length (big endian), then a SOCKS5 UDP datagram.
    /// UDP ASSOCIATE (command 3) is refused with the reply 0x07. UDP leaves
    /// from this host when the process has no proxy; with a proxy it is
    /// refused (reply 0x07). The run ends when the client closes stdin.
    #[arg(long)]
    stdio: bool,
}

/// Run the exit.
pub(crate) async fn exit(args: ExitArgs) -> Result<()> {
    if !args.stdio {
        return Err(anyhow::anyhow!("give --stdio")).fault(Fault::Usage);
    }
    let io = super::connect_stdio::StdioIo::new()?;
    podnet::exit_stdio::serve(
        io,
        std::sync::Arc::new(podnet::exit_stdio::ExitDial::process()),
    )
    .await
}
