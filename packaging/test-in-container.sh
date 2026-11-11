#!/usr/bin/env bash
# Install a built package in a clean container and check that it plays.
#
#   packaging/test-in-container.sh <image> <artifact>
#
# <artifact> is a .deb, an .rpm or an .AppImage. The container gets only
# what the package declares (apt/dnf/zypper resolve its dependencies) plus
# Xvfb and Mesa for software OpenGL; mpv is never installed by hand. The
# check passes when the player reports its first video frame of a generated
# H.264/AAC clip. The libraries the player process has mapped are printed,
# so it is visible where libmpv, FFmpeg and the GL/VA libraries came from.
#
# Examples:
#   packaging/test-in-container.sh debian:bookworm dist/crab-player_0.1.0-1_amd64.deb
#   packaging/test-in-container.sh fedora:latest dist/crab-player-0.1.0-1.fc43.x86_64.rpm
#   packaging/test-in-container.sh ubuntu:22.04 dist/Crab_Player-0.1.0-x86_64.AppImage
set -euo pipefail

image="${1:?usage: $0 <image> <artifact>}"
artifact="$(readlink -f "${2:?usage: $0 <image> <artifact>}")"
engine="$(command -v docker || command -v podman)"
command -v ffmpeg >/dev/null || { echo "ffmpeg is needed to generate the test clip" >&2; exit 2; }

clip_dir="$(mktemp -d)"
trap 'rm -rf "$clip_dir"' EXIT
ffmpeg -v error -f lavfi -i "testsrc2=s=640x360:r=30:d=8" -f lavfi -i "sine=d=8" \
    -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -shortest "$clip_dir/clip.mkv"

"$engine" run --rm \
    -v "$artifact:/pkg/$(basename "$artifact"):ro" -v "$clip_dir:/clip:ro" \
    "$image" bash -euo pipefail -c '
pkg="$(ls /pkg/*)"
echo "== $(. /etc/os-release; echo "$PRETTY_NAME"), glibc $(ldd --version | head -1 | grep -o "[0-9.]*$")"
case "$pkg" in
  *.deb)      kind=apt ;;
  *.rpm)      kind=$(command -v dnf >/dev/null && echo dnf || echo zypper) ;;
  *.AppImage) kind=$(command -v apt-get >/dev/null && echo apt || (command -v dnf >/dev/null && echo dnf || echo zypper)) ;;
esac
case "$kind" in
  apt)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    extra=""; case "$pkg" in *.deb) extra="$pkg" ;; esac
    apt-get install -y -qq --no-install-recommends xvfb libgl1-mesa-dri libgl1 libegl1 \
        libwayland-client0 libwayland-egl1 libwayland-cursor0 libxkbcommon0 libxkbcommon-x11-0 \
        libxcursor1 libxi6 libxrandr2 $extra >/dev/null ;;
  dnf)
    extra=""; case "$pkg" in *.rpm) extra="$pkg" ;; esac
    dnf install -y -q xorg-x11-server-Xvfb mesa-dri-drivers mesa-libEGL mesa-libGL \
        libwayland-client libwayland-egl libwayland-cursor libxkbcommon libxkbcommon-x11 \
        libXcursor libXi libXrandr findutils $extra >/dev/null ;;
  zypper)
    extra=""; case "$pkg" in *.rpm) extra="$pkg" ;; esac
    zypper --non-interactive -q install --allow-unsigned-rpm xorg-x11-server-Xvfb Mesa-dri \
        Mesa-libEGL1 Mesa-libGL1 libwayland-client0 libwayland-egl1 libwayland-cursor0 \
        libxkbcommon0 libxkbcommon-x11-0 libXcursor1 libXi6 libXrandr2 findutils $extra >/dev/null ;;
esac
if command -v mpv >/dev/null; then echo "note: the mpv command line player is installed too"; fi
case "$pkg" in
  *.AppImage)
    cp "$pkg" /tmp/player.AppImage && chmod +x /tmp/player.AppImage
    # No FUSE in a container: run from the extracted image.
    run=(/tmp/player.AppImage --appimage-extract-and-run) ;;
  *) run=(crab-player) ;;
esac
echo "== libmpv on the system: $(ldconfig -p | grep -c "libmpv\.so\.2 ") entries"
Xvfb :7 -screen 0 1120x700x24 -nolisten tcp >/dev/null 2>&1 &
sleep 1
export DISPLAY=:7 HOME=/tmp/home XDG_RUNTIME_DIR=/tmp/run
mkdir -p "$HOME" "$XDG_RUNTIME_DIR"
RUST_LOG=crab_player=debug,warn CRAB_PLAYER_MPV_OPTIONS=ao=null "${run[@]}" /clip/clip.mkv >/tmp/player.log 2>&1 &
for _ in $(seq 60); do
  grep -q "first video frame" /tmp/player.log && break
  sleep 0.5
done
pid="$(pgrep -f "usr/bin/crab-player|^crab-player" | head -1 || true)"
if [ -n "$pid" ]; then
  echo "== libraries mapped by the player (libmpv, FFmpeg, GL, VA):"
  grep -oE "/[^ ]*(libmpv|libavcodec|libGLX_mesa|libEGL_mesa|libEGL\.so|libGL\.so|libva\.so|swrast|llvmpipe)[^ ]*" /proc/$pid/maps | sort -u | sed "s/^/   /" || true
fi
sed "s/\x1b\[[0-9;]*m//g" /tmp/player.log | grep -E "backend ready|GL renderer|software OpenGL|first video frame|ERROR|libmpv" | cut -c28-200 || true
if grep -q "first video frame" /tmp/player.log; then
  echo "PASS  $(basename "$pkg") plays on this system"
else
  echo "FAIL  no video frame; full log:"; sed "s/\x1b\[[0-9;]*m//g" /tmp/player.log | tail -30; exit 1
fi
'
