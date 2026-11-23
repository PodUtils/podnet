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
network. The crate `pod-manual` of PodUtils/podcrates writes it. A pager shows
it only when stdin and stdout are a terminal; `--no-pager` turns the pager off.

```sh
podnet man                      # the whole manual, in a pager on a terminal
podnet man warp run             # one command
podnet man exit-status          # one topic: environment, files, inventory, exit-status, examples, see-also
podnet man --roff > podnet.1      # a man(7) page (also a release asset)
podnet man --json               # the model and the facts, for a program
podnet warp run --help --json   # the same JSON for one command (at each level)
podnet status                   # one JSON line about this host; no network
podnet ports --json             # the listening TCP ports, as one JSON object
mkdir -p ~/.local/share/bash-completion/completions
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

`--json` is not a global flag. A command without JSON output refuses it (exit 64). To know if a command takes
`--json`, read its flags: `podnet COMMAND --help --json | jq -e '.command.flags[] | select(.long == "json")'`
(see "Machine-readable help"). Each `--detach` start prints a JSON document with or without `--json`
([background.md](background.md)). `route --config FILE --detach --json` is the same as without `--json`.

## The version line

`podnet --version` prints one line: `podnet VERSION (COMMIT)`. `COMMIT` is one word (`unknown` for a local build).
Programs read the second word as the version (podrunner does). The test `the_version_line_keeps_its_form` of
`tests/contracts.rs` holds the form. A change of the form is a change of a contract: it needs a COORD issue
`(to all)` (`docs/decisions.md`, 2026-11-18, Schema ids of `pod-job`).

## JSON documents

Each JSON document of podnet on stdout obeys these rules. The test `tests/contracts.rs` checks them for each
command that runs with no network (`cargo test -p podnet --test contracts`).

1. Shape: a document is one JSON object. An event stream (a server command with `--json` in the foreground) is
   one JSON object on each line. Each line has a string `event`.
2. Stamp: each object has `schema` (a positive integer) and `podnet` (the version line of the binary). The
   documents of `man --json` and `COMMAND --help --json` keep the `schema` of `pod-manual`. A document with a
   named form (`rules check`, `init check`, `ci check`) gives the name in `kind`, for example
   `podnet.rules-check/1`. An environment
   document (`net shim`, `net socksify`, `net passwd-shim`, `net ts-shims`) is the exception: each key is a
   variable name and each value is a string, so that a shell can export each key.
3. Endpoint: each `ready` event and each `detached` document has `url` (a string or `null`), `listen` (a string
   or `null`) and `listens` (an array of strings). `listen` is the first item of `listens`, or `null` when
   `listens` is empty. A command with a local listener puts each listener in `listens`. A `detached` document
   also has `name`, `pid`, `command` and `log`.
4. Secrets: no document holds the value of a token or a password, from the environment or from the command
   line. The keys `token`, `password`, `secret`, `passphrase`, `private_key` and `seed` have no value.

`link listen --json` and `link connect --json` print a `ready` event. `link listen` puts the carrier address in
`listen` (a path for `unix`, `IP:PORT` for `tcp`): give it to `link dial` as it is.

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
`serve --check --local ADDR` and `git serve --check --local ADDR` (or
`--ssh ADDR`) with no tunnel check the directory and each listener (rows
`files`, `local N` and `ssh N`; see [serve.md](serve.md)). They bind each TCP
address with no listen, and close it at once.

```sh
podnet serve --check --dir ./site --spa
podnet tunnel --check --json --config config.yml | jq -e .ok
```

The exit codes follow sysexits.h: 64 for a bad command line, 65 for input
that `podnet receive` refuses (a bad name in a transfer, a name that
exists), 66 when a file
that must exist is missing (the key or the allow list of the link), 69 when a
service or a network path is not available (`tunnel`, `warp` and the other
commands that start a session: no edge leg worked), 71 when a
file of the host cannot be used (a key file with a mode that is not 0600), 77
when a peer is not in the allow list of the link or of a sealed relay session
(`relay node --seal`, `relay connect --seal`), when a peer refused a key (a
`PresharedKey` of `wg:FILE` that differs), or when the code of `podnet send`
did not match, 78 for a bad configuration
(for example `PODNET_EDGE`). 68 is for an address of a peer that does not
resolve. 124 is for a time limit: `--detach` was not ready in
`--ready-timeout`, `podnet send` got no receiver in `--timeout`, or the hello
of a link did not end in time. `podnet doctor` gives 1 when no leg can work, and `podnet bench`
gives 1 when no request worked: their job is the check, and the check
failed. `podnet man exit-status` gives all of them.
The exit codes of `podnet link` are in [link.md](link.md).

`PODNET_ERROR_PROGRAM=/path/to/program` gives a JSON report of each failure
(version, system, command, exit code, the last 64 log lines) to that program
on its stdin. Credentials are masked. The program does not get
`TUNNEL_TOKEN`, the relay tokens, or a proxy variable with a user and a
password in its environment. Use it to collect failures in tests.

## Machine-readable help and the llms files

* `podnet man --json` prints the manual as one JSON object of `pod-manual` (schema 1): `schema`, `name`,
  `version`, `date`, `about`, `description`, `commands`, `topics`, `facts`, `examples` and `see_also`. Each
  entry of `commands` has `path`, `aliases`, `about`, `notes`, `usage`, `args` (positional), `flags` and `rules`.
  Each flag has `short`, `long`, `value`, `help`, `kind`, `default`, `values`, `env`, `required` and
  `repeatable`. `facts` has `variables` (in groups), `exits` (each `code` is a value, `128+N` or the code of
  the program), `files` and `tables` (the inventory).
* `podnet man --json SECTION` prints one section: `{"schema", "name", "command"}` for a command, or
  `variables`, `files`, `exits`, `examples` or `table` for a topic.
* `podnet COMMAND --help --json` prints the JSON of `podnet man --json COMMAND` for one command, at each level
  (`podnet route --help --json`, `podnet warp run --help --json`). At the root it prints the root command.
* `podnet man --roff` prints a man(7) page. `podnet.1` is a release asset.
* `llms.txt` is an index of the commands and the documents. `llms-full.txt` holds the manual (the output of
  `podnet man --no-pager`) and the text of the documents of this repository. Both are in the root of the
  repository and are release assets. The writer of `pod-manual` makes them from the model. The test
  `the_llms_files_equal_the_model` fails when a file is old. After a change of the command tree, of the manual,
  of the version or of a listed document, run `cargo test --bin podnet -- --ignored write_the_llms_files`.
* `podnet completions SHELL` prints a completion script for `bash`, `zsh`, `fish`, `elvish` or `powershell`.
