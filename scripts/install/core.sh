#!/bin/sh
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
