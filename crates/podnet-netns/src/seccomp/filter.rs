//! The seccomp filter of the pod. pod-hold (PodUtils/podcrates) has the
//! kernel structures, the list of the stopped system calls
//! (`Calls::sockets`) and the BPF program.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::abi::SYS_ACCEPT;

/// Install the filter of the pod on this thread. Return the notification
/// descriptor. The backend starts the program with `pod_hold::spawn::start`,
/// which installs the same filter; the test of the doctor's probe calls
/// this.
#[cfg(test)]
pub(crate) fn install_notif_filter() -> Result<RawFd> {
    use std::os::fd::IntoRawFd;
    let calls = pod_hold::filter::Calls::sockets();
    Ok(pod_hold::filter::install(&calls)?.into_raw_fd())
}
