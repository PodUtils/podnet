//! `podnet autostart`: start a podnet command at boot.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use podnet::exit::{Fault, ResultExt};

use crate::cli::Cli;

#[derive(Args, Debug)]
#[command(subcommand_required = true, arg_required_else_help = false)]
pub(crate) struct AutostartArgs {
    #[command(subcommand)]
    pub(crate) command: AutostartCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum AutostartCommand {
    /// Add an entry that runs `podnet ARGS`. The token and a proxy password
    /// go to a private env file (mode 0600), not to the unit, the crontab
    /// or the command line. Exit 69 when this host has no systemd user
    /// manager and no crontab, unless --manual is given.
    Add {
        /// The name of the entry (letters, digits, - and _).
        name: String,
        /// The podnet command and its arguments, after `--`, with no leading
        /// `podnet` (one is dropped). The command must parse.
        #[arg(last = true, required_unless_present = "from")]
        args: Vec<String>,
        /// Take the command and its arguments from the detached instance
        /// INSTANCE (podnet ps), with no options of the detach.
        #[arg(long, value_name = "INSTANCE", conflicts_with = "args")]
        from: Option<String>,
        /// Install no unit and no crontab line: write the entry, print the
        /// line to run at login (for example in ~/.profile) and exit 0.
        #[arg(long)]
        manual: bool,
    },
    /// Remove an entry: its unit or its crontab line, and its files. Other
    /// units and crontab lines stay as they are.
    Remove {
        /// The name of the entry.
        name: String,
    },
    /// List the entries.
    List {
        /// Print one JSON document: {"entries": [...]}. Each entry has its
        /// name, its arguments, the names of its variables (not the
        /// values), its way (systemd, cron or manual) and its run line.
        #[arg(long)]
        json: bool,
    },
    /// Run an entry in the place of this process: the unit and the crontab
    /// line call this. It reads NAME.args and NAME.env in DIR.
    Run {
        /// The name of the entry.
        name: String,
        /// The directory of the entries. The default is `<state>/autostart`,
        /// the directory of `autostart add`.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

/// The arguments of `autostart add` (issue 117): a first argument `podnet`
/// (or a path to it) is dropped, and the rest must parse as a podnet command.
/// Before, `-- podnet warp up` was kept as `podnet podnet warp up`, which failed
/// at each boot with no message.
pub(crate) fn autostart_args(mut args: Vec<String>) -> Result<Vec<String>> {
    let is_podnet = |arg: &str| std::path::Path::new(arg).file_name() == Some("podnet".as_ref());
    if args.first().is_some_and(|a| is_podnet(a)) {
        args.remove(0);
    }
    // Issue 265: an entry with --detach forked at each boot, and its
    // service manager saw the parent end.
    let kept = podnet::detach::without_start_options(&args);
    if kept != args {
        eprintln!("podnet: autostart add: --detach, --name, --ready-timeout and --no-restart are removed; the entry runs in the foreground");
        args = kept;
    }
    if let Err(err) = Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(args.clone()))
    {
        let first = err.to_string();
        let first = first
            .lines()
            .next()
            .unwrap_or_default()
            .trim_start_matches("error: ");
        return Err(podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!(
                "autostart add: `podnet {}` is not a podnet command: {first}",
                args.join(" ")
            ),
        ));
    }
    Ok(args)
}

pub(crate) fn autostart(command: AutostartCommand) -> Result<()> {
    use podnet::autostart;
    match command {
        AutostartCommand::Add {
            name,
            args,
            from,
            manual,
        } => {
            let (args, secrets) = match from {
                Some(instance) => {
                    let dir = podnet::instances::run_dir().fault(Fault::Config)?;
                    let record = podnet::instances::read(&dir, &instance).ok_or_else(|| {
                        podnet::exit::fault(
                            Fault::Usage,
                            anyhow::anyhow!("no detached instance {instance} (podnet ps)"),
                        )
                    })?;
                    // Issue 265: the record has no secret; NAME.secrets has them.
                    (
                        record.args,
                        podnet::instances::read_secrets(&dir, &instance),
                    )
                }
                None => (args, Vec::new()),
            };
            let args = autostart_args(args)?;
            let added = autostart::add(&name, &args, &secrets, autostart::PROGRAM_LIMIT, manual)?;
            println!(
                "podnet: autostart {name} added ({}): {}",
                added.way.name(),
                added.detail
            );
            Ok(())
        }
        AutostartCommand::Remove { name } => {
            for line in autostart::remove(&name, autostart::PROGRAM_LIMIT)? {
                println!("podnet: {line}");
            }
            Ok(())
        }
        AutostartCommand::List { json: false } => {
            for name in autostart::list()? {
                println!("{name}");
            }
            Ok(())
        }
        AutostartCommand::List { json: true } => {
            println!("{}", list_json()?);
            Ok(())
        }
        AutostartCommand::Run { name, dir } => {
            let dir = match dir {
                Some(dir) => dir,
                None => autostart::dir().fault(Fault::Config)?,
            };
            use std::os::unix::process::CommandExt;
            let (args, vars) = autostart::load(&name, &dir).fault(Fault::Config)?;
            let exe = podnet::self_exe::current().context("the path of this binary")?;
            let err = std::process::Command::new(exe)
                .args(&args)
                .envs(vars)
                .exec();
            Err(anyhow::anyhow!("starting podnet {}: {err}", args.join(" ")))
        }
    }
}

/// The document of `autostart list --json` (#630). It holds no secret: the
/// variables are names only.
fn list_json() -> Result<String> {
    use podnet::autostart;
    let dir = autostart::dir().fault(Fault::Config)?;
    let exe = podnet::self_exe::this_binary().context("the path of this binary")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let mut entries = Vec::new();
    for name in autostart::list()? {
        let (args, vars) = autostart::load(&name, &dir).fault(Fault::Config)?;
        let env: Vec<&str> = vars.iter().map(|(k, _)| k.as_str()).collect();
        entries.push(serde_json::json!({
            "name": name,
            "args": args,
            "env": env,
            "way": autostart::way_of(&name, autostart::PROGRAM_LIMIT).name(),
            "line": autostart::run_line(&exe, &name, &dir),
        }));
    }
    Ok(serde_json::to_string(&podnet::status::stamped(
        serde_json::json!({ "entries": entries }),
    ))?)
}
