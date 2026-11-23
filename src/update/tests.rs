use base64::Engine as _;

use super::*;

/// A shell script in the place of a podnet binary.
fn fake(version_line: &str, extra: &str) -> Vec<u8> {
    format!("#!/bin/sh\n{extra}\necho '{version_line}'\n").into_bytes()
}

/// A line of a fake binary that puts another file at its own path while it
/// runs its trial: the file at the path is not the file of the trial.
const SWAP_SELF: &str = "cp \"$0\" \"$0.swap\" && mv \"$0.swap\" \"$0\"";

fn put(path: &Path, body: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn version_of_file(path: &Path) -> String {
    let out = std::process::Command::new(path)
        .arg("--version")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect()
}

/// An update keeps the binary before as podnet.prev; a rollback puts it
/// back, and a second rollback goes forward again.
#[test]
fn install_keeps_the_binary_before_and_rollback_swaps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("podnet");
    put(&path, &fake("podnet 0.3.0-beta.4 (old)", ""));
    install(
        &path,
        &fake("podnet 0.3.0-beta.5 (new)", ""),
        "v0.3.0-beta.5",
    )
    .unwrap();
    assert_eq!(version_of_file(&path), "podnet 0.3.0-beta.5 (new)");
    assert_eq!(
        version_of_file(&previous_of(&path)),
        "podnet 0.3.0-beta.4 (old)"
    );
    assert_eq!(rollback(&path).unwrap(), "podnet 0.3.0-beta.4 (old)");
    assert_eq!(
        version_of_file(&previous_of(&path)),
        "podnet 0.3.0-beta.5 (new)"
    );
    assert_eq!(rollback(&path).unwrap(), "podnet 0.3.0-beta.5 (new)");
    let names = names(dir.path());
    assert_eq!(names.len(), 2, "no temporary file stays: {names:?}");
}

/// The kept binary of `podnet update` is the file that the supervisor of
/// a detached run starts after a failed start (`detach::previous_of`).
#[test]
fn the_kept_binary_is_the_fallback_of_the_supervisor() {
    for path in ["/usr/local/bin/podnet", "/opt/x/podnet-0.4.0", "podnet"] {
        let path = Path::new(path);
        assert_eq!(previous_of(path), crate::detach::previous_of(path));
    }
}

/// The trial refuses a binary that exits with an error, that prints
/// another version or another program, that does not answer in time, or
/// that is another file after its run (issue 236). The binary in place
/// does not change, and no file stays.
#[test]
fn a_bad_new_binary_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("podnet");
    put(&path, &fake("podnet 0.3.0-beta.4 (old)", ""));
    let cases: [(Vec<u8>, &str); 6] = [
        (
            fake("podnet 0.3.0-beta.5 (new)", "exit 3"),
            "did not run here",
        ),
        (
            fake("podnet 0.3.0-beta.5 (new)", "")
                .into_iter()
                .chain(b"exit 1\n".iter().copied())
                .collect(),
            "did not run here",
        ),
        (fake("podnet 0.2.0 (other)", ""), "not 0.3.0-beta.5"),
        (fake("podssh 0.3.0-beta.5", ""), "not podnet VERSION"),
        (fake("podnet 0.3.0-beta.5 (new)", "sleep 30"), "did not end"),
        (
            fake("podnet 0.3.0-beta.5 (new)", SWAP_SELF),
            "changed after its trial run",
        ),
    ];
    for (body, why) in cases {
        let err = install_with(&path, &body, "v0.3.0-beta.5", Duration::from_millis(500))
            .expect_err("a bad binary is refused");
        let err = format!("{err:#}");
        assert!(err.contains(why), "{why}: {err}");
        assert!(err.contains("nothing was replaced"), "{err}");
        assert_eq!(version_of_file(&path), "podnet 0.3.0-beta.4 (old)");
        assert!(!previous_of(&path).exists());
        assert_eq!(names(dir.path()), ["podnet"]);
    }
}

/// A rollback with no kept binary, with a kept binary that fails its
/// trial, or with a kept binary that is another file after its run,
/// changes nothing.
#[test]
fn rollback_needs_a_kept_binary_that_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("podnet");
    put(&path, &fake("podnet 0.3.0-beta.5 (new)", ""));
    let err = format!("{:#}", rollback(&path).unwrap_err());
    assert!(err.contains("no update kept a binary"), "{err}");
    for (extra, why) in [
        ("exit 2", "does not run here"),
        (SWAP_SELF, "changed after its trial run"),
    ] {
        put(
            &previous_of(&path),
            &fake("podnet 0.3.0-beta.4 (old)", extra),
        );
        let err = format!(
            "{:#}",
            rollback_with(&path, Duration::from_millis(500)).unwrap_err()
        );
        assert!(err.contains(why), "{why}: {err}");
        assert_eq!(version_of_file(&path), "podnet 0.3.0-beta.5 (new)");
    }
}

const SUMS_FIXTURE: &[u8] = include_bytes!("../../tests/data/v0.4.0-beta.1.SHA256SUMS");
const BUNDLE_FIXTURE: &[u8] =
    include_bytes!("../../tests/data/v0.4.0-beta.1.SHA256SUMS.sigstore.json");

/// The real files of v0.4.0-beta.1 of PodUtils/podnet: the bundle passes
/// for its tag, and fails for another tag, for a changed SHA256SUMS, for a
/// changed signature and for another repository.
#[test]
fn the_bundle_of_a_release() {
    let verified = verify(SUMS_FIXTURE, BUNDLE_FIXTURE, "v0.4.0-beta.1").unwrap();
    assert!(verified
        .sums
        .get("podnet-x86_64-unknown-linux-musl")
        .is_ok());
    assert!(verified
        .sums
        .get("podnet-sparc-unknown-linux-musl")
        .is_err());

    let err = format!(
        "{:#}",
        verify(SUMS_FIXTURE, BUNDLE_FIXTURE, "v0.4.0-beta.2").unwrap_err()
    );
    assert!(
        err.contains("checking the signature of v0.4.0-beta.2"),
        "{err}"
    );
    assert!(err.contains("not https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/v0.4.0-beta.2"), "{err}");

    let mut changed = SUMS_FIXTURE.to_vec();
    changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
    let err = format!(
        "{:#}",
        verify(&changed, BUNDLE_FIXTURE, "v0.4.0-beta.1").unwrap_err()
    );
    assert!(err.contains("signs another file"), "{err}");

    let bad = change(BUNDLE_FIXTURE, |b| {
        flip(&mut b["messageSignature"]["signature"]);
    });
    let err = format!(
        "{:#}",
        verify(SUMS_FIXTURE, &bad, "v0.4.0-beta.1").unwrap_err()
    );
    assert!(err.contains("signature"), "{err}");

    let other = Identity::github("PodUtils/other", WORKFLOW, "v0.4.0-beta.1");
    let err = pod_release::verify_release(SUMS_FIXTURE, BUNDLE_FIXTURE, &other, &Roots::sigstore())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("not https://github.com/PodUtils/other/"),
        "{err}"
    );
}

/// The bundle `bundle` with the change `how`.
fn change(bundle: &[u8], how: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut v: serde_json::Value = serde_json::from_slice(bundle).unwrap();
    how(&mut v);
    serde_json::to_vec(&v).unwrap()
}

/// One bit of the base64 text `field` changed.
fn flip(field: &mut serde_json::Value) {
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut raw = b64.decode(field.as_str().unwrap()).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 1;
    *field = b64.encode(raw).into();
}

/// The ruling of 2026-11-23: the entry of Rekor and the timestamps of the
/// bundle are checked. Each change below passed podnet's own verifier
/// before T-494, which read only the time of the first entry.
#[test]
fn the_entry_of_rekor_and_the_timestamps_are_checked() {
    type Change = fn(&mut serde_json::Value);
    let cases: [(&str, Change); 6] = [
        ("no promise and no inclusion proof", |b| {
            let entry = &mut b["verificationMaterial"]["tlogEntries"][0];
            let entry = entry.as_object_mut().unwrap();
            entry.remove("inclusionPromise");
            entry.remove("inclusionProof");
        }),
        ("a changed promise", |b| {
            flip(
                &mut b["verificationMaterial"]["tlogEntries"][0]["inclusionPromise"]
                    ["signedEntryTimestamp"],
            )
        }),
        ("a changed hash of the inclusion proof", |b| {
            flip(&mut b["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]["hashes"][0])
        }),
        ("a changed root of the inclusion proof", |b| {
            flip(&mut b["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]["rootHash"])
        }),
        ("an unknown log", |b| {
            flip(&mut b["verificationMaterial"]["tlogEntries"][0]["logId"]["keyId"])
        }),
        ("a changed timestamp", |b| {
            flip(
                &mut b["verificationMaterial"]["timestampVerificationData"]["rfc3161Timestamps"][0]
                    ["signedTimestamp"],
            )
        }),
    ];
    for (what, how) in cases {
        let bad = change(BUNDLE_FIXTURE, how);
        let err = verify(SUMS_FIXTURE, &bad, "v0.4.0-beta.1")
            .map(|_| ())
            .expect_err(what);
        assert!(
            format!("{err:#}").contains("checking the signature of v0.4.0-beta.1"),
            "{what}: {err:#}"
        );
    }
    // The bundle with no change passes, so each change above is the cause.
    verify(SUMS_FIXTURE, BUNDLE_FIXTURE, "v0.4.0-beta.1").unwrap();
}

#[test]
fn the_asset_is_the_static_binary_of_this_architecture() {
    let name = asset();
    assert!(name.starts_with("podnet-"), "{name}");
    assert!(!name.contains("-gnu"), "{name}");
    assert_eq!(
        file_url("v0.3.0-beta.9", SUMS),
        "https://github.com/PodUtils/podnet/releases/download/v0.3.0-beta.9/SHA256SUMS"
    );
    assert_eq!(
        identity("v0.3.0-beta.9").uri,
        "https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/v0.3.0-beta.9"
    );
}

// Issue 227: the form of a tag. A tag goes into the URLs and into the
// identity of the signature.
#[test]
fn a_tag_must_be_a_tag() {
    for good in ["v0.3.0-beta.9", "v1.0.0", "v0.4.0-rc.1"] {
        assert!(valid_tag(good), "{good}");
    }
    let long = format!("v{}", "1".repeat(64));
    for bad in [
        "",
        "v1/../x",
        "v1..2",
        "../v1",
        "x1.0",
        "v1 2",
        "v1%2f",
        "v1?x",
        "v1",
        "v1.2",
        "0.3.0",
        "v01.0.0",
        "v1.0.0+build.1",
        long.as_str(),
    ] {
        assert!(!valid_tag(bad), "{bad}");
    }
}

// Issue 166: the newest tag of the API and of releases.atom; a tag with a
// path is skipped (issue 227); the newest is the highest version, not the
// first of the list.
#[test]
fn the_newest_tag_of_a_list() {
    let feed = br#"<feed><entry><id>tag:github.com,2008:Repository/1/v0.4.0-beta.1</id>
<link rel="alternate" type="text/html" href="https://github.com/PodUtils/podnet/releases/tag/v0.4.0-beta.1"/></entry>
<entry><link href="https://github.com/PodUtils/podnet/releases/tag/v0.4.0-beta.9/../../x"/></entry>
<entry><link rel="alternate" href="https://github.com/PodUtils/podnet/releases/tag/v0.3.0"/></entry></feed>"#;
    let tags = version::tags_of_atom(feed).unwrap();
    assert_eq!(tags, ["v0.4.0-beta.1", "v0.3.0"]);
    let newest = |tags: &[String], pre| version::newest(tags, pre).map(|v| v.tag());
    assert_eq!(newest(&tags, true).as_deref(), Some("v0.4.0-beta.1"));
    assert_eq!(newest(&tags, false).as_deref(), Some("v0.3.0"));
    let api = br#"[{"tag_name":"v0.3.0-beta.9"},{"tag_name":"v0.3.0-beta.10"},
{"tag_name":"v0.3.0-beta.11","draft":true}]"#;
    let tags = version::tags_of_api(api).unwrap();
    assert_eq!(newest(&tags, true).as_deref(), Some("v0.3.0-beta.10"));
}

// Issue 197: the version of this binary is not installed again, except
// with force; issue 275: --force with no --version installs the same
// version again; no downgrade with no --version, on purpose with it.
#[test]
fn the_plan_of_a_release() {
    let v = |tag: &str| release_version(tag).unwrap();
    let plan = |tag, current, pinned, check, force| {
        pod_release::update::plan(&v(tag), &v(current), pinned, check, force)
    };
    let (a, b) = ("v0.4.0", "v0.5.0");
    let cases = [
        (a, a, false, false, false, Plan::UpToDate),
        (a, a, true, false, false, Plan::Same),
        (a, a, true, true, false, Plan::Same),
        (a, a, true, false, true, Plan::Install),
        (a, a, false, false, true, Plan::Install),
        (a, a, false, true, true, Plan::Available),
        (a, b, false, false, true, Plan::Newer),
        (b, a, true, true, false, Plan::Available),
        (b, a, false, false, false, Plan::Install),
        (
            "v0.3.0-beta.9",
            "v0.3.0-beta.10",
            false,
            true,
            false,
            Plan::Newer,
        ),
        (
            "v0.3.0-beta.9",
            "v0.3.0-beta.10",
            true,
            true,
            false,
            Plan::Available,
        ),
    ];
    for (tag, current, pinned, check, force, want) in cases {
        assert_eq!(
            plan(tag, current, pinned, check, force),
            want,
            "{tag} {current} pinned={pinned} check={check} force={force}"
        );
    }
}

#[test]
fn this_binary_has_a_version() {
    let current = current().unwrap();
    assert_eq!(current.tag(), format!("v{}", env!("CARGO_PKG_VERSION")));
}

// Issue 168: PODNET_REPO is not used, and a warning says so.
#[test]
fn podnet_repo_is_reported_as_ignored() {
    assert_eq!(ignored_repo_warning(None), None);
    assert_eq!(ignored_repo_warning(Some(REPOSITORY.into())), None);
    let w = ignored_repo_warning(Some("me/fork".into())).unwrap();
    assert!(
        w.contains("PODNET_REPO=me/fork") && w.contains(REPOSITORY),
        "{w}"
    );
}
