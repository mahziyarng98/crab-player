#!/usr/bin/env bash
# Pack the generic binary tarball:
#   <out-dir>/crab-player-<version>-linux-x86_64.tar.gz
#
#   packaging/tarball.sh <path/to/crab-player> <out-dir>
#
# The tarball holds the prebuilt binary, the desktop entry, AppStream data,
# icons and scripts/install.sh + uninstall.sh, which install it for the
# user (~/.local) or system-wide (--prefix /usr/local). The binary needs
# libmpv.so.2 from the system; packaging/appimage/build.sh builds it on a
# glibc 2.35 baseline and calls this script.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
binary="$(readlink -f "${1:?usage: $0 <binary> <out-dir>}")"
out="$(mkdir -p "${2:?usage: $0 <binary> <out-dir>}" && cd "$2" && pwd)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
app_id="io.github.mahziyarng98.CrabPlayer"
name="crab-player-$version-linux-x86_64"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
dir="$stage/$name"
mkdir -p "$dir/scripts" "$dir/packaging" "$dir/assets"
install -m 755 "$binary" "$dir/crab-player"
install -m 755 "$root/scripts/install.sh" "$root/scripts/uninstall.sh" "$dir/scripts/"
install -m 644 "$root/packaging/$app_id.desktop" "$root/packaging/$app_id.metainfo.xml" "$dir/packaging/"
cp -r "$root/assets/icons" "$dir/assets/"
install -m 644 "$root/LICENSE" "$root/README.md" "$dir/"
cat >"$dir/INSTALL.txt" <<EOF
Crab Player $version, prebuilt for x86_64 Linux (glibc 2.35 or newer).

Needs libmpv (libmpv.so.2) from your distribution:
  Debian / Ubuntu: sudo apt install libmpv2
  Fedora:          sudo dnf install mpv-libs
  openSUSE:        sudo zypper install libmpv2
  Arch:            sudo pacman -S mpv

Install for your user (no root):   scripts/install.sh
Install system-wide:               sudo scripts/install.sh --prefix /usr/local
Remove:                            scripts/uninstall.sh   (add --prefix /usr/local and sudo for a system install)
EOF

tarball="$out/$name.tar.gz"
tar -C "$stage" --owner=0 --group=0 -czf "$tarball" "$name"
(cd "$out" && sha256sum "$(basename "$tarball")" >"$(basename "$tarball").sha256")
ls -l "$tarball"
