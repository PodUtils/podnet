//! Link devices for the userspace stack: the devices of `pod-netstack`
//! (PodUtils/podcrates) at their old path.
//!
//! The stack's link layer is an in-process packet queue pair: packets the
//! stack transmits land in an outbound queue, and packets read from the
//! outside world are pushed into an inbound queue. A pump moves bytes between
//! those queues and the real link — a unix socket, a QUIC stream, or a
//! `podnet` WebSocket forward. The recorder of this crate
//! ([`crate::record::Recorder`]) is the packet tap of a [`RecordingDevice`].

pub use pod_netstack::device::*;

/// Alias kept for the protocol documentation's name.
pub type TunnelDevice = LinkDevice;

/// A device wrapper that records every packet in both directions with the
/// recorder of this crate.
pub type RecordingDevice<D> = pod_netstack::device::RecordingDevice<D, crate::record::Recorder>;
