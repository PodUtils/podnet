//! The tests of git hosting (T-093). The SSH tests use a unix socket, so
//! they need no loopback interface. A test that needs `git` is skipped,
//! with one line, when `git` is not on `PATH`.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{Config, Handle, Handler};
use russh::keys::ssh_key::rand_core::UnwrapErr;
use russh::keys::ssh_key::{Algorithm, PrivateKey};
use russh::keys::PrivateKeyWithHashAlg;
use russh::ChannelMsg;
use ssh_key::getrandom::SysRng;

use super::ssh::{self, Options, ANON_USER};
use super::{parse_exec, repo_dir, Service};

/// Skip the test, with one line, when `git` is not on `PATH`.
macro_rules! need_git {
    () => {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipped: git is not on PATH");
            return;
        }
    };
}

#[test]
fn parse_exec_takes_the_git_commands_only() {
    assert_eq!(
        parse_exec("git-upload-pack '/a/b.git'"),
        Ok((Service::UploadPack, "/a/b.git".to_string()))
    );
    assert_eq!(
        parse_exec("git-receive-pack 'it'\\''s'"),
        Ok((Service::ReceivePack, "it's".to_string()))
    );
    assert_eq!(
        parse_exec("git-upload-archive repo"),
        Ok((Service::UploadArchive, "repo".to_string()))
    );
    for bad in [
        "ls",
        "sh -c id",
        "git-upload-pack",
        "git-upload-pack ''",
        "git-upload-pack 'a' 'b'",
        "git-upload-pack 'a",
        "git upload-pack 'a'",
        "git-upload-pack 'a\nb'",
    ] {
        assert!(parse_exec(bad).is_err(), "{bad:?} is not refused");
    }
}

#[test]
fn repo_dir_stays_in_the_root() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("root");
    std::fs::create_dir_all(root.join("a.git")).unwrap();
    std::fs::create_dir_all(root.join("team/b")).unwrap();
    std::fs::create_dir_all(base.path().join("outside")).unwrap();
    std::os::unix::fs::symlink(base.path().join("outside"), root.join("link")).unwrap();
    let real = root.canonicalize().unwrap();
    assert_eq!(repo_dir(&root, "/a.git").unwrap(), real.join("a.git"));
    assert_eq!(repo_dir(&root, "a").unwrap(), real.join("a.git"));
    assert_eq!(repo_dir(&root, "team/b/").unwrap(), real.join("team/b"));
    for bad in [
        "",
        "/",
        "../root/a.git",
        "a.git/..",
        "./a.git",
        "team//b",
        ".git",
        "~/a.git",
        "link",
        "none",
    ] {
        assert!(repo_dir(&root, bad).is_err(), "{bad:?} is not refused");
    }
}

#[test]
fn host_key_is_made_once_with_mode_0600() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("git_host_ed25519");
    let first = ssh::load_or_create_host_key(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let second = ssh::load_or_create_host_key(&path).unwrap();
    assert_eq!(first.public_key(), second.public_key());
    assert_eq!(first.algorithm(), Algorithm::Ed25519);
}

#[test]
fn authorized_keys_takes_ed25519_and_ecdsa_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authorized_keys");
    let ed = new_key();
    let ed_line = ed.public_key().to_openssh().unwrap();
    // The RSA and the ECDSA keys are the examples of the crate ssh-key.
    let rsa = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQCmjkeMm8k3JkNrf16eb5pG4bc77B6Mt3VN4saltsRV8vASpyWa/PlBgdaeldOaNJ5NK0gqU3KyiUNzHbdcc8572e7IUBDJS/rlaWARiSL4aos2VbNX0k56Z5zYp9m/bq5m9/mlb+PQkNBjIhimgpYNiq2TwBiYeA6tLb79cPtHA0cX5BLk/a5oUpLsiR4kI/f+Q98vVDKasKXXVh5YLkLobrruDB6er2A9fOcIUF0O4JCRLh/Dc161gE3fQrYTMQenbppZzfxrZfQ8YwLPvKjnqm+XRX+pbTtaJuj0EgTSzUK+EZxoSw8CNwiZpxrjwecTMVQ8w/srQmh4ABGuTqk0wP8HcI7hg+fpBv7kiejh5X/Oehxt+Puu85u9GVXb1a0av/vhJvUCBcuISvCA/z1wVJ0xdLhb1/ZiTDdTzyNbZQ0OQijzK+e1SlkNhp+3eGVZu3pNZvnTppwIXv3wg6kV1HodkWGgh1ayY7Buc52Z8okDYqvJat5CzOj5OaQNr/k= r";
    let ecdsa = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBHwf2HMM5TRXvo2SQJjsNkiDD5KqiiNjrGVv3UUh+mMT5RHxiRtOnlqvjhQtBq0VpmpCV/PwUdhOig4vkbqAcEc= e";
    let opts = format!("restrict {ed_line} c");
    std::fs::write(&path, format!("{ed_line}\n{rsa}\n{opts}\n{ecdsa}\n")).unwrap();
    let keys = ssh::read_authorized_keys(&path).unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].key_data(), ed.public_key().key_data());
    assert!(keys[1].algorithm().is_ecdsa());
}

fn new_key() -> PrivateKey {
    PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).unwrap()
}

struct Client;

impl Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// A root with the bare repository `repo.git` and one commit on `main`.
fn make_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo.git");
    let git = |args: &[&str], input: Option<&str>| -> String {
        let mut cmd = Command::new("git");
        cmd.arg("--git-dir").arg(&repo).args(args);
        cmd.env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t");
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        {
            use std::io::Write as _;
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(input.unwrap_or("").as_bytes()).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "git {args:?} fails");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let status = Command::new("git")
        .args(["init", "-q", "--bare", "-b", "main"])
        .arg(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    let tree = git(&["mktree"], Some(""));
    let commit = git(&["commit-tree", &tree, "-m", "one"], None);
    git(&["update-ref", "refs/heads/main", &commit], None);
    dir
}

/// Start the server on a unix socket in `dir`, with `options`.
fn start(dir: &Path, options: Options) -> std::path::PathBuf {
    let socket = dir.join("ssh.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let config = Arc::new(ssh::config(new_key()));
    tokio::spawn(ssh::serve_unix(listener, config, Arc::new(options)));
    socket
}

/// Connect to the server at `socket`.
async fn connect(socket: &Path) -> Handle<Client> {
    let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    russh::client::connect_stream(Arc::new(Config::default()), stream, Client)
        .await
        .unwrap()
}

/// Connect and authenticate with `key`. Gives the session when the
/// server accepts the key.
async fn login(socket: &Path, key: &PrivateKey) -> Option<Handle<Client>> {
    let mut handle = connect(socket).await;
    let auth = handle
        .authenticate_publickey(
            "git",
            PrivateKeyWithHashAlg::new(Arc::new(key.clone()), None),
        )
        .await
        .unwrap();
    auth.success().then_some(handle)
}

/// What the server gives for `command`.
#[derive(Debug, Default)]
struct Outcome {
    refused: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: Option<u32>,
}

/// Run `command`, send `input` and EOF, and collect the outcome.
async fn exec(handle: &Handle<Client>, command: &str, input: &[u8]) -> Outcome {
    exec_with(handle, None, command, input).await
}

/// [`exec`] with `GIT_PROTOCOL` set to `protocol`.
async fn exec_with(
    handle: &Handle<Client>,
    protocol: Option<&str>,
    command: &str,
    input: &[u8],
) -> Outcome {
    let mut channel = handle.channel_open_session().await.unwrap();
    if let Some(protocol) = protocol {
        channel
            .set_env(false, "GIT_PROTOCOL", protocol)
            .await
            .unwrap();
    }
    channel.exec(true, command).await.unwrap();
    let mut outcome = Outcome::default();
    let mut sent = false;
    let run = async {
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Success if !sent => {
                    sent = true;
                    channel.data_bytes(input.to_vec()).await.unwrap();
                    channel.eof().await.unwrap();
                }
                ChannelMsg::Failure => {
                    outcome.refused = true;
                    break;
                }
                ChannelMsg::Data { data } => outcome.stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => outcome.stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => outcome.status = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .expect("the command ends in 30 s");
    outcome
}

/// The refs of a ref advertisement: each packet line up to its NUL or
/// its newline.
fn refs(advertisement: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = advertisement;
    while rest.len() >= 4 {
        let len = usize::from_str_radix(std::str::from_utf8(&rest[..4]).unwrap(), 16).unwrap();
        if len == 0 {
            rest = &rest[4..];
            continue;
        }
        let line = &rest[4..len];
        let end = line
            .iter()
            .position(|&b| b == 0 || b == b'\n')
            .unwrap_or(line.len());
        out.push(String::from_utf8_lossy(&line[..end]).into_owned());
        rest = &rest[len..];
    }
    out
}

#[tokio::test]
async fn git_ssh_upload_pack_matches_git() {
    need_git!();
    let root = make_root();
    let key = new_key();
    let mut options = Options::new(root.path());
    options.authorized_keys = vec![key.public_key().clone()];
    let socket = start(root.path(), options);
    let repo = root.path().join("repo.git");
    // The same program on a pipe, with a flush packet on stdin.
    let expected = {
        use std::io::Write as _;
        let mut child = Command::new("git")
            .arg("upload-pack")
            .arg(&repo)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"0000").unwrap();
        child.wait_with_output().unwrap()
    };
    assert!(expected.status.success());
    assert!(expected.stdout.ends_with(b"0000"));
    // `--advertise-refs` is the stateless form: it adds the capability
    // `no-done`. Its refs are the same.
    let advertised = Command::new("git")
        .args(["upload-pack", "--advertise-refs"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(advertised.status.success());
    assert_eq!(refs(&advertised.stdout), refs(&expected.stdout));

    let handle = login(&socket, &key).await.expect("the key is accepted");
    // A flush packet after the refs: the client wants nothing.
    let outcome = exec(&handle, "git-upload-pack '/repo.git'", b"0000").await;
    assert!(!outcome.refused, "{outcome:?}");
    assert_eq!(
        String::from_utf8_lossy(&outcome.stdout),
        String::from_utf8_lossy(&expected.stdout)
    );
    assert_eq!(refs(&outcome.stdout).len(), 2);
    assert_eq!(outcome.status, Some(0));

    // The path with no `.git` names the same repository.
    let outcome = exec(&handle, "git-upload-pack 'repo'", b"0000").await;
    assert_eq!(outcome.stdout, expected.stdout);

    // Protocol v2 from the env request of the client.
    let outcome = exec_with(
        &handle,
        Some("version=2"),
        "git-upload-pack 'repo.git'",
        b"0000",
    )
    .await;
    assert!(
        outcome.stdout.starts_with(b"000eversion 2\n"),
        "{outcome:?}"
    );
    assert_eq!(outcome.status, Some(0));

    // A key of the file can push: receive-pack gives its refs.
    let outcome = exec(&handle, "git-receive-pack 'repo.git'", b"0000").await;
    assert!(!outcome.refused, "{outcome:?}");
    assert!(outcome.stdout.ends_with(b"0000"), "{outcome:?}");
    assert_eq!(outcome.status, Some(0));
}

#[tokio::test]
async fn git_ssh_refuses_an_unknown_key() {
    need_git!();
    let root = make_root();
    let known = new_key();
    let mut options = Options::new(root.path());
    options.authorized_keys = vec![known.public_key().clone()];
    let socket = start(root.path(), options.clone());
    assert!(login(&socket, &new_key()).await.is_none());
    let mut handle = connect(&socket).await;
    let auth = handle.authenticate_none(ANON_USER).await.unwrap();
    assert!(!auth.success(), "no anonymous access without anon_read");

    // With anon_read, an unknown key and the anonymous user can read and
    // cannot push.
    let dir = tempfile::tempdir().unwrap();
    options.anon_read = true;
    let socket = start(dir.path(), options);
    let handle = login(&socket, &new_key()).await.expect("read access");
    let outcome = exec(&handle, "git-receive-pack 'repo.git'", b"").await;
    assert!(outcome.refused, "{outcome:?}");
    assert!(String::from_utf8_lossy(&outcome.stderr).contains("authorized_keys"));
    let outcome = exec(&handle, "git-upload-pack 'repo.git'", b"0000").await;
    assert_eq!(outcome.status, Some(0), "{outcome:?}");

    let mut handle = connect(&socket).await;
    let auth = handle.authenticate_none(ANON_USER).await.unwrap();
    assert!(auth.success());
    let outcome = exec(&handle, "git-receive-pack 'repo.git'", b"").await;
    assert!(outcome.refused, "{outcome:?}");
    let mut other = connect(&socket).await;
    assert!(!other.authenticate_none("git").await.unwrap().success());
}

#[tokio::test]
async fn git_ssh_refuses_a_shell_and_other_commands() {
    let root = tempfile::tempdir().unwrap();
    let key = new_key();
    let mut options = Options::new(root.path());
    options.authorized_keys = vec![key.public_key().clone()];
    let socket = start(root.path(), options);
    let handle = login(&socket, &key).await.expect("the key is accepted");
    for command in [
        "ls",
        "git-upload-pack '../x'",
        "git-upload-pack 'none.git'",
        "git-upload-pack 'a' 'b'",
    ] {
        let outcome = exec(&handle, command, b"").await;
        assert!(outcome.refused, "{command:?} is not refused: {outcome:?}");
    }
    let mut channel = handle.channel_open_session().await.unwrap();
    channel.request_shell(true).await.unwrap();
    let refused = loop {
        match channel.wait().await {
            Some(ChannelMsg::Failure) => break true,
            Some(ChannelMsg::Success) | None => break false,
            _ => {}
        }
    };
    assert!(refused, "a shell is not refused");
    let forward = handle
        .channel_open_direct_tcpip("127.0.0.1", 22, "127.0.0.1", 1)
        .await;
    assert!(forward.is_err(), "direct-tcpip is not refused");
}
