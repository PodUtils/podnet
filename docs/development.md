# Development

Rules for the code and the tests of podnet. `CLAUDE.md` has the rules of
each session; `docs/maintenance.md` has the procedures.

## Checks before a push

Run `sh scripts/agent/check.sh`. It runs these checks:

* `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`;
* `cargo todo check` (the work record in `TODO/`, `TODO/RULES.md`);
* `python3 -I scripts/check-size.py`, `python3 -I scripts/check-layers.py` and `python3 -I scripts/check-claims.py`;
* `cargo doc` with `-D warnings`;
* `python3 -I scripts/doc-commands.py --check` and `python3 -I scripts/gate-scenarios.py --check`;
* the man page, the installer core, the workflows and `shellcheck`;
* `cargo test --workspace`. `sh scripts/agent/check.sh notest` skips it.

Push only when each exits 0. CI (`.github/workflows/ci.yml`) runs the same.

## File size

A Rust file has 800 lines or fewer. `scripts/check-size.py` fails on a
longer file, except the files of `scripts/size-allow.txt`. An allowlisted
file can only become smaller: when it becomes smaller, lower its count
(`python3 -I scripts/check-size.py --update`). Do not add a file to the
list: split the file.

## Layers

Each top module of `src/` belongs to one crate of the split
(`docs/research/podnet/architecture.md`, section 4.1): core, vnet, relay,
warp, serve, cloudflare or podnet (`LAYER` in the script is the list). `scripts/check-layers.py` has the table
of module to crate and the crates that each crate may use. A `crate::X`
reference (outside `#[cfg(test)]`) to a crate that the crate of the module
may not use is a fault, except the rows of `scripts/layers-allow.txt`.
The list can only shrink: remove a row when its reference is cut. Do not
add a row: move the item to a lower crate. A new top module needs a row
in `LAYER` of the script. `--list` prints each reference between crates.

## Documented commands

Each fenced `sh` or `bash` block of `README.md` and `docs/*.md` (not
`docs/releases/`, `docs/reports/`, `docs/research/`) is a test (T-358,
step 2). `scripts/doc-commands.txt` gives each block a class, with one
line `FILE HASH CLASS REASON`:

| Class | Meaning |
| --- | --- |
| `run` | It runs offline as written, with `podnet` on PATH. |
| `live` | It needs a live service or a second host. |
| `root` | It needs root or a kernel feature. |
| `skip` | It is not a command to run as written: a template with placeholders, a reference list, two shells. |

HASH is the first 10 hex digits of the SHA-256 of the block. A change of a
block changes its hash, so a new or changed block needs a new line.
`--list` prints each block with its file, line, hash and class.
`--check` fails on a block with no line and on a line with no block.
`--run` runs each `run` block with `sh -e` in a new temporary HOME and
working directory, with `target/debug/podnet` (or `--bin PATH`) first on
PATH, and a timeout of 30 s for each block. A reason that starts with
`[needs TOOL,TOOL]` names the other programs of the block: with one of
them absent, `--run` does not run the block and exits 1. CI runs
`--check` (job `workflows`) and `--run` (job `test`).

A `run` block that fails is a defect of the code or of the document:
record it in `TODO/fixes.md`. Do not give a failing block the class
`skip`. Give a block the class `skip` only when it is not a command to run
as written, and write why.

## The lane in the profiles of pod-lab

`scripts/lab-lane.sh` runs the lane of podnet (T-358, step 1) in the
profiles of `pod-lab` (`PodUtils/podcrates:crates/pod-lab/LANES.md`). It
takes the binary `podlab` and the podnet binary to test, and runs each
command in a lab of each profile with `podlab run PROFILE -- ...`.

1. Get `podlab`. The release of podcrates `v0.58.0` or later has the static
   binary: `sh scripts/get-podlab.sh TAG` (for example `v0.82.1`) downloads
   `podlab-ARCH-unknown-linux-musl` and `SHA256SUMS` into the new directory
   `.work/podlab-TAG` and checks the SHA-256. Not each tag of podcrates
   has a release: when the release of the tag does not have the asset, the
   script takes the newest release at or below the tag that has it, and
   writes one line with the two versions (T-514). It reads the list of
   releases from the GitHub REST API (`GET_PODLAB_API`, default
   `https://api.github.com`; `GH_TOKEN` when it is set). Exit codes: 1 when
   a download or the check fails or no release has the asset, 64 for usage.
   The release has no
   signature of its assets (their entry 717 of podcrates), so the check stops at
   `SHA256SUMS`. Give the directory as PODLAB: the lane checks the asset
   against `SHA256SUMS` again before each run. Or build `podlab` in a clone
   of podcrates at the tag that podnet takes: `cargo build --release -p
   pod-lab --features cli --target x86_64-unknown-linux-musl` (with
   `CC_x86_64_unknown_linux_musl=clang`), and give the binary as PODLAB.
2. Build the release binary of podnet (`docs/build.md`).
3. Run `sh scripts/lab-lane.sh PODLAB PODNET [PROFILE...]` as a user that
   is not root. With no PROFILE, it runs each profile of `podlab list`
   and the row `allow-connect`. Use a clean environment (`env -i
   PATH=/usr/bin:/bin HOME=DIR`): a proxy variable of the host must not
   go into a lab.
   As root, the profile `nonroot-nocaps` does not hold, and each other lab
   runs as root. As root, run it as uid 65534: `setpriv --reuid 65534
   --regid 65534 --clear-groups env LANE_DIR=DIR sh scripts/lab-lane.sh
   PODLAB PODNET`, with a DIR that uid 65534 can write.

The commands: `podnet --version`, `podnet man --no-pager`, `podnet doctor`
(exit 0 or 1, no panic), `podnet net shim --forward
127.0.0.1:7001=unix:PATH` with a dynamic Python client and a unix echo
server (with a negative control: with no shim, the client gets no echo),
`podnet link listen` and `podnet link dial` over the unix carrier,
`podnet pod --backend seccomp` and `--backend netns` around `/bin/true`,
and `podnet net proxy --listen unix:PATH` with a SOCKS5 greeting.

The script prints one row for each profile and command: `pass`, `fail`
with the log, or `n.a.` with the reason (a lab that does not hold here; a
pod backend that podnet refuses with exit 3 and a reason). It exits 1 on a
failure. The logs are in `LANE_DIR` (default `.work/lab-lane`), which must
not be in `/tmp` or in `HOME`. The podnet binary must not be in `HOME`
(the lab `noexec-home` mounts it noexec). It needs `python3`, `cc` (for the shim) and
`timeout`.

`.github/workflows/gate.yml` runs the lane on a GitHub runner as the
user `runner` (not root): the static release binary, `podlab` of the
podcrates tag of `Cargo.toml`, each profile and `allow-connect`, then the
plant `LANE_PLANT=net` in `proxy-only`. It turns off
`kernel.apparmor_restrict_unprivileged_userns` first: the labs need a user
namespace. Start it by hand: `gh workflow run gate.yml -R PodUtils/podnet
--ref main`. `release.yml` calls it as its job `gate`, in the dry run and
on the tag: the job `publish` needs it, so no release is published
without a passing gate. `gate.yml` keeps the rows of the lane as the
artifact `gate`. `scripts/gate-notes.sh LANE.TXT [RUN-URL]` writes them as
the section "The release gate" of the notes of a release (the counts, and
each row in a block that opens on a click); the job `publish` adds it to
the body, and stops when a row fails.

### The scenarios of each command

`src/cli/scenarios.txt` has one line for each command with no subcommand:
the path of the command, then each scenario that runs the release binary
with that command (T-358 step 4):

* `live:STEP`: a step of `live.yml` (live services, each push to `main`),
* `ci:STEP`: a step of the job `static` of `ci.yml` (the consumers),
* `lane:CASE`: a case of `scripts/lab-lane.sh` (the gate of each release).

`scripts/gate-scenarios.py` finds each call of the binary in the steps and
in the scripts that the steps run. The cases of the lane have a map in the
script (`LANE_MAP`), and so do the commands that a step runs through
another program, such as a unit of systemd (`VIA`, with the text that the
step must hold). A command with no scenario is experimental: its help
starts with "Experimental:" (`src/cli/help.rs`). After a change of a
workflow, of the lane or of the commands, write the file again:

```sh
python3 -I scripts/gate-scenarios.py --write
```

`--check` fails when the file does not agree (`check.sh` and the job
`workflows` of `ci.yml` run it). A new command needs a new line `PATH |`
first; a test of `src/cli/help_tests.rs` checks that each command has a
line. `--notes LANE.TXT` writes the section "The scenarios of each command"
of the notes of a release: the experimental commands, and each other
command with its scenarios. A lane scenario counts only when a row of its
case passed in LANE.TXT. The job `publish` of `release.yml` adds it after
the section of the gate.

### The network labs

`podlab` of podcrates `v0.60.0` or later has network labs. Each lab is a
network namespace with no route out, with its own stand-in proxy on
`127.0.0.1:3128` (or no proxy), a TLS stand-in server that answers each
name, and `SSL_CERT_FILE`. The lane runs these commands in the labs
`proxy-only`, `proxy-tls-reterminate`, `proxy-per-name`,
`ws-upgrade-refused`, `sni-allowlist`, `no-proxy-no-dns` and `ipv6-only`:

* `version` and `doctor`. The doctor must name the proxy of the lab:
  `proxy  http://127.0.0.1:3128 (credentials: present)` in `proxy-only`,
  `(credentials: none)` in the other proxy labs, and `proxy  none: each
  connection is direct` in a lab with no proxy.
* `net-curl`: no podnet. `curl https://lab.test/` gets HTTP 200 through
  the proxy of the lab (the control of the lab).
* `net-pod-auto`, `net-pod-seccomp` and `net-pod-netns`: `podnet pod
  --backend BACKEND -- curl https://lab.test/` with the proxy of the lab
  from the environment. The program must get HTTP 200.

In a lab with no proxy, `net-curl` is n.a., and a `net-pod` command is
n.a. when `podnet pod` refuses with `no proxy: give -x PROXY`. The plant
`LANE_PLANT=net` gives podnet a proxy that does not listen: each `net-pod`
command must fail. `ipv6-only` does not hold on a kernel with no IPv6.

### The row allow-connect

The row `allow-connect` is not a profile of `pod-lab`. The lane runs
it in `podlab run --allow-connect PORT bind-denied` (podlab
`v0.58.0` or later): Landlock refuses each TCP connect, except a connect to
PORT. Outside the lab, a stand-in HTTP CONNECT proxy
(`tests/consumers/connect-proxy.py`) listens on `127.0.0.1:PORT`. It writes
each request line to `LANE_DIR/proxy.txt`, and connects only
`proxy.test:PORT+1`, to a server on `127.0.0.1:PORT+1` with the body
`podnet-proxy-only`. It refuses each other target with HTTP 403. PORT is
`LANE_PROXY_PORT` (default 7120). Run the row alone with
`sh scripts/lab-lane.sh PODLAB PODNET allow-connect`.

The commands:

* `proxy-direct`: no podnet. A GET of `http://127.0.0.1:PORT+1/` must
  fail with `Permission denied` (the negative control).
* `proxy-port`: no podnet. A CONNECT through the proxy gets the body (the
  control of the lab: the proxy port is open).
* `proxy-pod-auto` and `proxy-pod-seccomp`: `podnet pod [--backend
  seccomp] -x http://127.0.0.1:PORT -- python3 client.py urllib
  http://proxy.test:PORT+1/`.
* `proxy-pod-env`: `podnet pod` with no `-x`, and `HTTPS_PROXY`,
  `HTTP_PROXY` and `NO_PROXY=localhost,127.0.0.1` as a proxy-only host
  sets them.
* `proxy-doctor`: `podnet doctor` with `HTTPS_PROXY`. It must exit 0 or
  1 with no panic, the proxy must log its CONNECT requests, and the doctor
  must name the HTTP 403 of the proxy.

A pod command passes when the pod exits 0, the program printed the body,
and the proxy logged `CONNECT proxy.test:PORT+1`. `LANE_PLANT` plants a
defect: `refuse` (the proxy refuses each CONNECT) or `port` (podnet gets
the port PORT+2, which the lab refuses). With each plant, each pod command
must fail.

## The consumer .NET HttpClient

`scripts/consumer-dotnet.sh` runs the real use of a consumer (T-358, step
3): the HTTP client of the runner of PodUtils/podrunner, under the vnet
shim. The client is `tests/consumers/dotnet-http` (.NET 8, no NuGet
package). It sends GET to `http://127.0.0.1:PORT/` with the default
`SocketsHttpHandler`, which opens dual-mode AF_INET6 sockets.

1. Install the SDK: `apt-get install -y -qq dotnet-sdk-8.0`.
2. Build the release binary of podnet (`docs/build.md`), and `podlab`
   (above) for the lab.
3. Run `sh scripts/consumer-dotnet.sh PODNET [PODLAB [PROFILE]]`. The
   default PROFILE is `bind-denied`. As root, the lab cases run as uid
   65534.

The script builds the client, serves a broker on a unix socket and a TCP
server on `127.0.0.1:PORT` with another body, and builds the shim with
`--forward 127.0.0.1:PORT=unix:PATH`. The cases:

* `direct`: no lab, no shim. The client gets the body of the TCP server.
* `shim`: no lab, the shim. The client gets the body of the broker.
* `lab-bare`: the lab, no shim. The connect fails with `Permission denied`.
* `lab-shim`: the lab, the shim. The client gets the body of the broker.

A shim case also checks the shim log (`connect -> forward`) and the peer
of the AF_UNIX connection of the broker (`SO_PEERCRED`: the `dotnet`
process). `CONSUMER_SHIM=PATH` takes another shim: the job `static` of
`ci.yml` plants a shim with no match of the IPv4-mapped address, and the check must fail. CI runs the cases with no lab.

## The consumer Python HTTP client

`scripts/consumer-python.sh` runs a Python HTTP client under the vnet shim
(T-358, step 3). The client is `tests/consumers/python-http/client.py`:
`urllib.request` of the standard library, and `requests` when `python3`
can import it (the script copies the package for uid 65534). The servers
are `tests/consumers/servers.py`.

1. Build the release binary of podnet (`docs/build.md`), and `podlab`
   (above) for the lab.
2. Run `sh scripts/consumer-python.sh PODNET [PODLAB [PROFILE]]`. The
   default PROFILE is `bind-denied`. As root, the lab cases run as uid
   65534.

The script serves a broker and an HTTP proxy on unix sockets, and a decoy
TCP server with another body on each port. The shim has the forwards
`127.0.0.1:PORT` to the broker and `127.0.0.1:PORT+1` to the proxy. The
cases, for each library:

* `direct`: no lab, no shim. The client gets the body of the decoy.
* `shim`: no lab, the shim. The client gets the body of the broker.
* `proxy`: no lab, the shim and `HTTP_PROXY=http://127.0.0.1:PORT+1`. The
  client gets the body of the proxy, and the proxy gets the request line
  `GET http://consumer.test/`.
* `lab-bare`: the lab, no shim. The connect fails with `Permission denied`.
* `lab-shim` and `lab-proxy`: the lab and the shim, as `shim` and `proxy`.

A shim case also checks the shim log (`connect -> forward`) and the peer
of the AF_UNIX connection (`SO_PEERCRED`: the `python3` process). The job
`static` of `ci.yml` runs the cases with no lab, and a planted shim with no
match of a forward, which must fail with `Connection refused`.

## The consumer pod: podnet pod around a dynamic program

`scripts/consumer-pod.sh` runs `podnet pod` around a dynamic program in
the labs of `pod-lab` (T-358, step 3). It is a local command: CI does not
build `podlab`.

1. Build the release binary of podnet and `podlab` (above).
2. Run `sh scripts/consumer-pod.sh PODNET PODLAB [PROFILE...]`. With no
   PROFILE, it runs in `bind-denied`, `no-net-admin`, `no-tun` and
   `udp-denied`. As root, each lab case runs as uid 65534.

Outside the lab, a SOCKS5 server on a unix socket is the upstream of the
pod. It connects only `consumer.test:PORT`, to a server with the body
`podnet-consumer-pod`. A decoy on `127.0.0.1:PORT` has another body. The
programs are `python3` (`urllib`) and `curl` when it exists. Each one sends
GET to `http://consumer.test:PORT/` and exits 42 when it got the body. The
rows:

* `control`: no lab, no pod. The program gets the decoy.
* `bare`: the lab, no pod. In `bind-denied`, the connect must fail with
  `Permission denied`. In another lab, the row only says what the lab does.
* `auto`, `netns`, `seccomp` and `netns-kernel`: `podnet pod --backend
  BACKEND -x socks5://unix:PATH -- PROGRAM` in the lab. A row passes when
  the pod exits 42, the program got the body, and the upstream got the
  CONNECT of `consumer.test:PORT`. A row is n.a. only when podnet exits 3
  with a reason, in a lab that takes what the backend needs by design. A
  failure with an entry of the record says `fail, known: T-NNN`.

`CONSUMER_PLANT=decoy` makes the upstream send the name to the decoy: each
pod row must fail.

## Rules for tests

* **A test for each fix.** A fix of a defect comes with a test that fails
  on the code before the fix. Run the new test on the old code one time
  (the control), and write in the commit message that it fails there.
* **A planted defect for each check.** A new check (a test, a CI step, a
  scan) must fail on a planted defect before it is trusted. Examples: the
  secret scan of `secrets.yml` scans a planted key first; the HTTP/2 data
  frame test fails with no `data_frame_budget`.
* **No sleep as a proof.** A test does not wait a fixed time and then
  assert that a thing did not occur. It waits for an event (a channel, a
  socket count) with a time limit. For time limits of podnet (backoff,
  expiry, budgets), use `#[tokio::test(start_paused = true)]` and a
  paused clock, or make the limit a parameter.
* **Loopback only.** A unit or integration test uses loopback servers
  (`127.0.0.1:0`, a Unix socket in a temporary directory). It does not
  use the internet. The `live.yml` workflow does the live checks.
* **The quick tunnel has a stand-in.** `tests/standin/` holds a stand-in of
  the quick-tunnel service and of the HTTP/2 leg of the Cloudflare edge
  (`mod standin;`). Use it for a test of podnet that needs a tunnel: it
  makes no call to Cloudflare. It uses `--service-url`, `PODNET_EDGE` and a
  test CA in `SSL_CERT_FILE`. Cloudflare does not use it. It cannot prove
  that the real edge accepts a change: `live.yml` keeps one real quick
  tunnel for each leg. The design is in
  `docs/research/podnet/cf-edge-standins.md`.
* **Skip on a host with no loopback.** A host can refuse
  `bind(AF_INET)`, have no loopback interface, or not resolve
  `localhost` (issue 217). A test that needs TCP or UDP on `127.0.0.1`
  starts with `crate::test_host::need_loopback!();` (in `src/`) or
  `test_host::need_loopback!();` (in `tests/`,
  after `mod test_host;`). A test that resolves `localhost` starts with
  `need_localhost!()`. The probe is `tests/test_host/mod.rs`. On such a
  host the test prints `skipping, REASON` and returns.
* **The restricted-host check.** `sh scripts/restricted-host.sh` runs
  `cargo test --workspace` with `bind()` of `AF_INET` and `AF_INET6`
  refused (EACCES) in each test binary (`scripts/deny-bind.c`). The CI job
  `restricted-host` runs it. A new test that binds a loopback socket
  and has no skip fails there. For the case of no loopback, run the test
  binary in a new network namespace:
  `unshare -Urn target/debug/deps/podnet-HASH` (the loopback is down there).
* **No shared state.** A test makes its own temporary directory
  (`tempfile`) and sets `PODNET_STATE_DIR` or the paths that it needs. It
  does not write in the `HOME` of the host.
* **No secret in a test.** A test token or password is a made-up value. A
  URL with a user part in a test uses a loopback port with no server. Add
  each finding of the secret scan of such a value to
  `.github/secrets-allow.txt`, with the reason.

## Random failures

* A test that fails one time in many runs has a defect: in the test, or in
  podnet. Do not run CI again until it is green, and do not mark the test
  ignored to get green.
* Reproduce it: run the test in a loop
  (`for i in $(seq 50); do cargo test NAME -- --exact || break; done`),
  or with less CPU (`taskset -c 0`).
* Find the race. Common causes: an assert on a sleep, a port that another
  test took, a shared file, an order of two tasks that the test takes as
  fixed.
* Fix the cause, and give the loop count of the proof in the commit
  message.

## Commits

* The subject is `area: text`, for example `relay node: expiry warnings`.
  `cliff.toml` groups the changelog by the area.
* The body says what changed and why, and names the tests.
* Write in ASD-STE100 Simplified Technical English.
