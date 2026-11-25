//! Whether the netns backend can run here (T-113, T-382, GitHub #713 and
//! #740). One rule for `auto`, `--backend netns`, `--backend netns-kernel`
//! and `pod doctor`: the doctor and the start of a backend agree.

use crate::doctor::{probe_unshare, Probe};

/// The test hook of the netns backend (T-113): set and not empty, the
/// netns backend cannot run.
pub(crate) const TEST_NO_NETNS_VARIABLE: &str = "PODNET_NETNS_TEST_NO_NETNS";

/// Why the netns backend cannot run here.
#[derive(Clone, Debug)]
pub(crate) enum NoNetns {
    /// [`TEST_NO_NETNS_VARIABLE`] is set.
    Hook,
    /// A TCP connect of this process is refused in each namespace.
    Tcp(String),
    /// No network namespace (with or without a user namespace), or no
    /// interface in it.
    Namespace(String),
}

impl std::fmt::Display for NoNetns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoNetns::Hook => write!(f, "{TEST_NO_NETNS_VARIABLE} is set"),
            NoNetns::Tcp(why) | NoNetns::Namespace(why) => f.write_str(why),
        }
    }
}

impl NoNetns {
    /// The error line of a start of `backend` (`netns` or `netns-kernel`)
    /// that cannot run: the cause and the next step.
    pub(crate) fn line(&self, backend: &str) -> String {
        match self {
            NoNetns::Tcp(why) => format!(
                "the {backend} backend cannot carry a TCP connect here ({why}; the refusal holds \
                 in each network namespace); use --backend seccomp"
            ),
            _ => format!(
                "the {backend} backend cannot run here ({self}); use --backend seccomp, or run \
                 the pod where an unprivileged user namespace works (podnet pod doctor shows \
                 each probe)"
            ),
        }
    }
}

/// Whether the netns backend can run here: no TCP refusal, and `unshare`
/// of a network namespace (as root) or of a user and a network namespace,
/// with `lo` and a TUN (or a veth pair) in it.
pub(crate) fn netns_probe() -> Result<(), NoNetns> {
    netns_probe_with(tcp_connect_refused(), probe_unshare, probe_id_maps)
}

/// [`netns_probe`] over a TCP probe, the `unshare` probes that `unshare`
/// gives and the probe of the id maps that `maps` gives (the doctor gives
/// the rows that it already has). The id maps count only for a user
/// namespace: a network namespace alone (root) needs none.
pub(crate) fn netns_probe_with(
    tcp: Option<String>,
    unshare: impl Fn(libc::c_int) -> Probe,
    maps: impl FnOnce() -> Probe,
) -> Result<(), NoNetns> {
    if std::env::var_os(TEST_NO_NETNS_VARIABLE).is_some_and(|v| !v.is_empty()) {
        return Err(NoNetns::Hook);
    }
    if let Some(why) = tcp {
        return Err(NoNetns::Tcp(why));
    }
    if unshare(libc::CLONE_NEWNET).ok {
        return Ok(());
    }
    let mut probe = unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET);
    if probe.ok {
        // GitHub #749: the parent writes the id maps of the child in
        // /proc; a masked /proc refuses them.
        probe = maps();
        if probe.ok {
            return Ok(());
        }
    }
    Err(NoNetns::Namespace(format!(
        "{}: {}",
        probe.name, probe.detail
    )))
}

/// Whether the seccomp backend can hold a program here: the filter with
/// `SECCOMP_RET_USER_NOTIF` (`notif`), and the read of the memory of a
/// program (`mem_read`, T-231). The doctor and the start use this rule
/// (GitHub #749). Gives the reason when it cannot.
pub(crate) fn seccomp_probe_with(notif: &Probe, mem_read: &Probe) -> Result<(), String> {
    for probe in [notif, mem_read] {
        if !probe.ok {
            return Err(format!("{}: {}", probe.name, probe.detail));
        }
    }
    Ok(())
}

/// The name of the row of [`probe_id_maps`].
const ID_MAPS: &str = "write the id maps (NEWUSER)";

/// Whether this process can write the id maps of a child in a new user
/// namespace (`/proc/<pid>/setgroups`, `uid_map` and `gid_map`), as the
/// start of the netns backend does (GitHub #749). The child calls only
/// `unshare`, `write`, `read` and `_exit`.
pub(crate) fn probe_id_maps() -> Probe {
    let fail = |detail: String| Probe {
        name: ID_MAPS,
        ok: false,
        detail,
    };
    let (mut ready, mut go) = ([0; 2], [0; 2]);
    // SAFETY: pipe2(2) writes two descriptors into each array.
    if unsafe { libc::pipe2(ready.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return fail(format!("pipe: {}", std::io::Error::last_os_error()));
    }
    if unsafe { libc::pipe2(go.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        let e = std::io::Error::last_os_error();
        close_all(&ready);
        return fail(format!("pipe: {e}"));
    }
    // SAFETY: the child calls only async-signal-safe functions.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        let e = std::io::Error::last_os_error();
        close_all(&ready);
        close_all(&go);
        return fail(format!("fork: {e}"));
    }
    if pid == 0 {
        unsafe {
            // The ends of the parent: a held write end of `go` would keep
            // the read below from its end of file.
            libc::close(ready[0]);
            libc::close(go[1]);
            let byte: u8 = u8::from(libc::unshare(libc::CLONE_NEWUSER) != 0);
            libc::write(ready[1], (&byte as *const u8).cast(), 1);
            let mut wait = 0u8;
            libc::read(go[0], (&mut wait as *mut u8).cast(), 1);
            libc::_exit(0);
        }
    }
    close_all(&[ready[1], go[0]]);
    let mut byte = 2u8;
    // SAFETY: a read of one byte into `byte`.
    let n = unsafe { libc::read(ready[0], (&mut byte as *mut u8).cast(), 1) };
    let result = match (n, byte) {
        (1, 0) => crate::netns::write_id_maps(pid as u32).map_err(|e| format!("{e:#}")),
        _ => Err("unshare(CLONE_NEWUSER) of the probe child failed".to_string()),
    };
    close_all(&[ready[0], go[1]]);
    // A child of another thread that forked between the pipes and this
    // close can hold the write end of `go`: the kill ends the child also
    // then. SAFETY: a signal to our own child, then a wait for it.
    let mut status = 0;
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, &mut status, 0);
    }
    match result {
        Ok(()) => Probe {
            name: ID_MAPS,
            ok: true,
            detail: "setgroups, uid_map and gid_map of a child".into(),
        },
        Err(detail) => fail(detail),
    }
}

fn close_all(fds: &[libc::c_int]) {
    for fd in fds {
        // SAFETY: each descriptor is a pipe end of probe_id_maps.
        unsafe { libc::close(*fd) };
    }
}

/// Whether a TCP connect of this process is refused with `EACCES` or
/// `EPERM` (T-382). A Landlock domain that handles `CONNECT_TCP` (the lab
/// `bind-denied`) refuses each TCP connect of the program in each network
/// namespace, so the program cannot reach the stack of the netns backend.
/// The probe connects to `127.0.0.1:1` with no wait: a refused port or a
/// listener both mean that the connect is allowed. Gives the reason.
pub(crate) fn tcp_connect_refused() -> Option<String> {
    let refused = |e: &std::io::Error| matches!(e.raw_os_error(), Some(libc::EACCES | libc::EPERM));
    let fd = unsafe {
        libc::socket(
            libc::AF_INET,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return refused(&e).then(|| format!("a TCP socket is refused: {e}"));
    }
    let addr = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: 1u16.to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from(std::net::Ipv4Addr::LOCALHOST).to_be(),
        },
        sin_zero: [0; 8],
    };
    let rc = unsafe {
        libc::connect(
            fd,
            (&addr as *const libc::sockaddr_in).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    let e = std::io::Error::last_os_error();
    unsafe { libc::close(fd) };
    (rc != 0 && refused(&e)).then(|| format!("a TCP connect to 127.0.0.1 is refused: {e}"))
}

/// The row of `pod doctor` for [`tcp_connect_refused`].
pub(crate) fn tcp_row(refused: &Option<String>) -> Probe {
    Probe {
        name: "TCP connect (127.0.0.1)",
        ok: refused.is_none(),
        detail: refused.clone().unwrap_or_else(|| "allowed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ok: bool) -> Probe {
        Probe {
            name: "unshare",
            ok,
            detail: if ok { "ok" } else { "Operation not permitted" }.into(),
        }
    }

    // GitHub #713, #740, #749: one rule for the doctor and the start. A
    // TCP refusal fails the backend also where each namespace works; a user
    // namespace needs the id maps; the line of each cause gives a next step.
    #[test]
    fn the_netns_rule_and_its_line() {
        let refused = Some("a TCP connect to 127.0.0.1 is refused: EACCES".to_string());
        let tcp = "cannot carry a TCP connect here";
        let userns = "an unprivileged user namespace works";
        // (tcp, unshare NEWNET, unshare NEWUSER|NEWNET, the id maps, the
        // start works, the words of the line)
        let table = [
            (None, true, true, true, true, ""),
            (None, true, false, false, true, ""),
            (None, false, true, true, true, ""),
            (refused.clone(), true, true, true, false, tcp),
            (None, false, false, true, false, userns),
            (None, false, true, false, false, userns),
        ];
        for (tcp, net, pair, maps, works, words) in table {
            let got = netns_probe_with(
                tcp.clone(),
                |flags| {
                    row(if flags == libc::CLONE_NEWNET {
                        net
                    } else {
                        pair
                    })
                },
                || row(maps),
            );
            assert_eq!(got.is_ok(), works, "{tcp:?} {net} {pair} {maps}");
            if let Err(no) = got {
                let line = no.line("netns");
                assert!(line.contains(words), "{line}");
                assert!(line.contains("use --backend seccomp"), "{line}");
            }
        }
    }

    // GitHub #749: the seccomp backend needs the filter and the read of
    // the memory of a program; the reason names the probe that failed.
    #[test]
    fn the_seccomp_rule_names_the_probe() {
        let probe = |name, ok| Probe {
            name,
            ok,
            detail: "Permission denied".into(),
        };
        let table = [
            (true, true, None),
            (false, true, Some("notif")),
            (true, false, Some("read program memory")),
        ];
        for (notif, mem, want) in table {
            let got =
                seccomp_probe_with(&probe("notif", notif), &probe("read program memory", mem));
            assert_eq!(
                got.as_ref().err().map(|e| e.split(':').next().unwrap()),
                want,
                "{got:?}"
            );
        }
    }

    // GitHub #749: the probe of the id maps runs and gives one row.
    #[test]
    fn the_id_maps_probe_gives_one_row() {
        let probe = probe_id_maps();
        assert_eq!(probe.name, ID_MAPS);
        assert!(!probe.detail.is_empty());
    }
}
