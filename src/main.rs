//! podnet command line.

/// `println!`, `print!` and `eprintln!` that drop a failed write (issue
/// 74). The std macros panic when the write fails: a detached instance
/// writes to its log, and a full disk or a closed stream must not stop it.
/// A command that only prints keeps the default action of SIGPIPE.
macro_rules! println {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stdout(), $($arg)*);
    }};
}

macro_rules! print {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::write!(::std::io::stdout(), $($arg)*);
    }};
}

macro_rules! eprintln {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stderr(), $($arg)*);
    }};
}

mod cli;

use anyhow::Context;
use clap::FromArgMatches as _;
use tracing_subscriber::EnvFilter;

use cli::argv::{
    apply_token_files, cloudflared_compat, command_name, foreground_hint, no_terminal, pod_args_of,
    pod_network_option, LOG_JSON,
};
use cli::help::{error_json, json_wanted, print_error, top_command, usage_hints};
use cli::opts::{detach_of, network_options, replaces_a_socket};
use cli::signals::install_signals;
use cli::{async_main, Cli, Command};

fn main() {
    // podnet-netns forks and makes its own runtime. Start it before the async
    // runtime of podnet exists.
    // args() panics on an argument that is not UTF-8 (issue 57); clap then
    // reports the bad argument as a usage error.
    // A detached child (issue 70) closes the descriptors that it got and
    // drops its flag before anything else.
    let args = podnet::detach::prepare(std::env::args_os().collect());
    let args = cloudflared_compat(args);
    let argv: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if let Err(err) = apply_token_files(&argv) {
        eprintln!("podnet: {err:#}");
        std::process::exit(podnet::exit::USAGE);
    }
    if argv.get(1).map(String::as_str) == Some("pod") {
        podnet_netns::set_program_name("podnet pod");
        std::process::exit(podnet_netns::main_with_args(&argv[2..]));
    }
    let cli = match top_command()
        .try_get_matches_from(&args)
        .and_then(|matches| Cli::from_arg_matches(&matches))
    {
        Ok(cli) => cli,
        Err(err) => {
            use clap::error::ErrorKind;
            // Help that the user asked for exits 0. Help that comes up
            // because a subcommand is missing (podnet net) is a usage error.
            let code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => podnet::exit::SUCCESS,
                _ => podnet::exit::USAGE,
            };
            // Issue 181: with --log-format json, a usage error is one JSON
            // line too (found in argv: the parse failed).
            if code != podnet::exit::SUCCESS && json_wanted(&argv) {
                eprintln!("{}", error_json(&err.render().to_string(), code));
            } else {
                let _ = err.print();
                for line in usage_hints(&err) {
                    eprintln!("{line}");
                }
            }
            if code != podnet::exit::SUCCESS {
                podnet::report::send(command_name(&argv).as_deref(), code, None);
            }
            std::process::exit(code);
        }
    };
    let command = command_name(&argv);
    LOG_JSON.store(
        cli.log_format == "json",
        std::sync::atomic::Ordering::Relaxed,
    );
    // Issue 115: -q silences the warnings of the pod too. No thread runs
    // yet.
    if cli.quiet {
        // SAFETY: one thread: the runtime does not exist yet.
        unsafe { std::env::set_var(podnet_netns::QUIET_VARIABLE, "1") };
    }
    // Issue 189: `podnet -v pod -- PROG` (a global option before pod) runs the
    // pod too; before, it panicked. Still one thread: the pod forks.
    if let Command::Pod { args } = &cli.command {
        if let Some(option) = pod_network_option(&cli) {
            eprintln!("podnet: podnet pod does not use {option}; give the proxy to the pod with -x (podnet pod --help)");
            std::process::exit(podnet::exit::USAGE);
        }
        podnet_netns::set_program_name("podnet pod");
        std::process::exit(podnet_netns::main_with_args(&pod_args_of(&argv, args)));
    }
    // Issue 126: a server command holds one descriptor for each connection.
    // A command that runs a program of the user leaves its limit as it is.
    let runs_a_program = matches!(
        command.as_deref(),
        Some("warp exec" | "warp shell" | "warp run" | "autostart run")
    );
    if !runs_a_program {
        podnet::rlimit::raise();
    }
    if let Some(detach) = detach_of(&cli.command).filter(|d| d.detach) {
        let name = command.clone().unwrap_or_default();
        if !podnet::detach::is_child() {
            std::process::exit(podnet::detach::start(&name, &detach.options()));
        }
        if let Err(err) = podnet::detach::child_begin() {
            eprintln!("podnet: --detach: {err:#}");
            std::process::exit(podnet::detach::TEMPFAIL);
        }
    } else if podnet::detach::is_child() {
        eprintln!("podnet: --detach-child is for podnet itself");
        std::process::exit(podnet::exit::USAGE);
    } else if let Some(detach) = detach_of(&cli.command) {
        if detach.replace && !replaces_a_socket(&cli.command) {
            eprintln!("podnet: --replace needs --detach");
            std::process::exit(podnet::exit::USAGE);
        }
        // Issue 209: --name with no --detach registers this process.
        match &detach.name {
            Some(name) => {
                if let Err(err) = podnet::instances::check_name(name) {
                    eprintln!("podnet: --name: {err:#}");
                    std::process::exit(podnet::exit::USAGE);
                }
                if let Err(err) =
                    podnet::detach::register(name, command.as_deref().unwrap_or_default())
                {
                    eprintln!("podnet: --name: {err:#}");
                    std::process::exit(podnet::detach::TEMPFAIL);
                }
            }
            None if detach.no_restart => {
                eprintln!("podnet: --no-restart needs --detach or --name");
                std::process::exit(podnet::exit::USAGE);
            }
            None => {}
        }
    }
    if let Some(path) = &cli.ca_file {
        podnet::trust::set_ca_file(path.clone());
        // Issue 173: a roots file that cannot be used exits 78 for each
        // command (status and doctor too), before any work.
        if let Err(err) = podnet::trust::context().roots() {
            // Issue 253: trust::roots names the file already.
            let err = podnet::exit::fault(podnet::exit::Fault::Config, err.into());
            print_error(
                &podnet::report::error_line(&err),
                podnet::exit::code_of(&err),
            );
            let code = podnet::exit::code_of(&err);
            podnet::report::send(command.as_deref(), code, Some(&err));
            std::process::exit(code);
        }
    }
    if let Err(err) = network_options(&cli) {
        // A flag is a usage error; a variable (PODNET_RESOLVE) has its class.
        let err = if podnet::exit::code_of(&err) == podnet::exit::FAILURE {
            podnet::exit::fault(podnet::exit::Fault::Usage, err)
        } else {
            err
        };
        print_error(
            &podnet::report::error_line(&err),
            podnet::exit::code_of(&err),
        );
        let code = podnet::exit::code_of(&err);
        podnet::report::send(command.as_deref(), code, Some(&err));
        std::process::exit(code);
    }
    // A command that only prints ends at a closed stdout (podnet status |
    // head) with no panic: the default action of SIGPIPE. The server
    // commands keep SIGPIPE ignored.
    if matches!(
        command.as_deref(),
        Some(
            "status"
                | "ports"
                | "qr"
                | "man"
                | "completions"
                | "doctor"
                | "net doctor"
                | "net addresses"
                | "net demo"
        )
    ) {
        // SAFETY: signal() with SIG_DFL, before any thread starts.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    let level = if cli.quiet {
        podnet::report::log_filter("warn")
    } else {
        match cli.verbose {
            0 => podnet::report::log_filter("info"),
            1 => podnet::report::log_filter("debug"),
            _ => podnet::report::log_filter("trace"),
        }
    };
    let filter = match std::env::var("RUST_LOG") {
        Ok(value) if !value.is_empty() => EnvFilter::try_new(&value).unwrap_or_else(|e| {
            // Before, a bad RUST_LOG gave the default level with no word.
            eprintln!("podnet: RUST_LOG={value:?} does not parse ({e}); the log level is {level}");
            EnvFilter::new(&level)
        }),
        _ => EnvFilter::new(&level),
    };
    // Colours only on a terminal: a log file or a pipe gets plain text.
    let ansi = match cli.color.as_str() {
        "always" => true,
        "never" => false,
        // A detached instance writes its log to a file (issue 103).
        _ => {
            std::io::IsTerminal::is_terminal(&std::io::stderr())
                && std::env::var_os("NO_COLOR").is_none()
                && !podnet::detach::is_child()
        }
    };
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(podnet::report::TeeStderr);
    if cli.log_format == "json" {
        builder.json().with_ansi(false).init();
    } else {
        builder.with_ansi(ansi).init();
    }
    if let Some(line) = foreground_hint(
        command.as_deref(),
        &argv,
        podnet::detach::is_child(),
        no_terminal(),
    ) {
        tracing::info!("{line}");
    }
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")
        .and_then(|runtime| {
            runtime.block_on(async {
                install_signals();
                async_main(cli).await
            })
        });
    podnet::detach::unregister();
    if let Err(err) = result {
        let code = podnet::exit::code_of(&err);
        let line = podnet::report::error_line(&err);
        // Issues 182 and 210: a supervised child tells its supervisor how
        // long the service asks to wait.
        if let Some(secs) = podnet::quicktunnel::retry_of(&err).and_then(|r| r.retry_after_s) {
            podnet::detach::retry_after(secs);
        }
        // The report has the error in "error"; "messages" holds the log
        // lines before it (issue 55: the line was in both).
        print_error(&line, code);
        podnet::report::send(command.as_deref(), code, Some(&err));
        std::process::exit(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::cmd::agent::agent_commands;
    use crate::cli::cmd::run::idle_watch;
    use crate::cli::cmd::share::{share_mode, ModeArg, ShareArgs};
    use crate::cli::cmd::tunnel::split_host_port;
    use crate::cli::cmd::warp::account::plain_value;
    use crate::cli::cmd::warp::args::non_empty_key;
    use crate::cli::cmd::warp::client::identity_pairs;
    use crate::cli::cmd::warp::probe::parse_door;
    use crate::cli::cmd::warp::up::{warp_up_args, warp_up_tun_args};
    use crate::cli::cmd::warp::WarpCommand;
    use crate::cli::help::{example_path, first_sentence, word_hint, HELP_GROUPS};
    use crate::cli::signals::{deadline_of, until_stop_or};
    use clap::Parser as _;
    use podnet::share::ShareControl;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    /// Issues 253 and 254: a lint of each help screen. No issue numbers of
    /// this project, no `[env: X=]`, no `<a|b>` value name next to a list
    /// of possible values, and each shell of completions.
    #[test]
    fn each_help_screen_passes_the_lint() {
        fn walk(cmd: &clap::Command, path: String, out: &mut Vec<(String, String)>) {
            let mut cmd = cmd.clone();
            out.push((path.clone(), cmd.render_long_help().to_string()));
            for sub in cmd.get_subcommands() {
                if sub.get_name() != "help" {
                    walk(sub, format!("{path} {}", sub.get_name()), out);
                }
            }
        }
        let mut screens = Vec::new();
        let mut root = Cli::command();
        root.build();
        walk(&root, "podnet".into(), &mut screens);
        let mut faults = Vec::new();
        for (path, help) in &screens {
            for line in help.lines() {
                let lower = line.to_ascii_lowercase();
                if let Some(at) = lower.find("issue ") {
                    if lower[at + 6..].starts_with(|c: char| c.is_ascii_digit()) {
                        faults.push(format!("{path}: an issue number: {line}"));
                    }
                }
                if line.contains("=]") && line.contains("[env: ") {
                    faults.push(format!("{path}: an env value bracket: {line}"));
                }
                if let Some(at) = line.find(" <") {
                    let name = &line[at + 2..];
                    if let Some(end) = name.find('>') {
                        let value = &name[..end];
                        if value.contains('|')
                            && !value.contains("--")
                            && !line.trim_start().starts_with("Usage:")
                        {
                            faults.push(format!("{path}: a value name with |: {line}"));
                        }
                    }
                }
            }
        }
        assert!(faults.is_empty(), "{}", faults.join("\n"));
        let completions = &screens
            .iter()
            .find(|(p, _)| p == "podnet completions")
            .unwrap()
            .1;
        assert!(completions.contains("PowerShell"), "{completions}");
        assert!(completions.contains("Elvish"), "{completions}");
    }

    // Issue 271: the idle clock of a share starts when the session serves.
    // A control that was built 1.1 s before does not end a session with
    // --idle-secs 1 at the first check (5 s); in 60 s of the paused clock
    // the watch does not end.
    #[tokio::test(start_paused = true)]
    async fn the_idle_clock_starts_when_the_session_serves() {
        let control = Arc::new(ShareControl::default());
        std::thread::sleep(Duration::from_millis(1100));
        let watch = idle_watch(vec![control], 1);
        let ended = tokio::time::timeout(Duration::from_secs(60), watch).await;
        assert!(ended.is_err(), "the watch ended at the first check");
    }

    /// E1: each command of podnet is in one group of the top help.
    #[test]
    fn each_command_is_in_one_help_group() {
        let cmd = Cli::command();
        let mut names: Vec<String> = cmd
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        names.push("help".into());
        let listed: Vec<&str> = HELP_GROUPS
            .iter()
            .flat_map(|(_, n)| n.iter().copied())
            .collect();
        for name in &names {
            let count = listed.iter().filter(|l| **l == name.as_str()).count();
            assert_eq!(count, 1, "{name} is in {count} groups");
        }
        for name in &listed {
            assert!(names.iter().any(|n| n == name), "{name} is no command");
        }
        let help = top_command().render_help().to_string();
        assert!(help.contains("Start here:"), "{help}");
        assert!(!help.contains("(issue "), "{help}");
        assert_eq!(first_sentence("Test it: the proxy. More"), "Test it");
        assert_eq!(first_sentence("One command (issue 87): x"), "One command");
    }

    /// E12: a server command with no terminal names --detach.
    #[test]
    fn a_foreground_server_with_no_terminal_gets_one_line() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let tunnel = argv(&["podnet", "tunnel", "--url", "http://localhost:1"]);
        let line = foreground_hint(Some("tunnel"), &tunnel, false, true).unwrap();
        assert!(
            line.contains("--detach") && line.contains("--run-for"),
            "{line}"
        );
        assert!(
            foreground_hint(Some("tunnel"), &tunnel, false, false).is_none(),
            "a terminal"
        );
        assert!(
            foreground_hint(Some("tunnel"), &tunnel, true, true).is_none(),
            "a detached child"
        );
        assert!(foreground_hint(Some("status"), &tunnel, false, true).is_none());
        for flag in ["--detach", "--run-for=5", "--exit-on-verify"] {
            let mut with = tunnel.clone();
            with.push(flag.into());
            assert!(
                foreground_hint(Some("tunnel"), &with, false, true).is_none(),
                "{flag}"
            );
        }
        assert!(foreground_hint(Some("relay node"), &tunnel, false, true).is_some());
    }

    /// E9: the arguments of cloudflared for a tunnel.
    #[test]
    fn cloudflared_arguments_work() {
        let run = |a: &[&str]| -> Vec<String> {
            cloudflared_compat(a.iter().map(Into::into).collect())
                .into_iter()
                .map(|o| o.into_string().unwrap())
                .collect()
        };
        assert_eq!(
            run(&["podnet", "--no-autoupdate", "tunnel", "run", "--token", "T"]),
            ["podnet", "tunnel", "--token", "T"]
        );
        assert_eq!(
            run(&["podnet", "tunnel", "--loglevel", "debug", "--url", "u"]),
            ["podnet", "tunnel", "-v", "--url", "u"]
        );
        assert_eq!(
            run(&["podnet", "tunnel", "--loglevel=warn"]),
            ["podnet", "tunnel", "-q"]
        );
        assert_eq!(
            run(&["podnet", "tunnel", "--loglevel", "info"]),
            ["podnet", "tunnel"]
        );
        // Not for another command, and not after --.
        let other = [
            "podnet",
            "warp",
            "run",
            "--",
            "tunnel",
            "run",
            "--no-autoupdate",
        ];
        assert_eq!(run(&other), other);
        let other = ["podnet", "serve", "--name", "tunnel", "--dir", "run"];
        assert_eq!(run(&other), other);
        // A parse of the result works.
        let parsed = top_command().try_get_matches_from(run(&[
            "podnet",
            "tunnel",
            "run",
            "--no-autoupdate",
            "--loglevel",
            "error",
            "--url",
            "http://127.0.0.1:1",
        ]));
        assert!(parsed.is_ok(), "{parsed:?}");
        let err = top_command()
            .try_get_matches_from(["podnet", "tunnel", "--logfile", "x"])
            .unwrap_err();
        let hints = usage_hints(&err);
        assert!(
            hints[0].contains("--logfile is a flag of cloudflared"),
            "{hints:?}"
        );
        assert_eq!(hints.len(), 1, "{hints:?}");
    }

    /// E7: an example is in the help of its command.
    #[test]
    fn the_help_of_a_command_has_its_examples() {
        let help = |path: &[&str]| -> String {
            let mut cmd = top_command();
            for name in path {
                cmd = cmd.find_subcommand(name).unwrap().clone();
            }
            cmd.render_long_help().to_string()
        };
        let tunnel = help(&["tunnel"]);
        assert!(tunnel.contains("Examples:"), "{tunnel}");
        assert!(
            tunnel.contains("podnet tunnel --url http://127.0.0.1:8080"),
            "{tunnel}"
        );
        assert!(!tunnel.contains("podnet warp"), "{tunnel}");
        let node = help(&["relay", "node"]);
        assert!(
            node.contains("podnet relay node --to 127.0.0.1:22"),
            "{node}"
        );
        assert_eq!(
            example_path(&Cli::command(), "podnet warp proxy -r 1080"),
            "podnet warp proxy"
        );
        // Each example belongs to a command (not to podnet alone).
        for e in podnet::manual::facts::EXAMPLES
            .iter()
            .filter(|e| e.command.starts_with("podnet "))
        {
            assert_ne!(
                example_path(&Cli::command(), e.command),
                "podnet",
                "{}",
                e.command
            );
        }
    }

    /// E8: a word that is no command names the command for it.
    #[test]
    fn a_word_that_is_no_command_gets_a_hint() {
        let hint = |w: &str| word_hint(w).unwrap_or_default();
        assert!(hint("8080").contains("podnet tunnel --url http://localhost:8080"));
        assert!(hint("https://example.org").contains("podnet tunnel --url https://example.org"));
        assert!(hint("wss://x.example").contains("podnet connect --local"));
        assert!(hint("/").contains("podnet serve --dir /"));
        assert!(hint("Cargo.toml").contains("podnet share --mode download --file Cargo.toml"));
        assert_eq!(word_hint("tunel"), None);
        assert_eq!(word_hint("123456"), None, "not a port");
    }

    /// E10: an unknown command or flag names the version and the update.
    #[test]
    fn an_unknown_command_names_the_version() {
        let err = top_command()
            .try_get_matches_from(["podnet", "8080"])
            .unwrap_err();
        let lines = usage_hints(&err);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[1].contains(env!("CARGO_PKG_VERSION")), "{lines:?}");
        assert!(lines[1].contains("podnet update --check"), "{lines:?}");
        let err = top_command()
            .try_get_matches_from(["podnet", "tunnel", "--bogus"])
            .unwrap_err();
        assert!(usage_hints(&err)[0].contains("'--bogus'"));
        let err = top_command()
            .try_get_matches_from(["podnet", "tunnel", "--port", "x"])
            .unwrap_err();
        assert!(
            usage_hints(&err).is_empty(),
            "a bad value is no unknown flag"
        );
    }

    fn share_args(argv: &[&str]) -> ShareArgs {
        let cli = Cli::try_parse_from(["podnet", "share"].iter().chain(argv)).unwrap();
        match cli.command {
            Command::Share(args) => args,
            other => panic!("{other:?}"),
        }
    }

    // Issue 118: the mode follows the flags; flags of two modes are an
    // error. Before, --file with no --mode gave an upload session.
    #[test]
    fn the_share_mode_follows_the_flags() {
        let mode = |argv: &[&str]| share_mode(&share_args(argv));
        assert_eq!(mode(&["--file", "f"]).unwrap(), ModeArg::Download);
        assert_eq!(mode(&["--port", "3000"]).unwrap(), ModeArg::Proxy);
        assert_eq!(mode(&["--mode", "upload"]).unwrap(), ModeArg::Upload);
        assert_eq!(mode(&["--dir", "d"]).unwrap(), ModeArg::Upload);
        assert_eq!(
            mode(&["--mode", "download", "--file", "f"]).unwrap(),
            ModeArg::Download
        );
        for bad in [
            // Issue 250: no mode flag is an error.
            &[][..],
            &["--file", "f", "--port", "1"][..],
            &["--mode", "upload", "--file", "f"][..],
            &["--mode", "proxy", "--dir", "d"][..],
        ] {
            let err = mode(bad).unwrap_err();
            assert_eq!(podnet::exit::code_of(&err), 64, "{bad:?}: {err}");
        }
    }

    // The IPv6-only kit, 0002: warp up --tun gives its child the front door
    // and the TLS name.
    #[test]
    fn the_pairs_of_listen_identity() {
        let main = std::path::Path::new("/s/warp.json");
        let pairs = identity_pairs(
            &[
                "tcp://127.0.0.1:40001=/s/b.json".into(),
                "unix:/run/c.sock=/s/c.json".into(),
            ],
            Some(main),
            &["tcp://127.0.0.1:40000".into()],
        )
        .unwrap();
        assert_eq!(pairs.len(), 2);
        // Issue 178: the listener of --listen.
        assert!(identity_pairs(
            &["tcp://127.0.0.1:40000=/s/b.json".into()],
            Some(main),
            &["tcp://127.0.0.1:40000".into()],
        )
        .is_err());
        assert_eq!(pairs[1].1, PathBuf::from("/s/c.json"));
        for bad in [
            vec!["127.0.0.1:40001=/s/warp.json"],
            vec!["127.0.0.1:40001=/s/b.json", "127.0.0.1:40002=/s/b.json"],
            vec!["127.0.0.1:40001=/s/b.json", "127.0.0.1:40001=/s/c.json"],
            vec!["127.0.0.1:40001"],
            vec!["=/s/b.json"],
            vec!["nothing=/s/b.json"],
        ] {
            let specs: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
            assert!(identity_pairs(&specs, Some(main), &[]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_front_doors_of_a_scan() {
        assert_eq!(
            parse_door("162.159.198.1").unwrap().to_string(),
            "162.159.198.1:443"
        );
        assert_eq!(parse_door("162.159.198.1:8443").unwrap().port(), 8443);
        assert_eq!(
            parse_door("2606:4700:103::1").unwrap().to_string(),
            "[2606:4700:103::1]:443"
        );
        assert_eq!(parse_door("[2606:4700:103::1]:443").unwrap().port(), 443);
        assert!(parse_door("engage.cloudflareclient.com").is_err());
        assert!(parse_door("162.159.198.0/24").is_err());
    }

    #[test]
    fn warp_up_tun_keeps_the_front_door() {
        let cli = Cli::try_parse_from([
            "podnet",
            "warp",
            "--masque-endpoint",
            "[2606:4700:103::2]:443",
            "--masque-sni",
            "example.test",
            "--ephemeral",
            "up",
            "--tun",
        ])
        .unwrap();
        let Command::Warp(args) = cli.command else {
            panic!("not warp")
        };
        let argv = warp_up_tun_args(&args, 30, false);
        let has = |a: &str, b: &str| argv.windows(2).any(|w| w[0] == a && w[1] == b);
        assert!(
            has("--masque-endpoint", "[2606:4700:103::2]:443"),
            "{argv:?}"
        );
        assert!(has("--masque-sni", "example.test"), "{argv:?}");
        assert!(argv.contains(&"--ephemeral".to_string()));
        let tun = argv.iter().position(|a| a == "tun").unwrap();
        assert!(argv.iter().position(|a| a == "--masque-endpoint").unwrap() < tun);
        let parsed = Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(argv));
        assert!(parsed.is_ok(), "the argv parses");
    }

    // Issues 110 and 116: a unix-only warp up gets warp exec and
    // ssh-config, and no eval line.
    #[test]
    fn the_agent_commands_follow_the_listeners() {
        let unix_only = agent_commands(true, false);
        assert!(
            !unix_only.iter().any(|c| c.contains("eval")),
            "{unix_only:?}"
        );
        assert!(unix_only.iter().any(|c| c.contains("podnet warp exec")));
        assert!(unix_only.iter().any(|c| c.contains("ssh-config")));
        let tcp = agent_commands(true, true);
        assert!(tcp[0].starts_with("eval"), "{tcp:?}");
        let direct = agent_commands(false, true);
        assert_eq!(direct, ["podnet agent status", "podnet agent down"]);
    }

    /// Issue 101: once the stop is armed (at the ready line), a SIGTERM
    /// ends the work of `until_stop_or` (the verify) with `None`: a clean
    /// stop, not the default action. The only test here that takes
    /// signals: the stop request is global to the process.
    #[test]
    fn a_sigterm_after_the_ready_line_stops_the_verify_cleanly() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            install_signals();
            podnet::detach::arm_stop();
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                // SAFETY: kill() to this process.
                unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
            });
            let out = tokio::time::timeout(
                Duration::from_secs(10),
                until_stop_or(None, std::future::pending::<()>()),
            )
            .await
            .expect("the signal ends the wait");
            assert!(out.is_none());
            // The stop request stays: a later wait ends at once.
            assert!(until_stop_or(None, async { 1 }).await.is_none());
        });
    }

    /// Issue 179: --run-for ends the verify.
    #[tokio::test]
    async fn until_stop_or_ends_at_the_deadline() {
        let deadline = deadline_of(Some(0));
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            until_stop_or(deadline, std::future::pending::<()>()),
        )
        .await
        .expect("the deadline ends the wait");
        assert!(out.is_none());
    }

    /// warp probe targets (issue 67): a name, an IPv4 and an IPv6 address
    /// with a port; no port, port 0 or an empty host is refused.
    #[test]
    fn probe_targets_take_a_name_or_an_address() {
        assert_eq!(
            split_host_port("example.com:443").unwrap(),
            ("example.com".into(), 443)
        );
        assert_eq!(
            split_host_port("1.1.1.1:443").unwrap(),
            ("1.1.1.1".into(), 443)
        );
        assert_eq!(
            split_host_port("[2606:4700::1]:80").unwrap(),
            ("2606:4700::1".into(), 80)
        );
        for bad in ["example.com", "example.com:0", ":443", "a b:1", "host:http"] {
            assert!(split_host_port(bad).is_err(), "{bad}");
        }
    }

    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    /// Each example of the manual is a command line that the parser accepts.
    #[test]
    fn each_example_of_the_manual_parses() {
        for example in podnet::manual::facts::EXAMPLES {
            let words: Vec<&str> = example.command.split_whitespace().collect();
            assert_eq!(words[0], "podnet", "{}", example.command);
            if words.get(1) == Some(&"pod") {
                // pod has its own parser, in podnet-netns.
                continue;
            }
            if let Err(err) = Cli::try_parse_from(&words) {
                panic!("the example {:?} does not parse: {err}", example.command);
            }
        }
    }

    /// The manual names each command and each flag of the command tree.
    #[test]
    fn the_manual_names_each_command_and_flag() {
        let manual = podnet::manual::model::build(Cli::command());
        let text = podnet::manual::text::render(&manual);
        fn walk(cmd: &clap::Command, path: &mut Vec<String>, text: &str) {
            for sub in cmd.get_subcommands().filter(|c| c.get_name() != "help") {
                path.push(sub.get_name().to_string());
                assert!(
                    text.contains(&format!("podnet {}", path.join(" "))),
                    "the manual has no section for podnet {}",
                    path.join(" ")
                );
                for arg in sub.get_arguments().filter(|a| !a.is_hide_set()) {
                    if let Some(long) = arg.get_long() {
                        if long != "help" && long != "version" {
                            assert!(
                                text.contains(&format!("--{long}")),
                                "the manual has no --{long} (podnet {})",
                                path.join(" ")
                            );
                        }
                    }
                }
                walk(sub, path, text);
                path.pop();
            }
        }
        walk(&Cli::command(), &mut Vec::new(), &text);
        assert!(
            text.contains("podnet-netns"),
            "the pod section has the podnet-netns usage"
        );
    }

    #[test]
    fn the_warp_proxy_of_warp_up_parses() {
        let cli =
            Cli::try_parse_from(["podnet", "warp", "--no-relay", "up", "--port", "40100"]).unwrap();
        let Command::Warp(args) = cli.command else {
            panic!("not warp")
        };
        let argv = warp_up_args(&args, 40100, std::path::Path::new("/r/warp.sock"), 30);
        let parsed =
            Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(argv)).unwrap();
        let Command::Warp(inner) = parsed.command else {
            panic!("not warp: the child command line")
        };
        assert!(inner.no_relay);
        match inner.command {
            WarpCommand::Proxy { listen, detach, .. } => {
                assert_eq!(listen, ["tcp://127.0.0.1:40100", "unix:/r/warp.sock"]);
                assert!(detach.detach);
                assert_eq!(detach.name.as_deref(), Some("warp"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_bad_command_line_is_a_usage_error() {
        let err = Cli::try_parse_from(["podnet", "tunel"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
        assert!(
            err.to_string().contains("tunnel"),
            "clap suggests the nearest command"
        );
    }

    #[test]
    fn the_command_name_of_the_report() {
        let argv = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            command_name(&argv("podnet warp run -- curl")).as_deref(),
            Some("warp run")
        );
        assert_eq!(
            command_name(&argv("podnet tunnel --url x")).as_deref(),
            Some("tunnel")
        );
        assert_eq!(command_name(&argv("podnet nothing")), None);
        // Issue 181: options before the command are skipped.
        for (line, want) in [
            ("podnet -q tunnel", "tunnel"),
            ("podnet -vv doctor", "doctor"),
            ("podnet --proxy http://x:1 warp run -- curl", "warp run"),
            (
                "podnet --color never --log-format json net doctor",
                "net doctor",
            ),
            ("podnet --ca-file=a.pem status", "status"),
        ] {
            assert_eq!(command_name(&argv(line)).as_deref(), Some(want), "{line}");
        }
    }

    // Issue 178: an empty license key is a usage error; a text field has no
    // quotes; warp_plus_active follows the account.
    #[test]
    fn warp_license_and_status_fields() {
        let err = Cli::try_parse_from(["podnet", "warp", "license", ""]).unwrap_err();
        assert_eq!(err.exit_code(), 2, "clap error");
        assert!(non_empty_key("  ").is_err());
        assert_eq!(non_empty_key(" k ").unwrap(), "k");
        assert_eq!(plain_value(&serde_json::json!("free")), "free");
        assert_eq!(plain_value(&serde_json::json!(5)), "5");
        assert_eq!(plain_value(&serde_json::Value::Null), "-");
        let free = serde_json::json!({"account_type": "free", "warp_plus": true});
        assert_eq!(podnet::warp::identity::warp_plus_label(&free), "no");
        let plus = serde_json::json!({"account_type": "limited", "premium_data": 10});
        assert_eq!(podnet::warp::identity::warp_plus_label(&plus), "yes");
    }

    #[test]
    fn the_pod_keeps_its_double_dash() {
        let v = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            pod_args_of(&v("podnet -v pod -- true"), &v("true")),
            v("-- true")
        );
        assert_eq!(
            pod_args_of(&v("podnet pod -x u -- a"), &v("-x u -- a")),
            v("-x u -- a")
        );
    }
}
