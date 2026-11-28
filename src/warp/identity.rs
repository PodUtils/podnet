//! The MASQUE identity of a device: `pod-warp`'s P-256 key and its
//! certificate, podnet's WARP+ account classifier (issue 407), and podnet's
//! class of a failed `warp license`.

pub use pod_warp::identity::{spki_from_cert, spki_from_pem, Identity};

/// `yes` when the account record of `warp status` or `warp license` is a paid
/// WARP+ account, else `no` (issue 407, T-210).
///
/// The official `cloudflare-warp` 2026.8.2115.0 client defines a closed
/// `WarpAccount` enum of four account types: `free`, `limited` (referral or
/// quota WARP+, paid only while `premium_data > 0`), `unlimited`, and `team`.
/// `premium` is also accepted for older API responses. Any other or empty
/// `account_type` fails closed to `no` unless `premium_data > 0`.
pub fn warp_plus_label(account: &serde_json::Value) -> &'static str {
    let kind = account
        .get("account_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let premium = account
        .get("premium_data")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let paid_type = matches!(kind, "unlimited" | "team" | "premium");
    if paid_type && account.get("warp_plus").and_then(|v| v.as_bool()) == Some(false) {
        tracing::debug!(
            account_type = kind,
            "WARP account reports a paid account_type with warp_plus=false"
        );
    }
    if paid_type || premium > 0 {
        "yes"
    } else {
        "no"
    }
}

/// The class of a failed `warp license` (issue 173): the API refused the
/// key (HTTP 4xx, other than 401, 403 and 429) is a usage error; else the
/// service is not available.
pub fn license_fault(err: &anyhow::Error) -> crate::exit::Fault {
    use crate::exit::Fault;
    match super::register::api_status(err) {
        Some(401 | 403 | 429) => Fault::Unavailable,
        Some(400..=499) => Fault::Usage,
        _ => Fault::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue 407 (T-210): warp_plus_label names the paid account types and
    // fails closed to "no" for limited without premium_data, empty, free,
    // and unknown types.
    #[test]
    fn warp_plus_label_names_the_paid_account_types() {
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "free", "warp_plus": true})),
            "no"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "limited"})),
            "no"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "limited", "premium_data": 0})),
            "no"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "limited", "premium_data": 10})),
            "yes"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "unlimited"})),
            "yes"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "unlimited", "warp_plus": false})),
            "yes"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "team"})),
            "yes"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "premium"})),
            "yes"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": ""})),
            "no"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "enterprise"})),
            "no"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"account_type": "enterprise", "premium_data": 10})),
            "yes"
        );
        assert_eq!(
            warp_plus_label(&serde_json::json!({"premium_data": 10})),
            "yes"
        );
    }

    // Issue 173: the classes of warp license.
    #[test]
    fn the_classes_of_warp_license() {
        use crate::exit::Fault;
        let api = |status| {
            let failure = pod_warp::api::ApiError {
                status,
                body: String::new(),
            }
            .failure("PUT /reg/{id}/account");
            super::super::failed(failure)
        };
        assert!(matches!(license_fault(&api(400)), Fault::Usage));
        assert!(matches!(license_fault(&api(403)), Fault::Unavailable));
        assert!(matches!(license_fault(&api(500)), Fault::Unavailable));
        assert!(matches!(
            license_fault(&anyhow::anyhow!("no route")),
            Fault::Unavailable
        ));
    }
}
