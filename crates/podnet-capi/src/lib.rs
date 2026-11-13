//! The C ABI of podnet (T-304): `include/podnet.h`.
//!
//! Each `podnet_*` function is an entry point. It runs inside
//! `guard`: a panic becomes `PODNET_E_PANIC`, and a failure becomes a
//! negative code and the thread-local text of `podnet_last_error`. A handle
//! is a boxed Rust value that the C side holds as an opaque pointer. A
//! blocking call runs on the tokio runtime that `podnet_init` starts, with
//! `block_on` on the thread of the caller.
//!
//! The list of the exported functions is the list of `include/podnet.h`
//! (`tests/c_abi.rs` checks it against the dynamic symbols of the shared
//! library; rustc's own version script hides the other symbols).

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use podnet::exit::code_of;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings};
use podnet::link_service;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::runtime::{Builder, Runtime};

/// The success code.
pub const OK: c_int = 0;
/// A bad argument.
pub const E_ARG: c_int = -1;
/// The peer, the service or the carrier is not there.
pub const E_UNAVAILABLE: c_int = -2;
/// A peer key was refused.
pub const E_PERMISSION: c_int = -3;
/// A failure of the link or of the host.
pub const E_IO: c_int = -4;
/// A call that does not fit the state of the handle.
pub const E_STATE: c_int = -5;
/// A size over a limit.
pub const E_LIMIT: c_int = -6;
/// A panic inside the library.
pub const E_PANIC: c_int = -99;

/// The most bytes of one read or write.
pub const MAX_IO: usize = 1 << 20;
/// The most bytes of the error text, with the NUL.
pub const MAX_ERROR: usize = 1 << 16;
/// The size of a buffer that always holds a fingerprint.
pub const FP_MAX: usize = 128;
/// The ring of the memory carrier of `podnet_link_pair`.
const RING: usize = 1 << 20;
/// The default and the most worker threads.
const THREADS: usize = 2;
const MAX_THREADS: u64 = 64;

static RUNTIME: Mutex<Option<Arc<Runtime>>> = Mutex::new(None);
/// The links and the streams that are open.
static LIVE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static LAST: RefCell<CString> = RefCell::new(CString::default());
}

/// A failure: a code and the text for `podnet_last_error`.
struct Fail {
    code: c_int,
    text: String,
}

type Res<T> = Result<T, Fail>;

fn fail<T>(code: c_int, text: impl Into<String>) -> Res<T> {
    Err(Fail {
        code,
        text: text.into(),
    })
}

/// A failure of podnet with the code of its exit class.
fn from(e: anyhow::Error) -> Fail {
    let code = match code_of(&e) {
        64 | 78 => E_ARG,
        69 => E_UNAVAILABLE,
        77 => E_PERMISSION,
        _ => E_IO,
    };
    Fail {
        code,
        text: format!("{e:#}"),
    }
}

fn set_error(text: &str) {
    let mut end = text.len().min(MAX_ERROR - 1);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let bytes: Vec<u8> = text.as_bytes()[..end]
        .iter()
        .map(|&b| if b == 0 { b'?' } else { b })
        .collect();
    let c = CString::new(bytes).unwrap_or_default();
    let _ = LAST.try_with(|l| *l.borrow_mut() = c);
}

/// Run an entry point: clear the last error, catch a panic, and map a
/// failure to its code and text.
fn guard(f: impl FnOnce() -> Res<c_int>) -> c_int {
    set_error("");
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => {
            set_error(&e.text);
            e.code
        }
        Err(p) => {
            let why = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no message".into());
            set_error(&format!("panic inside podnet: {why}"));
            E_PANIC
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A string argument: not NULL, UTF-8.
///
/// # Safety
/// `p` is NULL or a NUL-terminated string that lives as long as the call.
unsafe fn text<'a>(p: *const c_char, what: &str) -> Res<&'a str> {
    if p.is_null() {
        return fail(E_ARG, format!("{what} is NULL"));
    }
    match CStr::from_ptr(p).to_str() {
        Ok(s) => Ok(s),
        Err(_) => fail(E_ARG, format!("{what} is not UTF-8")),
    }
}

fn threads_of(config: &str) -> Res<usize> {
    if config.trim().is_empty() {
        return Ok(THREADS);
    }
    let value: serde_json::Value = serde_json::from_str(config).map_err(|e| Fail {
        code: E_ARG,
        text: format!("config_json is no JSON: {e}"),
    })?;
    let Some(object) = value.as_object() else {
        return fail(E_ARG, "config_json is no JSON object");
    };
    if let Some(key) = object.keys().find(|k| k.as_str() != "threads") {
        return fail(E_ARG, format!("config_json: unknown key {key:?}"));
    }
    match object.get("threads") {
        None => Ok(THREADS),
        Some(v) => match v.as_u64() {
            Some(n) if (1..=MAX_THREADS).contains(&n) => Ok(n as usize),
            _ => fail(E_ARG, "config_json: threads is 1 to 64"),
        },
    }
}

fn start(threads: usize) -> Res<Arc<Runtime>> {
    let mut slot = lock(&RUNTIME);
    if let Some(rt) = slot.as_ref() {
        return Ok(rt.clone());
    }
    let rt = Builder::new_multi_thread()
        .worker_threads(threads)
        .thread_name("podnet")
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| Fail {
            code: E_IO,
            text: format!("no runtime: {e}"),
        })?;
    let rt = Arc::new(rt);
    *slot = Some(rt.clone());
    Ok(rt)
}

/// One end of a link.
pub struct Endpoint {
    rt: Arc<Runtime>,
    identity: Identity,
    allow: AllowList,
    link: Mutex<Option<Arc<Link>>>,
}

/// One stream of a link.
pub struct StreamHandle {
    rt: Arc<Runtime>,
    // The link lives as long as its streams.
    _link: Arc<Link>,
    service: CString,
    read: Mutex<ReadHalf<link::Stream>>,
    write: Mutex<WriteHalf<link::Stream>>,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

fn make_stream(
    rt: &Arc<Runtime>,
    link: &Arc<Link>,
    stream: link::Stream,
    service: &str,
) -> Res<*mut StreamHandle> {
    let service = CString::new(service).map_err(|_| Fail {
        code: E_ARG,
        text: "the service name holds a NUL".into(),
    })?;
    let (r, w) = tokio::io::split(stream);
    LIVE.fetch_add(1, Ordering::SeqCst);
    Ok(Box::into_raw(Box::new(StreamHandle {
        rt: rt.clone(),
        _link: link.clone(),
        service,
        read: Mutex::new(r),
        write: Mutex::new(w),
    })))
}

/// The link of an end, or a failure when no `podnet_link_pair` joined it.
///
/// # Safety
/// `l` is NULL or a live handle.
unsafe fn joined<'a>(l: *mut Endpoint) -> Res<(&'a Endpoint, Arc<Link>)> {
    let Some(end) = l.as_ref() else {
        return fail(E_ARG, "the link is NULL");
    };
    match lock(&end.link).clone() {
        Some(link) => Ok((end, link)),
        None => fail(E_STATE, "the link has no peer: call podnet_link_pair"),
    }
}

/// # Safety
/// `config_json` is NULL or a string; `err` is NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn podnet_init(config_json: *const c_char, err: *mut *const c_char) -> c_int {
    let code = guard(|| {
        let config = if config_json.is_null() {
            ""
        } else {
            text(config_json, "config_json")?
        };
        let threads = threads_of(config)?;
        start(threads)?;
        Ok(OK)
    });
    if code != OK && !err.is_null() {
        *err = podnet_last_error();
    }
    code
}

#[no_mangle]
pub extern "C" fn podnet_free() -> c_int {
    guard(|| {
        let mut slot = lock(&RUNTIME);
        if LIVE.load(Ordering::SeqCst) != 0 {
            return fail(E_STATE, "a link or a stream is still open");
        }
        if let Some(rt) = slot.take() {
            if let Ok(rt) = Arc::try_unwrap(rt) {
                // Join the worker threads and the blocking threads (at most 5 s), so
                // that a leak check sees no thread stack.
                rt.shutdown_timeout(std::time::Duration::from_secs(5));
            }
        }
        Ok(OK)
    })
}

/// # Safety
/// `identity_path` is a string; `buf` has `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn podnet_identity_fingerprint(
    identity_path: *const c_char,
    buf: *mut c_char,
    len: usize,
) -> c_int {
    guard(|| {
        if buf.is_null() {
            return fail(E_ARG, "buf is NULL");
        }
        let dir = text(identity_path, "identity_path")?;
        let fp = identity::create(Path::new(dir))
            .map_err(from)?
            .fingerprint();
        if fp.len() >= len.min(FP_MAX) {
            return fail(E_LIMIT, "buf is too small for the fingerprint");
        }
        std::ptr::copy_nonoverlapping(fp.as_ptr(), buf as *mut u8, fp.len());
        *buf.add(fp.len()) = 0;
        Ok(OK)
    })
}

/// # Safety
/// The strings are NUL-terminated; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn podnet_link_new(
    identity_path: *const c_char,
    peer_fp: *const c_char,
    out: *mut *mut Endpoint,
) -> c_int {
    guard(|| {
        if out.is_null() {
            return fail(E_ARG, "out is NULL");
        }
        *out = std::ptr::null_mut();
        let dir = text(identity_path, "identity_path")?;
        let fp = text(peer_fp, "peer_fp")?.trim();
        if fp.is_empty() || fp.contains(['\n', '\r']) {
            return fail(E_ARG, "peer_fp is one fingerprint");
        }
        let allow = AllowList::parse(fp).map_err(from)?;
        if allow.names().len() != 1 {
            return fail(E_ARG, "peer_fp is one fingerprint");
        }
        let identity = identity::create(Path::new(dir)).map_err(from)?;
        let rt = start(THREADS)?;
        LIVE.fetch_add(1, Ordering::SeqCst);
        *out = Box::into_raw(Box::new(Endpoint {
            rt,
            identity,
            allow,
            link: Mutex::new(None),
        }));
        Ok(OK)
    })
}

/// # Safety
/// Each argument is NULL or a live handle.
#[no_mangle]
pub unsafe extern "C" fn podnet_link_pair(dialer: *mut Endpoint, listener: *mut Endpoint) -> c_int {
    guard(|| {
        let (Some(d), Some(l)) = (dialer.as_ref(), listener.as_ref()) else {
            return fail(E_ARG, "a link is NULL");
        };
        if std::ptr::eq(d, l) {
            return fail(E_ARG, "the two ends are one handle");
        }
        let (mut dg, mut lg) = (lock(&d.link), lock(&l.link));
        if dg.is_some() || lg.is_some() {
            return fail(E_STATE, "an end has a peer already");
        }
        let (dl, ll) = d.rt.block_on(async {
            let (dl, ll) = link::memory_pair(RING, Settings::default()).map_err(from)?;
            let (a, b) = tokio::join!(
                hello::dial(&dl, &d.identity, d.allow.clone()),
                hello::accept(&ll, &l.identity, l.allow.clone())
            );
            a.map_err(from)?;
            b.map_err(from)?;
            Ok::<_, Fail>((dl, ll))
        })?;
        *dg = Some(Arc::new(dl));
        *lg = Some(Arc::new(ll));
        Ok(OK)
    })
}

/// # Safety
/// `l` is a live handle; `service` is a string; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn podnet_link_open(
    l: *mut Endpoint,
    service: *const c_char,
    out: *mut *mut StreamHandle,
) -> c_int {
    guard(|| {
        if out.is_null() {
            return fail(E_ARG, "out is NULL");
        }
        *out = std::ptr::null_mut();
        let service = text(service, "service")?;
        let (end, link) = joined(l)?;
        let stream = end.rt.block_on(async {
            link_service::check_name(service)?;
            let mut stream = link.open().map_err(link::fault)?;
            link_service::request(&mut stream, service).await?;
            Ok::<_, anyhow::Error>(stream)
        });
        *out = make_stream(&end.rt, &link, stream.map_err(from)?, service)?;
        Ok(OK)
    })
}

/// # Safety
/// `l` is a live handle; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn podnet_link_accept(
    l: *mut Endpoint,
    out: *mut *mut StreamHandle,
) -> c_int {
    guard(|| {
        if out.is_null() {
            return fail(E_ARG, "out is NULL");
        }
        *out = std::ptr::null_mut();
        let (end, link) = joined(l)?;
        let (stream, service) = end
            .rt
            .block_on(async {
                let mut stream = link.accept().await.map_err(link::fault)?;
                let name = link_service::accept(&mut stream).await?;
                link_service::reply(&mut stream, link_service::OPEN).await?;
                Ok::<_, anyhow::Error>((stream, name))
            })
            .map_err(from)?;
        *out = make_stream(&end.rt, &link, stream, &service)?;
        Ok(OK)
    })
}

/// # Safety
/// `l` is NULL or a live handle that no thread uses; it is freed.
#[no_mangle]
pub unsafe extern "C" fn podnet_link_close(l: *mut Endpoint) -> c_int {
    guard(|| {
        if l.is_null() {
            return fail(E_ARG, "the link is NULL");
        }
        let end = Box::from_raw(l);
        let link = lock(&end.link).take();
        if let Some(link) = link {
            end.rt.block_on(link.close());
        }
        Ok(OK)
    })
}

/// # Safety
/// `s` is NULL or a live handle.
#[no_mangle]
pub unsafe extern "C" fn podnet_stream_service(s: *const StreamHandle) -> *const c_char {
    match catch_unwind(AssertUnwindSafe(|| match s.as_ref() {
        Some(s) => s.service.as_ptr(),
        None => std::ptr::null(),
    })) {
        Ok(p) => p,
        Err(_) => std::ptr::null(),
    }
}

/// # Safety
/// `s` is a live handle; `buf` has `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn podnet_stream_read(
    s: *mut StreamHandle,
    buf: *mut c_void,
    len: usize,
) -> c_int {
    guard(|| {
        let Some(s) = s.as_ref() else {
            return fail(E_ARG, "the stream is NULL");
        };
        if buf.is_null() || len == 0 {
            return fail(E_ARG, "buf is NULL or len is 0");
        }
        if len > MAX_IO {
            return fail(E_LIMIT, "len is over 1 MiB");
        }
        let buf = std::slice::from_raw_parts_mut(buf as *mut u8, len);
        let mut half = lock(&s.read);
        match s.rt.block_on(half.read(buf)) {
            Ok(n) => Ok(n as c_int),
            Err(e) => fail(E_IO, format!("read: {e}")),
        }
    })
}

/// # Safety
/// `s` is a live handle; `buf` has `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn podnet_stream_write(
    s: *mut StreamHandle,
    buf: *const c_void,
    len: usize,
) -> c_int {
    guard(|| {
        let Some(s) = s.as_ref() else {
            return fail(E_ARG, "the stream is NULL");
        };
        if len > MAX_IO {
            return fail(E_LIMIT, "len is over 1 MiB");
        }
        if len == 0 {
            return Ok(0);
        }
        if buf.is_null() {
            return fail(E_ARG, "buf is NULL");
        }
        let buf = std::slice::from_raw_parts(buf as *const u8, len);
        let mut half = lock(&s.write);
        match s.rt.block_on(half.write_all(buf)) {
            Ok(()) => Ok(len as c_int),
            Err(e) => fail(E_IO, format!("write: {e}")),
        }
    })
}

/// # Safety
/// `s` is NULL or a live handle that no thread uses; it is freed.
#[no_mangle]
pub unsafe extern "C" fn podnet_stream_close(s: *mut StreamHandle) -> c_int {
    guard(|| {
        if s.is_null() {
            return fail(E_ARG, "the stream is NULL");
        }
        let s = Box::from_raw(s);
        let mut half = lock(&s.write);
        // The end of the stream may fail when the peer is gone: the handle
        // is freed all the same.
        let _ = s.rt.block_on(half.shutdown());
        drop(half);
        Ok(OK)
    })
}

#[no_mangle]
pub extern "C" fn podnet_last_error() -> *const c_char {
    LAST.try_with(|l| l.borrow().as_ptr())
        .unwrap_or(c"".as_ptr())
}

#[no_mangle]
pub extern "C" fn podnet_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

#[cfg(test)]
mod tests;
