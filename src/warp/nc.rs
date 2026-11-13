//! `podnet warp nc HOST PORT` and `podnet warp ssh-config` (issue 85): a byte
//! pipe from stdin and stdout to HOST:PORT, for `ssh -o ProxyCommand` and
//! git over SSH on a network that refuses port 22.
//!
//! The pipe goes through the running `podnet warp up` (SOCKS5 on its unix
//! socket), else through a WARP session of this process, else (with
//! `--via-relay`) through the WebSocket relay straight to HOST:PORT. SSH
//! checks the host key of the server as on a direct connection.

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context as _, Result};

use super::ssh_config::{proxy_command, shell_quote};
use crate::net::BoxedIo;

pub use pod_warp::nc::{target, PipeEnd, TCP_IDLE};

/// The two channels of a UDP pipe.
pub type UdpEnds = pod_warp::nc::UdpEnds;

/// Through the running `podnet warp up`, if its socket answers.
pub async fn via_running(at: &super::find::At, host: &str, port: u16) -> Result<Option<BoxedIo>> {
    let Ok(socket) = super::find::running(at) else {
        return Ok(None);
    };
    let proxy = podnet_netns::Proxy::parse(&format!("socks5://unix:{}", socket.display()))?;
    let upstream = podnet_netns::upstream::connect_upstream_async(&[proxy], &target(host, port))
        .await
        .with_context(|| format!("{host}:{port} through podnet warp up"))?;
    Ok(Some(Box::new(upstream)))
}

/// Through the WebSocket relay, with no WARP: TCP to an IPv4 address of
/// `host`.
pub async fn via_relay(relay: &crate::ws_relay::Relay, host: &str, port: u16) -> Result<BoxedIo> {
    let addrs = crate::resolve::resolve(host)
        .await
        .with_context(|| format!("resolving {host}"))?;
    let ip = addrs
        .into_iter()
        .find(|a| a.is_ipv4())
        .with_context(|| format!("{host} has no IPv4 address (the relay has IPv4 egress only)"))?;
    relay
        .connect(
            SocketAddr::new(ip, port),
            std::time::Duration::from_secs(20),
        )
        .await
}

/// Copy stdin to `io` and `io` to stdout, until the far end closes; then
/// end the process (a read of stdin blocks a thread, and the runtime would
/// wait for it).
pub async fn pipe(io: BoxedIo, end: PipeEnd) -> Result<()> {
    pipe_with(io, tokio::io::stdin(), tokio::io::stdout(), end).await?;
    std::process::exit(0);
}

/// Copy `input` to `io` and `io` to `output`, until the far end closes
/// (issue 188: no FIN at the end of the input unless `end.shutdown`).
pub async fn pipe_with<R, W>(io: BoxedIo, input: R, output: W, end: PipeEnd) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    pod_warp::nc::pipe_with(Box::new(io), input, output, end)
        .await
        .map_err(super::failed)
}

/// `podnet warp nc -u` (issue 115): each read of `input` is one datagram to
/// the far end; each datagram that comes back goes to `output`.
pub async fn udp_pipe_with<R, W>(
    out: tokio::sync::mpsc::Sender<Vec<u8>>,
    back: tokio::sync::mpsc::Receiver<Vec<u8>>,
    input: R,
    output: W,
    idle: std::time::Duration,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    Ok(pod_warp::nc::udp_pipe_with(out, back, input, output, idle).await?)
}

/// The two channels of a UDP pipe through the running `podnet warp up` (the
/// association in the stream, 0x83, on its unix socket), or `None` when it
/// does not run.
pub async fn udp_via_running(
    at: &super::find::At,
    host: &str,
    port: u16,
) -> Result<Option<UdpEnds>> {
    let Ok(socket) = super::find::running(at) else {
        return Ok(None);
    };
    // SOCKS5 carries a name of 255 bytes at most: refuse a longer one here.
    if host.parse::<std::net::IpAddr>().is_err() {
        pod_proxy::udp::wrap_name(&[], host, port)?;
    }
    let proxy = podnet_netns::Proxy::parse(&format!("socks5://unix:{}", socket.display()))?;
    let stream = podnet_netns::upstream::dial_proxy(&proxy).await?;
    let stream = podnet_netns::upstream::socks5_udp_in_stream(stream, &proxy)
        .await
        .context("a UDP association through podnet warp up")?;
    Ok(Some(
        pod_warp::nc::udp_via_stream(stream, host, port).map_err(super::failed)?,
    ))
}

/// The two channels of a UDP pipe through a WARP session of this process.
pub async fn udp_via_stack(
    stack: std::sync::Arc<super::WarpStack>,
    host: &str,
    port: u16,
) -> Result<UdpEnds> {
    let names = super::proxy::names();
    // T-332: the datagrams bind the family of the far end (see one_family).
    let ip = names
        .resolve(stack.as_ref(), host)
        .await
        .map_err(super::failed)?;
    let way = std::sync::Arc::new(super::one_family::OneFamily::to(stack, ip));
    pod_warp::nc::udp_via_net(way, &names, host, port)
        .await
        .map_err(super::failed)
}

/// The `export GIT_SSH_COMMAND=...` line (T-214): the command is one word
/// for the shell that reads the line, and the `ProxyCommand` option is one
/// word for the shell that runs `ssh`.
fn git_ssh_command(exe: &Path) -> String {
    let option = shell_quote(&format!("ProxyCommand={}", proxy_command(exe)));
    format!(
        "export GIT_SSH_COMMAND={}",
        shell_quote(&format!("ssh -o {option}"))
    )
}

/// An SSH config block and `GIT_SSH_COMMAND` that use `podnet warp nc`. The
/// `ProxyCommand` line is the one of `warp ssh-config --write`.
pub fn ssh_config(exe: &Path, host: &str) -> String {
    format!(
        "# podnet warp ssh-config: SSH through WARP (podnet warp nc)\n\
Host {host}\n    ProxyCommand {}\n\n\
# For git over SSH with no SSH config:\n\
# {}\n",
        proxy_command(exe),
        git_ssh_command(exe)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ssh_config_uses_warp_nc() {
        let text = ssh_config(Path::new("/b/podnet"), "github.com");
        assert!(text.contains("Host github.com\n    ProxyCommand '/b/podnet' warp nc %h %p\n"));
        assert!(text.contains("GIT_SSH_COMMAND"));
    }

    /// T-214: the printed `ProxyCommand` line is the line of `--write`, and
    /// the `GIT_SSH_COMMAND` line gives `sh` the same path in two steps.
    #[test]
    fn the_printed_block_quotes_the_path_as_the_written_block() {
        for exe in ["/b/podnet", "/opt/c f/podnet", "/o'q/podnet"] {
            let exe = Path::new(exe);
            let line = |text: &str| {
                text.lines()
                    .find(|l| l.trim_start().starts_with("ProxyCommand "))
                    .map(str::to_string)
            };
            let printed = ssh_config(exe, "h");
            assert_eq!(
                line(&printed),
                line(&crate::warp::ssh_config::block(exe, "h"))
            );
            // The shell reads the export line; ssh reads the option word.
            let script = format!(
                "{}; eval set -- \"$GIT_SSH_COMMAND\"; eval set -- \"${{3#ProxyCommand=}}\"; printf %s \"$1\"",
                git_ssh_command(exe)
            );
            let out = std::process::Command::new("sh")
                .args(["-c", &script])
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                exe.display().to_string()
            );
        }
    }
}
