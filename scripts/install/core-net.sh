#!/bin/sh
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
