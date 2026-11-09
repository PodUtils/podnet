use crate::cli::cmd::share::{share_mode, ModeArg, ShareArgs};
use crate::cli::{Cli, Command};
use clap::Parser as _;

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
