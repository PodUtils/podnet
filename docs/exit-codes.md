# Exit codes

podnet uses one table of exit codes. The constant is
`podnet_core::exit::CONTRACT`. The help of the binary prints the table from
that constant, and this document holds the same table. A test compares the
two.

| Code | Name | Meaning | Commands |
| --- | --- | --- | --- |
| 64 | usage | an unknown or malformed command, flag or value | each command |
| 66 | no-input | a key, identity or allow list that must exist is missing | link, send, receive, warp |
| 69 | unavailable | a carrier, listener or relay disabled by policy, or not answering | link, relay, init, harness |
| 71 | os-error | identity file permissions or ownership are wrong | link, serve |
| 77 | no-perm | authentication failed or the peer is not in the allow list | link, send, receive, relay |
| 78 | config | a schema, policy, protocol or suite mismatch | init, rules, status, harness, link |

## Why the codes stay distinct

A script that retries a failure with code 77 makes a different decision from
one that retries a failure with code 78. Code 77 is a refusal of the peer:
the allow list does not admit it. Code 78 is a mismatch of a configuration,
a protocol or a suite: a retry sends the same failing request. The codes in
the first table stay distinct for that reason. `tests/exit_contract.rs`
invokes the binary for each code and reads the status directly.

## How to read each code

- 64: the command line is wrong. podnet did nothing.
- 66: a file that must exist is absent. The line names the file.
- 69: the path is disabled by policy, or the service does not answer. The
  line names the policy or the service, not a timeout.
- 71: the identity file has a wide mode or the wrong owner. The line gives
  the mode to set.
- 77: the peer is not in the allow list, or its authentication failed.
- 78: the configuration, the protocol or the suite does not match. The line
  names the part that does not match.

## The test of the table

The test `the_help_holds_the_table_of_the_one_constant` reads the help of
`podnet --help` and compares each row with the constant. The test
`the_document_holds_the_same_table` reads this page and compares each row
with the same constant.

## The labels

Each number in this document is a code, not a measurement. The four labels
of a measurement are measured here, read from source, reported by a third
party and inferred. This document has no measurement, so it has no label of
that kind.
