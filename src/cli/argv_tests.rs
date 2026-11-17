use crate::cli::argv::{
    cloudflared_compat, command_name, foreground_hint, pod_args_of, SERVER_COMMANDS,
};
use crate::cli::help::{top_command, usage_hints};

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

/// Issue 617: a mode that ends alone (a check, a hash, one stream) gets no
/// line; the same command that stays up gets it.
#[test]
fn a_mode_that_ends_alone_gets_no_foreground_line() {
    let argv = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
    for (command, line) in [
        ("serve", "podnet serve --hash-password"),
        ("tunnel", "podnet tunnel --hello-world --check"),
        ("serve", "podnet serve www --check"),
        ("share", "podnet share --mode download --file f --check"),
        ("connect", "podnet connect wss://h/x --stdio"),
        (
            "share",
            "podnet share --mode download --file f --stop-after",
        ),
    ] {
        assert!(
            foreground_hint(Some(command), &argv(line), false, true).is_none(),
            "{line}"
        );
    }
    for (command, line) in [
        ("serve", "podnet serve www"),
        ("tunnel", "podnet tunnel --hello-world"),
        ("connect", "podnet connect wss://h/x -L 127.0.0.1:2222"),
    ] {
        assert!(
            foreground_hint(Some(command), &argv(line), false, true).is_some(),
            "{line}"
        );
    }
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

/// GitHub #647: the foreground line names --run-for, so each server command
/// takes it. The help of --no-restart names --stop-after and --idle-secs
/// only as flags of share.
#[test]
fn each_server_command_takes_the_flags_of_its_hints() {
    use clap::CommandFactory as _;
    let root = crate::cli::Cli::command();
    for path in SERVER_COMMANDS {
        let mut cmd = &root;
        for word in path.split(' ') {
            cmd = cmd
                .find_subcommand(word)
                .unwrap_or_else(|| panic!("{path}"));
        }
        let has = |flag: &str| cmd.get_arguments().any(|a| a.get_long() == Some(flag));
        assert!(has("run-for"), "{path} takes no --run-for");
        let help = cmd
            .get_arguments()
            .find(|a| a.get_long() == Some("no-restart"))
            .and_then(|a| a.get_help())
            .map(ToString::to_string)
            .unwrap_or_default();
        for flag in ["stop-after", "idle-secs"] {
            if help.contains(&format!("--{flag}")) && !has(flag) {
                assert!(help.contains("of share)"), "{path}: {help}");
            }
        }
    }
}
