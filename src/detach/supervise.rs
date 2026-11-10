use super::*;
use std::sync::atomic::AtomicBool;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_stop(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// Exit codes after which the supervisor does not start the child
/// again: a bad command line, a bad configuration.
pub(super) const FINAL: [i32; 2] = [crate::exit::USAGE, crate::exit::CONFIG];
/// A run this long resets the backoff.
const STABLE: Duration = Duration::from_secs(30);
/// A run shorter than this is a quick failure.
const QUICK: Duration = Duration::from_secs(10);
/// After this many quick failures in sequence, run the kept binary.
const QUICK_LIMIT: u32 = 5;

/// The wait before the next start: the backoff, or the Retry-After that
/// the child asked for when it is longer (at most 1 h).
pub(super) fn next_wait(backoff: Duration, asked: Option<u64>) -> Duration {
    match asked {
        Some(secs) => backoff.max(Duration::from_secs(secs.min(3600))),
        None => backoff,
    }
}

/// The next delay: twice the delay, at most 30 s.
pub(super) fn next_backoff(delay: Duration) -> Duration {
    (delay * 2).min(Duration::from_secs(30))
}

/// A factor from 0.5 to 1.5, from the clock.
fn jitter() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    0.5 + (nanos % 1000) as f64 / 1000.0
}

fn note(text: &str) {
    eprintln!("podnet: supervisor: {text}");
}

pub(super) fn run(args: &[OsString]) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(err) => {
            note(&format!("{err:#}"));
            TEMPFAIL
        }
    }
}

fn run_inner(args: &[OsString]) -> Result<i32> {
    let name = std::env::var(NAME_VARIABLE).context("a supervisor with no instance name")?;
    instances::check_name(&name)?;
    let dir = instances::run_dir()?;
    let paths = instances::paths(&dir, &name)?;
    let _lock = instances::try_lock(&paths.lock)?
        .with_context(|| format!("the instance {name} runs already"))?;
    // SAFETY: a handler that only stores to an atomic.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_stop as extern "C" fn(libc::c_int) as usize;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in [libc::SIGTERM, libc::SIGINT] {
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
    let child_args: Vec<OsString> = args.iter().skip(1).cloned().collect();
    let mut record = Record {
        name: name.clone(),
        pid: 0,
        supervisor: Some(std::process::id()),
        started: instances::now(),
        command: std::env::var(COMMAND_VARIABLE).unwrap_or_default(),
        args: child_args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
        hash: std::env::var(HASH_VARIABLE).unwrap_or_default(),
        cwd: std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
        state: "starting".into(),
        ready: None,
        log: paths.log.display().to_string(),
        restarts: 0,
        retry_after_s: None,
    };
    instances::write(&dir, &record)?;
    let current = binaries()
        .into_iter()
        .next()
        .context("no path of this binary")?;
    let previous = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| crate::update::previous_of(&p))
        .filter(|p| p.exists());
    let mut program = current;
    let mut ready_seen = false;
    let mut delay = Duration::from_secs(1);
    let mut quick = 0u32;
    loop {
        let mut cmd = Command::new(&program);
        {
            use std::os::unix::process::CommandExt;
            cmd.arg0("podnet");
        }
        if ready_seen {
            // A restart has no caller that waits: the full budget.
            cmd.env_remove(READY_TIMEOUT_VARIABLE);
        }
        cmd.arg(CHILD_FLAG)
            .args(&child_args)
            .env(SUPERVISED_VARIABLE, "1")
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {}", program.display()))?;
        let started = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        return;
                    }
                }
            });
        }
        let status = loop {
            if STOP.load(Ordering::SeqCst) {
                stop_child(&mut child);
                instances::remove(&dir, &name);
                return Ok(0);
            }
            while let Ok(line) = rx.try_recv() {
                if line.starts_with(READY) {
                    if !ready_seen {
                        ready_seen = true;
                        // The supervisor leaves the terminal too, and
                        // passes the ready line to the parent.
                        let log = open_private(&paths.log).ok();
                        if !unix::leave(log.as_ref(), format!("{line}\n").as_bytes()) {
                            stop_child(&mut child);
                            instances::remove(&dir, &name);
                            return Ok(TEMPFAIL);
                        }
                    } else {
                        note(&format!("ready again: {}", line.trim_start_matches(READY)));
                    }
                } else {
                    println!("{line}");
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                break status;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        // The lines that came before the end.
        while let Ok(line) = rx.try_recv() {
            if !line.starts_with(READY) {
                println!("{line}");
            }
        }
        let code = code_of(status);
        // Issues 195, 182 and 210: what the child said of its end.
        let said = instances::read(&dir, &name);
        if ready_seen && code == 0 && said.as_ref().is_some_and(|r| r.state == "done") {
            note("the instance ended on request (--stop-after, --idle-secs or --run-for); it is not started again");
            instances::remove(&dir, &name);
            return Ok(0);
        }
        let asked = said.as_ref().and_then(|r| r.retry_after_s);
        if !ready_seen {
            // The first run ended before it was ready: the parent gets
            // its code.
            instances::remove(&dir, &name);
            return Ok(code);
        }
        if FINAL.contains(&code) {
            note(&format!(
                "the instance exited {code}; it is not started again"
            ));
            instances::remove(&dir, &name);
            return Ok(code);
        }
        if STOP.load(Ordering::SeqCst) {
            instances::remove(&dir, &name);
            return Ok(0);
        }
        let ran = started.elapsed();
        if ran >= STABLE {
            delay = Duration::from_secs(1);
            quick = 0;
        } else if ran < QUICK && asked.is_none() {
            // A refusal with a Retry-After is not a quick failure: it
            // must not swap to the kept binary (issue 182).
            quick += 1;
        }
        if quick >= QUICK_LIMIT {
            if let Some(prev) = previous.as_ref().filter(|p| **p != program) {
                note(&format!(
                    "{quick} quick failures in sequence: the kept binary {} runs now",
                    prev.display()
                ));
                program = prev.clone();
                quick = 0;
            }
        }
        record = instances::read(&dir, &name).unwrap_or(record);
        record.restarts += 1;
        record.pid = 0;
        record.state = "restarting".into();
        record.retry_after_s = None;
        instances::write(&dir, &record)?;
        let wait = next_wait(delay.mul_f64(jitter()), asked);
        note(&format!(
            "the instance exited {code} after {} s; start {} in {} ms",
            ran.as_secs(),
            record.restarts,
            wait.as_millis()
        ));
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if STOP.load(Ordering::SeqCst) {
                instances::remove(&dir, &name);
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        delay = next_backoff(delay);
    }
}

/// SIGTERM to the child, SIGKILL after 10 s.
fn stop_child(child: &mut std::process::Child) {
    // SAFETY: a signal to a child of this process.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}
