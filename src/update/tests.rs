use super::*;

/// A shell script in the place of a podnet binary.
fn fake(version_line: &str, extra: &str) -> Vec<u8> {
    format!("#!/bin/sh\n{extra}\necho '{version_line}'\n").into_bytes()
}

fn put(path: &Path, body: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn version_of(path: &Path) -> String {
    let out = std::process::Command::new(path)
        .arg("--version")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
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
    assert_eq!(version_of(&path), "podnet 0.3.0-beta.5 (new)");
    assert_eq!(version_of(&previous_of(&path)), "podnet 0.3.0-beta.4 (old)");
    assert_eq!(rollback(&path).unwrap(), "podnet 0.3.0-beta.4 (old)");
    assert_eq!(version_of(&previous_of(&path)), "podnet 0.3.0-beta.5 (new)");
    assert_eq!(rollback(&path).unwrap(), "podnet 0.3.0-beta.5 (new)");
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "no temporary file stays: {names:?}");
}

/// The trial refuses a binary that exits with an error, that prints
/// another version, or that does not answer in time. The binary in
/// place does not change, and no file stays.
#[test]
fn a_bad_new_binary_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("podnet");
    put(&path, &fake("podnet 0.3.0-beta.4 (old)", ""));
    let cases = [
        fake("podnet 0.3.0-beta.5 (new)", "exit 3"),
        fake("podnet 0.3.0-beta.5 (new)", "")
            .into_iter()
            .chain(b"exit 1\n".iter().copied())
            .collect(),
        fake("podnet 0.2.0 (other)", ""),
        fake("podnet 0.3.0-beta.5 (new)", "sleep 30"),
    ];
    for body in cases {
        let err = install_with(&path, &body, "v0.3.0-beta.5", Duration::from_millis(500))
            .expect_err("a bad binary is refused");
        assert!(format!("{err:#}").contains("new binary"), "{err:#}");
        assert_eq!(version_of(&path), "podnet 0.3.0-beta.4 (old)");
        assert!(!previous_of(&path).exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

/// Issue 236: a file that is replaced after the trial run is not
/// installed, and the binary in place does not change.
#[test]
fn a_file_swapped_after_the_trial_is_not_installed() {
    fn swap(path: &Path) {
        let other = path.with_file_name("swapped");
        put(&other, &fake("podnet 0.3.0-beta.5 (swapped)", ""));
        std::fs::rename(&other, path).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("podnet");
    put(&path, &fake("podnet 0.3.0-beta.4 (old)", ""));
    AFTER_TRIAL.with(|h| h.set(Some(swap)));
    let err = install(
        &path,
        &fake("podnet 0.3.0-beta.5 (new)", ""),
        "v0.3.0-beta.5",
    )
    .expect_err("a swapped file is refused");
    AFTER_TRIAL.with(|h| h.set(None));
    assert!(
        format!("{err:#}").contains("changed after its trial"),
        "{err:#}"
    );
    assert_eq!(version_of(&path), "podnet 0.3.0-beta.4 (old)");
    assert!(!previous_of(&path).exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

    // The kept binary of a rollback is checked the same way.
    put(&previous_of(&path), &fake("podnet 0.3.0-beta.3 (kept)", ""));
    AFTER_TRIAL.with(|h| h.set(Some(swap)));
    let err = rollback(&path).expect_err("a swapped kept binary is refused");
    AFTER_TRIAL.with(|h| h.set(None));
    assert!(
        format!("{err:#}").contains("changed after its trial"),
        "{err:#}"
    );
    assert_eq!(version_of(&path), "podnet 0.3.0-beta.4 (old)");
}

/// A rollback with no kept binary, or with a kept binary that fails its
/// trial, changes nothing.
#[test]
fn rollback_needs_a_kept_binary_that_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("podnet");
    put(&path, &fake("podnet 0.3.0-beta.5 (new)", ""));
    assert!(format!("{:#}", rollback(&path).unwrap_err()).contains("no kept binary"));
    put(
        &previous_of(&path),
        &fake("podnet 0.3.0-beta.4 (old)", "exit 2"),
    );
    assert!(rollback_with(&path, Duration::from_millis(500)).is_err());
    assert_eq!(version_of(&path), "podnet 0.3.0-beta.5 (new)");
}

/// The repository of the releases up to v0.3.0-beta.12 (the fixture).
const OLD: &str = "Azathothas/cfrs";
const SUMS: &[u8] = include_bytes!("../../tests/data/v0.3.0-beta.1.SHA256SUMS");
const BUNDLE: &[u8] = include_bytes!("../../tests/data/v0.3.0-beta.1.SHA256SUMS.sigstore.json");

const PODNET_SUMS: &[u8] = include_bytes!("../../tests/data/v0.4.0-beta.1.SHA256SUMS");
const PODNET_BUNDLE: &[u8] =
    include_bytes!("../../tests/data/v0.4.0-beta.1.SHA256SUMS.sigstore.json");

/// The real files of v0.4.0-beta.1 of PodUtils/podnet: the bundle passes
/// on the default path for its tag, and fails for another tag, for a
/// changed SHA256SUMS and for the cfrs identity.
#[test]
fn the_bundle_of_a_podnet_release() {
    verify_bundle(PODNET_BUNDLE, PODNET_SUMS, "v0.4.0-beta.1").unwrap();

    let err = verify_bundle(PODNET_BUNDLE, PODNET_SUMS, "v0.4.0-beta.2")
        .unwrap_err()
        .to_string();
    assert!(err.contains("not https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/v0.4.0-beta.2"), "{err}");

    let mut changed = PODNET_SUMS.to_vec();
    changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
    let err = verify_bundle(PODNET_BUNDLE, &changed, "v0.4.0-beta.1")
        .unwrap_err()
        .to_string();
    assert!(err.contains("digest"), "{err}");

    let err = verify_bundle_for(
        PODNET_BUNDLE,
        PODNET_SUMS,
        &identity_in(OLD, "v0.4.0-beta.1"),
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("not https://github.com/Azathothas/cfrs/"),
        "{err}"
    );
}

/// The real files of v0.3.0-beta.1: the bundle passes for its tag, and
/// fails for another tag, for a changed SHA256SUMS and for a changed
/// signature.
#[test]
fn the_bundle_of_a_release() {
    verify_bundle_for(BUNDLE, SUMS, &identity_in(OLD, "v0.3.0-beta.1")).unwrap();

    let err = verify_bundle_for(BUNDLE, SUMS, &identity_in(OLD, "v0.3.0-beta.2"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("not https://github.com/Azathothas/cfrs/.github/workflows/release.yml@refs/tags/v0.3.0-beta.2"), "{err}");

    let mut changed = SUMS.to_vec();
    changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
    let err = verify_bundle_for(BUNDLE, &changed, &identity_in(OLD, "v0.3.0-beta.1"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("digest"), "{err}");

    let text = std::str::from_utf8(BUNDLE).unwrap();
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    let sig = v["messageSignature"]["signature"].as_str().unwrap();
    let mut raw = base64::engine::general_purpose::STANDARD
        .decode(sig)
        .unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 1;
    let bad = text.replace(sig, &base64::engine::general_purpose::STANDARD.encode(raw));
    let err = verify_bundle_for(bad.as_bytes(), SUMS, &identity_in(OLD, "v0.3.0-beta.1"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("signature"), "{err}");

    // The identity of this repository refuses a release of the old one.
    let err = verify_bundle(BUNDLE, SUMS, "v0.3.0-beta.1")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("not https://github.com/PodUtils/podnet/.github"),
        "{err}"
    );
}

#[test]
fn a_glibc_build_takes_the_musl_asset() {
    assert_eq!(
        asset_target("x86_64-unknown-linux-gnu"),
        "x86_64-unknown-linux-musl"
    );
    assert_eq!(
        asset_target("armv7-unknown-linux-gnueabihf"),
        "armv7-unknown-linux-musleabihf"
    );
    assert_eq!(
        asset_target("riscv64gc-unknown-linux-musl"),
        "riscv64gc-unknown-linux-musl"
    );
}

#[test]
fn a_line_of_sha256sums() {
    let sum = sum_of(SUMS, "cfrs-x86_64-unknown-linux-musl").unwrap();
    assert_eq!(sum.len(), 32);
    assert!(sum_of(SUMS, "cfrs-sparc-unknown-linux-musl").is_err());
}

#[test]
fn sha256sums_must_have_exactly_one_valid_line() {
    let good = "a".repeat(64);
    let bad = "b".repeat(64);
    let name = "podnet-x";
    let cases: Vec<(String, bool)> = vec![
        (String::new(), false),
        (format!("{good}  other\n"), false),
        (format!("{}  {name}\n", "a".repeat(63)), false),
        (format!("{}  {name}\n", "g".repeat(64)), false),
        (format!("{good}  {name} extra\n"), false),
        (format!("{good}  {name}\n{bad}  {name}\n"), false),
        (format!("{bad}  {name}\n{good}  {name}\n"), false),
        (format!("{good}  {name}\n{good}  {name}\n"), false),
        (format!("{good}  {name}\n"), true),
        (format!("{good} *{name}\n"), true),
    ];
    for (sums, ok) in cases {
        let r = sum_of(sums.as_bytes(), name);
        assert_eq!(r.is_ok(), ok, "{sums:?}: {r:?}");
        if !ok && sums.contains(name) {
            let e = format!("{:#}", r.unwrap_err());
            assert!(e.contains("exactly one valid line"), "{e}");
        }
    }
}

#[test]
fn the_choice_of_a_release() {
    let r = |tag: &str, pre: bool| Release {
        tag_name: tag.into(),
        prerelease: pre,
        draft: false,
        assets: Vec::new(),
    };
    let list = vec![
        r("v0.4.0-beta.1", true),
        r("v0.3.0", false),
        r("v0.3.0-beta.1", true),
    ];
    assert_eq!(choose(&list, None, "v0.3.0").unwrap().tag_name, "v0.3.0");
    assert_eq!(
        choose(&list, None, "v0.3.0-beta.1").unwrap().tag_name,
        "v0.4.0-beta.1"
    );
    assert_eq!(
        choose(&list, Some("v0.3.0-beta.1"), "v0.3.0")
            .unwrap()
            .tag_name,
        "v0.3.0-beta.1"
    );
    assert!(choose(&list, Some("v9"), "v0.3.0").is_err());
}

// Issue 166: the files of a pinned tag, with no API.
#[test]
fn a_pinned_release_has_the_three_files() {
    let r = Release::pinned("v0.3.0-beta.9");
    assert!(r.prerelease);
    let urls: Vec<&str> = r
        .assets
        .iter()
        .map(|a| a.browser_download_url.as_str())
        .collect();
    assert_eq!(urls.len(), 3);
    for url in urls {
        assert!(
            url.starts_with("https://github.com/PodUtils/podnet/releases/download/v0.3.0-beta.9/"),
            "{url}"
        );
    }
    assert!(r.asset("SHA256SUMS.sigstore.json").is_ok());
    assert!(r.asset(&format!("podnet-{}", asset_target(TARGET))).is_ok());
}

// Issue 227: a tag with a path is not a tag.
#[test]
fn atom_tags_refuses_a_path_in_a_tag() {
    let feed = r#"<link href="https://github.com/PodUtils/podnet/releases/tag/v0.3.0-beta.9/../../x"/>
<link href="https://github.com/PodUtils/podnet/releases/tag/v0.3.0-beta.8"/>"#;
    assert_eq!(atom_tags(feed), ["v0.3.0-beta.8"]);
    assert_eq!(
        newest_in_atom(feed, "v0.3.0-beta.1").as_deref(),
        Some("v0.3.0-beta.8")
    );
}

// Issue 227: the form of a tag.
#[test]
fn a_tag_must_be_a_tag() {
    for good in ["v0.3.0-beta.9", "v1.0.0", "0.3.0", "v1.0.0+build.1"] {
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
        long.as_str(),
    ] {
        assert!(!valid_tag(bad), "{bad}");
    }
    let r = |tag: &str| Release {
        tag_name: tag.into(),
        prerelease: false,
        draft: false,
        assets: Vec::new(),
    };
    let list = vec![r("v9.0.0/../x"), r("v1.0.0")];
    assert_eq!(choose(&list, None, "v0.1.0").unwrap().tag_name, "v1.0.0");
}

// Issue 166: the newest tag of releases.atom.
#[test]
fn the_newest_tag_of_releases_atom() {
    let feed = r#"<feed><entry><id>tag:github.com,2008:Repository/1/v0.4.0-beta.1</id>
<link rel="alternate" type="text/html" href="https://github.com/PodUtils/podnet/releases/tag/v0.4.0-beta.1"/></entry>
<entry><link rel="alternate" href="https://github.com/PodUtils/podnet/releases/tag/v0.3.0"/></entry></feed>"#;
    assert_eq!(atom_tags(feed), ["v0.4.0-beta.1", "v0.3.0"]);
    assert_eq!(
        newest_in_atom(feed, "v0.3.0-beta.9").as_deref(),
        Some("v0.4.0-beta.1")
    );
    assert_eq!(newest_in_atom(feed, "v0.2.0").as_deref(), Some("v0.3.0"));
    assert_eq!(newest_in_atom("<feed/>", "v0.2.0"), None);
}

// The API lists by tag name: beta.10 after beta.7. The newest is the
// highest version, and a release is after its prereleases.
#[test]
fn the_newest_is_the_highest_version() {
    use std::cmp::Ordering::*;
    assert_eq!(version_order("v0.3.0-beta.10", "v0.3.0-beta.9"), Greater);
    assert_eq!(version_order("v0.3.0", "v0.3.0-beta.10"), Greater);
    assert_eq!(version_order("v0.3.0-rc.1", "v0.3.0-beta.10"), Greater);
    assert_eq!(version_order("v0.4.0-beta.1", "v0.3.0"), Greater);
    assert_eq!(version_order("v0.3.0-beta.9", "v0.3.0-beta.9"), Equal);
    let r = |tag: &str| Release {
        tag_name: tag.into(),
        prerelease: tag.contains('-'),
        draft: false,
        assets: Vec::new(),
    };
    let list = vec![
        r("v0.3.0-beta.9"),
        r("v0.3.0-beta.8"),
        r("v0.3.0-beta.7"),
        r("v0.3.0-beta.10"),
    ];
    assert_eq!(
        choose(&list, None, "v0.3.0-beta.10").unwrap().tag_name,
        "v0.3.0-beta.10"
    );
    let feed =
        "<a href=\"/releases/tag/v0.3.0-beta.9\"/><a href=\"/releases/tag/v0.3.0-beta.10\"/>";
    assert_eq!(
        newest_in_atom(feed, "v0.3.0-beta.9").as_deref(),
        Some("v0.3.0-beta.10")
    );
}

// Issue 197: the version of this binary is not installed again, except
// with force.
#[test]
fn the_same_tag_is_not_installed_again() {
    assert_eq!(plan("v1", "v1", false, false, false), Plan::UpToDate);
    assert_eq!(plan("v1", "v1", true, false, false), Plan::Same);
    assert_eq!(plan("v1", "v1", true, true, false), Plan::Same);
    assert_eq!(plan("v1", "v1", true, false, true), Plan::Install);
    // Issue 275: --force with no --version installs the same version
    // again, and --check says that it is available. No downgrade.
    assert_eq!(plan("v1", "v1", false, false, true), Plan::Install);
    assert_eq!(plan("v1", "v1", false, true, true), Plan::Available);
    assert_eq!(plan("v0", "v1", false, false, true), Plan::UpToDate);
    assert_eq!(plan("v2", "v1", true, true, false), Plan::Available);
    assert_eq!(plan("v2", "v1", false, false, false), Plan::Install);
    // No downgrade with no --version; on purpose with it.
    assert_eq!(
        plan("v0.3.0-beta.9", "v0.3.0-beta.10", false, true, false),
        Plan::UpToDate
    );
    assert_eq!(
        plan("v0.3.0-beta.9", "v0.3.0-beta.10", true, true, false),
        Plan::Available
    );
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
