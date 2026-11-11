#!/usr/bin/env bash
# Synthetic-input check for middle-click (wheel button) full screen.
#
# Starts the player on a private Xvfb display with a temporary HOME and
# drives it with xdotool. Every step counts the "fullscreen toggled" debug
# lines the player logs, so it checks the player's own state. There is no
# window manager on the Xvfb display, so the window's _NET_WM_STATE is not
# part of this check; the real-session check covers the compositor side.
#
# Covered: middle click on the video, the caption strip and an empty panel
# area toggles; it does nothing with a context menu open, with Go To or Open
# URL open, over the Media Info dialog, or while the seek bar or the volume
# slider is being dragged; the wheel still changes the volume; a double
# click toggles exactly once; F, Enter and Esc still work; the chrome
# auto-hides in full screen entered by middle click and comes back on
# pointer movement.
#
# Needs: Xvfb, xdotool, ffmpeg.
# Usage: scripts/middle-click-check.sh [path/to/crab-player]

set -u

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${1:-$root/target/release/crab-player}"
display=":${E2E_DISPLAY:-96}"

for tool in Xvfb xdotool ffmpeg; do
    command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 2; }
done
[ -x "$bin" ] || { echo "no player binary at $bin (build it first)" >&2; exit 2; }

work="$(mktemp -d "${TMPDIR:-/tmp}/crab-middle.XXXXXX")"
xvfb_pid=""
app_pid=""
cleanup() {
    [ -n "$app_pid" ] && kill "$app_pid" 2>/dev/null
    [ -n "$xvfb_pid" ] && kill "$xvfb_pid" 2>/dev/null
    wait 2>/dev/null
    [ -n "${E2E_KEEP:-}" ] && echo "kept: $work" || rm -rf "$work"
}
trap cleanup EXIT

Xvfb "$display" -screen 0 1120x700x24 -nolisten tcp >/dev/null 2>&1 &
xvfb_pid=$!
sleep 1
kill -0 "$xvfb_pid" 2>/dev/null || { echo "Xvfb did not start on $display" >&2; exit 2; }
export DISPLAY="$display"

mkdir -p "$work/home/.config/crab-player"
# A short auto-hide delay keeps the run quick.
printf '[ui]\ncontrols_autohide = true\nautohide_ms = 1200\n' \
    >"$work/home/.config/crab-player/config.toml"
ffmpeg -v error -y -f lavfi -i "testsrc2=s=640x360:r=30:d=60" \
    -pix_fmt yuv420p -c:v libx264 -preset ultrafast "$work/clip.mkv" ||
    { echo "ffmpeg could not create the clip" >&2; exit 2; }

log="$work/player.log"
env -u WAYLAND_DISPLAY HOME="$work/home" XDG_CONFIG_HOME="$work/home/.config" \
    XDG_DATA_HOME="$work/home/.local/share" \
    RUST_LOG="crab_player=debug,warn" CRAB_PLAYER_MPV_OPTIONS="ao=null" \
    "$bin" "$work/clip.mkv" >"$log" 2>&1 &
app_pid=$!
for _ in $(seq 50); do
    grep -q "first video frame\|playback backend ready" "$log" 2>/dev/null && break
    sleep 0.2
done
sleep 1.5

# Window layout at 1120x700 (no window manager: the window is at 0,0).
video="420 330"         # middle of the picture
caption="300 16"        # empty part of the caption strip
empty_panel="980 400"   # empty part of the playlist panel
control_bar="300 677"   # empty part of the control bar
volume="960 677"        # volume slider
seek="300 647"          # seek bar

failures=0
# The log is coloured; strip the escape codes before matching.
plain_log() { sed 's/\x1b\[[0-9;]*m//g' "$log"; }
toggles() { plain_log | grep -c "fullscreen toggled"; }
# expect <name> <toggles> <shell commands>
expect() {
    local name="$1" want="$2" before after
    before="$(toggles)"
    eval "$3"
    sleep 0.6
    after="$(toggles)"
    if [ $((after - before)) -eq "$want" ]; then
        echo "PASS  $name (toggled $want×)"
    else
        echo "FAIL  $name: toggled $((after - before))×, expected $want×"
        failures=$((failures + 1))
    fi
}
click_at() { xdotool mousemove $1 click "$2"; }
# Press the left button on <at>, move to <to>, middle-click, release.
middle_during_drag() {
    xdotool mousemove $1 mousedown 1; sleep 0.2
    xdotool mousemove $2; sleep 0.2
    xdotool click 2; sleep 0.2
    xdotool mouseup 1
}

expect "middle click on the video enters full screen" 1 "click_at '$video' 2"
expect "middle click on the video leaves full screen" 1 "click_at '$video' 2"
expect "middle click on the caption strip" 1 "click_at '$caption' 2"
expect "  and back" 1 "click_at '$video' 2"
expect "middle click on an empty panel area" 1 "click_at '$empty_panel' 2"
expect "  and back" 1 "click_at '$video' 2"

expect "context menu open: middle click ignored" 0 \
    "click_at '$video' 3; sleep 0.4; click_at '200 500' 2"
expect "  menu closed: middle click works again" 1 "xdotool key Escape; sleep 0.3; click_at '$video' 2"
expect "  and back" 1 "click_at '$video' 2"

expect "Go To open: middle click ignored" 0 "xdotool key ctrl+g; sleep 0.4; click_at '$video' 2"
expect "  Go To closed" 0 "xdotool key Escape; sleep 0.3"
expect "Open URL open: middle click on its text field ignored" 0 \
    "xdotool key ctrl+u; sleep 0.4; click_at '560 350' 2; click_at '$video' 2"
expect "  Open URL closed" 0 "xdotool key Escape; sleep 0.3"

expect "volume slider drag: middle click ignored" 0 "middle_during_drag '$volume' '940 677'"
expect "seek bar drag: middle click ignored" 0 "middle_during_drag '$seek' '320 647'"

expect "wheel over the control bar: no toggle" 0 "xdotool mousemove $control_bar click 4 click 4"

expect "double click toggles exactly once" 1 "xdotool mousemove $video click --repeat 2 --delay 80 1"
expect "  double click again leaves" 1 "xdotool mousemove $video click --repeat 2 --delay 80 1"
expect "F" 1 "xdotool key f"
expect "Esc leaves" 1 "xdotool key Escape"
expect "Enter" 1 "xdotool key Return"
expect "Enter again" 1 "xdotool key Return"

# Auto-hide after entering full screen with the middle button.
expect "middle click into full screen for auto-hide" 1 "click_at '$video' 2"
sleep 2.5
if plain_log | grep "chrome visibility changed" | tail -1 | grep -q "visible=false"; then
    echo "PASS  chrome auto-hides in full screen"
else
    echo "FAIL  chrome did not auto-hide"; failures=$((failures + 1))
fi
xdotool mousemove 400 300; sleep 0.2; xdotool mousemove 420 330; sleep 0.5
if plain_log | grep "chrome visibility changed" | tail -1 | grep -q "visible=true"; then
    echo "PASS  chrome comes back on pointer movement"
else
    echo "FAIL  chrome did not come back"; failures=$((failures + 1))
fi
expect "middle click leaves full screen" 1 "click_at '$video' 2"

# A dialog without text fields: Media Info, opened from the main menu. It
# opens at the top left, over the video.
expect "Media Info open: middle click on the dialog ignored" 0 \
    "click_at '128 16' 1; sleep 0.5; click_at '150 359' 1; sleep 0.6; click_at '200 300' 2"
expect "  Media Info open: middle click on the video beside it toggles" 1 \
    "click_at '700 330' 2"
expect "  and back" 1 "click_at '700 330' 2"

kill -0 "$app_pid" 2>/dev/null || { echo "FAIL  the player exited"; failures=$((failures + 1)); }
echo
[ "$failures" -eq 0 ] && echo "All middle-click checks passed." || echo "$failures check(s) failed."
[ "$failures" -eq 0 ]
