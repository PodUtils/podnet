//! The seccomp filter of the pod. pod-hold (PodUtils/podcrates) has the
//! kernel structures, the list of the stopped system calls
//! (`Calls::sockets`) and the BPF program.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::abi::SYS_ACCEPT;

/// The calls that the filter of the pod stops. `pod-hold` 0.44.0 has two
/// parts of socket rules. The ping part (`SOCK_DGRAM` with `IPPROTO_ICMP`
/// or `IPPROTO_ICMPV6`) goes to the supervisor, which emulates each ping
/// socket (T-066: an echo to the gateway gets an answer, no packet leaves
/// the host). The raw part (`SOCK_RAW` and `SOCK_PACKET` of `AF_INET` and
/// `AF_INET6`, `AF_PACKET` and `AF_XDP`) gets `EPERM` in the kernel
/// (T-334). With `--allow-raw icmp` (`allow_raw`) the raw part goes to the
/// supervisor, which lets `SOCK_RAW` of `AF_INET` and `AF_INET6` through
/// and refuses the rest (`crate::socket::raw_refused`).
pub(crate) fn calls(allow_raw: bool) -> pod_hold::filter::Calls {
    let calls = pod_hold::filter::Calls::sockets().allow_ping();
    if allow_raw {
        calls.allow_raw()
    } else {
        calls
    }
}

/// Install the filter of the pod on this thread. Return the notification
/// descriptor. The backend starts the program with `pod_hold::spawn::start`,
/// which installs the same filter; the test of the doctor's probe calls
/// this.
#[cfg(test)]
pub(crate) fn install_notif_filter() -> Result<RawFd> {
    use std::os::fd::IntoRawFd;
    Ok(pod_hold::filter::install(&calls(false))?.into_raw_fd())
}

#[cfg(test)]
mod tests {
    /// T-350: the kernel filter refuses the raw part unless `--allow-raw
    /// icmp`; the ping part always goes to the supervisor (T-066).
    #[test]
    fn the_raw_part_is_refused_in_the_kernel_unless_allow_raw() {
        assert!(super::calls(false).denies_raw());
        assert!(!super::calls(false).denies_ping());
        assert!(!super::calls(true).denies_raw());
        assert!(!super::calls(true).denies_ping());
    }
}
