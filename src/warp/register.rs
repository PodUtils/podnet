//! Anonymous Cloudflare WARP registration: `pod-warp`'s calls, the device
//! record, and podnet's variable of the API.

use anyhow::Result;

pub use crate::services::warp::{
    api_base, api_second, api_version, client_version, connect_uri, masque_port, masque_sni,
    user_agent,
};
pub use pod_warp::api::ApiError;
pub use pod_warp::device::*;
pub use pod_warp::register::*;

pub use super::hints::{
    DeviceHints, DEVICE_LOCALE_VARIABLE, DEVICE_MODEL_VARIABLE, DEVICE_NAME_VARIABLE,
};

/// The environment variable with the one API base to use
/// (`https://HOST[:PORT]`).
pub const API_VARIABLE: &str = "PODNET_WARP_API";

/// The API bases to try, in order: [`API_VARIABLE`] alone when it is set,
/// else [`api_base`] and [`api_second`].
pub fn api_bases(value: Option<&str>) -> Result<Vec<String>> {
    pod_warp::api::bases(value)
        .map_err(|failure| anyhow::anyhow!("{API_VARIABLE}: {}", failure.chain()))
}

/// The HTTP status of a WARP API answer in the chain of `err`.
pub fn api_status(err: &anyhow::Error) -> Option<u16> {
    err.chain()
        .find_map(|e| e.downcast_ref::<ApiError>())
        .map(|e| e.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_api_bases() {
        assert_eq!(api_bases(None).unwrap(), [api_base(), api_second()]);
        assert_eq!(api_bases(Some(" ")).unwrap(), [api_base(), api_second()]);
        assert_eq!(
            api_bases(Some("http://127.0.0.1:8080")).unwrap(),
            ["http://127.0.0.1:8080"]
        );
        for bad in [
            "http://api.cloudflareclient.com",
            "api.cloudflareclient.com",
            "https://api.cloudflareclient.com/v0",
            "https://",
        ] {
            let err = api_bases(Some(bad)).unwrap_err().to_string();
            assert!(err.starts_with(API_VARIABLE), "{bad}: {err}");
        }
    }
}
