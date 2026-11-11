#!/usr/bin/env bash
# Install Crab Player from this checkout.
#
#   scripts/install.sh                         build, then install for the current user (~/.local)
#   scripts/install.sh --no-build              install an already built target/release/crab-player
#   sudo scripts/install.sh --prefix /usr/local --no-build
#                                              system-wide install (build first, as yourself:
#                                              cargo build --release --locked)
#   DESTDIR=/tmp/stage scripts/install.sh --prefix /usr/local --no-build
#                                              stage a system-wide install without root
#
# Installs into PREFIX (default ~/.local; --prefix or $PREFIX):
#   bin/crab-player
#   share/applications/io.github.mahziyarng98.CrabPlayer.desktop   (Exec = absolute path above)
#   share/icons/hicolor/<size>x<size>/apps/io.github.mahziyarng98.CrabPlayer.png
#   share/metainfo/io.github.mahziyarng98.CrabPlayer.metainfo.xml
#   share/crab-player/install-manifest             (what uninstall.sh removes)
#
# The desktop entry lists the media types Crab Player opens, so it appears
# under "Open with". Default applications are NOT changed.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
prefix="${PREFIX:-$HOME/.local}"
destdir="${DESTDIR:-}"
build=1
while [ $# -gt 0 ]; do
    case "$1" in
        --no-build) build=0 ;;
        --prefix) prefix="${2:?--prefix needs a directory}"; shift ;;
        --prefix=*) prefix="${1#--prefix=}" ;;
        -h|--help) sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
case "$prefix" in
    /*) prefix="${prefix%/}" ;;
    *) echo "--prefix must be an absolute path" >&2; exit 2 ;;
esac

# Writing outside the home directory needs root, unless it is only staged.
if [ -z "$destdir" ] && [ "$(id -u)" -ne 0 ]; then
    case "$prefix/" in
        "$HOME"/*) ;;
        *) echo "Installing into $prefix needs root: sudo $0 --prefix $prefix --no-build" >&2
           echo "(or stage it with DESTDIR=/some/dir $0 --prefix $prefix)" >&2
           exit 1 ;;
    esac
fi

# The release tarball ships a prebuilt binary next to this script's parent
# instead of a source tree; then there is nothing to build.
binary="$root/target/release/crab-player"
if [ ! -f "$root/Cargo.toml" ] && [ -x "$root/crab-player" ]; then
    binary="$root/crab-player"
    build=0
fi

if [ "$build" = 1 ]; then
    if [ "$(id -u)" -eq 0 ]; then
        echo "Not building as root (it would leave root-owned files in target/)." >&2
        echo "Build as yourself first: cargo build --release --locked; then pass --no-build." >&2
        exit 1
    fi
    echo "Building the release binary…"
    (cd "$root" && cargo build --release --locked)
fi
[ -x "$binary" ] || { echo "no binary at $binary; build it first" >&2; exit 1; }

app_id="io.github.mahziyarng98.CrabPlayer"
# Final locations (what the desktop entry and the manifest refer to) and
# where the files are written now (the same unless DESTDIR stages them).
bin_dir="$prefix/bin"
app_dir="$prefix/share/applications"
icon_root="$prefix/share/icons/hicolor"
metainfo_dir="$prefix/share/metainfo"
data_dir="$prefix/share/crab-player"
manifest="$data_dir/install-manifest"
stage() { printf '%s%s' "$destdir" "$1"; }

# A previous install is removed first, so files it had that this version
# no longer installs do not linger.
if [ -f "$(stage "$manifest")" ]; then
    PREFIX="$prefix" DESTDIR="$destdir" "$root/scripts/uninstall.sh" --quiet
fi

mkdir -p "$(stage "$bin_dir")" "$(stage "$app_dir")" "$(stage "$metainfo_dir")" "$(stage "$data_dir")"
installed=()

install -m 755 "$binary" "$(stage "$bin_dir/crab-player")"
installed+=("$bin_dir/crab-player")

desktop="$app_dir/$app_id.desktop"
# Exec gets the absolute path: ~/.local/bin is not on PATH in every
# session that launches desktop entries.
sed "s|^Exec=crab-player |Exec=$bin_dir/crab-player |" \
    "$root/packaging/$app_id.desktop" >"$(stage "$desktop")"
chmod 644 "$(stage "$desktop")"
installed+=("$desktop")

for size in 16 24 32 48 64 128 256 512; do
    dir="$icon_root/${size}x${size}/apps"
    mkdir -p "$(stage "$dir")"
    install -m 644 "$root/assets/icons/hicolor/${size}x${size}/apps/$app_id.png" "$(stage "$dir/$app_id.png")"
    installed+=("$dir/$app_id.png")
done

install -m 644 "$root/packaging/$app_id.metainfo.xml" "$(stage "$metainfo_dir/$app_id.metainfo.xml")"
installed+=("$metainfo_dir/$app_id.metainfo.xml")

printf '%s\n' "${installed[@]}" >"$(stage "$manifest")"
chmod 644 "$(stage "$manifest")"

if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$(stage "$desktop")"
fi
# Caches belong to the live system; a staged install leaves them alone.
if [ -z "$destdir" ]; then
    if command -v update-desktop-database >/dev/null; then
        update-desktop-database -q "$app_dir" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null && [ -f "$icon_root/index.theme" ]; then
        gtk-update-icon-cache -q -t "$icon_root" || true
    fi
    # Touching the theme directory makes icon-theme watchers reload.
    touch "$icon_root" 2>/dev/null || true
fi

echo "Installed Crab Player $("$(stage "$bin_dir/crab-player")" --version | awk '{print $2}')${destdir:+ (staged in $destdir)}:"
printf '  %s\n' "${installed[@]}"
if [ -z "$destdir" ]; then
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *) echo "Note: $bin_dir is not on PATH; run it as $bin_dir/crab-player or add it to PATH." ;;
    esac
fi
