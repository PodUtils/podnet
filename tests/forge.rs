//! FORGE-1: a repository identity is a key, not a hostname. `podnet forge
//! clone` reads a local seed, clones it with no forge and no network, and
//! verifies the checkout hash. A tampered seed is refused by name.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A local seed with one commit, and the hash of that commit.
fn make_seed(dir: &Path) -> String {
    let seed = dir.join("seed");
    std::fs::create_dir(&seed).unwrap();
    git(&seed, &["init", "--quiet"]);
    git(&seed, &["config", "user.name", "Seed"]);
    git(&seed, &["config", "user.email", "seed@example.invalid"]);
    std::fs::write(seed.join("README"), "the seed content\n").unwrap();
    git(&seed, &["add", "README"]);
    git(&seed, &["commit", "--quiet", "-m", "the seed"]);
    git(&seed, &["rev-parse", "HEAD"])
}

fn clone(seed: &Path, expect: &str, dest: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "forge",
            "clone",
            "--seed",
            seed.to_str().unwrap(),
            "--expect",
            expect,
            dest.to_str().unwrap(),
        ])
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_local_seed_clones_with_no_forge_and_keeps_its_hash() {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("seed");
    let hash = make_seed(dir.path());
    let dest = dir.path().join("checkout");
    let out = clone(&seed, &hash, &dest);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(git(&dest, &["rev-parse", "HEAD"]), hash);
    assert_eq!(
        std::fs::read_to_string(dest.join("README")).unwrap(),
        "the seed content\n"
    );
    let printed = text(&out.stdout);
    assert!(printed.contains(&hash), "{printed}");
}

#[test]
fn a_tampered_seed_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("seed");
    let hash = make_seed(dir.path());
    // The seed changes after the key was published: its HEAD is no longer
    // the hash that the caller expects.
    std::fs::write(seed.join("README"), "tampered\n").unwrap();
    git(&seed, &["add", "README"]);
    git(&seed, &["commit", "--quiet", "-m", "tampered"]);
    let tampered = git(&seed, &["rev-parse", "HEAD"]);
    assert_ne!(hash, tampered);
    let dest = dir.path().join("checkout");
    let out = clone(&seed, &hash, &dest);
    assert_eq!(
        out.status.code(),
        Some(podnet::exit::DATA_ERROR),
        "{}",
        text(&out.stderr)
    );
    let err = text(&out.stderr);
    assert!(err.contains(&seed.display().to_string()), "{err}");
    let joined = err.replace(' ', "");
    assert!(joined.contains(&hash), "{err}");
    assert!(joined.contains(&tampered), "{err}");
    assert!(!err.contains("<hidden>"), "the hashes are readable: {err}");
    assert!(!dest.exists(), "a refused clone left a tree");
}

#[test]
fn a_missing_seed_is_no_input_and_a_bad_hash_is_usage() {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("seed");
    let hash = make_seed(dir.path());
    let missing = dir.path().join("absent");
    let out = clone(&missing, &hash, &dir.path().join("a"));
    assert_eq!(
        out.status.code(),
        Some(podnet::exit::NO_INPUT),
        "{}",
        text(&out.stderr)
    );
    let out = clone(&seed, "not-a-hash", &dir.path().join("b"));
    assert_eq!(
        out.status.code(),
        Some(podnet::exit::USAGE),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn an_existing_destination_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let seed = dir.path().join("seed");
    let hash = make_seed(dir.path());
    let dest: PathBuf = dir.path().join("there");
    std::fs::create_dir(&dest).unwrap();
    let out = clone(&seed, &hash, &dest);
    assert_eq!(
        out.status.code(),
        Some(podnet::exit::USAGE),
        "{}",
        text(&out.stderr)
    );
    assert!(
        text(&out.stderr).contains("exists"),
        "{}",
        text(&out.stderr)
    );
}
