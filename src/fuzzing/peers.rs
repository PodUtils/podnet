//! The fuzz targets of the parsers of peer and file input that came after
//! 2026-11-12 (T-338): the wg-quick file, the network specs and the router
//! file, the messages, the zip and the records of the transfer, and the
//! route announcements of the link mesh. Each function takes untrusted
//! bytes and must not panic, hang or allocate much more than the input.
//!
//! No target opens a file of the input: a spec or a file that holds `wg:`
//! (a path of a wg-quick file) is not parsed.

use std::io::Write as _;

use pod_identity::PublicKey;

use crate::mesh_table::{Announcement, Limits, RouteTable};
use crate::transfer::messages::{self, Hint, Message, Offer};
use crate::transfer::transit::{self, Records, Role};
use crate::transfer::{land, piping, zip};

fn text(data: &[u8]) -> Option<&str> {
    std::str::from_utf8(data).ok()
}

/// A wg-quick file (`wg:FILE`, `podnet wg serve FILE`): the parse, the
/// checks of `wg serve`, and the rules of a packet of a peer. The bytes
/// after the first 0 byte are the packet.
pub fn wg_quick(data: &[u8]) {
    let (file, packet) = match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[][..]),
    };
    let Some(file) = text(file) else { return };
    let Ok(conf) = crate::vnet::wg_quick::parse(file) else {
        return;
    };
    let cidrs = conf
        .interface
        .addresses
        .iter()
        .chain(conf.peers.iter().flat_map(|p| p.allowed_ips.iter()));
    for cidr in cidrs {
        assert!(cidr.prefix <= if cidr.addr.is_ipv4() { 32 } else { 128 });
    }
    for peer in &conf.peers {
        if let Some(endpoint) = &peer.endpoint {
            assert!(!endpoint.host.is_empty() && endpoint.port != 0);
            let _ = endpoint.socket_addr();
        }
    }
    if let Ok(serve) = crate::networks::wg_serve::check(&conf) {
        assert_eq!(serve.peers(), conf.peers.len());
        crate::networks::wg_serve::fuzz_packet(&serve, packet);
    }
}

/// A network spec, as the router file and the control socket of the router
/// give it: one network or a chain (`TOP over BELOW`). The parse and the
/// label never panic, an error is one line, and a chain has at most
/// [`crate::networks::chain::MAX_LAYERS`] layers.
pub fn network_spec(data: &[u8]) {
    let Some(spec) = text(data) else { return };
    if spec.contains("wg:") {
        return;
    }
    match crate::networks::parse(spec) {
        Ok(network) => {
            let label = network.label();
            if network.is_chain() {
                let layers = label.matches(" over ").count() + 1;
                assert!(layers <= crate::networks::chain::MAX_LAYERS, "{label}");
            }
            let _ = crate::network::Network::checks(&network);
        }
        Err(e) => assert!(!e.text().contains('\n') || spec.contains('\n'), "{e}"),
    }
}

/// The runtime of the targets that build a router: a part of the build can
/// need a runtime.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    })
}

/// The router file (`podnet route --config FILE`): the parse, and the plan
/// of `route --check` (each network spec, each group, each door and each
/// guest, with no network up and no dial).
pub fn router_config(data: &[u8]) {
    let Some(text) = text(data) else { return };
    if text.contains("wg:") {
        return;
    }
    let Ok(config) = crate::router::parse(text) else {
        return;
    };
    let _guard = runtime().enter();
    let _ = crate::router::Router::plan(&config);
}

/// A message of the other end of a transfer (sealed in a phase of the
/// wormhole), and the ack record. A message that parses goes back to the
/// same message through its encoding. The name of an offer is one
/// component when the receiver takes it.
pub fn transfer_message(data: &[u8]) {
    let _ = messages::read_ack(data);
    let Ok(message) = messages::parse(data) else {
        return;
    };
    match &message {
        Message::Transit(transit) => {
            assert_eq!(messages::parse(&transit.encode()).as_ref(), Ok(&message));
            let ours = [Hint::Piping {
                url: "https://piping.example".into(),
            }];
            for role in [Role::Sender, Role::Receiver] {
                if let Some(url) = piping::common(role, &ours, &transit.hints) {
                    assert_eq!(url, "https://piping.example");
                }
            }
            for hint in &transit.hints {
                if let Hint::PodnetRelay { url, .. } = hint {
                    let _ = crate::ws_relay::Relay::single(url, None);
                }
            }
        }
        Message::Offer(offer) => {
            assert_eq!(messages::parse(&offer.encode()).as_ref(), Ok(&message));
            let name = match offer {
                Offer::File { name, .. } | Offer::Directory { name, .. } => name,
                Offer::Message(_) => return,
            };
            if let Ok(top) = land::top(name) {
                assert!(!top.is_empty() && !top.contains('/') && top != "." && top != "..");
            }
        }
        _ => {}
    }
}

/// A file in memory with the bytes of `data`.
fn memory_file(data: &[u8]) -> std::fs::File {
    use std::os::fd::FromRawFd as _;
    // SAFETY: the name is a C string; the fd is new and owned by the file.
    let fd = unsafe { libc::memfd_create(c"podnet-fuzz".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0, "memfd_create: {}", std::io::Error::last_os_error());
    // SAFETY: memfd_create gave a new fd that nothing else holds.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(data).expect("a write to a memfd");
    file
}

/// A zip of a directory that a sender gives (`podnet receive`): the
/// directory of the zip, the rules of the names (each entry is checked
/// before a write), and the data of each entry, under small caps.
pub fn transfer_zip(data: &[u8]) {
    let mut file = memory_file(data);
    let Ok(entries) = zip::entries(&mut file) else {
        return;
    };
    let caps = land::Caps {
        max_files: 64,
        max_bytes: 1 << 20,
    };
    let Ok(plan) = land::plan(&entries, &caps) else {
        return;
    };
    assert_eq!(plan.len(), entries.len());
    for parts in &plan {
        assert!(!parts.is_empty());
        for part in parts {
            assert!(!part.is_empty() && !part.contains('/') && part != "." && part != "..");
        }
    }
    for entry in entries.iter().filter(|e| !e.is_dir) {
        if let Ok(n) = zip::copy(&mut file, entry, &mut std::io::sink()) {
            assert_eq!(n, entry.size);
        }
    }
}

/// The key of the records of the target `transit_record`.
const TRANSIT_KEY: [u8; 32] = [0x11; 32];

/// The bytes of a sender on a transit connection, as a receiver reads
/// them: the handshake of the sender and `go`, then the records. A record
/// that opens is not longer than the input.
pub fn transit_record(data: &[u8]) {
    let mut reader = data;
    futures::executor::block_on(async {
        let shake = transit::handshake(&TRANSIT_KEY, Role::Sender);
        let shaken = transit::expect(&mut reader, &shake).await.unwrap_or(false)
            && transit::expect(&mut reader, b"go\n").await.unwrap_or(false);
        if !shaken {
            reader = data;
        }
        let io = tokio::io::join(reader, tokio::io::sink());
        let mut records = Records::new(io, &TRANSIT_KEY, Role::Receiver);
        for _ in 0..64 {
            match records.receive().await {
                Ok(record) => assert!(record.len() <= data.len()),
                Err(_) => break,
            }
        }
    });
}

/// A signed route announcement of the link mesh, as a peer sends it on the
/// stream `announce` ([`crate::mesh_announce`]).
pub fn mesh_announce(data: &[u8]) {
    crate::mesh_announce::fuzz(data);
}

/// The route table of the link mesh under a sequence of steps that the
/// bytes give, 8 bytes for each step. A peer signs what it announces, so
/// the content of an announcement is untrusted. After each step: the table
/// holds at most its origins, an origin has at most its routes, and each
/// usable route runs from its origin to its next hop, holds no node two
/// times and not this node, and has at most its hops.
pub fn mesh_table(data: &[u8]) {
    let key = |b: u8| PublicKey([b % 12; 32]);
    let me = key(0);
    let limits = Limits {
        origins: 6,
        max_hops: 4,
        expiry_ms: 100,
        routes_per_origin: 3,
    };
    let mut table = RouteTable::new(me, limits);
    let mut now = 0u64;
    for step in data.chunks(8) {
        let b = |i: usize| step.get(i).copied().unwrap_or(0);
        now += u64::from(b(0) % 64);
        match b(1) % 8 {
            0..=4 => {
                let length = 1 + usize::from(b(2) % 5);
                let path: Vec<PublicKey> = (0..length).map(|i| key(b(3 + i % 5) >> i)).collect();
                let from = if b(1) % 8 == 4 {
                    key(b(7))
                } else {
                    *path.last().expect("a path")
                };
                let ann = Announcement {
                    origin: path[0],
                    seq: u64::from(b(2) >> 3),
                    cost: u32::from(b(6)) * 1000,
                    path,
                    expires_ms: now + u64::from(b(7) % 200),
                };
                let _ = table.learn(from, u32::from(b(5)), &ann, now);
            }
            5 => {
                let _ = table.session_closed(&key(b(2)), now);
            }
            6 => {
                let _ = table.purge(now);
            }
            _ => {}
        }
        assert!(table.len() <= limits.origins);
        let origins: Vec<PublicKey> = table.origins().copied().collect();
        for origin in &origins {
            assert_ne!(*origin, me);
            let ranked = table.ranked(origin, now);
            assert!(ranked.len() <= limits.routes_per_origin);
            assert_eq!(table.best(origin, now), ranked.first().copied());
            for route in ranked {
                assert_eq!(route.path.first(), Some(origin));
                assert_eq!(route.path.last(), Some(&route.next_hop));
                assert!(route.path.len() <= limits.max_hops && !route.path.contains(&me));
                for (i, node) in route.path.iter().enumerate() {
                    assert!(!route.path[..i].contains(node));
                }
            }
        }
    }
}

/// The seed inputs of the targets of this module.
pub fn seeds(target: &str) -> Vec<Vec<u8>> {
    let s = |items: &[&str]| items.iter().map(|i| i.as_bytes().to_vec()).collect();
    match target {
        "wg_quick" => {
            let mut packet = vec![0x45, 0, 0, 20, 0, 0, 0, 0, 64, 17, 0, 0];
            packet.extend_from_slice(&[10, 9, 0, 2, 10, 9, 0, 3]);
            let mut serve = format!(
                "[Interface]\nPrivateKey = {K}\nAddress = 10.9.0.1/24\nListenPort = 51820\n\
                 [Peer]\nPublicKey = {K}\nAllowedIPs = 10.9.0.2/32\n\
                 [Peer]\nPublicKey = {P}\nPresharedKey = {K}\nAllowedIPs = 10.9.0.3/32\n",
                K = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=",
                P = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
            )
            .into_bytes();
            serve.push(0);
            serve.extend_from_slice(&packet);
            vec![
                serve,
                format!(
                    "[Interface]\nPrivateKey = {K}\nAddress = 172.16.0.2/32, fd00::2\nDNS = 1.1.1.1, corp.example\nMTU = 1280\n\
                     [Peer]\nPublicKey = {K}\nEndpoint = [2001:db8::1]:2408\nAllowedIPs = 0.0.0.0/0, ::/0\n\
                     PersistentKeepalive = 25\nReserved = 1, 2, 3\n",
                    K = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
                )
                .into_bytes(),
            ]
        }
        "network_spec" => s(&[
            "socks5h://127.0.0.1:1080 over http://proxy:3128",
            "warp over direct",
            "turn://user:pass@turn.example:3478 over relay:relay.example",
            "netbird:51820",
            "peer:0123456789abcdef/web over socks5://u:p@[::1]:1080",
            "devtunnel:abc",
        ]),
        "router_config" => s(&[
            "networks:\n  home: direct\n  far: socks5h://127.0.0.1:1080 over direct\ngroups:\n  out: home=2,far\n\
             policies:\n  out: minrtt\ndoors:\n  - listen: 127.0.0.1:1080\n    via: out\n\
             health:\n  target: example.com:443\ncontrol: unix:/tmp/route.sock\n",
            "kernel:\n  wans:\n    - name: a\n      dev: tun0\n    - name: b\n      dev: tun1\n      weight: 3\n  priority: 1000\n",
            "networks:\n  home: direct\nguests:\n  - wire: unix:/tmp/g.sock\n    via: home\n    net: 10.7.0.0/24\n",
        ]),
        "transfer_message" => {
            let transit = messages::Transit {
                abilities: vec!["direct-tcp-v1".into(), "relay-v1".into()],
                hints: vec![
                    Hint::Direct {
                        host: "192.0.2.1".into(),
                        port: 4001,
                    },
                    Hint::Relay {
                        name: Some("r".into()),
                        ends: vec![
                            messages::RelayEnd::Tcp {
                                host: "relay.example".into(),
                                port: 4001,
                            },
                            messages::RelayEnd::WebSocket {
                                url: "wss://relay.example/".into(),
                            },
                        ],
                    },
                    Hint::PodnetRelay {
                        url: "https://podnet-relay.example".into(),
                        name: "pair".into(),
                    },
                    Hint::Piping {
                        url: "https://piping.example".into(),
                    },
                ],
                relay_token: Some("t".into()),
            };
            let offer = Offer::Directory {
                mode: messages::ZIP_MODE.into(),
                name: "dir".into(),
                zipsize: 10,
                numbytes: 20,
                numfiles: 2,
            };
            vec![
                transit.encode(),
                offer.encode(),
                Offer::File {
                    name: "a.txt".into(),
                    size: 3,
                }
                .encode(),
                Offer::Message("hi".into()).encode(),
                messages::answer(&messages::Answer::FileAck),
                messages::error("no"),
                messages::ack(&[0xab; 32]),
            ]
        }
        "transfer_zip" => {
            let dir = std::env::temp_dir().join(format!("podnet-fuzz-seed-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("sub")).expect("a seed directory");
            std::fs::write(dir.join("a.txt"), b"hello").expect("a seed file");
            std::fs::write(dir.join("sub/b.txt"), b"world world world").expect("a seed file");
            let mut out = memory_file(&[]);
            let zipped = zip::write(&dir, &mut out).is_ok();
            let _ = std::fs::remove_dir_all(&dir);
            let mut bytes = Vec::new();
            if zipped {
                use std::io::{Read as _, Seek as _};
                let _ = out.rewind();
                let _ = out.read_to_end(&mut bytes);
            }
            vec![bytes]
        }
        "transit_record" => futures::executor::block_on(async {
            let mut out = transit::handshake(&TRANSIT_KEY, Role::Sender);
            out.extend_from_slice(b"go\n");
            let io = tokio::io::join(&[][..], Vec::new());
            let mut records = Records::new(io, &TRANSIT_KEY, Role::Sender);
            for record in [&b"one"[..], b"", &[7u8; 300]] {
                records.send(record).await.expect("a record to memory");
            }
            let (_, written) = records.into_inner().into_inner();
            out.extend_from_slice(&written);
            vec![out, written]
        }),
        "mesh_announce" => {
            let origin = pod_identity::Identity::from_seed(&[9u8; 32]);
            let body = crate::mesh_announce::Body {
                origin: origin.public(),
                seq: 1,
                issued_ms: 1_000_000,
                expires_ms: 1_030_000,
                neighbours: vec![crate::mesh_announce::Neighbour {
                    key: PublicKey([4u8; 32]),
                    cost: 10,
                }],
                services: vec!["files".into(), "git".into()],
            };
            let wire = crate::mesh_announce::encode(&origin, &body).expect("a seed");
            let hop = pod_identity::Identity::from_seed(&[8u8; 32]);
            let forwarded = crate::mesh_announce::append_hop(&wire, &hop, 5).expect("a seed");
            vec![wire, forwarded]
        }
        "mesh_table" => vec![
            vec![5, 0, 2, 1, 2, 3, 4, 9, 7, 1, 3, 1, 2, 5, 1, 9],
            vec![
                1, 0, 0, 3, 0, 0, 0, 50, 1, 1, 9, 3, 4, 0, 2, 60, 9, 5, 4, 0, 0, 0, 0, 0, 9, 6, 0,
                0, 0, 0, 0, 0,
            ],
        ],
        _ => Vec::new(),
    }
}
