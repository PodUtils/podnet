//! `tunnel --config` refuses the origin flags (T-204).

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap()
}

#[test]
fn tunnel_config_with_origin_flag_is_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.yml");
    std::fs::write(&file, "ingress:\n  - service: http_status:404\n").unwrap();
    let file = file.to_str().unwrap();
    for (flags, named) in [
        (vec!["--port", "8080"], "--port"),
        (vec!["--forward", "tcp://127.0.0.1:22"], "--forward"),
        (vec!["--no-tls-verify"], "--no-tls-verify"),
    ] {
        let mut args = vec!["tunnel", "--config", file];
        args.extend(flags);
        let out = run(&args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(64), "{err}");
        assert!(err.contains(named), "{err}");
        assert!(err.contains("ingress"), "{err}");
    }
}
