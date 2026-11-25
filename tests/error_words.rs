//! T-521 and T-522 (GitHub #695 to #698): the error line of a command that
//! dials, and the rows of `--check`. The rule (`podnet man exit-status`,
//! the error line): one line that names each cause once, with no Debug form
//! of a Rust value, then one line that says what to do, and the documented
//! exit code.
//!
//! The causes are local: a dead proxy (127.0.0.1:1), a refused port, a name
//! under `.invalid`, the address 192.0.2.1 (TEST-NET-1: no answer, or no
//! route), and a quick-tunnel service that answers with a page that is not
//! its API (a stand-in on 127.0.0.1). No row needs the network outside.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::{Command, Stdio};

mod test_host;

/// The Debug forms of Rust values that a line for a person does not hold.
const DEBUG_FORMS: [&str; 7] = [
    "DnsName(",
    "Os {",
    "Custom {",
    "kind:",
    "Query {",
    "Name(\"",
    "IpAddress(",
];

/// The second line of each hop (`crates/podnet-core/src/hop.rs`) and of
/// the quick-tunnel service (`src/report.rs`).
const PROXY: &str = "podnet: the proxy hop failed; ";
const DNS: &str = "podnet: the dns hop failed; ";
const CONNECT: &str = "podnet: the connect hop failed; ";
const SERVICE: &str = "podnet: the quick-tunnel service did not answer with its API";

/// One command and one cause.
struct Row {
    name: &'static str,
    args: Vec<String>,
    /// The documented exit code.
    code: i32,
    /// The start of the second line.
    hint: &'static str,
    /// A text that each line names at most one time (a target, a lead).
    once: &'static [&'static str],
}

fn row(
    name: &'static str,
    args: &[&str],
    code: i32,
    hint: &'static str,
    once: &'static [&'static str],
) -> Row {
    Row {
        name,
        args: args.iter().map(|a| a.to_string()).collect(),
        code,
        hint,
        once,
    }
}

fn podnet(args: &[String], home: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.args(args);
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "PODNET_PROXY",
        "PODNET_RESOLVE",
        "PODNET_ERROR_PROGRAM",
        "RUST_LOG",
    ] {
        command.env_remove(name);
    }
    command
        .env("HOME", home)
        .env("PODNET_RUN_DIR", home.join("run"))
        .env("PODNET_UDP", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// A quick-tunnel service that answers each request with 200 and a page
/// that is not JSON, as the proxy of #695 did.
fn page_service() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut head = [0u8; 4096];
            let _ = stream.read(&mut head);
            let body = "pod-lab stand-in";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

/// The parts of `line` (joined with `: `) that come again at once.
fn repeated_part(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split(": ").collect();
    for i in 0..parts.len() {
        for n in 1..=(parts.len() - i) / 2 {
            if parts[i..i + n] == parts[i + n..i + 2 * n] {
                return Some(parts[i..i + n].join(": "));
            }
        }
    }
    None
}

/// The faults of one line for a person: a part said two times, a Debug
/// form, a text of `once` said more than one time.
fn faults_of_words(line: &str, once: &[&str]) -> Vec<String> {
    let mut faults = Vec::new();
    if let Some(part) = repeated_part(line) {
        faults.push(format!("says {part:?} two times"));
    }
    for form in DEBUG_FORMS {
        if line.contains(form) {
            faults.push(format!("has the Debug form {form:?}"));
        }
    }
    for text in once.iter().chain(&["could not connect to"]) {
        let n = line.matches(text).count();
        if n > 1 {
            faults.push(format!("says {text:?} {n} times"));
        }
    }
    faults
}

/// GitHub #695 to #698: for each command that dials and each cause, the
/// error line names each cause once with no Debug form, the second line is
/// the hint of the true hop, and the exit code is the documented one.
#[test]
fn each_error_line_names_its_cause_once_and_says_what_to_do() {
    test_host::need_loopback!();
    let home = tempfile::tempdir().unwrap();
    let service = page_service();
    let sock = home.path().join("w.sock");
    let warp_listen = format!("unix:{}", sock.display());
    let dead = ["--proxy", "http://127.0.0.1:1"];
    let ws = "wss://example.com/__podnet/ws";
    // (the row: the command, the exit code, the hint, the texts said once,
    // the bound of the length)
    let rows = vec![
        row(
            "connect, a dead proxy",
            &[&dead[..], &["connect", "--stdio", ws]].concat(),
            69,
            PROXY,
            &["example.com:443", "could not reach the proxy"],
        ),
        row(
            "relay connect, a dead proxy",
            &[&dead[..], &["relay", "connect", "--token", "x", "qa-row"]].concat(),
            69,
            PROXY,
            &["could not reach the proxy"],
        ),
        row(
            "bench, a dead proxy",
            &[
                &dead[..],
                &["bench", "--duration", "1", "http://example.com/"],
            ]
            .concat(),
            1,
            PROXY,
            &["example.com:80", "could not reach the proxy"],
        ),
        row(
            "send, a dead proxy",
            &[&dead[..], &["send", "--text", "hi"]].concat(),
            69,
            PROXY,
            &[],
        ),
        row(
            "warp proxy, a dead proxy",
            &[
                &dead[..],
                &[
                    "warp",
                    "proxy",
                    "--ephemeral",
                    "--listen",
                    warp_listen.as_str(),
                    "--run-for",
                    "5",
                ],
            ]
            .concat(),
            69,
            PROXY,
            &["could not reach the proxy"],
        ),
        row(
            "connect, a refused port",
            &[
                "--no-proxy",
                "connect",
                "--stdio",
                "wss://127.0.0.1:1/__podnet/ws",
            ],
            69,
            CONNECT,
            &[],
        ),
        row(
            "relay connect, a refused port",
            &[
                "--no-proxy",
                "relay",
                "connect",
                "--relay",
                "https://127.0.0.1:1",
                "--token",
                "x",
                "qa-row",
            ],
            69,
            CONNECT,
            &[],
        ),
        row(
            "bench, a refused port",
            &[
                "--no-proxy",
                "bench",
                "--duration",
                "1",
                "http://127.0.0.1:1/",
            ],
            1,
            CONNECT,
            &[],
        ),
        row(
            "connect, a name that does not resolve",
            &[
                "--no-proxy",
                "connect",
                "--stdio",
                "wss://nowhere.invalid/__podnet/ws",
            ],
            69,
            DNS,
            &[],
        ),
        row(
            "relay connect, a name that does not resolve",
            &[
                "--no-proxy",
                "relay",
                "connect",
                "--relay",
                "https://nowhere.invalid",
                "--token",
                "x",
                "qa-row",
            ],
            69,
            DNS,
            &[],
        ),
        row(
            "bench, a name that does not resolve",
            &[
                "--no-proxy",
                "bench",
                "--duration",
                "1",
                "--timeout",
                "3",
                "http://nowhere.invalid/",
            ],
            1,
            DNS,
            &[],
        ),
        row(
            "bench, an address with no answer",
            &[
                "--no-proxy",
                "bench",
                "--duration",
                "1",
                "--timeout",
                "2",
                "http://192.0.2.1/",
            ],
            1,
            CONNECT,
            &[],
        ),
        row(
            "tunnel, a service page that is not its API",
            &[
                "--no-proxy",
                "tunnel",
                "--service-url",
                service.as_str(),
                "--url",
                "http://127.0.0.1:1",
                "--run-for",
                "5",
            ],
            69,
            SERVICE,
            &[],
        ),
    ];
    let runs: Vec<_> = rows
        .into_iter()
        .map(|row| {
            let mut command = podnet(&row.args, home.path());
            std::thread::spawn(move || (row, command.output().unwrap()))
        })
        .collect();
    let mut faults = Vec::new();
    for run in runs {
        let (row, out) = run.join().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        // A log line of a crate (a timestamp first) is not an error line.
        let lines: Vec<&str> = stderr
            .lines()
            .filter(|l| l.starts_with("podnet: "))
            .collect();
        let mut fault = |why: String| faults.push(format!("{}: {why}\n{stderr}", row.name));
        if out.status.code() != Some(row.code) {
            fault(format!("exit {:?}, not {}", out.status.code(), row.code));
        }
        let Some(first) = lines.first() else {
            fault("no error line".into());
            continue;
        };
        for why in faults_of_words(first, row.once) {
            fault(why);
        }
        // T-522: each error line is shorter than 300 bytes, also for a name
        // that no resolver answers (pod-dns v0.83.0).
        if first.len() >= 300 {
            fault(format!("a line of {} bytes", first.len()));
        }
        match lines.get(1) {
            Some(hint) if hint.starts_with(row.hint) => {}
            other => fault(format!("the second line is {other:?}, not {:?}", row.hint)),
        }
        if lines.iter().any(|l| l.contains("edge-register")) {
            fault("names the edge-register hop".into());
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n\n"));
}

/// GitHub #696: each row of `tunnel --check` and `serve --check` names its
/// cause once, with no Debug form: a refused origin, an origin name that
/// does not resolve, and the row of the edge (the system resolver of the
/// host; with no SRV records, the row of #696).
#[test]
fn each_row_of_a_check_names_its_cause_once() {
    test_host::need_loopback!();
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("public");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "hi").unwrap();
    let dir = dir.display().to_string();
    // (the command, the exit code, the row that fails)
    let checks: [(&[&str], i32, Option<&str>); 3] = [
        (
            &["tunnel", "--check", "--url", "http://127.0.0.1:1"],
            1,
            Some("origin 1"),
        ),
        (
            &["tunnel", "--check", "--url", "http://nowhere.invalid:80"],
            1,
            Some("origin 1"),
        ),
        (
            &["serve", "--check", "--dir", dir.as_str(), "--spa"],
            0,
            None,
        ),
    ];
    let mut faults = Vec::new();
    for (args, code, failing) in checks {
        let mut all = vec!["--no-proxy".to_string()];
        all.extend(args.iter().map(|a| a.to_string()));
        let out = podnet(&all, home.path()).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let name = args.join(" ");
        if out.status.code() != Some(code) {
            faults.push(format!("{name}: exit {:?}, not {code}", out.status.code()));
        }
        let rows: Vec<&str> = stdout
            .lines()
            .chain(stderr.lines())
            .filter(|l| l.starts_with("  ") || l.starts_with("podnet: "))
            .collect();
        if let Some(failing) = failing {
            if !rows
                .iter()
                .any(|r| r.trim_start().starts_with("fail") && r.contains(failing))
            {
                faults.push(format!("{name}: no fail row {failing}\n{stdout}"));
            }
        }
        for row in &rows {
            for why in faults_of_words(row, &[]) {
                faults.push(format!("{name}: {why}: {row}"));
            }
            if row.len() >= 300 {
                faults.push(format!("{name}: a row of {} bytes: {row}", row.len()));
            }
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
