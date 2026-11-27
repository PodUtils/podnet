//! T-632 (GitHub #798): `podnet --log-format json pod ...` writes each
//! `podnet pod:` stderr line as one JSON object, with `exit_code` on the final
//! error line, while leaving plain-text mode and the child program's output
//! unchanged.

mod pod_log_format {
    use std::process::Command;

    fn podnet() -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        for key in [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "NO_PROXY",
            "no_proxy",
            "RUST_LOG",
            "PODNET_ERROR_PROGRAM",
            "PODNET_NETNS_QUIET",
            "PODNET_RESOLVE",
        ] {
            cmd.env_remove(key);
        }
        cmd
    }

    fn parse_json_lines(stderr: &str) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for line in stderr.lines() {
            assert!(
                !line.is_empty(),
                "unexpected empty stderr line in:\n{stderr}"
            );
            let val: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("stderr line is not valid JSON ({e}): {line:?}\nfull stderr:\n{stderr}")
            });
            assert!(val.is_object(), "expected JSON object: {line}");
            assert!(val["timestamp"].is_string(), "missing timestamp: {line}");
            assert!(val["level"].is_string(), "missing level: {line}");
            assert!(
                val["fields"]["message"].is_string(),
                "missing fields.message: {line}"
            );
            out.push(val);
        }
        out
    }

    #[test]
    fn each_podnet_pod_line_is_json_with_log_format_json() {
        // 1. Usage error: no proxy (exit 64).
        let out = podnet()
            .args(["--log-format", "json", "pod", "--", "/bin/true"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64));
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines = parse_json_lines(&stderr);
        assert!(!lines.is_empty(), "expected error line on stderr");
        let last = lines.last().unwrap();
        assert_eq!(last["level"], "ERROR", "{last}");
        assert_eq!(last["fields"]["exit_code"], 64, "{last}");
        assert!(
            last["fields"]["message"]
                .as_str()
                .unwrap()
                .contains("no proxy"),
            "{last}"
        );

        // 2. Parse error on pod flag (exit 64).
        let out = podnet()
            .args([
                "--log-format",
                "json",
                "pod",
                "--no-such-flag",
                "--",
                "/bin/true",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64));
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines = parse_json_lines(&stderr);
        assert_eq!(lines.last().unwrap()["fields"]["exit_code"], 64);

        // 3. Global proxy option refused by pod (exit 64).
        let out = podnet()
            .args([
                "--log-format",
                "json",
                "--proxy",
                "http://127.0.0.1:1",
                "pod",
                "--",
                "/bin/true",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64));
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines = parse_json_lines(&stderr);
        assert_eq!(lines.last().unwrap()["fields"]["exit_code"], 64);

        // 4. Bad PODNET_RESOLVE variable (exit 78).
        let out = podnet()
            .env("PODNET_RESOLVE", "not-valid")
            .args([
                "--log-format",
                "json",
                "pod",
                "-x",
                "direct",
                "--",
                "/bin/true",
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(78));
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines = parse_json_lines(&stderr);
        assert_eq!(lines.last().unwrap()["fields"]["exit_code"], 78);

        // 5. Dead upstream warning + --name-backend line; child stdout stays plain text.
        let dead_sock =
            std::env::temp_dir().join(format!("podnet-t632-dead-{}.sock", std::process::id()));
        let dead_spec = format!("socks5h+unix:{}", dead_sock.display());
        let out = podnet()
            .args([
                "--log-format",
                "json",
                "pod",
                "--name-backend",
                "-x",
                &dead_spec,
                "--",
                "/bin/echo",
                "child-plain-output",
            ])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines = parse_json_lines(&stderr);
        assert!(
            lines.iter().any(|v| v["fields"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("does not answer"))),
            "expected dead-upstream warning in JSON stderr:\n{stderr}"
        );
        assert!(
            lines.iter().any(|v| v["fields"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("backend:"))),
            "expected --name-backend line in JSON stderr:\n{stderr}"
        );
        if out.status.success() {
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                "child-plain-output\n",
                "child stdout must stay untouched"
            );
        } else {
            let code = out.status.code().unwrap();
            assert_eq!(lines.last().unwrap()["fields"]["exit_code"], code);
        }

        // 6. Refused rule line (when a backend can run on this host) is JSON too.
        let out = podnet()
            .args([
                "--log-format",
                "json",
                "pod",
                "-r",
                "cidr:192.0.2.0/24=reject",
                "-x",
                "direct",
                "--",
                "python3",
                "-c",
                "import socket\ns = socket.socket()\ns.settimeout(1)\ntry:\n s.connect(('192.0.2.1', 80))\nexcept OSError:\n pass\n",
            ])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines = parse_json_lines(&stderr);
        if out.status.success() {
            assert!(
                lines.iter().any(|v| v["fields"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("refused"))),
                "expected refused-rule JSON line in stderr:\n{stderr}"
            );
        }

        // 7. Plain-text default remains unchanged.
        let out = podnet().args(["pod", "--", "/bin/true"]).output().unwrap();
        assert_eq!(out.status.code(), Some(64));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.starts_with("podnet pod: no proxy:"),
            "plain text form must stay unchanged, got: {stderr:?}"
        );
    }
}
