//! The offline bundle of PodUtils/podnet#499: a signed manifest, a delta
//! from the previous bundle, and the apply of the delta on the host that
//! holds the previous one.
//!
//! The delta carries the changed file and not the unchanged one, so it is
//! smaller than the base. The apply gives the new manifest and the new
//! bytes, and a delta that names another base is refused.

use std::path::Path;
use std::process::Command;

fn podnet(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("RUST_LOG");
    cmd
}

fn key(dir: &Path) -> podnet::identity::Identity {
    std::fs::create_dir_all(dir).expect("a state directory");
    podnet::identity::create(dir).expect("a key")
}

fn bytes_of(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

#[test]
fn a_delta_applies_on_the_host_of_its_base_and_carries_only_the_change() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let key_dir = dir.path().join("key");
    let me = key(&key_dir);
    let key = key_dir.to_str().expect("a path").to_string();

    let one = dir.path().join("one");
    let two = dir.path().join("two");
    let b = dir.path().join("b");
    std::fs::write(&one, b"alpha one").expect("a file");
    std::fs::write(&two, b"alpha two, and the second release is longer").expect("a file");
    std::fs::write(&b, vec![b'b'; 4096]).expect("a file");

    let base = dir.path().join("base");
    let out = podnet(
        dir.path(),
        &[
            "offline",
            "bundle",
            "build",
            "--out",
            base.to_str().expect("a path"),
            "--name",
            "one",
            "--file",
            &format!("{}:a", one.display()),
            "--file",
            &format!("{}:b", b.display()),
            "--key",
            &key,
        ],
    )
    .output()
    .expect("bundle build");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let built = String::from_utf8_lossy(&out.stdout);
    assert!(built.starts_with("bundle one "), "{built:?}");
    assert!(built.trim_end().ends_with("files 2"), "{built:?}");

    let verify = podnet(
        dir.path(),
        &[
            "offline",
            "bundle",
            "verify",
            base.to_str().expect("a path"),
        ],
    )
    .output()
    .expect("bundle verify");
    assert_eq!(verify.status.code(), Some(0), "{verify:?}");
    assert!(
        String::from_utf8_lossy(&verify.stdout).contains(&me.public().hex()),
        "{verify:?}"
    );

    let delta = dir.path().join("delta");
    let out = podnet(
        dir.path(),
        &[
            "offline",
            "bundle",
            "build",
            "--out",
            delta.to_str().expect("a path"),
            "--name",
            "two",
            "--base",
            base.to_str().expect("a path"),
            "--file",
            &format!("{}:a", two.display()),
            "--file",
            &format!("{}:b", b.display()),
            "--key",
            &key,
        ],
    )
    .output()
    .expect("bundle build, a delta");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.starts_with("delta two "), "{said:?}");
    assert!(said.contains(" add 0 change 1 delete 0"), "{said:?}");

    assert!(
        delta.join("data/a").exists(),
        "the delta does not carry the changed file"
    );
    assert!(
        !delta.join("data/b").exists(),
        "the delta carries the unchanged file"
    );
    let delta_bytes = bytes_of(&delta.join("data"));
    let base_bytes = bytes_of(&base.join("data"));
    assert!(
        delta_bytes < base_bytes,
        "the delta is {delta_bytes} bytes and the base is {base_bytes}"
    );

    let out = podnet(
        dir.path(),
        &[
            "offline",
            "bundle",
            "apply",
            "--base",
            base.to_str().expect("a path"),
            "--delta",
            delta.to_str().expect("a path"),
        ],
    )
    .output()
    .expect("bundle apply");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).starts_with("applied two "),
        "{out:?}"
    );

    assert_eq!(
        std::fs::read(base.join("data/a")).expect("the changed file"),
        b"alpha two, and the second release is longer"
    );
    assert_eq!(
        std::fs::read(base.join("data/b")).expect("the unchanged file"),
        vec![b'b'; 4096]
    );
    let verify = podnet(
        dir.path(),
        &[
            "offline",
            "bundle",
            "verify",
            base.to_str().expect("a path"),
        ],
    )
    .output()
    .expect("bundle verify");
    assert_eq!(verify.status.code(), Some(0), "{verify:?}");
    assert!(
        String::from_utf8_lossy(&verify.stdout).starts_with("bundle two "),
        "{verify:?}"
    );
}

#[test]
fn a_delta_of_another_base_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let key_dir = dir.path().join("key");
    let _ = key(&key_dir);
    let key = key_dir.to_str().expect("a path").to_string();
    let file = dir.path().join("f");
    std::fs::write(&file, b"one").expect("a file");

    let build = |name: &str, out: &Path, extra: &[&str]| {
        let file_arg = format!("{}:data/f", file.display());
        let mut args = vec![
            "offline",
            "bundle",
            "build",
            "--out",
            out.to_str().expect("a path"),
            "--name",
            name,
            "--file",
            &file_arg,
            "--key",
            &key,
        ];
        args.extend_from_slice(extra);
        podnet(dir.path(), &args).output().expect("bundle build")
    };
    let base = dir.path().join("base");
    assert_eq!(build("one", &base, &[]).status.code(), Some(0));
    std::fs::write(&file, b"two").expect("a file");
    let delta = dir.path().join("delta");
    let base_arg = base.to_str().expect("a path").to_string();
    let built = build("two", &delta, &["--base", &base_arg]);
    assert_eq!(built.status.code(), Some(0), "{built:?}");

    // Another bundle with the same name still has another id.
    let other = dir.path().join("other");
    assert_eq!(build("other", &other, &[]).status.code(), Some(0));
    let out = podnet(
        dir.path(),
        &[
            "offline",
            "bundle",
            "apply",
            "--base",
            other.to_str().expect("a path"),
            "--delta",
            delta.to_str().expect("a path"),
        ],
    )
    .output()
    .expect("bundle apply");
    assert_eq!(out.status.code(), Some(65), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("wrong base")
            || String::from_utf8_lossy(&out.stderr).contains("base"),
        "{out:?}"
    );
}
