//! The key of an install, its fingerprint and its allow list (T-261,
//! GitHub #466, row E0-1). `pod-identity` (podcrates call C118) holds the
//! key files, the key and the fingerprints; these tests prove the glue and
//! the rules of the row: a key with the mode 0644 exits 71, a missing key
//! exits 66, a wrong peer exits 77, the seed is never shown, and the allow
//! list is default deny with 4096 entries at most. The tests bind no port.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use pod_identity::{DisplayFingerprint, PublicKey};
use podnet::exit::{self, code_of};
use podnet::identity::{self, AllowList, MAX_ALLOWED};

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The seed of the key file, as the file holds it.
fn seed_of(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().trim().to_string()
}

/// A key that is not the key of the install.
fn other_key(dir: &Path, name: &str) -> PublicKey {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap().public()
}

#[test]
fn the_first_use_makes_a_key_with_the_mode_0600() {
    let dir = tempfile::tempdir().unwrap();
    let key = identity::create(dir.path()).unwrap();
    assert!(key.made());
    assert_eq!(mode(key.path()), 0o600);
    assert_eq!(key.path(), identity::key_path(dir.path()));
}

#[test]
fn the_same_key_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let first = identity::create(dir.path()).unwrap();
    let again = identity::create(dir.path()).unwrap();
    assert!(!again.made());
    assert_eq!(first.fingerprint(), again.fingerprint());
    let opened = identity::open(dir.path()).unwrap();
    assert_eq!(first.public(), opened.public());
}

#[test]
fn a_key_with_the_mode_0644_exits_71() {
    let dir = tempfile::tempdir().unwrap();
    let key = identity::create(dir.path()).unwrap();
    std::fs::set_permissions(key.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    for result in [identity::open(dir.path()), identity::create(dir.path())] {
        let error = result.unwrap_err();
        assert_eq!(code_of(&error), 71, "{error:#}");
        let text = format!("{error:#}");
        assert!(text.contains("0644"), "{text}");
        assert!(!text.contains(&seed_of(key.path())), "{text}");
    }
    // The file is not replaced: the identity stays.
    assert_eq!(mode(key.path()), 0o644);
}

#[test]
fn a_missing_key_exits_66() {
    let dir = tempfile::tempdir().unwrap();
    let error = identity::open(dir.path()).unwrap_err();
    assert_eq!(code_of(&error), 66, "{error:#}");
    assert!(
        !identity::key_path(dir.path()).exists(),
        "open makes no key"
    );
}

#[test]
fn the_fingerprint_is_base32_in_five_groups_with_a_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let key = identity::create(dir.path()).unwrap();
    let text = key.fingerprint();
    let groups: Vec<&str> = text.split('-').collect();
    assert_eq!(groups.len(), 5, "{text}");
    for group in &groups {
        assert_eq!(group.len(), 8, "{text}");
        assert!(
            group
                .bytes()
                .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b)),
            "{text}"
        );
    }
    assert_eq!(text, key.public().display_fingerprint());
    let parsed = DisplayFingerprint::parse(&text).unwrap();
    assert!(parsed.matches(&key.public()));
    // One changed character fails the checksum.
    let mut changed = text.clone().into_bytes();
    changed[0] = if changed[0] == b'A' { b'B' } else { b'A' };
    let changed = String::from_utf8(changed).unwrap();
    assert!(DisplayFingerprint::parse(&changed).is_err(), "{changed}");
}

#[test]
fn the_allow_list_is_default_deny() {
    let dir = tempfile::tempdir().unwrap();
    let peer = other_key(dir.path(), "peer");
    let list = AllowList::default();
    assert!(list.is_empty());
    assert!(!list.admits(&peer));
    let error = list.check(&peer).unwrap_err();
    assert_eq!(code_of(&error), 77, "{error:#}");
    let blank = AllowList::parse("# nobody\n\n   \n").unwrap();
    assert!(blank.is_empty() && !blank.admits(&peer));
}

#[test]
fn a_wrong_peer_exits_77() {
    let dir = tempfile::tempdir().unwrap();
    let friend = other_key(dir.path(), "friend");
    let stranger = other_key(dir.path(), "stranger");
    let list = AllowList::parse(&format!("{}\n", friend.display_fingerprint())).unwrap();
    list.check(&friend).unwrap();
    let error = list.check(&stranger).unwrap_err();
    assert_eq!(code_of(&error), exit::NO_PERM);
    assert_eq!(code_of(&error), 77);
    let text = format!("{error:#}");
    assert!(text.contains(&stranger.display_fingerprint()), "{text}");
}

#[test]
fn the_allow_list_reads_each_form_of_a_key_and_comments() {
    let dir = tempfile::tempdir().unwrap();
    let a = other_key(dir.path(), "a");
    let b = other_key(dir.path(), "b");
    let c = other_key(dir.path(), "c");
    let d = other_key(dir.path(), "d");
    let text = format!(
        "# the peers of this install\n{} laptop\n  {}  \n{}\n{}\n",
        a.display_fingerprint().to_ascii_lowercase(),
        b.fingerprint(),
        c.hex(),
        d.display_fingerprint(),
    );
    let list = AllowList::parse(&text).unwrap();
    assert_eq!(list.len(), 4);
    for key in [a, b, c, d] {
        assert!(list.admits(&key), "{key:?}");
    }
    assert!(!list.admits(&other_key(dir.path(), "e")));
}

#[test]
fn the_allow_list_holds_4096_entries_at_most() {
    assert_eq!(MAX_ALLOWED, 4096);
    let line = |n: u32| {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(&n.to_be_bytes());
        format!("{}\n", PublicKey(bytes).display_fingerprint())
    };
    let full: String = (0..4096).map(line).collect();
    let list = AllowList::parse(&full).unwrap();
    assert_eq!(list.len(), 4096);
    let over = format!("{full}{}", line(4096));
    let error = AllowList::parse(&over).unwrap_err();
    assert_eq!(code_of(&error), 78, "{error:#}");
    assert!(format!("{error:#}").contains("4096"), "{error:#}");
}

#[test]
fn a_line_that_is_no_key_exits_78_and_names_the_line() {
    let error = AllowList::parse("# one\nnot-a-key\n").unwrap_err();
    assert_eq!(code_of(&error), 78, "{error:#}");
    assert!(format!("{error:#}").contains("line 2"), "{error:#}");
}

#[test]
fn the_allow_list_is_read_from_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let friend = other_key(dir.path(), "friend");
    let path = dir.path().join("allow");
    // A missing file exits 66.
    let error = AllowList::read(&path).unwrap_err();
    assert_eq!(code_of(&error), 66, "{error:#}");
    std::fs::write(&path, format!("{}\n", friend.display_fingerprint())).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let list = AllowList::read(&path).unwrap();
    assert!(list.admits(&friend));
    // Others must not change the file.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    let error = AllowList::read(&path).unwrap_err();
    assert_eq!(code_of(&error), 71, "{error:#}");
}

#[test]
fn the_seed_is_never_shown() {
    let dir = tempfile::tempdir().unwrap();
    let key = identity::create(dir.path()).unwrap();
    let seed = seed_of(key.path());
    assert_eq!(seed.len(), 64);
    let json = serde_json::to_string(&key.report()).unwrap();
    for text in [
        format!("{key:?}"),
        format!("{key:#?}"),
        json.clone(),
        key.fingerprint(),
    ] {
        assert!(!text.contains(&seed), "{text}");
    }
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["fingerprint"], key.fingerprint());
    assert!(value.get("seed").is_none() && value.get("secret").is_none());
    // A file that is no key: the message names the file, never its text.
    let bad = tempfile::tempdir().unwrap();
    let path = identity::key_path(bad.path());
    std::fs::write(&path, "ab".repeat(31)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = identity::open(bad.path()).unwrap_err();
    assert!(!format!("{error:#}").contains(&"ab".repeat(31)));
}

#[test]
fn the_table_of_exit_codes_lists_66_68_71_and_77() {
    for (code, name) in [
        (66, "no-input"),
        (68, "no-host"),
        (71, "os-error"),
        (77, "no-permission"),
    ] {
        let row = exit::TABLE.iter().find(|r| r.code == code);
        let row = row.unwrap_or_else(|| panic!("no row for {code}"));
        assert_eq!(row.name, name);
    }
    assert_eq!(exit::NO_INPUT, 66);
    assert_eq!(exit::NO_HOST, 68);
    assert_eq!(exit::OS_ERROR, 71);
    assert_eq!(exit::NO_PERM, 77);
}
