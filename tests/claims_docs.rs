//! CLAIM-1 to CLAIM-4: the documentation of the harness and of the exit
//! contract pass the claim check, and a bare comparative fails it.

use pod_harness::check_claims;

#[test]
fn the_two_documents_pass_the_claim_check() {
    for path in [
        "docs/harness.md",
        "docs/exit-codes.md",
        "docs/host-runtime.md",
    ] {
        let text = std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let findings = check_claims(&text);
        assert!(
            findings.is_empty(),
            "{path}:
{}",
            findings
                .iter()
                .map(|finding| finding.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn a_bare_comparative_fails_and_a_conditioned_claim_passes() {
    let bare = check_claims("The tunnel is faster than the proxy.");
    assert_eq!(bare.len(), 1, "{bare:?}");
    assert_eq!(bare[0].rule, "bare-comparative");

    let conditioned = check_claims(
        "The tunnel is faster: 12 MiB/s on host lab-a, podnet 0.79.0, input 1 GiB, 3 runs, 2026-11-20.",
    );
    assert!(conditioned.is_empty(), "{conditioned:?}");
}

#[test]
fn a_measurement_without_conditions_fails() {
    let findings = check_claims("The harness takes 42 ms.");
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "no-conditions");
}
