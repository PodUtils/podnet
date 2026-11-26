//! The commands of the binary: one file for each command or group.

pub(crate) mod acme;
pub(crate) mod agent;
pub(crate) mod autostart;
pub(crate) mod bench;
pub(crate) mod ci;
pub(crate) mod components;
pub(crate) mod connect;
mod connect_stdio;
pub(crate) mod cover;
pub(crate) mod e2e;
pub(crate) mod exit;
pub(crate) mod expose;
pub(crate) mod forge;
pub(crate) mod forward;
pub(crate) mod git;
mod git_serve;
pub(crate) mod host;
pub(crate) mod instances;
pub(crate) mod link;
pub(crate) mod local;
pub(crate) mod net;
pub(crate) mod offline;
pub(crate) mod relay;
pub(crate) mod route;
pub(crate) mod run;
pub(crate) mod runtime;
pub(crate) mod serve;
mod serve_at;
mod serve_relay;
mod serve_users;
pub(crate) mod share;
pub(crate) mod sidecar;
pub(crate) mod ssh_relay;
pub(crate) mod transfer;
pub(crate) mod tunnel;
pub(crate) mod tunnel_inspect;
pub(crate) mod warp;
pub(crate) mod wg;

/// The key `unlock` of a ready document (#628): the path of the unlock
/// form when a PIN locks the site, else none.
pub(crate) fn unlock_key(pin: Option<&str>) -> Option<&'static str> {
    pin.map(|_| podnet::gate::UNLOCK_PATH)
}

/// The text after the PIN line (#628): how a client opens the site. `next`
/// is the path to go to after the unlock.
pub(crate) fn unlock_hint(next: &str) -> String {
    format!(
        "open the URL and enter the PIN, or POST {} with the form fields password=PIN and next={next}; the answer sets the cookie {}",
        podnet::gate::UNLOCK_PATH,
        podnet::gate::COOKIE_NAME
    )
}

#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod run_tests;
#[cfg(test)]
mod share_tests;
#[cfg(test)]
mod tunnel_tests;
