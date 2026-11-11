//! The hints of a new WARP device (issue 141), moved out of `register.rs`.

use anyhow::{bail, Result};

/// The name, the model and the locale that a new device declares (issue
/// 141). The default gives the bytes of before: model `PC`, locale
/// `en_US`, no name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceHints {
    pub name: Option<String>,
    pub model: Option<String>,
    pub locale: Option<String>,
}

/// The variables of [`DeviceHints`].
pub const DEVICE_NAME_VARIABLE: &str = "PODNET_WARP_DEVICE_NAME";
pub const DEVICE_MODEL_VARIABLE: &str = "PODNET_WARP_DEVICE_MODEL";
pub const DEVICE_LOCALE_VARIABLE: &str = "PODNET_WARP_DEVICE_LOCALE";

static HINTS: std::sync::OnceLock<DeviceHints> = std::sync::OnceLock::new();

impl DeviceHints {
    /// The hints of the variables.
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self {
            name: var(DEVICE_NAME_VARIABLE),
            model: var(DEVICE_MODEL_VARIABLE),
            locale: var(DEVICE_LOCALE_VARIABLE),
        }
    }

    /// A name of 1 to 64 printable characters, a model of 1 to 32, a
    /// locale as `ll_CC`.
    pub fn check(&self) -> Result<()> {
        let printable = |s: &str| s.chars().all(|c| !c.is_control());
        if let Some(name) = &self.name {
            if name.is_empty() || name.chars().count() > 64 || !printable(name) {
                bail!("the device name has 1 to 64 printable characters: {name:?}");
            }
        }
        if let Some(model) = &self.model {
            if model.is_empty() || model.chars().count() > 32 || !printable(model) {
                bail!("the device model has 1 to 32 printable characters: {model:?}");
            }
        }
        if let Some(locale) = &self.locale {
            let b = locale.as_bytes();
            let ok = b.len() == 5
                && b[..2].iter().all(u8::is_ascii_lowercase)
                && b[2] == b'_'
                && b[3..].iter().all(u8::is_ascii_uppercase);
            if !ok {
                bail!("the device locale is as en_US: {locale:?}");
            }
        }
        Ok(())
    }

    /// Set the hints of this process (the flags). A later call does
    /// nothing.
    pub fn set(self) -> Result<()> {
        self.check()?;
        let _ = HINTS.set(self);
        Ok(())
    }

    /// The hints of this process: the flags, else the variables.
    pub fn current() -> Self {
        HINTS.get().cloned().unwrap_or_else(Self::from_env)
    }
}
