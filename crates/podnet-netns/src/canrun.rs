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
    netns_probe_with(tcp_connect_refused(), probe_unshare)
}

/// [`netns_probe`] over a TCP probe and the `unshare` probes that
/// `unshare` gives (the doctor gives the rows that it already has).
pub(crate) fn netns_probe_with(
    tcp: Option<String>,
    unshare: impl Fn(libc::c_int) -> Probe,
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
    let probe = unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET);
    if probe.ok {
        return Ok(());
    }
    Err(NoNetns::Namespace(format!(
        "{}: {}",
        probe.name, probe.detail
    )))
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

    // GitHub #713, #740: one rule for the doctor and the start. A TCP
    // refusal fails the backend also where each namespace works, and the
    // line of each cause gives a next step.
    #[test]
    fn the_netns_rule_and_its_line() {
        let refused = Some("a TCP connect to 127.0.0.1 is refused: EACCES".to_string());
        // (tcp, unshare NEWNET, unshare NEWUSER|NEWNET, the start works,
        // the words of the line)
        let table = [
            (None, true, true, true, ""),
            (None, false, true, true, ""),
            (
                refused.clone(),
                true,
                true,
                false,
                "cannot carry a TCP connect here",
            ),
            (
                None,
                false,
                false,
                false,
                "an unprivileged user namespace works",
            ),
        ];
        for (tcp, net, pair, works, words) in table {
            let got = netns_probe_with(tcp.clone(), |flags| {
                row(if flags == libc::CLONE_NEWNET {
                    net
                } else {
                    pair
                })
            });
            assert_eq!(got.is_ok(), works, "{tcp:?} {net} {pair}");
            if let Err(no) = got {
                let line = no.line("netns");
                assert!(line.contains(words), "{line}");
                assert!(line.contains("use --backend seccomp"), "{line}");
            }
        }
    }
}
