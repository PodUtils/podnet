use clap::Parser;

use super::*;
use crate::cli::{Cli, Command};

fn warp_args(extra: &[&str]) -> Result<WarpArgs, clap::Error> {
    let mut argv = vec!["podnet", "warp", "up"];
    argv.extend_from_slice(extra);
    match Cli::try_parse_from(argv)?.command {
        Command::Warp(args) => Ok(*args),
        _ => panic!("not warp"),
    }
}

#[test]
fn warp_up_tun_args_replace_only_when_set() {
    let args = warp_args(&["--tun"]).unwrap();
    let plain = warp_up_tun_args(&args, 30, false);
    assert!(!plain.contains(&"--replace".to_string()), "{plain:?}");
    let replaced = warp_up_tun_args(&args, 30, true);
    assert!(replaced.contains(&"--replace".to_string()), "{replaced:?}");
    let parsed = Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(replaced));
    assert!(parsed.is_ok(), "the argv parses");
}

#[test]
fn warp_up_tun_refuses_the_proxy_flags() {
    for flags in [
        &["--tun", "--env-d"][..],
        &["--tun", "--dns", "127.0.0.1:5353"],
        &["--tun", "--proxy-user", "u", "--proxy-password-file", "/f"],
    ] {
        let err = warp_args(flags)
            .err()
            .unwrap_or_else(|| panic!("{flags:?}"));
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
    assert!(warp_args(&["--tun", "--replace"]).is_ok());
}
