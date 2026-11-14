//! `podnet send` and `podnet receive` (T-105): the file transfer of
//! magic-wormhole over `podnet::transfer`. The sender prints the code; the
//! receiver reads it from the terminal or from stdin, never from the
//! command line, where each user of the host sees it.

use std::io::{BufRead, IsTerminal, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Args;
use serde_json::json;

use podnet::exit::{self, Fault};
use podnet::transfer::land::{Caps, MAX_BYTES, MAX_FILES};
use podnet::transfer::{self, Listen, Offered, Receive, Received, Relay, Settings, What};

/// The flags of the meeting and of the transit, of both commands.
#[derive(Args, Debug)]
pub(crate) struct WayArgs {
    /// A mailbox of magic-wormhole (wss:// or ws://), in place of the
    /// mailboxes of pod-services. Repeat it for a list, in order: the
    /// sender takes the first that answers, the receiver looks for the code
    /// at each one. PODNET_MAILBOX gives the same list, separated by commas.
    #[arg(
        long,
        value_name = "URL",
        env = "PODNET_MAILBOX",
        hide_env_values = true,
        value_delimiter = ','
    )]
    mailbox: Vec<String>,
    /// A transit relay of this end: tcp:HOST:PORT, ws://... or wss://...
    /// Repeat it for more. The default is each transit relay of
    /// pod-services; none gives no relay of this end.
    #[arg(long, value_name = "URL")]
    transit_relay: Vec<String>,
    /// A piping server (https://HOST[:PORT][/PATH]) for the last way of
    /// the transit between two podnet ends: the sealed records go through
    /// it in pieces. Repeat it for a list, in order: the two ends use the
    /// first server of the sender that the receiver names too. The default
    /// is each piping server of pod-services; none gives no piping server.
    /// PODNET_PIPING gives the same list, separated by commas.
    #[arg(
        long,
        value_name = "URL",
        env = "PODNET_PIPING",
        hide_env_values = true,
        value_delimiter = ','
    )]
    piping: Vec<String>,
    /// Listen for a direct connection on this address only (the hint is
    /// this address, also a loopback address). The default is each address
    /// of the host, on a free port.
    #[arg(long, value_name = "ADDR", conflicts_with = "no_listen")]
    listen: Option<SocketAddr>,
    /// Do not listen: the other end must listen, or a relay must carry the
    /// transfer.
    #[arg(long)]
    no_listen: bool,
    /// Print one JSON object for each event on stdout, in place of the text.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct SendArgs {
    /// The file or the directory to send. A directory goes as a zip.
    #[arg(
        value_name = "PATH",
        required_unless_present = "text",
        conflicts_with = "text"
    )]
    path: Option<PathBuf>,
    /// Send this text in place of a file.
    #[arg(long, value_name = "TEXT")]
    text: Option<String>,
    /// Offer the node of the stored pair NAME of the podnet relay (podnet
    /// relay pair) as the last way of the transit, to a podnet receiver.
    /// The node runs for the time of the transfer; do not run another node
    /// of the pair at the same time. A podnet extension: a client of
    /// magic-wormhole ignores it.
    #[arg(long, value_name = "NAME", conflicts_with = "text")]
    relay_pair: Option<String>,
    /// The count of words of the code after its number, 2 to 8 (as
    /// magic-wormhole's --code-length). Each word adds 8 bits: 2 words give
    /// a guesser one chance in 65536.
    #[arg(long, value_name = "N", default_value_t = pod_wormhole::WORDS as u8,
          value_parser = clap::value_parser!(u8).range(pod_wormhole::WORDS_LEAST as i64..=pod_wormhole::WORDS_MOST as i64))]
    words: u8,
    /// Seconds to wait for a receiver to take the code, and for its answer.
    #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..=86_400))]
    timeout: u64,
    #[command(flatten)]
    way: WayArgs,
}

#[derive(Args, Debug)]
pub(crate) struct ReceiveArgs {
    /// The directory where the file or the directory lands. It must exist.
    #[arg(long, value_name = "DIR", default_value = ".")]
    out: PathBuf,
    /// Accept the offer with no question. Necessary when stdin is not a
    /// terminal and the sender offers a file or a directory.
    #[arg(long)]
    yes: bool,
    /// The most entries of a directory. More is refused (exit 65).
    #[arg(long, value_name = "N", default_value_t = MAX_FILES)]
    max_files: u64,
    /// The most bytes of a file or of the files of a directory. More is
    /// refused (exit 65).
    #[arg(long, value_name = "N", default_value_t = MAX_BYTES)]
    max_bytes: u64,
    #[command(flatten)]
    way: WayArgs,
}

fn usage(why: impl std::fmt::Display) -> anyhow::Error {
    exit::fault(Fault::Usage, anyhow::anyhow!("{why}"))
}

fn settings(way: &WayArgs) -> Result<Settings> {
    let mailboxes = transfer::mailboxes(&way.mailbox, None).map_err(usage)?;
    let relays = match way.transit_relay.as_slice() {
        [] => transfer::default_relays(),
        [none] if none == "none" => Vec::new(),
        given => given
            .iter()
            .map(|r| Relay::parse(r))
            .collect::<Result<_, _>>()
            .map_err(usage)?,
    };
    let mut settings = Settings::new(mailboxes, relays, podnet::ws::way());
    settings.piping = transfer::piping(&way.piping).map_err(usage)?;
    settings.listen = match (way.listen, way.no_listen) {
        (Some(at), _) => Listen::At(at),
        (None, true) => Listen::No,
        (None, false) => Listen::Any,
    };
    Ok(settings)
}

/// One event: a JSON line on stdout, or a line of text on stderr.
fn say(json: bool, event: serde_json::Value, text: &str) {
    if json {
        println!("{event}");
        let _ = std::io::stdout().flush();
    } else {
        eprintln!("{text}");
    }
}

pub(crate) async fn send(args: SendArgs) -> Result<()> {
    let mut settings = settings(&args.way)?;
    settings.wait = Duration::from_secs(args.timeout);
    settings.ends.config.words = usize::from(args.words);
    if let Some(name) = &args.relay_pair {
        settings.relay_pair = Some(transfer::RelayPair {
            dir: podnet::relay_reverse::pairs_dir()?,
            name: name.clone(),
        });
    }
    let what = match (&args.text, &args.path) {
        (Some(text), _) => What::Text(text.clone()),
        (None, Some(path)) => What::Path(path.clone()),
        (None, None) => return Err(usage("give a PATH or --text TEXT")),
    };
    if let What::Path(path) = &what {
        std::fs::metadata(path)
            .map_err(|e| exit::fault(Fault::NoInput, anyhow::anyhow!("{}: {e}", path.display())))?;
    }
    let opened = transfer::open(&settings).await?;
    let code = opened.code().to_string();
    say(
        args.way.json,
        json!({ "event": "code", "code": code, "mailbox": opened.mailbox() }),
        &format!("Wormhole code is: {code}\nOn the other computer, run: podnet receive (and type the code)"),
    );
    let sent = opened.send(&settings, &what).await?;
    let text = match &sent.sha256 {
        Some(sha) => format!("Sent {} bytes ({}), SHA-256 {sha}", sent.bytes, sent.via),
        None => "Sent the text.".to_string(),
    };
    say(
        args.way.json,
        json!({ "event": "sent", "bytes": sent.bytes, "sha256": sent.sha256, "via": sent.via }),
        &text,
    );
    Ok(())
}

/// The code, from the terminal or from one line of stdin.
fn read_code() -> Result<pod_wormhole::Code> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprint!("Enter the code: ");
        let _ = std::io::stderr().flush();
    }
    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .map_err(|e| usage(format!("no code on stdin: {e}")))?;
    pod_wormhole::Code::parse(line.trim()).map_err(usage)
}

/// The question of an offer: `--yes`, else a line of the terminal.
fn ask(yes: bool) -> impl FnMut(&Offered) -> Result<bool> + Send {
    move |offered: &Offered| {
        if yes {
            return Ok(true);
        }
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            return Err(usage(
                "stdin is not a terminal: give --yes to accept a file or a directory",
            ));
        }
        eprint!(
            "Receive the {} {:?}: {} bytes in {} entries? [y/N] ",
            offered.kind, offered.name, offered.bytes, offered.files
        );
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
    }
}

pub(crate) async fn receive(args: ReceiveArgs) -> Result<()> {
    let settings = settings(&args.way)?;
    if !args.out.is_dir() {
        let why = anyhow::anyhow!("{}: not a directory (--out)", args.out.display());
        return Err(exit::fault(Fault::NoInput, why));
    }
    let code = read_code()?;
    let how = Receive {
        out: args.out.clone(),
        caps: Caps {
            max_files: args.max_files,
            max_bytes: args.max_bytes,
        },
    };
    let mut ask = ask(args.yes);
    match transfer::receive(&settings, &code, &how, &mut ask).await? {
        Received::Text(text) => {
            if args.way.json {
                println!("{}", json!({ "event": "received", "text": text }));
            } else if std::io::stdout().is_terminal() {
                // A control sequence of the sender does not reach the
                // terminal; a pipe gets the text as it came (T-347).
                println!("{}", transfer::for_terminal(&text, true));
            } else {
                println!("{text}");
            }
        }
        Received::Path {
            path,
            bytes,
            sha256,
            files,
            via,
        } => say(
            args.way.json,
            json!({ "event": "received", "path": path, "bytes": bytes, "sha256": sha256, "files": files, "via": via }),
            &format!(
                "Received {} ({bytes} bytes, {files} entries, {via}), SHA-256 {sha256}",
                transfer::for_terminal(&path.display().to_string(), false)
            ),
        ),
    }
    Ok(())
}
