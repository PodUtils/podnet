# shellcheck shell=sh
# docs/install.md: the check of the downloaded files. The set-up downloads
# SHA256SUMS and the binary of this machine of the newest release (the
# feed of the releases gives its tag, else the API of GitHub), as the
# document says before the block.

# shellcheck disable=SC2034
LIMIT=60

setup() {
    tag=$(curl -fsSL https://github.com/PodUtils/podnet/releases.atom |
        grep -o 'releases/tag/v[^"<]*' | head -n 1 | cut -d/ -f3)
    if [ -z "$tag" ]; then
        tag=$(curl -fsSL 'https://api.github.com/repos/PodUtils/podnet/releases?per_page=1' |
            sed -n 's/^ *"tag_name": "\(v[^"]*\)".*/\1/p' | head -n 1)
    fi
    if [ -z "$tag" ]; then
        echo "doc-scenario: the feed of the releases gives no tag" >&2
        return 1
    fi
    for f in SHA256SUMS "podnet-$(uname -m)-unknown-linux-musl"; do
        curl -fsSLO "https://github.com/PodUtils/podnet/releases/download/$tag/$f"
    done
}
