# The host runtime

This document states the host runtime of podnet#500: the file system
properties, the crash log, the external measurement, the initial
configuration and the classified egress. The limits page is
`docs/target-environment.md`; this document does not repeat it.

## The file system probe

`podnet host probe PATH` states five properties of the file system at PATH:
timestamps, atomic rename, hard links, permission bits and case sensitivity.
Each property is `required` or `emulated`. The probe reads a boot sector of
an image, so it needs no mount, and it reads the mount of a directory.

```sh
podnet host probe /mnt/data
podnet host probe volume.img --json
```

A FAT volume has a two-second timestamp granularity and no permission bits.
A sequence ordered by timestamp is not reliable there. The report says that
the runtime keeps its own serial number, copies a file instead of linking
it, verifies each hash after an atomic rename, and compares names as lower
case. The probe runs at mount time, so a property that a file system cannot
give is known before the first transfer. `src/hostprobe.rs` holds the probe.

A read-only mount refuses each write with a named reason before the write
starts. The fallback keeps the mount read-only and writes state and keys to
a writable directory.

The measured FAT image is a synthetic boot sector, because this host has no
`mkfs.vfat`. The conditions are the host `errand-sa9k4d8jrmz3`, podnet
`0.4.0-beta.17` and 1 run, on 2026-11-20. The label is measured here.

| Probe | Family | Read-only | Properties | Fallback |
| --- | --- | --- | --- | --- |
| a 512-byte FAT16 boot sector | fat, fat16 | no | 5 emulated | serial number and copy |
| the same image with mode 0444 | fat, fat16 | yes | 5 emulated | refuse each write |
| a temporary unix directory | unix | no | 5 required | none |

The test `a_fat_image_is_named_and_each_property_is_emulated` and the test
`a_read_only_image_refuses_a_write_with_a_reason` prove the rows.

## The crash log

A panic writes one file with the backtrace, the component versions and the
last events. The text passes the redaction at write time. The file ends with
a marker that names the number of hidden values. The directory is
`PODNET_CRASH_LOG_DIR`, else the state directory.

The log is bounded and rotated. The variables `PODNET_CRASH_LOG_MAX` and
`PODNET_CRASH_LOG_SLOTS` set the size of one file and the number of files.
The default is three files of 65536 bytes. A panic loop fills the bound and
no more.

```sh
PODNET_CRASH_LOG_DIR=/tmp/crash podnet host panic --seed VALUE
```

The test `a_panic_writes_the_backtrace_the_versions_and_the_events` starts a
child that panics with a seeded value, reads the file, and searches it for
the value. The test `the_crash_log_is_bounded_and_rotated` runs three
children with two slots of 4096 bytes and checks each file against the
bound. Both labels are measured here, from the host `errand-sa9k4d8jrmz3`,
podnet `0.4.0-beta.17` and 1 run, on 2026-11-20.

`src/crashlog.rs` holds the hook. `podnet host panic` is hidden from the
help and exists for the test.

## The external measurement

`podnet stat` sends probes to a peer that timestamps each arrival. The
throughput, the latency and the loss come from the peer's clock. The sender
also measures its own rate, to compare. A difference over five percent is a
disagreement with both numbers; the run does not average them.

```sh
podnet stat --peer 127.0.0.1:9000 --rate 1000 --size 50 --count 10 --out pod-stat.txt
podnet stat --serve 127.0.0.1:9000
```

The output is a capture file with one line for each packet and a summary. A
regression is a diff of two capture files. `src/stat.rs` holds the run.

The measured run is a loopback peer at 1 Kbit/s with packets of 50 bytes.
The conditions are the host `errand-sa9k4d8jrmz3`, podnet `0.4.0-beta.17`
and 1 run, on 2026-11-20. The label is measured here.

| Run | Peer rate | External rate | Difference | Capture |
| --- | --- | --- | --- | --- |
| 10 packets at 1 Kbit/s | within 5 percent | timestamped by the peer | under 5 percent | 10 sent, 10 received |
| a forged peer counter | peer rate | sender rate | over 5 percent | `disagreement` line |
| a peer that answers nothing | 0 | 0 | not compared | loss 1.0000 |

The test `a_throttled_loopback_agrees_with_an_external_clock` computes the
rate from the arrival times of the peer and compares it with the reported
rate. The test `a_forged_timestamp_is_reported_as_a_disagreement` reads the
capture file and checks the disagreement line.

## The initial configuration

`podnet init generate NODE --relay URL` writes a configuration that
listens on nothing. `podnet init check FILE` validates the document against
schema 1. `podnet init enroll DIR --node NAME --relay URL` issues the
identity, registers the node with the relay and writes the allow list. The
three parts are written together, or none of them.

The generated document is valid against the schema that the generator emits,
`node-config.schema.json`. A hand-edited listener is refused with the named
error `INIT-1`. A failed enrolment leaves no destination and no temporary
directory.

```sh
podnet init generate node-a --relay http://relay.example
podnet init check node.json
podnet init enroll state --node node-a --relay http://relay.example --offline
```

`src/initcfg.rs` holds the configuration and the enrolment. The tests
`a_generated_configuration_listens_on_nothing`,
`a_hand_edited_listener_is_refused` and
`a_failed_enrolment_leaves_no_partial_state` prove the three parts.

## The classified egress

A destination is classified before any connection: `direct`, `proxy`,
`relay`, `refuse` or `queue`. The inputs are the destination, the resolved
address family, the port and the policy of the deployment. A refusal carries
its reason, so a policy decision is not a timeout. The policy lives in
`src/policy.rs`; a socket asks it and does not decide for itself.

`PODNET_POLICY` names a policy file. `PODNET_NODE_CONFIG` names a node
configuration whose `egress` object is the policy. The process installs the
policy before the first dial. A file that cannot be read or parsed exits 78.

With neither variable set, no policy is installed and `policy::permit`
allows every destination: the default is allow-all, not refuse. The broker
is active only when a policy is installed. Set one in a deployment that
needs a classified egress.

```sh
PODNET_POLICY=rules.json podnet bench --duration 1 https://example.net/file
podnet rules check rules.json
podnet rules check rules.json --json
```

A policy file (`rules.json`):

```json
{"default": "refuse", "rules": [
  {"name": "api", "host": "*.example.net", "port": 443, "action": "direct"},
  {"name": "dns", "host": "1.1.1.1", "port": "53", "family": "ipv4", "action": "proxy"},
  {"name": "no-ads", "host": "ads.example.com", "action": "refuse"}
]}
```

| Field | Necessary | Value |
| --- | --- | --- |
| `default` | no (`refuse`) | The action for a destination that no rule matches. |
| `rules` | yes | The rules, in order. The first rule that matches decides. |
| `name` | yes | The name of the rule. A refusal and `rules check` name it. |
| `host` | yes | A name or an address (`example.net`), a suffix (`*.example.net`: each name under it), or `*` (each host). |
| `port` | no (`*`) | A number (`443`), a string of a number (`"443"`), a range (`"8000-8999"`), or `"*"` (each port). |
| `family` | no (`any`) | `any`, `ipv4` or `ipv6`: the family of the resolved address. A name has `any` until it resolves. |
| `action` | yes | `direct`, `proxy`, `relay`, `refuse` or `queue`. |

`podnet bench` sends its requests through the policy: a refused destination
is an error of each request. `podnet rules check` names each rule that an
earlier rule covers, and exits 78 when a later rule is unreachable. A file
that cannot be read or parsed (also a missing file) exits 78 too. Its
`--json` document has the stamp (`schema` 1 and `podnet`) and the key
`kind`: `podnet.rules-check/1`. `podnet init check --json` and
`podnet ci check --json` have the same form, with the `kind`
`podnet.init-check/1` and `podnet.ci-check/1`. The test
`a_destination_that_matches_no_rule_is_refused_before_a_connect` uses a
connector that counts its calls and proves the count is zero.

The test `no_source_path_dials_without_a_policy_check` scans the source for
`TcpStream::connect`, `egress::dial_addrs`, `TcpSocket::new_v4` and
`new_v6`, `UdpSocket::connect`, `socket.connect`, `UdpSocket::bind` and
`send_to`. A dial must have `policy::permit`, `permit_fault`, `permit_io`
or `permit_failure` in the lines above it. A local bind or a reply may
carry `egress-exempt: REASON` instead. A self-written exemption does not
excuse a dial, and the scan has a test that proves it fires on one.

`tests/egress_refusal.rs` installs `{"default":"refuse","rules":[]}` and
drives each path at run time: `egress::dial_addrs`, `cover::open_target`,
`defer::cache::HttpOrigin`, `networks::netbird::greeting`, the probes of
`agent`, the far end of `e2e`, and the `link` services over a unix socket.
Each check holds a bound listener and asserts that it never accepts. The
near end of `podnet tunnel --e2e` is a command-level check with exit 69.

`podnet ci check FILE` reads a declaration of the hosts, ports and UDP
needs of a pipeline and checks it against the installed policy before the
first step. It names each offending host and exits 69. Without an installed
policy it exits 78.

```json
{"schema": 1, "kind": "podnet.ci-egress/1", "hosts": [
  {"host": "api.example.net", "port": 443, "udp": false}
]}
```

`schema` is 1 and `kind` is `podnet.ci-egress/1`; the older form `"schema": "podnet.ci-egress/1"` is read too. Another
value exits 78. `udp` is `false` when it is not given.

`podnet forge clone --seed PATH --expect HASH DEST` clones a local
content-addressed seed with the `file` protocol only, verifies the commit
hash of the checkout, and refuses a mismatch with exit 65.

## The status page

`podnet status --page` generates the status page of the tree. The page names
each repository, its version, its last commit and its state. `--check FILE`
compares a page with the tree and exits 78 when an entry is stale.

```sh
podnet status --page --root . --out status.md
podnet status --check status.md --root .
```

`src/page.rs` holds the scan and the check. The test
`a_stale_version_fails_the_check` mutates a version and checks that the
check names the repository and the field.

## The network surface

The runtime's network surface is the egress broker, not the guest. The
limits page `docs/target-environment.md` states the network surface and the
chroot caveat. Do not repeat it here.
