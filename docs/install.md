# Install, update and verify

This page gives the ways to get `podnet`, to check a download, to update the binary and to
rebuild a release.

## Release binaries

Each [release](https://github.com/PodUtils/podnet/releases) has static
Linux binaries of `podnet` for `x86_64`, `aarch64`, `armv7`,
`riscv64gc`, `i686`, `arm` (ARMv6, hard float), `loongarch64` and
`powerpc64le`. podnet is for Linux only. This page describes `main`; the notes in `docs/releases/` say what
a tag has. Each release also has these files:

| File | Contents |
| --- | --- |
| `SHA256SUMS` | The SHA-256 of each binary, of `podnet.1`, of the llms files, of `pod-report.schema.json` and of `release-index.json`. |
| `SHA256SUMS.sigstore.json` | The keyless cosign signature of `SHA256SUMS`. |
| `provenance.sigstore.json` | The SLSA provenance of the binaries. |
| `podnet.1` | The manual page (`podnet man --roff`). |
| `llms.txt`, `llms-full.txt` | The index, and the manual with the documents, for agents. |
| `pod-report.schema.json` | The JSON Schema of `podnet pod --report` ([pods.md](pods.md)). |
| `THIRD-PARTY-LICENSES.md` | The license texts of the crates in the binaries. |
| `release-index.json` | In each release after `v0.4.0-beta.18`: each file with its SHA-256, its size and its target, the tag, the commit and the identity of the signer, in the schema of `pod-release` of PodUtils/podcrates. `SHA256SUMS` names it, so its signature covers it. |

Verify the files before you use them. Download the binary, `SHA256SUMS` and
the two bundles to one directory. Then do these steps:

1. Check the checksums. The command ignores the files that you did not
   download:

   ```sh
   sha256sum -c --ignore-missing SHA256SUMS
   ```

2. Check the signature of `SHA256SUMS`. Replace `vX.Y.Z` with the tag:

   ```sh
   cosign verify-blob --bundle SHA256SUMS.sigstore.json \
     --certificate-identity https://github.com/PodUtils/podnet/.github/workflows/release.yml@refs/tags/vX.Y.Z \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS
   ```

3. Check the provenance of the binary (gh 2.49 or later):

   ```sh
   gh attestation verify podnet-x86_64-unknown-linux-musl --repo PodUtils/podnet \
     --signer-workflow PodUtils/podnet/.github/workflows/release.yml
   ```

   With no network access to the GitHub API, give the bundle and a trusted
   root. Make `trusted_root.jsonl` on a host with network access
   (`gh attestation trusted-root > trusted_root.jsonl`), and copy it:

   ```sh
   gh attestation verify podnet-x86_64-unknown-linux-musl --repo PodUtils/podnet \
     --signer-workflow PodUtils/podnet/.github/workflows/release.yml \
     --bundle provenance.sigstore.json --custom-trusted-root trusted_root.jsonl
   ```

A binary that has one changed byte fails each check. The workflow
`.github/workflows/release-proofs.yml` does these checks on a fresh runner
after each release. It also does each check on a changed copy, which must
fail.

## Install script

`scripts/install.sh` installs the binary of the newest release
(prereleases too) for this machine. It is POSIX `sh` (`dash`, `bash
--posix`, `busybox sh`):

```sh
s=$(curl -fsSL https://raw.githubusercontent.com/PodUtils/podnet/main/scripts/install.sh) && sh -c "$s"
```

With wget: `s=$(wget -qO- URL) && sh -c "$s"`. Do not use `curl URL | sh`
in a script or a CI step: a pipe has the exit status of `sh`, and `sh`
exits 0 when `curl` fails or is missing.

The script is the installer core of PodUtils/podcrates
(`scripts/install/core.sh` and `scripts/install/core-net.sh` at the tag of `scripts/install/SOURCES`,
inline and unchanged) and the part of podnet after it. Each release also
has `podnet-install.sh`: the same script with the pins of the release (the
SHA-256 and the size of each file, written by
`scripts/install/pins.py stamp` in `release.yml`). Its line in
`SHA256SUMS` is under the cosign signature.

* The copy of the tree finds the release, gets `SHA256SUMS`, checks its
  cosign signature when `cosign` is installed, and takes the pin of the
  binary from its one line in `SHA256SUMS`. `podnet-install.sh` of a
  release has the pins in the file: it needs no API. When `cosign` is
  installed and the run is not `--offline`, it gets `SHA256SUMS` and its
  signature before the download, checks the signature (the certificate
  of `release.yml` at the tag), and compares each pin and the installer
  itself with the signed lines. A failed check stops the run.
* `PODNET_COSIGN=CMD` gives the cosign command; `PODNET_COSIGN=none` turns
  off the check of the signature (the pins and `SHA256SUMS` still check
  each file), and the run says so.
* A second run with the same version changes nothing and says so (`podnet
  VERSION is already installed`): a binary in place that agrees with its
  pin stays (its inode and its time too), and no binary is downloaded.
* A new binary goes to a temporary name in the install directory. Its
  SHA-256 must agree with the pin, and it must run (`--version`, with no
  stdin) and print `podnet VERSION` of the release. Only then does a rename
  put it in place, with mode 0755 (also with a restrictive umask). A file
  in place is old or new, never half. A binary that does not agree, does
  not run or prints another version does not replace the old one. When the
  path of the binary is a directory or another file that is not a regular
  file, the script stops before the downloads.
* The directory: `--prefix DIR`, else `PODNET_INSTALL_DIR`, else
  `/usr/local/bin` for root, else `$XDG_BIN_HOME`, else `~/.local/bin`.
  The script checks the directory before the downloads. A directory that
  cannot run programs (`noexec`) is refused with its reason, and the free
  space is checked with `df`. A line says how to add the directory to
  `PATH`; `--add-path FILE` adds the line to FILE once.
* The downloader: the first of `curl`, `wget`, `aria2c`, `http` (HTTPie),
  `python3`, `python` or `perl` (`HTTP::Tiny` with TLS), and last `busybox
  wget` (also a `wget` that is the busybox applet). `PODNET_DOWNLOADER=TOOL`
  selects one. An https URL allows no redirect to plain HTTP. busybox wget
  does not check TLS certificates: the script says so in a warning, and
  with no `cosign` the check of `SHA256SUMS` then does not protect the copy
  of the tree. A busybox with no TLS (no `ssl_client`) is not used.
* The SHA-256 tool: `sha256sum`, `shasum`, `openssl`, `busybox` or
  `python3`. With none, the script stops; `PODNET_NO_VERIFY=1` then
  installs with no check, with a warning.
* `PODNET_VERSION=<TAG>` or `--version <TAG>` (for example
  `v0.4.0-beta.1`) installs that release, with no request to the GitHub
  API. For the newest release, the script reads the API; when the API
  refuses (403 on a shared address), it reads `releases.atom`. A draft
  release is not used. `podnet-install.sh` of a release installs its own
  version only, and refuses another one with the name of its installer.
  `GITHUB_TOKEN` or `GH_TOKEN` goes to `api.github.com` only. The token is
  not in the trace of `DEBUG=1` and not in the arguments of the
  downloader: curl reads it from stdin, wget from a private file.
* `PODNET_REPO=OWNER/NAME` sets the repository of the releases.
  `PODNET_MIRROR=URL` gets each file from `URL/FILE` (`https://`,
  `http://` or `file://`); with no `cosign`, a warning says that nothing
  checks `SHA256SUMS` of a mirror. `PODNET_TARGET=TRIPLE` selects the
  binary of a target. The proxy comes from `https_proxy`, `HTTPS_PROXY`,
  `no_proxy` and `NO_PROXY`; the output shows it with no credential.
* `--dry-run` shows the plan and changes no file. The copy of the tree
  still gets `SHA256SUMS` (and the release list for the newest release),
  but no binary.
* `--offline DIR` (`podnet-install.sh` of a release only) installs from a
  directory with the files of the release, or from a `.tar.gz` of them,
  with no network.
* A missing `SHA256SUMS`, an empty download and a failed download have
  different messages. INT, TERM and HUP stop the script and remove its
  files (exit 130, 143, 129). After a KILL, the next run removes the
  temporary files of a run that does not run.
* Each line of the output starts with `podnet-install:`. The exit codes:
  0 done, 1 a fault, 2 a usage error.

The release of a version, with no API and no `SHA256SUMS` download:

```sh
curl -fsSLO https://github.com/PodUtils/podnet/releases/download/<TAG>/podnet-install.sh
sh podnet-install.sh
```

`sh scripts/install-test.sh SHELL` tests the script in one shell with a
fake release on the loopback and in a `file://` directory, and no network
(`cargo test --test install_script` runs it in `dash`, `bash --posix` and
`busybox sh`). `python3 -I scripts/check-install.py` checks that
`scripts/install/core.sh`, `scripts/install/core-net.sh` and `scripts/install/pins.py` have the SHA-256
of `scripts/install/SOURCES`, that `scripts/install.sh` holds the core
inline, and that it holds no pins.

## Ephemeral hosts

A hosted notebook, a rebuilt container or a CI job can lose its disk at
each start. Then three things go:

* The binary: give `PODNET_INSTALL_DIR` a directory that stays.
* The WARP device: give `PODNET_STATE_DIR` (or `--identity PATH`) a directory
  that stays. Without it, each start registers a new anonymous device (the
  service limits new devices; see [warp.md](warp.md)). `PODNET_STATE_DIR=none` uses a new
  device on purpose.
* The release list: a pinned `PODNET_VERSION` uses no GitHub API.

This cell (Python 3, no curl) installs podnet in `KEEP`. The same cell is in
`notebooks/install.ipynb`. It fails when the install fails (`check=True`):

```python
# Install podnet on a host that loses its disk at each start (a notebook).
# KEEP is a directory that stays, for example a mounted drive.
import os
import subprocess
import urllib.request

KEEP = os.environ.get("PODNET_KEEP", os.path.expanduser("~/podnet-keep"))
VERSION = "latest"  # A tag (for example v0.4.0-beta.1) uses no GitHub API.
URL = "https://raw.githubusercontent.com/PodUtils/podnet/main/scripts/install.sh"

os.makedirs(os.path.join(KEEP, "bin"), exist_ok=True)
script = os.path.join(KEEP, "install.sh")
with open(script, "wb") as f:
    f.write(urllib.request.urlopen(URL, timeout=60).read())
env = dict(os.environ, PODNET_INSTALL_DIR=os.path.join(KEEP, "bin"), PODNET_VERSION=VERSION)
subprocess.run(["sh", script], env=env, check=True)

# The binary and the WARP device stay in KEEP.
os.environ["PATH"] = os.path.join(KEEP, "bin") + ":" + os.environ["PATH"]
os.environ["PODNET_STATE_DIR"] = os.path.join(KEEP, "state")
print(subprocess.run(["podnet", "--version"], check=True, capture_output=True, text=True).stdout)
```

## Container image

Each release has an image for each Linux architecture of the binaries:
`ghcr.io/podutils/podnet:<tag>` (a manifest list), `:<tag>-<arch>` for one
architecture (`amd64`, `arm64`, `armv7`, `armv6`, `386`, `riscv64`,
`ppc64le`, `loong64`), and `:beta` for the newest prerelease. `:latest` is
for the newest release that is not a prerelease. No such release exists
now, so `:latest` has no image. The image is `FROM scratch` with `podnet`
in `/usr/local/bin`. It runs as user 65534. The state
directory is the volume `/var/lib/podnet` (`PODNET_STATE_DIR`). The entry point
is `podnet`. In the container, podnet is PID 1 (no init): SIGTERM and SIGINT
stop it at each stage, also before the ready line (exit 143 and 130).

```sh
docker run --rm ghcr.io/podutils/podnet:beta --version
docker run -d --name warp -v podnet-state:/var/lib/podnet -p 127.0.0.1:40000:40000 \
    ghcr.io/podutils/podnet:beta warp proxy --listen tcp://0.0.0.0:40000
docker run --rm ghcr.io/podutils/podnet:beta demo --exit-on-verify
```

The proxy listens on all addresses of the container: publish the port on
127.0.0.1 only, or give `--proxy-user` and `--proxy-password-file`.

Pin the image by digest. After `v0.4.0-beta.17`, the release notes of each release have the section "The image, by
digest": the digest of the manifest list and of each image of one architecture. The provenance of the manifest list
is attested in the registry. The binary in each image is the file of the release (the same SHA-256 as in
`SHA256SUMS`).

```sh
docker pull ghcr.io/podutils/podnet@sha256:DIGEST
gh attestation verify oci://ghcr.io/podutils/podnet@sha256:DIGEST --repo PodUtils/podnet
```

## Update

```sh
podnet update --check              # say whether a newer release exists
podnet update                      # install it
podnet update --version <TAG>      # install the release <TAG>
podnet update --rollback           # put the binary before the update back
```

`podnet update` installs a release only after these checks. The checks are
the checks of the crate `pod-release` of PodUtils/podcrates:

1. The bundle `SHA256SUMS.sigstore.json` signs `SHA256SUMS`: the digest and
   the signature are valid with the key of the leaf certificate.
2. The bundle has an entry of the Rekor log of Sigstore. Each proof of the
   entry is valid: the signed entry timestamp, and the inclusion proof with
   its signed checkpoint. Each RFC 3161 timestamp of the bundle is valid by
   the timestamp authority of Sigstore. A bundle with no entry of Rekor is
   refused.
3. The leaf certificate chains to the Fulcio roots of Sigstore at each time
   that the bundle proves, for code signing. It names `release.yml` of this
   repository at the tag of the release, and the issuer of GitHub Actions.
4. Exactly one valid line of `SHA256SUMS` names the binary, and the SHA-256
   of the binary is that line.
5. The new binary runs `--version` beside the old binary in 10 seconds, with
   no stdin. It must exit 0 and print `podnet VERSION` of the tag. After the
   run, the file must still be the same file (device and inode), a regular
   file of this user that other users cannot write.

The roots of Sigstore are compiled into the binary; the checks use no
network. Then the binary before is kept as `podnet.prev` beside it, and a
rename puts the new binary in place. The new binary gets the mode of the old
binary, with no write permission for the group and others and no set-user-ID
bit (0755 for a binary of mode 0755, also with a restrictive umask). When
`podnet.prev` cannot be written, the new binary is installed and a warning
says so.

`podnet update --rollback` puts `podnet.prev` back. The kept binary must pass
the same run first, and print `podnet VERSION`. The binary that was in place
becomes `podnet.prev`, so a second `--rollback` goes forward again. It uses no
network. A prerelease is installed only with `--version`, or when the current
binary is a prerelease.

Each failure prints one line and exits 69. A text that is not a tag exits 64.

* `--version TAG` uses no request to the GitHub API: the URLs of the files
  follow from the tag. A tag with no release exits 69 ("no release has the
  tag"). A text that is not a tag exits 64: a tag is `v` and a version of
  semver with no build part (`v0.4.0-beta.1`), with no `/`, no `..` and no
  character other than `0-9 A-Z a-z . + -`. podnet skips such a tag in the
  API and in `releases.atom` too.
* With no `--version`, podnet reads the GitHub API, else `releases.atom` (the
  API answers 403 on a shared address; the feed has no quota).
* `--version` of the installed version says so and exits 0; `--force`
  installs it again. With no `--version`, a binary that is newer than the
  newest release says so and exits 0. `--force` with no `--version` installs the newest
  release also when it is the installed version; it does not go back to
  an older release.
* The repository is fixed (it names the signer of the binary). podnet does
  not read `PODNET_REPO` (a variable of `scripts/install.sh`); a warning says
  so.

## Rebuild a release

`scripts/rebuild.sh TAG [TARGET]` rebuilds `podnet` of a
release from its tag, at another path, in the pinned image of
`images/build/Dockerfile` of the tag, with the command of `release.yml`.
The binary must have the SHA-256 of `SHA256SUMS`. TARGET is
`x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl` (the default is
the one of the machine). It needs git, curl and docker
(`CONTAINER=podman` for podman). `release-proofs.yml` runs it for both
targets after each release.

```sh
sh scripts/rebuild.sh <TAG>
```
