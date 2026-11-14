//! The seccomp filter of the pod. pod-hold (PodUtils/podcrates) has the
//! kernel structures, the list of the stopped system calls
//! (`Calls::sockets`) and the BPF program.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::abi::SYS_ACCEPT;

/// The calls that the filter of the pod stops. `pod-hold` 0.42.0 refuses a
/// raw socket and a ping socket in the kernel by default
/// (`Calls::sockets`). The pod emulates each ping socket in the supervisor
/// (T-066: an echo to the gateway gets an answer, no packet leaves the
/// host), so the filter lets these sockets through to the supervisor
/// (`Calls::allow_raw`).
pub(crate) fn calls() -> pod_hold::filter::Calls {
    pod_hold::filter::Calls::sockets().allow_raw()
}

/// Install the filter of the pod on this thread. Return the notification
/// descriptor. The backend starts the program with `pod_hold::spawn::start`,
/// which installs the same filter; the test of the doctor's probe calls
/// this.
#[cfg(test)]
pub(crate) fn install_notif_filter() -> Result<RawFd> {
    use std::os::fd::IntoRawFd;
    Ok(pod_hold::filter::install(&calls())?.into_raw_fd())
}
