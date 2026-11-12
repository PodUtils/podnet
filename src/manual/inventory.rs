//! The section INVENTORY of the manual: the carriers and services of
//! `podnet components`.

use serde::Serialize;

/// One component of the inventory (`podnet components`): a carrier or a
/// service. `tests/cli/manual.rs` checks that each row of the inventory
/// that this build holds has an entry.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ComponentEntry {
    pub name: &'static str,
    /// `carrier` or `service`.
    pub kind: &'static str,
    pub meaning: &'static str,
}

/// Each carrier and service of the inventory.
pub const COMPONENTS: &[ComponentEntry] = &[
    ComponentEntry {
        name: "unix",
        kind: "carrier",
        meaning: "A stream over AF_UNIX at a path, mode 0600. Each end checks the uid of the other end (SO_PEERCRED); --allow-other-uid admits another uid. Duplex, ordered, no cost, no privilege. Address: PATH. Always built.",
    },
    ComponentEntry {
        name: "memory",
        kind: "carrier",
        meaning: "Two ends in one process with a ring for each direction (--ring sets the bytes). A full ring blocks the writer. For tests and for programs that embed the link. Duplex, ordered, no cost, no privilege. Address: NAME. Always built.",
    },
    ComponentEntry {
        name: "tcp",
        kind: "carrier",
        meaning: "A TCP stream with TLS 1.3, ALPN podlink/1. Each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. Duplex, ordered, cost class network, no privilege. Address: KEY@HOST:PORT to dial, IP:PORT to listen. Build feature tcp.",
    },
    ComponentEntry {
        name: "quic",
        kind: "carrier",
        meaning: "QUIC streams to a peer, TLS 1.3 with the key of the peer pinned. Cost class network, no privilege. Address: HOST:PORT. Build feature quic.",
    },
    ComponentEntry {
        name: "s3",
        kind: "carrier",
        meaning: "A simplex pair of objects in a bucket of an object store. Cost class network, no privilege. Address: BUCKET/PREFIX. Build feature s3.",
    },
    ComponentEntry {
        name: "icmp",
        kind: "carrier",
        meaning: "ICMP echo requests and replies, a datagram carrier. Cost class network. It needs a privilege of the process. Address: HOST. Build feature icmp.",
    },
    ComponentEntry {
        name: "ss",
        kind: "service",
        meaning: "The Shadowsocks service over a stream of the link. Cost class network, no privilege. Build feature ss.",
    },
    ComponentEntry {
        name: "trojan",
        kind: "service",
        meaning: "The Trojan service over a stream of the link. Cost class network, no privilege. Build feature trojan.",
    },
];
