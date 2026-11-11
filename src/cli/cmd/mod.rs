//! The commands of the binary: one file for each command or group.

pub(crate) mod agent;
pub(crate) mod autostart;
pub(crate) mod bench;
pub(crate) mod connect;
mod connect_stdio;
pub(crate) mod expose;
pub(crate) mod forward;
pub(crate) mod git;
mod git_serve;
pub(crate) mod host;
pub(crate) mod instances;
pub(crate) mod local;
pub(crate) mod net;
pub(crate) mod relay;
pub(crate) mod route;
pub(crate) mod run;
pub(crate) mod serve;
mod serve_relay;
pub(crate) mod share;
pub(crate) mod ssh_relay;
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
