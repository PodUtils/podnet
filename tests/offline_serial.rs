//! The `serial` carrier of PodUtils/podnet#499 over a pseudo-terminal pair
//! at 1200 baud.
//!
//! Two pseudo-terminal pairs are the two ends of one wire: the test copies
//! the bytes of each master to the other at 1200 baud (a pseudo-terminal
//! does not pace itself), so two `podnet link` processes each open one
//! slave and speak AX.25 in KISS framing through the test. The service is
//! `/bin/cat`, because a shell is refused as a service program by name, and
//! the bytes `hi` must come back inside 3 s of the write. The frame limit
//! and the retransmission timeout are checked against the module that the
//! carrier uses.
//!
//! A host with no `/dev/ptmx` skips the wire test with a named reason.

#![cfg(unix)]

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A pseudo-terminal pair: the master side, and the path of the slave.
struct Pty {
    master: File,
    slave: PathBuf,
}

fn pty() -> Option<Pty> {
    use std::os::fd::FromRawFd;

    if !Path::new("/dev/ptmx").exists() {
        eprintln!("skip: /dev/ptmx is unavailable on this host");
        return None;
    }
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    // SAFETY: openpty writes two descriptors into the integers passed.
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if opened != 0 {
        eprintln!(
            "skip: /dev/ptmx gave no pseudo-terminal: {}",
            std::io::Error::last_os_error()
        );
        return None;
    }
    let mut name = [0 as libc::c_char; 256];
    // SAFETY: ttyname_r writes into the buffer of this function.
    let named = unsafe { libc::ttyname_r(slave, name.as_mut_ptr(), name.len()) };
    if named != 0 {
        // SAFETY: this function owns the two descriptors.
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
        eprintln!("skip: the slave of /dev/ptmx has no name (errno {named})");
        return None;
    }
    // SAFETY: the name is a NUL-terminated string that ttyname_r wrote.
    let path = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    // The line is raw from the start: a byte of a KISS frame is not a line
    // of a terminal, and the echo of the terminal would go back on the wire.
    let keeper = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("the slave of the pair");
    let fd = keeper.as_raw_fd();
    // SAFETY: tcgetattr writes into the struct of this function.
    let mut settings: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(fd, &mut settings) }, 0);
    // SAFETY: cfmakeraw changes only the struct passed.
    unsafe { libc::cfmakeraw(&mut settings) };
    // SAFETY: a termios that this descriptor read and this function changed.
    assert_eq!(unsafe { libc::tcsetattr(fd, libc::TCSANOW, &settings) }, 0);
    drop(keeper);
    // SAFETY: this function owns the slave descriptor and gives it up here.
    unsafe { libc::close(slave) };
    // SAFETY: the master descriptor is open, and the File owns it now.
    Some(Pty {
        master: unsafe { File::from_raw_fd(master) },
        slave: PathBuf::from(path),
    })
}

/// The time that `bytes` spend on a 1200-baud 8N1 line: ten bits for each
/// byte, with no parity and one stop bit. A pseudo-terminal does not pace
/// itself, so the test wire carries the bytes at the speed of the line.
fn wire_time(bytes: usize) -> Duration {
    Duration::from_nanos((bytes as u64) * 10 * 1_000_000_000 / 1200)
}

/// Copy each direction of a wire between the two masters until it closes.
///
/// A master whose slave is not open yet gives an end of file or `EIO`; it
/// waits a moment and reads again, so that either end of the wire may open
/// after the bridge starts.
fn bridge(mut one: File, mut other: File, stop: Arc<AtomicBool>) {
    let mut buf = [0u8; 8192];
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let mut fds = [
            libc::pollfd {
                fd: one.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: other.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: poll reads the two descriptors of this function.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, 50) };
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        for (side, fd) in fds.iter().enumerate() {
            if fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) == 0 {
                continue;
            }
            let (from, to) = if side == 0 {
                (&mut one, &mut other)
            } else {
                (&mut other, &mut one)
            };
            match from.read(&mut buf) {
                Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                Ok(count) => {
                    if to.write_all(&buf[..count]).is_err() || to.flush().is_err() {
                        std::thread::sleep(Duration::from_millis(5));
                    } else {
                        std::thread::sleep(wire_time(count));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}

struct Running {
    child: Child,
    stop: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn state(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).expect("a state directory");
    path
}

fn podnet(dir: &Path, name: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", state(dir, name))
        .env("PODNET_RUN_DIR", dir.join(format!("run-{name}")))
        .env_remove("RUST_LOG");
    cmd
}

#[test]
fn an_echo_over_a_1200_baud_wire_comes_back_inside_three_seconds() {
    let (Some(listener_pty), Some(dialer_pty)) = (pty(), pty()) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Both ends have a key; the listener admits the key of the dialer.
    let dialer =
        podnet::identity::create(&state(dir.path(), "dial")).expect("the key of the dialer");
    let listener =
        podnet::identity::create(&state(dir.path(), "listen")).expect("the key of the listener");
    let allow = dir.path().join("allow");
    std::fs::write(&allow, format!("{}\n", dialer.fingerprint())).expect("the allow list");

    let listener_address = format!("serial:{},1200", listener_pty.slave.display());
    let listener_args = [
        "link",
        "listen",
        "serial",
        "--carrier",
        listener_address.as_str(),
        "--allow",
        allow.to_str().expect("a path"),
        // A shell is refused as a service program by name (SHELLS in
        // src/link_exec.rs), so the echo service is cat, which returns the
        // bytes it reads.
        "--service",
        "raw=exec:/bin/cat",
    ];
    let mut listener_process = podnet(dir.path(), "listen", &listener_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the listener");
    let listener_stderr = Arc::new(Mutex::new(Vec::<u8>::new()));
    {
        let listener_stderr = listener_stderr.clone();
        let mut stderr = listener_process
            .stderr
            .take()
            .expect("the stderr of the listener");
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) => return,
                    Ok(count) => listener_stderr
                        .lock()
                        .unwrap()
                        .extend_from_slice(&buf[..count]),
                    Err(_) => return,
                }
            }
        });
    }
    let listener_stdout = listener_process
        .stdout
        .take()
        .expect("the stdout of the listener");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(listener_stdout).lines() {
            let Ok(line) = line else { return };
            let _ = ready_tx.send(line);
        }
    });

    let stop = Arc::new(AtomicBool::new(false));
    let wire = {
        let stop = stop.clone();
        std::thread::spawn(move || bridge(listener_pty.master, dialer_pty.master, stop))
    };

    // Wait for the line and the fingerprint of the listener.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut fingerprint = None;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = ready_rx.recv_timeout(left) else {
            break;
        };
        if let Some(text) = line.strip_prefix("podnet: fingerprint") {
            fingerprint = Some(text.trim().to_string());
            break;
        }
    }
    let fingerprint = fingerprint.unwrap_or_else(|| {
        panic!(
            "the listener printed no fingerprint; the listener stderr is {:?}",
            String::from_utf8_lossy(&listener_stderr.lock().unwrap())
        )
    });
    assert_eq!(fingerprint, listener.fingerprint());

    let dialer_address = format!("serial:{},1200", dialer_pty.slave.display());
    let dialer_args = [
        "link",
        "dial",
        "serial",
        "--carrier",
        dialer_address.as_str(),
        "--peer",
        fingerprint.as_str(),
        "--service",
        "raw",
    ];
    let mut dialer_process = podnet(dir.path(), "dial", &dialer_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the dialer");
    let stdin: ChildStdin = dialer_process
        .stdin
        .take()
        .expect("the stdin of the dialer");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    {
        let output = output.clone();
        let stdout = dialer_process
            .stdout
            .take()
            .expect("the stdout of the dialer");
        std::thread::spawn(move || {
            let mut stdout = stdout;
            let mut buf = [0u8; 4096];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => return,
                    Ok(count) => output.lock().unwrap().extend_from_slice(&buf[..count]),
                    Err(_) => return,
                }
            }
        });
    }
    let running = Running {
        child: listener_process,
        stop,
    };

    // Wait until the listener admits the dialer, so that the time of the
    // echo is the time of the echo and not the time of the hello.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut admitted = false;
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = ready_rx.recv_timeout(left) else {
            break;
        };
        if line.contains("peer admitted") {
            admitted = true;
            break;
        }
    }
    assert!(admitted, "the link was never admitted");

    let started = Instant::now();
    {
        let mut stdin = stdin;
        stdin
            .write_all(b"hi\n")
            .and_then(|()| stdin.flush())
            .expect("a write to the service");
    }
    let mut took = None;
    while started.elapsed() < Duration::from_secs(3) {
        if output.lock().unwrap().windows(2).any(|w| w == &b"hi"[..]) {
            took = Some(started.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let took = took.unwrap_or_else(|| {
        panic!(
            "no echo inside 3 s; the dialer wrote {:?}",
            String::from_utf8_lossy(&output.lock().unwrap())
        )
    });
    assert!(took < Duration::from_secs(3), "the echo took {took:?}");
    println!("the echo came back in {took:?} over a wire at 1200 baud");

    let _ = dialer_process.kill();
    let _ = dialer_process.wait();
    drop(running);
    let _ = wire.join();
}

#[test]
fn a_frame_above_256_bytes_is_refused_by_name() {
    let refused =
        podnet::offline::serial::frame(&vec![0u8; 257]).expect_err("a frame above 256 bytes");
    assert_eq!(refused.name(), "frame-too-large");
    assert!(podnet::offline::serial::frame(&vec![0u8; 256]).is_ok());
    let refused = podnet::offline::serial::arq(1200, 1)
        .expect("a window")
        .push_at(Instant::now(), &vec![0u8; 256])
        .expect_err("a payload above 255");
    assert_eq!(refused.name(), "frame-too-large");
}

#[test]
fn the_timeout_at_1200_baud_comes_from_the_frame_time() {
    let frame = podnet::offline::serial::frame_time(1200);
    let again = podnet::offline::serial::retransmit_after(1200);
    assert!(again > Duration::from_secs(4), "the timeout is {again:?}");
    assert_eq!(again, frame * 2);
    assert!(again < Duration::from_secs(5), "the timeout is {again:?}");
}
