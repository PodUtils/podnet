//! The carriers and the stores of a host with no network (PodUtils/podnet#499).
//!
//! A host with no path has a serial port, a radio and a disk. This module
//! is the glue between `pod-offline` of PodUtils/podcrates and the surfaces
//! of `podnet`: the carriers [`serial`] and [`hf`] of `--carrier`, and the
//! commands `spool`, `discover`, `bundle`, `capture`, `pair` and `replica`.
//!
//! * [`serial`]: `serial:PATH[,BAUD]`, AX.25 in KISS framing, 256-byte
//!   frames, a window of 1 to 7 with acknowledgements, and a retransmission
//!   timeout that comes from the baud rate.
//! * [`hf`]: `hf:PATH[,BPS]`, an ARDOP or VARA modem on the same serial
//!   line, 300 to 3000 bit/s, a busy signal, a turnaround delay, and the
//!   degraded profile of the deferring ARQ.
//! * [`mdns`]: the DNS-SD record of the signed announcement on mDNS, and
//!   the two commands that send and hear it.
//!
//! The stores are the modules of `pod-offline` itself, re-exported here so
//! that a command and a test name one crate: [`bundle`], [`capture`],
//! [`discovery`], [`pairing`], [`replication`] and [`spool`]. There is no
//! second spool, codec or serial port in this tree.

pub mod bridge;
pub mod hf;
pub mod mdns;
pub mod serial;

pub use pod_offline::{bundle, capture, discovery, pairing, replication, spool, Error};
