#!/usr/bin/env python3
"""Codec and container test matrix for Crab Player.

Generates a short clip with the local ffmpeg for every case below, plays
each one through the real player and records, per clip:

  opens     the file loaded and the playback position advanced
  picture   the middle of the player window shows the clip's colour
  audio     an audio decoder ran and an audio output was opened
  decoder   hardware (and which) or software, as reported by mpv

Nothing is downloaded. A case whose encoder or muxer is missing from the
local ffmpeg is reported as "not generated" rather than as a failure.

The player is driven through mpv's JSON IPC socket, enabled for the test
with CRAB_PLAYER_MPV_OPTIONS; settings and data go to a temporary HOME.

Sessions:
  --session hyprland   the real Wayland session (real GPU, VA-API); the
                       window is located with hyprctl and captured with grim
  --session xvfb       a private Xvfb display (software OpenGL, so software
                       decoding); captured with ImageMagick's import

Usage:
  scripts/codec-matrix.py --session hyprland --out results.md
  scripts/codec-matrix.py --session xvfb --only h264,vp9
"""

import argparse
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# The clip is one flat colour, so a correct picture is easy to tell from a
# black, green-garbage or missing one.
COLOUR = "0x18C038"
EXPECTED_RGB = (24, 192, 56)
TOLERANCE = 60
SECONDS = 5

V = ["-f", "lavfi", "-i", f"color=c={COLOUR}:s=640x360:r=30:d={SECONDS}"]
V_PAL = ["-f", "lavfi", "-i", f"color=c={COLOUR}:s=720x576:r=25:d={SECONDS}"]
V_CIF = ["-f", "lavfi", "-i", f"color=c={COLOUR}:s=352x288:r=30:d={SECONDS}"]
A = ["-f", "lavfi", "-i", f"sine=frequency=440:sample_rate=48000:duration={SECONDS}"]
A51 = ["-f", "lavfi", "-i",
       f"sine=frequency=440:sample_rate=48000:duration={SECONDS}", "-ac", "6"]

SRT = "1\n00:00:00,000 --> 00:00:05,000\nCRAB SUBTITLE\n"
ASS = """[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Sans,40,&H00FFFFFF,&H00000000,&H00000000,1,0,1,2,0,2,10,10,20,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,CRAB SUBTITLE
"""


def video(name, ext, label, args, source=V, pix="yuv420p"):
    return dict(kind="video", name=name, ext=ext, label=label, source=source,
                args=["-pix_fmt", pix] + args)


def audio(name, ext, label, args, source=A):
    return dict(kind="audio", name=name, ext=ext, label=label, source=source, args=args)


X264 = ["-c:v", "libx264", "-preset", "ultrafast"]
X265 = ["-c:v", "libx265", "-preset", "ultrafast", "-x265-params", "log-level=error"]

CASES = [
    # --- video codecs -------------------------------------------------
    video("h264-8bit-mkv", "mkv", "H.264 High 8-bit 4:2:0 / Matroska", X264),
    video("h264-8bit-mp4", "mp4", "H.264 8-bit 4:2:0 / MP4", X264),
    video("h264-10bit-mkv", "mkv", "H.264 High 10 (10-bit) / Matroska", X264, pix="yuv420p10le"),
    video("h264-422-mkv", "mkv", "H.264 High 4:2:2 / Matroska", X264, pix="yuv422p"),
    video("h264-444-mkv", "mkv", "H.264 High 4:4:4 / Matroska", X264, pix="yuv444p"),
    video("hevc-8bit-mp4", "mp4", "HEVC Main 8-bit / MP4", X265 + ["-tag:v", "hvc1"]),
    video("hevc-10bit-mkv", "mkv", "HEVC Main 10 (10-bit) / Matroska", X265, pix="yuv420p10le"),
    video("hevc-444-mkv", "mkv", "HEVC 4:4:4 8-bit / Matroska", X265, pix="yuv444p"),
    video("vp8-webm", "webm", "VP8 / WebM", ["-c:v", "libvpx", "-b:v", "400k"]),
    video("vp9-webm", "webm", "VP9 profile 0 / WebM", ["-c:v", "libvpx-vp9", "-b:v", "400k", "-deadline", "realtime"]),
    video("vp9-10bit-webm", "webm", "VP9 profile 2 (10-bit) / WebM",
          ["-c:v", "libvpx-vp9", "-b:v", "400k", "-deadline", "realtime"], pix="yuv420p10le"),
    video("av1-mkv", "mkv", "AV1 8-bit / Matroska", ["-c:v", "libsvtav1", "-preset", "12", "-svtav1-params", "lp=2"]),
    video("av1-10bit-mp4", "mp4", "AV1 10-bit / MP4",
          ["-c:v", "libsvtav1", "-preset", "12", "-svtav1-params", "lp=2"], pix="yuv420p10le"),
    video("mpeg1-mpg", "mpg", "MPEG-1 / MPEG-PS", ["-c:v", "mpeg1video", "-b:v", "1500k"]),
    video("mpeg2-ts", "ts", "MPEG-2 progressive / MPEG-TS", ["-c:v", "mpeg2video", "-b:v", "2000k"]),
    video("mpeg2-interlaced-mpg", "mpg", "MPEG-2 interlaced 576i / MPEG-PS",
          ["-c:v", "mpeg2video", "-b:v", "3000k", "-flags", "+ilme+ildct", "-field_order", "tt"], source=V_PAL),
    video("mpeg2-mxf", "mxf", "MPEG-2 / MXF", ["-c:v", "mpeg2video", "-b:v", "3000k"], source=V_PAL),
    video("mpeg4-avi", "avi", "MPEG-4 Part 2 (ASP) / AVI", ["-c:v", "mpeg4", "-q:v", "4"]),
    video("xvid-avi", "avi", "MPEG-4 Part 2 (Xvid) / AVI", ["-c:v", "libxvid", "-q:v", "4"]),
    video("msmpeg4v3-avi", "avi", "MS MPEG-4 v3 (DivX 3) / AVI", ["-c:v", "msmpeg4", "-q:v", "4"]),
    video("h263-3gp", "3gp", "H.263 / 3GP", ["-c:v", "h263", "-q:v", "4"], source=V_CIF),
    video("theora-ogv", "ogv", "Theora / Ogg", ["-c:v", "libtheora", "-q:v", "6"]),
    video("prores-mov", "mov", "ProRes 422 10-bit / QuickTime",
          ["-c:v", "prores_ks", "-profile:v", "2"], pix="yuv422p10le"),
    video("mjpeg-avi", "avi", "Motion JPEG / AVI", ["-c:v", "mjpeg", "-q:v", "4"], pix="yuvj420p"),
    video("mjpeg-mov", "mov", "Motion JPEG / QuickTime", ["-c:v", "mjpeg", "-q:v", "4"], pix="yuvj422p"),
    video("wmv1-asf", "asf", "WMV 7 (wmv1) / ASF", ["-c:v", "wmv1", "-q:v", "4"]),
    video("wmv2-wmv", "wmv", "WMV 8 (wmv2) / ASF (.wmv)", ["-c:v", "wmv2", "-q:v", "4"]),
    video("flv1-flv", "flv", "Sorenson Spark (flv1) / FLV", ["-c:v", "flv", "-q:v", "4"]),
    video("h264-flv", "flv", "H.264 / FLV", X264),
    video("h264-ts", "ts", "H.264 / MPEG-TS", X264),
    video("h264-mov", "mov", "H.264 / QuickTime", X264),
    video("ffv1-mkv", "mkv", "FFV1 (lossless) / Matroska", ["-c:v", "ffv1"]),
    video("huffyuv-avi", "avi", "HuffYUV (lossless) / AVI", ["-c:v", "huffyuv"], pix="yuv422p"),
    video("dv-pal-dv", "dv", "DV (PAL) / raw DV", ["-c:v", "dvvideo"], source=V_PAL),
    video("rv10-rm", "rm", "RealVideo 1.0 / RealMedia", ["-c:v", "rv10", "-q:v", "4"], source=V_CIF),
    # --- audio codecs -------------------------------------------------
    audio("aac-m4a", "m4a", "AAC-LC / MP4 audio", ["-c:a", "aac"]),
    audio("mp3-mp3", "mp3", "MP3", ["-c:a", "libmp3lame"]),
    audio("mp2-mp2", "mp2", "MPEG-1 Layer II", ["-c:a", "mp2"]),
    audio("opus-opus", "opus", "Opus / Ogg", ["-c:a", "libopus"]),
    audio("vorbis-ogg", "ogg", "Vorbis / Ogg", ["-c:a", "libvorbis"]),
    audio("flac-flac", "flac", "FLAC", ["-c:a", "flac"]),
    audio("alac-m4a", "m4a", "ALAC / MP4 audio", ["-c:a", "alac"]),
    audio("ac3-ac3", "ac3", "AC-3 5.1", ["-c:a", "ac3"], source=A51),
    audio("eac3-eac3", "eac3", "E-AC-3 5.1", ["-c:a", "eac3"], source=A51),
    audio("dts-dts", "dts", "DTS 5.1", ["-c:a", "dca", "-strict", "-2"], source=A51),
    audio("truehd-thd", "thd", "Dolby TrueHD 5.1", ["-c:a", "truehd", "-strict", "-2"], source=A51),
    audio("pcm-wav", "wav", "PCM 16-bit / WAV", ["-c:a", "pcm_s16le"]),
    audio("pcm24-wav", "wav", "PCM 24-bit / WAV", ["-c:a", "pcm_s24le"]),
    audio("wma-wma", "wma", "WMA v2 / ASF", ["-c:a", "wmav2"]),
    audio("wavpack-wv", "wv", "WavPack", ["-c:a", "wavpack"]),
    audio("tta-tta", "tta", "True Audio", ["-c:a", "tta"]),
    audio("amr-amr", "amr", "AMR-NB", ["-c:a", "libopencore_amrnb", "-ar", "8000", "-ac", "1"]),
    # --- audio + video in one file -----------------------------------
    dict(kind="av", name="h264-aac-mp4", ext="mp4", label="H.264 + AAC / MP4",
         source=V + A, args=["-pix_fmt", "yuv420p"] + X264 + ["-c:a", "aac", "-shortest"]),
    dict(kind="av", name="hevc10-eac3-mkv", ext="mkv", label="HEVC 10-bit + E-AC-3 5.1 / Matroska",
         source=V + A51, args=["-pix_fmt", "yuv420p10le"] + X265 + ["-c:a", "eac3", "-shortest"]),
    dict(kind="av", name="vp9-opus-webm", ext="webm", label="VP9 + Opus / WebM",
         source=V + A, args=["-pix_fmt", "yuv420p", "-c:v", "libvpx-vp9", "-b:v", "400k",
                             "-deadline", "realtime", "-c:a", "libopus", "-shortest"]),
    dict(kind="av", name="mpeg2-ac3-ts", ext="ts", label="MPEG-2 + AC-3 / MPEG-TS",
         source=V + A, args=["-pix_fmt", "yuv420p", "-c:v", "mpeg2video", "-b:v", "2000k",
                             "-c:a", "ac3", "-shortest"]),
    dict(kind="av", name="wmv2-wma-wmv", ext="wmv", label="WMV 8 + WMA v2 / ASF",
         source=V + A, args=["-pix_fmt", "yuv420p", "-c:v", "wmv2", "-q:v", "4", "-c:a", "wmav2", "-shortest"]),
    # --- subtitles ----------------------------------------------------
    dict(kind="sub", name="sub-srt-embedded-mkv", ext="mkv", label="SubRip embedded in Matroska",
         sub=("srt", SRT), embed=["-c:s", "srt"]),
    dict(kind="sub", name="sub-ass-embedded-mkv", ext="mkv", label="ASS embedded in Matroska",
         sub=("ass", ASS), embed=["-c:s", "ass"]),
    dict(kind="sub", name="sub-movtext-mp4", ext="mp4", label="mov_text (tx3g) embedded in MP4",
         sub=("srt", SRT), embed=["-c:s", "mov_text"]),
    dict(kind="sub", name="sub-webvtt-webm", ext="webm", label="WebVTT embedded in WebM",
         sub=("srt", SRT), embed=["-c:s", "webvtt"], vcodec=["-c:v", "libvpx", "-b:v", "400k"]),
    dict(kind="sub", name="sub-srt-external", ext="mkv", label="External .srt file", sub=("srt", SRT), external=True),
    dict(kind="sub", name="sub-ass-external", ext="mkv", label="External .ass file", sub=("ass", ASS), external=True),
]


def run(cmd, **kw):
    return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, **kw)


def generate(case, folder):
    """Create the clip; returns (path, extra player args) or (None, reason)."""
    path = os.path.join(folder, f"{case['name']}.{case['ext']}")
    extra = []
    if case["kind"] == "sub":
        ext, text = case["sub"]
        sub_path = os.path.join(folder, f"{case['name']}.{ext}")
        with open(sub_path, "w", encoding="utf-8") as handle:
            handle.write(text)
        vcodec = case.get("vcodec", X264)
        if case.get("external"):
            # Stored under another name so it is not picked up by mpv's own
            # "same name" matching: the player has to load it.
            other = os.path.join(folder, f"{case['name']}-external.{ext}")
            os.replace(sub_path, other)
            cmd = ["ffmpeg", "-v", "error", "-y"] + V + ["-pix_fmt", "yuv420p"] + vcodec + [path]
            extra = [other]
        else:
            cmd = (["ffmpeg", "-v", "error", "-y"] + V + ["-i", sub_path, "-pix_fmt", "yuv420p"]
                   + vcodec + case["embed"] + [path])
    else:
        cmd = ["ffmpeg", "-v", "error", "-y"] + case["source"] + case["args"] + [path]
    done = run(cmd)
    if done.returncode != 0 or not os.path.exists(path) or os.path.getsize(path) == 0:
        reason = (done.stderr.strip().splitlines() or ["ffmpeg failed"])[-1]
        return None, reason[:120]
    return path, extra


class Ipc:
    """Minimal client for mpv's JSON IPC."""

    def __init__(self, path, timeout=10.0):
        deadline = time.time() + timeout
        self.sock = None
        while time.time() < deadline:
            try:
                sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                sock.connect(path)
                sock.settimeout(2.0)
                self.sock = sock
                break
            except OSError:
                time.sleep(0.1)
        self.buffer = b""
        self.request = 0

    def ok(self):
        return self.sock is not None

    def get(self, name):
        if not self.sock:
            return None
        self.request += 1
        message = json.dumps({"command": ["get_property", name], "request_id": self.request}) + "\n"
        try:
            self.sock.sendall(message.encode())
            while True:
                while b"\n" not in self.buffer:
                    chunk = self.sock.recv(65536)
                    if not chunk:
                        return None
                    self.buffer += chunk
                line, self.buffer = self.buffer.split(b"\n", 1)
                reply = json.loads(line)
                if reply.get("request_id") == self.request:
                    return reply.get("data") if reply.get("error") == "success" else None
        except (OSError, ValueError):
            return None

    def close(self):
        if self.sock:
            self.sock.close()


class Session:
    def __init__(self, kind, work):
        self.kind = kind
        self.work = work
        self.xvfb = None
        self.display = ":96"
        self.magick = ["magick"] if shutil.which("magick") else ["convert"]
        if kind == "xvfb":
            self.xvfb = subprocess.Popen(["Xvfb", self.display, "-screen", "0", "1120x700x24", "-nolisten", "tcp"],
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(1.0)

    def env(self, home, ipc):
        env = dict(os.environ)
        env.update(HOME=home, XDG_CONFIG_HOME=os.path.join(home, ".config"),
                   XDG_DATA_HOME=os.path.join(home, ".local/share"), RUST_LOG="crab_player=debug,warn")
        options = [f"input-ipc-server={ipc}"]
        if self.kind == "xvfb":
            env.pop("WAYLAND_DISPLAY", None)
            env["DISPLAY"] = self.display
            options.append("ao=null")
        env["CRAB_PLAYER_MPV_OPTIONS"] = ",".join(options)
        return env

    def capture(self, pid, path):
        if self.kind == "xvfb":
            return run(["import", "-display", self.display, "-window", "root", path]).returncode == 0
        clients = json.loads(run(["hyprctl", "clients", "-j"]).stdout or "[]")
        for client in clients:
            if client.get("pid") == pid:
                (x, y), (w, h) = client["at"], client["size"]
                return run(["grim", "-g", f"{x},{y} {w}x{h}", path]).returncode == 0
        return False

    def measure(self, shot):
        """Mean RGB of the middle of the window and the brightest red value
        in the area where subtitles are drawn."""
        fmt = "%[fx:int(255*r)] %[fx:int(255*g)] %[fx:int(255*b)]"
        centre = run(self.magick + [shot, "-gravity", "center", "-crop", "12%x8%+0+0", "+repage",
                                    "-resize", "1x1!", "-format", fmt, "info:"]).stdout.split()
        text = run(self.magick + [shot, "-gravity", "center", "-crop", "70%x74%+0+0", "+repage",
                                  "-channel", "R", "-separate", "-format", "%[fx:int(255*maxima)]",
                                  "info:"]).stdout.split()
        try:
            return tuple(int(v) for v in centre[:3]), int(text[0])
        except (ValueError, IndexError):
            return None, None

    def close(self):
        if self.xvfb:
            self.xvfb.terminate()


def play(case, path, extra, session, binary, work):
    home = os.path.join(work, "home")
    config = os.path.join(home, ".config", "crab-player")
    os.makedirs(config, exist_ok=True)
    with open(os.path.join(config, "config.toml"), "w") as handle:
        # No playlist panel, so the picture is centred; silent, so the real
        # audio output can be opened without a tone on the speakers.
        handle.write("volume = 0.0\nmuted = true\n[ui]\nshow_playlist = false\n")

    # A socket path may not exceed about 100 bytes: keep it short.
    runtime = os.environ.get("XDG_RUNTIME_DIR") or "/tmp"
    ipc = os.path.join(runtime, f"crab-matrix-{os.getpid()}.sock")
    if os.path.exists(ipc):
        os.remove(ipc)
    log = os.path.join(work, f"{case['name']}.log")
    result = dict(opens=False, picture=None, audio=None, decoder="", note="", container="", vcodec="",
                  acodec="", pixfmt="", sub=None)
    with open(log, "w") as out:
        proc = subprocess.Popen([binary, path] + extra, env=session.env(home, ipc), stdout=out,
                                stderr=subprocess.STDOUT)
    try:
        client = Ipc(ipc)
        if not client.ok():
            result["note"] = "player did not start"
            return result

        # Wait until the position moves: loaded *and* actually playing.
        deadline = time.time() + 12
        position = None
        while time.time() < deadline and proc.poll() is None:
            position = client.get("time-pos")
            if isinstance(position, (int, float)) and position >= 1.0:
                break
            time.sleep(0.15)
        result["opens"] = isinstance(position, (int, float)) and position >= 1.0
        result["container"] = client.get("file-format") or ""
        result["vcodec"] = client.get("video-format") or ""
        result["acodec"] = client.get("audio-codec-name") or ""
        # With a hardware decoder the frame format is the surface type
        # (`vaapi`); the format of the picture inside it is reported apart.
        result["pixfmt"] = (client.get("video-params/hw-pixelformat")
                            or client.get("video-params/pixelformat") or "")
        hwdec = client.get("hwdec-current") or "no"
        has_video = bool(result["vcodec"])
        if has_video:
            result["decoder"] = "software" if hwdec in ("no", "") else f"hardware ({hwdec})"
        if case["kind"] in ("audio", "av"):
            samplerate = client.get("audio-params/samplerate")
            output = client.get("current-ao")
            result["audio"] = bool(result["acodec"]) and bool(samplerate) and bool(output)
            if result["audio"]:
                result["note"] = f"ao={output}"
        if case["kind"] == "sub":
            result["sub"] = (client.get("current-tracks/sub/codec") or "", client.get("sub-text") or "")

        if has_video or case["kind"] in ("video", "av", "sub"):
            shot = os.path.join(work, f"{case['name']}.png")
            time.sleep(0.4)
            if session.capture(proc.pid, shot):
                centre, brightest = session.measure(shot)
                if centre is None:
                    result["note"] = "screenshot could not be measured"
                else:
                    result["picture"] = all(abs(a - e) <= TOLERANCE for a, e in zip(centre, EXPECTED_RGB))
                    result["rgb"] = centre
                    if case["kind"] == "sub":
                        codec, text = result["sub"]
                        result["sub_ok"] = bool(codec) and "CRAB" in text and brightest is not None and brightest >= 200
            else:
                result["note"] = "window could not be captured"
        with open(log) as handle:
            text = handle.read()
        if "retrying with software decoding" in text:
            result["note"] = (result["note"] + " software retry").strip()
        if "playback error" in text:
            result["note"] = (result["note"] + " playback error reported").strip()
        client.close()
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        if os.path.exists(ipc):
            os.remove(ipc)
    return result


def mark(value):
    return {True: "yes", False: "**NO**", None: "–"}[value]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", default=os.path.join(ROOT, "target", "release", "crab-player"))
    parser.add_argument("--session", choices=["hyprland", "xvfb"], default="xvfb")
    parser.add_argument("--out", help="write the Markdown table here as well")
    parser.add_argument("--only", help="comma-separated substrings of case names to run")
    parser.add_argument("--keep", action="store_true", help="keep the clips, logs and screenshots")
    args = parser.parse_args()

    binary = os.path.abspath(args.bin)
    if not os.access(binary, os.X_OK):
        sys.exit(f"no player binary at {binary}")
    needed = ["ffmpeg"] + (["hyprctl", "grim"] if args.session == "hyprland" else ["Xvfb", "import"])
    missing = [tool for tool in needed if not shutil.which(tool)]
    if missing:
        sys.exit("missing tools: " + ", ".join(missing))

    cases = CASES
    if args.only:
        wanted = [part.strip() for part in args.only.split(",")]
        cases = [case for case in CASES if any(part in case["name"] for part in wanted)]

    work = tempfile.mkdtemp(prefix="crab-matrix.")
    clips = os.path.join(work, "clips")
    os.makedirs(clips)
    session = Session(args.session, work)
    rows, failures = [], 0
    try:
        for case in cases:
            path, extra = generate(case, clips)
            if path is None:
                rows.append((case, None, extra))
                print(f"SKIP  {case['name']}: not generated ({extra})", flush=True)
                continue
            result = play(case, path, extra, session, binary, work)
            if case["kind"] == "audio":
                good = result["opens"] and result["audio"]
            elif case["kind"] == "sub":
                good = result["opens"] and result["picture"] and result.get("sub_ok")
            elif case["kind"] == "av":
                good = result["opens"] and result["picture"] and result["audio"]
            else:
                good = result["opens"] and result["picture"]
            failures += 0 if good else 1
            rows.append((case, result, good))
            print(f"{'PASS' if good else 'FAIL'}  {case['name']}: opens={result['opens']} "
                  f"picture={result['picture']} rgb={result.get('rgb')} audio={result['audio']} "
                  f"decoder={result['decoder']} sub={result.get('sub')} {result['note']}", flush=True)
    finally:
        session.close()
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)

    lines = ["| Case | Container (demuxer) | Video | Audio | Opens | Picture | Audio plays | Subtitles | Decoder |",
             "| --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for case, result, good in rows:
        if result is None:
            lines.append(f"| {case['label']} | – | – | – | not generated: {good} | – | – | – | – |")
            continue
        vcodec = result["vcodec"] + (f" ({result['pixfmt']})" if result["pixfmt"] else "")
        subs = mark(result.get("sub_ok")) if case["kind"] == "sub" else "–"
        lines.append(f"| {case['label']} | {result['container']} | {vcodec or '–'} | {result['acodec'] or '–'} "
                     f"| {mark(result['opens'])} | {mark(result['picture'])} | {mark(result['audio'])} "
                     f"| {subs} | {result['decoder'] or '–'} |")
    table = "\n".join(lines)
    played = [row for row in rows if row[1] is not None]
    summary = (f"{len(played) - failures} of {len(played)} generated clips passed; "
               f"{len(rows) - len(played)} could not be generated by the local ffmpeg.")
    print()
    print(table)
    print()
    print(summary)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(table + "\n\n" + summary + "\n")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
