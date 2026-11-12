#!/usr/bin/env bash
# Build the universal AppImage: dist/Crab_Player-<version>-x86_64.AppImage
#
#   packaging/appimage/build.sh            build inside an Ubuntu 22.04 container
#                                          (docker or podman; the default)
#   packaging/appimage/build.sh --inside   build on the current system; meant
#                                          for a CI job that already runs in
#                                          ubuntu:22.04 as root
#
# Baseline: Ubuntu 22.04, glibc 2.35. The AppImage runs on distributions
# with glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 36+,
# openSUSE Tumbleweed and Leap 16, RHEL 10, Arch, …).
#
# What is bundled: libmpv (built here, client API 2) and its media stack:
# FFmpeg 4.4 from Ubuntu 22.04 with its codec libraries, libass, libpulse.
# Never bundled, so the host's drivers keep working: glibc, libstdc++ and
# the GPU stack (libGL/libEGL/libGLX/libgbm/libdrm/libvulkan).
# Shipped only as per-library fallbacks, which AppRun uses just for the
# libraries the host lacks: libva, libvdpau, ALSA, D-Bus, udev,
# fontconfig, freetype, harfbuzz, expat, zlib, libuuid and the X11/xcb/
# Wayland/xkbcommon client libraries. On a normal desktop all of these come
# from the host, so VA-API runs on the host's libva and drivers, ALSA on
# its plugins and fonts on its configuration.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
image="ubuntu:22.04"
mpv_version="0.36.0"
mpv_sha256="29abc44f8ebee013bb2f9fe14d80b30db19b534c679056e4851ceadf5a5e8bf6"
rust_version="1.95.0"
# appimagetool and the AppImage runtime it embeds, both pinned to releases
# and checked by sha256 (never the moving "continuous" builds).
appimagetool_version="1.9.1"
appimagetool_sha256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
runtime_version="20251108"
runtime_sha256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

if [ "${1:-}" != "--inside" ]; then
    engine="$(command -v docker || command -v podman || true)"
    [ -n "$engine" ] || { echo "docker or podman is required (or run with --inside on Ubuntu 22.04)" >&2; exit 2; }
    mkdir -p "$root/dist"
    # Cargo's registry and the mpv build are cached in named volumes, so a
    # second build is quick. The checkout is mounted read-only and copied.
    # The host's cargo registry cache, when there is one, seeds the
    # container's, so crates the host already has are not downloaded again
    # (CRAB_NO_HOST_CRATES=1 turns this off).
    vendor=()
    host_registry="${CARGO_HOME:-$HOME/.cargo}/registry"
    if [ -z "${CRAB_NO_HOST_CRATES:-}" ] && [ -d "$host_registry/cache" ]; then
        vendor=(-v "$host_registry:/host-registry:ro")
    fi
    exec "$engine" run --rm "${vendor[@]}" \
        -v "$root:/src:ro" \
        -v "$root/dist:/dist" \
        -v crab-player-cargo:/root/.cargo \
        -v crab-player-rustup:/root/.rustup \
        -v crab-player-appimage-cache:/cache \
        -v crab-player-ubuntu2204-apt:/var/cache/apt/archives \
        -v crab-player-ubuntu2204-lists:/var/lib/apt/lists \
        -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
        "$image" bash /src/packaging/appimage/build.sh --inside
fi

# ---------------------------------------------------------------- inside
src="${CRAB_SRC:-/src}"
[ -d "$src" ] || src="$root"
dist="${CRAB_DIST:-/dist}"
[ -d "$dist" ] || dist="$root/dist"
cache="${CRAB_CACHE:-/cache}"
mkdir -p "$dist" "$cache"
export DEBIAN_FRONTEND=noninteractive

echo "== build dependencies"
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
    build-essential ca-certificates curl file patchelf pkg-config python3 \
    ninja-build desktop-file-utils appstream \
    libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev \
    libswresample-dev libswscale-dev libavdevice-dev \
    libass-dev libfreetype-dev libfribidi-dev libharfbuzz-dev libfontconfig-dev \
    libva-dev libdrm-dev libegl-dev libgl-dev libgbm-dev \
    libwayland-dev wayland-protocols libxkbcommon-dev \
    libx11-dev libxext-dev libxrandr-dev libxinerama-dev libxss-dev libxpresent-dev \
    libpulse-dev libasound2-dev zlib1g-dev >/dev/null

# A toolchain cached in a volume is used as is.
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if ! command -v cargo >/dev/null || ! cargo --version | grep -q "$rust_version"; then
    echo "== Rust $rust_version"
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

# mpv needs meson 0.62 or newer; Ubuntu 22.04 has 0.61.
if [ ! -f "$cache/meson-1.4.1/meson.py" ]; then
    curl -sSfL --retry 5 --retry-all-errors -o "$cache/meson.tar.gz" \
        https://github.com/mesonbuild/meson/releases/download/1.4.1/meson-1.4.1.tar.gz
    echo "1b8aad738a5f6ae64294cc8eaba9a82988c1c420204484ac02ef782e5bba5f49  $cache/meson.tar.gz" | sha256sum -c -
    tar -C "$cache" -xzf "$cache/meson.tar.gz"
fi
meson() { python3 "$cache/meson-1.4.1/meson.py" "$@"; }

echo "== libmpv $mpv_version"
mpv_prefix="$cache/mpv-$mpv_version"
if [ ! -f "$mpv_prefix/lib/libmpv.so.2" ]; then
    work="$(mktemp -d)"
    curl -sSfL --retry 5 --retry-all-errors -o "$work/mpv.tar.gz" \
        "https://github.com/mpv-player/mpv/archive/refs/tags/v$mpv_version.tar.gz"
    echo "$mpv_sha256  $work/mpv.tar.gz" | sha256sum -c -
    tar -C "$work" -xzf "$work/mpv.tar.gz"
    # Only libmpv: no command line player, no scripting, no optional
    # extras that would pull more libraries into the bundle.
    meson setup "$work/build" "$work/mpv-$mpv_version" \
        --prefix "$mpv_prefix" --libdir lib --buildtype release \
        -Dlibmpv=true -Dcplayer=false -Dlua=disabled -Djavascript=disabled \
        -Dmanpage-build=disabled -Dhtml-build=disabled -Dpdf-build=disabled \
        -Dlibplacebo=disabled -Dvulkan=disabled -Dshaderc=disabled -Dspirv-cross=disabled \
        -Dcdda=disabled -Ddvdnav=disabled -Dlibbluray=disabled -Dlibarchive=disabled \
        -Djack=disabled -Dpipewire=disabled -Dopenal=disabled -Dsdl2=disabled \
        -Dsdl2-audio=disabled -Dsdl2-video=disabled -Dsdl2-gamepad=disabled \
        -Dcaca=disabled -Dsixel=disabled -Dzimg=disabled -Drubberband=disabled \
        -Duchardet=disabled -Dlcms2=disabled -Dvdpau=disabled \
        -Dvaapi=enabled -Dvaapi-drm=enabled -Dvaapi-wayland=enabled -Dvaapi-x11=enabled \
        -Dwayland=enabled -Dx11=enabled -Degl=enabled -Dgl=enabled \
        -Dpulse=enabled -Dalsa=enabled >"$work/meson.log" 2>&1 ||
        { tail -40 "$work/meson.log"; exit 1; }
    ninja -C "$work/build" >"$work/ninja.log" 2>&1 || { tail -40 "$work/ninja.log"; exit 1; }
    ninja -C "$work/build" install >/dev/null
    rm -rf "$work"
fi

echo "== crab-player"
build="$cache/build"
rm -rf "$build"
mkdir -p "$build"
# A clean copy: the read-only checkout may carry a host target/ directory.
tar -C "$src" --exclude=./target --exclude=./dist --exclude=./.git -cf - . | tar -C "$build" -xf -
cd "$build"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
# libmpv2-sys links -lmpv; the reference is resolved by the built-in shim
# and dropped (--as-needed), so this path only satisfies the linker. The
# binary finds the bundled libmpv through its RUNPATH.
export LIBRARY_PATH="$mpv_prefix/lib"
export RUSTFLAGS="-C link-arg=-Wl,-rpath,\$ORIGIN/../lib"
cargo build --release --locked
if readelf -d target/release/crab-player | grep -q 'NEEDED.*libmpv'; then
    echo "the binary links libmpv directly; the runtime loader shim is not in effect" >&2
    exit 1
fi

echo "== AppDir"
app_id="io.github.mahziyarng98.crab-player"
appdir="$cache/AppDir"
rm -rf "$appdir"
lib="$appdir/usr/lib"
fallback="$appdir/usr/lib/fallback"
mkdir -p "$appdir/usr/bin" "$lib" "$fallback" \
    "$appdir/usr/share/applications" "$appdir/usr/share/metainfo"
install -m 755 target/release/crab-player "$appdir/usr/bin/crab-player"
cp -L "$mpv_prefix/lib/libmpv.so.2" "$lib/"

# Libraries that must come from the host: the C runtime, the GPU and
# windowing stack (they have to match the host's drivers and compositor),
# and system services whose configuration lives on the host. Everything
# else is bundled, including libraries whose soname or presence differs
# between distributions (libpcre.so.3 and libselinux are missing on Arch,
# for example).
host_only='^(ld-linux.*|libc|libm|libdl|libpthread|librt|libresolv|libutil|libanl|libnsl|libBrokenLocale|libmvec|libstdc\+\+|libgcc_s|libGL|libGLX|libGLdispatch|libOpenGL|libEGL|libGLESv1_CM|libGLESv2|libgbm|libdrm.*|libvulkan|libsystemd)\.so'
# Used from the host when present, from the bundle otherwise: libva must
# match the host's VA drivers, libasound the host's ALSA plugins, the X11
# and Wayland client libraries the host's Mesa, and fontconfig/freetype/
# harfbuzz/D-Bus the host's configuration, but a minimal system may lack
# any of them. Each one gets its own directory, so
# AppRun adds exactly the ones the host does not have.
fallback_only='^(libva|libva-drm|libva-x11|libva-wayland|libvdpau|libasound|libdbus-1|libudev|libfontconfig|libfreetype|libharfbuzz|libexpat|libz|libuuid|libX11|libX11-xcb|libxcb.*|libXau|libXdmcp|libwayland-.*|libxkbcommon.*)\.so'

copy_deps() { # <elf>: copy the bundleable libraries it needs, recursively
    ldd "$1" | awk '/=> \//{print $1" "$3}' | while read -r name path; do
        if [[ "$name" =~ $host_only ]]; then continue; fi
        dest="$lib"
        if [[ "$name" =~ $fallback_only ]]; then dest="$fallback/$name"; mkdir -p "$dest"; fi
        [ -e "$dest/$name" ] && continue
        cp -L "$path" "$dest/$name"
        copy_deps "$path"
    done
}
copy_deps "$lib/libmpv.so.2"
chmod 644 "$lib"/*.so* "$fallback"/*/*.so* 2>/dev/null || true
# Each bundled library finds its siblings next to itself, without
# LD_LIBRARY_PATH (which would leak into programs the player starts).
for so in "$lib"/*.so*; do patchelf --set-rpath '$ORIGIN' "$so"; done
for so in "$fallback"/*/*.so*; do patchelf --set-rpath '$ORIGIN/../..' "$so"; done

desktop="$appdir/usr/share/applications/$app_id.desktop"
install -m 644 "packaging/$app_id.desktop" "$desktop"
install -m 644 "packaging/$app_id.metainfo.xml" "$appdir/usr/share/metainfo/$app_id.appdata.xml"
for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "assets/icons/hicolor/${size}x${size}/apps/$app_id.png" \
        "$appdir/usr/share/icons/hicolor/${size}x${size}/apps/$app_id.png"
done
ln -s "usr/share/applications/$app_id.desktop" "$appdir/$app_id.desktop"
ln -s "usr/share/icons/hicolor/256x256/apps/$app_id.png" "$appdir/$app_id.png"
ln -s "$app_id.png" "$appdir/.DirIcon"

cat >"$appdir/AppRun" <<'EOF'
#!/bin/sh
# Crab Player AppImage entry point.
here="$(dirname "$(readlink -f "$0")")"
# libva, libvdpau and libasound come from the host whenever it has them
# (they must match the host's VA drivers and ALSA plugins); a bundled copy
# is used only for a library the host does not have.
cache="$( (/sbin/ldconfig -p || /usr/sbin/ldconfig -p || ldconfig -p) 2>/dev/null)"
for dir in "$here"/usr/lib/fallback/*/; do
    [ -d "$dir" ] || continue
    name="$(basename "$dir")"
    if ! printf '%s\n' "$cache" | grep -q "^[[:space:]]*$name "; then
        LD_LIBRARY_PATH="${dir%/}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    fi
done
[ -n "${LD_LIBRARY_PATH:-}" ] && export LD_LIBRARY_PATH
exec "$here/usr/bin/crab-player" "$@"
EOF
chmod 755 "$appdir/AppRun"

echo "== bundled libraries"
ls "$lib" "$fallback"/* | sed 's/^/  /'

echo "== AppImage"
fetch_pinned() { # <url> <sha256> <dest>
    if [ ! -f "$3" ] || ! echo "$2  $3" | sha256sum -c --quiet - 2>/dev/null; then
        curl -sSfL --retry 5 --retry-all-errors -o "$3.part" "$1"
        echo "$2  $3.part" | sha256sum -c --quiet - || { echo "checksum mismatch for $1" >&2; exit 1; }
        mv "$3.part" "$3"
    fi
}
tool="$cache/appimagetool-$appimagetool_version-x86_64.AppImage"
runtime="$cache/runtime-$runtime_version-x86_64"
fetch_pinned "https://github.com/AppImage/appimagetool/releases/download/$appimagetool_version/appimagetool-x86_64.AppImage" \
    "$appimagetool_sha256" "$tool"
fetch_pinned "https://github.com/AppImage/type2-runtime/releases/download/$runtime_version/runtime-x86_64" \
    "$runtime_sha256" "$runtime"
chmod 755 "$tool"
out="$dist/Crab_Player-$version-x86_64.AppImage"
# No FUSE in a container: the tool runs from its extracted copy.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$tool" --no-appstream --runtime-file "$runtime" \
    "$appdir" "$out" >/dev/null
(cd "$dist" && sha256sum "$(basename "$out")" >"$(basename "$out").sha256")

echo "== tarball"
bash "$src/packaging/tarball.sh" "$build/target/release/crab-player" "$dist"
if [ -n "${HOST_UID:-}" ]; then
    chown "$HOST_UID:${HOST_GID:-$HOST_UID}" "$out" "$out.sha256" "$dist"/crab-player-*-linux-x86_64.tar.gz*
fi
ls -l "$out"
