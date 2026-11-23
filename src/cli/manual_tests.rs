//! The checks of `pod-manual` over the manual of podnet: the text rules, the
//! model against the parser, the llms files, the variables and the exit
//! codes. `cargo test --bin podnet -- --ignored write_the_llms_files` writes
//! the llms files again.

use std::path::{Path, PathBuf};

use clap::CommandFactory as _;
use pod_manual::check::{assert_none, exit_codes, flags, generated, model, texts, variables};
use pod_manual::{llms, scan};

use crate::cli::Cli;

fn manual() -> pod_manual::Manual {
    podnet::manual::manual(Cli::command())
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The llms files of the repository, each with its text now.
fn llms_files() -> Vec<(PathBuf, String)> {
    let files = podnet::manual::llms(&manual(), root());
    vec![
        (root().join(llms::INDEX), files.index),
        (root().join(llms::FULL), files.full),
    ]
}

const WRITE: &str = "run cargo test --bin podnet -- --ignored write_the_llms_files";

#[test]
fn each_text_of_the_manual_keeps_the_rules() {
    let m = manual();
    assert_none(texts(&m));
    assert_none(model(&m));
    // The help of each command holds its flags, and the help of the root
    // the titles of its groups and its "Start here" lines (T-473).
    assert_none(pod_manual::check::help(&m));
    assert!(m.commands.len() > 50, "{} commands", m.commands.len());
}

/// Each command and flag of the model is in the parser, and the reverse.
/// The manual does not show the flags of clap for its own help and version.
#[test]
fn the_manual_agrees_with_the_parser() {
    let parser = pod_manual::clap::parser(Cli::command());
    let model = manual();
    assert_none(flags(&model, |path: &[String]| {
        if path == ["pod"] {
            // `pod` has its own parser (podnet-netns): the manual reads its
            // flags from the help of pod, and the test
            // `each_flag_of_the_help_is_a_flag_of_the_parser` of
            // podnet-netns proves that its parser takes each of them (T-500).
            let pod = model.commands.iter().find(|c| c.path == ["pod"])?;
            let flags = pod
                .flags
                .iter()
                .map(|f| pod_manual::check::ParserFlag {
                    spellings: f.spellings(),
                    takes_value: f.value.is_some(),
                })
                .collect();
            return Some(pod_manual::check::ParserCommand {
                flags,
                subcommands: Vec::new(),
            });
        }
        let mut c = parser(path)?;
        c.flags.retain(|f| {
            let flag = pod_manual::Flag {
                short: f
                    .spellings
                    .iter()
                    .find_map(|s| s.strip_prefix('-').filter(|s| s.len() == 1))
                    .and_then(|s| s.chars().next()),
                long: f
                    .spellings
                    .iter()
                    .find_map(|s| s.strip_prefix("--"))
                    .map(str::to_string),
                value: f.takes_value.then(String::new),
                ..pod_manual::Flag::default()
            };
            !podnet::manual::is_clap_flag(&flag)
        });
        Some(c)
    }));
}

/// `llms.txt` and `llms-full.txt` are the text of the model and of the
/// documents now. After a change of a command, of the manual, of the
/// version or of a listed document, write them again (see [`WRITE`]).
#[test]
fn the_llms_files_equal_the_model() {
    for (path, want) in llms_files() {
        assert_none(generated(&path, &want, WRITE));
    }
}

/// Writes the llms files from the model.
#[test]
#[ignore = "it writes the files of the repository"]
fn write_the_llms_files() {
    for (path, want) in llms_files() {
        std::fs::write(&path, want).unwrap();
    }
}

/// The source of podnet: `src/`, `build.rs` and each crate of `crates/`,
/// with the C code of the shims, and the pager of pod-manual.
fn sources() -> Vec<(PathBuf, String)> {
    let mut dirs = vec![root().join("src")];
    let mut builds = vec![root().join("build.rs")];
    let mut c_files = Vec::new();
    let mut crates: Vec<PathBuf> = std::fs::read_dir(root().join("crates"))
        .expect("crates/")
        .flatten()
        .map(|e| e.path())
        .collect();
    crates.sort();
    for dir in &crates {
        dirs.push(dir.join("src"));
        builds.push(dir.join("build.rs"));
        if let Ok(entries) = std::fs::read_dir(dir.join("shim")) {
            c_files.extend(
                entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "c")),
            );
        }
    }
    let mut sources = scan::rust_sources(&dirs);
    for path in builds.into_iter().chain(c_files) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            sources.push((path, scan::non_test(&text)));
        }
    }
    sources.push((
        PathBuf::from("pod-manual/src/pager.rs"),
        scan::non_test(pod_manual::pager::SOURCE),
    ));
    sources
}

/// The names that need no fact, and the facts that no source names: cargo
/// sets TARGET and RUSTC for build.rs; tracing-subscriber reads RUST_LOG;
/// the shell sets HOSTNAME for the conditions of a measurement; git reads
/// GIT_PAGER, which podnet sets for its own children.
const EXEMPT: &[&str] = &["TARGET", "RUSTC", "RUST_LOG", "HOSTNAME", "GIT_PAGER"];

/// Each variable that the code reads or sets is a fact, and each fact is in
/// the code.
#[test]
fn each_variable_of_the_code_is_a_fact() {
    let sources = sources();
    assert!(sources.len() > 300, "{} files", sources.len());
    assert_none(variables(&manual(), &sources, EXEMPT));
}

/// Each exit code that the code gives has a fact, and each fact of one code
/// is a code that the code gives: the constants of `podnet_core::exit`, the
/// codes of podnet-netns, and 128 and the number of a signal.
#[test]
fn each_exit_code_has_a_fact() {
    let mut given: Vec<i32> = Vec::new();
    for (file, prefix) in [
        (include_str!("../../crates/podnet-core/src/exit.rs"), ""),
        (
            include_str!("../../crates/podnet-netns/src/netns/mod.rs"),
            "EXIT_",
        ),
        (
            include_str!("../../crates/podnet-netns/src/learn.rs"),
            "EXIT_",
        ),
    ] {
        given.extend(
            scan::exit_constants(file, prefix)
                .into_iter()
                .map(|(_, n)| n),
        );
    }
    // src/cli/signals.rs: a second stop signal exits 128 and its number.
    given.push(128 + libc::SIGINT);
    given.sort_unstable();
    given.dedup();
    assert!(given.len() >= 14, "{given:?}");
    assert_none(exit_codes(&manual(), &given));
}

/// `COMMAND --help --json` of each command gives the JSON of its model.
#[test]
fn the_json_help_of_each_command_comes_from_the_model() {
    let m = manual();
    for c in &m.commands {
        let mut args = c.path.clone();
        args.extend(["--help".to_string(), "--json".to_string()]);
        let help = pod_manual::help::request(&m, &args).expect("a help request");
        assert_eq!(help.path, c.path);
        let text = pod_manual::help::answer(&m, &help).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["command"]["path"], serde_json::json!(c.path), "{text}");
    }
}

/// The rows of EXIT STATUS ascend (issue 254), no code is in two rows, and
/// each class of failure has a row.
#[test]
fn each_class_of_failure_has_one_row() {
    use podnet::exit::Fault;
    let codes: Vec<i32> = podnet::manual::facts::exits()
        .iter()
        .filter_map(|e| match e.code {
            pod_manual::Code::Value(n) => Some(n),
            _ => None,
        })
        .collect();
    assert!(codes.windows(2).all(|w| w[0] < w[1]), "{codes:?}");
    for class in [
        Fault::Usage,
        Fault::Unavailable,
        Fault::Config,
        Fault::TempFail,
        Fault::Timeout,
        Fault::NoInput,
        Fault::NoHost,
        Fault::OsError,
        Fault::NoPermission,
        Fault::DataError,
    ] {
        assert!(codes.contains(&class.code()), "{class:?}");
    }
}
