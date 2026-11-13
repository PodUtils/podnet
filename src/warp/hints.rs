//! The hints of a new WARP device (issue 141): podnet's flags and
//! variables into `pod_warp::DeviceHints`.

use anyhow::Result;

pub use pod_warp::DeviceHints;

/// The variables of [`DeviceHints`].
pub const DEVICE_NAME_VARIABLE: &str = "PODNET_WARP_DEVICE_NAME";
pub const DEVICE_MODEL_VARIABLE: &str = "PODNET_WARP_DEVICE_MODEL";
pub const DEVICE_LOCALE_VARIABLE: &str = "PODNET_WARP_DEVICE_LOCALE";

static HINTS: std::sync::OnceLock<DeviceHints> = std::sync::OnceLock::new();

/// The hints of the variables.
pub fn from_env() -> DeviceHints {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    DeviceHints {
        name: var(DEVICE_NAME_VARIABLE),
        model: var(DEVICE_MODEL_VARIABLE),
        locale: var(DEVICE_LOCALE_VARIABLE),
    }
}

/// Set the hints of this process (the flags). A bad hint is a usage
/// error. A later call does nothing.
pub fn set(hints: DeviceHints) -> Result<()> {
    hints.check().map_err(super::failed)?;
    let _ = HINTS.set(hints);
    Ok(())
}

/// The hints of this process: the flags, else the variables.
pub fn current() -> DeviceHints {
    HINTS.get().cloned().unwrap_or_else(from_env)
}
