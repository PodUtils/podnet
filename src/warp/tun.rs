//! A TUN interface over a WARP session (`podnet warp tun`, `podnet warp
//! run`): `pod-warp`'s TUN and its loop, with podnet's `--probe`,
//! `--session-max-age` and `--keep-rp-filter`.

use anyhow::Result;

use super::MasqueSession;
use super::Warped as _;

pub use pod_warp::tun::{probe, Tun, TunSettings, KEEPALIVE};

/// The settings of the loop of this process.
pub fn settings() -> TunSettings {
    TunSettings {
        probe: super::live::probe(),
        max_age: super::live::max_age(),
        rp_filter: Some(rp_filter::system()),
        report: super::report(),
        ..TunSettings::default()
    }
}

/// Move packets between `tun` and `session` until one of them ends: the
/// session closes, the TCP probe gets no answer (T-069), or the session
/// reaches `--session-max-age`.
pub async fn run(tun: &Tun, session: MasqueSession) -> Result<()> {
    pod_warp::tun::run(tun, session, &settings()).await.warped()
}

/// The reverse path filter of the TUN of `warp tun` (T-072).
pub mod rp_filter {
    use std::sync::atomic::{AtomicBool, Ordering};

    pub use pod_warp::tun::rp_filter::{effective, RpFilter};

    static KEEP: AtomicBool = AtomicBool::new(false);

    /// `--keep-rp-filter`: do not write `rp_filter`.
    pub fn set_keep(keep: bool) {
        KEEP.store(keep, Ordering::SeqCst);
    }

    /// The filter of the system, with `--keep-rp-filter`.
    pub fn system() -> RpFilter {
        RpFilter::system(KEEP.load(Ordering::SeqCst))
    }

    /// Write 0 for `interface` (unless `--keep-rp-filter`), and give the
    /// value that the kernel uses, with a warning when it is not 0.
    pub fn apply(interface: &str) -> Option<u32> {
        system().apply(interface, &super::super::report())
    }
}
