#[allow(unused_imports)]
use crate::prelude::*;

// ── capability probes ───────────────────────────────────────────────────────

pub(crate) struct Probe {
    pub(crate) name: &'static str,
    pub(crate) ok: bool,
    pub(crate) detail: String,
}

pub(crate) fn probe_unshare(flags: libc::c_int) -> Probe {
    // Probe in a forked child: a successful unshare would otherwise take the
    // probe process out of the host's network namespace permanently.
    //
    // A successful unshare is not sufficient. A security module (for example
    // the AppArmor setting `apparmor_restrict_unprivileged_userns`) can remove
    // the capabilities in the new user namespace. Thus the child also does
    // what the backend does: bring up `lo` and make a TUN device.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        close_inherited();
        let r = unsafe { libc::unshare(flags) };
        let code = if r != 0 {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(1)
        } else if bringup_loopback().is_err() {
            PROBE_NO_LOOPBACK
        } else if create_tun("pnprobe0").is_ok() {
            0
        } else if create_veth("pnprobe0", "pnwire0").is_ok() {
            // T-174: no TUN, but the veth wire works.
            PROBE_VETH
        } else {
            PROBE_NO_TUN
        };
        unsafe { libc::_exit(code) };
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        -1
    };
    let name = if flags == libc::CLONE_NEWNET {
        "unshare(CLONE_NEWNET)"
    } else {
        "unshare(NEWUSER|NEWNET)"
    };
    if code == 0 || code == PROBE_VETH {
        let detail = match code {
            0 => "ok",
            _ => "ok (no TUN: the veth wire)",
        };
        Probe {
            name,
            ok: true,
            detail: detail.into(),
        }
    } else {
        let detail = match code {
            PROBE_NO_LOOPBACK => "unshare ok, but lo cannot be set up (no capability)".to_string(),
            PROBE_NO_TUN => {
                "unshare ok, but no TUN device and no veth pair in the namespace".to_string()
            }
            code => format!("{}", std::io::Error::from_raw_os_error(code)),
        };
        Probe {
            name,
            ok: false,
            detail,
        }
    }
}

/// Child exit codes of [`probe_unshare`] after a successful `unshare`.
const PROBE_NO_LOOPBACK: i32 = 200;
const PROBE_NO_TUN: i32 = 201;
/// No TUN, but a veth pair works (T-174).
const PROBE_VETH: i32 = 202;

/// In a probe child: close each descriptor above stderr. A child of a
/// process with threads (a test, a daemon) holds the sockets of the other
/// threads open while it lives; a socket that the parent closes must not
/// stay open in the child. The child calls only async-signal-safe functions.
fn close_inherited() {
    // SAFETY: close_range(2) with no flags closes descriptors only.
    unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3 as libc::c_uint,
            libc::c_uint::MAX,
            0 as libc::c_uint,
        );
    }
}

/// Can the namespace make the veth wire (T-174)? The netns backend takes it
/// when the namespace cannot open `/dev/net/tun`.
pub(crate) fn probe_veth() -> Probe {
    const NAME: &str = "veth wire (no TUN)";
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        close_inherited();
        let entered = unsafe { libc::unshare(libc::CLONE_NEWNET) } == 0
            || unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } == 0;
        let code = if !entered {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(1)
        } else if bringup_loopback().is_err() {
            PROBE_NO_LOOPBACK
        } else if create_veth("pnprobe0", "pnwire0").is_err() {
            PROBE_NO_TUN
        } else {
            0
        };
        unsafe { libc::_exit(code) };
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    let code = match libc::WIFEXITED(status) {
        true => libc::WEXITSTATUS(status),
        false => -1,
    };
    let detail = match code {
        0 => "a veth pair and a packet socket in the namespace".to_string(),
        PROBE_NO_LOOPBACK => "unshare ok, but lo cannot be set up (no capability)".to_string(),
        PROBE_NO_TUN => "unshare ok, but no veth pair or no packet socket".to_string(),
        -1 => "the probe process was killed".to_string(),
        code => format!("unshare: {}", std::io::Error::from_raw_os_error(code)),
    };
    Probe {
        name: NAME,
        ok: code == 0,
        detail,
    }
}

pub(crate) fn probe_tun() -> Probe {
    let path = c"/dev/net/tun";
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return Probe {
            name: "open(/dev/net/tun)",
            ok: false,
            detail: format!("{e}"),
        };
    }
    unsafe { libc::close(fd) };
    Probe {
        name: "open(/dev/net/tun)",
        ok: true,
        detail: "ok".into(),
    }
}

pub(crate) fn probe_ptrace() -> Probe {
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        close_inherited();
        let r = unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) };
        if r != 0 {
            unsafe { libc::_exit(1) };
        }
        unsafe { libc::raise(libc::SIGSTOP) };
        unsafe { libc::_exit(0) };
    }
    // The child is our tracer's child, so its SIGSTOP is a ptrace-stop; use
    // WUNTRACED so a plain stop is still reported if TRACEME failed silently.
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
    let stopped = libc::WIFSTOPPED(status);
    let detail = if stopped {
        "descendant ptrace allowed".to_string()
    } else {
        "ptrace not permitted".to_string()
    };
    unsafe {
        libc::ptrace(libc::PTRACE_DETACH, pid, 0, 0);
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, &mut status, 0);
    }
    Probe {
        name: "PTRACE_TRACEME (not a backend)",
        ok: stopped,
        detail,
    }
}

/// Can a seccomp filter that returns `SECCOMP_RET_USER_NOTIF` be installed and
/// handed a listener fd? That is the only `LD_PRELOAD`-free interception left
/// when namespaces and ptrace are both denied.
///
/// The probe does what the backend does (`install_notif_filter`): it sets
/// `no_new_privs` first, because an unprivileged task cannot install a filter
/// without it (`EACCES`). It runs in a forked child, so the flag and the
/// filter do not stay on the doctor process.
pub(crate) fn probe_seccomp_notif() -> Probe {
    const BPF_LD_ABS_W: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const BPF_JEQ_K: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
    const BPF_RET_K: u16 = 0x06; // BPF_RET | BPF_K
    const RET_USER_NOTIF: u32 = 0x7fc0_0000;
    const RET_ALLOW: u32 = 0x7fff_0000;
    const SET_MODE_FILTER: libc::c_uint = 1;
    const FLAG_NEW_LISTENER: libc::c_uint = 8;
    const NAME: &str = "seccomp(SECCOMP_RET_USER_NOTIF)";

    let filter = [
        libc::sock_filter {
            code: BPF_LD_ABS_W,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: BPF_JEQ_K,
            jt: 0,
            jf: 1,
            k: libc::SYS_getpid as u32,
        },
        libc::sock_filter {
            code: BPF_RET_K,
            jt: 0,
            jf: 0,
            k: RET_USER_NOTIF,
        },
        libc::sock_filter {
            code: BPF_RET_K,
            jt: 0,
            jf: 0,
            k: RET_ALLOW,
        },
    ];
    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut _,
    };
    // The child does no allocation: the doctor process can have threads.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Probe {
            name: NAME,
            ok: false,
            detail: format!("fork: {}", std::io::Error::last_os_error()),
        };
    }
    if pid == 0 {
        close_inherited();
        let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(1);
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            unsafe { libc::_exit(PROBE_NO_NNP_BASE + errno().clamp(1, 50)) };
        }
        let r = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                SET_MODE_FILTER,
                FLAG_NEW_LISTENER,
                &prog as *const libc::sock_fprog,
            )
        };
        let code = if r < 0 { errno().clamp(1, 199) } else { 0 };
        unsafe { libc::_exit(code) };
    }
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        -1
    };
    match code {
        0 => Probe {
            name: NAME,
            ok: true,
            detail: "listener installed".into(),
        },
        code if code > PROBE_NO_NNP_BASE => Probe {
            name: NAME,
            ok: false,
            detail: format!(
                "prctl(PR_SET_NO_NEW_PRIVS): {}",
                std::io::Error::from_raw_os_error(code - PROBE_NO_NNP_BASE)
            ),
        },
        -1 => Probe {
            name: NAME,
            ok: false,
            detail: "the probe process was killed".into(),
        },
        code => Probe {
            name: NAME,
            ok: false,
            detail: format!("{}", std::io::Error::from_raw_os_error(code)),
        },
    }
}

/// Child exit codes of [`probe_seccomp_notif`] above this value: `prctl`
/// failed with the errno `code - PROBE_NO_NNP_BASE`.
const PROBE_NO_NNP_BASE: i32 = 200;

/// Can the supervisor write a child's memory? That is what faking
/// `getsockname`/`getpeername`/`accept` addresses needs.
/// The read of the memory of a program: the seccomp backend reads the
/// address of each `connect` and `sendto` there. With no read, no call can
/// be held (T-231).
pub(crate) fn probe_mem_read() -> Probe {
    let Some((pid, hold)) = waiting_child() else {
        return Probe {
            name: "read program memory",
            ok: false,
            detail: format!("fork: {}", std::io::Error::last_os_error()),
        };
    };
    let path = CString::new(format!("/proc/{pid}/mem")).unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    let (ok, detail) = if fd >= 0 {
        unsafe { libc::close(fd) };
        (true, "O_RDONLY on /proc/<pid>/mem".to_string())
    } else {
        let error = std::io::Error::last_os_error();
        (
            false,
            format!("{error}; the seccomp backend cannot read the address of a connect"),
        )
    };
    release(pid, hold);
    Probe {
        name: "read program memory",
        ok,
        detail,
    }
}

/// A child that waits until [`release`]: it blocks on the read end of a
/// pipe. A child that exits at once is a zombie, and the open of its
/// `/proc/<pid>/mem` then fails with `ESRCH` (T-231). The pid and the write
/// end of the pipe, or `None` when the pipe or the fork fails.
fn waiting_child() -> Option<(libc::pid_t, libc::c_int)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    // SAFETY: the child calls only close(2), close_range(2), read(2) and
    // _exit(2).
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        let mut b = [0u8; 1];
        // SAFETY: the read end of the pipe; the read returns when the
        // parent closes the write end.
        unsafe {
            libc::close(fds[1]);
            // The child holds no descriptor of the parent but its pipe: a
            // socket that the parent closes must not stay open here.
            let read_end = fds[0] as libc::c_uint;
            if read_end > 3 {
                libc::syscall(
                    libc::SYS_close_range,
                    3 as libc::c_uint,
                    read_end - 1,
                    0 as libc::c_uint,
                );
            }
            libc::syscall(
                libc::SYS_close_range,
                read_end + 1,
                libc::c_uint::MAX,
                0 as libc::c_uint,
            );
            libc::read(fds[0], b.as_mut_ptr() as *mut _, 1);
            libc::_exit(0);
        }
    }
    // SAFETY: the parent does not use the read end.
    unsafe { libc::close(fds[0]) };
    if pid < 0 {
        // SAFETY: the write end of the pipe of this function.
        unsafe { libc::close(fds[1]) };
        return None;
    }
    Some((pid, fds[1]))
}

/// End the child of [`waiting_child`] and reap it.
fn release(pid: libc::pid_t, hold: libc::c_int) {
    // SAFETY: the write end of the pipe, and a child of this process.
    unsafe {
        libc::close(hold);
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, std::ptr::null_mut(), 0);
    }
}

pub(crate) fn probe_mem_write() -> Probe {
    let Some((pid, hold)) = waiting_child() else {
        return Probe {
            name: "write program memory",
            ok: false,
            detail: format!("fork: {}", std::io::Error::last_os_error()),
        };
    };
    let path = CString::new(format!("/proc/{pid}/mem")).unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    // Issue 98: process_vm_writev is the second path of the supervisor.
    let (ok, detail) = if fd >= 0 {
        unsafe { libc::close(fd) };
        (true, "O_RDWR on /proc/<pid>/mem".to_string())
    } else {
        let open_error = std::io::Error::last_os_error();
        if crate::seccomp::process_vm_writable(pid as u32) {
            (
                true,
                format!("process_vm_writev (the open gave: {open_error})"),
            )
        } else {
            (
                false,
                format!("{open_error}; process_vm_writev is refused too: getsockname and getpeername fail with EOPNOTSUPP, QUIC clients fail, and a program that reads its own address can crash (a Go program); use --backend netns"),
            )
        }
    };
    release(pid, hold);
    Probe {
        name: "write program memory",
        ok,
        detail,
    }
}

pub(crate) fn doctor(json: bool) -> i32 {
    let probes = [
        probe_unshare(libc::CLONE_NEWNET),
        probe_unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET),
        probe_tun(),
        probe_ptrace(),
        probe_seccomp_notif(),
        probe_mem_write(),
        // T-152: a measure for the netfilter plane; no backend uses it.
        crate::nfprobe::probe_netfilter(),
        probe_mem_read(),
        probe_veth(),
    ];
    // The unshare probes make the TUN, else the veth wire (T-174).
    let netns_available = probes[0].ok || probes[1].ok;
    let seccomp_available = probes[4].ok;
    let hold = Hold::of(netns_available, seccomp_available && probes[7].ok);
    let any = netns_available || seccomp_available;
    let budget = flow_budget();

    if json {
        let mut root = serde_json::Map::new();
        let capabilities: serde_json::Map<String, serde_json::Value> = probes
            .iter()
            .map(|p| {
                (
                    p.name.to_string(),
                    serde_json::json!({ "ok": p.ok, "verdict": verdict(p.ok), "detail": p.detail }),
                )
            })
            .collect();
        // T-078: the schema and the verdict words of podnet doctor.
        root.insert("schema".into(), SCHEMA.into());
        root.insert("capabilities".into(), capabilities.into());
        root.insert(
            "verdicts".into(),
            serde_json::json!({
                "netns": verdict(netns_available),
                "seccomp": verdict(seccomp_available),
                "any": verdict(any),
            }),
        );
        root.insert("backend_netns".into(), netns_available.into());
        root.insert("backend_seccomp".into(), seccomp_available.into());
        root.insert("backend_available".into(), any.into());
        root.insert("flow_budget".into(), budget.json());
        root.insert("hold".into(), hold.json());
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Object(root))
                .unwrap_or_else(|_| "{}".to_string())
        );
        return if any { 0 } else { EXIT_NO_BACKEND };
    }

    println!("{}: capability report", crate::cli::program_name());
    for p in &probes {
        println!("  {:<34} {:<5} {}", p.name, verdict(p.ok), p.detail);
    }
    println!(
        "{}: backend netns:   {}",
        crate::cli::program_name(),
        verdict(netns_available)
    );
    println!(
        "{}: backend seccomp: {}",
        crate::cli::program_name(),
        if !seccomp_available {
            "fail"
        } else if probes[5].ok {
            "ok"
        } else {
            "ok (getsockname/getpeername: fail)"
        }
    );
    println!("{}: {}", crate::cli::program_name(), budget.line());
    for line in hold.lines() {
        println!("{}: {line}", crate::cli::program_name());
    }
    // T-149: the fidelity of UDP stops at the proxy.
    println!(
        "{}: UDP goes through the proxy: no ICMP error gets to the program \
         (no ECONNREFUSED, no EMSGSIZE of the path MTU, no IP_MTU)",
        crate::cli::program_name()
    );
    println!("{}: the host conditions that podnet handles: https://github.com/PodUtils/podnet/blob/main/docs/target-environment.md", crate::cli::program_name());
    if any {
        0
    } else {
        EXIT_NO_BACKEND
    }
}

/// The flow budget of the seccomp backend on this host (T-181).
pub(crate) struct FlowBudget {
    /// The ceiling of the tracked sockets that the pod takes by default.
    pub(crate) ceiling: usize,
    pub(crate) soft: u64,
    pub(crate) hard: u64,
}

/// The budget of a pod that starts here: the supervisor takes the hard
/// `RLIMIT_NOFILE`, and the ceiling follows it.
pub(crate) fn flow_budget() -> FlowBudget {
    let (soft, hard) = crate::fdtable::nofile();
    FlowBudget {
        ceiling: crate::fdtable::derived_ceiling(hard.max(soft)),
        soft,
        hard,
    }
}

impl FlowBudget {
    /// The row of the text report.
    pub(crate) fn line(&self) -> String {
        format!(
            "flow budget (seccomp): {} tracked sockets (RLIMIT_NOFILE soft {}, hard {}; \
             --flow-budget changes it). Above it a socket call gets EMFILE; the pod \
             counts each refusal by class (table full, socketpair failed, other) and \
             prints the counts at the exit of the program",
            self.ceiling, self.soft, self.hard
        )
    }

    /// The object of the JSON report.
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "ceiling": self.ceiling,
            "nofile_soft": self.soft,
            "nofile_hard": self.hard,
            "refusal_classes": ["table full", "socketpair failed", "other"],
        })
    }
}

/// The ways out that the seccomp backend does not hold: it holds only the
/// `AF_INET` and `AF_INET6` sockets of the kinds stream and datagram, and
/// each other socket is the kernel's.
pub const SECCOMP_NOT_HELD: &[&str] = &[
    "unix sockets",
    "netlink sockets",
    "vsock sockets",
    "raw and packet sockets (they need CAP_NET_RAW)",
    "loopback TCP (127.0.0.0/8)",
    "descriptors that the program inherits",
];

/// The ways out that the netns backend does not hold: the namespace holds
/// each IP socket of the program.
pub const NETNS_NOT_HELD: &[&str] = &[
    "unix sockets of the file system",
    "descriptors that the program inherits",
];

/// The hold of each backend on this host (T-230): `enforced` (the kernel
/// keeps the program in its network namespace), `advisory` (the supervisor
/// holds the calls that it intercepts; other calls go to the kernel) or
/// `none` (the backend cannot run here).
pub(crate) struct Hold {
    netns: &'static str,
    seccomp: &'static str,
}

impl Hold {
    /// The hold of a host where the netns backend can run (`netns`), and
    /// where the seccomp backend can run and read the memory of a program
    /// (`seccomp`).
    pub(crate) fn of(netns: bool, seccomp: bool) -> Hold {
        Hold {
            netns: if netns { "enforced" } else { "none" },
            seccomp: if seccomp { "advisory" } else { "none" },
        }
    }

    /// The object `hold` of the JSON report.
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "netns": { "level": self.netns, "not_held": NETNS_NOT_HELD },
            "seccomp": { "level": self.seccomp, "not_held": SECCOMP_NOT_HELD },
        })
    }

    /// The rows of the text report.
    pub(crate) fn lines(&self) -> [String; 2] {
        [
            format!(
                "hold netns:   {} (not held: {})",
                self.netns,
                NETNS_NOT_HELD.join(", ")
            ),
            format!(
                "hold seccomp: {} (not held: {})",
                self.seccomp,
                SECCOMP_NOT_HELD.join(", ")
            ),
        ]
    }
}

/// The version of the JSON form of `pod doctor` (T-078).
pub const SCHEMA: u32 = 1;

/// The verdict word of `podnet doctor` for a probe that passed or failed.
pub fn verdict(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "fail"
    }
}

#[cfg(test)]
mod tests {
    // T-181: the doctor row names the ceiling and the descriptor limits.
    #[test]
    fn doctor_flow_budget_row_names_the_ceiling() {
        let budget = super::flow_budget();
        assert!(budget.ceiling >= crate::fdtable::MIN_CEILING);
        let line = budget.line();
        assert!(
            line.contains(&format!("{} tracked sockets", budget.ceiling)),
            "{line}"
        );
        assert!(line.contains("RLIMIT_NOFILE"), "{line}");
        assert_eq!(budget.json()["ceiling"], budget.ceiling);
    }

    // T-231: a host where the pod cannot read the memory of a program
    // refuses the seccomp backend at the start, with the reason and the way
    // out.
    #[test]
    fn seccomp_refuses_a_run_with_no_read_of_the_memory() {
        let probe = super::Probe {
            name: "read program memory",
            ok: false,
            detail: "Permission denied (os error 13)".into(),
        };
        let err = crate::seccomp_can_hold(&probe).expect_err("no read");
        let text = format!("{err}");
        assert!(text.starts_with("the seccomp backend cannot hold this program: the pod cannot read its memory (Permission denied (os error 13))"), "{text}");
        assert!(text.contains("use --backend netns"), "{text}");
        let ok = super::Probe { ok: true, ..probe };
        assert!(crate::seccomp_can_hold(&ok).is_ok());
    }

    // T-231: the child of a probe waits, so its memory can be opened each
    // time. A child that exited at once gave ESRCH under load.
    #[test]
    fn the_memory_probes_hold_their_child() {
        for _ in 0..200 {
            let read = super::probe_mem_read();
            assert!(read.ok, "{}", read.detail);
        }
        let write = super::probe_mem_write();
        assert!(write.ok, "{}", write.detail);
    }

    // T-230: the object `hold` names the level of each backend and what it
    // does not hold.
    #[test]
    fn doctor_hold_row_names_the_level() {
        let hold = super::Hold::of(true, true);
        let json = hold.json();
        assert_eq!(json["netns"]["level"], "enforced");
        assert_eq!(json["seccomp"]["level"], "advisory");
        let keys: Vec<&String> = json["seccomp"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["level", "not_held"]);
        let not_held = json["seccomp"]["not_held"].as_array().unwrap();
        assert!(not_held.iter().any(|v| v == "loopback TCP (127.0.0.0/8)"));
        assert!(not_held.iter().any(|v| v == "unix sockets"));
        assert_eq!(json["netns"]["not_held"].as_array().unwrap().len(), 2);
        let none = super::Hold::of(false, false).json();
        assert_eq!(
            (
                none["netns"]["level"].as_str(),
                none["seccomp"]["level"].as_str()
            ),
            (Some("none"), Some("none"))
        );
        let [netns, seccomp] = hold.lines();
        assert!(
            netns.starts_with("hold netns:   enforced (not held: unix sockets of the file system"),
            "{netns}"
        );
        assert!(
            seccomp.starts_with("hold seccomp: advisory (not held: unix sockets,"),
            "{seccomp}"
        );
        // This host: the probe of the read runs and names its way.
        let read = super::probe_mem_read();
        assert_eq!(read.name, "read program memory");
        assert!(!read.detail.is_empty());
    }

    /// Issue 299: the probe must agree with the filter of the backend. A
    /// task with no `CAP_SYS_ADMIN` and `no_new_privs` clear (an unprivileged
    /// user) can install the filter of the backend, because the backend sets
    /// `no_new_privs`. Before, the probe did not set it and said `no`
    /// (`EACCES`). The test drops root in a child to make that task.
    #[test]
    fn the_seccomp_probe_agrees_with_the_backend_filter() {
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            if unsafe { libc::geteuid() } == 0 {
                // nobody: no capabilities after the change of the uid.
                unsafe {
                    libc::setresgid(65534, 65534, 65534);
                    libc::setresuid(65534, 65534, 65534);
                }
            }
            let probe = super::probe_seccomp_notif().ok;
            let backend = crate::seccomp::install_notif_filter().is_ok();
            unsafe { libc::_exit(i32::from(probe) | (i32::from(backend) << 1)) };
        }
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "the child did not exit");
        let code = libc::WEXITSTATUS(status);
        let probe = code & 1 != 0;
        let backend = code & 2 != 0;
        assert_eq!(
            probe, backend,
            "probe_seccomp_notif says {probe}, install_notif_filter says {backend}"
        );
    }
}
