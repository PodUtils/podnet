//! The unix-socket carrier of the link (T-274): a socket at a path with
//! the mode 0600, the checks of the path, the check of the peer uid, and
//! the socket pair for one process. The tests bind no TCP port, so they
//! run on the restricted host and on the host with no loopback.

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::time::Duration;

use podnet::carriers::{self, Options};
use podnet::exit::{self, code_of};
use podnet::link::{self, Settings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(20);

fn unix(options: &Options) -> Box<dyn pod_link::Carrier> {
    carriers::open("unix", options).unwrap()
}

async fn echo_once(dialer: &podnet::link::Link, listener: &podnet::link::Link, msg: &[u8]) {
    let mut near = dialer.open().unwrap();
    near.write_all(msg).await.unwrap();
    let mut far = timeout(LIMIT, listener.accept()).await.unwrap().unwrap();
    let mut got = vec![0; msg.len()];
    timeout(LIMIT, far.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, msg);
    far.write_all(&got).await.unwrap();
    let mut back = vec![0; msg.len()];
    timeout(LIMIT, near.read_exact(&mut back))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back, msg);
}

#[tokio::test]
async fn a_link_crosses_a_socket_file_of_mode_0600() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("link.sock");
    let path = path.to_str().unwrap();
    let carrier = unix(&Options::default());
    let mut door = link::listen(&*carrier, path).await.unwrap();
    assert_eq!(door.address(), path);
    let meta = std::fs::symlink_metadata(path).unwrap();
    assert!(meta.file_type().is_socket());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let (dialer, listener) = tokio::join!(
        link::dial(&*carrier, path, Settings::default()),
        door.accept(Settings::default())
    );
    let (dialer, listener) = (dialer.unwrap(), listener.unwrap());
    echo_once(&dialer, &listener, b"over a unix socket").await;
    drop(door);
    assert!(
        std::fs::symlink_metadata(path).is_err(),
        "the drop of the door removes the socket"
    );
}

#[tokio::test]
async fn a_regular_file_at_the_path_exits_64_and_stays() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data");
    std::fs::write(&path, b"keep me").unwrap();
    let carrier = unix(&Options::default());
    let e = link::listen(&*carrier, path.to_str().unwrap())
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), exit::USAGE, "{e:#}");
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}

#[tokio::test]
async fn a_live_socket_exits_69() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("link.sock");
    let path = path.to_str().unwrap();
    let carrier = unix(&Options::default());
    let _door = link::listen(&*carrier, path).await.unwrap();
    let e = link::listen(&*carrier, path).await.err().unwrap();
    assert_eq!(code_of(&e), exit::UNAVAILABLE, "{e:#}");
}

#[tokio::test]
async fn a_stale_socket_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("link.sock");
    // A socket file with no listener behind it.
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(std::fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_socket());
    let carrier = unix(&Options::default());
    let mut door = link::listen(&*carrier, path.to_str().unwrap())
        .await
        .unwrap();
    let (d, l) = tokio::join!(
        link::dial(&*carrier, path.to_str().unwrap(), Settings::default()),
        door.accept(Settings::default())
    );
    echo_once(&d.unwrap(), &l.unwrap(), b"after a stale socket").await;
}

#[tokio::test]
async fn a_path_of_108_bytes_exits_64_and_one_of_107_binds() {
    let carrier = unix(&Options::default());
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_str().unwrap().len() + 1;
    assert!(
        base < 100,
        "the temporary directory is too deep for this test"
    );
    let exact = format!("{}/{}", dir.path().display(), "b".repeat(107 - base));
    assert_eq!(exact.len(), 107);
    let door = link::listen(&*carrier, &exact).await.unwrap();
    assert_eq!(
        std::fs::symlink_metadata(&exact)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(door);
    let long = format!("{}/{}", dir.path().display(), "a".repeat(108 - base));
    assert_eq!(long.len(), 108);
    let e = link::listen(&*carrier, &long).await.err().unwrap();
    assert_eq!(code_of(&e), exit::USAGE, "{e:#}");
    let e = link::dial(&*carrier, &long, Settings::default())
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), exit::USAGE, "{e:#}");
    let e = link::dial(&*carrier, "", Settings::default())
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), exit::USAGE, "{e:#}");
}

#[tokio::test]
async fn a_dial_with_no_listener_exits_69() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("none.sock");
    let carrier = unix(&Options::default());
    let e = link::dial(&*carrier, path.to_str().unwrap(), Settings::default())
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), exit::UNAVAILABLE, "{e:#}");
}

#[tokio::test]
async fn the_socket_pair_runs_a_link_in_one_process() {
    let (dialer, listener) = link::unix_pair(Settings::default()).unwrap();
    echo_once(&dialer, &listener, b"over a socket pair").await;
}

/// A thread with the uid 65534 connects to a socket that the mode 0666
/// opens. The listener refuses it, unless `allow_other_uid` is set.
/// The test needs root (it changes the uid of one thread).
#[tokio::test]
async fn a_peer_of_another_uid_is_refused_unless_allowed() {
    // SAFETY: geteuid has no argument and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipping, this test needs root to change the uid of a thread");
        return;
    }
    for allow in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.path().join("link.sock");
        let path = path.to_str().unwrap().to_string();
        let options = Options {
            allow_other_uid: allow,
            ..Options::default()
        };
        let carrier = unix(&options);
        let mut door = link::listen(&*carrier, &path).await.unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let p = path.clone();
        let other = std::thread::spawn(move || {
            // SAFETY: the raw system call changes the ids of this thread
            // only; it reads no pointer.
            let rc = unsafe { libc::syscall(libc::SYS_setresuid, 65534, 65534, 65534) };
            assert_eq!(rc, 0, "setresuid");
            let s = std::os::unix::net::UnixStream::connect(&p).unwrap();
            // Hold the socket until the test has the result.
            let _ = rx.recv_timeout(LIMIT);
            drop(s);
        });
        let got = timeout(LIMIT, door.accept(Settings::default()))
            .await
            .unwrap();
        match (allow, got) {
            (false, Err(e)) => assert_eq!(code_of(&e), exit::UNAVAILABLE, "{e:#}"),
            (true, Ok(_)) => {}
            (false, Ok(_)) => panic!("a peer of another uid was accepted"),
            (true, Err(e)) => panic!("--allow-other-uid refused a peer: {e:#}"),
        }
        let _ = tx.send(());
        other.join().unwrap();
    }
}
