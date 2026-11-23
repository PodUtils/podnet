//! Deferred responses (T-371, GitHub #494): prepare the next response while
//! the client is served a plausible one now.
//!
//! The first request of a key is served live, with the origin's own timing
//! (DEFER-4). The answer is then prepared for the next request of the same
//! key, and that request is served from the prepared copy after a plausible
//! delay (DEFER-1). A prepared copy is never put in place of a response that
//! is already on its way: the cache holds the next response only.
//!
//! The most bytes of one prepared response is the chunk size of this cache
//! (DEFER-2). A response that is larger is not prepared, so it is served
//! live each time. The bandwidth that the cache adds is the bytes of the
//! prepared copies that no client took, and it stays within that bound.
//!
//! The traffic generator of [`generator`] gives request/response
//! alternation at human timescales from a profile file. It is a tool for the
//! tests, not a mode of a road (SIM-4).

pub mod cache;
pub mod carrier;
pub mod generator;

pub use cache::{Defer, HttpOrigin, Origin, Served, Timing};
pub use carrier::Carrier;
pub use generator::{Generator, Profile, Request, NAMES as PROFILE_NAMES};
