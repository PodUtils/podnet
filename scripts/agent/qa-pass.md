# The black-box QA pass of podnet

This file is the prompt of the black-box QA pass before each beta (T-454; `docs/maintenance.md`, Release, step 0).
Give it to a new agent, as it is. Fill in the three values of "The inputs". Each pass uses the same prompt, so two
passes can be compared.

## Your task

You are a tester of podnet. podnet is one static binary that brings its own network to restricted hosts. Find each
fault that a user can find with the binary and the documents only. Write each fault as an issue.

## The inputs

- VERSION: the version to test (for example `0.4.0-beta.18`).
- BINARY: the path of the release binary `podnet-x86_64-unknown-linux-musl` of VERSION, or of the artifact of `ci`
  for the commit of the dry run.
- DOCS: a directory with `README.md` and `docs/` of the same commit.
- PODLAB: the binary `podlab` of PodUtils/podcrates (`scripts/get-podlab.sh` of the repository gives it), when you
  have it.

## What you may use

- The binary: `podnet --help`, `podnet COMMAND --help`, `podnet man`, `podnet man exit-status`, the commands.
- The documents in DOCS, and the files that the commands write.
- The tools of the host (`curl`, `python3`, `ss`, `strace` when present).

## What you must not use

- No source code of podnet or of PodUtils/podcrates. Do not clone a repository. Do not read `src/`, `crates/`,
  `tests/`, `TODO/`, `scripts/` or the git history.
- No issue, pull request or record of podnet. Find the faults yourself; do not look for known ones.

## The host classes

Run each command as a user that is not root. When PODLAB is given, run each command also in the labs:
`PODLAB list` gives them, and `PODLAB run PROFILE -- COMMAND` runs a command in one. Give the most time to these:
`bind-denied` (no bind of AF_INET), `userns-off` (no user namespace), `proxy-only` (the only way out is a proxy),
`no-proxy-no-dns`, `no-loopback` and `noexec-tmp`. Put the binary in a directory that the lab user can read, not in
`/tmp` or HOME.

## What to check

For each command of `podnet --help`:

1. The help: each flag that the help names exists, and each flag does what the help says. Each example of the help
   and of the documents runs as written.
2. The documents: each promise of the topic document of the command (`docs/*.md`) holds. Note the file and the line of
   each promise that you check.
3. The error paths. For each command that dials (`pod`, `connect`, `forward`, `net proxy`, `relay connect`,
   `warp proxy`, `bench`, `tunnel`): a dead proxy (`127.0.0.1:1`), a refused port, a name that does not resolve (a
   name under `.invalid`), an unreachable address (`192.0.2.1`), and a reject rule where the command has rules. Each
   must end within a time limit (no hang), with the exit code that `podnet man exit-status` documents, and with one
   error line that names each cause once and says what to do.
4. The outputs: each `--json` output parses, and the same keys mean the same thing in each command. Each exit code is
   one that `podnet man exit-status` documents. No output has a panic or a Rust backtrace.
5. The background: `--detach`, `podnet ps`, `podnet stop`, `podnet logs` on each command that serves.
6. A command that waits: a stop signal (`SIGTERM`, `SIGINT`) ends it at once and cleanly.

## How to write a finding

Write each fault as one issue of PodUtils/podnet, titled `[qa VERSION] COMMAND: the fault in one line`. The body has:

- The host: the class (the lab profile, or "plain host"), the user, the kernel (`uname -r`).
- The command, exactly as you ran it, and its full output and exit code.
- What the help or the document promises: a quote, with the file and the line, or the flag of the help.
- What happened instead.
- Whether it happens each time.

One fault for each issue. Do not write a fix or a guess of the cause. A fault of the documents (a promise that is
wrong, an example that does not run) is a fault too.

## The end of the pass

Write a last comment on the first issue of the pass (or, with no finding, one issue titled
`[qa VERSION] no finding`): each command that you checked, each host class that you used, the count of the findings,
and the time that you spent. The release of VERSION waits until each finding is an entry of the record of podnet.
