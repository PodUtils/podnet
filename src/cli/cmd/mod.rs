//! The commands of the binary: one file for each command or group.

pub(crate) mod agent;
pub(crate) mod autostart;
pub(crate) mod connect;
mod connect_stdio;
pub(crate) mod host;
pub(crate) mod instances;
pub(crate) mod net;
pub(crate) mod relay;
pub(crate) mod run;
pub(crate) mod share;
pub(crate) mod tunnel;
pub(crate) mod warp;

#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod run_tests;
#[cfg(test)]
mod share_tests;
#[cfg(test)]
mod tunnel_tests;
