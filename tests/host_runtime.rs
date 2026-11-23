//! FS-1 to FS-3: the probe of a file system states the five properties,
//! and a read-only mount refuses a write with a named reason.
//!
//! `mkfs.vfat` is not installed on this host, so the FAT image here is a
//! synthetic boot sector: the detection reads bytes 54..62 (FAT12/16) or
//! 82..90 (FAT32), exactly as a mount would. The image is not mounted, and
//! the test says so. The probe also reads the mount of a real path.

use std::io::Write as _;
use std::process::Command;

use podnet::hostprobe::{self, Family, Status};

fn podnet(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env_remove("PODNET_ERROR_PROXY")
        .output()
        .expect("podnet runs")
}

/// A 512-byte FAT16 boot sector, with the label at the offset that a mount
/// reads.
fn fat16_image(path: &std::path::Path) {
    let mut sector = vec![0u8; 512];
    sector[0..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    sector[3..11].copy_from_slice(b"mkfs.fat");
    sector[54..62].copy_from_slice(b"FAT16   ");
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(&sector).unwrap();
}

#[test]
fn a_fat_image_is_named_and_each_property_is_emulated() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("fat16.img");
    fat16_image(&image);
    let report = hostprobe::probe(&image).unwrap();
    assert_eq!(report.family, Family::Fat, "{}", report.filesystem);
    assert_eq!(report.filesystem, "fat16");
    assert!(!report.read_only);
    assert!(!report.required());
    for property in &report.properties {
        assert_eq!(
            property.status,
            Status::Emulated,
            "{}: {}",
            property.name,
            property.detail
        );
    }
    let names: Vec<&str> = report.properties.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "timestamps",
            "atomic rename",
            "hard links",
            "permission bits",
            "case sensitivity"
        ]
    );
    assert!(
        report.fallback.contains("serial number"),
        "{}",
        report.fallback
    );
    assert!(report.fallback.contains("copies instead of hard links"));

    let out = podnet(&["host", "probe", image.to_str().unwrap(), "--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["family"], "fat");
    assert_eq!(doc["filesystem"], "fat16");
    assert_eq!(doc["read_only"], false);
    assert_eq!(doc["properties"].as_array().unwrap().len(), 5);
    assert_eq!(doc["properties"][0]["status"], "emulated");
}

/// FS-3: a read-only mount keeps the five properties unknown and refuses
/// each write before it starts, naming the reason.
#[test]
fn a_read_only_image_refuses_a_write_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("read-only.img");
    fat16_image(&image);
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o444)).unwrap();
    let report = hostprobe::probe(&image).unwrap();
    assert!(report.read_only);
    assert!(
        report.fallback.starts_with("read-only"),
        "{}",
        report.fallback
    );
    let refusal = hostprobe::require_writable(&report).unwrap_err();
    assert!(refusal.reason.contains("read-only"), "{refusal}");
    assert!(refusal.reason.contains("refuses the write"), "{refusal}");
}

#[test]
fn a_missing_path_is_an_error_that_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("absent");
    let err = hostprobe::probe(&missing).unwrap_err();
    assert!(err.to_string().contains("absent"), "{err}");
    let out = podnet(&["host", "probe", missing.to_str().unwrap()]);
    assert_ne!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("absent"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A real directory of this host: a unix file system gives the five
/// properties, and the text form names each one.
#[test]
fn a_unix_directory_gives_the_five_properties() {
    let dir = tempfile::tempdir().unwrap();
    let out = podnet(&["host", "probe", dir.path().to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    for name in [
        "timestamps",
        "atomic rename",
        "hard links",
        "permission bits",
        "case sensitivity",
    ] {
        assert!(text.contains(name), "{text}");
    }
    assert!(text.contains("fallback:"), "{text}");
}
