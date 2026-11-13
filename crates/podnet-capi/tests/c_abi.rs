//! The C ABI of podnet (T-304): the exported symbols are the functions of
//! include/podnet.h, and a C program and a ctypes program echo 1 MiB over a
//! memory link. A test that needs a tool that the host lacks (`nm`, `cc`,
//! `python3`) prints a line `SKIP NAME: TOOL not found` and passes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The directory of the built libraries (the profile directory).
fn built() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

fn have(tool: &str, arg: &str) -> bool {
    Command::new(tool)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn skip(test: &str, tool: &str) {
    eprintln!("SKIP {test}: {tool} not found");
}

/// The functions that the header declares: the names `podnet_*` followed by
/// `(` once the comments are gone.
fn header_functions(text: &str) -> BTreeSet<String> {
    let mut code = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("/*") {
        code.push_str(&rest[..i]);
        let end = rest[i..].find("*/").expect("a comment that never ends") + i + 2;
        rest = &rest[end..];
    }
    code.push_str(rest);
    let mut out = BTreeSet::new();
    for line in code.lines() {
        let mut at = 0;
        while let Some(i) = line[at..].find("podnet_") {
            let start = at + i;
            let name: String = line[start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            at = start + name.len();
            if line[at..].trim_start().starts_with('(') {
                out.insert(name);
            }
        }
    }
    out
}

fn header() -> BTreeSet<String> {
    let text = std::fs::read_to_string(root().join("include/podnet.h")).unwrap();
    header_functions(&text)
}

/// The names `podnet_*` that `nm ARGS FILE` lists as defined.
fn nm(args: &[&str], file: &Path) -> BTreeSet<String> {
    let out = Command::new("nm").args(args).arg(file).output().unwrap();
    assert!(out.status.success(), "nm failed: {out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace().rev();
            let name = w.next()?;
            let kind = w.next()?;
            (kind == "T" && name.starts_with("podnet_")).then(|| name.to_string())
        })
        .collect()
}

#[test]
fn the_header_lists_the_functions_of_the_api() {
    let names = header();
    for want in [
        "podnet_init",
        "podnet_free",
        "podnet_link_new",
        "podnet_link_open",
        "podnet_link_accept",
        "podnet_stream_read",
        "podnet_stream_write",
        "podnet_stream_close",
        "podnet_link_close",
        "podnet_last_error",
        "podnet_version",
    ] {
        assert!(names.contains(want), "the header lacks {want}");
    }
}

#[test]
fn the_exported_symbols_of_the_shared_library_equal_the_header() {
    if !have("nm", "--version") {
        return skip(
            "the_exported_symbols_of_the_shared_library_equal_the_header",
            "nm",
        );
    }
    let so = built().join("libpodnet_capi.so");
    assert!(so.is_file(), "{} is not built", so.display());
    let all = Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(&so)
        .output()
        .unwrap();
    let exported: BTreeSet<String> = String::from_utf8_lossy(&all.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().last().map(str::to_string))
        .collect();
    assert_eq!(exported, header(), "dynamic symbols against the header");
}

#[test]
fn the_static_library_defines_the_functions_of_the_header() {
    if !have("nm", "--version") {
        return skip(
            "the_static_library_defines_the_functions_of_the_header",
            "nm",
        );
    }
    let a = built().join("libpodnet_capi.a");
    assert!(a.is_file(), "{} is not built", a.display());
    // A static library keeps the other symbols of Rust; the interface is
    // the symbols with the prefix podnet_.
    assert_eq!(nm(&["-g", "--defined-only"], &a), header());
}

fn run_c(test: &str, shared: bool) {
    if !have("cc", "--version") {
        return skip(test, "cc");
    }
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("c_abi");
    let mut cc = Command::new("cc");
    cc.args(["-Wall", "-Wextra", "-Werror", "-I"])
        .arg(root().join("include"))
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c_abi.c"))
        .arg("-o")
        .arg(&exe);
    if shared {
        let so = dir.path().join("libpodnet.so");
        std::fs::copy(built().join("libpodnet_capi.so"), &so).unwrap();
        cc.arg("-L")
            .arg(dir.path())
            .arg("-lpodnet")
            .arg(format!("-Wl,-rpath,{}", dir.path().display()));
    } else {
        cc.arg(built().join("libpodnet_capi.a"))
            .args(["-lpthread", "-lm", "-ldl"]);
    }
    cc.arg("-lpthread");
    let out = cc.output().unwrap();
    assert!(out.status.success(), "cc failed: {out:?}");
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let out = Command::new(&exe).arg(&work).output().unwrap();
    let said =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "c_abi failed: {said}");
    assert!(said.contains("1 MiB echoed"), "{said}");
}

#[test]
fn a_c_program_echoes_1_mib_over_a_memory_link_with_the_shared_library() {
    run_c(
        "a_c_program_echoes_1_mib_over_a_memory_link_with_the_shared_library",
        true,
    );
}

#[test]
fn a_c_program_echoes_1_mib_over_a_memory_link_with_the_static_library() {
    run_c(
        "a_c_program_echoes_1_mib_over_a_memory_link_with_the_static_library",
        false,
    );
}

#[test]
fn ctypes_echoes_1_mib_over_a_memory_link() {
    if !have("python3", "--version") {
        return skip("ctypes_echoes_1_mib_over_a_memory_link", "python3");
    }
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new("python3")
        .arg("-I")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c_abi.py"))
        .arg(built().join("libpodnet_capi.so"))
        .arg(dir.path())
        .output()
        .unwrap();
    let said =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "c_abi.py failed: {said}");
    assert!(said.contains("1 MiB echoed"), "{said}");
}
