//! The exit of a WARP session (issue 147): `pod-warp`'s trace of 1.1.1.1
//! through the tunnel, with podnet's TLS client.

use anyhow::Result;

use super::Warped as _;

pub use pod_warp::exit::{
    first_txt, parse, trace_address, trace_address_v6, trace_name, whoami_query, Exit,
    TRACE_TIMEOUT,
};

/// The exit of the session of `stack`.
pub async fn trace(stack: &super::WarpStack) -> Result<Exit> {
    pod_warp::exit::trace(stack, &super::tls_client()?)
        .await
        .warped()
}

/// The IPv6 egress address of the session of `stack` (issue 184): the
/// trace over IPv6, else the TXT record of whoami.cloudflare (class CH) at
/// the IPv6 resolver of Cloudflare.
pub async fn egress_v6(stack: &super::WarpStack) -> Result<String> {
    pod_warp::exit::egress_v6(stack, &super::tls_client()?)
        .await
        .warped()
}
