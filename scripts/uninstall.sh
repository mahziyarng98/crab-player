#!/usr/bin/env bash
# Remove a user-level Crab Player install made by scripts/install.sh.
#
# Removes exactly the files listed in $PREFIX/share/crab-player/install-manifest
# (default PREFIX ~/.local) and the manifest itself, then any directory
# that this left empty inside share/crab-player. Settings in
# ~/.config/crab-player are kept.

set -euo pipefail

prefix="${PREFIX:-$HOME/.local}"
quiet=0
[ "${1:-}" = "--quiet" ] && quiet=1
say() { [ "$quiet" = 1 ] || echo "$@"; }

data_dir="$prefix/share/crab-player"
manifest="$data_dir/install-manifest"
if [ ! -f "$manifest" ]; then
    say "Crab Player is not installed under $prefix (no $manifest)."
    exit 0
fi

while IFS= read -r file; do
    [ -n "$file" ] || continue
    # Only paths inside the prefix are ever removed.
    case "$file" in
        "$prefix"/*) ;;
        *) say "skipping $file: outside $prefix"; continue ;;
    esac
    if [ -e "$file" ] || [ -L "$file" ]; then
        rm -f -- "$file"
        say "removed $file"
    fi
done <"$manifest"
rm -f -- "$manifest"
rmdir -- "$data_dir" 2>/dev/null || true

if command -v update-desktop-database >/dev/null; then
    update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
fi
icon_root="$prefix/share/icons/hicolor"
if command -v gtk-update-icon-cache >/dev/null && [ -f "$icon_root/index.theme" ]; then
    gtk-update-icon-cache -q -t "$icon_root" || true
fi
say "Crab Player was removed. Settings in ${XDG_CONFIG_HOME:-$HOME/.config}/crab-player were kept."
