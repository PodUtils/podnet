//! Run documented examples in fresh temporary directories (T-628).

mod docs_examples {
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn extract_uploads_block(doc: &str) -> Vec<String> {
        let section = doc
            .split("### Uploads")
            .nth(1)
            .expect("docs/serve.md has ### Uploads");
        let block = section
            .split("```sh\n")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("### Uploads has a fenced sh block");
        block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect()
    }

    fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn path_with_podnet(bin: &Path) -> String {
        let bin_dir = bin.parent().unwrap().display();
        let rest = std::env::var("PATH").unwrap_or_default();
        format!("{bin_dir}:{rest}")
    }

    #[test]
    fn the_upload_example_of_serve_md_runs_as_written() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let doc = std::fs::read_to_string(root.join("docs/serve.md")).unwrap();
        let lines = extract_uploads_block(&doc);
        assert!(
            lines.len() >= 2,
            "expected upload example commands, got {lines:?}"
        );

        let dir = tempfile::tempdir().unwrap();
        let report_body = b"%PDF-1.4 a report\n";
        std::fs::write(dir.path().join("report.pdf"), report_body).unwrap();

        let bin = Path::new(env!("CARGO_BIN_EXE_podnet"));
        let path_env = path_with_podnet(bin);
        let password = "bob-upload-password";
        let port = free_port();
        let addr = format!("127.0.0.1:{port}");

        let serve_idx = lines
            .iter()
            .position(|l| l.starts_with("podnet serve ") && l.contains("--upload"))
            .expect("upload block has a podnet serve --upload line");
        let upload_idx = lines
            .iter()
            .position(|l| l.starts_with("curl "))
            .expect("upload block has a curl upload line");
        assert!(serve_idx < upload_idx, "serve must come before curl");

        // Run every setup step written before the server command.
        for step in &lines[..serve_idx] {
            let out = Command::new("sh")
                .args(["-ec", step])
                .current_dir(dir.path())
                .env("PATH", &path_env)
                .env("PASSWORD", password)
                .env("HOME", dir.path())
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "setup step failed: {step}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }

        // Start the documented server on a free loopback port.
        let serve_cmd = lines[serve_idx].replace("127.0.0.1:8080", &addr);
        let exec_serve = format!("exec {serve_cmd}");
        let mut server = Command::new("sh")
            .args(["-ec", &exec_serve])
            .current_dir(dir.path())
            .env("PATH", &path_env)
            .env("PASSWORD", password)
            .env("HOME", dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut ready = false;
        while Instant::now() < deadline {
            if let Some(status) = server.try_wait().unwrap() {
                let out = server.wait_with_output().unwrap();
                panic!(
                    "server exited early ({status}) for `{serve_cmd}`\nstdout: {}\nstderr: {}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            if TcpStream::connect(&addr).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(ready, "server did not listen on {addr}");

        // Run the documented upload command with bob's password supplied.
        let upload_cmd = lines[upload_idx]
            .replace("127.0.0.1:8080", &addr)
            .replace(" -u bob ", &format!(" --max-time 10 -u bob:{password} "));
        let upload_out = Command::new("sh")
            .args(["-ec", &upload_cmd])
            .current_dir(dir.path())
            .env("PATH", &path_env)
            .env("HOME", dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();

        let _ = server.kill();
        let server_out = server.wait_with_output().unwrap();

        assert!(
            upload_out.status.success(),
            "upload command failed: {upload_cmd}\nstdout: {}\nstderr: {}\nserver stderr: {}",
            String::from_utf8_lossy(&upload_out.stdout),
            String::from_utf8_lossy(&upload_out.stderr),
            String::from_utf8_lossy(&server_out.stderr),
        );

        let landed = dir.path().join("share/inbox/report.pdf");
        assert_eq!(
            std::fs::read(&landed).expect("uploaded file exists at share/inbox/report.pdf"),
            report_body
        );
    }
}
