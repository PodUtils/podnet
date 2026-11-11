# A native git server with no installed `git`

> Research of T-122. Date of reading: 2026-11-11. Nothing was run against a service. No account was
> used. The local measures are lock files and source line counts of scratch projects. No binary was
> built, so no binary size was measured. Each fact has a mark.

Marks:

* **read**: the fact is in a document or in source code. The source is a URL, read on 2026-11-11.
  "Registry copy" means the crate source that cargo downloaded to the local registry. It is the same
  text as the file at the docs.rs URL of the same version.
* **measured**: a command gave the number on 2026-11-11. The command is in the text.
* **inferred**: the fact follows from read or measured facts. It was not run.

## 0. Result

| Question | Answer | Mark |
| --- | --- | --- |
| Does `gix` have a server for upload-pack and receive-pack? | No. The status page of gitoxide has the lines "upload-pack / receive-pack server plumbing for in-process transports" and "general purpose `accept(…)` for servers" unchecked. The crates `gix-protocol` and `gix-transport` hold the client side only. | read |
| Does `gix` have the parts of a server? | Yes, most of the data layer: the pkt-line codec, pack writing (counts, entries, bytes), pack reading with index and thin-pack completion, ref transactions with locks, bare repository creation. | read |
| What does podnet write? | The protocol of both services (v0, v1, v2 for upload-pack; v0 and v1 for receive-pack), the negotiation, the connectivity and object checks of a push, the ref update with the report, the HTTP and SSH glue, and a maintenance pass. About 3,700 lines (3,850 with `git-upload-archive`). | inferred |
| Does the pack writer match `git`? | No. `gix-pack` does not compress deltas. It copies deltas from existing packs and sends loose objects whole. A clone of a repository with loose objects is larger than from `git`. | read |
| Which gaps does `gix` have for a host? | No hook runner, no `reference-transaction`, no bitmaps, no repack, no `fsck` checks of `receive.fsckObjects`, no quarantine. | read |
| How many crates? | 64 crates in the normal graph (musl target) for the data layer without the `gix` facade; 88 for `gix` with `default-features = false`; 136 for `gix` with the default features (each count includes the scratch crate). 24 of these names are in the lock of podnet already. | measured |
| License? | `MIT OR Apache-2.0` for each `gix` crate that was read. This fits 0BSD code and `deny.toml` (inferred: `cargo deny` was not run). | read, inferred |
| ring or aws-lc? | The server side needs no TLS. No crate of the measured graphs links ring, aws-lc or openssl. The choice appears only if podnet turns on an HTTP client feature of `gix`, which it does not need. | measured, inferred |
| Is `git` still needed for anything? | For `git-upload-archive` and for hooks, unless podnet writes them. A hook is a program, so a host with no programs runs no hook. | inferred |
| Should podnet write it now? | Not as the only server. Write it as a second backend behind one trait, in stages, with the `git` program as the default when it exists. See section 7. | inferred |

The recommendation is in section 7. The questions for the owner are in section 8. The proposed entries
are in section 9.

## 1. The target

All facts of this section are **read** from the repository.

* `docs/target-environment.md` describes hosts with no `CAP_NET_ADMIN`, with proxy-only egress and often
  with few programs. A podnet binary is static. It must not need a program on the host.
* `docs/decisions.md`, 2026-11-05, Git hosting: `podnet git serve` uses the installed `git` now
  (`git http-backend`, and `upload-pack` and `receive-pack` under a russh SSH server). A native server
  comes later.
* `docs/research/podnet/capabilities.md`, section 5.3, design C (native): "Not recommended" at that time.
  It named `gix 0.90` and `git2`. This study replaces that row with measures.
* The git server of podnet today (`crates/podnet-serve/src/git/`, 2,020 lines with tests):
  * `ssh.rs` (499 lines): a russh server. The function `run_git` starts `git upload-pack DIR`,
    `git receive-pack DIR` or `git upload-archive DIR` and connects the channel to the pipes of the
    child. It sets `GIT_PROTOCOL` from the SSH environment request.
  * `cgi.rs` (288 lines): a CGI runner. `http.rs` (204 lines): the smart HTTP router. It runs
    `git http-backend` through the runner.
  * `mod.rs` (270 lines): `Service`, `parse_exec`, `repo_dir` (path rules), `create_repo`.
  * `src/cli/cmd/git_serve.rs` (442 lines): the command. It runs `git --version` at the start and
    stops with a fault when `git` is not on `PATH`.
  * The tests skip with one line when `git` is not on `PATH`.
* The seam for a native backend is small. The authentication, the path rules and the transport
  (axum request body, SSH channel) stay. Only "run a program and pipe bytes" changes to "call a function
  with a reader and a writer".

## 2. What the services must do

### 2.1 Protocol versions

* **read**: protocol v2 defines three commands: `ls-refs`, `fetch` and `object-info`. The page defines
  no command for push. Source: <https://git-scm.com/docs/protocol-v2>, 2026-11-11.
* **read**: `builtin/receive-pack.c` of git ignores a request for v2 and falls back to v0. The comment
  says "push support for protocol v2 has not been implemented yet". Source:
  <https://raw.githubusercontent.com/git/git/master/builtin/receive-pack.c>, 2026-11-11
  (the `switch` on `determine_protocol_version_server()`).
  So a native receive-pack needs v0 and v1 only. Upload-pack needs v0, v1 and v2, and v2 is the one
  that current clients use.
* **read**: the fetch arguments of v2 are `want`, `have`, `done`, `thin-pack`, `no-progress`,
  `include-tag`, `ofs-delta`. The features `shallow`, `filter`, `ref-in-want`, `sideband-all`,
  `packfile-uris` and `wait-for-done` are optional. The server advertises the ones it has. Source:
  <https://git-scm.com/docs/protocol-v2>, 2026-11-11.
* **inferred**: a minimal v2 server advertises `ls-refs`, `fetch` (no feature), `object-format=sha1` and
  `agent`. A client works without the optional features. Shallow clones (`--depth`) need the `shallow`
  feature. Partial clones (`--filter`) need `filter`.

### 2.2 Smart HTTP

* **read**: `GET info/refs?service=NAME` answers `200`, `Content-Type:
  application/x-NAME-advertisement`, `Cache-Control: no-cache`. The body starts with the pkt-line
  `# service=NAME`, then a flush packet, then the refs (v0) or the capabilities (v2). `POST
  git-upload-pack` and `POST git-receive-pack` use the types `application/x-git-NAME-request` and
  `application/x-git-NAME-result`. A server must refuse a `want` that is not reachable from an
  advertised ref, unless it advertises `allow-tip-sha1-in-want` or `allow-reachable-sha1-in-want`.
  Source: <https://git-scm.com/docs/gitprotocol-http>, 2026-11-11.
* **read**: the page does not describe v2 over HTTP, and it does not describe gzip request bodies. The
  existing code of podnet already handles both through `git http-backend`. The test
  `git_http_clone_fetch_and_push` requires a gzip upload-pack request over 1 KiB and a chunked push of
  8 MiB (`TODO/services.md`, T-092, Prove).
* **inferred**: a native HTTP handler must inflate `Content-Encoding: gzip` bodies and read chunked
  bodies. `zlib-rs` is in the graph of `gix` (measured, section 4). `flate2` is not in the lock of
  podnet (measured, `grep` of `Cargo.lock`).

### 2.3 SSH

* **read**: the SSH exec request has the command `git-upload-pack 'PATH'`. `parse_exec` reads it today
  (`crates/podnet-serve/src/git/mod.rs`). The channel then carries the same pkt-line stream as HTTP,
  without the `# service=` header and without the HTTP framing.
* **inferred**: a native service is one function `serve(service, repo, protocol, reader, writer)`. The
  HTTP handler and the SSH handler call it. For HTTP, `info/refs` calls it with the "advertise only"
  flag, as `git upload-pack --advertise-refs` does.

## 3. What `gix` has

The version is `gix 0.90.0`, published 2026-10-22, `rust-version` 1.88 (**read**, crates.io API,
<https://crates.io/api/v1/crates/gix>, 2026-11-11). podnet has Rust 1.99, so the version is no limit.

### 3.1 The status page

Source: <https://raw.githubusercontent.com/GitoxideLabs/gitoxide/main/crate-status.md>, 2026-11-11.
Mark: **read** for each row.

| Part | State on the page |
| --- | --- |
| `gix-packetline`: pkt-line read and write, error line, v2 delimiter and response-end, sideband | done |
| `gix-transport` client: `file://`, `ssh://` (by an external `ssh`), `git://`, `http(s)://`; v1 and v2 handshake | done (client only) |
| `gix-transport` server: "general purpose `accept(…)` for servers" | not done |
| `gix-protocol`: handshake, `ls-refs`, `fetch` with negotiation, shallow | done (client only) |
| `gix-protocol`: push (`send-pack` plumbing, report-status, atomic) | not done |
| `gix-protocol`: "upload-pack / receive-pack server plumbing for in-process transports" | not done |
| `gix-pack` decode: full and deltified objects; index from a pack; thin-pack resolution | done |
| `gix-pack` encode: object to entries, base-object compression, "create 'thin' pack" | done |
| `gix-pack` encode: delta compression | not done |
| `gix-pack` encode: "write index along with the new pack" | not done |
| `gix-pack`: bitmap file | not done |
| `gix-ref` transactions: create, update, delete, many refs at once, reflog optional | done |
| `gix-ref`: namespaces ("a server-side feature to transparently isolate refs") | done |
| `gix-ref`: "disable transactions during quarantine" | not done |
| hooks: "receive-side hooks and `reference-transaction`" | not done |
| `fsck`: "all individual checks available in `git fsck`" | not done |
| archive: "create a byte stream and create archives for such a stream" | done (feature `worktree-archive`) |

The page says it was generated by Codex in part ("still good for an overview"). So the unchecked rows
are the claim of the maintainers, not a test.

### 3.2 The source (registry copy)

Mark: **read** for each row. The files are in `gix-pack 0.77.0`, `gix 0.90.0`.

* `gix_pack::data::output::count::objects` walks the objects that a want set needs, with the objects
  the client has excluded. `gix_pack::data::output::entry::iter_from_counts` makes the entries.
  `gix_pack::data::output::bytes` writes the pack bytes. The only `Mode` is `PackCopyAndBaseObjects`.
  Its doc comment says: copy base objects and deltas from packs, and treat non-packed objects as base
  objects "without trying to delta compress them ... at the cost of bandwidth".
* `gix_pack::Bundle::write_to_directory` reads a pack from a stream, writes the pack and its index,
  and takes `thin_pack_base_object_lookup` to complete a thin pack with the bases from the repository.
  A second function (`write_to_directory_eagerly`) does the same with a thread.
* `gix::init_bare(directory)` creates a bare repository (`src/lib.rs` line 328).
* `gix-negotiate 0.38.0` (513 lines) is the client side of negotiation (the algorithms that choose
  `have` lines). It does not decide ACK or NAK on the server.

### 3.3 Other native servers

| Project | What the source says | Mark |
| --- | --- | --- |
| `ggueret/git-server` (MIT, 4 stars) | "Serves bare repositories over HTTP for `git clone` and `git fetch`. No `git` binary at runtime. Built with gitoxide." The endpoints are upload-pack only. No push. The page does not say how it writes packs. | read: <https://github.com/ggueret/git-server>, 2026-11-11 |
| `git-internal 0.10.4` (MIT, 27,647 downloads) | A library of git objects and packs with delta compression and "smart protocol support". Its dependency list has `axum`, `tokio`, `flate2`, `rayon`, `rkyv`, `sha1`, `sha2`, `zstd-sys` (a C build) and about 20 more crates. The docs name v1 only. | read: crates.io API, 2026-11-11; <https://docs.rs/crate/git-internal/0.8.5/source/docs/git-protocol-guide.md> (search result, 0.8.5) |
| `guts-git 0.1.0` (23 KB, 205 downloads, `MIT OR Apache-2.0`) | A small crate with smart HTTP handlers. Too young to depend on. | read: crates.io API, 2026-11-11 |
| `git2` (libgit2) | A C build. It has no server. The bundled library is GPL-2.0 with a linking exception. | read: `capabilities.md`, section 5.3 (not re-read) |
| `go-git` | Go. It has a server transport. It is no Rust option. | read: search result, <https://github.com/go-git/go-git/issues/234> |

**inferred**: no project gives a complete, maintained, push-capable server on `gix`. The closest is
fetch-only with 4 stars. podnet writes the server. It may read these projects as examples.

## 4. The crates, the size and the license

### 4.1 Lock-file counts (measured)

Method: a scratch project in the scratchpad (not in the repository). One `cargo generate-lockfile`,
then `cargo tree --target x86_64-unknown-linux-musl -e normal --prefix none | sort -u`. The count
includes the scratch crate itself. Build scripts and proc-macro crates are in the normal graph.

| Variant | Lock packages | Normal graph (musl) | `gix*` source lines of the graph |
| --- | --- | --- | --- |
| A: `gix = "=0.90.0"`, `default-features = false` | 113 | 88 | 128,861 |
| B: `gix = "=0.90.0"`, default features | 164 | 136 | not counted |
| C: `gix`, `default-features = false`, `max-performance`, `sha1` | 129 | 95 | 128,861 |
| D: the data layer only: `gix-pack` (`generate`, `streaming-input`), `gix-odb`, `gix-ref`, `gix-packetline`, `gix-object`, `gix-traverse`, `gix-negotiate` | 92 | 64 | 63,900 |

* The lock of podnet has 415 packages (measured, `grep -c '^name = ' Cargo.lock`).
* In variant D, 24 crate names are in the lock of podnet already: `bitflags byteorder cfg-if fastrand
  getrandom hash32 hashbrown heapless itoa libc linux-raw-sys lock_api memchr once_cell parking_lot
  parking_lot_core regex-automata rustix rustversion same-file scopeguard smallvec stable_deref_trait
  tempfile tinyvec walkdir`. So variant D adds about 39 new names (inferred: versions can differ, so
  some of the 24 can come twice).
* Variant A has no SHA-1 implementation: a repository needs the feature `sha1`. Variants B and C
  have `sha1dc 0.1.5`. Its `Cargo.toml` has `build = false` and no C dependency (**read**, registry
  copy). So the hash is pure Rust.
* The measured graphs have no `ring`, `aws-lc-sys`, `openssl`, `cc`, `cmake`, `reqwest`, `curl` or
  `rustls`. The only compression crate is `zlib-rs 0.6.8` (pure Rust). So the build needs no C
  compiler for this part.

### 4.2 The size of the binary

**Not measured.** A binary needs the real server code, because the linker removes unused code.
The numbers that exist are source lines (**measured**, `wc -l` of `src/**/*.rs` in the registry
copy): `gix-pack` 11,262, `gix-odb` 5,608, `gix-ref` 6,446, `gix-object` 6,313, `gix-traverse` 1,899,
`gix-packetline` 2,066, `gix-negotiate` 513, `gix-protocol` 3,918, `gix-transport` 6,619, `gix` 33,548.
**Inferred**: variant D adds less than 1 MB to a release binary. The basis is the line count and the
size of similar crates in other studies (`iroh-relay.md` did not measure either). A spike must
measure it (proposed entry G1).

### 4.3 License

All `gix*` crates that were read are `MIT OR Apache-2.0` (**read**, crates.io API, 2026-11-11):
`gix`, `gix-pack`, `gix-packetline`, `gix-protocol`, `gix-transport`, `gix-negotiate`, `gix-odb`,
`gix-ref`, `gix-hash`, `gix-features`, `gix-object`, `gix-traverse`. `sha1dc` is `MIT OR Apache-2.0`.
The terms need a notice in a binary release. **Inferred**: `cargo deny` accepts them, because podnet
uses other `MIT OR Apache-2.0` crates. It was not run.

### 4.4 ring or aws-lc

* **measured**: no variant links a TLS or crypto-provider crate. The server needs the hash of
  git objects only. podnet gives the TLS for HTTPS (`rustls` with ring) and the SSH crypto (`russh`
  with ring) already.
* **read**: the TLS features of `gix` are `blocking-http-transport-reqwest-rust-tls` and
  `blocking-http-transport-curl-rustls`. They belong to the HTTP client of `gix-transport`
  (`Cargo.toml` of `gix-transport 0.62.0` and `gix 0.90.0`, registry copy).
* **inferred**: podnet must not turn these features on. A client feature of `gix` would pull `reqwest`,
  and a `reqwest` with `rustls` can pick aws-lc as its provider. The lock of podnet has no `aws-lc-sys`
  and must keep this (`CLAUDE.md`, ring rule of `iroh-relay.md`, section 1).

## 5. What podnet must write

Estimates are **inferred** from the number of protocol states and from the size of the existing code.
They are lines of Rust without tests.

| Part | What it does | Lines |
| --- | --- | --- |
| P1 Ref advertisement | v0 and v1 ref list with capabilities on the first ref, `HEAD` first, symref, peeled tags `^{}`; the v2 capability advertisement; the `# service=` header for HTTP | 250 |
| P2 `ls-refs` (v2) | `ref-prefix`, `symrefs`, `peel`, `unborn` | 150 |
| P3 Upload-pack v0 and v1 | `want` and `have` lines, `multi_ack_detailed`, `no-done`, `side-band-64k`, `ofs-delta`, `thin-pack`, `include-tag`, `shallow` and `deepen` (optional), the stateless-rpc form for HTTP | 600 |
| P4 Fetch (v2) | the command and the arguments of section 2.1, the acknowledgments section and the `packfile` section, `ready`, `done` | 450 |
| P5 Want validation | refuse a `want` that no ref reaches (HTTP rule); compute the common base from the `have` set with a walk of `gix-traverse`/`gix-revwalk` | 250 |
| P6 Pack streaming | `count::objects`, `iter_from_counts`, `bytes` in a blocking thread; progress on band 2; backpressure to the async writer | 250 |
| P7 Receive-pack v0 and v1 | the command list with `old new ref`, `report-status`, `report-status-v2`, `delete-refs`, `atomic`, `push-options`, `ofs-delta`, `side-band-64k`, `quiet`; refuse a ref outside `refs/` | 500 |
| P8 Push ingest and checks | read the pack into a temporary directory under `objects/` with `Bundle::write_to_directory` and a thin-pack lookup; check that each new object decodes; check connectivity (each new ref tip reaches only known objects); move the pack into place only on success (own quarantine) | 400 |
| P9 Ref update | `gix-ref` transaction with the old value check for each command; one result line for each ref (`ok`, `ng` with a reason); `receive.denyNonFastForwards`, `denyDeletes`, `denyCurrentBranch` for a non-bare repository | 250 |
| P10 Transport glue | gzip and chunked HTTP bodies; the `Git-Protocol` header; SSH `GIT_PROTOCOL`; the bridge from `async` readers to the blocking `gix` calls (`spawn_blocking` with channels) | 300 |
| P11 Maintenance | after a push: if the pack has few objects, unpack them to loose objects (git's `receive.unpackLimit`); a repack or `gc` pass to keep the pack count low; `gix-pack` has no repack and no delta search | 300 |
| P12 `git-upload-archive` | a tar or zip stream of a tree with `gix-archive` (feature `worktree-archive`); or drop the service | 150 |
| Total | | 3,850 |

P12 is optional. Without P12, the total is 3,700. These numbers are rough.

The existing code stays: the SSH server (`ssh.rs` except `run_git`), the router and the path rules
(`http.rs`, `mod.rs`), the key and password gates, and `create_repo` (it can call `gix::init_bare`).
The CGI runner (`cgi.rs`) stays for the backend with the `git` program.

### 5.1 What is missing in `gix` and has a cost

* **Delta compression** (read). A clone from a repository that has only loose objects sends each
  object whole (zlib only). A repository that `git` or a pack from a push made has deltas, and
  `PackCopyAndBaseObjects` copies them. So the cost is real for a first clone of a fresh repository
  and for a repository that grows by many small pushes. P11 reduces it only if it can repack with
  deltas, which `gix` cannot do. A repack by `git gc` needs the `git` program.
* **Hooks** (read). `gix` has no hook runner. A hook is a program. A host with no programs has no hooks
  to run in practice. A native server can run `hooks/pre-receive`, `update` and `post-receive` with
  `tokio::process` (about 150 lines, inferred) and skip them when the file is missing.
* **fsck on receive** (read: `fsck` rows are unchecked). The check of P8 is "decodes and connects".
  It is weaker than `receive.fsckObjects`. A malformed tree is accepted.
* **Bitmaps and the commit-graph** (read: bitmap not done). Counting objects for a big repository is
  slower than `git`. **Inferred**: it matters above some size, not measured.
* **SHA-256** (read: "Git 3.0 compatibility" unchecked). The server speaks `object-format=sha1`.

## 6. Options

| Option | What it is | Cost | Use |
| --- | --- | --- | --- |
| A. Keep the `git` program | the code of today | 0 | works where `git` is installed |
| B. Native on `gix`, all parts | P1 to P12 | about 3,700 lines, about 39 new crates, no C | the full replacement |
| C. Native upload-pack only | P1 to P6, P10; push is refused or goes to the `git` program | about 1,900 lines | read-only hosting and mirrors; covers most of "serve a repository to clients" |
| D. Ship a static `git` | a build of `git` as a second static binary, started from a temporary file | build and size work; a second binary; GPL-2.0 for `git` | **inferred**: it meets "no program on the host", but it is not one binary and `git` is GPL-2.0 (a code license check) |
| E. Take a crate | `ggueret/git-server` or `git-internal` | a young fetch-only crate, or a crate with 40 dependencies and a C build | not advised (section 3.3) |

## 7. Recommendation

**Inferred.** Do option C first, then the rest of B behind the same trait, and keep A as the default
while it works.

1. Make one trait for the backend: `serve(service, protocol, repo, reader, writer)`. The `git`
   program backend is today's code. The native backend is new. A flag `--git-backend git|native|auto`
   picks. `auto` uses the program when `git --version` runs, and the native code when it does not.
2. Write upload-pack first (P1 to P6, P10). It has the widest use (a clone from a host with no
   `git`), the least risk (read-only), and a test that needs only a `git` client. Test it against
   the `git` client of the test host for v0, v1 and v2, and compare the bytes of the advertisement
   with `git upload-pack --advertise-refs` (the existing tests already do this).
3. Write receive-pack second (P7 to P9, P11). It writes to the repository, so its test must include
   a push that fails the connectivity check, a push of a thin pack, a delete, a rejected
   non-fast-forward and a push of 8 MiB over a chunked body.
4. Do not drop the `git` backend. `gix` has no delta compression, no hooks and no fsck. A host that
   has `git` gets better packs from it.
5. Do a spike (G1) before any decision on size: link the data layer in a scratch binary with a
   stub server and measure the release size for `x86_64-unknown-linux-musl`.

`auto` keeps the rule "no feature or capability may be lost" (`CLAUDE.md`, Decisions): a host with
`git` loses nothing. A host with no `git` gains upload-pack, and push once G3 is done.

## 8. Questions for the owner

1. Accept the `git`-or-native `auto` backend (the trait, section 7)?
2. Accept that the native backend has no hooks, no `fsck` and weak packs at first, and say so in
   `podnet doctor` (it reports the backend in use)?
3. Is `git-upload-archive` needed without `git`? If not, drop P12.
4. May podnet add about 39 crates to the lock for the native backend (variant D, section 4.1)? Or does
   the owner want the data layer in a crate of `PodUtils/podcrates` (for example `pod-git-serve`)?
   The rule "code that a crate of podcrates will hold is not developed in podnet" decides this.

## 9. Proposed entries

The lead opens these. Each needs the ruling of the owner on the question that is named.

| Id | Title | Depends on | Effort | Prove |
| --- | --- | --- | --- | --- |
| G1 | Spike: link `gix-pack`, `gix-odb`, `gix-ref`, `gix-packetline` in a scratch binary; measure the release size (musl) and the lock | question 4 | S | A report in `docs/research/podnet/`; no code in the workspace |
| G2 | Backend trait for `podnet git serve` (`--git-backend git\|native\|auto`); the `git` program is the first backend; `doctor` reports the backend | question 1 | M | `cargo test --workspace -- git_` passes with the program backend; a test with `PATH` empty gives a clear fault |
| G3 | Native upload-pack: P1 to P6 and P10 (v0, v1, v2; HTTP and SSH) | G2; G1 | L | `git clone` and `git fetch` (v0, v1, v2, and shallow) from a native server on a test host; the advertisement equals `git upload-pack --advertise-refs`; the tests run with no `git` on the server side |
| G4 | Native receive-pack: P7 to P9 and P11 | G3; question 2 | L | A push, a thin push, a delete, a rejected non-fast-forward, a failed connectivity check and a chunked push of 8 MiB; the repository passes `git fsck` after each |
| G5 | Native hooks and `git-upload-archive` (P12 and the hook runner) | G4; question 3 | M | A `pre-receive` hook that refuses a push; `git archive --remote` through SSH |
| G6 | Pack quality: a repack with deltas, by `git gc` when `git` exists, or by own code | G4 | M | The size of a clone after 100 small pushes is within 20 % of the size from `git` |

## 10. Not verified

* The release binary size with the native backend (needs G1).
* The speed of `count::objects` and of the pack writer on a large repository, and the effect of the
  missing bitmaps.
* That `cargo deny` accepts the 39 new crates (not run).
* The wire form of protocol v2 over HTTP and the gzip request body (the pages that were read do not
  describe them; the existing tests of podnet do).
* The line counts of section 5 (estimates).
* The claim that no complete push-capable server on `gix` exists. The search covered crates.io and the
  results of two web searches only. The issue list of `GitoxideLabs/gitoxide` was not read.
* The license of `git` as a second binary (option D) and the license of the bundled libgit2 were
  not re-read.

## 11. Sources

| URL | Read |
| --- | --- |
| <https://raw.githubusercontent.com/GitoxideLabs/gitoxide/main/crate-status.md> | 2026-11-11 |
| <https://crates.io/api/v1/crates/gix> and the pages of `gix-pack`, `gix-packetline`, `gix-protocol`, `gix-transport`, `gix-negotiate`, `gix-odb`, `gix-ref`, `gix-hash`, `gix-features`, `gix-object`, `gix-traverse`, `git-internal`, `guts-git` (same URL form) | 2026-11-11 |
| <https://git-scm.com/docs/protocol-v2> | 2026-11-11 |
| <https://git-scm.com/docs/gitprotocol-http> | 2026-11-11 |
| <https://raw.githubusercontent.com/git/git/master/builtin/receive-pack.c> | 2026-11-11 |
| <https://raw.githubusercontent.com/git/git/master/upload-pack.c> (fetched, 1,886 lines; not studied) | 2026-11-11 |
| <https://github.com/ggueret/git-server> | 2026-11-11 |
| <https://docs.rs/crate/git-internal/0.8.5/source/docs/git-protocol-guide.md> (search result only) | 2026-11-11 |
| <https://github.com/go-git/go-git/issues/234> (search result only) | 2026-11-11 |
| Registry copies: `gix 0.90.0`, `gix-pack 0.77.0`, `gix-protocol 0.68.0`, `gix-transport 0.62.0`, `gix-packetline 0.24.1`, `gix-negotiate 0.38.0`, `sha1dc 0.1.5` (same text as `https://docs.rs/crate/NAME/VERSION/source/`) | 2026-11-11 |

## Update of 2026-11-11 (the latest facts)

Read on 2026-11-11 (each source below).

- gix: no change (gix 0.90.0, gix-pack 0.77.0, 2026-10-22). Its maintainer closed two server pull
  requests on 2026-08-06 (#2465 upload-pack; #2887 upload-pack, receive-pack and a server transport)
  and asked the authors to keep a server in a fork. No delta compression in gix-pack (issue #2611).
- New: grit-lib 0.5.33 (MIT, GitButler, `gitbutlerapp/grit`, 2026-11-09) has
  `grit_lib::remote::serve::{UploadPack, ReceivePack}` over byte streams: upload-pack v0, v1, v2 with
  shallow and filters; receive-pack with a policy, hooks, a connectivity check; delta compression.
  A smoke test (a gnu build, git 2.43 as the client) passed clone v0, v1, v2, `--depth 1`, push,
  an incremental pull and `git fsck`; its pack of a loose-only test repository was smaller than that
  of `git upload-pack`. About 150 crates; 11.7 MB stripped for a small test binary (gnu). Risks: large
  server defects fixed from 2026-11-02 to 2026-11-10 (grit issues #1744, #2134, #2287, #2390, #2568),
  mostly written by agents, an API break before 0.6.
- Other young crates: mizzle, puregit, git-zero-server, alkgit (none tested). gunnar is AGPL-3.0
  (excluded). radicle-node spawns `git upload-pack`. libgit2 has no server and needs C. A static `git`
  exists only from a third-party packager (`darkvertex/static-git`, git 2.56.0).

The owner, 2026-11-11: no native server in podnet now. A shared crate `pod-git` of
`PodUtils/podcrates` holds it, with the contract and the acceptance of PodUtils/podnet#453.

Sources: https://github.com/GitoxideLabs/gitoxide/pull/2887, /pull/2465, /issues/2611,
https://raw.githubusercontent.com/GitoxideLabs/gitoxide/main/crate-status.md,
https://github.com/gitbutlerapp/grit (ROADMAP.md, issues), https://crates.io/crates/grit-lib,
https://github.com/willstott101/mizzle, https://github.com/KarpelesLab/puregit,
https://gunnar.rs/nordisk/gunnar, https://github.com/darkvertex/static-git,
https://github.com/libgit2/libgit2.
