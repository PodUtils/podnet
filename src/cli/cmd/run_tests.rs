use crate::cli::cmd::run::idle_watch;
use podnet::share::ShareControl;
use std::sync::Arc;
use std::time::Duration;

// Issue 271: the idle clock of a share starts when the session serves.
// A control that was built 1.1 s before does not end a session with
// --idle-secs 1 at the first check (5 s); in 60 s of the paused clock
// the watch does not end.
#[tokio::test(start_paused = true)]
async fn the_idle_clock_starts_when_the_session_serves() {
    let control = Arc::new(ShareControl::default());
    std::thread::sleep(Duration::from_millis(1100));
    let watch = idle_watch(vec![control], 1);
    let ended = tokio::time::timeout(Duration::from_secs(60), watch).await;
    assert!(ended.is_err(), "the watch ended at the first check");
}
