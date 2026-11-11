#!/usr/bin/env bash
# Build the Flatpak and a single-file bundle: dist/Crab_Player-<version>-x86_64.flatpak
#
#   packaging/flatpak/build.sh            build into packaging/flatpak/repo and
#                                         write the bundle
#   packaging/flatpak/build.sh --install  also install it for the current user
#
# Needs flatpak and flatpak-builder; the runtime, SDK and Rust extension
# (org.freedesktop.Platform/Sdk 26.08) are installed for the user from
# Flathub when missing.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app_id="io.github.mahziyarng98.CrabPlayer"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
install=0
[ "${1:-}" = "--install" ] && install=1

for tool in flatpak flatpak-builder python3; do
    command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 2; }
done
if ! flatpak remotes --user | grep -q '^flathub'; then
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
fi

python3 "$here/cargo-sources.py" "$root/Cargo.lock" "$here/cargo-sources.json"

args=(--user --force-clean --install-deps-from=flathub --repo="$here/repo"
      --state-dir="$here/.flatpak-builder")
# Crates already in the host's cargo cache are used instead of downloaded
# again (flatpak-builder still checks each against its sha256).
host_crates=("${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/)
if [ -d "${host_crates[0]}" ]; then
    for dir in "${host_crates[@]}"; do args+=(--extra-sources="$dir"); done
fi
[ "$install" = 1 ] && args+=(--install)
flatpak-builder "${args[@]}" "$here/build-dir" "$here/$app_id.yml"

mkdir -p "$root/dist"
bundle="$root/dist/Crab_Player-$version-x86_64.flatpak"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$here/repo" "$bundle" "$app_id"
(cd "$root/dist" && sha256sum "$(basename "$bundle")" >"$(basename "$bundle").sha256")
ls -l "$bundle"
