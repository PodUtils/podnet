//! `podnet tunnel --inspect` (the start of the inspector) and `podnet
//! tunnel inspect list|show|replay|clear` (its client) (T-244).

use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use podnet::config::Config;
use podnet::exit::Fault;
use podnet::tunnel::inspect::{self, Inspector, Options, Served};

/// The commands of `podnet tunnel` (with no subcommand, `tunnel` starts a
/// tunnel).
#[derive(Subcommand, Debug)]
pub(crate) enum TunnelAction {
    /// Read the exchanges that a tunnel with --inspect keeps in memory,
    /// over its unix socket (mode 0600, the user of the tunnel only).
    #[command(subcommand_required = true, arg_required_else_help = false)]
    Inspect {
        #[command(subcommand)]
        command: InspectCommand,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum InspectCommand {
    /// List the exchanges in the ring, the oldest first: the id, the
    /// method, the target, the status and the state.
    List(InspectTarget),
    /// Show one exchange: the heads and the bodies (each secret value is
    /// [redacted]; a body keeps the cap of tunnel --inspect-body).
    Show {
        /// The id of the exchange (from list).
        id: u64,
        #[command(flatten)]
        target: InspectTarget,
    },
    /// Send the request of one exchange to its origin again. The ring gets
    /// a new exchange that names the first. A redacted header or query
    /// parameter is left out, and the output says so. An open or
    /// incomplete exchange, a cut request body and a WebSocket are refused
    /// (exit 65).
    Replay {
        /// The id of the exchange (from list).
        id: u64,
        #[command(flatten)]
        target: InspectTarget,
    },
    /// Remove each exchange from the ring. The ids go on from the last
    /// one. An exchange that is open stays out of the ring; its copy goes
    /// on.
    Clear(InspectTarget),
}

#[derive(Args, Debug)]
pub(crate) struct InspectTarget {
    /// The instance name of the tunnel (--name, or the name of --detach;
    /// a tunnel with no name is tunnel-HEX, in its log). Default: the one tunnel with
    /// --inspect that runs.
    #[arg(long, value_name = "INSTANCE")]
    name: Option<String>,
}

/// The flags of `tunnel --inspect`.
pub(crate) struct Flags {
    pub(crate) keep: Option<usize>,
    pub(crate) body: Option<usize>,
    pub(crate) raw: bool,
}

/// Start the inspector of `config`: the sink of the next tunnel of this
/// process and its socket. Keep the value until the tunnel ends: its drop
/// removes the socket.
pub(crate) fn start(flags: &Flags, config: &Config) -> Result<Served> {
    let options = Options {
        keep: flags.keep.unwrap_or(inspect::DEFAULT_KEEP),
        body: flags.body.unwrap_or(inspect::DEFAULT_BODY),
        raw: flags.raw,
    };
    if options.raw {
        tracing::warn!(
            "--inspect-raw: the inspector keeps the value of each Authorization, Cookie and token in memory"
        );
    }
    let http = config.ingress.rules.iter().any(|rule| {
        matches!(
            rule.service,
            podnet::Service::Http(_) | podnet::Service::Unix(_) | podnet::Service::UnixTls(_)
        )
    });
    if !http {
        tracing::warn!("--inspect: the tunnel has no HTTP origin; the inspector gets no exchange");
    }
    let inspector = Inspector::new(options, config.ingress.clone());
    inspect::install(inspector.clone());
    let path = inspect::socket_path()?;
    let served = inspect::serve(inspector, &path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(inspect::SUFFIX))
        .unwrap_or_default();
    tracing::info!(
        "inspector: unix:{} (podnet tunnel inspect list --name {name})",
        path.display()
    );
    Ok(served)
}

/// `podnet tunnel inspect ...`.
pub(crate) async fn run(command: &InspectCommand, json: bool) -> Result<()> {
    let (request, target) = match command {
        InspectCommand::List(target) => (json!({"verb": "list"}), target),
        InspectCommand::Show { id, target } => (json!({"verb": "show", "id": id}), target),
        InspectCommand::Replay { id, target } => (json!({"verb": "replay", "id": id}), target),
        InspectCommand::Clear(target) => (json!({"verb": "clear"}), target),
    };
    let path = inspect::find_socket(target.name.as_deref())?;
    let mut answer = inspect::client::ask(&path, &request).await?;
    if let Some(map) = answer.as_object_mut() {
        map.remove("ok");
    }
    if json {
        println!("{}", podnet::status::stamped(answer));
        return Ok(());
    }
    match command {
        InspectCommand::List(_) => print!("{}", list_text(&answer)),
        InspectCommand::Show { .. } => print!("{}", show_text(&answer["exchange"])),
        InspectCommand::Clear(_) => println!(
            "cleared {} exchanges from the ring",
            number(&answer["cleared"])
        ),
        InspectCommand::Replay { .. } => {
            print!("{}", replay_text(&answer));
            if let Some(error) = answer["error"].as_str() {
                return Err(podnet::exit::fault(
                    Fault::Unavailable,
                    anyhow::anyhow!("replay: {error}"),
                ));
            }
        }
    }
    Ok(())
}

/// The UTC time of the day of a time in milliseconds from the epoch.
fn clock(ms: u64) -> String {
    let secs = (ms / 1000) % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

fn number(value: &Value) -> String {
    value
        .as_u64()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".into())
}

/// The text of `list`: one line for each exchange.
pub(crate) fn list_text(doc: &Value) -> String {
    let empty = Vec::new();
    let rows = doc["exchanges"].as_array().unwrap_or(&empty);
    if rows.is_empty() {
        return format!(
            "no exchange yet (the ring keeps the last {})\n",
            number(&doc["keep"])
        );
    }
    let mut out = String::from(
        "ID     TIME      METHOD  STATUS  MS      IN/OUT BYTES       STATE       TARGET\n",
    );
    for row in rows {
        let mut note = String::new();
        if let Some(from) = row["replayed_from"].as_u64() {
            note.push_str(&format!(" (replay of {from})"));
        }
        if row["cut"].as_u64().unwrap_or(0) > 0 {
            note.push_str(" (body cut)");
        }
        out.push_str(&format!(
            "{:<6} {}  {:<7} {:<7} {:<7} {:<18} {:<11} {}{note}\n",
            number(&row["id"]),
            clock(row["started_ms"].as_u64().unwrap_or(0)),
            row["method"].as_str().unwrap_or("-"),
            number(&row["status"]),
            number(&row["elapsed_ms"]),
            format!(
                "{}/{}",
                number(&row["request_bytes"]),
                number(&row["response_bytes"])
            ),
            row["state"].as_str().unwrap_or("-"),
            row["target"].as_str().unwrap_or("-"),
        ));
    }
    out
}

/// The lines of one side of an exchange: the heads with `mark`, and the
/// body.
fn side(out: &mut String, mark: &str, first: &str, part: &Value) {
    out.push_str(&format!("{mark} {first}\n"));
    for pair in part["headers"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{mark} {}: {}\n",
            pair[0].as_str().unwrap_or_default(),
            pair[1].as_str().unwrap_or_default()
        ));
    }
    out.push_str(&format!("{mark}\n"));
    match (part["body"].as_str(), part["body_base64"].as_str()) {
        (Some(text), _) if !text.is_empty() => {
            out.push_str(text);
            if !text.ends_with('\n') {
                out.push('\n');
            }
        }
        (_, Some(_)) => out.push_str(&format!(
            "[{} bytes that are not UTF-8: --json gives them in base64]\n",
            number(&part["bytes"])
        )),
        _ => {}
    }
    let cut = part["cut"].as_u64().unwrap_or(0);
    if cut > 0 {
        out.push_str(&format!(
            "[the cap of --inspect-body cut {cut} of {} bytes]\n",
            number(&part["bytes"])
        ));
    }
}

/// The text of `show`.
pub(crate) fn show_text(exchange: &Value) -> String {
    let mut out = format!(
        "exchange {}: {}, status {}, {} ms, started {} UTC\n",
        number(&exchange["id"]),
        exchange["state"].as_str().unwrap_or("-"),
        number(&exchange["status"]),
        number(&exchange["elapsed_ms"]),
        clock(exchange["started_ms"].as_u64().unwrap_or(0)),
    );
    if let Some(from) = exchange["replayed_from"].as_u64() {
        out.push_str(&format!("a replay of exchange {from}\n"));
    }
    let first = format!(
        "{} {}",
        exchange["method"].as_str().unwrap_or("-"),
        exchange["target"].as_str().unwrap_or("-")
    );
    side(&mut out, ">", &first, &exchange["request"]);
    let status = number(&exchange["status"]);
    side(&mut out, "<", &status, &exchange["response"]);
    let redacted: Vec<&str> = exchange["redacted"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !redacted.is_empty() {
        out.push_str(&format!("redacted: {}\n", redacted.join(", ")));
    }
    out
}

/// The text of `replay`.
pub(crate) fn replay_text(doc: &Value) -> String {
    let exchange = &doc["exchange"];
    let mut out = format!(
        "exchange {} replays exchange {}: {}, status {}, {} ms\n",
        number(&exchange["id"]),
        number(&doc["replayed_from"]),
        exchange["state"].as_str().unwrap_or("-"),
        number(&exchange["status"]),
        number(&exchange["elapsed_ms"]),
    );
    let left: Vec<&str> = doc["left_out"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !left.is_empty() {
        out.push_str(&format!(
            "left out (the inspector keeps no value of a redacted secret): {}\n",
            left.join(", ")
        ));
    }
    out.push_str(&format!(
        "podnet tunnel inspect show {} shows the answer\n",
        number(&exchange["id"])
    ));
    out
}
