#[allow(unused_imports)]
use crate::prelude::*;

// ── command line ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendKind {
    Auto,
    Netns,
    Seccomp,
}

pub(crate) struct Args {
    pub(crate) proxies: Vec<Proxy>,
    pub(crate) rules: Vec<Rule>,
    pub(crate) maps: Vec<(String, IpAddr)>,
    pub(crate) program: Vec<String>,
    pub(crate) doctor: bool,
    pub(crate) doctor_json: bool,
    pub(crate) verbose: bool,
    pub(crate) inbound_dir: std::path::PathBuf,
    /// `--inbound-dir` was given (the seccomp backend only uses it).
    pub(crate) inbound_dir_given: bool,
    /// `--session DIR` (T-110): the fake DNS table of the seccomp backend
    /// that other pods of DIR share (`DIR/dns`).
    pub(crate) dns_state: Option<std::path::PathBuf>,
    /// `--host-gateway` (T-112): the gateway of the netns backend reaches
    /// the loopback of the host.
    pub(crate) host_gateway: bool,
    /// `--keep-proxy-env`: the program keeps the proxy variables.
    pub(crate) keep_proxy_env: bool,
    pub(crate) serve: Option<crate::listen::ProxyListen>,
    pub(crate) backend: BackendKind,
    /// `--prefer-udp`: `auto` takes the seccomp backend first (it carries
    /// UDP datagrams of any size).
    pub(crate) prefer_udp: bool,
    /// `--name-backend`: print one line that names the backend that runs,
    /// its limits (T-071) and the reason of the choice (T-113).
    pub(crate) name_backend: bool,
    /// `--flow-log FILE`: the JSON record of each decision (T-157).
    pub(crate) flow_log: Option<std::path::PathBuf>,
    /// `--learn FILE` or `--fail-on-violation FILE` (T-158).
    pub(crate) learned: Option<Learned>,
    /// `--flow-budget N`: the ceiling of the tracked sockets (T-181).
    pub(crate) flow_budget: Option<usize>,
    /// `--loopback-rules` (T-231): the seccomp backend applies the rules to
    /// a TCP connect to 127.0.0.0/8 and ::1.
    pub(crate) loopback_rules: bool,
}

/// The mode of the learned set (T-158).
pub(crate) enum Learned {
    /// `--learn FILE`: write the destinations of the run to FILE.
    Learn(std::path::PathBuf),
    /// `--fail-on-violation FILE`: refuse each destination that is not in
    /// FILE, and fail the run.
    Enforce(std::path::PathBuf),
}

/// The name in the help, the usage lines and the start of each message:
/// `podnet pod`, or the name that [`set_program_name`] gives.
static PROGRAM_NAME: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// The line of `--version`, when the program gives one.
static VERSION_LINE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Set the name that the help and each message show (once, before
/// [`crate::main_with_args`]).
pub fn set_program_name(name: &'static str) {
    let _ = PROGRAM_NAME.set(name);
}

/// Set the line that `--version` prints (once, before
/// [`crate::main_with_args`]): the version line of the program that runs
/// the pod. Without it, `--version` prints the program name and the
/// version of this library.
pub fn set_version_line(line: &'static str) {
    let _ = VERSION_LINE.set(line);
}

pub(crate) fn program_name() -> &'static str {
    PROGRAM_NAME.get().copied().unwrap_or("podnet pod")
}

/// The help, in the layout of the other podnet commands (issue 95).
pub fn usage() -> String {
    usage_for(program_name())
}

/// The help with `name` as the name of the program.
pub fn usage_for(name: &str) -> String {
    format!(
        "\
Run a program with all egress through a proxy, without shims

Usage: {name} [OPTIONS] -- <PROGRAM> [ARGS]...
       {name} doctor [--json]
       {name} --serve <LISTEN> -x <PROXY>

Arguments:
  <PROGRAM> [ARGS]...  The program to run, and its arguments

Options:
  -x, --proxy <SPEC>       Upstream proxy, repeatable; more than one chains:
                             socks5://[user:pass@]host:port
                             http://[user:pass@]host:port
                             socks5://unix:/path     (unix-socket proxy)
                             http://unix:/path       (HTTP CONNECT on a unix socket)
                             socks5h+unix:/path, http+unix:/path (the same)
                             tailscale:/path/to/tailscaled.sock
                           Default: $ALL_PROXY, $HTTPS_PROXY or $HTTP_PROXY.
                           --egress SPEC is the same option (its name in each
                           podnet command)
  -r, --rule <SPEC>        Per-destination route; first match wins:
                             domain:NAME=socks5://host:port
                             domain:.example.com=...  (the name and each
                                                       name under it)
                             cidr:10.0.0.0/8=socks5://host:port
                             cidr:10.0.0.0/8=direct   (no proxy)
                             domain:.ads.example=reject  (refuse; also block):
                             connect fails with ECONNREFUSED (netns: a RST),
                             a datagram with EACCES (netns: dropped),
                             --serve answers SOCKS5 0x02 or HTTP 403
                           With the seccomp backend, 127.0.0.0/8 goes direct
                           (or to a listener of the program itself)
      --default-action <A> allow (default) or reject: what the pod does with a
                           target that no rule matches. reject refuses it as
                           a reject rule does
      --loopback-rules     Seccomp backend: apply the rules to a TCP connect to
                           127.0.0.0/8 and ::1 (by default it goes direct). A
                           refused one gets ECONNREFUSED; an allowed one goes
                           direct
      --flow-log <FILE>    Append one JSON line to FILE for each decision
                           (time, target, rule or default, chain, verdict)
                           and one at the close of each relayed TCP flow
                           (bytes_out, bytes_in)
      --learn <FILE>       Write each destination of the run to FILE, one
                           PROTO TARGET line each. A flow that a rule
                           refuses goes through the -x chain
      --fail-on-violation <FILE>
                           Refuse each destination that is not in FILE (a
                           file of --learn). A learned destination that a
                           rule refuses is also a violation. The pod writes
                           each violation on stderr and exits 4 when the
                           program exits 0
      --flow-budget <N>    Seccomp backend: the most sockets of the program
                           that the pod tracks at one time (64 to 1048576).
                           A socket call above it gets EMFILE. Default: from
                           RLIMIT_NOFILE (pod doctor shows it)
      --map <NAME=ADDR>    Resolve NAME to ADDR locally, repeatable
      --keep-proxy-env     Keep HTTPS_PROXY, HTTP_PROXY, ALL_PROXY and NO_PROXY
                           (and the lower-case forms) for the program; by
                           default the program does not get them
      --backend <NAME>     auto (default), netns or seccomp. With netns, a
                           listener of the program is in the network of the
                           pod: a loopback TCP listener is not seen from
                           outside; use a unix socket (connect -L unix:///PATH)
      --prefer-udp         With auto: seccomp first when it works (UDP
                           datagrams of any size go through; netns carries
                           a datagram of 1500 bytes at most)
      --name-backend       Print one line on stderr that names the backend
                           that runs, its limits and the reason of the
                           choice (also after auto)
      --inbound-dir <DIR>  Where a bound port is exposed as a unix socket
                           (seccomp backend only). The directory must be of
                           this user and not writable by the group or by
                           others; a new one gets mode 0700
                           [default: $XDG_RUNTIME_DIR/podnet-netns-inbound,
                           else $TMPDIR/podnet-netns-inbound-UID]
      --host-gateway       Netns backend: a TCP connect to the gateway of the
                           pod (10.66.255.254) and a port goes to 127.0.0.1
                           and that port on the host; a UDP datagram to the
                           gateway (not port 53) goes from a UDP socket of
                           the host. Off by default: the connect is refused
      --session <DIR>      Seccomp backend: share the inbound listeners
                           (DIR/inbound, unless --inbound-dir) and the fake
                           DNS table (DIR/dns) with each pod of DIR. The
                           directory must be of this user and not writable
                           by the group or by others. The netns backend does
                           not use it
      --serve <LISTEN>     Serve SOCKS5, HTTP CONNECT and UDP ASSOCIATE on
                           unix:/path or tcp://host:port, forwarding through
                           the -x chain
  -v, --verbose            Log each flow; with seccomp, also each socket and
                           each setsockopt that the pod emulates
  -V, --version            Print the version
  -h, --help               Print help

Exit status: the exit code of the program; 64 for a usage mistake (also no
proxy at all); 3 when no backend can run on this host; 4 for a violation of
--fail-on-violation when the program exits 0. The pod returns when
the program exits; a process that the program started keeps the network of
the pod until it exits. doctor measures what
the host permits and exits 3 when no backend can run.
"
    )
}

pub(crate) fn parse_args(argv: &[String]) -> Result<Args> {
    let mut proxies = Vec::new();
    let mut rules = Vec::new();
    let mut program = Vec::new();
    let mut doctor = false;
    let mut doctor_json = false;
    let mut verbose = false;
    let mut maps: Vec<(String, IpAddr)> = Vec::new();
    let mut backend = BackendKind::Auto;
    let mut prefer_udp = false;
    let mut name_backend = false;
    let mut inbound_dir = crate::listen::default_inbound_dir();
    let mut inbound_dir_given = false;
    let mut session: Option<std::path::PathBuf> = None;
    let mut host_gateway = false;
    let mut keep_proxy_env = false;
    let mut serve: Option<crate::listen::ProxyListen> = None;
    let mut default_reject = false;
    let mut flow_log = None;
    let mut loopback_rules = false;
    let mut learned = None;
    let mut flow_budget = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--" => {
                program = argv[i + 1..].to_vec();
                break;
            }
            "doctor" if i == 0 => {
                doctor = true;
                i += 1;
                if argv.get(i).map(String::as_str) == Some("--json") {
                    doctor_json = true;
                    i += 1;
                }
            }
            "-V" | "--version" => {
                match VERSION_LINE.get() {
                    Some(line) => println!("{line}"),
                    None => println!(
                        "{} {} ({})",
                        program_name(),
                        env!("CARGO_PKG_VERSION"),
                        env!("PODNET_BUILD_COMMIT")
                    ),
                }
                std::process::exit(0);
            }
            "--keep-proxy-env" => {
                keep_proxy_env = true;
                i += 1;
            }
            "--prefer-udp" => {
                prefer_udp = true;
                i += 1;
            }
            "--name-backend" => {
                name_backend = true;
                i += 1;
            }
            "--default-action" => {
                let value = argv
                    .get(i + 1)
                    .context("--default-action needs allow or reject")?;
                default_reject = match value.to_ascii_lowercase().as_str() {
                    "allow" => false,
                    "reject" | "block" => true,
                    other => bail!("--default-action {other:?}: give allow or reject"),
                };
                i += 2;
            }
            "--loopback-rules" => {
                loopback_rules = true;
                i += 1;
            }
            "--flow-log" => {
                let path = argv.get(i + 1).context("--flow-log needs a file")?;
                flow_log = Some(std::path::PathBuf::from(path));
                i += 2;
            }
            "--learn" | "--fail-on-violation" => {
                let flag = argv[i].as_str();
                let path = argv
                    .get(i + 1)
                    .with_context(|| format!("{flag} needs a file"))?;
                if learned.is_some() {
                    bail!("give --learn or --fail-on-violation one time");
                }
                let path = std::path::PathBuf::from(path);
                learned = Some(match flag {
                    "--learn" => Learned::Learn(path),
                    _ => Learned::Enforce(path),
                });
                i += 2;
            }
            "--flow-budget" => {
                let value = argv.get(i + 1).context("--flow-budget needs a number")?;
                let n: usize = value
                    .parse()
                    .ok()
                    .filter(|n| {
                        (crate::fdtable::MIN_CEILING..=crate::fdtable::MAX_CEILING).contains(n)
                    })
                    .with_context(|| format!("--flow-budget {value:?}: give 64 to 1048576"))?;
                flow_budget = Some(n);
                i += 2;
            }
            "--map" => {
                let spec = argv.get(i + 1).context("--map needs NAME=ADDR")?;
                let (name, addr) = spec.split_once('=').context("--map needs NAME=ADDR")?;
                if name.is_empty() {
                    bail!("--map has an empty name");
                }
                maps.push((name.to_string(), addr.parse().context("--map address")?));
                i += 2;
            }
            "-h" | "--help" => {
                print!("{}", usage());
                std::process::exit(0);
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            // T-060: --egress is the name of -x in each podnet command.
            "-x" | "--proxy" | "--egress" => {
                let spec = argv.get(i + 1).context("--proxy needs a value")?;
                proxies.push(Proxy::parse(spec)?);
                i += 2;
            }
            "-r" | "--rule" => {
                let spec = argv.get(i + 1).context("--rule needs a value")?;
                rules.push(Rule::parse(spec)?);
                i += 2;
            }
            "--serve" => {
                let spec = argv.get(i + 1).context("--serve needs a listen spec")?;
                serve = Some(
                    crate::listen::ProxyListen::parse(spec).map_err(|e| anyhow::anyhow!("{e}"))?,
                );
                i += 2;
            }
            "--inbound-dir" => {
                inbound_dir = std::path::PathBuf::from(
                    argv.get(i + 1).context("--inbound-dir needs a value")?,
                );
                inbound_dir_given = true;
                i += 2;
            }
            "--host-gateway" => {
                host_gateway = true;
                i += 1;
            }
            "--session" => {
                session = Some(std::path::PathBuf::from(
                    argv.get(i + 1).context("--session needs a value")?,
                ));
                i += 2;
            }
            "--backend" => {
                let value = argv.get(i + 1).context("--backend needs a value")?;
                backend = match value.as_str() {
                    "auto" => BackendKind::Auto,
                    "netns" => BackendKind::Netns,
                    "seccomp" => BackendKind::Seccomp,
                    other => bail!("unknown backend {other:?} (auto, netns, seccomp)"),
                };
                i += 2;
            }
            other => bail!("unexpected argument {other:?}; see --help"),
        }
    }
    // An ambient proxy is used when no -x is given, so the tool composes with
    // whatever the environment already names.
    if proxies.is_empty() {
        for var in [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ] {
            if let Ok(value) = std::env::var(var) {
                if !value.is_empty() {
                    if let Ok(proxy) = Proxy::parse(&value) {
                        proxies.push(proxy);
                        break;
                    }
                }
            }
        }
    }
    // T-157: a target that no rule matches is refused: a last rule that
    // matches each target.
    if default_reject {
        rules.push(Rule::default_reject());
    }
    if learned.is_some() && serve.is_some() {
        bail!("--learn and --fail-on-violation work with a program, not with --serve");
    }
    if !doctor && serve.is_none() && program.is_empty() {
        bail!(
            "nothing to run; usage: {} [OPTIONS] -- PROGRAM",
            program_name()
        );
    }
    // T-110: the pods of one session share the inbound listeners and the
    // fake DNS table of the seccomp backend.
    if let (Some(dir), false) = (&session, inbound_dir_given) {
        inbound_dir = dir.join("inbound");
    }
    let dns_state = session.map(|dir| dir.join("dns"));
    Ok(Args {
        proxies,
        rules,
        maps,
        program,
        doctor,
        doctor_json,
        verbose,
        inbound_dir,
        inbound_dir_given,
        dns_state,
        host_gateway,
        keep_proxy_env,
        serve,
        backend,
        prefer_udp,
        name_backend,
        flow_log,
        learned,
        flow_budget,
        loopback_rules,
    })
}
