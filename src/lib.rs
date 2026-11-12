//! podnet — expose anything on the public Electrosphere through Cloudflare
//! tunnels, from Rust.
//!
//! The crate is usable as a library:
//!
//! ```no_run
//! use podnet::{Service, Tunnel};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let handle = Tunnel::quick()
//!     .service(Service::parse("http://127.0.0.1:8080")?)
//!     .build()?
//!     .start()
//!     .await?;
//! println!("{:?}", handle.url());
//! handle.shutdown().await;
//! # Ok(())
//! # }
//! ```
//!
//! It speaks the Cloudflare quick-tunnel protocol natively (QUIC + Cap'n Proto
//! RPC, no `cloudflared` subprocess) and also accepts named-tunnel tokens.

/// `println!`, `print!` and `eprintln!` that drop a failed write (issue
/// 74). The std macros panic when the write fails: a detached instance
/// writes to its log, and a full disk or a closed stream must not stop it.
/// A command that only prints keeps the default action of SIGPIPE.
macro_rules! println {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stdout(), $($arg)*);
    }};
}

#[allow(unused_macros)]
macro_rules! print {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::write!(::std::io::stdout(), $($arg)*);
    }};
}

macro_rules! eprintln {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stderr(), $($arg)*);
    }};
}

// The modules of the core layer (crates/podnet-core, T-047), at their
// old paths.
#[doc(inline)]
pub use podnet_core::{
    autostart, connect, decisions, detach, exit, hop, instances, metrics, net, network, ports, qr,
    race, rlimit, services, state, trust, udp, util, yaml,
};

// The modules of the services (crates/podnet-serve, T-051), at their old
// paths.
#[doc(inline)]
pub use podnet_serve::{feature, files, gate, git, ingress, origin, relay, road, service, share};

// The userspace virtual network (crates/podnet-vnet, T-048), at its old
// path.
#[doc(inline)]
pub use podnet_vnet as vnet;

pub mod agent;
pub mod agent_files;
pub mod bug_report;
pub mod build_features;
pub mod carriers;
pub mod cloudflare;
pub mod components;
pub mod config;
#[cfg(test)]
mod core_tests;
pub mod dialers;
pub mod doctor;
pub mod edge_discovery;
pub mod edge_h2;
pub mod egress;
pub mod fetch;
#[doc(hidden)]
pub mod forwards;
pub mod fuzzing;
pub mod hello;
pub mod identity;
pub mod link;
pub mod link_alias;
pub mod link_ctl;
pub mod link_query;
pub mod link_service;
#[cfg(feature = "tcp")]
pub mod link_tcp;
pub mod manual;
pub mod networks;
pub mod preflight;
pub mod proxy;
pub mod quicktunnel;
pub mod relay_reverse;
pub mod report;
pub mod resolve;
pub mod roads;
pub mod router;
#[cfg(test)]
mod serve_tests;
pub mod session;
pub mod status;
#[cfg(test)]
#[path = "../tests/test_host/mod.rs"]
mod test_host;
pub mod transport;
pub mod tunnel;
pub mod update;
pub mod warp;
#[cfg(test)]
mod wg_quick_tests;
pub mod ws;
pub mod ws_relay;

pub use config::{Config, Credentials, Protocol, ReconnectPolicy, Transport};
pub use egress::Proxy;
pub use ingress::{ForwardTarget, Ingress, IngressRule, OriginOptions, Service};
pub use metrics::{Metrics, MetricsSnapshot};
pub use share::{ShareApp, ShareConfig, ShareControl, ShareMode};
pub use tunnel::{Tunnel, TunnelBuilder, TunnelHandle, Verified};
pub use vnet::{NetStack, VirtAddr, VirtualSubnet};
pub use warp::{Identity, MasqueSession, StackConfig, WarpClient, WarpStack};
pub use ws::LocalSpec;

/// Install the rustls crypto provider. Called automatically by
/// [`Tunnel::start`]; call it early if you build TLS yourself.
pub fn init_crypto() {
    net::init_crypto();
}
