//! The L4 door of `warp proxy --l4` (issue 140): `pod-warp`'s, with
//! podnet's variable of the form of the CONNECT request.

pub use pod_warp::l4::*;
pub use pod_warp::service::l4_sni;

/// The variable of the form of the CONNECT request (`plain`, `extended` or
/// `header`; another value is a configuration error).
pub const FORM_VARIABLE: &str = "PODNET_WARP_L4_FORM";
