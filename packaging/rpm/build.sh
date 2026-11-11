#!/usr/bin/env bash
# Build the .rpm: dist/crab-player-<version>-1.<dist>.x86_64.rpm
#
#   packaging/rpm/build.sh            build in a Fedora container (docker/podman)
#   packaging/rpm/build.sh --inside   build on the current Fedora/openSUSE system
#                                     (as root, e.g. a CI job in fedora:latest)
#
# RPM_IMAGE picks the container (default fedora:latest). The binary needs
# the build system's glibc or newer: a package built on current Fedora
# installs on that Fedora release and newer and on openSUSE Tumbleweed.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
image="${RPM_IMAGE:-fedora:latest}"
rust_version="1.95.0"

if [ "${1:-}" != "--inside" ]; then
    engine="$(command -v docker || command -v podman || true)"
    [ -n "$engine" ] || { echo "docker or podman is required (or run with --inside)" >&2; exit 2; }
    mkdir -p "$root/dist"
    # The host's cargo registry cache, when there is one, seeds the
    # container's, so crates the host already has are not downloaded again
    # (CRAB_NO_HOST_CRATES=1 turns this off).
    vendor=()
    host_registry="${CARGO_HOME:-$HOME/.cargo}/registry"
    if [ -z "${CRAB_NO_HOST_CRATES:-}" ] && [ -d "$host_registry/cache" ]; then
        vendor=(-v "$host_registry:/host-registry:ro")
    fi
    exec "$engine" run --rm "${vendor[@]}" \
        -v "$root:/src:ro" -v "$root/dist:/dist" \
        -v crab-player-cargo:/root/.cargo -v crab-player-rustup:/root/.rustup \
        -v crab-player-fedora-dnf:/var/cache/libdnf5 \
        -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
        "$image" bash /src/packaging/rpm/build.sh --inside
fi

src="${CRAB_SRC:-/src}"
[ -d "$src" ] || src="$root"
dist="${CRAB_DIST:-/dist}"
[ -d "$dist" ] || dist="$root/dist"
mkdir -p "$dist"

if command -v dnf >/dev/null; then
    dnf install -y -q --setopt=keepcache=True rpm-build gcc pkgconfig mpv-devel desktop-file-utils tar gzip curl >/dev/null
else
    zypper --non-interactive -q install rpm-build gcc pkgconf mpv-devel desktop-file-utils tar gzip curl >/dev/null
fi
# A toolchain cached in a volume is used as is.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if ! command -v cargo >/dev/null || ! cargo --version | grep -q "$rust_version"; then
    curl -sSf --retry 5 --retry-all-errors https://sh.rustup.rs |
        RUSTUP_MAX_RETRIES=20 sh -s -- -y -q --profile minimal --default-toolchain "$rust_version"
fi
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
# Seed the crate cache from the host's and try offline first; whatever the
# host does not have is fetched afterwards.
if [ -d /host-registry/cache ]; then
    mkdir -p "$HOME/.cargo/registry"
    cp -a /host-registry/index /host-registry/cache "$HOME/.cargo/registry/"
    (cd "$src" && CARGO_TARGET_DIR=/tmp/fetch cargo fetch --locked --offline \
        --target x86_64-unknown-linux-gnu >/dev/null 2>&1) ||
        (cd "$src" && CARGO_TARGET_DIR=/tmp/fetch cargo fetch --locked --target x86_64-unknown-linux-gnu)
fi

version="$(sed -n 's/^Version: *//p' "$src/packaging/rpm/crab-player.spec")"
top="$(mktemp -d)"
mkdir -p "$top/SOURCES"
# The same layout as the GitHub release tarball: crab-player-<version>/...
tar -C "$src" --exclude=./target --exclude=./dist --exclude=./.git \
    --transform "s,^\.,crab-player-$version," -czf "$top/SOURCES/crab-player-$version.tar.gz" .
rpmbuild -bb --define "_topdir $top" "$src/packaging/rpm/crab-player.spec"
rpm="$(find "$top/RPMS" -name 'crab-player-*.rpm' | head -1)"
cp "$rpm" "$dist/"
out="$dist/$(basename "$rpm")"
(cd "$dist" && sha256sum "$(basename "$out")" >"$(basename "$out").sha256")
if [ -n "${HOST_UID:-}" ]; then chown "$HOST_UID:${HOST_GID:-$HOST_UID}" "$out" "$out.sha256"; fi
rpm -qp --requires "$out" | grep -v '^rpmlib'
ls -l "$out"
