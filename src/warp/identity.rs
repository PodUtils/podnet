//! The MASQUE identity of a device: `pod-warp`'s P-256 key and its
//! certificate, and podnet's class of a failed `warp license`.

pub use pod_warp::identity::*;

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
