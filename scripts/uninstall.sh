#!/usr/bin/env bash
# Remove an install made by scripts/install.sh.
#
#   scripts/uninstall.sh                          the user install in ~/.local
#   sudo scripts/uninstall.sh --prefix /usr/local a system-wide install
#   DESTDIR=/tmp/stage scripts/uninstall.sh --prefix /usr/local
#                                                 a staged install
#
# Removes exactly the files listed in PREFIX/share/crab-player/install-manifest
# and the manifest itself, then that directory if it is empty. Settings in
# ~/.config/crab-player are kept.

set -euo pipefail

prefix="${PREFIX:-$HOME/.local}"
destdir="${DESTDIR:-}"
quiet=0
while [ $# -gt 0 ]; do
    case "$1" in
        --quiet) quiet=1 ;;
        --prefix) prefix="${2:?--prefix needs a directory}"; shift ;;
        --prefix=*) prefix="${1#--prefix=}" ;;
        -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
prefix="${prefix%/}"
say() { [ "$quiet" = 1 ] || echo "$@"; }

data_dir="$destdir$prefix/share/crab-player"
manifest="$data_dir/install-manifest"
if [ ! -f "$manifest" ]; then
    say "Crab Player is not installed under $destdir$prefix (no $manifest)."
    exit 0
fi

while IFS= read -r file; do
    [ -n "$file" ] || continue
    # Only paths inside the prefix are ever removed.
    case "$file" in
        "$prefix"/*) ;;
        *) say "skipping $file: outside $prefix"; continue ;;
    esac
    case "$file" in
        */../*|*/..) say "skipping $file: not a plain path"; continue ;;
    esac
    target="$destdir$file"
    if [ -e "$target" ] || [ -L "$target" ]; then
        rm -f -- "$target"
        say "removed $target"
    fi
done <"$manifest"
rm -f -- "$manifest"
rmdir -- "$data_dir" 2>/dev/null || true

if [ -z "$destdir" ]; then
    if command -v update-desktop-database >/dev/null; then
        update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
    fi
    icon_root="$prefix/share/icons/hicolor"
    if command -v gtk-update-icon-cache >/dev/null && [ -f "$icon_root/index.theme" ]; then
        gtk-update-icon-cache -q -t "$icon_root" || true
    fi
fi
say "Crab Player was removed from $destdir$prefix. Settings in ${XDG_CONFIG_HOME:-$HOME/.config}/crab-player were kept."
