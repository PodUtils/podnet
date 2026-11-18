#!/bin/sh
# SPDX-License-Identifier: 0BSD
# Install the static Linux binary of podnet from a release (docs/install.md).
#
#   s=$(curl -fsSL https://raw.githubusercontent.com/PodUtils/podnet/main/scripts/install.sh) && sh -c "$s"
#   s=$(wget -qO- https://raw.githubusercontent.com/PodUtils/podnet/main/scripts/install.sh) && sh -c "$s"
#   sh podnet-install.sh            (the copy of a release, with its pins)
#
# Do not use `curl ... | sh` in a script: a pipe has the exit status of sh,
# and sh exits 0 when curl fails.
#
# The script is POSIX sh. It is the installer core of PodUtils/podcrates
# (scripts/install/core.sh and core-net.sh, inline below, unchanged) and the part of podnet
# after it. Each release publishes podnet-install.sh: this file with the
# pins of the release (scripts/install/pins.py stamp). This copy of the
# tree has no pins: it finds the release (PODNET_VERSION, else the newest,
# also a prerelease) and takes the pin of the binary from SHA256SUMS. Then
# the core checks the cosign signature of SHA256SUMS when cosign is
# installed (each pin must agree with the signed lines), and does the
# install: a file in place that agrees with its pin stays,
# and a new file goes to a temporary name in the directory, must agree
# with its pin, must run and print "podnet VERSION", and only then does a
# rename put it in place.
#
# Variables (sh install.sh --help gives the options):
#   PODNET_VERSION       The release: latest (the newest, also a prerelease;
#                        the default), or a tag (v0.3.0-beta.7, or
#                        0.3.0-beta.7). latest reads the GitHub API, else
#                        releases.atom (no quota). A tag needs neither.
#   PODNET_INSTALL_DIR   The directory of the binary. Default: /usr/local/bin
#                        for root, else $XDG_BIN_HOME, else ~/.local/bin.
#   PODNET_REPO          OWNER/NAME of the releases (PodUtils/podnet). podnet
#                        update does not read it.
#   PODNET_NO_VERIFY=1   Install with no SHA-256 check (no tool for it).
#   PODNET_DOWNLOADER    Use this downloader: curl, wget, aria2c, http,
#                        python3, python, perl or busybox.
#   PODNET_MIRROR        Get the files of the release from URL/FILE.
#   PODNET_TARGET        Use the binary of this target triple.
#   GITHUB_TOKEN         Sent to api.github.com only (curl, wget), for its
#   GH_TOKEN             quota on a shared address. Not sent to other hosts.
#   NO_COLOR             No colors (the output has no colors).
#   DEBUG                set -x. The trace does not show the token.
#
# INT, TERM and HUP stop the script and remove its files.
INST_NAME=podnet
INST_ENV=PODNET
INST_REPO=PodUtils/podnet

# BEGIN PINS (scripts/install/pins.py writes this block; do not edit it)
INST_VERSION=""
INST_TARGETS=""
# END PINS

# BEGIN CORE (scripts/install/pins.py puts scripts/install/core.sh here)
# SPDX-License-Identifier: 0BSD
# The core of each installer of PodUtils (docs/install.md). POSIX sh: it
# runs under dash, busybox sh, bash --posix, the sh of macOS and Git Bash.
# A project installer sets INST_NAME, INST_ENV and INST_REPO, holds the
# block of pins, includes this file and core-net.sh (scripts/install/pins.py
# puts them inline for a release), can replace the hooks inst_files,
# inst_selfcheck and inst_setup, and calls inst_main "$@".
#
# The rules: no root; no file outside the prefix; no file of the host read
# that the install does not need; a file goes to a temporary name in the
# prefix, its SHA-256 must agree with its pin, it must run, and then an
# atomic move puts it in place. A second run downloads no file that agrees.

# A stamped installer holds its version in the block of pins, which comes
# before the core. A copy that takes its pins at run time (a copy of the
# tree of a project, from the signed SHA256SUMS) holds none, and is no
# asset of a release.
# shellcheck disable=SC2034 # core-net.sh reads it.
inst_stamped=${INST_VERSION:+1}

inst_tag() { printf '%s-install' "$INST_NAME"; }
inst_say() { printf '%s: %s\n' "$(inst_tag)" "$*"; }
inst_warn() { printf '%s: warning: %s\n' "$(inst_tag)" "$*" >&2; }
inst_die() { printf '%s: error: %s\n' "$(inst_tag)" "$*" >&2; exit 1; }
inst_usage_die() { printf '%s: error: %s; give --help for the options\n' "$(inst_tag)" "$*" >&2; exit 2; }
inst_have() { command -v "$1" > /dev/null 2>&1; }
# The value of the variable INST_ENV_NAME (for example PODLAB_MIRROR).
inst_knob() { eval "printf '%s' \"\${${INST_ENV}_$1:-}\""; }
# The key of an asset name in the block of pins.
inst_key() { printf '%s' "$1" | tr -c 'A-Za-z0-9\n' '_'; }
inst_pin() { eval "printf '%s' \"\${INST_SUM_$(inst_key "$1"):-}\""; }
inst_size() { eval "printf '%s' \"\${INST_SIZE_$(inst_key "$1"):-0}\""; }

# The hooks. inst_files TARGET EXE: one line "NAME ASSET" for each file.
inst_files() { printf '%s %s\n' "$INST_NAME$2" "$INST_NAME-$1$2"; }
# The first line of FILE --version must be "INST_NAME VERSION".
# shellcheck disable=SC2317 # a project redefines each hook; the core calls it by name.
inst_version_ok() { case $1 in "$INST_NAME $INST_VERSION" | "$INST_NAME $INST_VERSION "*) return 0 ;; *) return 1 ;; esac; }
# The last step: a check of the installed program, with no network.
# shellcheck disable=SC2317 # a project redefines each hook; the core calls it by name.
inst_selfcheck() { return 0; }
# The setup of the project after the files (for example a config file).
# shellcheck disable=SC2317 # a project redefines each hook; the core calls it by name.
inst_setup() { return 0; }

inst_usage() {
    cat << EOF
usage: sh $(inst_tag).sh [OPTION...]

Install $INST_NAME $INST_VERSION for this user. No root is necessary.

  --prefix DIR      install into DIR (default: \$XDG_BIN_HOME, else
                    ~/.local/bin)
  --version VERSION install this version; this installer holds the pins of
                    $INST_VERSION only
  --dry-run         show the plan; change no file and download nothing
  --offline FILE    install from the bundle of a release (.tar.gz, or its
                    unpacked directory); no network
  --add-path FILE   add the PATH line to FILE (a shell profile)
  -h, --help        show this help

Variables:
  ${INST_ENV}_MIRROR=URL       download each file from URL/ASSET
  ${INST_ENV}_DOWNLOADER=TOOL  use TOOL: curl, wget, python3 or busybox
  ${INST_ENV}_TARGET=TRIPLE    use the file of this target
  ${INST_ENV}_COSIGN=CMD       check the signature with CMD (default: cosign), or none
  https_proxy, HTTPS_PROXY, no_proxy, NO_PROXY: the proxy
EOF
}

inst_parse() {
    inst_opt_prefix='' inst_opt_version='' inst_dry=0 inst_offline='' inst_addpath=''
    while [ $# -gt 0 ]; do
        case $1 in
            --prefix | --version | --offline | --add-path)
                [ $# -ge 2 ] || inst_usage_die "$1: give a value"
                case $1 in
                    --prefix) inst_opt_prefix=$2 ;;
                    --version) inst_opt_version=$2 ;;
                    --offline) inst_offline=$2 ;;
                    --add-path) inst_addpath=$2 ;;
                esac
                shift 2
                continue
                ;;
            --prefix=*) inst_opt_prefix=${1#*=} ;;
            --version=*) inst_opt_version=${1#*=} ;;
            --offline=*) inst_offline=${1#*=} ;;
            --add-path=*) inst_addpath=${1#*=} ;;
            --dry-run) inst_dry=1 ;;
            -h | --help) inst_usage; exit 0 ;;
            *) inst_usage_die "$1: unknown option" ;;
        esac
        shift
    done
}

# The target triple of this host. Under QEMU user mode, uname -m gives the
# emulated machine, which is the right file.
inst_detect_target() {
    inst_os=$(uname -s 2> /dev/null) || inst_os=unknown
    inst_machine=$(uname -m 2> /dev/null) || inst_machine=unknown
    inst_exe='' inst_target=''
    case $inst_os in
        Linux)
            case $inst_machine in
                x86_64 | amd64) inst_target=x86_64-unknown-linux-musl ;;
                aarch64 | arm64 | armv8*) inst_target=aarch64-unknown-linux-musl ;;
                armv7* | armhf) inst_target=armv7-unknown-linux-musleabihf ;;
                armv6* | arm) inst_target=arm-unknown-linux-musleabihf ;;
                i386 | i486 | i586 | i686 | x86) inst_target=i686-unknown-linux-musl ;;
                riscv64) inst_target=riscv64gc-unknown-linux-musl ;;
                ppc64le) inst_target=powerpc64le-unknown-linux-musl ;;
                loongarch64 | loong64) inst_target=loongarch64-unknown-linux-musl ;;
            esac
            ;;
        Darwin)
            # A shell under Rosetta says x86_64 on an arm64 Mac.
            if [ "$(sysctl -n hw.optional.arm64 2> /dev/null)" = 1 ]; then
                inst_target=aarch64-apple-darwin
            elif [ "$inst_machine" = x86_64 ]; then
                inst_target=x86_64-apple-darwin
            fi
            ;;
        MINGW* | MSYS* | CYGWIN* | Windows_NT)
            inst_exe=.exe
            case $inst_machine in
                x86_64 | amd64) inst_target=x86_64-pc-windows-msvc ;;
                aarch64 | arm64) inst_target=aarch64-pc-windows-msvc ;;
            esac
            ;;
    esac
    inst_v=$(inst_knob TARGET)
    if [ -n "$inst_v" ]; then
        inst_target=$inst_v
        case $inst_target in *windows*) inst_exe=.exe ;; *) inst_exe='' ;; esac
    fi
    [ -n "$inst_target" ] ||
        inst_die "no file for this host (uname -s: $inst_os, uname -m: $inst_machine); this release has: $INST_TARGETS"
}

# The first SHA-256 tool that a probe finds.
inst_detect_sha() {
    inst_sha=''
    for inst_t in sha256sum shasum openssl busybox python3; do
        inst_sha=$inst_t
        case $(inst_sha256 /dev/null 2> /dev/null) in
            e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855) return 0 ;;
        esac
    done
    inst_die "no SHA-256 tool: install sha256sum, shasum, openssl or python3"
}

inst_sha256() {
    case $inst_sha in
        sha256sum) sha256sum < "$1" ;;
        shasum) shasum -a 256 < "$1" ;;
        openssl) openssl dgst -sha256 -r < "$1" ;;
        busybox) busybox sha256sum < "$1" ;;
        python3)
            python3 -I -c 'import hashlib, sys
h = hashlib.sha256()
for b in iter(lambda: sys.stdin.buffer.read(1 << 20), b""):
    h.update(b)
sys.stdout.buffer.write(h.hexdigest().encode() + b"\n")' < "$1"
            ;;
    esac | cut -d ' ' -f 1
}

# Run a command in the background and wait, so that INT, TERM and HUP stop
# the run at once; the trap stops the child.
inst_wait() {
    "$@" < /dev/null &
    inst_child=$!
    wait "$inst_child"
    inst_rc=$?
    inst_child=''
    return "$inst_rc"
}

# The prefix: --prefix, else $XDG_BIN_HOME, else ~/.local/bin.
inst_resolve_prefix() {
    inst_default=1
    if [ -n "$inst_opt_prefix" ]; then
        inst_prefix=$inst_opt_prefix inst_default=0
    elif [ -n "${XDG_BIN_HOME:-}" ]; then
        inst_prefix=$XDG_BIN_HOME
    elif [ -n "${HOME:-}" ]; then
        inst_prefix=$HOME/.local/bin
    else
        inst_die "HOME is not set: give --prefix DIR"
    fi
    # A path of Windows (C:\...) under Git Bash, MSYS2 or Cygwin.
    case $inst_prefix in [A-Za-z]:*) ! inst_have cygpath || inst_prefix=$(cygpath -u "$inst_prefix") ;; esac
    case $inst_prefix in /*) ;; *) inst_prefix=$(pwd)/$inst_prefix ;; esac
    case $inst_prefix in
        *'"'* | *'$'* | *'`'* | *\\* | *'
'*) inst_die "$inst_prefix: the prefix has a quote, a dollar, a backslash or a newline: give --prefix DIR with no such character" ;;
    esac
    inst_prefix=${inst_prefix%/}
    inst_tmp=$inst_prefix/.$INST_NAME-install.$$
}

# The mount options of the file system of DIR, from /proc (empty when
# /proc is not there).
inst_mount_opts() {
    for inst_m in /proc/self/mounts /proc/mounts; do
        [ -r "$inst_m" ] || continue
        awk -v d="$1/" '
            { m = $2; if (m != "/") m = m "/" }
            index(d, m) == 1 && length(m) >= best { best = length(m); o = $4 }
            END { print o }' "$inst_m"
        return 0
    done
}

# Whether the prefix exists, takes a new file and runs a program. On a
# refusal, inst_why holds the text and inst_kind is make, write or run. A
# directory that this test made and that cannot be used goes again.
inst_try_prefix() {
    inst_new=0
    [ -d "$inst_prefix" ] || inst_new=1
    mkdir -p "$inst_prefix" 2> /dev/null || { inst_kind=make inst_why="cannot make $inst_prefix"; return 1; }
    inst_prefix=$(cd "$inst_prefix" && pwd -P) || { inst_kind=make inst_why="cannot open $inst_prefix"; return 1; }
    inst_tmp=$inst_prefix/.$INST_NAME-install.$$
    inst_made=1
    if ! { printf '#!/bin/sh\nexit 0\n' > "$inst_tmp.probe"; } 2> /dev/null; then
        [ "$inst_new" = 0 ] || rmdir "$inst_prefix" 2> /dev/null
        inst_kind=write inst_why="cannot write in $inst_prefix"
        return 1
    fi
    chmod 0755 "$inst_tmp.probe"
    if ! "$inst_tmp.probe" 2> /dev/null; then
        case ,$(inst_mount_opts "$inst_prefix"), in
            *,noexec,*) inst_why="its file system is mounted with noexec" ;;
            *) inst_why="a test program in it does not run" ;;
        esac
        rm -f "$inst_tmp.probe"
        [ "$inst_new" = 0 ] || rmdir "$inst_prefix" 2> /dev/null
        inst_kind=run
        return 1
    fi
    return 0
}

# The prefix exists, takes a new file, and runs a program. A default prefix
# that cannot run programs (a noexec mount) gives way to the next default
# (~/.local/bin, ~/bin), with a line; a prefix of --prefix never does.
inst_check_prefix() {
    if ! inst_try_prefix; then
        inst_first=$inst_prefix inst_firstwhy=$inst_why inst_firstkind=$inst_kind
        if [ "$inst_kind" = run ] && [ "${inst_default:-0}" = 1 ] && [ -n "${HOME:-}" ]; then
            for inst_alt in "$HOME/.local/bin" "$HOME/bin"; do
                [ "$inst_alt" != "$inst_first" ] || continue
                inst_prefix=$inst_alt
                if inst_try_prefix; then
                    inst_say "$inst_first cannot run programs ($inst_firstwhy): the install goes to $inst_prefix"
                    break
                fi
                inst_prefix=$inst_first
            done
        fi
        if [ "$inst_prefix" = "$inst_first" ]; then
            case $inst_firstkind in
                run) inst_die "$inst_first cannot run programs: $inst_firstwhy. Give --prefix DIR with a directory that can run programs" ;;
                *) inst_die "$inst_firstwhy: give --prefix DIR with a directory that you can write" ;;
            esac
        fi
    fi
    rm -f "$inst_tmp.probe"
    # The temporary files of a run that stopped with no cleanup (SIGKILL,
    # a power loss). A file of a run that still runs stays.
    for inst_f in "$inst_prefix/.$INST_NAME-install."*; do
        [ -e "$inst_f" ] || continue
        inst_v=${inst_f##*/."$INST_NAME"-install.}
        inst_v=${inst_v%%.*}
        case $inst_v in '' | *[!0-9]*) continue ;; esac
        [ "$inst_v" = $$ ] && continue
        kill -0 "$inst_v" 2> /dev/null && continue
        rm -rf "$inst_f"
        inst_say "removed $inst_f of a run that stopped"
    done
}

# The free space of the prefix, in KiB, against the need.
inst_check_space() {
    inst_have df || { inst_warn "no df: the free space of $inst_prefix is not checked"; return 0; }
    inst_free=$(df -Pk "$inst_prefix" 2> /dev/null | awk 'NR == 2 { print $4 }')
    case $inst_free in '' | *[!0-9]*) inst_warn "df gives no free space for $inst_prefix"; return 0 ;; esac
    [ "$inst_free" -ge "$1" ] ||
        inst_die "$inst_prefix has $inst_free KiB free; the install needs $1 KiB. Free space there, or give --prefix DIR on a file system with more space (the install writes no file outside the prefix)"
}

# The bundle of --offline, unpacked into the prefix once.
inst_bundle() {
    [ -z "${inst_bdir:-}" ] || return 0
    [ ! -d "$inst_offline" ] || { inst_bdir=$inst_offline && return 0; }
    [ -f "$inst_offline" ] || inst_die "$inst_offline: no such file: give --offline FILE with the bundle of a release"
    { inst_have gzip && inst_have tar; } || inst_die "no gzip or no tar: install them, or give --offline DIR with an unpacked bundle"
    inst_v=$(gzip -dc < "$inst_offline" 2> /dev/null | wc -c | tr -d ' ')
    inst_check_space $((inst_v / 1024 + inst_need))
    mkdir "$inst_tmp.bundle" || inst_die "cannot make $inst_tmp.bundle"
    gzip -dc < "$inst_offline" | (cd "$inst_tmp.bundle" && tar -xf -) ||
        inst_die "$inst_offline: not a bundle (.tar.gz): the unpack failed"
    inst_bdir=$inst_tmp.bundle
}

# Get ASSET to FILE (a temporary name in the prefix), and check its pin.
# shellcheck disable=SC2154 # core-net.sh sets inst_dl.
inst_get() {
    if [ -n "$inst_offline" ]; then
        inst_bundle
        for inst_f in "$inst_bdir/$1" "$inst_bdir"/*/"$1"; do
            [ -f "$inst_f" ] && break
        done
        [ -f "$inst_f" ] || inst_die "$1: not in the bundle $inst_offline"
        inst_wait cp "$inst_f" "$2" || inst_die "$1: cannot copy it from the bundle"
    else
        inst_v=$(inst_url "$1")
        inst_say "download $1"
        inst_fetch "$inst_v" "$2" || {
            rm -f "$2"
            inst_die "$1: the download from $inst_v failed ($inst_dl)"
        }
    fi
    inst_v=$(inst_sha256 "$2")
    if [ "$inst_v" != "$(inst_pin "$1")" ]; then
        rm -f "$2"
        inst_die "$1: the SHA-256 is $inst_v, not the pin $(inst_pin "$1"): the file is wrong or partial; no file was changed"
    fi
}

# Each file of the release for this target: NAME ASSET STATE, with STATE
# "ok" (present and agrees with its pin) or "get".
inst_plan() {
    inst_plan_text=''
    inst_need=0
    inst_list=$(inst_files "$inst_target" "$inst_exe")
    [ -n "$inst_list" ] || inst_die "the installer names no file"
    while read -r inst_n inst_a; do
        [ -n "$(inst_pin "$inst_a")" ] ||
            inst_die "$inst_a: no pin for the target $inst_target; this release has: $INST_TARGETS"
        inst_s='get'
        if [ -f "$inst_prefix/$inst_n" ] && [ "$(inst_sha256 "$inst_prefix/$inst_n")" = "$(inst_pin "$inst_a")" ]; then
            inst_s='ok'
        else
            inst_need=$((inst_need + $(inst_size "$inst_a") / 1024 + 1))
        fi
        inst_plan_text="$inst_plan_text$inst_n $inst_a $inst_s
"
    done << EOF
$inst_list
EOF
}

# The first line of FILE --version, in printable ASCII, in at most 30 s
# where timeout runs a command (timeout.exe of Windows does not).
inst_run_version() {
    if inst_have timeout && timeout 5 sh -c : 2> /dev/null; then
        timeout 30 "$1" --version < /dev/null 2> /dev/null
    else
        "$1" --version < /dev/null 2> /dev/null
    fi | head -n 1 | tr -cd ' -~'
}

inst_cleanup() {
    [ -z "${inst_child:-}" ] || kill "$inst_child" 2> /dev/null
    [ -z "${inst_made:-}" ] || rm -rf "$inst_tmp".*
}

inst_main() {
    inst_parse "$@"
    export LC_ALL=C
    unset CDPATH
    umask 022
    inst_made='' inst_child='' inst_bdir=''
    [ -n "$INST_VERSION" ] ||
        inst_die "this copy of the installer has no pins of a release: take $(inst_tag).sh of a release of $INST_REPO"
    if [ -n "$inst_opt_version" ] && [ "${inst_opt_version#v}" != "$INST_VERSION" ]; then
        inst_die "this installer holds the pins of $INST_VERSION, not ${inst_opt_version#v}: take $(inst_tag).sh of the release v${inst_opt_version#v}"
    fi
    trap inst_cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    trap 'exit 129' HUP
    inst_detect_target
    inst_resolve_prefix
    inst_detect_sha
    inst_plan
    inst_say "$INST_NAME $INST_VERSION for $inst_target into $inst_prefix"
    case $inst_plan_text in *" get
"*) inst_todo=1 ;; *) inst_todo=0 ;; esac
    [ "$inst_dry" = 1 ] || inst_check_prefix
    if [ "$inst_todo" = 1 ] && [ -z "$inst_offline" ]; then
        inst_proxy
        inst_detect_downloader
        inst_say "downloader: $inst_dl"
    fi
    if [ "$inst_dry" = 1 ]; then
        [ "$inst_todo" = 0 ] || inst_signature dry
        while read -r inst_n inst_a inst_s; do
            [ -n "$inst_n" ] || continue
            if [ "$inst_s" = ok ]; then
                inst_say "dry run: $inst_prefix/$inst_n agrees with its pin: no change"
            elif [ -n "$inst_offline" ]; then
                inst_say "dry run: $inst_a from the bundle $inst_offline to $inst_prefix/$inst_n"
            else
                inst_say "dry run: $(inst_url "$inst_a") to $inst_prefix/$inst_n"
            fi
        done << EOF
$inst_plan_text
EOF
        inst_say "dry run: no file was changed"
        return 0
    fi
    if [ "$inst_todo" = 1 ]; then
        inst_check_space $((inst_need + 1024))
        inst_signature
        # Each file to a temporary name, checked, then run once.
        while read -r inst_n inst_a inst_s; do
            [ "$inst_s" = get ] || continue
            inst_get "$inst_a" "$inst_tmp.$inst_n"
            chmod 0755 "$inst_tmp.$inst_n"
            # On macOS, curl, wget and python3 set no quarantine attribute; a
            # browser does, on a file of an --offline bundle.
            [ "$inst_os" != Darwin ] || xattr -d com.apple.quarantine "$inst_tmp.$inst_n" 2> /dev/null || true
            inst_v=$(inst_run_version "$inst_tmp.$inst_n")
            inst_version_ok "$inst_v" ||
                inst_die "$inst_a does not run on this host, or prints '$inst_v', not '$INST_NAME $INST_VERSION' (uname -m: $inst_machine); $inst_prefix/$inst_n is not changed"
        done << EOF
$inst_plan_text
EOF
        # Then the atomic moves: a file in place is old or new, never half.
        while read -r inst_n inst_a inst_s; do
            [ "$inst_s" = get ] || continue
            mv -f "$inst_tmp.$inst_n" "$inst_prefix/$inst_n" || inst_die "cannot move $inst_n into $inst_prefix"
            inst_say "installed $inst_prefix/$inst_n ($inst_a, sha256 $(inst_pin "$inst_a"))"
        done << EOF
$inst_plan_text
EOF
        rm -rf "$inst_tmp".*
    else
        inst_say "$INST_NAME $INST_VERSION is already installed in $inst_prefix: each file agrees with its pin; nothing was downloaded"
    fi
    inst_setup "$inst_prefix" || inst_die "the setup of $INST_NAME failed"
    inst_path_line
    inst_selfcheck "$inst_prefix" || inst_die "$INST_NAME is installed, but its self-check failed: the lines above give the fact of this host"
    inst_say "done"
}

inst_path_line() {
    inst_v="export PATH=\"$inst_prefix:\$PATH\""
    case ":${PATH:-}:" in
        *":$inst_prefix:"*) inst_say "$inst_prefix is in PATH" ;;
        *)
            inst_say "$inst_prefix is not in PATH. Add this line to your shell profile:"
            printf '    %s\n' "$inst_v"
            ;;
    esac
    [ -n "$inst_addpath" ] || return 0
    if [ -f "$inst_addpath" ] && grep -q -x -F -e "$inst_v" "$inst_addpath"; then
        inst_say "$inst_addpath has the PATH line already"
    else
        printf '%s\n' "$inst_v" >> "$inst_addpath" || inst_die "cannot write $inst_addpath"
        inst_say "added the PATH line to $inst_addpath"
    fi
}
# SPDX-License-Identifier: 0BSD
# The network part of the installer core (docs/install.md): the proxy, the
# downloader, the download, and the cosign signature of SHA256SUMS of the
# release. An installer includes it after core.sh; scripts/install/pins.py
# puts both inline for a release.
#
# The pins in the installer stay the first check of each file. Where
# cosign is there, the signature step adds: SHA256SUMS has a keyless
# signature of the release workflow of INST_REPO at the tag vINST_VERSION,
# issued to GitHub Actions; each pin of this target and this installer
# itself agree with SHA256SUMS. A mirror that changes the installer, its
# files and SHA256SUMS together then fails there.
# shellcheck disable=SC2154 # core.sh sets each inst_ variable.

# The proxy comes from the environment. curl, wget, python3 and busybox
# wget read it there: it never goes into argv or a file.
inst_proxy() {
    [ -n "${https_proxy:-}" ] || [ -z "${HTTPS_PROXY:-}" ] || export https_proxy="$HTTPS_PROXY"
    [ -n "${no_proxy:-}" ] || [ -z "${NO_PROXY:-}" ] || export no_proxy="$NO_PROXY"
    # One line for each proxy: the two variables often name the same one.
    inst_seen=''
    for inst_v in "${https_proxy:-}" "${http_proxy:-}"; do
        if [ -z "$inst_v" ] || [ "$inst_v" = "$inst_seen" ]; then continue; fi
        inst_seen=$inst_v
        # The credential of the proxy is not shown.
        inst_say "proxy: $(printf '%s' "$inst_v" | sed 's#^\([A-Za-z0-9+.-]*://\)[^@/]*@#\1#')"
    done
}

# The first downloader that a probe finds. busybox wget is the last one:
# some builds do not check TLS certificates.
inst_detect_downloader() {
    inst_dl='' inst_bbwget=''
    inst_v=$(inst_knob DOWNLOADER)
    for inst_t in ${inst_v:-curl wget python3 busybox}; do
        case $inst_t in
            curl) inst_have curl || continue ;;
            wget)
                inst_have wget || continue
                if wget --help 2>&1 | grep -q BusyBox; then
                    [ -n "$inst_v" ] || continue
                    inst_t=busybox inst_bbwget=wget
                else
                    inst_wget_opts=''
                    ! wget --help 2>&1 | grep -q -e '--no-config' || inst_wget_opts=--no-config
                fi
                ;;
            python3) python3 -I -c 'import urllib.request' > /dev/null 2>&1 || continue ;;
            busybox)
                if busybox wget --help > /dev/null 2>&1; then
                    inst_bbwget='busybox wget'
                elif inst_have wget && wget --help 2>&1 | grep -q BusyBox; then
                    inst_bbwget=wget
                else
                    continue
                fi
                ;;
            *) inst_usage_die "${INST_ENV}_DOWNLOADER=$inst_t: give curl, wget, python3 or busybox" ;;
        esac
        inst_dl=$inst_t
        break
    done
    if [ -z "$inst_dl" ]; then
        [ -z "$inst_v" ] || inst_die "${INST_ENV}_DOWNLOADER=$inst_v: the tool is not usable on this host"
        inst_die "no downloader: install curl, wget or python3, or give --offline FILE"
    fi
    [ "$inst_dl" != busybox ] ||
        inst_warn "busybox wget can download with no check of the TLS certificate; the SHA-256 pins of this installer still check each file"
}

# Download URL to FILE.
# shellcheck disable=SC2086 # inst_wget_opts and inst_bbwget are lists of words.
inst_fetch() {
    case $1 in
        file://*) inst_wait cp "${1#file://}" "$2"; return ;;
        https://*) inst_p='=https' ;;
        *) inst_p='=http,https' ;;
    esac
    case $inst_dl in
        curl) inst_wait curl -q -fsSL --proto "$inst_p" --proto-redir "$inst_p" --retry 3 --connect-timeout 30 -o "$2" "$1" ;;
        wget) inst_wait wget $inst_wget_opts -q -t 3 -T 60 -O "$2" "$1" ;;
        busybox) inst_wait $inst_bbwget -q -O "$2" "$1" ;;
        python3)
            inst_wait python3 -I -c 'import shutil, sys, urllib.request
with urllib.request.urlopen(sys.argv[1], timeout=60) as r, open(sys.argv[2], "wb") as f:
    shutil.copyfileobj(r, f)' "$1" "$2" 2> /dev/null
            ;;
    esac 2> "$2.err"
    inst_rc=$?
    # The reason of the downloader, in printable ASCII: curl of Windows
    # ends each line with a CR.
    [ ! -s "$2.err" ] || tr -cd '\n -~' < "$2.err" >&2
    rm -f "$2.err"
    return "$inst_rc"
}

# The URL of ASSET: NAME_MIRROR/ASSET, else the release of INST_REPO.
inst_url() {
    inst_v=$(inst_knob MIRROR)
    printf '%s/%s' "${inst_v:-https://github.com/$INST_REPO/releases/download/v$INST_VERSION}" "$1" | sed 's#//*\([^/]*\)$#/\1#'
}

# The identity of the signer: the release workflow at the tag. A project
# with another name of its release workflow sets INST_WORKFLOW.
inst_signer() {
    printf 'https://github.com/%s/.github/workflows/%s@refs/tags/v%s' "$INST_REPO" "${INST_WORKFLOW:-release.yml}" "$INST_VERSION"
}
inst_issuer=https://token.actions.githubusercontent.com

# The cosign command: NAME_COSIGN, else cosign on PATH. Empty, with the
# reason in inst_cosign_why, when the signature is not checked.
inst_detect_cosign() {
    inst_cosign=$(inst_knob COSIGN) inst_cosign_why=''
    case $inst_cosign in
        none) inst_cosign='' inst_cosign_why="${INST_ENV}_COSIGN=none" ;;
        '') if inst_have cosign; then inst_cosign=cosign; else inst_cosign_why='cosign is not on PATH'; fi ;;
        *) inst_have "$inst_cosign" || inst_usage_die "${INST_ENV}_COSIGN=$inst_cosign: no such command; give the path of cosign, or none" ;;
    esac
}

# The SHA-256 of NAME in the file SUMS: empty unless exactly one line names it.
inst_sumline() {
    awk -v f="$1" '{ sub(/\r$/, "") } NF == 2 && ($2 == f || $2 == "*" f) { n++; h = tolower($1) } END { if (n == 1) print h }' "$2"
}

# inst_signature [dry]: check SHA256SUMS of the release and its signature
# before a download, or say once why there is no check. The plan
# (inst_plan_text) gives the files of this target.
inst_signature() {
    if [ -n "$inst_offline" ]; then
        inst_say "offline: the signature of SHA256SUMS is not checked; the pins of this installer check each file"
        return 0
    fi
    inst_detect_cosign
    if [ -z "$inst_cosign" ]; then
        inst_say "$inst_cosign_why: the signature of SHA256SUMS is not checked; the pins of this installer check each file"
        return 0
    fi
    if [ "${1:-}" = dry ]; then
        inst_say "dry run: $(inst_url SHA256SUMS) and its signature, checked with $inst_cosign for $(inst_signer)"
        return 0
    fi
    for inst_f in SHA256SUMS SHA256SUMS.sigstore.json; do
        inst_v=$(inst_url "$inst_f")
        inst_fetch "$inst_v" "$inst_tmp.$inst_f" ||
            inst_die "$inst_f: the download from $inst_v failed ($inst_dl); no file was changed. With cosign there, the signature is necessary: give ${INST_ENV}_COSIGN=none to install with the pins of this installer alone"
    done
    # cosign keeps its trust root in TUF_ROOT, else in ~/.sigstore: the
    # install writes no file outside the prefix.
    export TUF_ROOT="${TUF_ROOT:-$inst_tmp.tuf}"
    inst_wait "$inst_cosign" verify-blob --bundle "$inst_tmp.SHA256SUMS.sigstore.json" \
        --certificate-identity "$(inst_signer)" --certificate-oidc-issuer "$inst_issuer" \
        "$inst_tmp.SHA256SUMS" > "$inst_tmp.cosign.log" 2>&1 || {
        tail -n 5 "$inst_tmp.cosign.log" | tr -cd '\n -~' | sed 's/^/    /' >&2
        inst_die "the cosign signature of SHA256SUMS is not valid for $(inst_signer) (the lines above are the output of $inst_cosign); no file was changed"
    }
    while read -r _ inst_a _; do
        [ -n "$inst_a" ] || continue
        inst_v=$(inst_sumline "$inst_a" "$inst_tmp.SHA256SUMS")
        [ "$inst_v" = "$(inst_pin "$inst_a")" ] ||
            inst_die "$inst_a: the pin of this installer does not agree with the signed SHA256SUMS (${inst_v:-no line}); no file was changed"
    done << EOF
$inst_plan_text
EOF
    # This installer itself, when it runs from a file (not from a pipe).
    inst_v=$(inst_sumline "$(inst_tag).sh" "$inst_tmp.SHA256SUMS")
    case $0 in *.sh) [ -f "$0" ] || inst_v='-' ;; *) inst_v='-' ;; esac
    if [ -z "$inst_stamped" ]; then
        inst_say "this installer holds no pins of its own: it is not compared with $(inst_tag).sh of SHA256SUMS"
    elif [ -z "$inst_v" ]; then
        inst_say "SHA256SUMS has no line for $(inst_tag).sh: this installer itself is not compared"
    elif [ "$inst_v" != - ] && [ "$(inst_sha256 "$0")" != "$inst_v" ]; then
        inst_die "$0 is not $(inst_tag).sh of the signed SHA256SUMS: take the installer of the release again; no file was changed"
    fi
    inst_say "the signature of SHA256SUMS: ok ($inst_cosign, $(inst_signer))"
}
# END CORE

# The part of podnet. The core gives inst_say, inst_warn, inst_die, the
# target (inst_detect_target), the knobs (inst_knob) and inst_main.

# The hooks of the core.
inst_files() { printf '%s %s\n' "podnet$2" "podnet-$1$2"; }
inst_version_ok() { case $1 in "podnet $INST_VERSION" | "podnet $INST_VERSION "*) return 0 ;; *) return 1 ;; esac; }
# podnet needs no configuration file.
inst_setup() { return 0; }
# The self-check: the installed binary prints its version, with no network.
inst_selfcheck() {
    podnet_shown=$(inst_run_version "$1/podnet$inst_exe")
    inst_version_ok "$podnet_shown" || return 1
    printf '%s\n' "$podnet_shown"
}

podnet_usage() {
    cat << 'EOF'
usage: sh install.sh [OPTION...]

Install the podnet binary of a release for this machine.

  --prefix DIR      install into DIR (default: PODNET_INSTALL_DIR, else
                    /usr/local/bin for root, else $XDG_BIN_HOME, else
                    ~/.local/bin)
  --version VERSION install this release (as PODNET_VERSION)
  --dry-run         show the plan; change no file and download no binary
  --offline DIR     install from DIR (an unpacked bundle, or a directory
                    with the files of the release); a copy with the pins
                    of a release only; no network
  --add-path FILE   add the PATH line to FILE (a shell profile)
  -h, --help        show this help

Variables:
  PODNET_VERSION=TAG       the release (latest: the newest, the default)
  PODNET_INSTALL_DIR=DIR   the directory of the binary
  PODNET_REPO=OWNER/NAME   the repository of the releases
  PODNET_NO_VERIFY=1       install with no SHA-256 check (no tool for it)
  PODNET_DOWNLOADER=TOOL   curl, wget, aria2c, http, python3, python, perl
                           or busybox
  PODNET_MIRROR=URL        download each file from URL/FILE
  PODNET_COSIGN=CMD        check the signature with CMD (default: cosign),
                           or none
  PODNET_TARGET=TRIPLE     use the binary of this target
  GITHUB_TOKEN, GH_TOKEN   sent to api.github.com only
  DEBUG=1                  trace the script (the token is not shown)
  https_proxy, HTTPS_PROXY, no_proxy, NO_PROXY: the proxy
EOF
}

podnet_main() {
    if [ -n "${DEBUG:-}" ]; then set -x; fi
    for podnet_a in "$@"; do
        case $podnet_a in -h | --help) podnet_usage; exit 0 ;; esac
    done
    inst_parse "$@"
    if command -v hash > /dev/null 2>&1; then hash -r > /dev/null 2>&1 || true; fi
    repo=${PODNET_REPO:-PodUtils/podnet}
    case "$repo" in
        */*/* | */ | /* | '' | *[!0-9A-Za-z._/-]*) inst_die "PODNET_REPO must be OWNER/NAME (got: $repo)" ;;
        */*) ;;
        *) inst_die "PODNET_REPO must be OWNER/NAME (got: $repo)" ;;
    esac
    INST_REPO=$repo
    podnet_os=$(uname -s 2> /dev/null) || podnet_os=unknown
    [ "$podnet_os" = Linux ] || inst_die "podnet has Linux binaries only (this system: $podnet_os)"
    inst_detect_target
    asset=$(inst_files "$inst_target" "$inst_exe" | awk 'NR == 1 { print $2 }')

    # The directory first: a bad directory fails before a download.
    podnet_dir
    if [ -e "$dir/podnet" ] && [ ! -f "$dir/podnet" ]; then
        inst_die "$dir/podnet exists and is not a file; remove it or set PODNET_INSTALL_DIR"
    fi

    podnet_version=${inst_opt_version:-${PODNET_VERSION:-latest}}
    tmp=''
    trap podnet_cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    trap 'exit 129' HUP
    if [ -n "$INST_VERSION" ]; then
        # The copy of a release: the pins are in this file.
        if [ "$podnet_version" != latest ] && [ -z "$inst_opt_version" ]; then
            podnet_tag "$podnet_version"
            set -- --version "$tag" "$@"
        fi
        if [ -z "$inst_offline" ]; then
            if detect_downloader; then
                podnet_bridge
            elif [ -n "${PODNET_DOWNLOADER:-}" ]; then
                podnet_no_downloader
            fi
        fi
        podnet_nosha
    else
        [ -z "$inst_offline" ] ||
            inst_die "--offline needs podnet-install.sh of a release (it holds the pins); this copy has no pins"
        detect_downloader || podnet_no_downloader
        inst_say "downloader: $downloader; target: $inst_target" >&2
        warn_insecure
        podnet_temp
        podnet_tag "$podnet_version"
        inst_say "release: $tag" >&2
        podnet_pins
        podnet_bridge
        podnet_nosha
        rm -rf "$tmp"
        tmp=''
    fi
    trap - EXIT INT TERM HUP
    inst_main --prefix "$dir" "$@"
}

podnet_cleanup() { [ -z "${tmp:-}" ] || rm -rf "$tmp"; }

podnet_no_downloader() {
    if [ -n "${PODNET_DOWNLOADER:-}" ]; then
        inst_die "PODNET_DOWNLOADER=$PODNET_DOWNLOADER: not found or not usable (valid: curl wget aria2c http python3 python perl busybox)"
    fi
    inst_die "no downloader: install curl or wget"
}

# The tag of VERSION in tag: latest is the newest release.
podnet_tag() {
    case "$1" in
        latest) resolve_latest ;;
        v*) tag=$1 ;;
        *) tag=v$1 ;;
    esac
    valid_tag "$tag" || inst_die "$tag is not a release tag"
}

# The directory of the binary in dir, as an absolute path. With no
# --dry-run, it exists and this user can make a file in it.
podnet_dir() {
    if [ -n "$inst_opt_prefix" ]; then
        podnet_want "$inst_opt_prefix" "--prefix $inst_opt_prefix: not a writable directory"
    elif [ -n "${PODNET_INSTALL_DIR:-}" ]; then
        podnet_want "$PODNET_INSTALL_DIR" "PODNET_INSTALL_DIR ($PODNET_INSTALL_DIR) is not a writable directory"
    elif [ "$(id -u 2> /dev/null || echo 1)" = 0 ] && writable_dir /usr/local/bin; then
        dir=/usr/local/bin
    else
        for d in "${XDG_BIN_HOME:-}" "${HOME:+$HOME/.local/bin}"; do
            [ -n "$d" ] || continue
            if [ "$inst_dry" = 1 ] || { mkdir -p "$d" 2> /dev/null && writable_dir "$d"; }; then
                podnet_want "$d" "$d: not a writable directory"
                return 0
            fi
        done
        inst_die "no writable directory for the binary; set PODNET_INSTALL_DIR"
    fi
}

# dir: the absolute path of DIR; fail with MESSAGE when it cannot take a file.
podnet_want() {
    if [ "$inst_dry" = 1 ]; then
        case $1 in /*) dir=${1%/} ;; *) dir=$(pwd)/${1%/} ;; esac
        [ -n "$dir" ] || dir=/
        return 0
    fi
    mkdir -p "$1" 2> /dev/null || true
    writable_dir "$1" || inst_die "$2"
    dir=$(cd "$1" && pwd -P) || inst_die "$2"
}

# True when DIR is a directory where this user can make a file. `[ -w ]`
# of busybox sh says yes for root on a read-only file system.
writable_dir() {
    [ -d "$1" ] || return 1
    probe="$1/.podnet-probe.$$"
    (: > "$probe") 2> /dev/null || return 1
    rm -f "$probe"
}

# A private directory for SHA256SUMS and the release list: in the directory
# of the binary (the core removes it after a run that stopped), or in
# TMPDIR for --dry-run.
podnet_temp() {
    if [ "$inst_dry" = 1 ]; then
        tmp=$(mktemp -d 2> /dev/null) || {
            tmp="${TMPDIR:-/tmp}/podnet-install.$$"
            (umask 077 && mkdir "$tmp") || { tmp='' && inst_die "cannot make a temporary directory"; }
        }
    else
        tmp="$dir/.podnet-install.$$.meta"
        (umask 077 && mkdir "$tmp") || { tmp='' && inst_die "cannot make $dir/.podnet-install.$$.meta"; }
    fi
}

# True when the signature of SHA256SUMS is checked (cosign is there, and
# PODNET_COSIGN is not none).
podnet_cosign() {
    inst_detect_cosign
    [ -n "$inst_cosign" ]
}

# The pins of this copy of the tree, from SHA256SUMS of the release.
podnet_pins() {
    base=$(inst_knob MIRROR)
    if [ -n "$base" ]; then
        podnet_cosign ||
            inst_warn "PODNET_MIRROR: SHA256SUMS comes from the mirror, and no cosign checks it"
    fi
    base=${base:-https://github.com/$repo/releases/download/$tag}
    base=${base%/}
    # This file is not under the signature: the core checks the signature
    # of SHA256SUMS before a download, and each pin against the signed
    # lines (inst_signature).
    get "$base/SHA256SUMS" "$tmp/SHA256SUMS" SHA256SUMS
    # Exactly one line names the file. It has two fields, and the hash has
    # 64 hex digits.
    found=$(awk -v f="$asset" 'NF == 2 && ($2 == f || $2 == "*" f) { n++ } END { print n + 0 }' "$tmp/SHA256SUMS")
    want=$(awk -v f="$asset" 'NF == 2 && ($2 == f || $2 == "*" f) && $1 ~ /^[0-9A-Fa-f]+$/ && length($1) == 64 { print $1 }' "$tmp/SHA256SUMS")
    if [ "$found" != 1 ] || [ -z "$want" ]; then
        inst_die "SHA256SUMS must have exactly one valid line for $asset (found $found)"
    fi
    want=$(printf '%s' "$want" | tr 'A-F' 'a-f')
    eval "INST_SUM_$(inst_key "$asset")=\$want"
    INST_VERSION=${tag#v}
    INST_TARGETS=$(awk 'NF == 2 { n = $2; sub(/^\*/, "", n); if (n ~ /^podnet-[a-z0-9_]+-[a-z0-9_-]+$/) { sub(/^podnet-/, "", n); printf "%s ", n } }' "$tmp/SHA256SUMS")
}

# With no SHA-256 tool, PODNET_NO_VERIFY=1 installs the binary with no
# check: the core then takes the pin as the SHA-256 of the new file, and
# never as the SHA-256 of a file in place.
podnet_nosha() {
    (inst_detect_sha) > /dev/null 2>&1 && return 0
    [ "${PODNET_NO_VERIFY:-}" = 1 ] ||
        inst_die "no SHA-256 tool (sha256sum, shasum, openssl, busybox, python3); set PODNET_NO_VERIFY=1 to install with no check"
    inst_warn "no SHA-256 tool: $asset is not checked (PODNET_NO_VERIFY=1)"
    inst_detect_sha() { inst_sha=none; }
    inst_sha256() {
        case $1 in
            "$inst_tmp".*) inst_pin "$asset" ;;
            *) echo unknown ;;
        esac
    }
}

# Give the downloader of this script to the core. The core knows curl,
# wget, python3 and busybox; for another tool, the core uses download.
podnet_bridge() {
    case $downloader in
        curl | wget | python3 | busybox) PODNET_DOWNLOADER=$downloader ;;
        wget-busybox) PODNET_DOWNLOADER=wget ;;
        *)
            # shellcheck disable=SC2317 # the core calls these by name.
            inst_detect_downloader() { inst_dl=$downloader; }
            # shellcheck disable=SC2317 # the core calls these by name.
            inst_fetch() { download "$1" "$2"; }
            ;;
    esac
}

# True when busybox has TLS (its ssl_client helper): a busybox wget with no
# TLS sends an https URL as plain HTTP.
busybox_tls() {
    busybox --list 2> /dev/null | grep -qx ssl_client || command -v ssl_client > /dev/null 2>&1
}

# The first downloader that this machine has, in downloader. busybox wget
# goes last: it does not check TLS certificates. A wget that is the busybox
# applet (Alpine) is busybox too.
detect_downloader() {
    downloader=''
    fallback=''
    for tool in ${PODNET_DOWNLOADER:-curl wget aria2c http python3 python perl busybox}; do
        if ! command -v "$tool" > /dev/null 2>&1; then continue; fi
        case "$tool" in
            wget)
                if wget --help 2>&1 | head -n 2 | grep -q BusyBox; then
                    busybox_tls || continue
                    [ -n "$fallback" ] || fallback=wget-busybox
                    continue
                fi
                ;;
            busybox)
                busybox wget --help > /dev/null 2>&1 || continue
                busybox_tls || continue
                [ -n "$fallback" ] || fallback=busybox
                continue
                ;;
            perl) perl -MHTTP::Tiny -e 'exit(HTTP::Tiny->can_ssl ? 0 : 1)' > /dev/null 2>&1 || continue ;;
            curl | aria2c | http | python3 | python) ;;
            *) continue ;;
        esac
        downloader=$tool
        return 0
    done
    downloader=$fallback
    [ -n "$downloader" ]
}

# The warning of a downloader that does not check TLS certificates.
warn_insecure() {
    case "$downloader" in
        busybox | wget-busybox)
            inst_warn "busybox wget does not check TLS certificates: a host on the path can change the files."
            if podnet_cosign; then
                inst_warn "cosign checks the signature of SHA256SUMS, so a changed file is refused."
            else
                inst_warn "SHA256SUMS comes the same way, so its check does not protect you. Install curl, wget, python3 or cosign, or check the binary with: sha256sum -c SHA256SUMS (from a trusted copy)."
            fi
            ;;
    esac
}

# Download URL to FILE. The status is the status of the downloader; a file
# that is empty gives 99.
fetch() {
    rm -f "$2"
    download "$1" "$2" || return $?
    [ -s "$2" ] || return 99
}

# Download URL to FILE for WHAT, or fail with a message that tells a
# missing file from a failed download.
get() {
    rc=0
    fetch "$1" "$2" || rc=$?
    [ "$rc" = 0 ] && return 0
    [ "$rc" = 99 ] && inst_die "the download of $3 is empty: $1"
    case "$downloader:$rc" in
        curl:22 | wget:8) inst_die "no $3 in $tag" ;;
    esac
    inst_die "the download of $3 failed (exit $rc of $downloader; is github.com reachable?): $1"
}

# A request to the GitHub API, with GITHUB_TOKEN or GH_TOKEN (curl and
# wget). The token goes to api.github.com only. The token is not in the
# trace of DEBUG and not in the arguments of the downloader (ps shows
# them): curl reads the header from stdin (-K -), and wget from a private
# wgetrc file in the temporary directory.
fetch_api() {
    case $- in *x*) trace=1 ;; *) trace= ;; esac
    { set +x; } 2> /dev/null
    api_rc=0
    fetch_api_quiet "$1" "$2" || api_rc=$?
    if [ -n "$trace" ]; then set -x; fi
    return "$api_rc"
}

fetch_api_quiet() {
    token=${GITHUB_TOKEN:-${GH_TOKEN:-}}
    rm -f "$2"
    case "$downloader:${token:+t}" in
        curl:t)
            printf 'header = "Authorization: Bearer %s"\n' "$token" |
                curl -q -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 -K - -o "$2" "$1" || return $?
            ;;
        wget:t)
            rc_file="$tmp/wgetrc"
            (umask 077 && printf 'header = Authorization: Bearer %s\n' "$token" > "$rc_file") || return 1
            s=0
            wget -q --https-only --config="$rc_file" -O "$2" "$1" || s=$?
            rm -f "$rc_file"
            [ "$s" = 0 ] || return "$s"
            ;;
        *) download "$1" "$2" || return $? ;;
    esac
    [ -s "$2" ]
}

# The download itself, by the downloader. An https URL allows no redirect
# to plain HTTP; an http or file URL is a mirror (PODNET_MIRROR).
download() {
    case $1 in
        file://*) cp "${1#file://}" "$2"; return ;;
        https://*) proto='=https' only=--https-only ;;
        *) proto='=http,https' only='' ;;
    esac
    case "$downloader" in
        curl) curl -q -fsSL --proto "$proto" --proto-redir "$proto" --tlsv1.2 --retry 3 -o "$2" "$1" ;;
        wget) wget -q $only -O "$2" "$1" ;;
        wget-busybox) wget -q -O "$2" "$1" ;;
        busybox) busybox wget -q -O "$2" "$1" ;;
        aria2c) aria2c -q --allow-overwrite=true -d "$(dirname "$2")" -o "$(basename "$2")" "$1" ;;
        http) http --check-status -Fdq -o "$2" GET "$1" ;;
        python3 | python)
            "$downloader" -c 'import sys
try:
    from urllib.request import urlopen
except ImportError:
    from urllib2 import urlopen
data = urlopen(sys.argv[1], timeout=60).read()
open(sys.argv[2], "wb").write(data)' "$1" "$2" ;;
        perl)
            perl -MHTTP::Tiny -e 'my $r = HTTP::Tiny->new(verify_SSL => 1)->mirror($ARGV[0], $ARGV[1]);
exit($r->{success} ? 0 : 1)' "$1" "$2" ;;
        *) return 1 ;;
    esac
}

# The newest tag of the releases in tag: the highest version of the API,
# prereleases too (podnet has prereleases only, and /releases/latest skips
# them). When the API fails (403 on a shared address), releases.atom gives
# the newest tag.
resolve_latest() {
    tag=''
    # The API lists by tag name (beta.10 after beta.7): the newest is the
    # highest version of the list. A draft is not a release.
    if fetch_api "https://api.github.com/repos/$repo/releases?per_page=30" "$tmp/releases.json" 2> "$tmp/api.err"; then
        tag=$(tr ',' '\n' < "$tmp/releases.json" | public_tags | highest)
    fi
    if [ -z "$tag" ]; then
        inst_say "the GitHub API did not answer; reading releases.atom" >&2
        if fetch "https://github.com/$repo/releases.atom" "$tmp/releases.atom" 2> "$tmp/atom.err"; then
            tag=$(tr '>' '\n' < "$tmp/releases.atom" | sed -n 's#.*/releases/tag/\([^"<]*\)".*#\1#p' | highest)
        fi
    fi
    if [ -z "$tag" ]; then
        tail -n 3 "$tmp/api.err" "$tmp/atom.err" >&2 2> /dev/null || true
        inst_die "cannot read the releases of $repo (the API and releases.atom); set PODNET_VERSION=vX.Y.Z"
    fi
}

# True when $1 can be a release tag. The rule of podnet update: 1 to 64
# characters of 0-9 A-Z a-z . + -, the first one v or a digit, and no "..".
valid_tag() {
    case "$1" in
        '' | *[!0-9A-Za-z.+-]*) return 1 ;;
        *..*) return 1 ;;
        v* | [0-9]*) ;;
        *) return 1 ;;
    esac
    [ "${#1}" -le 64 ]
}

# The tags of the release list on stdin (one JSON member on each line),
# with no draft. The API puts "draft" after "tag_name" in each release.
public_tags() {
    awk '
        /"tag_name"[[:space:]]*:/ {
            if (t != "") print t
            t = $0
            sub(/.*"tag_name"[[:space:]]*:[[:space:]]*"/, "", t)
            sub(/".*/, "", t)
            next
        }
        /"draft"[[:space:]]*:[[:space:]]*true/ { t = "" }
        END { if (t != "") print t }
    '
}

# The highest version of the tags on stdin (vX.Y.Z or vX.Y.Z-WORD.N): a
# release is after its prereleases, and a number compares as a number.
highest() {
    awk '{
        tag = $0; core = tag; pre = ""
        sub(/^v/, "", core)
        if (index(core, "-")) { pre = substr(core, index(core, "-") + 1); core = substr(core, 1, index(core, "-") - 1) }
        n = split(core, c, ".")
        word = pre; num = 0
        if (pre != "") { split(pre, p, "."); word = p[1]; num = p[2] + 0 }
        printf "%09d.%09d.%09d %d %s %09d %s\n", c[1], c[2], c[3], (pre == "") ? 1 : 0, (word == "") ? "~" : word, num, tag
    }' | sort | tail -n 1 | awk '{ print $NF }'
}

podnet_main "$@"
