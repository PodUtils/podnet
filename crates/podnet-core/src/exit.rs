//! Exit codes.
//!
//! The constants of the exit codes. Their meanings are the facts of the
//! manual (`podnet::manual::facts::exits`, `podnet man exit-status`); a test
//! of the binary compares the two. `main` maps each error to a code with
//! [`code_of`].
//!
//! The codes follow sysexits.h, so that a script can tell the usage error of
//! podnet from the failure of another program:
//!
//! * 64 (`EX_USAGE`): a bad command line, before podnet does anything.
//! * 65 (`EX_DATAERR`): input that podnet refuses: a received entry with a
//!   bad name or kind, a name that exists, data that does not agree.
//! * 66 (`EX_NOINPUT`): a file that must exist is missing: the key of the
//!   link, or its allow list.
//! * 68 (`EX_NOHOST`): the address of a peer does not resolve.
//! * 69 (`EX_UNAVAILABLE`): a service or a network path is not available:
//!   the edge, the quick-tunnel service, the WARP API or its MASQUE endpoint.
//! * 71 (`EX_OSERR`): a file of the host cannot be used: a key file with a
//!   wide mode, a link instead of a file, or a file of another user.
//! * 77 (`EX_NOPERM`): a peer that the allow list does not admit.
//! * 78 (`EX_CONFIG`): a configuration that cannot be used: a variable of the
//!   environment (for example `PODNET_EDGE`) or a configuration file.
//!
//! An error with no class exits 1. `podnet pod` and `podnet warp run` give the
//! status of the program that they run, and 3 when no backend can run it.

/// The class of a failure. Attach it to an error with [`fault`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A bad command line.
    Usage,
    /// A service or a network path is not available.
    Unavailable,
    /// A configuration that cannot be used.
    Config,
    /// A service refused for now and says when to try again (issue 210:
    /// a quick-tunnel provision with HTTP 429).
    TempFail,
    /// A wait ended at its time limit (124, as timeout(1)).
    Timeout,
    /// A file that must exist is missing (66).
    NoInput,
    /// The address of a peer does not resolve (68).
    NoHost,
    /// A file of the host cannot be used (71).
    OsError,
    /// A peer that the allow list does not admit (77).
    NoPermission,
    /// Input that podnet refuses (65).
    DataError,
}

impl Fault {
    /// The exit code of this class.
    pub fn code(self) -> i32 {
        match self {
            Fault::Usage => USAGE,
            Fault::Unavailable => UNAVAILABLE,
            Fault::Config => CONFIG,
            Fault::TempFail => TEMPFAIL,
            Fault::Timeout => TIMEOUT,
            Fault::NoInput => NO_INPUT,
            Fault::NoHost => NO_HOST,
            Fault::OsError => OS_ERROR,
            Fault::NoPermission => NO_PERM,
            Fault::DataError => DATA_ERROR,
        }
    }
}

pub const SUCCESS: i32 = 0;
pub const FAILURE: i32 = 1;
pub const NO_BACKEND: i32 = 3;
pub const USAGE: i32 = 64;
/// Input that podnet refuses (EX_DATAERR).
pub const DATA_ERROR: i32 = 65;
/// A file that must exist is missing (EX_NOINPUT).
pub const NO_INPUT: i32 = 66;
/// The address of a peer does not resolve (EX_NOHOST).
pub const NO_HOST: i32 = 68;
pub const UNAVAILABLE: i32 = 69;
/// A file of the host cannot be used (EX_OSERR).
pub const OS_ERROR: i32 = 71;
/// A peer that the allow list does not admit (EX_NOPERM).
pub const NO_PERM: i32 = 77;
pub const CONFIG: i32 = 78;
/// A refusal for now (EX_TEMPFAIL): a quick-tunnel provision with HTTP
/// 429 (issue 210), or `--detach` lost the pipe to its parent.
pub const TEMPFAIL: i32 = 75;
/// `--detach`: the instance was not ready in time (as timeout(1)).
pub const TIMEOUT: i32 = 124;

/// The exit contract of podnet#501, EXIT-1 to EXIT-3: the six codes that
/// a script depends on, with the meaning and the commands that can give
/// each. The binary help and `docs/exit-codes.md` are generated from this
/// one table.
pub const CONTRACT: [(i32, &str, &str, &str); 6] = [
    (
        USAGE,
        "usage",
        "an unknown or malformed command, flag or value",
        "each command",
    ),
    (
        NO_INPUT,
        "no-input",
        "a key, identity or allow list that must exist is missing",
        "link, send, receive, warp",
    ),
    (
        UNAVAILABLE,
        "unavailable",
        "a carrier, listener or relay disabled by policy, or not answering",
        "link, relay, init, harness",
    ),
    (
        OS_ERROR,
        "os-error",
        "identity file permissions or ownership are wrong",
        "link, serve",
    ),
    (
        NO_PERM,
        "no-perm",
        "authentication failed or the peer is not in the allow list",
        "link, send, receive, relay",
    ),
    (
        CONFIG,
        "config",
        "a schema, policy, protocol or suite mismatch",
        "init, rules, status, harness, link",
    ),
];

/// [`CONTRACT`] as the lines of a help screen or a document.
pub fn contract_table() -> String {
    let mut out = String::new();
    for (code, name, meaning, commands) in CONTRACT {
        out.push_str(&format!("  {code:>3}  {name:<12} {meaning} ({commands})\n"));
    }
    out
}

/// An error with a [`Fault`] class. Its text is the text of the error.
#[derive(Debug)]
pub struct Faulted {
    pub fault: Fault,
    inner: anyhow::Error,
}

/// The text is the outermost message of the error; the causes come through
/// `source`, so that `{:#}` and the report give the whole chain.
impl std::fmt::Display for Faulted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl std::error::Error for Faulted {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.inner.source()
    }
}

/// Give `err` the class `fault`.
pub fn fault(fault: Fault, err: anyhow::Error) -> anyhow::Error {
    anyhow::Error::new(Faulted { fault, inner: err })
}

/// Extension for `Result`: `.fault(Fault::Config)`.
pub trait ResultExt<T> {
    fn fault(self, fault: Fault) -> anyhow::Result<T>;
}

impl<T, E: Into<anyhow::Error>> ResultExt<T> for Result<T, E> {
    fn fault(self, class: Fault) -> anyhow::Result<T> {
        self.map_err(|e| fault(class, e.into()))
    }
}

/// The exit code for `err`: the class of the outermost [`Faulted`] error
/// or `pod_core::Failure` in its chain, else [`FAILURE`]. A refused
/// registration that a new start can fix
/// ([`crate::hop::EdgeRefusal::transient`]) is [`TEMPFAIL`].
pub fn code_of(err: &anyhow::Error) -> i32 {
    err.chain()
        .find_map(|e| {
            if let Some(f) = e.downcast_ref::<Faulted>() {
                return Some(f.fault.code());
            }
            if e.downcast_ref::<crate::hop::EdgeRefusal>()
                .is_some_and(|r| r.transient())
            {
                return Some(TEMPFAIL);
            }
            e.downcast_ref::<pod_core::Failure>()
                .map(|f| code_of_class(f.class))
        })
        .unwrap_or(FAILURE)
}

/// The text of `err` for one error line: the text of each cause, joined
/// with `: `. A cause whose text is in the line already is not named again
/// (GitHub #622): a failure of a crate of podcrates holds the words of its
/// causes, as `pod_core::Failure::chain` does.
pub fn text(err: &anyhow::Error) -> String {
    text_of(err.as_ref())
}

/// [`text`] of an error of another type (a failure of a crate).
pub fn text_of(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = String::new();
    let mut next = Some(err);
    while let Some(cause) = next {
        next = cause.source();
        let more = cause.to_string();
        if more.is_empty() || out.contains(&more) {
            continue;
        }
        if !out.is_empty() {
            out.push_str(": ");
        }
        out.push_str(&more);
    }
    tidy(&out)
}

/// `text` for one line (GitHub #696, #697): a run of `: `-joined parts that
/// comes again at once is said once (`A: B: B` is `A: B`), and the Debug
/// form of a name of a certificate of webpki (`DnsName("x")`,
/// `IpAddress(x)`, `UniformResourceIdentifier("x")`) is the name alone.
/// A layer that joins its cause into its own text and a crate that writes
/// a Debug form give these; this is the second path of podnet for them.
pub fn tidy(text: &str) -> String {
    let mut parts: Vec<&str> = text.split(": ").collect();
    let mut i = 0;
    while i < parts.len() {
        let again =
            (1..=(parts.len() - i) / 2).find(|&n| parts[i..i + n] == parts[i + n..i + 2 * n]);
        match again {
            Some(n) => drop(parts.drain(i + n..i + 2 * n)),
            None => i += 1,
        }
    }
    plain_names(&parts.join(": "))
}

/// `text` with each `DnsName("x")`, `UniformResourceIdentifier("x")` and
/// `IpAddress(x)` written as `x`.
fn plain_names(text: &str) -> String {
    const FORMS: [(&str, &str); 3] = [
        ("DnsName(\"", "\")"),
        ("UniformResourceIdentifier(\"", "\")"),
        ("IpAddress(", ")"),
    ];
    let mut out = text.to_string();
    for (open, close) in FORMS {
        let mut from = 0;
        while let Some(at) = out[from..].find(open).map(|a| a + from) {
            let inner = at + open.len();
            let Some(end) = out[inner..].find(close).map(|e| e + inner) else {
                break;
            };
            let name = out[inner..end].to_string();
            out.replace_range(at..end + close.len(), &name);
            from = at + name.len();
        }
    }
    out
}

/// The code of a class of a failure of a crate. A class with a code that
/// podnet does not give (76, 79, 80) exits [`FAILURE`]. `Class::Data`
/// (podcrates `v0.84.0`, T-1238) exits [`DATA_ERROR`].
pub fn code_of_class(class: pod_core::Class) -> i32 {
    match i32::from(class.exit_code()) {
        code @ (USAGE | DATA_ERROR | NO_INPUT | UNAVAILABLE | TEMPFAIL | NO_PERM | CONFIG) => code,
        _ => FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // GitHub #622: a cause that the line holds already is not named again.
    #[test]
    fn text_names_each_cause_once() {
        #[derive(Debug)]
        struct Api;
        impl std::fmt::Display for Api {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("WARP API answered HTTP 400")
            }
        }
        impl std::error::Error for Api {}
        let err = anyhow::Error::new(Api)
            .context("PUT /reg/{id}/account: WARP API answered HTTP 400")
            .context("the WARP license");
        assert_eq!(
            text(&err),
            "the WARP license: PUT /reg/{id}/account: WARP API answered HTTP 400"
        );
        assert_eq!(format!("{err:#}").matches("HTTP 400").count(), 2);
        let plain = anyhow::anyhow!("no route").context("the dial");
        assert_eq!(text(&plain), format!("{plain:#}"));
    }

    use anyhow::Context;

    /// GitHub #696, #697: the family of the words that a line says once:
    /// a cause that comes again at once, and the Debug form of a name of a
    /// certificate.
    #[test]
    fn tidy_says_each_part_once_and_no_debug_form() {
        let cases = [
            (
                "http://127.0.0.1:1: connecting to 127.0.0.1:1: Connection refused (os error 111): Connection refused (os error 111)",
                "http://127.0.0.1:1: connecting to 127.0.0.1:1: Connection refused (os error 111)",
            ),
            ("a: b: c: b: c: d", "a: b: c: d"),
            ("x: y: y: y", "x: y"),
            (
                "TLS: certificate not valid for name \"1.1.1.1\"; certificate is only valid for DnsName(\"default.domain\") or IpAddress(10.0.0.1)",
                "TLS: certificate not valid for name \"1.1.1.1\"; certificate is only valid for default.domain or 10.0.0.1",
            ),
            ("HTTP 403: Forbidden", "HTTP 403: Forbidden"),
            ("a: b: a", "a: b: a"),
            ("", ""),
        ];
        for (text, want) in cases {
            assert_eq!(tidy(text), want, "{text}");
        }
    }

    #[test]
    fn a_class_survives_context() {
        let err = fault(Fault::Config, anyhow::anyhow!("bad PODNET_EDGE"));
        let err = Err::<(), _>(err)
            .context("starting the tunnel")
            .unwrap_err();
        assert_eq!(code_of(&err), CONFIG);
        assert_eq!(format!("{err:#}"), "starting the tunnel: bad PODNET_EDGE");
        assert_eq!(code_of(&anyhow::anyhow!("plain")), FAILURE);
    }

    #[test]
    fn a_class_keeps_the_causes() {
        let cause = Err::<(), _>(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            .context("connecting to 127.0.0.1:9")
            .context("provisioning quick tunnel")
            .unwrap_err();
        let err = fault(Fault::Unavailable, cause);
        assert_eq!(
            format!("{err:#}"),
            "provisioning quick tunnel: connecting to 127.0.0.1:9: connection refused"
        );
        assert_eq!(err.chain().count(), 3);
        assert_eq!(code_of(&err), UNAVAILABLE);
    }

    #[test]
    fn a_failure_of_a_crate_gives_its_class() {
        let failure = pod_core::Failure::new(
            pod_core::Stage::EdgeRegister,
            pod_core::Class::TempFail,
            "429",
        );
        let err = Err::<(), _>(failure).context("starting").unwrap_err();
        assert_eq!(code_of(&err), TEMPFAIL);
        let failure = pod_core::Failure::new(pod_core::Stage::Quic, pod_core::Class::Protocol, "x");
        assert_eq!(code_of(&anyhow::Error::new(failure)), FAILURE);
    }

    /// Each class of `pod-core` exits a code of podnet's contract or
    /// [`FAILURE`]; a class whose code podnet gives keeps it (T-583:
    /// `Class::Data` of podcrates `v0.84.0` is 65).
    #[test]
    fn each_class_of_a_crate_has_a_code_of_podnet() {
        let ours = [
            USAGE,
            DATA_ERROR,
            NO_INPUT,
            UNAVAILABLE,
            TEMPFAIL,
            NO_PERM,
            CONFIG,
        ];
        for class in pod_core::Class::ALL {
            let raw = i32::from(class.exit_code());
            let want = if ours.contains(&raw) { raw } else { FAILURE };
            assert_eq!(code_of_class(class), want, "{class:?}");
        }
        assert_eq!(code_of_class(pod_core::Class::Data), DATA_ERROR);
    }
}
