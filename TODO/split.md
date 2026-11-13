This file holds the entries of milestone M2: the splits of the large files inside their crates,
and the cuts of the 18 upward references. `docs/research/podnet/architecture.md`, sections 2 and
3.3, names each new module and the items that move into it. The rules of each split:

- Move code verbatim. Change only `use` lines and visibility (`pub(super)`, `pub(crate)`).
- Keep each public path with `pub use` in the parent module. Use the `mod.rs` form. Do not name a
  child module as its parent (clippy `module_inception`).
- Move a test module to a child file `tests.rs`, with `#[cfg(test)] mod tests;`.
- In each commit: `sh scripts/agent/check.sh`, the same count of tests, an empty diff of the
  parity snapshot (T-008), the size check (T-009) with the file removed from its allowlist, and
  the paths of the documents moved (`tests/docs.rs::each_path_in_the_documents_exists`).
- A commit that changes `src/warp/`, `src/tunnel.rs`, `src/edge_h2.rs`, `src/proxy.rs`,
  `src/relay_reverse.rs`, `src/ws_relay.rs` or the pod crate also passes a run of `live.yml`:
  these commands have no loopback test of the command.

The commands of a Prove below are those of each commit of its entry:

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
cargo test --workspace -- --list 2>/dev/null | grep -c ': test$'
```

# T-026: Move the unit tests of nine files into their own files

**Source:** `docs/research/podnet/architecture.md`, sections 1.1 and 2.3.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Nine files are over 800 lines only because of their test module: `src/config.rs`,
`src/cloudflare/http2.rs`, `crates/podnet-vnet/src/switch.rs`, `src/warp/up.rs`,
`crates/podnet-netns/src/upstream.rs`, `src/ws_relay.rs`, `src/cloudflare/capnp.rs`,
`src/warp/register.rs` and `src/quicktunnel.rs`.

## Premise

- `docs/research/podnet/architecture.md`, section 1.1: each of the nine has fewer than 800 lines
  of code.
- A child module `tests` keeps `use super::*;` and its access to the private items of its parent.

## Approach

1. For each file, move the body of its test module into a child file `tests.rs`. Change nothing
   else.
2. Remove the nine rows from the allowlist of the size check.
3. Make one commit for the nine files.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
cargo test --workspace -- --list 2>/dev/null | grep -c ': test$'
```

The count equals the count before the commit.

## Start condition

T-008 and T-009 are done.

## Correction

2026-11-05, The named modules `src/config.rs`, `src/cloudflare/http2.rs`, `src/cloudflare/capnp.rs`, `src/quicktunnel.rs`, `src/warp/up.rs`, `src/warp/register.rs` and `src/ws_relay.rs` move to PodUtils/podcrates (`pod-cloudflare`, `pod-warp` and `pod-relay`, T-137). podnet does not split them: a split of a moved file is work of podcrates, under its checks. Split only the files that stay in podnet: `crates/podnet-vnet/src/switch.rs` and `crates/podnet-netns/src/upstream.rs`. Remove the row of a moved file from `scripts/size-allow.txt` when podnet takes its crate.

## Done

2026-11-06, commit 1a724fc (`T-026: the tests of crates/podnet-vnet/src/switch.rs and podnet-netns upstream.rs in
child files`). The test modules of `crates/podnet-vnet/src/switch.rs` and `crates/podnet-netns/src/upstream.rs`
are in `switch/tests.rs` and `upstream/tests.rs`; both files left `scripts/size-allow.txt` (711 and
773 lines). `scripts/agent/parity-snapshot.sh`: 1,020 tests before and after, none lost. The other
seven files move to podcrates (the Correction). CI run 42283721494 at 093938f: success.


# T-027: `src/main.rs` to a CLI module, part 1: the signals, the help, the argument rewrites and the shared options

**Source:** `docs/research/podnet/architecture.md`, section 2.1.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`src/main.rs` has 7,103 lines: 6,455 of code and 649 of tests. Its `warp()` function alone has
1,089 lines. One file holds the parse, the dispatch and each command.

## Premise

- The binary can declare a module that the library does not declare. Thus the new CLI module has
  no public path, and the library surface does not change.
- `docs/research/podnet/architecture.md`, section 2.1, gives each new file and the items that it
  takes, read at `3bed023`.

## Approach

1. Make the module src/cli/ in the binary, with `Cli`, `Command` and the dispatch of
   `async_main` in its mod.rs.
2. Move `STOP`, `signal_ignored`, `install_signals`, `shutdown_signal` and the sleep helpers to
   src/cli/signals.rs.
3. Move `HELP_GROUPS`, `START_HERE`, the help helpers, `json_wanted`, `error_json` and
   `print_error` to src/cli/help.rs.
4. Move `cloudflared_compat`, the token-file helpers, `SERVER_COMMANDS`, `foreground_hint`,
   `command_name` and the pod argument helpers to src/cli/argv.rs.
5. Move `TunnelOpts`, `Show`, `DetachArgs`, `detach_of` and `network_options` to src/cli/opts.rs.
6. Make one commit for each new file.

## Prove

```sh
sh scripts/agent/check.sh
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
cargo test --workspace -- --list 2>/dev/null | grep -c ': test$'
```

The parity diff is empty. The count of tests does not change.

## Start condition

T-008, T-009 and T-011 are done.

## Done

2026-11-06, commits 968d130 (signals), 0cb193a (help and argv), 4493510 (opts) and e3bf47a
(`T-027: move Cli, Command and the dispatch of src/main.rs to src/cli/mod.rs`). The binary
declares `mod cli;`; the library does not. `src/cli/signals.rs` (131 lines), `help.rs` (246),
`argv.rs` (293), `opts.rs` (284) and `mod.rs` (414: `Cli`, `Command`, `async_main`) hold the
items of the Approach. `src/main.rs` has 5,645 lines (6,939 before). `scripts/check-layers.py`
gives the module `cli` the layer podnet. The tests stay in `src/main.rs` with their names (they
move with T-028 to T-032). `scripts/agent/parity-snapshot.sh`: exit 0, no map; 1,285 commands,
1,278 public items and 1,020 tests before and after. Help and argv are in one commit: help uses the
items of argv (the commit order of architecture.md, section 2.1).

CI run 42286291297 at 8a3ec2c: success.

# T-028: `src/main.rs` to a CLI module, part 2: the host commands and the `net` commands

**Source:** `docs/research/podnet/architecture.md`, section 2.1.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The host commands (`ps`, `stop`, `logs`, `autostart`, `agent`, `status`, `doctor`, `man`,
`update`, `ports`, `qr`, `bug-report`, `completions`) and the `net` commands are in
`src/main.rs`.

## Premise

`docs/research/podnet/architecture.md`, section 2.1, gives the target files: instances,
autostart, agent and host under src/cli/cmd/, and a directory net/ with its proxy and shim parts.

## Approach

1. Move `instances_ps`, `instances_stop` and `instances_logs` to an instances file.
2. Move `AutostartArgs`, `AutostartCommand`, `autostart_args` and `autostart` to an autostart
   file.
3. Move `AgentCommand`, `run_self_json`, `agent`, `license_fault`, `exposes_failed` and
   `agent_commands` to an agent file.
4. Move `DoctorArgs`, `ManArgs`, `UpdateArgs`, `parse_release_tag`, `PortsArgs`, `QrArgs` and the
   arms of the other host commands to a host file.
5. Move `NetArgs`, `NetCommand`, `net()`, `probability`, `net_proxy`, `RealTarget`,
   `DialStream`, `net_shim`, `net_tailscale`, `print_environment`, `net_socksify` and
   `net_ts_shims` to the net directory.

## Prove

```sh
sh scripts/agent/check.sh
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
```

## Start condition

T-027 is done.

## Done

2026-11-06, commit db1e2a3 (`T-028: move the host commands and the net commands of src/main.rs
to src/cli/cmd/`). `src/cli/cmd/mod.rs` declares the files below. `instances.rs` (163 lines:
`instances_ps`, `instances_stop`, `instances_logs`, and the arm `wait` as `instances_wait`),
`autostart.rs` (144), `agent.rs` (232: `AgentCommand`, `run_self_json`, `agent`,
`agent_commands`), `host.rs` (280: the argument structs, `parse_release_tag`, and one function for
each arm of `update`, `completions`, `ports`, `qr`, `man`, `doctor`, `status` and `bug-report`),
`net/mod.rs` (330), `net/proxy.rs` (182) and `net/shim.rs` (157). `license_fault` and
`exposes_failed` were already in the library (`src/warp/identity.rs`, `src/agent.rs`); they stay
there. The dispatch in `src/cli/mod.rs` calls the new functions. `src/main.rs` has 4,441 lines
(5,645 before). The tests stay in `src/main.rs` with their names (T-032 moves them).
`scripts/agent/parity-snapshot.sh`: exit 0, no map; 1,285 commands, 1,278 public items and 1,020
tests before and after. One commit holds all the files: the net files and the host files change
the same `use` lines of `src/main.rs` and `src/cli/mod.rs`.

CI run 42290515084 at b385489: success.

# T-029: `src/main.rs` to a CLI module, part 3: the server commands

**Source:** `docs/research/podnet/architecture.md`, section 2.1.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The server commands (`tunnel`, `serve`, `share`, `demo`, `connect`, `relay`) and their shared run
loop are in `src/main.rs`.

## Premise

`docs/research/podnet/architecture.md`, section 2.1, gives the target files tunnel, run, share,
connect and relay under src/cli/cmd/.

## Approach

1. Move `TunnelArgs`, `tunnel`, `preflight`, `split_host_port` and `pick_optional_service` to a
   tunnel file.
2. Move `run_tunnel`, `run_share`, `run_demo`, `serve_until`, `tunnel_doc`, `listen_doc`,
   `verified`, `announce`, `announce_text`, `wait`, `idle_watch`, `wait_with_idle` and
   `wait_controls` to a run file.
3. Move `ServeArgs`, `ModeArg`, `ShareArgs`, `DemoArgs`, `serve`, `serve_local`, `share_mode`,
   `share`, `demo` and `default_upload_dir` to a share file.
4. Move `ConnectArgs`, `connect`, `accept_loop`, `SocketFile`, `file_id`, `unix_listener` and
   `accept_unix` to a connect file.
5. Move `RelayArgs`, `RelayCommand`, `relay`, `RelayIo` and `non_empty_key` to a relay file.

## Prove

```sh
sh scripts/agent/check.sh
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
gh workflow run live.yml
```

## Start condition

T-027 is done.

## Correction

Approach step 5 moves `non_empty_key` to the relay file. `non_empty_key` is the value parser of
the license key of `warp` (`WarpCommand`); `relay` does not use it. It stays in `src/main.rs`, and
it moves with the WARP arguments (T-030).

## Done

2026-11-06, commit 77782e3 (`T-029: move the server commands of src/main.rs to src/cli/cmd/`).
`src/cli/cmd/tunnel.rs` (259 lines: `TunnelArgs`, `tunnel`, `preflight`, `split_host_port`,
`pick_optional_service`), `run.rs` (352: the run loop, `listen_doc`, `verified`, `announce`,
`announce_text`, the waits, `idle_watch` and `_mode`), `share.rs` (393: `ServeArgs`, `ModeArg`,
`ShareArgs`, `DemoArgs`, `serve`, `serve_local`, `share_mode`, `share`, `demo`,
`default_upload_dir`), `connect.rs` (264: `ConnectArgs`, `connect`, the accept loops,
`SocketFile`, `file_id`, `unix_listener`) and `relay.rs` (237: `RelayArgs`, `RelayCommand`,
`relay`, `RelayIo`). The dispatch in `src/cli/mod.rs` and `detach_of` in `src/cli/opts.rs` use
the new paths. `src/main.rs` has 3,018 lines (4,441 before). The tests stay in `src/main.rs` with
their names (T-032 moves them). `scripts/agent/parity-snapshot.sh`: exit 0, no map; 1,285
commands, 1,278 public items and 1,020 tests before and after. `gh workflow run live.yml` is not
done here: the commit is not on `main` yet. Run it after the merge.

CI run 42292498940 and live run 42292490990 at 416574b: success.

# T-030: `src/main.rs` to a CLI module, part 4: the WARP arguments, the client and `warp up`

**Source:** `docs/research/podnet/architecture.md`, section 2.1.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`WarpArgs` and `WarpCommand` have 543 lines, and `warp()` has 1,089. The arms of `up`, `down`,
`env`, `exec`, `shell`, `ssh-config` and `nc` share helpers with `warp up`.

## Premise

`docs/research/podnet/architecture.md`, section 2.1: the largest new file is the WARP arguments
file (about 545 lines), under 800.

## Approach

1. Move `WarpArgs` and `WarpCommand` to an args file of a warp/ directory under src/cli/cmd/.
2. Move the prologue of `warp()` and the dispatch to the mod.rs of that directory.
3. Move `warp_client`, `carry_peer`, `wireguard_peer`, `masque_options`, `identity_pairs` and
   `warp_client_inner` to a client file.
4. Move the arms `Up`, `Down`, `Env`, `Exec`, `Shell`, `SshConfig` and `Nc`, with `UpFiles`,
   `warp_up`, `up_summary`, `udp_nc`, `ssh_config_file` and the `warp up` argument helpers, to
   an up file.

## Prove

```sh
sh scripts/agent/check.sh
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
gh workflow run live.yml
```

## Start condition

T-027 is done.

## Correction

Approach step 2 moves the dispatch to `src/cli/cmd/warp/mod.rs`. The arms of T-031 (`Register`
to `Run`, about 790 lines) cannot go into that file with the prologue: the file then has more
than 800 lines. They stay in `src/main.rs` in the function `warp_other`, and `warp()` of
`mod.rs` calls `crate::warp_other` after it makes the client. T-031 moves these arms to their
files and removes `warp_other`.

## Done

2026-11-06, commit 23759ba (`T-030: move the WARP arguments, the client and warp up to
src/cli/cmd/warp/`). `src/cli/cmd/warp/args.rs` (554 lines: `WarpArgs`, `WarpCommand`,
`non_empty_key`), `client.rs` (147: `warp_client`, `carry_peer`, `wireguard_peer`,
`masque_options`, `identity_pairs`, `warp_client_inner`), `up.rs` (438: the arms `Up`, `Down`,
`Env`, `Exec`, `Shell`, `SshConfig` and `Nc` as `before_client` and `nc_session`, `UpFiles`,
`warp_up`, `UDP_NC_IDLE`, `nc_end`, `udp_nc`, `ssh_config_file`, `warp_global_args`,
`warp_up_tun_args`, `warp_up_args`) and `mod.rs` (153: the prologue of `warp()`, the dispatch,
the check of the saved identity). `up_summary` was already in the library
(`src/warp/find.rs`); it stays there. The arms `Forward --list`, `Export` and the checks of
`Proxy` before the client stay in the dispatch of `mod.rs` (T-031 moves them). `src/main.rs`
has 1,810 lines (3,018 before). The tests stay in `src/main.rs` with their names (T-032 moves
them). `sh scripts/agent/check.sh`: exit 0. `scripts/agent/parity-snapshot.sh`: exit 0, no
map; 1,285 commands, 1,278 public items and 1,020 tests before and after. `gh workflow run
live.yml` is not done here: the commit is not on `main` yet. Run it after the merge.

# T-031: `src/main.rs` to a CLI module, part 5: the other WARP subcommands

**Source:** `docs/research/podnet/architecture.md`, section 2.1.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The arms `register`, `status`, `license`, `rotate`, `delete`, `scan`, `exit`, `probe`, `export`,
`proxy`, `forward`, `tun` and `run` stay in `warp()` after T-030.

## Premise

- `docs/research/podnet/architecture.md`, section 2.1, gives the target files account, probe,
  export, front, tun and run.
- Most of these commands have live steps only. T-012 gives some of them loopback tests.

## Approach

1. Move the arms `Register`, `Status`, `License`, `Rotate` and `Delete`, with `print_device`,
   `plain_value` and `warp_plus_label`, to an account file.
2. Move the arms `Scan`, `Exit` and `Probe`, with `parse_door` and `exit_line`, to a probe file.
3. Move the arm `Export` to an export file, the arms `Proxy` and `Forward` to a front file, the
   arm `Tun` to a tun file, and the arm `Run` with `warp_run_raw` to a run file.

## Prove

```sh
sh scripts/agent/check.sh
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
gh workflow run live.yml
```

## Start condition

T-030 is done.

## Done

2026-11-09, commit 7e5fe8d (`T-031: move the other WARP subcommands to src/cli/cmd/warp/`).
`src/cli/cmd/warp/account.rs` (139 lines: the arms `Register`, `Status`, `License`, `Rotate` and
`Delete`, `plain_value`, `print_device`), `probe.rs` (220: `Scan`, `Exit`, `Probe`,
`parse_door`, `exit_line`), `export.rs` (15: `Export` before the client), `front.rs` (322:
`Proxy` and `Forward`, with the checks before the client: `forward --list`, `proxy --check` and
`proxy --listen-identity`), `tun.rs` (190: `Tun`), `run.rs` (153: `Run`, `warp_run_raw`) and
`mod.rs` (143: the dispatch to each file). `warp_other` is removed. `warp_plus_label` was already
in the library (`src/warp/identity.rs`); it stays there. `src/main.rs` has 910 lines (1,810
before). The tests stay in `src/main.rs` with their names (T-032 moves them). `sh
scripts/agent/check.sh`: exit 0. `scripts/agent/parity-snapshot.sh`: exit 0, no map; 1,285
commands, 1,278 public items and 1,019 tests before and after. The live run is dispatched by the
lead after the merge.

# T-032: `src/main.rs` to a CLI module, part 6: the unit tests go to the modules that they test

**Source:** `docs/research/podnet/architecture.md`, section 2.1 (the row "tests").
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

After T-027 to T-031, the 649 lines of tests of `src/main.rs` still test code in other files.

## Premise

`docs/research/podnet/architecture.md`, section 2.1, gives the target of each group: the help
tests, the argument tests, the share tests, the WARP tests, the agent tests, and the tests of the
manual (`the_command_tree_is_valid`, `each_example_of_the_manual_parses`,
`the_manual_names_each_command_and_flag`).

## Approach

1. Move each test to a test module of the file that holds the code that it tests. Keep each test
   name.
2. Keep in `src/main.rs` the print macros, the `use` lines and `main()`: about 300 lines.
3. Remove `src/main.rs` from the allowlist of the size check.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
cargo test --workspace -- --list 2>/dev/null | grep -c ': test$'
```

## Start condition

T-028, T-029, T-030 and T-031 are done.

## Done

2026-11-09, commit ac018b1 (`T-032: move the unit tests of src/main.rs to the modules that they
test`). The 24 tests and the helper `share_args` moved with their names to test modules:
`src/cli/help_tests.rs` (the help, the hints, the manual: 9 tests), `src/cli/argv_tests.rs` (4),
`src/cli/signals_tests.rs` (2), `src/cli/cmd/run_tests.rs` (1), `share_tests.rs` (1),
`agent_tests.rs` (1), `tunnel_tests.rs` (1) and `src/cli/cmd/warp/cli_tests.rs` (5: the WARP tests
stay in the CLI, so `src/warp/` has no change). Each is declared with `#[cfg(test)]` in the
`mod.rs` beside it. `src/main.rs` has 281 lines (910 before): the print macros, the `use` lines
and `main()`. The line of `src/main.rs` is out of `scripts/size-allow.txt`. The record text that
named the lines of the moved tests is updated. `sh scripts/agent/check.sh`: exit 0.
`scripts/agent/parity-snapshot.sh`: 1,295 commands, 1,302 public items and 1,100 tests before and
after; the compare needs a map of the 24 test paths (`tests::NAME` to `cli::...::NAME`), and
reports no lost command, item or test. CI: at the merge.


# T-033: Split `src/egress.rs`, `crates/podnet-core/src/detach.rs`, `src/update.rs` and `src/doctor.rs`

**Source:** `docs/research/podnet/architecture.md`, section 2.2 (the four rows).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`src/egress.rs` has 1,707 lines, `crates/podnet-core/src/detach.rs` 1,210, `src/update.rs` 1,162 and `src/doctor.rs`
1,309.

## Premise

- The fuzz targets use `egress::{Proxy, NoProxy, parse_status}` (`src/fuzzing.rs`).
- `crates/podnet-core/src/detach.rs` has two inline modules, `supervise` and `unix`.

## Approach

1. `src/egress.rs`: a mod.rs with the choice and the dial functions; files for `Proxy`, the HTTP
   CONNECT client, the SOCKS5 client, `NoProxy` and the `+exec:` carrier. Keep the `pub use` of
   the items of the fuzz targets.
2. `crates/podnet-core/src/detach.rs`: a mod.rs with the record functions; files for the start functions, for
   `supervise`, for `unix` and for the tests.
3. `src/update.rs`: a mod.rs with `Release`, `get`, `run` and `Outcome`; files for the cosign
   checks, for the install and rollback, and for the tests.
4. `src/doctor.rs`: a mod.rs with `Options`, `run` and `Report`; files for the targets, the path,
   the TLS check, the clock and the legs.
5. Make one commit for each file.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
cargo check --manifest-path fuzz/Cargo.toml --bins
```

## Start condition

T-008 and T-009 are done.

## Correction

2026-11-05, The named modules `src/egress.rs` move to PodUtils/podcrates (`pod-proxy`, T-137). podnet does not split them: a split of a moved file is work of podcrates, under its checks. Split only the files that stay in podnet: `crates/podnet-core/src/detach.rs`, `src/update.rs` and `src/doctor.rs`. Remove the row of a moved file from `scripts/size-allow.txt` when podnet takes its crate.

## Done

2026-11-09, commits d0a6df0 (detach), a8e648d (update) and 9d9b2ed (doctor). `src/egress.rs` is not
split (the Correction). `crates/podnet-core/src/detach/` has `supervise.rs`, `unix.rs` and `tests.rs` (`crates/podnet-core/src/detach.rs`
759 lines). `src/update/` has `cosign.rs` and `tests.rs` (`src/update.rs` 642 lines; the public
paths stay by `pub use`). `src/doctor/` has `clock_check.rs`, `tls_check.rs` and `tests.rs`
(`src/doctor.rs` 754 lines); the allow list of `trust::tests::no_other_code_sets_its_own_verifier`
names `src/doctor/tls_check.rs`. Three rows left `scripts/size-allow.txt`. Citations of the record
moved. `scripts/agent/parity-snapshot.sh`: 1,295 commands, 1,293 items and 1,108 tests before and
after. `check.sh` exit 0 at each commit; `unshare -n -- cargo test --offline --workspace` exit 0;
`scripts/restricted-host.sh` exit 0. CI: at the merge.

# T-034: Split `crates/podnet-vnet/src/stack.rs`

**Source:** `docs/research/podnet/architecture.md`, section 2.2.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`crates/podnet-vnet/src/stack.rs` has 2,103 lines: 1,882 of code and 222 of tests.

## Premise

The library exports `NetStack` and its types (`crates/podnet-vnet/src/lib.rs`, `tests/api_surface.rs`).

## Approach

1. Keep `NetStack`, `StackConfig`, `StackEvent`, `Clock` and the re-exports in the mod.rs of a
   stack directory.
2. Move `TcpStream`, `UdpSocket` and `SocketCount` to a socket file; `Core::new`, `listen`,
   `connect` and `bind_udp` to a core file; `service_dns`, `accept_inbound`, `service_conns` and
   `service_udp` to a service file; the run loop and its timers to a run file; `ProofReport` and
   the loopback proofs to a proof file; the tests to `tests.rs`.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
cargo test --test vnet_regressions --test vnet_proxy_forward --test tcp_window
```

## Start condition

T-008 and T-009 are done.

## Done

2026-11-09, commit 8cb6ea8. `crates/podnet-vnet/src/stack/` has `reactor.rs` (the file of `Core::new`, `listen`,
`connect`, `bind_udp`; not named `core.rs`, because a module `core` hides the `core` crate),
`service.rs` (`service`, `service_dns`, `accept_inbound`, `service_conns`, `service_udp`),
`socket.rs` (`TcpStream`, `UdpSocket`, `TcpListener`, `SocketCount`), `proof.rs` (`ProofReport`
and the loopback proofs) and `tests.rs`. `crates/podnet-vnet/src/stack.rs` has 467 lines; its row left
`scripts/size-allow.txt`. The public paths stay by `pub use`. The run loop stays in `stack.rs`
beside `NetStack`. Citations of the record moved. Parity: 1,295 commands, 1,293 items and 1,108
tests before and after. `check.sh` exit 0 (it runs `vnet_regressions`, `vnet_proxy_forward` and
`tcp_window`). CI: at the merge.

# T-035: Split `src/warp/ip.rs`, `src/warp/session.rs` and `src/warp/wg.rs`

**Source:** `docs/research/podnet/architecture.md`, section 2.2 (the three rows).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`src/warp/ip.rs` has 1,680 lines, `src/warp/session.rs` 1,211 and `src/warp/wg.rs` 1,078.

## Premise

The three files are the data path of WARP: the userspace stack, the MASQUE session and the
WireGuard leg. Their commands have live steps only, or the stand-ins of T-012.

## Approach

1. `src/warp/ip.rs`: `WarpStack` and `StackConfig` in a mod.rs; files for the connections
   (`TcpConn`, `UdpConn`), the driver, the interface and the tests.
2. `src/warp/session.rs`: the session in a mod.rs; a file for `PinnedKey` and `Keepalive`; the
   tests to `tests.rs`.
3. `src/warp/wg.rs`: the peer and the transport in a mod.rs; files for BLAKE2s with its HMAC and
   HKDF helpers, for the Noise handshake, and for the tests.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
gh workflow run live.yml
```

## Start condition

T-008, T-009 and T-012 are done.

## Correction

2026-11-05, The named modules `src/warp/ip.rs`, `src/warp/session.rs` and `src/warp/wg.rs` move to PodUtils/podcrates (`pod-warp`, with `pod-netstack` and `pod-wg`, T-137). podnet does not split them: a split of a moved file is work of podcrates, under its checks. No file of this entry stays in podnet. Remove the row of a moved file from `scripts/size-allow.txt` when podnet takes its crate.

## Done

2026-11-13: done by the take of `pod-warp` (podcrates `v0.37.0`, f6abdc3; T-050): the WARP chain moved to the crate, and
`src/warp/ip.rs` (152 lines), `src/warp/session.rs` (72) and `src/warp/wg.rs` (70) are glue under 800 lines. No split in podnet was needed. Check: `python3 scripts/check-size.py` (0 faults).


# T-036: Split `src/warp/mod.rs` and `src/warp/proxy.rs`

**Source:** `docs/research/podnet/architecture.md`, section 2.2 (the two rows).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`src/warp/mod.rs` has 1,105 lines and `src/warp/proxy.rs` 1,991.

## Premise

- `src/warp/mod.rs` holds `WarpClient`, the session openers and the private-file helpers with
  their tests.
- `src/warp/proxy.rs` holds the SOCKS5 front door, its DNS resolver, the UDP relay and the route
  of each flow.

## Approach

1. `src/warp/mod.rs`: keep the modules, the re-exports and `WarpClient`. Move the session openers
   to a client-session file, and the private-file helpers with their tests to a private-file
   file.
2. `src/warp/proxy.rs`: a proxy directory with a mod.rs (`bind`, `admit`, `run`, `run_l4`,
   `WarpProxy`, `health_log`) and files for the DNS part, the SOCKS5 client handshake, the UDP
   association, the route and the tests. Do not name a file proxy.rs in it.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
gh workflow run live.yml
```

## Start condition

T-008, T-009 and T-012 are done.

## Correction

2026-11-05, The named modules `src/warp/mod.rs` and `src/warp/proxy.rs` move to PodUtils/podcrates (`pod-warp`, T-137). podnet does not split them: a split of a moved file is work of podcrates, under its checks. No file of this entry stays in podnet. Remove the row of a moved file from `scripts/size-allow.txt` when podnet takes its crate.

## Done

2026-11-13: done by the take of `pod-warp` (podcrates `v0.37.0`, f6abdc3; T-050): the WARP chain moved to the crate, and
`src/warp/mod.rs` (788 lines) and `src/warp/proxy.rs` (425) are glue under 800 lines. No split in podnet was needed. Check: `python3 scripts/check-size.py` (0 faults).


# T-037: Split `src/relay_reverse.rs` and `crates/podnet-serve/src/share/mod.rs`

**Source:** `docs/research/podnet/architecture.md`, section 2.2 (the two rows).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`src/relay_reverse.rs` has 1,139 lines and `crates/podnet-serve/src/share/mod.rs` 1,901.

## Premise

- `tests/relay_reverse.rs` calls `relay_reverse::{node, run_node, connect}` against a fake relay.
- `crates/podnet-serve/src/share/mod.rs` holds the upload router (its `upload_receive` alone has 195 lines), the
  download router and 756 lines of tests.

## Approach

1. `src/relay_reverse.rs`: the pair functions in a mod.rs; files for the node, for `connect` and
   for the tests.
2. `crates/podnet-serve/src/share/mod.rs`: the types and the app in a mod.rs; files for the upload part and the download
   part; the tests to a tests directory with an upload part and a download part.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
cargo test --test relay_reverse
gh workflow run live.yml
```

## Start condition

T-008 and T-009 are done.

## Correction

2026-11-05, The named modules `src/relay_reverse.rs` move to PodUtils/podcrates (`pod-relay`, T-137). podnet does not split them: a split of a moved file is work of podcrates, under its checks. Split only the files that stay in podnet: `crates/podnet-serve/src/share/mod.rs`. Remove the row of a moved file from `scripts/size-allow.txt` when podnet takes its crate.

## Done

2026-11-09, commit ba9be1d (`T-037: split crates/podnet-serve/src/share.rs into crates/podnet-serve/src/share/`). `crates/podnet-serve/src/share/mod.rs` (431
lines) holds the types, the app and the shared helpers; `upload.rs` (508) and `download.rs` (221)
hold the two routers; `crates/podnet-serve/src/share/tests/upload.rs` (440) and `crates/podnet-serve/src/share/tests/download.rs` (292) hold the tests.
`src/relay_reverse.rs` is not split (the Correction). Parity: commands 1295, items 1293, tests 1108,
equal before and after. check.sh, `unshare -n` tests and `scripts/restricted-host.sh` pass. CI: at
the merge.

# T-038: Split `crates/podnet-serve/src/feature/mod.rs` and `src/proxy.rs`

**Source:** `docs/research/podnet/architecture.md`, section 2.2 (the two rows).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`crates/podnet-serve/src/feature/mod.rs` has 2,139 lines and `src/proxy.rs` 2,034.

## Premise

- The library-only subsystem of `crates/podnet-serve/src/feature/` has no caller in the binary. It stays, as a
  capability of the library (the owner's ruling on `StaticServer`, #276, `docs/decisions.md`).
- The fuzz targets use `proxy::build_request_head` (`src/fuzzing.rs`).

## Approach

1. `crates/podnet-serve/src/feature/mod.rs`: `FeatureError` and the re-exports in the mod.rs; files for the access
   control, the QR rendering, the static server and `ShareLink`; the tests split by file.
2. `src/proxy.rs`: `serve_stream` and the dispatch in a mod.rs; files for the HTTP forward, the
   axum dispatch, the share proxy, the request and response heads, and the chunked copy; the
   tests to a tests directory.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
cargo check --manifest-path fuzz/Cargo.toml --bins
gh workflow run live.yml
```

## Start condition

T-008 and T-009 are done.

## Correction

2026-11-05, The named modules `src/proxy.rs` move to PodUtils/podcrates (`pod-cloudflare`, T-137). podnet does not split them: a split of a moved file is work of podcrates, under its checks. Split only the files that stay in podnet: `crates/podnet-serve/src/feature/mod.rs`. Remove the row of a moved file from `scripts/size-allow.txt` when podnet takes its crate.

## Done

2026-11-09, commit fee6f4f (`T-038: split crates/podnet-serve/src/feature/mod.rs into crates/podnet-serve/src/feature/`). `crates/podnet-serve/src/feature/mod.rs`
(83 lines) holds `FeatureError` and the re-exports; `access.rs` (575), `qr.rs` (96),
`static_server.rs` (414) and `share_link.rs` (232) hold the parts; `crates/podnet-serve/src/feature/tests/` holds the
tests, one file for each part. `src/proxy.rs` is not split (the Correction). The finding of
TruffleHog in the doc comment of `ShareLink` moved to `crates/podnet-serve/src/feature/share_link.rs:42`
(`.github/secrets-allow.txt`). Parity: commands 1295, items 1293, tests 1108, equal before and
after. check.sh passes. CI: at the merge.

# T-039: Split `src/edge_h2.rs` and `src/tunnel.rs`

**Source:** `docs/research/podnet/architecture.md`, section 2.2 (the two rows).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** open

## Problem

`src/edge_h2.rs` has 1,736 lines and `src/tunnel.rs` 1,599.

## Premise

- `Metered` is in `src/edge_h2.rs`. T-043 moves it to the core.
- `src/tunnel.rs` holds the public `TunnelBuilder` and `Tunnel` (`tests/api_surface.rs`).

## Approach

1. `src/edge_h2.rs`: the connect functions and the reactor in a mod.rs; files for the stream
   types and the stream dispatch, the frame scanners, `serve_connection` and the tests.
2. `src/tunnel.rs`: the public API in a mod.rs; files for the start of a tunnel, the credentials,
   the leg plan, the legs and the verification; the tests to `tests.rs`.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
gh workflow run live.yml
```

## Start condition

T-008 and T-009 are done.

## Correction

2026-11-05, The named modules `src/edge_h2.rs` and `src/tunnel.rs` move to PodUtils/podcrates (`pod-cloudflare`, T-137). podnet does not split them: a split of a moved file is work of podcrates, under its checks. No file of this entry stays in podnet. Remove the row of a moved file from `scripts/size-allow.txt` when podnet takes its crate.

# T-040: Split the seccomp backend and the netns backend of the pod crate

**Source:** `docs/research/podnet/architecture.md`, section 2.2 (the two rows of the pod crate);
`docs/research/podnet/vn.md`, section 3.4.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** L
**Status:** done

## Problem

crates/podnet-netns/src/seccomp.rs has 1,900 lines and crates/podnet-netns/src/netns.rs 1,199,
with no test module. The fixes of M5 (T-063 to T-067) and the features of M7 change both files.

## Premise

- `docs/research/podnet/architecture.md`, section 2.2, gives the new modules: for seccomp, the
  supervisor, the filter, the memory access, the notification helpers, the connect, the UDP
  relay, the listen and accept, and the socket options; for netns, the run, the TUN, the
  descriptor passing, the backend and the packet builders.
- `tests/docs.rs::the_seccomp_list_of_podnet_netns_is_one_list` reads the README, not this file.
- Two series of changes on one file conflict.

## Approach

1. Split crates/podnet-netns/src/seccomp.rs into a seccomp directory, one commit for each new file.
2. Split crates/podnet-netns/src/netns.rs into a netns directory, one commit for each new file.
3. Do this entry before T-063 starts, or after T-067 is done.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-size.py
cargo test --test pod_seccomp
cargo test -p podnet-netns --test raw
sh scripts/test_in_box.sh
gh workflow run live.yml
```

## Start condition

T-008, T-009 and T-019 are done.

## Correction

2026-11-09, T-063 to T-067 are done. The Problem counts are old: `seccomp.rs` has 1516 lines and `netns.rs` has 1200
lines now. The third step of the Approach (the order) holds: this entry runs after T-067.

## Done

2026-11-09, commits 9a0e8af to 6f57e61 (`T-040: move the seccomp filter to seccomp/filter.rs` to
`T-040: cite the new files of seccomp/ and netns/`). A pure move, one commit for each new file.
`crates/podnet-netns/src/seccomp/` has `mod.rs` (384 lines: `Sock`, `run_seccomp`,
`supervise_seccomp`, the relay count), `filter.rs` (239), `mem.rs` (166), `notif.rs` (155),
`send.rs` (187) and `accept.rs` (193). `crates/podnet-netns/src/netns/` has `mod.rs` (294 lines:
the constants, `run`, `child_setup`), `tun.rs` (217), `fdpass.rs` (122), `backend.rs` (481) and
`packet.rs` (110). The two lines of `scripts/size-allow.txt` are removed. The SOCKS5 datagram path of
`crates/podnet-netns/src/upstream.rs` did not change. `sh scripts/agent/parity-snapshot.sh`: 1295
commands, 1302 public items and 1101 tests before and after, no item lost. `sh scripts/agent/check.sh`:
exit 0. `unshare -n -- cargo test --offline --test pod_seccomp --test pod_seccomp_dgram --test
pod_seccomp_diff --test pod_seccomp_nonblock`: exit 0. `sh scripts/restricted-host.sh` with the same
tests: exit 0 (22 skipped). CI: at the merge.

# T-041: Split the four large test files

**Source:** `docs/research/podnet/architecture.md`, section 2.4.
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

`tests/cli/main.rs` has 2,305 lines, the seccomp tests of the pod 2,018, `tests/shim_direct.rs` 822
and `tests/detach/main.rs` 812.

## Premise

- Cargo finds a test directory with a main.rs as the test target of that name, so the names
  `cli`, `pod_seccomp` and `detach` do not change.
- `tests/shim_no_socket.rs` and `tests/af_inet_kernel.rs` repeat parts of the C harness of
  `tests/shim_direct.rs`.
- After T-019, the seccomp tests are in `tests/` of the root crate.

## Approach

1. `tests/cli/main.rs`: a cli directory with the helpers in main.rs, and modules for the exit codes,
   the log, the manual, detach and autostart, net, warp and serve.
2. The seccomp tests (`tests/pod_seccomp/main.rs`): a `pod_seccomp` directory with the SOCKS5 stand-in and the helpers in main.rs,
   and modules for TCP, UDP, listen, DNS and the rules.
3. `tests/shim_direct.rs`: move the compile and run harness to a shared module, and use it from
   `tests/shim_no_socket.rs` and `tests/af_inet_kernel.rs`.
4. `tests/detach/main.rs`: a detach directory with modules for the supervisor, the names and the logs.

## Prove

```sh
cargo test --test cli --test pod_seccomp --test shim_direct --test detach
python3 -I scripts/check-size.py
cargo test --workspace -- --list 2>/dev/null | grep -c ': test$'
```

## Start condition

T-019 is done.

## Done

2026-11-09, commits 0824db1, 1cbdad4, 3e89ce7, 88674cf. Each large test file is a directory with a
`main.rs`: `tests/pod_seccomp/` (main.rs 268 lines; tcp, udp, listen, dns, rules), `tests/cli/`
(main.rs 60 lines; exit_codes, startup, manual, log, autostart, net, warp, serve) and
`tests/detach/` (main.rs 123 lines; supervisor, names, logs). The test targets keep their names.
The harness of `tests/shim_direct.rs` (`have_compiler`, `build_program`, `run_pair`) is in
`tests/shim_harness/mod.rs`; `tests/shim_no_socket.rs` and `tests/af_inet_kernel.rs` use it
(`compile_quiet`, `compiler_or_skip`). `tests/shim_direct.rs` has 708 lines. The four rows are out
of `scripts/size-allow.txt`. Parity: commands 1295, items 1293, tests 1108; each test name has a new
module path only. check.sh and the targets cli, pod_seccomp, detach, shim_direct, shim_no_socket and
af_inet_kernel pass under `unshare -n`. The workspace run under `unshare -n` fails the test
`signals::sighup_and_sigterm_stop_with_run_for_and_exit_0` (10 s limit); it passes alone and also
failed in the same way after T-037. CI: at the merge.

# T-042: Cut the upward references 1 to 7: `net`, `util`, `fetch`, `hop`, `report`, `detach` and `relay_reverse`

**Source:** `docs/research/podnet/architecture.md`, section 3.3 (rows 1 to 7).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

Seven core modules use one item each of a higher module. The core cannot be a crate of its own
while they do.

## Premise

`docs/research/podnet/architecture.md`, section 3.3: `net` uses `ingress::{OriginOptions,
Service}`; `util` links to `StaticResponse`; `fetch` uses `proxy::MAX_HEADERS`; `hop` downcasts
to `rpc::Refused`; `report` uses `quicktunnel::retry_of`; `detach` uses `update::previous_of`;
`relay_reverse` uses `warp::register::utc_minutes`.

## Approach

1. Split `crates/podnet-core/src/net.rs`. The core keeps `BoxedIo`, `AsyncReadWrite`, `claim_socket_path`,
   `init_crypto`, `local_name` and `connect_origin`. The origin dial functions and their
   connectors move to the origin module, with a `pub use` in `net`.
2. Change the doc link of `crates/podnet-core/src/util/mod.rs`.
3. Move `MAX_HEADERS` to `crates/podnet-core/src/util/http.rs`. `src/proxy.rs` re-exports it.
4. Tag the hop at the error site in `src/cloudflare/rpc.rs`, or move a marker type `Refused` to
   `crates/podnet-core/src/hop.rs`.
5. Move a type `Refusal` (status, code, retry_after_s) to the core. `src/quicktunnel.rs` attaches
   it to its error. `crates/podnet-core/src/report.rs` reads it by downcast.
6. Move `previous_of` from `src/update.rs` to the core.
7. Move `utc_minutes` from `src/warp/register.rs` to a time module of `crates/podnet-core/src/util/`.
8. Make one commit for each row. Remove each row from the allowlist of the layer check.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-layers.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
```

The allowlist of the layer check has seven rows fewer.

## Start condition

T-010 is done.

## Correction

2026-11-05, `fetch` and `relay_reverse` (rows 5 and 7 of the 18) and the modules of
`src/warp/` leave podnet with `pod-proxy`, `pod-relay` and `pod-warp` (T-137). Read the rows again at
the start. Cut only the references that stay in podnet.

## Done

2026-11-10, commits 25378d9 (`detach -> update`: `previous_of` in `crates/podnet-core/src/detach.rs`, a `pub use` in
`src/update.rs`), b87ba27 and 22b00a2 (`net -> ingress`: `dial`, `origin_label`, the TLS connectors
and `note_unverified` in `crates/podnet-serve/src/origin/dial.rs`; the tests stay in `crates/podnet-core/src/net.rs`), 42a9c93
(`util -> feature`: the doc names `StaticResponse` as text), ffddf71 (`hop -> cloudflare`: the type
`Refused` in `crates/podnet-core/src/hop.rs`, a `pub use` in `src/cloudflare/rpc.rs`) and c154ccf
(`report -> quicktunnel`: the type `Refusal` in `crates/podnet-core/src/report.rs`; the binary gives
`quicktunnel::retry_of` to `report::read_refusals_with`; a `pub use` in `src/quicktunnel.rs`).
The rows of `fetch` and `relay_reverse` were not in `scripts/layers-allow.txt` at the start
(`pod-proxy` and `pod_core::clock`). `scripts/layers-allow.txt` has five rows fewer;
`python3 -I scripts/check-layers.py`: 0 faults. `scripts/agent/parity-snapshot.sh` with a map of
the six moved items: 1,295 commands, 1,108 tests, 1,294 items (1,293 before; the new item is
`report::read_refusals_with`), none lost. `sh scripts/agent/check.sh`,
`unshare -n -- cargo test --offline --workspace --no-fail-fast` (1,107 passed, 1 ignored) and
`sh scripts/restricted-host.sh --no-fail-fast`: exit 0. CI: at the merge.

# T-043: Cut the upward references 8, 9 and 18: a WebSocket bridge and `Metered` in the core

**Source:** `docs/research/podnet/architecture.md`, section 3.3 (rows 8, 9 and 18).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** M
**Status:** done

## Problem

The relay client uses `ws::bridge` and `edge_h2::Metered`, and `ws` uses
`relay_reverse::close_hint`. The relay crate of M3 cannot depend on the tunnel side.

## Premise

- `src/ws.rs` holds `bridge`, `dial`, `connect_bridge`, `accept_key`, `LocalSpec` and
  `DIAL_TIMEOUT`, beside the edge side of the WebSocket forward.
- `Metered` is in `src/edge_h2.rs`. `close_hint` is in `src/relay_reverse.rs`.

## Approach

1. Move the bridge items of `src/ws.rs` to a WebSocket bridge module of the core. Keep a
   `pub use` in `ws`.
2. Move `Metered` to `crates/podnet-core/src/metrics.rs`, with a `pub use` in `edge_h2`.
3. Move `close_hint`, with the WebSocket close codes, to the bridge module.
4. Remove the three rows from the allowlist of the layer check.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-layers.py
cargo test --test relay_reverse --test connect
gh workflow run live.yml
```

## Start condition

T-010 is done.

## Correction

2026-11-05, `src/ws.rs` (`pod-ws`), `edge_h2` (`pod-cloudflare`) and `relay_reverse` (`pod-relay`) move to
PodUtils/podcrates. The three rows disappear when podnet takes these crates. Read the rows again at the start.

2026-11-12, the rows are gone with the takes: `ws -> relay_reverse` at 5eace6e (the take of `pod-ws`; `close_hint` is `pod_ws::close_hint`), `ws_relay -> edge_h2` and `ws_relay -> ws` at 83e0c93 (the take of `pod-relay`; `Metered` is `pod_core::Metered`). The moves of the Approach were not necessary: the code that had the rows is gone.

## Done

2026-11-12, commits 5eace6e and 83e0c93 (the takes). `scripts/layers-allow.txt` has four rows (`warp -> cloudflare`, `warp -> quicktunnel`, `ws -> cloudflare`, `ws -> proxy`) and none of the three rows of this entry. `python3 -I scripts/check-layers.py`: 0 faults. `src/ws_relay.rs` names neither `edge_h2` nor `crate::ws`. `src/ws.rs` names no `relay_reverse`. `cargo test --test relay_reverse --test connect`: 2 targets, 16 passed (connect 8, relay_reverse 8). Not run: `gh workflow run live.yml`. The other rows of `scripts/layers-allow.txt` are T-044.

# T-044: Cut the upward references 10 to 17: the WARP QUIC configuration, `ShareLink`, `RULE_KEYS`, `connect_tunnel` and the edge side of `ws`

**Source:** `docs/research/podnet/architecture.md`, section 3.3 (rows 10 to 17).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** L
**Status:** blocked

## Problem

Eight references cross from the WARP, service and core modules to the Cloudflare tunnel side.
The WARP and service crates of M3 cannot depend on the Cloudflare crate.

## Premise

`docs/research/podnet/architecture.md`, section 3.3: `warp` uses `CF_EDGE_ROOTS_PEM`, `QUIC_ALPN`
and `quic::transport_config`; `warp` uses `quicktunnel::parse_retry_after`; `feature` uses
`ShareLink::from_quick_tunnel`; `ingress` uses `config::RULE_KEYS`; `relay` and `transport` use
`connect_tunnel` through `proxy`; `ws` uses `cloudflare::stream` and four items of `proxy`.

## Approach

1. `src/warp/quic.rs` keeps `WarpUdpSocket` and takes the client configuration as an argument.
   The client configuration (the edge roots, the ALPN, the MTU) moves to the tunnel side.
2. Move `parse_retry_after` to `crates/podnet-core/src/util/http.rs`.
3. Give `ShareLink` a constructor from a URL and an id. Move the conversion from a quick tunnel to
   the Cloudflare side.
4. Move `RULE_KEYS` to `crates/podnet-serve/src/ingress.rs`. `src/config.rs` re-exports it.
5. Make `crates/podnet-serve/src/relay.rs` and `src/transport/` use `crates/podnet-core/src/connect.rs` directly.
6. Move `serve_websocket` and `forward_path` from `src/ws.rs` to the tunnel side, beside
   `src/proxy.rs`.
7. Make one commit for each row or pair of rows.

## Prove

```sh
sh scripts/agent/check.sh
python3 -I scripts/check-layers.py
sh scripts/agent/parity-snapshot.sh /tmp/after /tmp/before
gh workflow run live.yml
```

The allowlist of the layer check is empty.

## Blocker

The session of `PodUtils/podcrates` and T-137. Four rows are cut (2026-11-10): `ingress -> config`
(3714039: `RULE_KEYS` in `crates/podnet-serve/src/ingress.rs`, a `pub use` in `src/config.rs`), `relay -> proxy`
(bc0f58d) and `transport -> proxy` (f1f69c4: they call `crates/podnet-core/src/connect.rs`), `feature -> quicktunnel`
(799d4be: `ShareLink::from_quick_tunnel` takes the URL; a8a36bf: the layer check skips each file of
a test module). Four rows stay in `scripts/layers-allow.txt`: `warp -> cloudflare` and
`warp -> quicktunnel` (`src/warp/`: `pod-warp`, to `pod-cloudflare`), `ws -> cloudflare` and
`ws -> proxy` (`src/ws.rs`: `pod-ws`, to `pod-cloudflare`). Each starts in frozen code; a cut is a
change in it beyond a fix. The rows disappear when podnet takes `pod-ws`, `pod-warp` (T-050) and
`pod-cloudflare` (T-052). Then remove the rows and close the entry.

## Start condition

T-010 is done.

## Correction

2026-11-05, The rows 10 to 17 cross from the WARP adapter and the services to the Cloudflare adapter. Both
adapters move to `pod-warp` and `pod-cloudflare`. Read the rows again at the start. Cut only the
references that stay in podnet (for example `ShareLink` and `RULE_KEYS`).

# T-045: Each file is under 800 lines, and the allowlists of the size check and the layer check are empty

**Source:** `docs/research/podnet/architecture.md`, sections 3.4 and 4.6; the owner's ruling of
2026-11-05 (`docs/decisions.md`, File size).
**Category:** chore
**Milestone:** M2
**Priority:** P2
**Effort:** S
**Status:** partial

## Problem

M3 extracts crates only when no file is over 800 lines and no module refers to a higher layer.

## Premise

- After the cuts, the simulation of `docs/research/podnet/architecture.md`, section 3.4, finds no
  upward reference. Two cycles stay, each inside one future crate: `egress`, `fetch`, `report`
  and `resolve` (the core), and `config`, `edge_h2`, `quicktunnel` and `tunnel` (the Cloudflare
  crate).
- The 29 files of 501 to 800 lines need no split under the ruling of 800 lines.

## Approach

1. Run both checks. Split each file that is still over 800 lines.
2. Empty both allowlists. Keep the files of the allowlists, so that a later exception needs a
   row.
3. Write the result in the Done section: the largest file and its count.

## Decision

The owner, 2026-11-05 (`docs/decisions.md`, File size): a source file has at most 800 lines.

## Prove

```sh
python3 -I scripts/check-size.py
python3 -I scripts/check-layers.py
git ls-files '*.rs' | xargs wc -l | sort -n | tail -3
```

## Start condition

T-026 to T-044 are done.

## Correction

2026-11-05, The two cycles of the Premise (the core and the Cloudflare crate) leave podnet with the takes of
`pod-proxy`, `pod-dns`, `pod-cloudflare` (T-137). The allowlists empty by moves and by splits.

2026-11-12, The 15 files of `scripts/size-allow.txt` all belong to a crate of podcrates, so none is split in
podnet (`CLAUDE.md`, Order; `docs/decisions.md`, 2026-11-11, the freeze of pod-relay, pod-warp and
pod-cloudflare). `src/config.rs` and `src/proxy.rs` have the layer `cloudflare` in
`scripts/check-layers.py`: they go to `pod-cloudflare` (T-052) and take fixes only. The size check lists
the files by take:

- T-049 (`pod-relay`): `src/relay_reverse.rs` (891), `src/ws_relay.rs` (994).
- T-050 (`pod-warp`): `src/warp/ip.rs` (1680), `src/warp/mod.rs` (1002), `src/warp/proxy.rs` (1870),
  `src/warp/register.rs` (855), `src/warp/session.rs` (1211), `src/warp/up.rs` (817).
- T-052 (`pod-cloudflare`): `src/cloudflare/capnp.rs` (944), `src/cloudflare/http2.rs` (1123),
  `src/config.rs` (1206), `src/edge_h2.rs` (1608), `src/proxy.rs` (2034), `src/quicktunnel.rs` (850),
  `src/tunnel.rs` (1404).

The five rows of `scripts/layers-allow.txt` stay: `warp -> cloudflare` and `warp -> quicktunnel` leave
with T-050 and T-052; `ws -> cloudflare`, `ws -> proxy` and `ws_relay -> edge_h2` leave with T-049, T-052
and the take of `pod-ws` (T-137). Each is a reference inside a frozen file.

## Done

2026-11-12, no code commit. Both checks pass (`check-size`: 447 files, 15 allowlisted, 0 faults;
`check-layers`: 0 faults). The largest file is `src/proxy.rs` (2034 lines). No file is split and no
allowlist row is removed: each row waits for a take (the Correction). The entry stays `partial` until
T-049, T-050 and T-052 have moved the files.
