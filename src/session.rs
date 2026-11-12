//! The resume of a link after the loss of its carrier (T-263).
//!
//! `pod-session` of PodUtils/podcrates does the work (podcrates call C119):
//! each side numbers the bytes that it sends with offsets that never reset,
//! the first carrier sets a session id and a 256-bit secret, and after a
//! loss the dialer proves the secret on a new carrier and both sides send
//! again from the offset of the other. This module is the glue. The link of
//! [`crate::link`] runs over the application side of a session, so the
//! stream table of the link survives the loss of a carrier:
//!
//! * [`Session::dial`] is the dialer. It starts the session on a first
//!   carrier and asks `connect` for a new carrier after each loss.
//! * [`Far::accept`] is the listener. It takes each carrier: a new session
//!   gives a [`Link`]; a resume joins the session that it names.
//! * The window is [`RESUME_WINDOW`], 10 minutes: the setting
//!   `resume_deadline` of `pod-session`, at both sides. After it, the far
//!   end has forgotten the session, and the resume fails with
//!   [`ResumeExpired`] (exit 69). [`link_settings`] sets `dead_after` of the
//!   link to the window, so that the link does not end as dead before the
//!   window ends.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use pod_session::client::{self, Outcome};
use pod_session::keep::Keeper;
use pod_session::record::RefuseCode;
use pod_session::resume;
pub use pod_session::resume::{Next, Note};
use pod_session::{far, Ask, End, LinkError, OsEntropy, Role as SessionRole};
pub use pod_session::{Ended, Settings};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::exit::{self, Fault};
use crate::link::{self, Link, Role};

/// How long a session waits for a new carrier after a loss: 10 minutes.
pub const RESUME_WINDOW: Duration = resume::RESUME_DEADLINE;

/// The bytes between the layer and the link, in each direction.
const PIPE: usize = 256 * 1024;

/// The settings of the layer for a window of `window`.
pub fn settings(window: Duration) -> Settings {
    Settings {
        resume_deadline: window,
        ..Settings::default()
    }
}

/// `base`, with a dead time of the link at the window or more. A link ends
/// as dead when its peer is silent for `dead_after`; while the carrier is
/// lost the peer is silent for the window.
pub fn link_settings(base: link::Settings, window: Duration) -> link::Settings {
    link::Settings {
        dead_after: base.dead_after.max(window),
        ..base
    }
}

/// The session ended because the window ended: no new carrier came in
/// `window`, or the far end had forgotten the session when one came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeExpired {
    /// The window of the session.
    pub window: Duration,
    /// What `pod-session` said.
    pub reason: String,
}

impl std::fmt::Display for ResumeExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the resume window of {} s ended: {}",
            self.window.as_secs(),
            self.reason
        )
    }
}

impl std::error::Error for ResumeExpired {}

/// The expiry that `end` means, when it means one. `pod-session` says it as
/// the text of `End::GaveUp` (the far end has no such session, or no new
/// carrier came in the window), or as a `REFUSE` with the code
/// `UNKNOWN_SESSION` (podcrates ask A12: a typed cause).
pub fn expired(end: &End, window: Duration) -> Option<ResumeExpired> {
    let reason = match end {
        End::GaveUp(reason)
            if reason.contains("no such session") || reason.contains("no new link within") =>
        {
            reason.clone()
        }
        End::Broken(LinkError::Refused { code, reason })
            if *code == RefuseCode::UNKNOWN_SESSION =>
        {
            format!("the far end refused the resume: {reason}")
        }
        _ => return None,
    };
    Some(ResumeExpired { window, reason })
}

fn failure(why: impl std::fmt::Display) -> anyhow::Error {
    exit::fault(Fault::Unavailable, anyhow!("{why}"))
}

/// The dialer side of a session: a [`Link`] that survives the loss of its
/// carrier.
pub struct Session {
    link: Link,
    window: Duration,
    task: Option<JoinHandle<Outcome>>,
}

impl Session {
    /// Start a session on `first`, a carrier that is connected, and run a
    /// link over it as the dialer. After a loss, `connect` gives a new
    /// carrier (or `Next::Retry`, or `Next::Stop`); `note` hears of each
    /// loss and resume. The session ends when the link ends, the peer
    /// closes it, or the window ends ([`Session::finish`]).
    pub async fn dial<C, F, Fut, N>(
        first: C,
        connect: F,
        window: Duration,
        link_settings_of: link::Settings,
        note: N,
    ) -> anyhow::Result<Session>
    where
        C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        F: FnMut(&Ended) -> Fut + Send + 'static,
        Fut: Future<Output = Next<C>> + Send + 'static,
        N: FnMut(Note) + Send + 'static,
    {
        let layer = settings(window);
        let client = client::start(first, Ask::New, layer, &mut OsEntropy)
            .await
            .map_err(failure)?;
        let (app, app_end) = tokio::io::duplex(PIPE);
        let task = tokio::spawn(async move {
            resume::run(client, app_end, connect, note, &mut OsEntropy).await
        });
        let link = link::from_io(app, Role::Dialer, link_settings(link_settings_of, window))?;
        Ok(Session {
            link,
            window,
            task: Some(task),
        })
    }

    /// The link. It stays open over the loss of a carrier.
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Wait for the end of the session. `Ok` when it ended with the link;
    /// else the reason: [`ResumeExpired`] when the window ended.
    pub async fn finish(mut self) -> anyhow::Result<()> {
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        match task.await {
            Ok(Outcome::Layer(ended)) => match &ended.end {
                End::LocalEnd | End::Closed(_) => Ok(()),
                end => match expired(end, self.window) {
                    Some(e) => Err(exit::fault(
                        Fault::Unavailable,
                        anyhow::Error::new(e).context("the session of the link ended"),
                    )),
                    None => Err(failure(format!("the session ended: {}", end.loss()))),
                },
            },
            Ok(Outcome::Plain(None)) => Ok(()),
            Ok(Outcome::Plain(Some(e))) => Err(failure(e)),
            Err(e) => Err(failure(e)),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// The listener side: it keeps the sessions of its dialers across carriers.
pub struct Far {
    keeper: Arc<Keeper<DuplexStream>>,
    window: Duration,
    lost: Arc<AtomicUsize>,
    notes: Arc<Mutex<Vec<String>>>,
}

impl Far {
    /// A far end that keeps a lost session for `window`.
    pub fn new(window: Duration) -> Far {
        Far {
            keeper: Arc::new(Keeper::new(window)),
            window,
            lost: Arc::default(),
            notes: Arc::default(),
        }
    }

    /// Take one carrier. A new session gives its [`Link`] as the listener;
    /// a resume joins the session that it names and gives `None` (the link
    /// of the session goes on). A refused resume (the session is
    /// forgotten, or the proof is wrong) is an error.
    pub async fn accept<C>(
        &self,
        carrier: C,
        link_settings_of: link::Settings,
    ) -> anyhow::Result<Option<Link>>
    where
        C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let accepted = far::accept(
            carrier,
            SessionRole::NODE,
            settings(self.window),
            &self.keeper.sessions,
            &mut OsEntropy,
        )
        .await
        .map_err(failure)?;
        let (keeper, lost) = (self.keeper.clone(), self.lost.clone());
        if accepted.established().resumed {
            let notes = self.notes.clone();
            tokio::spawn(async move {
                match keeper.run_resumed(accepted).await {
                    Ok(ended) if ended.end.resumable() => {
                        lost.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(_) => {}
                    Err(why) => notes.lock().unwrap_or_else(|e| e.into_inner()).push(why),
                }
            });
            return Ok(None);
        }
        let (target, app) = tokio::io::duplex(PIPE);
        let link = link::from_io(
            app,
            Role::Listener,
            link_settings(link_settings_of, self.window),
        )?;
        tokio::spawn(async move {
            let ended = keeper.run_new(accepted, target).await;
            if ended.end.resumable() {
                lost.fetch_add(1, Ordering::SeqCst);
            }
        });
        Ok(Some(link))
    }

    /// How many times a carrier was lost while its session stayed.
    pub fn lost(&self) -> usize {
        self.lost.load(Ordering::SeqCst)
    }

    /// Forget each session that has been without a carrier for the window.
    /// The count forgotten.
    pub async fn expire(&self) -> usize {
        self.keeper.expire().await
    }

    /// [`Far::expire`] as at `now`. A test gives a later `now`: a clock
    /// that ran on.
    pub async fn expire_at(&self, now: Instant) -> usize {
        self.keeper.expire_at(now).await
    }

    /// The refusals of resumes that the far end heard.
    pub fn refusals(&self) -> Vec<String> {
        self.notes.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
