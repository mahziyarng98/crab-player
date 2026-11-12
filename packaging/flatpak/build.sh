#!/usr/bin/env bash
# Build the Flatpak and a single-file bundle: dist/Crab_Player-<version>-x86_64.flatpak
#
#   packaging/flatpak/build.sh            build the working tree into
#                                         packaging/flatpak/repo and write the bundle
#   packaging/flatpak/build.sh --install  also install it for the current user
#
# The manifest in this directory is the one submitted to Flathub: its
# crab-player module is a git source pinned to the release tag. This script
# builds the checkout instead, so it writes a temporary copy of the manifest
# whose crab-player source is the working tree.
#
# Cargo is offline: packaging/flatpak/cargo-sources.json (generated from
# Cargo.lock by flatpak-cargo-generator.py) provides every crate. Regenerate
# it after changing Cargo.lock:
#
#   python3 flatpak-cargo-generator.py -o packaging/flatpak/cargo-sources.json Cargo.lock
#
# Needs flatpak and flatpak-builder; the runtime, SDK and Rust extension
# (org.freedesktop.Platform/Sdk 26.08) are installed for the user from
# Flathub when missing.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
manifest="$here/io.github.mahziyarng98.crab-player.yml"
app_id="$(sed -n 's/^app-id: //p' "$manifest")"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
install=0
[ "${1:-}" = "--install" ] && install=1

for tool in flatpak flatpak-builder awk; do
    command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 2; }
done
[ -f "$here/cargo-sources.json" ] || {
    echo "missing packaging/flatpak/cargo-sources.json" >&2
    exit 2
}
if [ "$here/cargo-sources.json" -ot "$root/Cargo.lock" ]; then
    echo "warning: cargo-sources.json is older than Cargo.lock" >&2
    echo "         regenerate it with flatpak-cargo-generator.py" >&2
fi
if ! flatpak remotes --user | grep -q '^flathub'; then
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
fi

# Build the checkout, not the release tag: swap the git source of the
# crab-player module for a directory source and leave out build products.
local_manifest="$(mktemp --suffix=.yml)"
trap 'rm -f "$local_manifest"' EXIT
CRAB_PLAYER_DIR="$root" CRAB_PLAYER_SOURCES="$here/cargo-sources.json" awk '
    { line[NR] = $0 }
    END {
        for (i = 1; i <= NR; i++) {
            if (line[i] == "      - cargo-sources.json") {
                # The temporary manifest lives elsewhere; make the path absolute.
                print "      - " ENVIRON["CRAB_PLAYER_SOURCES"]
                continue
            }
            if (line[i] == "      - type: git" &&
                line[i + 1] == "        url: https://github.com/mahziyarng98/crab-player.git") {
                print "      - type: dir"
                print "        path: " ENVIRON["CRAB_PLAYER_DIR"]
                print "        skip:"
                print "          - .git"
                print "          - target"
                print "          - dist"
                print "          - packaging/flatpak/build-dir"
                print "          - packaging/flatpak/repo"
                print "          - packaging/flatpak/.flatpak-builder"
                i += 3
                continue
            }
            print line[i]
        }
    }
' "$manifest" >"$local_manifest"

args=(--user --force-clean --install-deps-from=flathub --repo="$here/repo"
      --state-dir="$here/.flatpak-builder")
# Crates already in the host's cargo cache are used instead of downloaded
# again (flatpak-builder still checks each against its sha256).
host_crates=("${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/)
if [ -d "${host_crates[0]}" ]; then
    for dir in "${host_crates[@]}"; do args+=(--extra-sources="$dir"); done
fi
[ "$install" = 1 ] && args+=(--install)
flatpak-builder "${args[@]}" "$here/build-dir" "$local_manifest"

mkdir -p "$root/dist"
bundle="$root/dist/Crab_Player-$version-x86_64.flatpak"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$here/repo" "$bundle" "$app_id"
(cd "$root/dist" && sha256sum "$(basename "$bundle")" >"$(basename "$bundle").sha256")
ls -l "$bundle"
