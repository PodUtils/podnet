#[allow(unused_imports)]
use crate::prelude::*;

use super::{PROBE_NO_LOOPBACK, PROBE_NO_TUN, PROBE_VETH};

/// The variable that makes a started podnet process run one namespace probe
/// and exit (see [`probe_child_start`]). Its value is a [`ChildProbe`] name.
const CHILD_VARIABLE: &str = "PODNET_PROBE_CHILD";

/// The probes that run in a new process. They allocate (the netlink
/// requests, the error values), so they must not run in a `fork` child of a
/// process with threads: another thread can hold the allocator lock at the
/// time of the fork, and the child then waits for it for ever.
#[derive(Clone, Copy)]
pub(super) enum ChildProbe {
    /// `unshare(CLONE_NEWNET)`, then `lo`, a TUN device or a veth pair.
    Net,
    /// `unshare(CLONE_NEWUSER | CLONE_NEWNET)`, then as `Net`.
    UserNet,
    /// `unshare(CLONE_NEWNET)` or `unshare(CLONE_NEWUSER | CLONE_NEWNET)`,
    /// then `lo` and a veth pair, with no TUN.
    Veth,
}

impl ChildProbe {
    const ALL: [ChildProbe; 3] = [Self::Net, Self::UserNet, Self::Veth];

    fn name(self) -> &'static str {
        match self {
            Self::Net => "net",
            Self::UserNet => "user-net",
            Self::Veth => "veth",
        }
    }

    /// The probe code: 0, a `PROBE_*` code or an errno.
    fn run(self) -> i32 {
        let unshare = |flags| unsafe { libc::unshare(flags) } == 0;
        let entered = match self {
            Self::Net => unshare(libc::CLONE_NEWNET),
            Self::UserNet => unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET),
            Self::Veth => {
                unshare(libc::CLONE_NEWNET) || unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET)
            }
        };
        if !entered {
            return std::io::Error::last_os_error().raw_os_error().unwrap_or(1);
        }
        // A successful unshare is not sufficient. A security module (for
        // example the AppArmor setting `apparmor_restrict_unprivileged_userns`)
        // can remove the capabilities in the new user namespace. Thus the
        // probe also does what the backend does: bring up `lo` and make a
        // TUN device.
        if bringup_loopback().is_err() {
            PROBE_NO_LOOPBACK
        } else if matches!(self, Self::Veth) {
            match create_veth("pnprobe0", "pnwire0") {
                Ok(_) => 0,
                Err(_) => PROBE_NO_TUN,
            }
        } else if create_tun("pnprobe0").is_ok() {
            0
        } else if create_veth("pnprobe0", "pnwire0").is_ok() {
            // T-174: no TUN, but the veth wire works.
            PROBE_VETH
        } else {
            PROBE_NO_TUN
        }
    }
}

/// Run `probe` in a new process of this program and give its code, or -1
/// when the process cannot start or does not exit. A successful unshare
/// would otherwise take the probe process out of the host's network
/// namespace for ever.
pub(super) fn run_child_probe(probe: ChildProbe) -> i32 {
    let status = std::process::Command::new("/proc/self/exe")
        .env(CHILD_VARIABLE, probe.name())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    status.ok().and_then(|s| s.code()).unwrap_or(-1)
}

/// Runs before `main` in each process of this program (a test program
/// too). When [`CHILD_VARIABLE`] names a probe, it runs the probe and exits with
/// its code. At this time the process has one thread, so it can allocate.
#[used]
#[unsafe(link_section = ".init_array")]
static PROBE_CHILD_START: extern "C" fn() = probe_child_start;

extern "C" fn probe_child_start() {
    let Ok(name) = std::env::var(CHILD_VARIABLE) else {
        return;
    };
    if let Some(probe) = ChildProbe::ALL.into_iter().find(|p| p.name() == name) {
        let code = probe.run();
        // SAFETY: _exit(2) ends the process at once, with no destructors.
        unsafe { libc::_exit(code) };
    }
}
