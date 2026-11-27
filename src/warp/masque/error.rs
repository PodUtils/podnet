//! Distinct MASQUE failure types for enrolment and transport (QUIC-5).

use std::fmt;

use pod_core::Failure;
use pod_warp::EnrolError;

/// A MASQUE failure split into enrolment and transport phases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MasqueError {
    /// Provider enrolment failed (registration, credentials, policy, or API refusal).
    Enrolment(String),
    /// MASQUE transport failed (socket bind, QUIC handshake, HTTP/3 stream, or datagram I/O).
    Transport(String),
}

impl MasqueError {
    /// Build an enrolment failure.
    pub fn enrolment(detail: impl Into<String>) -> Self {
        Self::Enrolment(detail.into())
    }

    /// Build a transport failure.
    pub fn transport(detail: impl Into<String>) -> Self {
        Self::Transport(detail.into())
    }

    /// Convert a `pod-warp` enrolment error into an enrolment failure.
    pub fn from_enrol_error(err: &EnrolError) -> Self {
        Self::Enrolment(err.text())
    }

    /// Convert a `pod-core` transport failure into a transport failure.
    pub fn from_transport_failure(err: &Failure) -> Self {
        Self::Transport(err.chain())
    }

    /// Return the phase name: `"enrolment"` or `"transport"`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Enrolment(_) => "enrolment",
            Self::Transport(_) => "transport",
        }
    }

    /// Return the detail message without the phase prefix.
    pub fn detail(&self) -> &str {
        match self {
            Self::Enrolment(detail) | Self::Transport(detail) => detail,
        }
    }

    /// Return true when the failure is an enrolment failure.
    pub fn is_enrolment(&self) -> bool {
        matches!(self, Self::Enrolment(_))
    }

    /// Return true when the failure is a transport failure.
    pub fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    /// Format the failure as a `warp status` state line.
    pub fn status_line(&self) -> String {
        match self {
            Self::Enrolment(detail) => format!("state       enrolment failed: {detail}"),
            Self::Transport(detail) => format!("state       transport failed: {detail}"),
        }
    }

    /// Format the failure as a `warp status --json` state value.
    pub fn status_state(&self) -> &'static str {
        match self {
            Self::Enrolment(_) => "enrolment failed",
            Self::Transport(_) => "transport failed",
        }
    }

    /// Format the failure as a JSON object for `warp status --json`.
    pub fn status_json(&self) -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "state": self.status_state(),
            "failure_kind": self.kind(),
            "detail": self.detail(),
        })
    }
}

impl fmt::Display for MasqueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enrolment(detail) => write!(f, "enrolment failed: {detail}"),
            Self::Transport(detail) => write!(f, "transport failed: {detail}"),
        }
    }
}

impl std::error::Error for MasqueError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrolment_and_transport_errors_stay_distinct() {
        let enrol = MasqueError::enrolment("HTTP 403: token rejected");
        assert!(enrol.is_enrolment());
        assert!(!enrol.is_transport());
        assert_eq!(enrol.kind(), "enrolment");
        assert_eq!(enrol.status_state(), "enrolment failed");
        assert!(enrol.status_line().contains("enrolment failed:"));
        assert!(!enrol.status_line().contains("transport"));
        assert_eq!(enrol.status_json()["failure_kind"], "enrolment");
        assert_eq!(enrol.status_json()["state"], "enrolment failed");

        let transport = MasqueError::transport("QUIC handshake timed out");
        assert!(transport.is_transport());
        assert!(!transport.is_enrolment());
        assert_eq!(transport.kind(), "transport");
        assert_eq!(transport.status_state(), "transport failed");
        assert!(transport.status_line().contains("transport failed:"));
        assert!(!transport.status_line().contains("enrolment"));
        assert_eq!(transport.status_json()["failure_kind"], "transport");
        assert_eq!(transport.status_json()["state"], "transport failed");
    }
}
