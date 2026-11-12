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
| `SHA256SUMS` | The SHA-256 of each binary and of `podnet.1`. |
| `SHA256SUMS.sigstore.json` | The keyless cosign signature of `SHA256SUMS`. |
| `provenance.sigstore.json` | The SLSA provenance of the binaries. |
| `podnet.1` | The manual page (`podnet man --roff`). |
| `llms.txt`, `llms-full.txt` | The index, and the manual with the documents, for agents. |
| `THIRD-PARTY-LICENSES.md` | The license texts of the crates in the binaries. |

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
(prereleases too) for this machine. It is POSIX `sh`:

```sh
s=$(curl -fsSL https://raw.githubusercontent.com/PodUtils/podnet/main/scripts/install.sh) && sh -c "$s"
```

With wget: `s=$(wget -qO- URL) && sh -c "$s"`. Do not use `curl URL | sh`
in a script or a CI step: a pipe has the exit status of `sh`, and `sh`
exits 0 when `curl` fails or is missing.

* It uses the first downloader that it finds: `curl`, `wget`, `aria2c`,
  `http` (HTTPie), `python3`, `python` or `perl` (`HTTP::Tiny` with TLS),
  and last `busybox wget` (also a `wget` that is the busybox applet: it
  goes after `python3`, `python` and `perl`).
  HTTPS only; redirects to plain HTTP are refused. busybox wget does not
  check TLS certificates: the script says so in a warning, and with no
  `cosign` the check of `SHA256SUMS` then does not protect the download. A
  busybox with no TLS (no `ssl_client`) is not used.
* It checks the binary against `SHA256SUMS` (`sha256sum`, `shasum`,
  `openssl`, `busybox` or `python3`), and the cosign signature of
  `SHA256SUMS` when `cosign` is installed. A failed check stops it, and
  cosign then shows its output.
* It runs the new binary (`--version`) in the install directory before it
  replaces the old binary. A binary that does not run on this machine, or
  that does not print `podnet VERSION` of the release, does not replace the
  old one. The copy is a new file from `mktemp` with mode 0755 (also with
  a restrictive umask). When the path of the binary is a directory or
  another file that is not a regular file, the script stops before the
  downloads.
* The directory: `PODNET_INSTALL_DIR`, else `/usr/local/bin` for root, else
  `$XDG_BIN_HOME`, else `~/.local/bin`. A line says how to add it to `PATH`.
  The script checks the directory before the downloads.
* `PODNET_VERSION=<TAG>` (for example `v0.4.0-beta.1`) installs that release, with no request to
  the GitHub API. For the newest release, the script reads the API; when
  the API refuses (403 on a shared address), it reads `releases.atom`.
  A draft release is not used.
  `GITHUB_TOKEN` or `GH_TOKEN` goes to `api.github.com` only. The token is
  not in the trace of `DEBUG=1` and not in the arguments of the downloader:
  curl reads it from stdin, wget from a private file.
* `PODNET_DOWNLOADER=wget` selects the downloader.
* A missing file of the release, an empty download and a failed download
  have different messages. INT, TERM and HUP stop the script and remove its files.

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

## Update

```sh
podnet update --check              # say whether a newer release exists
podnet update                      # install it
podnet update --version <TAG>      # install the release <TAG>
podnet update --rollback           # put the binary before the update back
```

`podnet update` installs a release only after these checks: the certificate
of `SHA256SUMS.sigstore.json` chains to the Fulcio roots of Sigstore and
names `release.yml` of this repository at the tag, the signature over
`SHA256SUMS` is valid, the SHA-256 of the binary is its line in
`SHA256SUMS`, and the new binary runs `--version` in 10 seconds, with no
stdin, exits 0 and prints the version of the tag. After the run, the file
must still be the same file (device and inode), a regular file of this
user that other users cannot write; else podnet stops. Then the binary before
is kept as `podnet.prev` beside it, and a rename puts the new binary (mode
0755, also with a restrictive umask) in place. `podnet update --rollback` puts `podnet.prev` back (it must pass the
same run first) and keeps the binary that was in place as `podnet.prev`;
it uses no network. podnet does not check the inclusion proof of the Rekor
log. A prerelease is
installed only with `--version`, or when the current binary is a
prerelease.

* `--version TAG` uses no request to the GitHub API: the URLs of the files
  follow from the tag. A tag with no release exits 69 ("no release has the
  tag"). A text that is not a tag (a `/`, a `..`, a character other than
  `0-9 A-Z a-z . + -`) exits 64. podnet skips such a tag in the API and in
  `releases.atom` too.
* With no `--version`, podnet reads the GitHub API, else `releases.atom` (the
  API answers 403 on a shared address; the feed has no quota).
* `--version` of the installed version says so and exits 0; `--force`
  installs it again. `--force` with no `--version` installs the newest
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
