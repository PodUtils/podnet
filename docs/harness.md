# The verification harness

`podnet harness` runs the verification matrix of podnet#501. It measures how
visible each road is to a middlebox. The result is a table with one row for
each road and each deployment profile. It is not a pass count.

```sh
podnet harness --data DIR
podnet harness --data DIR --json --archive FILE
```

`--data` names the fixture directory. It holds `rules.json`, `flow.json`,
`prefix.json`, `cases.json` and `captures/`. `--json` prints the table and
its conditions as one JSON document. `--archive` also writes the relocatable
zip.

## The three classifiers

Each classifier reads one capture and returns a score in 0..1 and a named
reason. A higher score means that the road looks more like the tunnel model.

| Classifier | Input | Model | Cost |
| --- | --- | --- | --- |
| field | SNI, ALPN, JA3, JA4, TLS record sizes | `rules.json` | one parse of the ClientHello |
| flow | size mean, standard deviation, p10, p50, p90, inter-arrival histogram | `flow.json` | one pass over the packets |
| prefix | the first N bytes, as byte n-grams | `prefix.json` | one pass over the prefix |

The classifiers are in `pod-harness` of podcrates v0.79.0. The glue is
`src/harness.rs`; the command is in `src/cli/cmd/runtime.rs`.

## The table

The table names the road, the profile, the classifier, the score and the
date. The proxy profile pads each packet to 1200 bytes. The restricted
profile splits each packet into 64-byte chunks. The fixture has 11 roads and
3 profiles, so it has 33 rows. On this host six rows skip with a named
reason: the three icmp rows need a raw socket, and the three s3 rows need a
bucket and credentials. A host with a raw socket scores the icmp rows; s3
stays a named skip until a bucket exists.

A row that did not run stays in its place as `skip`. A skip is not a pass.

The following numbers come from one run of this tree. The conditions are the
host `errand-sa9k4d8jrmz3`, podnet `0.4.0-beta.17`, podcrates `0.79.0`,
rustc `1.100.0`, input `tests/data/harness` and 1 run, on 2026-11-21. The
label is measured here.

| Road | Profile | Field | Flow | Prefix | Note |
| --- | --- | --- | --- | --- | --- |
| unix | direct | 1.000 | 1.000 | 1.000 | |
| unix | proxy | 0.700 | 0.579 | 1.000 | |
| unix | restricted | 0.700 | 0.544 | 0.200 | |
| memory | direct | 0.650 | 0.525 | 1.000 | |
| memory | proxy | 0.350 | 0.458 | 1.000 | |
| memory | restricted | 0.350 | 0.543 | 0.200 | |
| tcp | direct | 0.550 | 0.794 | 1.000 | |
| tcp | proxy | 0.550 | 0.563 | 1.000 | |
| tcp | restricted | 0.550 | 0.566 | 0.200 | |
| ws | direct | 0.200 | 0.484 | 0.200 | |
| ws | proxy | 0.200 | 0.444 | 0.200 | |
| ws | restricted | 0.200 | 0.546 | 0.200 | |
| h2 | direct | 0.700 | 0.450 | 1.000 | |
| h2 | proxy | 0.700 | 0.424 | 1.000 | |
| h2 | restricted | 0.700 | 0.545 | 0.200 | |
| poll | direct | 0.500 | 0.502 | 1.000 | |
| poll | proxy | 0.200 | 0.404 | 1.000 | |
| poll | restricted | 0.200 | 0.554 | 0.200 | |
| quic | direct | 0.850 | 0.316 | 1.000 | |
| quic | proxy | 0.550 | 0.266 | 1.000 | |
| quic | restricted | 0.850 | 0.533 | 0.200 | |
| udp | direct | 0.550 | 0.448 | 1.000 | |
| udp | proxy | 0.550 | 0.448 | 1.000 | |
| udp | restricted | 0.550 | 0.545 | 0.200 | |
| dns | direct | 0.500 | 0.543 | 0.200 | |
| dns | proxy | 0.200 | 0.464 | 0.200 | |
| dns | restricted | 0.500 | 0.553 | 0.200 | |
| icmp | direct | - | - | - | skip: the icmp carrier needs a raw socket; this lab holds none |
| icmp | proxy | - | - | - | skip: the icmp carrier needs a raw socket; this lab holds none |
| icmp | restricted | - | - | - | skip: the icmp carrier needs a raw socket; this lab holds none |
| s3 | direct | - | - | - | skip: the s3 carrier needs a bucket and credentials; this lab has neither |
| s3 | proxy | - | - | - | skip: the s3 carrier needs a bucket and credentials; this lab has neither |
| s3 | restricted | - | - | - | skip: the s3 carrier needs a bucket and credentials; this lab has neither |

The unpadded road scores higher than the padded road under the field
classifier: 1.000 against 0.700 on the host, version, input and 1 run above,
on 2026-11-21 (measured here). This is the gate of DPI-5. The test
`the_padded_road_scores_lower_than_the_unpadded_one` reads the table and
fails when a padded row does not score lower.

## The benign cover site

The table ends with the false-positive report of the benign cover site. The
rate must be 0.100 or less; the line after the report names that gate. The
0.500 in the report is the score over which the classifier calls a sample
flagged, and it is a different number. The test
`the_table_has_the_date_the_profiles_and_the_skips` reads the rate and fails
over the bound. A classifier that flags every flow is not a classifier.

The measured rate of the fixture is 0.000 from 0 flagged samples of 32
draws, from the same host, version, input and 1 run, on 2026-11-21. The
label is measured here. Each draw is the benign capture with a deterministic
timing jitter, so the bound is a count over 32 draws rather than a bound over
one sample.

## The budget

The budget is `600 s`. The run above took 0.011 s of wall time, from the same
host, version, input and 1 run, on 2026-11-21. The label is measured here. A
run over the budget exits 69 and prints no table.

## The archive

`--archive FILE` writes one zip. The zip holds the harness binary, the
fixtures, the three models, the middlebox simulator source and its
decisions, one expected score for each row, the expected table, the manifest
and `run.sh`. It holds no repository path and no toolchain.

```sh
sh run.sh
sh run.sh --json
sh run.sh --list
```

The first form runs the bundled binary and prints the matrix. A carrier that
cannot run on that host is a named skip. The test
`the_archive_runs_the_matrix_with_no_toolchain` unzips the archive in a
temporary directory and runs `run.sh` with no argument.

## The exit codes

The harness uses the contract of `podnet_core::exit::CONTRACT`:

| Code | Class | When |
| --- | --- | --- |
| 64 | usage | an unknown or malformed argument |
| 66 | no-input | a fixture, model or key that must exist is missing |
| 69 | unavailable | the run is over its budget, or the benign rate is over its bound |
| 71 | os-error | a file cannot be used |
| 77 | no-perm | a peer that the allow list does not admit |
| 78 | config | a fixture, model or table that does not parse |

`docs/exit-codes.md` holds the same table.

## The claim labels

Each number in this document carries one label. The four labels are measured
here, read from source, reported by a third party and inferred. A number also
carries the host, the version, the input and the number of runs. A number
with no conditions is removed, not softened.

`scripts/check-claims.py` reads this document and `docs/exit-codes.md`. It
fails on a bare comparative and on a number with no conditions.
