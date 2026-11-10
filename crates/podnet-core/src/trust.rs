//! The roots that podnet trusts for each TLS connection that it opens as a
//! client: the quick-tunnel API, DNS over HTTPS, the WARP API, the fetch of
//! the public URL, the edge, and HTTPS origins. They are the roots of
//! `pod-tls` (PodUtils/podcrates) in one context of the process
//! ([`context`]), with podnet's settings and ring as its provider.
//!
//! The order:
//!
//! 1. A file given with `--ca-file`, else the file in `SSL_CERT_FILE`. The file
//!    is used alone: podnet adds no other root to it.
//! 2. Else all of these: `podnet-ca.pem` beside the binary, the first readable
//!    bundle of [`pod_tls::SYSTEM_BUNDLES`], and the Mozilla roots compiled
//!    into podnet (webpki-roots [`pod_tls::ROOTS_VERSION`]).
//!
//! The edge also trusts the Cloudflare roots of the edge of `pod-services`
//! (`edge_roots_pem` of the Cloudflare adapter). podnet has
//! no option that accepts each certificate for these connections; only an
//! origin can turn off the check (`noTLSVerify`, `--no-tls-verify`).
//!
//! When a certificate is expired or not valid yet, [`pod_tls::clock_hint`]
//! gives the clock of the host, because a wrong clock is a frequent cause on
//! a small host or in a container.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use pod_tls::{CaFile, Context, Settings, Words};

/// The variable that names a file of roots, as in OpenSSL.
pub const CA_FILE_VARIABLE: &str = "SSL_CERT_FILE";
/// The name of the file of roots beside the binary.
pub const BESIDE_BINARY: &str = "podnet-ca.pem";
/// The first date when a root of the edge stops being valid: the fact
/// `roots-first-end` of the edge (`pod-services`).
pub fn edge_roots_first_end() -> String {
    let edge = crate::services::get(crate::services::EDGE);
    edge.date("roots-first-end")
        .map(|d| d.to_string())
        .unwrap_or_default()
}

static CA_FILE: OnceLock<PathBuf> = OnceLock::new();

/// Use `path` as the only file of roots (`--ca-file`). Call it before the
/// first connection; a later call has no effect.
pub fn set_ca_file(path: PathBuf) {
    let _ = CA_FILE.set(path);
}

/// The TLS of this process: its roots, read one time, and its client
/// configurations, on ring.
pub fn context() -> &'static Arc<Context> {
    static CONTEXT: OnceLock<Arc<Context>> = OnceLock::new();
    CONTEXT.get_or_init(|| {
        let mut words = Words::new("podnet");
        words.replace_roots = format!("--ca-file FILE or {CA_FILE_VARIABLE}");
        words.add_roots = Some(format!("{BESIDE_BINARY} beside the binary"));
        let mut settings = Settings::new(words);
        settings.ca_file = CA_FILE
            .get()
            .map(|path| CaFile::new("--ca-file", path.clone()))
            .or_else(|| {
                std::env::var_os(CA_FILE_VARIABLE)
                    .filter(|v| !v.is_empty())
                    .map(|v| CaFile::new(CA_FILE_VARIABLE, v))
            });
        settings.extra_bundle = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(BESIDE_BINARY)));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(Context::new(provider, settings))
    })
}

/// The seconds of `time` since the Unix epoch, negative before it, for
/// [`pod_core::clock::http_date`].
pub fn unix_secs(time: std::time::SystemTime) -> i64 {
    match time.duration_since(std::time::SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}
