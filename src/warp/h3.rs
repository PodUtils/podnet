//! The HTTP/3 shape, the two states of a failure and the schedule of the
//! connection IDs (GitHub #486, QUIC-5, QUIC-6 and QUIC-9).
//!
//! * [`shape`] is the shape of an HTTP/3 server of this run: `common`
//!   (Cubic, the shape of a usual server) unless `--h3-shape` names another
//!   congestion profile. [`serve`] serves the shape on the QUIC stack of
//!   `pod-quic`, the command `podnet warp h3` binds it, and [`qlog`] traces
//!   its HTTP/3 events; [`line()`] reports the selected shape to `warp status`
//!   and `doctor`, which run no server.
//! * `enrolment` and `transport` name the two failures apart (QUIC-5):
//!   a provider that refused or did not answer is not a path that failed.
//! * [`rotation`] prints the schedule of the connection IDs, from
//!   `pod_quic::migrate::Schedule`: an interval that changes, because a
//!   fixed interval is a signature (QUIC-9). It is the schedule alone: no
//!   connection of podnet rotates a connection ID yet.
//! * [`family`] refuses an IPv4 target before any dial on a host with no
//!   IPv4 route (QUIC-1, PodUtils/podnet#366).

use std::sync::OnceLock;
use std::time::Duration;

pub mod qlog;
pub mod serve;

use pod_core::{Class, Failure, Stage, Target};
use pod_quic::migrate::Schedule;
use pod_warp::h3::server::ShapeLog;
use pod_warp::h3::{Shape, ShapeReport};
use pod_warp::EnrolError;

/// The shape of an HTTP/3 server of this run: a static, set one time at
/// the start.
static SHAPE: OnceLock<Shape> = OnceLock::new();

/// The shape of an HTTP/3 server of this run: [`Shape::common`] when
/// `--h3-shape` is absent. `podnet warp h3` serves it; `warp status` and
/// `doctor` report the selection without a session.
pub fn shape() -> &'static Shape {
    SHAPE.get_or_init(Shape::common)
}

/// Set the shape of an HTTP/3 server of this run from a `--h3-shape` value
/// (`common`, `cubic`, `newreno`, `bbr`). A value that is no shape is a
/// configuration failure.
pub fn set_shape(value: Option<&str>) -> Result<(), Failure> {
    let shape = match value {
        None => Shape::common(),
        Some(text) => Shape::parse(text)?,
    };
    let _ = SHAPE.set(shape);
    Ok(())
}

/// The report of the shape: the profile, the ALPN, and whether the control
/// stream and its SETTINGS were observed. `warp status` and `doctor` observe
/// no HTTP/3 session, so the two observations are `false`.
pub fn report() -> ShapeReport {
    ShapeReport::of(shape(), &ShapeLog::default())
}

/// The line of `warp status` and of `doctor`.
pub fn line() -> String {
    report().line()
}

/// The line of the control stream for a run with no HTTP/3 server.
pub fn stream_line() -> String {
    "h3-control-stream  not observed (this command runs no HTTP/3 server)".to_string()
}

/// The rotation schedule: its range and its jitter, with two draws. Two
/// commands of a run draw different intervals, and no observer of one
/// connection sees a period.
pub fn rotation() -> (Schedule, Duration, Duration) {
    let schedule = Schedule::common();
    let (a, b) = (schedule.interval_of(1), schedule.interval_of(2));
    (schedule, a, b)
}

/// The shape and the rotation of this run, for the JSON document of
/// `warp status` and of `doctor`.
pub fn doc() -> serde_json::Value {
    let report = report();
    let (schedule, a, b) = rotation();
    serde_json::json!({
        "shape": report.shape,
        "congestion": report.congestion,
        "alpn_h3": report.alpn_h3,
        "control_stream": report.control_stream,
        "settings": report.settings,
        "setting_ids": report.setting_ids,
        "cid_rotation": {
            "min_s": schedule.min.as_secs(),
            "max_s": schedule.max.as_secs(),
            "jitter": schedule.jitter,
            "sample_s": [a.as_secs(), b.as_secs()],
        },
    })
}

/// The lines of `doctor`.
pub fn doctor_lines() -> Vec<String> {
    let (schedule, a, b) = rotation();
    let report = report();
    vec![
        format!(
            "h3-shape  {} congestion={} control-stream={} settings={}",
            report.shape,
            report.congestion,
            if report.control_stream { "yes" } else { "no" },
            if report.settings { "yes" } else { "no" }
        ),
        stream_line(),
        format!(
            "h3-cid-rotation  {} to {} s, jitter {} per 1000, samples {} s and {} s",
            schedule.min.as_secs(),
            schedule.max.as_secs(),
            schedule.jitter,
            a.as_secs(),
            b.as_secs()
        ),
    ]
}

/// The failure of an enrolment, from an error of the WARP API: the status
/// of the answer gives the class, and a transport failure of the API is
/// `Unreachable`. The words are the words of an [`EnrolError`].
pub fn enrolment(err: &anyhow::Error) -> EnrolError {
    let text = format!("{err:#}");
    match crate::warp::register::api_status(err) {
        Some(429) => EnrolError::Limited {
            retry_after: None,
            detail: text,
        },
        Some(status @ 400..=499) => EnrolError::Refused {
            status,
            detail: text,
        },
        Some(status @ 500..=599) => EnrolError::Unreachable(format!("HTTP {status}: {text}")),
        _ if crate::exit::code_of(err) == crate::exit::CONFIG => EnrolError::Config(text),
        _ => EnrolError::Unreachable(text),
    }
}

/// The state line of an enrolment failure (QUIC-5).
pub fn enrolment_line(error: &EnrolError) -> String {
    format!("state       enrolment failed: {}", error.text())
}

/// The state line of a transport failure (QUIC-5).
pub fn transport_line(text: &str) -> String {
    format!("state       transport failed: {text}")
}

/// The exit class of an enrolment failure: a refused provider 77, a limit
/// or an unreachable API 69, a bad configuration 78.
pub fn enrolment_fault(error: &EnrolError) -> crate::exit::Fault {
    match error.class() {
        Class::Refused => crate::exit::Fault::NoPermission,
        Class::Config => crate::exit::Fault::Config,
        Class::Usage => crate::exit::Fault::Usage,
        Class::Timeout => crate::exit::Fault::Timeout,
        _ => crate::exit::Fault::Unavailable,
    }
}

/// Refuse an IPv4 target on a host with no IPv4 route before any dial
/// (QUIC-1, PodUtils/podnet#366). `log` keeps the decision, for a test with
/// a recorded log. `None` means the target may go.
pub fn family(has_ipv4: bool, target: &Target, log: &mut Vec<String>) -> Option<Failure> {
    let addr = match target {
        Target::Addr(addr) => *addr,
        Target::Name { .. } => {
            log.push(format!("family {target}: a name, resolved before a dial"));
            return None;
        }
    };
    log.push(format!(
        "family {addr}: target ipv4={} host ipv4={has_ipv4}",
        addr.is_ipv4()
    ));
    // A loopback target is reachable without a default route. The route
    // check exists for a remote target, and refusing 127.0.0.1 would make
    // a local stand-in unreachable on a host with no uplink.
    if addr.ip().is_loopback() {
        return None;
    }
    if addr.is_ipv4() && !has_ipv4 {
        return Some(Failure::new(
            Stage::Connect,
            Class::Unavailable,
            format!(
                "{addr}: this host has no IPv4 route, and the target is IPv4; no connection is made (PodUtils/podnet#366)"
            ),
        ));
    }
    None
}

/// Whether `door` goes through a proxy, passed explicitly or read from the
/// environment (`HTTPS_PROXY`, `ALL_PROXY`, and tool files). A proxy
/// reaches both address families without a local default route.
pub(crate) fn via_proxy(proxy: Option<&crate::egress::Proxy>, door: &pod_warp::Door) -> bool {
    if proxy.is_some() {
        return true;
    }
    match door {
        pod_warp::Door::Pinned(Target::Addr(addr)) => {
            crate::egress::proxy_for(&addr.ip().to_string(), addr.port()).is_some()
        }
        pod_warp::Door::Pinned(Target::Name { host, port }) => {
            crate::egress::proxy_for(host, *port).is_some()
        }
        pod_warp::Door::Record => {
            crate::egress::proxy_for(pod_warp::service::masque_sni(), 443).is_some()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loopback target needs no default route: the local stand-in is
    /// reachable on a host with no uplink, while a remote target is still
    /// refused by QUIC-1.
    #[test]
    fn a_loopback_target_needs_no_default_route() {
        let mut log = Vec::new();
        assert!(family(
            false,
            &Target::Addr("127.0.0.1:443".parse().unwrap()),
            &mut log
        )
        .is_none());
        assert!(family(false, &Target::Addr("[::1]:443".parse().unwrap()), &mut log).is_none());
        let remote = family(
            false,
            &Target::Addr("192.0.2.1:443".parse().unwrap()),
            &mut log,
        );
        assert!(
            remote.is_some(),
            "a remote IPv4 target without a route is refused"
        );
    }

    /// QUIC-6: the default shape is the common one, and another profile
    /// comes from `--h3-shape`. The line gives the profile and the
    /// observations.
    #[test]
    fn the_shape_line_comes_from_the_report() {
        // The default, when no command set one.
        assert_eq!(shape().name(), "common");
        let report = report();
        assert_eq!(report.congestion, "cubic");
        assert!(report.alpn_h3);
        assert_eq!(
            line(),
            "h3-shape=common congestion=cubic control-stream=no settings=no"
        );
    }

    /// A value that is no profile is a configuration failure.
    #[test]
    fn a_bad_shape_is_a_configuration_failure() {
        let error = Shape::parse("bbr4").unwrap_err();
        assert_eq!(error.class, Class::Config);
    }

    /// QUIC-9: the two draws of the schedule differ, and each one sits
    /// inside the jittered range.
    #[test]
    fn the_rotation_schedule_is_jittered() {
        let (schedule, a, b) = rotation();
        assert_eq!(schedule, Schedule::common());
        assert_ne!(a, b);
        let low = schedule.min.mul_f64(0.75);
        let high = schedule.max.mul_f64(1.25);
        for sample in [a, b] {
            assert!(sample >= low && sample <= high, "{sample:?}");
        }
        let text = doctor_lines().join("\n");
        assert!(
            text.contains("h3-cid-rotation  480 to 780 s, jitter 200"),
            "{text}"
        );
        assert!(text.contains("h3-shape  common congestion=cubic"), "{text}");
    }

    /// QUIC-6 and QUIC-9: the JSON document carries the shape and the
    /// rotation schedule.
    #[test]
    fn the_document_carries_the_shape_and_the_rotation() {
        let doc = doc();
        assert_eq!(doc["shape"], "common");
        assert_eq!(doc["congestion"], "cubic");
        assert_eq!(doc["control_stream"], false);
        assert_eq!(doc["settings"], false);
        assert_eq!(doc["cid_rotation"]["min_s"], 480);
        assert_eq!(doc["cid_rotation"]["max_s"], 780);
        assert_eq!(doc["cid_rotation"]["jitter"], 200);
        assert_eq!(doc["cid_rotation"]["sample_s"].as_array().unwrap().len(), 2);
    }

    /// QUIC-5: the two states have their own names, and their own classes.
    #[test]
    fn the_two_failures_are_named_apart() {
        let refused = EnrolError::Refused {
            status: 403,
            detail: "no".into(),
        };
        let line = enrolment_line(&refused);
        assert!(line.contains("enrolment failed"), "{line}");
        assert!(!line.contains("transport"), "{line}");
        assert_eq!(enrolment_fault(&refused), crate::exit::Fault::NoPermission);
        let limited = EnrolError::Limited {
            retry_after: None,
            detail: "1015".into(),
        };
        assert_eq!(enrolment_fault(&limited), crate::exit::Fault::Unavailable);
        let transport = transport_line("the QUIC handshake timed out");
        assert!(transport.contains("transport failed"), "{transport}");
        assert!(!transport.contains("enrolment"), "{transport}");
    }

    /// QUIC-1: an IPv4 target on a host with no IPv4 route is refused
    /// before a dial, with a recorded log. The host of the test has no
    /// dial at all: the decision alone is proved.
    #[test]
    fn an_ipv4_target_on_an_ipv6_only_host_is_refused_before_a_dial() {
        let target: Target = "203.0.113.9:443".parse().unwrap();
        let mut log = Vec::new();
        let failure = family(false, &target, &mut log).expect("a named error");
        assert_eq!(failure.class, Class::Unavailable);
        assert!(failure.text().contains("no IPv4 route"), "{failure}");
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("host ipv4=false"), "{log:?}");
        // The same target on a host with IPv4 goes.
        let mut log = Vec::new();
        assert!(family(true, &target, &mut log).is_none());
        assert!(log[0].contains("host ipv4=true"), "{log:?}");
        // An IPv6 target on an IPv6-only host goes.
        let v6: Target = "[2001:db8::1]:443".parse().unwrap();
        let mut log = Vec::new();
        assert!(family(false, &v6, &mut log).is_none());
        // A name is resolved first: the check is on the address that the
        // resolver gives.
        let name: Target = "relay.example:443".parse().unwrap();
        let mut log = Vec::new();
        assert!(family(false, &name, &mut log).is_none());
        assert!(log[0].contains("resolved before a dial"), "{log:?}");
    }
}
