use crate::cli::signals::{deadline_of, install_signals, until_stop_or};
use std::time::Duration;

/// Issue 101: once the stop is armed (at the ready line), a SIGTERM
/// ends the work of `until_stop_or` (the verify) with `None`: a clean
/// stop, not the default action. The only test here that takes
/// signals: the stop request is global to the process.
#[test]
fn a_sigterm_after_the_ready_line_stops_the_verify_cleanly() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        install_signals();
        podnet::detach::arm_stop();
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(200));
            // SAFETY: kill() to this process.
            unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
        });
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            until_stop_or(None, std::future::pending::<()>()),
        )
        .await
        .expect("the signal ends the wait");
        assert!(out.is_none());
        // The stop request stays: a later wait ends at once.
        assert!(until_stop_or(None, async { 1 }).await.is_none());
    });
}

/// Issue 179: --run-for ends the verify.
#[tokio::test]
async fn until_stop_or_ends_at_the_deadline() {
    let deadline = deadline_of(Some(0));
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        until_stop_or(deadline, std::future::pending::<()>()),
    )
    .await
    .expect("the deadline ends the wait");
    assert!(out.is_none());
}
