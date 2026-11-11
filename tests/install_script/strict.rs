//! Strict checks of scripts/install.sh (issue 430).

use super::*;

// Issue 430: the flags of the downloader are part of the contract.
#[test]
fn curl_gets_https_only_flags() {
    let lab = Lab::new("flags");
    lab.curl();
    let out = lab.run(&[]);
    assert!(out.status.success(), "{}", text(&out));
    let calls = lab.curl_args();
    assert!(calls.len() >= 2, "{calls:?}");
    for c in &calls {
        assert!(c.contains("--proto =https"), "{c}");
        assert!(c.contains("--proto-redir =https"), "{c}");
    }
}

// Issue 430: a copy of the script without the --proto words shows in the
// argument log, so the flag test above fails for it.
#[test]
fn a_script_without_proto_fails_the_flag_test() {
    let lab = Lab::new("planted");
    lab.curl();
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh"))
            .unwrap();
    let planted = src
        .replace("--proto '=https' ", "")
        .replace("--proto-redir '=https' ", "");
    assert_ne!(planted, src);
    let copy = lab.root.join("install-planted.sh");
    std::fs::write(&copy, planted).unwrap();
    let out = Command::new(lab.bin().join("sh"))
        .arg(&copy)
        .env_clear()
        .env("PATH", lab.bin())
        .env("HOME", &lab.root)
        .env("PODNET_VERSION", "v9.9.9")
        .env("PODNET_INSTALL_DIR", lab.dest())
        .env("TMPDIR", lab.tmp())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let calls = lab.curl_args();
    assert!(!calls.is_empty());
    assert!(
        calls.iter().all(|c| !c.contains("--proto =https")),
        "the planted defect must show in the argument log: {calls:?}"
    );
}

// Issue 430: a hostile version stops the script before any download.
#[test]
fn hostile_versions_stop_before_any_download() {
    let hostile = [
        "v1/../x",
        "v1?x=y",
        "v1 2",
        "v1..2",
        "$(touch injected)",
        "v$(touch injected)",
        "v1\nv2",
        "v1;touch injected",
        "1/2",
        "v1234567890123456789012345678901234567890123456789012345678901234567890",
    ];
    for (i, v) in hostile.iter().enumerate() {
        let lab = Lab::new(&format!("hostile{i}"));
        lab.curl();
        let out = lab.run(&[("PODNET_VERSION", v)]);
        let t = text(&out);
        assert!(!out.status.success(), "{v:?}: {t}");
        assert!(t.contains("not a release tag"), "{v:?}: {t}");
        assert!(lab.curl_args().is_empty(), "{v:?}: {:?}", lab.curl_args());
        assert!(!lab.root.join("injected").exists());
        assert!(!lab.dest().join("injected").exists());
        assert!(!lab.dest().join("podnet").exists());
    }
}

#[test]
fn a_bad_repo_stops_before_any_download() {
    for repo in ["a", "a/b/c", "a/b?x", "/b", "a/"] {
        let lab = Lab::new("repo");
        lab.curl();
        let out = lab.run(&[("PODNET_REPO", repo)]);
        let t = text(&out);
        assert!(!out.status.success(), "{repo}: {t}");
        assert!(t.contains("OWNER/NAME"), "{repo}: {t}");
        assert!(lab.curl_args().is_empty());
    }
}

// Issue 430: one strict line for the asset in SHA256SUMS.
#[test]
fn sha256sums_needs_exactly_one_valid_line() {
    let asset = "podnet-x86_64-unknown-linux-musl";
    let lab = Lab::new("sums");
    lab.curl();
    let good = std::fs::read_to_string(lab.root.join("release/SHA256SUMS")).unwrap();
    let hash = good.split_whitespace().next().unwrap().to_owned();
    let wrong = "0".repeat(64);
    let cases: Vec<(&str, String, &str)> = vec![
        ("no line", "\n".into(), "found 0"),
        (
            "short hash",
            format!("{}  {asset}\n", &hash[..63]),
            "found 1",
        ),
        (
            "non-hex hash",
            format!("{}  {asset}\n", "z".repeat(64)),
            "found 1",
        ),
        ("extra field", format!("{hash}  {asset} x\n"), "found 0"),
        (
            "good first",
            format!("{hash}  {asset}\n{wrong}  {asset}\n"),
            "found 2",
        ),
        (
            "wrong first",
            format!("{wrong}  {asset}\n{hash}  {asset}\n"),
            "found 2",
        ),
    ];
    for (name, sums, found) in cases {
        std::fs::write(lab.root.join("release/SHA256SUMS"), sums).unwrap();
        let out = lab.run(&[]);
        let t = text(&out);
        assert!(!out.status.success(), "{name}: {t}");
        assert!(
            t.contains("exactly one valid line") && t.contains(found),
            "{name}: {t}"
        );
        assert!(!lab.dest().join("podnet").exists(), "{name}");
    }
}
