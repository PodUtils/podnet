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
    // Issue 210: the error report gives the refusal of a quick tunnel.
    podnet::report::read_refusals_with(podnet::quicktunnel::retry_of);
    let args = cloudflared_compat(args);
    // T-060: each --egress becomes its old flag (--no-proxy, --proxy,
    // --transport relay, or -x of the pod).
    let args = match cli::way::egress_aliases(args) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("podnet: {err}");
            std::process::exit(podnet::exit::USAGE);
        }
    };
    let argv: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if let Err(err) = apply_token_files(&argv) {
        eprintln!("podnet: {err:#}");
        std::process::exit(podnet::exit::USAGE);
    }
    if argv.get(1).map(String::as_str) == Some("pod") {
        pod_names();
        std::process::exit(podnet_netns::main_with_args(&argv[2..]));
    }
    // T-222: `--help --json` prints the JSON of `podnet man --json` for the
    // command.
    if argv.iter().any(|a| a == "--json") {
        use clap::CommandFactory;
        let manual = podnet::manual::model::build(Cli::command());
        if let Some(request) = podnet::manual::help_json_request(&manual, &argv) {
            match podnet::manual::body(&manual, &request) {
                Ok(text) => {
                    print!("{text}");
                    std::process::exit(podnet::exit::SUCCESS);
                }
                Err(err) => {
                    eprintln!("podnet: {err}");
                    std::process::exit(podnet::exit::USAGE);
                }
            }
        }
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
        pod_names();
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
    // T-308: the name of a live link.
    if let (Command::Link(_), Some(detach)) = (&cli.command, detach_of(&cli.command)) {
        if let (Some(name), false) = (&detach.name, podnet::detach::is_child()) {
            let checked = match podnet::instances::run_dir() {
                Ok(dir) => podnet::link_alias::precheck(&dir, name, detach.replace),
                Err(_) => podnet::link_alias::check(name),
            };
            if let Err(err) = checked {
                eprintln!("podnet: {err:#}");
                std::process::exit(podnet::exit::code_of(&err));
            }
        }
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
                | "components"
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

/// The names of `podnet pod` (T-135): its messages start with
/// `podnet pod:`, and `--version` prints the version line of podnet.
fn pod_names() {
    podnet_netns::set_program_name("podnet pod");
    podnet_netns::set_version_line(concat!(
        "podnet ",
        env!("CARGO_PKG_VERSION"),
        " (",
        env!("PODNET_BUILD_COMMIT"),
        ")"
    ));
}
