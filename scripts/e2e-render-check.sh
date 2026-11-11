#!/usr/bin/env bash
# End-to-end check: does Crab Player actually put a video frame on screen?
#
# Starts the player on a private Xvfb display with a temporary HOME, opens
# generated clips in every way a path can reach it (plain path with spaces
# and non-ASCII characters, relative path, file:// URI) and asserts for each
# one that
#   1. the player reported its first video frame, and
#   2. the pixels in the middle of the window have the colour of the clip
#      (so it is this video that was drawn, not a black or stale picture).
# It also checks that a missing file is reported and does not end the
# process.
#
# Needs: Xvfb, ffmpeg, ImageMagick (import, magick or convert).
# Usage: scripts/e2e-render-check.sh [path/to/crab-player]
# Exit status: 0 when every case passes.

set -u

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${1:-$root/target/release/crab-player}"
display=":${E2E_DISPLAY:-97}"
size="1120x700"

for tool in Xvfb ffmpeg import; do
    command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 2; }
done
if command -v magick >/dev/null; then im=(magick); else im=(convert); fi
[ -x "$bin" ] || { echo "no player binary at $bin (build it first)" >&2; exit 2; }
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"

work="$(mktemp -d "${TMPDIR:-/tmp}/crab-e2e.XXXXXX")"
xvfb_pid=""
app_pid=""
cleanup() {
    [ -n "$app_pid" ] && kill "$app_pid" 2>/dev/null
    [ -n "$xvfb_pid" ] && kill "$xvfb_pid" 2>/dev/null
    wait 2>/dev/null
    [ -n "${E2E_KEEP:-}" ] && echo "kept: $work" || rm -rf "$work"
}
trap cleanup EXIT

Xvfb "$display" -screen 0 "${size}x24" -nolisten tcp >/dev/null 2>&1 &
xvfb_pid=$!
sleep 1
kill -0 "$xvfb_pid" 2>/dev/null || { echo "Xvfb did not start on $display" >&2; exit 2; }

# Solid-colour clips: the expected pixel is known exactly.
clips="$work/clips with space/پوشه"
mkdir -p "$clips" "$work/home"
make_clip() { # <file> <colour> [encoder args...]
    local file="$1" colour="$2"; shift 2
    ffmpeg -v error -y -f lavfi -i "color=c=$colour:s=640x360:r=30:d=6" \
        -pix_fmt yuv420p "$@" "$file" || { echo "ffmpeg could not create $file" >&2; exit 2; }
}
make_clip "$clips/green clip.mkv" "0x18C038" -c:v libx264 -preset ultrafast
make_clip "$clips/فیلم آبی.mp4" "0x2040E0" -c:v libx264 -preset ultrafast
make_clip "$clips/red 100% #1.webm" "0xD02020" -c:v libvpx -b:v 300k

failures=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; failures=$((failures + 1)); }

# run_player <log> <working dir> [args...]: start the player and leave it
# running.
run_player() {
    local log="$1" cwd="$2"; shift 2
    (
        cd "$cwd" && exec env -u WAYLAND_DISPLAY DISPLAY="$display" HOME="$work/home" \
            XDG_CONFIG_HOME="$work/home/.config" XDG_DATA_HOME="$work/home/.local/share" \
            XDG_PICTURES_DIR="$work/home/Pictures" \
            RUST_LOG="crab_player=debug,warn" CRAB_PLAYER_MPV_OPTIONS="ao=null" \
            "$bin" "$@"
    ) >"$log" 2>&1 &
    app_pid=$!
}
stop_player() {
    [ -n "$app_pid" ] && kill "$app_pid" 2>/dev/null && wait "$app_pid" 2>/dev/null
    app_pid=""
}
# wait_for <log> <text> <seconds>
wait_for() {
    local deadline=$((SECONDS + $3))
    while [ $SECONDS -lt $deadline ]; do
        grep -q "$2" "$1" 2>/dev/null && return 0
        kill -0 "$app_pid" 2>/dev/null || return 1
        sleep 0.2
    done
    return 1
}
# centre_colour <png>: mean "R G B" (0-255) of a patch in the middle of the
# video area. The playlist is hidden for these runs, so the video area is
# the window minus the bars and its centre is the window centre.
centre_colour() {
    "${im[@]}" "$1" -crop 120x80+500+290 +repage -resize 1x1\! \
        -format '%[fx:int(255*r)] %[fx:int(255*g)] %[fx:int(255*b)]' info:
}
# near <actual> <expected>: every channel within 40 of the expected value.
near() {
    local -a a e
    local i d
    read -r -a a <<<"$1"
    read -r -a e <<<"$2"
    for i in 0 1 2; do
        d=$((a[i] - e[i])); [ $d -lt 0 ] && d=$((-d))
        [ $d -le 40 ] || return 1
    done
}

# The playlist panel is hidden so the picture is centred in the window.
mkdir -p "$work/home/.config/crab-player"
printf '[ui]\nshow_playlist = false\n' >"$work/home/.config/crab-player/config.toml"

# check_render <name> <expected "R G B"> <working dir> <args...>
check_render() {
    local name="$1" expected="$2" cwd="$3"; shift 3
    local log="$work/$name.log" shot="$work/$name.png"
    run_player "$log" "$cwd" "$@"
    if ! wait_for "$log" "first video frame" 20; then
        fail "$name: no video frame was reported"
        sed 's/^/      /' "$log" | tail -n 8
        stop_player; return
    fi
    sleep 1
    import -display "$display" -window root "$shot" 2>/dev/null
    local colour; colour="$(centre_colour "$shot")"
    if near "$colour" "$expected"; then
        pass "$name: frame rendered, centre colour $colour"
    else
        fail "$name: centre colour is '$colour', expected about '$expected'"
        cp "$shot" "${TMPDIR:-/tmp}/crab-e2e-$name.png" 2>/dev/null
    fi
    stop_player
}

check_render "absolute-path" "24 192 56" "$work" "$clips/green clip.mkv"
check_render "relative-path-non-ascii" "32 64 224" "$clips" "فیلم آبی.mp4"
uri="file://$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$clips/red 100% #1.webm" 2>/dev/null)"
if [ "$uri" != "file://" ]; then
    check_render "file-uri-with-escapes" "208 32 32" "$work" "$uri"
else
    echo "SKIP  file-uri-with-escapes: python3 is needed to build the URI"
fi

# A file that does not exist: reported, and the player stays up.
log="$work/missing.log"
run_player "$log" "$work" "$work/does not exist.mkv"
if wait_for "$log" "File not found" 10 && sleep 1 && kill -0 "$app_pid" 2>/dev/null; then
    pass "missing-file: reported and the player kept running"
else
    fail "missing-file: not reported, or the player exited"
fi
stop_player

# A file that is not media: mpv's verdict reaches the UI.
printf 'this is not a video\n' >"$work/not media.mkv"
log="$work/notmedia.log"
run_player "$log" "$work" "$work/not media.mkv"
if wait_for "$log" "playback error" 15 && kill -0 "$app_pid" 2>/dev/null; then
    pass "not-media: reported and the player kept running"
else
    fail "not-media: not reported, or the player exited"
fi
stop_player

if [ "$failures" -eq 0 ]; then
    echo "all end-to-end render checks passed"
else
    echo "$failures end-to-end check(s) failed"
    exit 1
fi
