//! Fake dialers of the tests of the combinators: no network. A dial
//! writes the label of its way into the stream, so that a test reads which
//! way carried the flow.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

use super::{BoxFuture, Class, Connected, Datagrams, Dial, Failure, Stage, Target, Way};

/// The labels of the dials, in the order that they started.
pub type Log = Arc<Mutex<Vec<String>>>;

/// A fake way.
pub struct Fake {
    pub label: &'static str,
    /// The time before the dial ends.
    pub delay: Duration,
    /// True: the dial fails after its delay.
    pub fails: bool,
    pub names_inside: bool,
    pub log: Log,
}

impl Fake {
    /// A way that connects at once.
    pub fn up(label: &'static str, log: &Log) -> Fake {
        Fake {
            label,
            delay: Duration::ZERO,
            fails: false,
            names_inside: false,
            log: log.clone(),
        }
    }

    /// A way that fails at once.
    pub fn down(label: &'static str, log: &Log) -> Fake {
        Fake {
            fails: true,
            ..Fake::up(label, log)
        }
    }

    /// The same way, with `delay` before its end.
    pub fn after(mut self, delay: Duration) -> Fake {
        self.delay = delay;
        self
    }

    pub fn way(self) -> Way {
        Way::destination(Arc::new(self))
    }
}

impl Dial for Fake {
    fn label(&self) -> String {
        self.label.to_string()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(self.label.to_string());
            tokio::time::sleep(self.delay).await;
            if self.fails {
                return Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    format!("{} failed", self.label),
                ));
            }
            let (near, mut far) = tokio::io::duplex(256);
            let label = self.label.as_bytes().to_vec();
            tokio::spawn(async move {
                let _ = far.write_all(&label).await;
                let _ = far.shutdown().await;
                let mut rest = Vec::new();
                let _ = far.read_to_end(&mut rest).await;
            });
            Ok(Connected::new(Box::new(near)))
        })
    }

    fn names_inside(&self) -> bool {
        self.names_inside
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(format!("{} udp", self.label));
            Err(Failure::new(
                Stage::Connect,
                Class::Unavailable,
                format!("{} carries no UDP", self.label),
            ))
        })
    }
}

/// A new empty log.
pub fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

/// The labels in `log`.
pub fn seen(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// The label that the far end of `conn` wrote: the way that carried it.
pub async fn carrier(mut conn: Connected) -> String {
    let mut text = String::new();
    conn.io.read_to_string(&mut text).await.unwrap();
    text
}

/// A deadline in 30 s.
pub fn soon() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
