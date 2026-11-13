//! The rules of the names that `podnet receive` writes (T-105, GitHub
//! #436): a table of hostile zips, each one built here and unpacked into an
//! empty output directory. A refused zip changes nothing: the listing of
//! the output directory and of its parent is the same before and after.

use std::fs::File;
use std::io::{Seek, Write};
use std::path::Path;

use podnet::exit;
use podnet::transfer::land::{self, Caps, Landing};

/// One entry of a zip of the test: its name, its Unix mode (0: none), its
/// data, and the size that its headers say (the data's when `None`).
struct Raw<'a> {
    name: &'a [u8],
    mode: u32,
    data: &'a [u8],
    says: Option<u32>,
}

fn raw<'a>(name: &'a str, data: &'a [u8]) -> Raw<'a> {
    Raw {
        name: name.as_bytes(),
        mode: 0o100_644,
        data,
        says: None,
    }
}

/// A zip of stored entries, written by hand: no code of podnet makes it.
fn zip(entries: &[Raw<'_>]) -> File {
    let mut file = tempfile::tempfile().unwrap();
    let mut central = Vec::new();
    for e in entries {
        let offset = file.stream_position().unwrap() as u32;
        let crc = crc32(e.data);
        let says = e.says.unwrap_or(e.data.len() as u32);
        let mut head = Vec::new();
        head.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        head.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        head.extend_from_slice(&crc.to_le_bytes());
        head.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
        head.extend_from_slice(&says.to_le_bytes());
        head.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        head.extend_from_slice(&[0, 0]);
        head.extend_from_slice(e.name);
        file.write_all(&head).unwrap();
        file.write_all(e.data).unwrap();
        let mut row = Vec::new();
        row.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        row.extend_from_slice(&[20, 3, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        row.extend_from_slice(&crc.to_le_bytes());
        row.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
        row.extend_from_slice(&says.to_le_bytes());
        row.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        row.extend_from_slice(&[0; 8]);
        row.extend_from_slice(&(e.mode << 16).to_le_bytes());
        row.extend_from_slice(&offset.to_le_bytes());
        row.extend_from_slice(e.name);
        central.push(row);
    }
    let start = file.stream_position().unwrap() as u32;
    let size: usize = central.iter().map(Vec::len).sum();
    for row in &central {
        file.write_all(row).unwrap();
    }
    let mut end = Vec::new();
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    end.extend_from_slice(&[0; 4]);
    end.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    end.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    end.extend_from_slice(&(size as u32).to_le_bytes());
    end.extend_from_slice(&start.to_le_bytes());
    end.extend_from_slice(&[0, 0]);
    file.write_all(&end).unwrap();
    file
}

/// CRC-32 (IEEE), written out.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Each path under `dir`, sorted, with its size.
fn listing(dir: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let entry = entry.unwrap();
            let meta = std::fs::symlink_metadata(entry.path()).unwrap();
            out.push((entry.path().display().to_string(), meta.len()));
            if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    out.sort();
    out
}

/// Unzip `file` into a new empty directory as `top`: the error, and
/// whether the listing of the directory and of its parent stayed the same.
fn refused_cleanly(mut file: File, caps: &Caps) -> (anyhow::Error, bool) {
    let parent = tempfile::tempdir().unwrap();
    let out = parent.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let before = listing(parent.path());
    let err = land::unzip(&mut file, &out, "top", caps).expect_err("a refusal");
    (err, listing(parent.path()) == before)
}

#[test]
fn transfer_land_refuses_each_hostile_name_and_writes_nothing() {
    let long = "x".repeat(300);
    let table: Vec<(&str, Vec<Raw<'_>>, &str)> = vec![
        ("parent", vec![raw("../x", b"1")], "not a name"),
        ("climb", vec![raw("a/../../x", b"1")], "not a name"),
        ("absolute", vec![raw("/etc/x", b"1")], "absolute"),
        ("backslash", vec![raw("a\\..\\x", b"1")], "backslash"),
        (
            "nul",
            vec![Raw {
                name: b"a\0b",
                mode: 0o100_644,
                data: b"1",
                says: None,
            }],
            "NUL",
        ),
        ("long", vec![raw(&long, b"1")], "over 255 bytes"),
        ("case", vec![raw("A", b"1"), raw("a", b"2")], "case folded"),
        (
            "symlink",
            vec![
                Raw {
                    name: b"evil",
                    mode: 0o120_777,
                    data: b"/tmp",
                    says: None,
                },
                raw("evil/f", b"1"),
            ],
            "a symbolic link",
        ),
        (
            "fifo",
            vec![Raw {
                name: b"p",
                mode: 0o010_644,
                data: b"",
                says: None,
            }],
            "a fifo",
        ),
        (
            "device",
            vec![Raw {
                name: b"d",
                mode: 0o020_644,
                data: b"",
                says: None,
            }],
            "a device",
        ),
    ];
    for (what, entries, words) in table {
        let (err, clean) = refused_cleanly(zip(&entries), &Caps::default());
        assert_eq!(exit::code_of(&err), exit::DATA_ERROR, "{what}: {err:#}");
        assert!(format!("{err:#}").contains(words), "{what}: {err:#}");
        assert!(clean, "{what}: a refused zip changed the disk");
    }
}

#[test]
fn transfer_land_a_header_that_lies_is_cut_at_its_size() {
    let big = vec![7u8; 10 << 20];
    let caps = Caps {
        max_files: 10,
        max_bytes: 1 << 20,
    };
    let (err, clean) = refused_cleanly(
        zip(&[Raw {
            name: b"f",
            mode: 0o100_644,
            data: &big,
            says: Some(10),
        }]),
        &caps,
    );
    assert_eq!(exit::code_of(&err), exit::DATA_ERROR, "{err:#}");
    assert!(
        format!("{err:#}").contains("more data than the 10 bytes"),
        "{err:#}"
    );
    assert!(clean, "the file of the lie was removed");
    let (err, _) = refused_cleanly(zip(&[raw("f", &big)]), &caps);
    assert!(format!("{err:#}").contains("the cap is 1048576"), "{err:#}");
    let many = Caps {
        max_files: 1,
        max_bytes: 1 << 20,
    };
    let (err, _) = refused_cleanly(zip(&[raw("a", b"1"), raw("b", b"2")]), &many);
    assert!(format!("{err:#}").contains("--max-files"), "{err:#}");
}

#[test]
fn transfer_land_an_existing_name_is_refused_and_keeps_its_bytes() {
    let parent = tempfile::tempdir().unwrap();
    std::fs::create_dir(parent.path().join("top")).unwrap();
    std::fs::write(parent.path().join("top/f"), b"old").unwrap();
    let err = land::unzip(
        &mut zip(&[raw("f", b"new")]),
        parent.path(),
        "top",
        &Caps::default(),
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("EEXIST"), "{err:#}");
    assert_eq!(std::fs::read(parent.path().join("top/f")).unwrap(), b"old");
    let mut landing = Landing::open(parent.path()).unwrap();
    std::fs::write(parent.path().join("g"), b"old").unwrap();
    let err = landing.file(&["g".to_string()]).unwrap_err();
    assert!(err.to_string().contains("EEXIST"), "{err}");
    assert_eq!(std::fs::read(parent.path().join("g")).unwrap(), b"old");
}

#[test]
fn transfer_land_two_receivers_of_one_name_one_wins() {
    for _ in 0..50 {
        let parent = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let racers: Vec<_> = (0..2)
            .map(|i| {
                let (out, barrier) = (parent.path().to_path_buf(), barrier.clone());
                std::thread::spawn(move || {
                    let mut landing = Landing::open(&out).unwrap();
                    barrier.wait();
                    let made = landing
                        .file(&["same".to_string()])
                        .map(|mut f| f.write_all(&[i]).unwrap());
                    if made.is_ok() {
                        landing.keep();
                    }
                    made.is_ok()
                })
            })
            .collect();
        let won: Vec<bool> = racers.into_iter().map(|r| r.join().unwrap()).collect();
        assert_eq!(
            won.iter().filter(|w| **w).count(),
            1,
            "one receiver wins: {won:?}"
        );
    }
}

fn umask() -> u32 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("Umask:")).unwrap();
    u32::from_str_radix(line.split_whitespace().nth(1).unwrap(), 8).unwrap()
}

#[test]
fn transfer_land_a_good_zip_lands_with_the_bytes_and_the_mode_of_the_rules() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().unwrap();
    let entries = [
        Raw {
            name: b"a/b.txt",
            mode: 0o104_755,
            data: b"bee",
            says: None,
        },
        raw("c.txt", b"sea"),
    ];
    let files = land::unzip(&mut zip(&entries), parent.path(), "top", &Caps::default()).unwrap();
    assert_eq!(files, 2);
    let top = parent.path().join("top");
    assert_eq!(std::fs::read(top.join("a/b.txt")).unwrap(), b"bee");
    assert_eq!(std::fs::read(top.join("c.txt")).unwrap(), b"sea");
    let mode = std::fs::metadata(top.join("a/b.txt"))
        .unwrap()
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(
        mode,
        0o666 & !umask(),
        "no executable or set-id bit of the zip"
    );
}

#[test]
fn transfer_land_a_deflated_zip_of_python_lands() {
    // Made by the zipfile module of Python 3 with ZIP_DEFLATED: software
    // that podnet did not write (tests/data/transfer/python-deflated.zip).
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/transfer/python-deflated.zip");
    let parent = tempfile::tempdir().unwrap();
    let files = land::unzip(
        &mut File::open(fixture).unwrap(),
        parent.path(),
        "top",
        &Caps::default(),
    )
    .unwrap();
    assert_eq!(files, 3);
    let sha = |name: &str| {
        pod_wormhole::hex(&<sha2::Sha256 as sha2::Digest>::digest(
            std::fs::read(parent.path().join(name)).unwrap(),
        ))
    };
    assert_eq!(
        sha("top/sub/words.txt"),
        "bc1562eb0b355f923738e0c50af73ae846674dc3a64b0c36b95ca7e724ef7e74"
    );
    assert_eq!(
        sha("top/one.bin"),
        "e96760a87768717bcebcfd25ddc7d46b4dbc95a4b0014def080c08539f7d90d0"
    );
}

#[test]
fn transfer_land_python_reads_the_zip_of_podnet() {
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("d/e")).unwrap();
    std::fs::write(src.path().join("d/e/f.txt"), b"eff").unwrap();
    std::fs::write(src.path().join("g"), vec![1u8; 70_000]).unwrap();
    let mut out = tempfile::NamedTempFile::new().unwrap();
    let written = podnet::transfer::zip::write(src.path(), out.as_file_mut()).unwrap();
    assert_eq!((written.numfiles, written.numbytes), (4, 70_003));
    let check = "import sys, zipfile\nz = zipfile.ZipFile(sys.argv[1])\nassert z.testzip() is None\n\
                 print(','.join(sorted(z.namelist())), z.read('d/e/f.txt').decode(), oct(z.getinfo('g').external_attr >> 16))";
    let run = std::process::Command::new("python3")
        .args(["-I", "-c", check])
        .arg(out.path())
        .output();
    let Ok(run) = run else {
        eprintln!("did not run: no python3 here");
        return;
    };
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let said = String::from_utf8(run.stdout).unwrap();
    assert!(said.starts_with("d/,d/e/,d/e/f.txt,g eff 0o100"), "{said}");
}
