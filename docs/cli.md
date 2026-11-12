# The manual, the output formats and the exit codes

This page gives the manual of podnet, the machine-readable forms (`--help --json`, `man --json`, the llms
files), the global flags, the log and JSON output, the stop signals, `--check` and the exit codes.

## Global flags

These flags work before the command (`podnet --proxy URL tunnel ...`) and after it. A flag of `podnet warp`
is not global: it goes after `warp`.

| Flag | Function |
| --- | --- |
| `--ca-file PATH` | The only TLS roots (PEM) for each connection that podnet opens ([proxies.md](proxies.md)). |
| `--proxy URL`, `--no-proxy` | The proxy of each connection, or none ([proxies.md](proxies.md)). |
| `--egress SPEC` | The way out, as a network spec ([egress.md](egress.md)). |
| `--resolve HOST=IP` | Addresses for names in place of DNS ([proxies.md](proxies.md)). |
| `-q`, `-v`, `-vv` | Warnings and errors only; debug; trace. `RUST_LOG` takes priority. |
| `--color auto\|always\|never` | Colours in the log. |
| `--log-format text\|json` | The log on stderr as text, or as one JSON object for each line. |

## The manual

`podnet man` shows the manual. It is generated from the command tree of the
binary, so it always agrees with `--help`. It needs no man(1), no groff and no
network.

```sh
podnet man                      # the whole manual, in a pager on a terminal
podnet man warp run             # one command
podnet man exit-status          # one topic: environment, files, inventory, exit-status, examples, pod
podnet man --roff > podnet.1      # a man(7) page (also a release asset)
podnet man --json               # the tables, for a program
podnet warp run --help --json   # the same JSON for one command (at each level)
podnet status                   # one JSON line about this host; no network
podnet ports --json             # the listening TCP ports, as one JSON object
podnet completions bash > ~/.local/share/bash-completion/completions/podnet   # also zsh, fish, elvish, powershell
podnet -q tunnel ...            # warnings and errors only; -v debug, -vv trace
podnet --log-format json ...    # the log as one JSON object for each line
podnet --color never ...        # colours: auto (a terminal, no NO_COLOR), always, never
```

podnet raises the soft limit of open files (`ulimit -n`) to the hard limit at
start, at most 1048576. `podnet status` shows it in `nofile`. `warp exec`,
`warp shell`, `warp run` and `autostart run` keep the limit of the shell for
the program.

With `--json`, `podnet tunnel`, `serve`, `share` and `demo` write JSON lines on
stdout in place of the text lines. A line with `"event": "ready"` gives
`url`, `tunnel_id`, `locations` and `leg` (and `pin` for `share`). `leg` is
the edge leg that carries the tunnel: `quic`, `http2`, `quic+warp` or
`http2+relay`. The text `edge` line names it too. A line with
`"event": "verified"` comes after the fetch of the public URL. Its `dns`
field names the resolver step that resolves the public name at that time:
`system`, `doh` (public DNS resolves it, and the system resolver of this
host does not: a visitor whose resolver cached NXDOMAIN gets an error for
some seconds more), `pin` (`--resolve`) or `none`. While the fetch fails, podnet
logs the first try and each fifth try at INFO, and each try after the tenth
at WARN.

Stop signals: from the ready line (or the `ready` event) of a server
command on, SIGINT (Ctrl-C), SIGTERM and SIGHUP each start the clean
shutdown, also during the verify of the public URL and with `--run-for`.
A stop before the `verified` event gives no `verified` event. The exit
code is 0. A second signal during the shutdown stops podnet at once. Before
the ready, each signal has its default action. A detached instance ignores
SIGHUP. When the parent ignores SIGHUP (`nohup`), podnet ignores it too.

`--check` on `tunnel`, `serve`, `share` and `demo` checks the plan and stops.
It tests the flags and the config file, each origin (a connection, and the
TLS handshake for `https`; no request is sent), the files and the
directories (`--dir`, `--spa` and its `index.html`, `--file`, the upload
directory, `--warp-identity`, `--credentials-file`), `PODNET_EDGE` and the edge
legs (the `legs` row is the plan: `podnet doctor` tests the reach of each
leg). It makes no Cloudflare API call and no edge connection, opens no
listening socket and writes no file. It prints no token and no PIN. Exit 0:
each check passed. Exit 1: a check of an origin failed. Exit 78: each
failed check is a check of the configuration (`PODNET_EDGE`, the credentials
file, the roots). A bad command line (64) or a bad config file (78) stops
before the check. With `--json`, the output is one
document: `{"ok", "plan", "checks": [{"name", "status", "detail"}]}`.

```sh
podnet serve --check --dir ./site --spa
podnet tunnel --check --json --config config.yml | jq -e .ok
```

The exit codes follow sysexits.h: 64 for a bad command line, 66 when a file
that must exist is missing (the key or the allow list of the link), 69 when a
service or a network path is not available (no edge leg worked), 71 when a
file of the host cannot be used (a key file with a mode that is not 0600), 77
when a peer is not in the allow list of the link, 78 for a bad configuration
(for example `PODNET_EDGE`). 68 is for an address of a peer that does not
resolve. `podnet man exit-status` gives all of them.
The exit codes of `podnet link` are in [link.md](link.md).

`PODNET_ERROR_PROGRAM=/path/to/program` gives a JSON report of each failure
(version, system, command, exit code, the last 64 log lines) to that program
on its stdin. Credentials are masked. The program does not get
`TUNNEL_TOKEN`, the relay tokens, or a proxy variable with a user and a
password in its environment. Use it to collect failures in tests.

## Machine-readable help and the llms files

* `podnet man --json` prints the manual as one JSON object: `name`, `version`, `about`, `description`,
  `commands`, `pod_usage`, `environment`, `files`, `exit_status` and `examples`. Each entry of `commands`
  has `path`, `usage`, `about`, `long_about`, `args` and `subcommands`.
* `podnet COMMAND --help --json` prints the same entry for one command, at each level
  (`podnet route --help --json`, `podnet warp run --help --json`). Each argument has `synopsis`, `id`,
  `positional`, `required`, `repeatable`, `takes_value`, `help`, `default`, `env` and `values`.
* `podnet man --roff` prints a man(7) page. `podnet.1` is a release asset.
* `llms.txt` is an index of the commands and the documents. `llms-full.txt` holds the manual (the output of
  `podnet man --no-pager`) and the text of the documents of this repository. Both are in the root of the
  repository and are release assets. `scripts/gen-llms.py` writes them from a binary; run it after a
  change of the command tree, of the manual or of a document, and `--check` says whether they are current.
* `podnet completions SHELL` prints a completion script for `bash`, `zsh`, `fish`, `elvish` or `powershell`.
