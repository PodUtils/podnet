//! `podnet link` (T-265, GitHub #466, row E0-4): listen, dial, status,
//! close and carriers. The tests use unix sockets only: they bind no TCP
//! port, so they run on a host that denies `bind(AF_INET)`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// One install: a state directory and a run directory of its own.
struct Install {
    dir: tempfile::TempDir,
}

impl Install {
    fn new() -> Install {
        Install {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_STATE_DIR", self.path("state"))
            .env("PODNET_RUN_DIR", self.path("run"))
            .env_remove("RUST_LOG")
            .env_remove("PODNET_ERROR_PROGRAM");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// The fingerprint of this install (the key is made when it is not).
    fn fingerprint(&self) -> String {
        let out = self.run(&["link", "status", "--create", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["identity"]["fingerprint"].as_str().unwrap().to_string()
    }

    /// An allow list file with `lines`.
    fn allow(&self, name: &str, lines: &[&str]) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    /// Start `link listen` in the foreground and read its ready document.
    fn listener(&self, address: &Path, allow: &Path, more: &[&str]) -> Listener {
        let mut args = vec![
            "link",
            "listen",
            address.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--json",
        ];
        args.extend_from_slice(more);
        let mut child = self
            .command(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        let ready = serde_json::from_str(&line).unwrap_or_else(|e| {
            let _ = child.kill();
            let mut err = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut err);
            panic!("no ready document ({e}): {line:?} {err}")
        });
        Listener { child, ready }
    }
}

struct Listener {
    child: Child,
    ready: Value,
}

impl Listener {
    /// SIGTERM, and the exit status.
    fn stop(&mut self) -> Option<i32> {
        unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) };
        self.child.wait().unwrap().code()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

/// An echo service on a unix socket: it copies each connection back.
fn echo(path: &Path) {
    let listener = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = conn.read(&mut buf) {
                    if n == 0 || conn.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
}

#[test]
fn a_unix_listener_prints_its_0600_path() {
    let host = Install::new();
    let me = host.fingerprint();
    let allow = host.allow("allow", &[&me]);
    let sock = host.path("l.sock");
    let mut l = host.listener(&sock, &allow, &[]);
    assert_eq!(l.ready["carrier"], "unix", "{}", l.ready);
    assert_eq!(l.ready["address"], sock.to_str().unwrap());
    assert_eq!(l.ready["mode"], "0600");
    assert_eq!(l.ready["fingerprint"], me.as_str());
    assert_eq!(l.ready["allow"], 1);
    assert_eq!(mode(&sock), 0o600);
    assert_eq!(l.stop(), Some(0));
    assert!(!sock.exists(), "the socket is removed at the stop");
}

#[test]
fn the_text_form_names_the_path_and_the_mode() {
    let host = Install::new();
    let allow = host.allow("allow", &[&host.fingerprint()]);
    let sock = host.path("t.sock");
    let mut child = host
        .command(&[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert!(line.contains(sock.to_str().unwrap()), "{line}");
    assert!(line.contains("mode 0600"), "{line}");
}

#[test]
fn a_duplicate_listener_exits_69() {
    let host = Install::new();
    let allow = host.allow("allow", &[&host.fingerprint()]);
    let sock = host.path("d.sock");
    let first = host.listener(&sock, &allow, &[]);
    let out = host.run(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    assert!(sock.exists(), "the first listener keeps its socket");
    drop(first);
}

#[test]
fn a_regular_file_at_the_address_exits_64_and_stays() {
    let host = Install::new();
    let allow = host.allow("allow", &[&host.fingerprint()]);
    let file = host.path("data");
    std::fs::write(&file, b"keep").unwrap();
    let out = host.run(&[
        "link",
        "listen",
        file.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
}

#[test]
fn an_allow_file_that_others_can_change_exits_71_and_a_missing_one_66() {
    let host = Install::new();
    let sock = host.path("a.sock");
    let allow = host.allow("allow", &[&host.fingerprint()]);
    std::fs::set_permissions(&allow, std::fs::Permissions::from_mode(0o666)).unwrap();
    let args = |allow: &Path| {
        host.run(&[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
        ])
    };
    assert_eq!(args(&allow).status.code(), Some(71));
    assert_eq!(args(&host.path("none")).status.code(), Some(66));
    assert!(!sock.exists(), "no socket is made before the checks");
}

#[test]
fn a_dialer_in_the_allow_list_reaches_the_service_of_the_listener() {
    let server = Install::new();
    let client = Install::new();
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let origin = server.path("origin.sock");
    echo(&origin);
    let sock = server.path("l.sock");
    let allow = server.allow("allow", &[&cfp]);
    let to = format!("unix:{}", origin.display());
    let _l = server.listener(&sock, &allow, &["--to", &to]);
    let mut dial = client
        .command(&["link", "dial", sock.to_str().unwrap(), "--peer", &sfp])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    dial.stdin
        .take()
        .unwrap()
        .write_all(b"over a link")
        .unwrap();
    let out = dial.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "over a link");
    assert!(text(&out.stderr).contains(&sfp), "{}", text(&out.stderr));
}

#[test]
fn a_dialer_that_the_list_does_not_hold_exits_77() {
    let server = Install::new();
    let client = Install::new();
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let sock = server.path("l.sock");
    let allow = server.allow("allow", &[&sfp]);
    let mut l = server.listener(&sock, &allow, &[]);
    let out = client.run(&[
        "link",
        "dial",
        sock.to_str().unwrap(),
        "--peer",
        &sfp,
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains(&sfp), "{}", text(&out.stderr));
    // The listener runs on, and names the peer that it refused.
    assert_eq!(l.stop(), Some(0));
    let mut err = String::new();
    l.child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(err.contains(&cfp), "{err}");
}

#[test]
fn a_listener_that_the_dialer_does_not_trust_exits_77() {
    let server = Install::new();
    let client = Install::new();
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let stranger = Install::new().fingerprint();
    let sock = server.path("l.sock");
    let allow = server.allow("allow", &[&cfp]);
    let _l = server.listener(&sock, &allow, &[]);
    let out = client.run(&[
        "link",
        "dial",
        sock.to_str().unwrap(),
        "--peer",
        &stranger,
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains(&sfp), "{}", text(&out.stderr));
}

#[test]
fn dial_check_prints_the_fingerprint_of_the_peer() {
    let server = Install::new();
    let client = Install::new();
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let sock = server.path("l.sock");
    let allow = server.allow("allow", &[&cfp]);
    let _l = server.listener(&sock, &allow, &[]);
    let list = client.allow("trusted", &[&sfp]);
    let out = client.run(&[
        "link",
        "dial",
        sock.to_str().unwrap(),
        "--allow",
        list.to_str().unwrap(),
        "--check",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["peer"], sfp.as_str());
    assert_eq!(doc["fingerprint"], cfp.as_str());
    assert_eq!(doc["schema"], 1);
}

#[test]
fn a_dialer_with_no_trusted_listener_exits_64() {
    let client = Install::new();
    let out = client.run(&["link", "dial", "/nonexistent.sock"]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("--peer"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn no_listener_at_the_address_exits_69() {
    let client = Install::new();
    let sfp = Install::new().fingerprint();
    let missing = client.path("none.sock");
    let out = client.run(&[
        "link",
        "dial",
        missing.to_str().unwrap(),
        "--peer",
        &sfp,
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
}

#[test]
fn carriers_lists_each_carrier_with_its_diagnosis() {
    let host = Install::new();
    let out = host.run(&["link", "carriers"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let lines: Vec<String> = text(&out.stdout).lines().map(String::from).collect();
    assert_eq!(lines.len(), 6, "{lines:?}");
    assert!(lines[0].starts_with("unix: stream"), "{}", lines[0]);
    assert!(lines[0].ends_with(": available"), "{}", lines[0]);
    assert!(lines[1].starts_with("memory: stream"), "{}", lines[1]);

    let out = host.run(&["link", "carriers", "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], 1);
    let rows = doc["carriers"].as_array().unwrap();
    assert_eq!(rows[0]["name"], "unix");
    assert_eq!(rows[0]["available"], true);
    assert_eq!(rows[0]["addressing"], "peer");
    assert!(rows[0]["reason"].is_null());
    assert_eq!(rows[1]["name"], "memory");
}

#[test]
fn a_ring_out_of_its_limit_is_a_usage_error_and_a_reason() {
    let host = Install::new();
    let allow = host.allow("allow", &[&host.fingerprint()]);
    let out = host.run(&[
        "link",
        "listen",
        "ring",
        "--allow",
        allow.to_str().unwrap(),
        "--carrier",
        "memory",
        "--ring",
        "100",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    let out = host.run(&["link", "carriers", "--ring", "100", "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["carriers"][1]["available"], false, "{doc}");
    assert!(doc["carriers"][1]["reason"].is_string(), "{doc}");
    let out = host.run(&["link", "carriers", "--ring", "65536", "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["carriers"][1]["available"], true, "{doc}");
}

#[test]
fn an_unknown_carrier_exits_64_and_lists_the_names() {
    let host = Install::new();
    let out = host.run(&["link", "dial", "x", "--carrier", "nope", "--peer", "y"]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    let err = text(&out.stderr);
    assert!(err.contains("unix") && err.contains("memory"), "{err}");
}

#[test]
fn the_flags_allow_other_uid_and_ring_are_in_the_help() {
    let host = Install::new();
    for sub in ["listen", "dial", "carriers"] {
        let out = host.run(&["link", sub, "--help"]);
        let help = text(&out.stdout);
        assert!(help.contains("--allow-other-uid"), "{sub}: {help}");
        assert!(help.contains("--ring"), "{sub}: {help}");
    }
}

#[test]
fn status_shows_the_fingerprint_and_never_the_seed() {
    let host = Install::new();
    let out = host.run(&["link", "status", "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(doc["identity"].is_null(), "no key yet: {doc}");
    assert!(!host.path("state/link-identity.key").exists());
    let fp = host.fingerprint();
    let seed = std::fs::read_to_string(host.path("state/link-identity.key")).unwrap();
    for args in [&["link", "status"][..], &["link", "status", "--json"][..]] {
        let out = host.run(args);
        assert_eq!(out.status.code(), Some(0));
        let shown = text(&out.stdout);
        assert!(shown.contains(&fp), "{shown}");
        assert!(!shown.contains(seed.trim()), "the seed is shown");
    }
}

#[test]
fn a_detached_listener_is_in_ps_logs_status_and_close() {
    let host = Install::new();
    let allow = host.allow("allow", &[&host.fingerprint()]);
    let sock = host.path("bg.sock");
    let out = host.run(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--detach",
        "--name",
        "lk",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let ready: Value = serde_json::from_str(text(&out.stdout).lines().last().unwrap()).unwrap();
    assert_eq!(ready["listen"], sock.to_str().unwrap(), "{ready}");
    assert_eq!(mode(&sock), 0o600);

    let ps: Value = serde_json::from_slice(&host.run(&["ps", "--json"]).stdout).unwrap();
    let row = &ps["instances"][0];
    assert_eq!(row["name"], "lk", "{ps}");
    assert_eq!(row["command"], "link listen", "{ps}");

    // A peer that the list does not hold: the log of the instance names it.
    let stranger = Install::new();
    let sfp = stranger.fingerprint();
    let listener_fp = host.fingerprint();
    let out = stranger.run(&[
        "link",
        "dial",
        sock.to_str().unwrap(),
        "--peer",
        &listener_fp,
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
    let deadline = Instant::now() + Duration::from_secs(10);
    let log = loop {
        let logs = host.run(&["logs", "lk"]);
        assert_eq!(logs.status.code(), Some(0));
        let log = text(&logs.stdout);
        if log.contains(&sfp) || Instant::now() > deadline {
            break log;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(log.contains("peer refused") && log.contains(&sfp), "{log}");

    let status: Value =
        serde_json::from_slice(&host.run(&["link", "status", "--json"]).stdout).unwrap();
    assert_eq!(status["links"][0]["name"], "lk", "{status}");
    let text_form = text(&host.run(&["link", "status", "lk"]).stdout);
    assert!(
        text_form.contains("lk") && text_form.contains("ready"),
        "{text_form}"
    );

    let out = host.run(&["link", "close", "lk"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let deadline = Instant::now() + Duration::from_secs(10);
    while sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!sock.exists(), "close removes the socket");
    let ps: Value = serde_json::from_slice(&host.run(&["ps", "--json"]).stdout).unwrap();
    assert!(ps["instances"].as_array().unwrap().is_empty(), "{ps}");
    assert_eq!(host.run(&["link", "close", "lk"]).status.code(), Some(69));
}

/// A dialer of another uid (65534) is refused by the listener unless the
/// listener has `--allow-other-uid`. The test needs root.
#[test]
fn a_dialer_of_another_uid_needs_allow_other_uid() {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: geteuid has no argument and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        super::test_host::skip_line("the test needs root to run a dialer as another uid");
        return;
    }
    let open = |path: &Path| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o777)).unwrap()
    };
    let server = Install::new();
    let client = Install::new();
    open(server.dir.path());
    open(client.dir.path());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    // The key of the client belongs to root; the dialer of uid 65534 makes
    // its own state directory.
    let state = client.path("other-state");
    std::fs::create_dir(&state).unwrap();
    std::os::unix::fs::chown(&state, Some(65534), Some(65534)).unwrap();
    let allow = server.allow("allow", &[]);
    // Each end checks the uid of the other end: the dialer needs the flag
    // too.
    let dialer = |sock: &Path, fingerprint: &str, flag: &[&str]| {
        let mut args = vec![
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            fingerprint,
            "--check",
        ];
        args.extend_from_slice(flag);
        let mut cmd = client.command(&args);
        cmd.env("PODNET_STATE_DIR", &state);
        // SAFETY: setuid is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setgid(65534) != 0 || libc::setuid(65534) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        cmd.output().unwrap()
    };
    let other = dialer(&server.path("none.sock"), &sfp, &[]);
    // The dialer of uid 65534 has its own key; its fingerprint is in the
    // state directory.
    assert_eq!(other.status.code(), Some(69), "{}", text(&other.stderr));
    assert!(state.join("link-identity.key").exists());
    let mine = {
        let mut cmd = client.command(&["link", "status", "--json"]);
        cmd.env("PODNET_STATE_DIR", &state);
        // SAFETY: setuid is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setgid(65534) != 0 || libc::setuid(65534) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let out = cmd.output().unwrap();
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["identity"]["fingerprint"].as_str().unwrap().to_string()
    };
    let _ = cfp;
    std::fs::write(&allow, mine).unwrap();
    let denied = server.path("denied.sock");
    let allowed = server.path("allowed.sock");
    let l1 = server.listener(&denied, &allow, &[]);
    let l2 = server.listener(&allowed, &allow, &["--allow-other-uid"]);
    open(&denied);
    open(&allowed);
    let out = dialer(&denied, &sfp, &["--allow-other-uid"]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    let out = dialer(&allowed, &sfp, &[]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    let out = dialer(&allowed, &sfp, &["--allow-other-uid"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    drop((l1, l2));
}

#[test]
fn close_ends_a_link_instance_only() {
    let host = Install::new();
    let out = host.run(&["link", "close", "nobody"]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    let switch = format!("unix:{}", host.path("sw.sock").display());
    let out = host.run(&[
        "net", "switch", "--listen", &switch, "--detach", "--name", "sw",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let out = host.run(&["link", "close", "sw"]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("podnet stop sw"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(host.run(&["stop", "sw"]).status.code(), Some(0));
}
