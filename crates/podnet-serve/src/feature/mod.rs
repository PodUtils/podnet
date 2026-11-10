//! Operator-facing features that sit beside the data path.
//!
//! None of these move tunnelled bytes. They decide who gets in, publish the
//! counters an operator needs to know the tunnel is alive, print the public URL
//! in a form a phone can read, wire a `static:` origin into the request path,
//! and package all of that into something worth pasting into a chat.
//!
//! They live in one module because they share one constraint that shapes every
//! design decision in it: **the host can refuse to bind a TCP port**, so
//! nothing here may assume a listening socket exists, and nothing here may
//! assume an unauthenticated stranger is not on the other end of the URL.
//!
//! * [`AccessController`] enforces the gate on the client, exactly as
//!   `cloudflared --allowed-mail` does. Cloudflare is told only that
//!   authentication is *required*; the allow-list itself never leaves the
//!   machine. Nothing is read from the environment to build a policy: an
//!   unconfigured gate must fail closed rather than open.
//! * `qr` renders a URL as a terminal QR code so a tunnel can be opened on a
//!   phone without typing a hostname.
//! * `static_files` is the glue between [`crate::origin::StaticDir`] and the
//!   byte pipe, adding the headers a tunnelled asset needs and refusing the
//!   methods that have no meaning without a request body.
//! * [`ShareLink`] validates a public URL before it is printed, because a share
//!   link is the one string a user copies somewhere they cannot audit.

use podnet_core::util::http::{RequestHead, ResponseWriter};
use std::io;
use std::net::IpAddr;

/// Everything this module can fail at.
///
/// Each variant carries the context needed to act on it: which file, which
/// address, which setting. A bare `io::Error` in a tunnel is unactionable,
/// because the operator cannot tell which of four sockets it came from.
#[derive(Debug)]
pub enum FeatureError {
    /// An operating-system call failed, with a description of what was being
    /// attempted.
    Io {
        /// What podnet was trying to do, e.g. `bind /tmp/podnet-metrics.sock`.
        context: String,
        /// The underlying error.
        source: io::Error,
    },
    /// A request, allow-list or share URL that this module refuses to act on.
    Invalid(String),
    /// The QR encoder could not represent the data.
    Qr(String),
    /// The access policy is self-contradictory, or too large to evaluate.
    Policy(String),
}

impl std::fmt::Display for FeatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeatureError::Io { context, source } => write!(f, "{context}: {source}"),
            FeatureError::Invalid(m) => write!(f, "invalid: {m}"),
            FeatureError::Qr(m) => write!(f, "could not encode a QR code: {m}"),
            FeatureError::Policy(m) => write!(f, "invalid access policy: {m}"),
        }
    }
}

impl std::error::Error for FeatureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FeatureError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

mod access;
mod qr;
mod share_link;
mod static_server;
#[cfg(test)]
mod tests;

pub use access::*;
pub use qr::*;
pub use share_link::*;
pub use static_server::*;
