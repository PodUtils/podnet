//! `podnet man`, `--help --json` and the llms files, on the crate
//! `pod-manual` of PodUtils/podcrates. podnet keeps its facts here
//! ([`facts`], [`inventory`]) and fills the model of the crate from the clap
//! command tree of the binary ([`manual`]). The crate writes the text, the
//! man(7) page, the JSON, the help as JSON and the llms files; it holds the
//! pager and the checks that the tests of the binary call.
//!
//! * `podnet man` pages the text on a terminal, else writes it to stdout.
//! * `--no-pager` writes it to stdout, also on a terminal.
//! * `--roff` writes a man(7) page, for `man -l -` or a man directory.
//! * `--json` writes the manual as one JSON object, for a program.
//! * `podnet man SECTION` writes one section: a command (`tunnel`, `warp run`)
//!   or a topic (`environment`, `files`, `inventory`, `exit-status`,
//!   `examples`, `see-also`).

pub mod facts;
pub mod inventory;
mod pod_flags;

use std::path::Path;

pub use pod_manual::{Manual, Request};

/// The paragraphs of DESCRIPTION.
const DESCRIPTION: &[&str] = &[
    "podnet is one static binary that brings its own network to hosts with strong network limits: no root, no TUN device, no network namespace, no UDP, no AF_INET bind, or only TCP 443 out.",
    "To publish a local service: Cloudflare quick and named tunnels (tunnel, expose), SSH forward relays (ssh-relay), the file server (serve), file transfer (share) and git hosting (git serve). To reach out: the Cloudflare WARP client with no root (warp, agent), the relay (relay, connect), forwards (forward) and a SOCKS5 exit on stdin and stdout (exit). The router has networks, groups, policies and a kernel mode (route). To join two installs: the link (link) over a carrier, with services at a peer and a mesh. To contain programs: pods (pod) and virtual networks (net). To check and keep it running: doctor, bench, status, components, the background runs (--detach, ps, logs, stop, autostart) and update.",
    "A tunnel reaches the edge with one of four legs. They are QUIC on UDP 7844, HTTP/2 on TCP 7844 (directly or through HTTPS_PROXY), and QUIC inside a WARP MASQUE tunnel on TCP 443. The fourth is HTTP/2 through the WebSocket relay on TCP 443 (--transport relay). With --protocol auto and --transport auto, podnet tries the first three in that order.",
    "podnet pod runs a program with all its traffic sent through a proxy, with no LD_PRELOAD (podnet-netns). podnet net gives programs virtual IP addresses on a host that refuses AF_INET binds.",
    "This manual is generated from the command tree of the binary. podnet man --roff writes it as a man(7) page, and podnet man --json writes its tables.",
];

/// The lines of SEE ALSO.
const SEE_ALSO: &[&str] = &[
    "podnet pod --help; cloudflared(1).",
    "https://github.com/PodUtils/podnet: README.md, the topic documents in docs/, docs/protocol.md and docs/virtual-network.md.",
];

/// The manual of the command tree `cmd`, the parser of the binary: the
/// commands from the reader of clap of `pod-manual`, then the facts, the
/// description, the examples and the usage of `podnet pod`.
pub fn manual(cmd: clap::Command) -> Manual {
    let mut m = pod_manual::clap::read(cmd);
    for c in &mut m.commands {
        // The flags of clap for its own help and version: the manual does
        // not show them. `podnet update --version VERSION` is a flag of
        // podnet: it takes a value, and it stays.
        c.flags.retain(|f| !is_clap_flag(f));
    }
    m.date = env!("PODNET_BUILD_DATE").to_string();
    m.description = DESCRIPTION.iter().map(|p| p.to_string()).collect();
    m.facts = facts::facts();
    m.examples = facts::examples();
    m.see_also = SEE_ALSO.iter().map(|l| l.to_string()).collect();
    // `podnet pod` has its own parser (podnet-netns): its usage is a note of
    // the command, one note for each paragraph.
    if let Some(pod) = m.commands.iter_mut().find(|c| c.path == ["pod"]) {
        let usage = podnet_netns::usage_for("podnet pod");
        // T-455: the flags of pod are those of its help, not those of the
        // stub of clap (the global flags of podnet do not apply to pod).
        pod.flags = pod_flags::flags(&usage);
        // The rules of clap are those of the global flags (--quiet excludes
        // --verbose), which pod does not have.
        pod.rules.clear();
        pod.notes.extend(
            usage
                .split("\n\n")
                .map(|p| p.trim_end().to_string())
                .filter(|p| !p.trim().is_empty()),
        );
    }
    m
}

/// Whether `f` is the flag that clap adds for its help (`-h, --help`) or
/// its version (`-V, --version`).
pub fn is_clap_flag(f: &pod_manual::Flag) -> bool {
    f.value.is_none()
        && matches!(
            (f.short, f.long.as_deref()),
            (Some('h'), Some("help")) | (Some('V'), Some("version")) | (None, Some("help"))
        )
}

/// The base of the links of `llms.txt`.
pub const DOCS_BASE: &str = "https://github.com/PodUtils/podnet/blob/main/";

/// The documents of the llms files: the path and what the document gives.
pub const DOCS: &[(&str, &str)] = &[
    (
        "README.md",
        "What podnet is, install, quick start, and the map of the capabilities.",
    ),
    (
        "docs/install.md",
        "Install, verify, update and rebuild a release.",
    ),
    (
        "docs/tunnels.md",
        "Quick and named tunnels, the configuration file, the edge protocol, raw streams.",
    ),
    (
        "docs/serve.md",
        "The file server, file transfer with a PIN, git hosting.",
    ),
    (
        "docs/transfer.md",
        "podnet send and podnet receive: a transfer by code, as magic-wormhole does.",
    ),
    (
        "docs/warp.md",
        "The WARP client: proxy, run, up, exec, nc, tun, transports.",
    ),
    (
        "docs/pods.md",
        "podnet pod: a program with all its traffic through a proxy chain.",
    ),
    (
        "docs/networks.md",
        "podnet net, Tailscale, NetBird and the passwd shim.",
    ),
    (
        "docs/router.md",
        "podnet route: networks, groups, policies, health, doors and the kernel mode.",
    ),
    (
        "docs/forward.md",
        "podnet forward, the DNS door and podnet expose.",
    ),
    ("docs/relay.md", "podnet relay and podnet ssh-relay."),
    ("docs/egress.md", "Network specs and --egress."),
    ("docs/proxies.md", "Proxies, DNS and TLS roots."),
    (
        "docs/background.md",
        "--detach, ps, logs, wait, stop and autostart.",
    ),
    (
        "docs/doctor.md",
        "doctor, pod doctor, net doctor, status, bench, bug-report and agent.",
    ),
    (
        "docs/cli.md",
        "The manual, JSON help, the llms files, global flags and exit codes.",
    ),
    ("docs/library.md", "The Rust library."),
    ("docs/build.md", "Build from source, fuzz targets."),
    (
        "docs/protocol.md",
        "The edge protocol and the relay protocol.",
    ),
    (
        "docs/virtual-network.md",
        "The virtual network: design and use.",
    ),
    (
        "docs/target-environment.md",
        "The hosts that podnet is for, and their limits.",
    ),
];

/// `llms.txt` and `llms-full.txt` of the repository at `root`: the manual
/// with the version and no commit (the commit changes at each build), and
/// the text of each document of [`DOCS`]. A document that cannot be read
/// has no text, and the check of the files fails.
pub fn llms(manual: &Manual, root: &Path) -> pod_manual::llms::Files {
    let mut m = manual.clone();
    m.version = env!("CARGO_PKG_VERSION").to_string();
    let docs: Vec<pod_manual::llms::Doc> = DOCS
        .iter()
        .map(|(path, meaning)| pod_manual::llms::Doc {
            path: path.to_string(),
            meaning: meaning.to_string(),
            body: std::fs::read_to_string(root.join(path)).unwrap_or_default(),
        })
        .collect();
    pod_manual::llms::Files {
        index: pod_manual::llms::index(&m, &docs, DOCS_BASE),
        full: pod_manual::llms::full(&m, &docs),
    }
}
