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
        name: "ws",
        kind: "carrier",
        meaning: "A WebSocket over TLS 1.3, ALPN http/1.1, binary messages only. Each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. Duplex, ordered, cost class network, no privilege. Address: KEY@wss://HOST[:PORT]/PATH to dial, IP:PORT to listen. Build feature ws.",
    },
    ComponentEntry {
        name: "h2",
        kind: "carrier",
        meaning: "One POST of HTTP/2 over TLS 1.3, ALPN h2: the request body is one direction and the response body is the other. Each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. Duplex, ordered, cost class network, no privilege. Address: KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen. Build feature h2.",
    },
    ComponentEntry {
        name: "quic",
        kind: "carrier",
        meaning: "One stream of a QUIC connection, TLS 1.3 with ALPN podlink/1. Each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. Duplex, ordered, cost class network, no privilege. Address: KEY@HOST:PORT to dial, IP:PORT (UDP) to listen. Build feature quic.",
    },
    ComponentEntry {
        name: "udp",
        kind: "carrier",
        meaning: "KCP over UDP (datagrams of 1200 bytes, segments of 1 KiB, the congestion window of KCP), TLS 1.3 with ALPN podlink/1 above it. Each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. The query key kcp-nodelay=1 turns on the nodelay of KCP with a window of 1024 segments: it is unfair to other flows. Duplex, ordered, cost class network, no privilege. Address: KEY@HOST:PORT to dial, IP:PORT (UDP) to listen. Build feature udp.",
    },
    ComponentEntry {
        name: "poll",
        kind: "carrier",
        meaning: "Bytes up in POSTs, bytes down in a POST that the listener holds up to 25 s (the query key poll=1..60 sets the hold). Overhead class medium. TLS 1.3 with ALPN http/1.1; each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. No request lives as long as the link, so a proxy that ends idle connections or a front of HTTP/1.1 can carry it. A listener of any poll carrier serves each form. Duplex, ordered, cost class network, no privilege. Address: KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen. Build feature poll.",
    },
    ComponentEntry {
        name: "sse",
        kind: "carrier",
        meaning: "Bytes up in POSTs, bytes down in one answer of server-sent events (base64, 8 KiB each, a ping comment each 15 s). A listener that refuses a push makes the dialer take the long poll. Overhead class high. TLS 1.3 with ALPN http/1.1; each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. No request lives as long as the link, so a proxy that ends idle connections or a front of HTTP/1.1 can carry it. A listener of any poll carrier serves each form. Duplex, ordered, cost class network, no privilege. Address: KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen. Build feature poll.",
    },
    ComponentEntry {
        name: "chunked",
        kind: "carrier",
        meaning: "Bytes up in POSTs, bytes down in one chunked answer of raw bytes. A listener that refuses a push makes the dialer take the long poll. Overhead class medium. TLS 1.3 with ALPN http/1.1; each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. No request lives as long as the link, so a proxy that ends idle connections or a front of HTTP/1.1 can carry it. A listener of any poll carrier serves each form. Duplex, ordered, cost class network, no privilege. Address: KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen. Build feature poll.",
    },
    ComponentEntry {
        name: "httppoll",
        kind: "carrier",
        meaning: "The simplex poll: bytes up in POSTs (202 and an ack), bytes down in a GET at once (204 when empty) again after 100 ms to 5 s; 64 KiB for each request. Overhead class highest. TLS 1.3 with ALPN http/1.1; each end pins the Ed25519 key of the other end in the handshake; a key that is not pinned exits 77. No request lives as long as the link, so a proxy that ends idle connections or a front of HTTP/1.1 can carry it. A listener of any poll carrier serves each form. Duplex, ordered, cost class network, no privilege. Address: KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen. Build feature poll.",
    },
    ComponentEntry {
        name: "dns",
        kind: "carrier",
        meaning: "The DNS simplex carrier: the bytes up go in base32 labels of query names under a zone, the bytes down in TXT answers (1232 bytes with EDNS0). The ARQ of pod-link runs over the queries and TLS 1.3 with ALPN podlink/1 over the ARQ; each end pins the Ed25519 key of the other end; a key that is not pinned exits 77. The dialer starts at most 20 queries a second, 8 in flight, and polls when it has nothing to send. Duplex, ordered, cost class network, overhead highest, no privilege. Address: KEY@ZONE to dial (--dns-server IP:PORT, default the first nameserver of /etc/resolv.conf), IP:PORT (UDP) to listen as the authoritative server of --dns-zone ZONE. Build feature dns.",
    },
    ComponentEntry {
        name: "s3",
        kind: "carrier",
        meaning: "The object-storage carrier: each end writes its bytes as objects of a bucket of the S3 API (256 KiB each, 8 in flight, a PUT with If-None-Match, a GET with If-Match) and polls the objects of the other end (100 ms to 5 s). TLS 1.3 with ALPN podlink/1 runs over the objects; each end pins the Ed25519 key of the other end; a key that is not pinned exits 77. --s3-endpoint URL is necessary (no default host); --s3-region (default us-east-1); the key of the bucket comes from --s3-credentials FILE, else AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, else a profile file, never from argv; no key, or a key that the bucket refuses, exits 77. --s3-prune removes each object read. Duplex, ordered, cost class network, overhead highest, no privilege. Address: KEY@BUCKET[/PREFIX] to dial, BUCKET[/PREFIX] to listen. Build feature s3.",
    },
    ComponentEntry {
        name: "icmp",
        kind: "carrier",
        meaning: "The ICMP echo carrier: the frames ride in the data of ICMP echo messages (1400 bytes of payload) on a raw ICMPv4 socket, with a session id that tells one link from another. The ARQ of pod-link runs over the echoes and TLS 1.3 with ALPN podlink/1 over the ARQ; each end pins the Ed25519 key of the other end; a key that is not pinned exits 77. It needs CAP_NET_RAW (or uid 0) and the flag --allow-raw icmp; a host that refuses the raw socket exits 69. Duplex, ordered, cost class network, overhead high, privilege needed. Address: KEY@HOST to dial (IPv4), an IPv4 address with no port to listen. Build feature icmp.",
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
