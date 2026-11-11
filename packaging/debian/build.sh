#!/usr/bin/env bash
# Build the .deb: dist/crab-player_<version>-1_amd64.deb
#
#   packaging/debian/build.sh            build in a Debian 12 container (docker/podman)
#   packaging/debian/build.sh --inside   build on the current Debian/Ubuntu system
#                                        (as root, e.g. a CI job in debian:bookworm)
#
# Built on Debian 12 (glibc 2.36, libmpv2 0.35), so the package installs on
# Debian 12 and 13 and Ubuntu 23.04 and newer. Rust comes from rustup.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
image="${DEB_IMAGE:-debian:bookworm}"
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
        -v crab-player-debian12-apt:/var/cache/apt/archives -v crab-player-debian12-lists:/var/lib/apt/lists \
        -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
        "$image" bash /src/packaging/debian/build.sh --inside
fi

src="${CRAB_SRC:-/src}"
[ -d "$src" ] || src="$root"
dist="${CRAB_DIST:-/dist}"
[ -d "$dist" ] || dist="$root/dist"
mkdir -p "$dist"
export DEBIAN_FRONTEND=noninteractive

# Keep downloaded packages (the image deletes them by default), so a
# cached /var/cache/apt makes the next container build quicker.
rm -f /etc/apt/apt.conf.d/docker-clean
echo 'Acquire::Retries "10";' >/etc/apt/apt.conf.d/80-retries
for attempt in 1 2 3 4 5; do
    apt-get update -qq 2>&1 | grep -q '^[WE]: Failed to fetch' || break
    [ "$attempt" = 5 ] && { echo "apt-get update kept failing" >&2; exit 1; }
    sleep 15
done
apt-get install -y -qq --no-install-recommends \
    build-essential ca-certificates curl debhelper fakeroot pkg-config \
    libmpv-dev >/dev/null
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

build="$(mktemp -d)/crab-player"
mkdir -p "$build"
tar -C "$src" --exclude=./target --exclude=./dist --exclude=./.git -cf - . | tar -C "$build" -xf -
cp -r "$build/packaging/debian" "$build/debian"
cd "$build"
dpkg-buildpackage -b -us -uc
deb="$(ls ../crab-player_*.deb | grep -v dbgsym | head -1)"
cp "$deb" "$dist/"
out="$dist/$(basename "$deb")"
(cd "$dist" && sha256sum "$(basename "$out")" >"$(basename "$out").sha256")
if [ -n "${HOST_UID:-}" ]; then chown "$HOST_UID:${HOST_GID:-$HOST_UID}" "$out" "$out.sha256"; fi
dpkg-deb --info "$out" | sed -n '/Depends/,/Description/p'
ls -l "$out"
