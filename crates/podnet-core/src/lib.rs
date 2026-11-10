//! podnet-core: the core layer of podnet. The other crates of the
//! workspace use it; it uses none of them. It is a library of podnet, not a
//! stable interface: use the crate `podnet`.

// The macros below are a copy of the macros of `src/lib.rs` of podnet
// (T-046). Each crate defines them at its root. One macro of this crate
// that defines them in the crate that calls it does not compile: a
// `macro_rules! println` that a macro makes is ambiguous with the
// `println!` of the prelude (E0659).

/// `println!`, `print!` and `eprintln!` that drop a failed write (issue
/// 74). The std macros panic when the write fails: a detached instance
/// writes to its log, and a full disk or a closed stream must not stop it.
/// A command that only prints keeps the default action of SIGPIPE.
#[allow(unused_macros)]
macro_rules! println {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stdout(), $($arg)*);
    }};
}

#[allow(unused_macros)]
macro_rules! print {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::write!(::std::io::stdout(), $($arg)*);
    }};
}

#[allow(unused_macros)]
macro_rules! eprintln {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stderr(), $($arg)*);
    }};
}

/// `fetch_update` of an atomic with a loop of `compare_exchange_weak`
/// (#214: `fetch_update` is deprecated in a later Rust; `try_update`
/// needs a newer Rust than the MSRV). Gives the old value, or `Err` with
/// the value when `f` gives `None`.
#[allow(unused_macros)]
macro_rules! update_atomic {
    ($atomic:expr, $f:expr) => {{
        let atomic = $atomic;
        let f = $f;
        let mut now = atomic.load(::std::sync::atomic::Ordering::SeqCst);
        loop {
            match f(now) {
                None => break Err(now),
                Some(next) => {
                    match atomic.compare_exchange_weak(
                        now,
                        next,
                        ::std::sync::atomic::Ordering::SeqCst,
                        ::std::sync::atomic::Ordering::SeqCst,
                    ) {
                        Ok(old) => break Ok(old),
                        Err(seen) => now = seen,
                    }
                }
            }
        }
    }};
}

pub mod build;
