//! T-173: another owner is a `Config` error (exit 78), the mode is still
//! checked, and a refused file leads to no registration.

use std::os::unix::fs::PermissionsExt as _;

use super::super::private::check_private_as;
use super::super::WarpClient;

fn mode(path: &std::path::Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_foreign_owner_is_a_config_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    std::fs::write(&path, "{}").unwrap();
    mode(&path, 0o600);
    let me = unsafe { libc::geteuid() };
    let err = check_private_as(&path, me + 1).unwrap_err();
    assert_eq!(crate::exit::code_of(&err), crate::exit::CONFIG);
    assert!(err.to_string().contains("not to this user"), "{err}");
    check_private_as(&path, me).unwrap();
    // The mode of an own file is still checked.
    mode(&path, 0o644);
    assert!(check_private_as(&path, me).is_err());
}

#[tokio::test]
async fn a_refused_identity_leads_to_no_registration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    std::fs::write(&path, "{}").unwrap();
    mode(&path, 0o600);
    // A real foreign owner needs root. Else the test has no subject.
    if unsafe { libc::geteuid() } != 0
        || std::os::unix::fs::chown(&path, Some(54321), None).is_err()
    {
        return;
    }
    let err = WarpClient::register_or_load(&path).await.err().unwrap();
    assert_eq!(crate::exit::code_of(&err), crate::exit::CONFIG, "{err:#}");
    assert_eq!(std::fs::read(&path).unwrap(), b"{}");
}
