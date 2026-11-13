# Background runs and autostart

## Run in the background: `--detach`

```sh
podnet tunnel --port 3000 --detach --name web   # prints one JSON line, exits 0
podnet warp proxy --listen unix:/tmp/warp.sock --detach
podnet ps                                        # name, state, pid, URL or listen address
podnet logs web -f                               # -n LINES: only the last lines
podnet wait web                                  # the ready document again (exit 124 at --timeout)
podnet stop web                                  # or: podnet stop --all
```

`--detach` works with `tunnel`, `serve`, `share`, `demo`, `connect`,
`warp proxy`, `warp forward`, `warp tun`, `warp exec`, `relay node`,
`relay connect`, `ssh-relay`, `git serve`, `route`, `link listen`, `link connect`,
`net proxy` and `net switch`. The command runs in the foreground until it
is ready: until the public URL answers, or until the listener is bound.
Then it leaves the terminal (`setsid`, SIGHUP ignored, stdin to
`/dev/null`, stdout and stderr to its log), and the start prints one JSON
document on stdout: `name`, `pid`, `log`, and `url` or `listen`. The lines
of the instance before the ready go to stderr.
The instance keeps running when the shell or the terminal closes. A sandbox
or a container that kills each process of its run also ends it.

* A second start with the same name and the same arguments prints the same
  document and starts nothing. Other arguments exit 69; `--replace` stops
  the instance and starts the new one.
* Without `--name`, the name is the command and a hash of its arguments.
* `--ready-timeout SECS` (default 60): after it, the start stops the
  instance and exits 124 (as `timeout`). An instance that ends before it is ready gives its
  own exit code.
* A token or a proxy password on the command line goes to the environment
  of the instance, not to its arguments or its record. The record of a
  foreground `--name` (no `--detach`) has no secret either.
* The files are in `$XDG_RUNTIME_DIR/podnet/run/` (else `run` in the state
  directory, or `PODNET_RUN_DIR`), mode 0700: `NAME.json` (0600), `NAME.lock`
  (held while the instance runs), `NAME.secrets` (0600, only when the
  instance has a token or a proxy password: the variables that hold them)
  and `NAME.log` (0600, cut at 1 MiB; the
  part before goes to `NAME.log.1`). Each start writes a line
  `podnet: ---- start TIME pid PID name NAME ----`, then its log lines from
  the start (also the lines before the ready, which the start shows too).
  The log has no colour codes.
* A supervisor starts the instance again when it ends, with a backoff of
  1 s doubled to 30 s, but not after exit 64 or 78, and not before the first
  ready. A planned end is not restarted: `--stop-after`, `--idle-secs` and
  `--run-for` end the instance for good (a crash or a kill still restarts
  it). When the quick-tunnel service refused a provision with a
  `Retry-After` (HTTP 429), the next start waits that long (at most 1 h),
  and the run is not a quick failure. After 5 quick failures in sequence it
  runs `podnet.prev` (the binary before the last `podnet update`). A quick
  tunnel gets a new URL at a restart: `podnet ps` shows it, and the log has a
  line for it.
  `--no-restart` turns the supervisor off. `podnet stop` stops the supervisor
  and its instance.
* podnet never removes a lock file that a live process holds. When the
  record of a live instance cannot be read, `podnet ps` shows the instance
  with the state `unknown`, and `podnet stop` exits 69 and removes nothing.
  `podnet stop --all` goes on to the next instance after a failure.
* `podnet autostart add NAME --from INSTANCE` makes a detached instance start
  at boot too. It copies the arguments of the record and the secrets of
  `INSTANCE.secrets`.
* `podnet status` lists the instances.
* `--name NAME` with no `--detach` registers the process in the foreground
  (for a program that runs podnet as its child): it is not forked and not
  restarted, its stdout and stderr stay, and its log lines go to `NAME.log`
  too. `podnet ps`, `logs`, `wait` and `stop` see it. A second process with
  the name fails ("runs already"). `--replace` needs `--detach`.

## Autostart

`podnet autostart` starts a podnet command at boot and after a logout, with no
root. It uses a systemd user unit when `systemctl --user` answers, else an
`@reboot` line in the crontab. With neither, it prints the line to run at
login and exits 69.

```sh
TUNNEL_TOKEN=... podnet autostart add web -- tunnel --url http://127.0.0.1:8080
podnet autostart list
podnet autostart remove web
```

* The arguments and the variables (`TUNNEL_TOKEN`, the proxy variables,
  `SSL_CERT_FILE`, `PODNET_*`) go to private files (mode 0600) in
  `<state>/autostart/`. `--token` and a `--proxy` with a password move to
  `TUNNEL_TOKEN` and `PODNET_PROXY`. The unit, the crontab line and the
  command line of the process hold no secret.
* The unit and the line run this binary by its absolute path. A binary in
  a temporary directory is refused. The unit quotes for systemd (`%` and
  `$` are escaped), the crontab line for `sh`.
* An entry runs in the foreground under its service manager: `add`
  removes `--detach`, `--name`, `--ready-timeout` and `--no-restart`, and
  says so.
* A systemd user manager stops at logout unless linger is on:
  `loginctl enable-linger`.
* `remove` deletes only what `add` wrote (a marker line in the unit, a
  marker comment on the crontab line).
* Each program that `add` or `remove` runs has a limit of 10 s.
* `podnet autostart run NAME --dir DIR` is the entry point of the unit and of the crontab line. It reads
  `NAME.args` and `NAME.env` in DIR and runs the command in the place of its own process. Do not call it by hand.
* With no systemd user manager and no cron, `add` exits 69. Its error text
  gives a `nohup` line: put it in a login script (for example
  `~/.profile`).
