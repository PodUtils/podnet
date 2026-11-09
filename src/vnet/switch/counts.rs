//! The counters of a switch.

/// The packets that the switch queued for one peer, and the packets that it
/// dropped because the queue of the peer was full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerCount {
    pub sent: u64,
    pub dropped: u64,
}

/// The counters of a switch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SwitchCounts {
    /// Each peer that is connected, by its id.
    pub peers: Vec<(u64, PeerCount)>,
    /// The sum of the peers that left.
    pub left: PeerCount,
    /// The packets with the source address of another peer.
    pub spoofed: u64,
}

impl SwitchCounts {
    /// The packets that the switch dropped on a full queue, in all.
    pub fn dropped(&self) -> u64 {
        self.left.dropped + self.peers.iter().map(|(_, c)| c.dropped).sum::<u64>()
    }
}
