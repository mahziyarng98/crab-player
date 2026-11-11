#!/usr/bin/env bash
# Install Crab Player for the current user, without root.
#
#   scripts/install.sh            build the release binary, then install
#   scripts/install.sh --no-build install an already built target/release/crab-player
#
# Installs into $PREFIX (default ~/.local):
#   bin/crab-player
#   share/applications/io.github.mahziyarng98.CrabPlayer.desktop   (Exec = absolute path above)
#   share/icons/hicolor/<size>x<size>/apps/io.github.mahziyarng98.CrabPlayer.png
#   share/metainfo/io.github.mahziyarng98.CrabPlayer.metainfo.xml
#   share/crab-player/install-manifest           (what uninstall.sh removes)
#
# The desktop entry lists the media types Crab Player opens, so it appears
# under "Open with". Default applications are NOT changed.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
prefix="${PREFIX:-$HOME/.local}"
build=1
for arg in "$@"; do
    case "$arg" in
        --no-build) build=0 ;;
        -h|--help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

if [ "$build" = 1 ]; then
    echo "Building the release binary…"
    (cd "$root" && cargo build --release --locked)
fi
binary="$root/target/release/crab-player"
[ -x "$binary" ] || { echo "no binary at $binary; build it first" >&2; exit 1; }

bin_dir="$prefix/bin"
app_dir="$prefix/share/applications"
icon_root="$prefix/share/icons/hicolor"
data_dir="$prefix/share/crab-player"
manifest="$data_dir/install-manifest"

# A previous install is removed first, so files it had that this version
# no longer installs do not linger.
if [ -f "$manifest" ]; then
    "$root/scripts/uninstall.sh" --quiet
fi

mkdir -p "$bin_dir" "$app_dir" "$data_dir"
installed=()

install -m 755 "$binary" "$bin_dir/crab-player"
installed+=("$bin_dir/crab-player")

app_id="io.github.mahziyarng98.CrabPlayer"
desktop="$app_dir/$app_id.desktop"
# Exec gets the absolute path: ~/.local/bin is not on PATH in every
# session that launches desktop entries.
sed "s|^Exec=crab-player |Exec=$bin_dir/crab-player |" \
    "$root/packaging/$app_id.desktop" >"$desktop"
chmod 644 "$desktop"
installed+=("$desktop")

for size in 16 24 32 48 64 128 256 512; do
    dir="$icon_root/${size}x${size}/apps"
    mkdir -p "$dir"
    install -m 644 "$root/assets/icons/hicolor/${size}x${size}/apps/$app_id.png" "$dir/$app_id.png"
    installed+=("$dir/$app_id.png")
done

metainfo_dir="$prefix/share/metainfo"
mkdir -p "$metainfo_dir"
install -m 644 "$root/packaging/$app_id.metainfo.xml" "$metainfo_dir/$app_id.metainfo.xml"
installed+=("$metainfo_dir/$app_id.metainfo.xml")

printf '%s\n' "${installed[@]}" >"$manifest"

# Refresh the caches when the tools are there; nothing depends on them.
if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$desktop"
fi
if command -v update-desktop-database >/dev/null; then
    update-desktop-database -q "$app_dir" || true
fi
if command -v gtk-update-icon-cache >/dev/null && [ -f "$icon_root/index.theme" ]; then
    gtk-update-icon-cache -q -t "$icon_root" || true
fi
# Touching the theme directory makes icon-theme watchers reload.
touch "$icon_root" 2>/dev/null || true

echo "Installed Crab Player $("$bin_dir/crab-player" --version | awk '{print $2}'):"
printf '  %s\n' "${installed[@]}"
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "Note: $bin_dir is not on PATH; run it as $bin_dir/crab-player or add it to PATH." ;;
esac
