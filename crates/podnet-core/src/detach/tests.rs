use super::*;

#[test]
fn the_detach_options_are_not_in_the_hash() {
    let args: Vec<String> = [
        "tunnel",
        "--detach",
        "--name",
        "web",
        "--ready-timeout=30",
        "--port",
        "3000",
        "--replace",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        without_detach_options(&args),
        vec!["tunnel", "--port", "3000"]
    );
}

#[test]
fn the_backoff_doubles_to_30_s() {
    let mut delay = Duration::from_secs(1);
    let mut seen = Vec::new();
    for _ in 0..7 {
        seen.push(delay.as_secs());
        delay = supervise::next_backoff(delay);
    }
    assert_eq!(seen, vec![1, 2, 4, 8, 16, 30, 30]);
    assert_eq!(supervise::FINAL, [64, 78]);
}

/// Issues 182 and 210: a Retry-After of the child is the wait when it is
/// longer than the backoff, at most 1 h.
// podssh: podnet wait at its time limit exits 124.
#[test]
fn wait_at_its_limit_exits_124() {
    let dir = tempfile::tempdir().unwrap();
    let paths = instances::paths(dir.path(), "slow").unwrap();
    let _lock = instances::try_lock(&paths.lock).unwrap().unwrap();
    let record = Record {
        name: "slow".into(),
        pid: std::process::id(),
        supervisor: None,
        started: instances::now(),
        command: "net switch".into(),
        args: Vec::new(),
        hash: String::new(),
        cwd: String::new(),
        state: "starting".into(),
        ready: None,
        log: paths.log.display().to_string(),
        restarts: 0,
        retry_after_s: None,
    };
    instances::write(dir.path(), &record).unwrap();
    let err = wait_ready_record(dir.path(), "slow", Duration::from_millis(300)).unwrap_err();
    assert_eq!(crate::exit::code_of(&err), 124, "{err:#}");
}

// Issue 182: the first child does not wait past --ready-timeout.
#[test]
fn the_provision_budget_of_the_first_child() {
    let full = Duration::from_secs(300);
    std::env::remove_var(READY_TIMEOUT_VARIABLE);
    assert_eq!(provision_budget(full), full);
    std::env::set_var(READY_TIMEOUT_VARIABLE, "60");
    assert_eq!(provision_budget(full), Duration::from_secs(60));
    std::env::set_var(READY_TIMEOUT_VARIABLE, "900");
    assert_eq!(provision_budget(full), full);
    std::env::remove_var(READY_TIMEOUT_VARIABLE);
}

#[test]
fn the_wait_honours_a_retry_after() {
    let backoff = Duration::from_secs(4);
    assert_eq!(supervise::next_wait(backoff, None), backoff);
    assert_eq!(supervise::next_wait(backoff, Some(1)), backoff);
    assert_eq!(
        supervise::next_wait(backoff, Some(1379)),
        Duration::from_secs(1379)
    );
    assert_eq!(
        supervise::next_wait(backoff, Some(99_999)),
        Duration::from_secs(3600)
    );
}

#[test]
fn default_names() {
    assert_eq!(default_name("tunnel", "0123456789abcdef"), "tunnel-012345");
    assert_eq!(
        default_name("warp proxy", "abcdef0123"),
        "warp-proxy-abcdef"
    );
    instances::check_name(&default_name("net switch", "ffffffffffffffff")).unwrap();
}

/// T-236: a held ready document goes out at the next call of ready, once.
#[test]
fn a_held_ready_document_goes_with_the_next_ready() {
    super::ready_when_reachable(serde_json::json!({"pair": "p-1", "pin": "1234"}));
    assert_eq!(
        super::ready_doc(serde_json::json!({"pair": "p-1", "target": "unix:/x"})),
        serde_json::json!({"pair": "p-1", "pin": "1234"})
    );
    let own = serde_json::json!({"url": "http://127.0.0.1:1/"});
    assert_eq!(super::ready_doc(own.clone()), own);
}
