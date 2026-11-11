#!/usr/bin/env bash
# Build the Arch package from this checkout instead of a release tarball.
# Usage: packaging/arch/build-local.sh [output-dir]
# Nothing is installed; the package ends up in the output directory.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$root/target/arch}"
version="$(sed -n 's/^pkgver=//p' "$root/packaging/arch/PKGBUILD")"
mkdir -p "$out"

# The files a release tarball contains.
tar -C "$root" --transform "s,^,crab-player-$version/," \
  -czf "$out/crab-player-$version.tar.gz" \
  Cargo.toml Cargo.lock LICENSE README.md src assets packaging docs scripts

sed "s|^source=.*|source=(\"crab-player-$version.tar.gz\")|" \
  "$root/packaging/arch/PKGBUILD" > "$out/PKGBUILD"

cd "$out"
# Reuse the user's cargo registry so the build also works offline.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
makepkg --force --nodeps
ls -1 "$out"/*.pkg.tar.*
