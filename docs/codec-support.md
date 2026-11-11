# Codec and format support

## What "all codecs" means

Crab Player has no list of allowed formats. Every file goes to libmpv, and
mpv/FFmpeg choose the demuxer and the decoder. So Crab Player plays exactly
what **the FFmpeg installed on the system** can decode, and nothing is
filtered on the way:

- no demuxer or decoder whitelist, no forced `demuxer`/`vd`/`ad` options;
- a file opened explicitly (command line, dialog, drop, "Open with") is
  never checked against a list of extensions; only *folder* scans skip
  files that do not look like media (covers, text files);
- the Open dialog's "Media" filter has an "All files" alternative;
- any URL scheme mpv accepts can be opened (`http(s)`, `rtsp`, `rtmp`,
  `udp`, `srt`, `smb`, …).

Distribution notes:

- **Arch**: `ffmpeg` is a full build (dav1d for AV1, libvpx, every built-in
  decoder). The package depends on it.
- **Debian/Ubuntu**: install `libavcodec-extra` for the codecs packaged
  separately; the Debian package recommends it.

## Hardware decoding never decides whether a file plays

- The default `hwdec=auto-safe` lets mpv use the GPU only for codec and
  profile combinations known to work, and decode everything else in
  software (in the table: H.264 10-bit, 4:2:2, 4:4:4 and HEVC 4:4:4 decode
  in software automatically on this GPU).
- If a file still fails to load with hardware decoding on, Crab Player
  loads it once more with `hwdec=no`, logs `retrying with software
  decoding`, and restores the setting for the next file
  (`retry_or_fail` in `src/player/mpv.rs`). Files that cannot be opened or
  are not media at all are not retried.
- The HW/SW badge and Media Info show `hwdec-current`, i.e. what mpv really
  uses for the current file.

Typical drivers for hardware decoding: `libva-mesa-driver` (AMD: H.264,
HEVC, VP9; AV1 on RDNA2 and newer), `intel-media-driver` (Intel Broadwell
and newer; AV1 from Tiger Lake), `libva-intel-driver` (older Intel), the
NVIDIA driver for NVDEC.

## Verified here

Clips were generated locally with ffmpeg (nothing downloaded) and played
through the real player by `scripts/codec-matrix.py`, which checks for each
clip that playback advanced past 1 s, that the middle of the window shows
the clip's colour (so it is that picture, not black or garbage), that an
audio decoder and an audio output ran, that subtitles were decoded and
drawn, and which decoder mpv used.

Test system (2026-11-11): Arch Linux, mpv/libmpv 0.41.0, FFmpeg 9.0.2,
Mesa 26.2.5 (radeonsi), AMD Radeon 680M, Hyprland 0.56.2, native Wayland.

| Case | Container (demuxer) | Video | Audio | Opens | Picture | Audio plays | Subtitles | Decoder |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| H.264 High 8-bit 4:2:0 / Matroska | mkv | h264 (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| H.264 8-bit 4:2:0 / MP4 | mov,mp4,m4a,3gp,3g2,mj2 | h264 (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| H.264 High 10 (10-bit) / Matroska | mkv | h264 (yuv420p10) | – | yes | yes | – | – | software |
| H.264 High 4:2:2 / Matroska | mkv | h264 (yuv422p) | – | yes | yes | – | – | software |
| H.264 High 4:4:4 / Matroska | mkv | h264 (yuv444p) | – | yes | yes | – | – | software |
| HEVC Main 8-bit / MP4 | mov,mp4,m4a,3gp,3g2,mj2 | hevc (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| HEVC Main 10 (10-bit) / Matroska | mkv | hevc (p010) | – | yes | yes | – | – | hardware (vaapi) |
| HEVC 4:4:4 8-bit / Matroska | mkv | hevc (yuv444p) | – | yes | yes | – | – | software |
| VP8 / WebM | mkv | vp8 (yuv420p) | – | yes | yes | – | – | software |
| VP9 profile 0 / WebM | mkv | vp9 (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| VP9 profile 2 (10-bit) / WebM | mkv | vp9 (p010) | – | yes | yes | – | – | hardware (vaapi) |
| AV1 8-bit / Matroska | mkv | av1 (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| AV1 10-bit / MP4 | mov,mp4,m4a,3gp,3g2,mj2 | av1 (p010) | – | yes | yes | – | – | hardware (vaapi) |
| MPEG-1 / MPEG-PS | mpeg | mpeg1video (yuv420p) | – | yes | yes | – | – | software |
| MPEG-2 progressive / MPEG-TS | mpegts | mpeg2video (yuv420p) | – | yes | yes | – | – | software |
| MPEG-2 interlaced 576i / MPEG-PS | mpeg | mpeg2video (yuv420p) | – | yes | yes | – | – | software |
| MPEG-2 / MXF | mxf | mpeg2video (yuv420p) | – | yes | yes | – | – | software |
| MPEG-4 Part 2 (ASP) / AVI | avi | mpeg4 (yuv420p) | – | yes | yes | – | – | software |
| MPEG-4 Part 2 (Xvid) / AVI | avi | mpeg4 (yuv420p) | – | yes | yes | – | – | software |
| MS MPEG-4 v3 (DivX 3) / AVI | avi | msmpeg4v3 (yuv420p) | – | yes | yes | – | – | software |
| H.263 / 3GP | mov,mp4,m4a,3gp,3g2,mj2 | h263 (yuv420p) | – | yes | yes | – | – | software |
| Theora / Ogg | ogg | theora (yuv420p) | – | yes | yes | – | – | software |
| ProRes 422 10-bit / QuickTime | mov,mp4,m4a,3gp,3g2,mj2 | prores (yuv422p10) | – | yes | yes | – | – | hardware (vulkan-copy) |
| Motion JPEG / AVI | avi | mjpeg (yuv420p) | – | yes | yes | – | – | software |
| Motion JPEG / QuickTime | mov,mp4,m4a,3gp,3g2,mj2 | mjpeg (yuvj422p) | – | yes | yes | – | – | software |
| WMV 7 (wmv1) / ASF | asf | wmv1 (yuv420p) | – | yes | yes | – | – | software |
| WMV 8 (wmv2) / ASF (.wmv) | asf | wmv2 (yuv420p) | – | yes | yes | – | – | software |
| Sorenson Spark (flv1) / FLV | flv | flv1 (yuv420p) | – | yes | yes | – | – | software |
| H.264 / FLV | flv | h264 (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| H.264 / MPEG-TS | mpegts | h264 (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| H.264 / QuickTime | mov,mp4,m4a,3gp,3g2,mj2 | h264 (nv12) | – | yes | yes | – | – | hardware (vaapi) |
| FFV1 (lossless) / Matroska | mkv | ffv1 (yuv420p) | – | yes | yes | – | – | hardware (vulkan-copy) |
| HuffYUV (lossless) / AVI | avi | huffyuv (yuv422p) | – | yes | yes | – | – | software |
| DV (PAL) / raw DV | dv | dvvideo (yuv420p) | – | yes | yes | – | – | software |
| RealVideo 1.0 / RealMedia | rm | rv10 (yuv420p) | – | yes | yes | – | – | software |
| AAC-LC / MP4 audio | mov,mp4,m4a,3gp,3g2,mj2 | – | aac | yes | – | yes | – | – |
| MP3 | mp3 | – | mp3 | yes | – | yes | – | – |
| MPEG-1 Layer II | mp3 | – | mp2 | yes | – | yes | – | – |
| Opus / Ogg | ogg | – | opus | yes | – | yes | – | – |
| Vorbis / Ogg | ogg | – | vorbis | yes | – | yes | – | – |
| FLAC | flac | – | flac | yes | – | yes | – | – |
| ALAC / MP4 audio | mov,mp4,m4a,3gp,3g2,mj2 | – | alac | yes | – | yes | – | – |
| AC-3 5.1 | ac3 | – | ac3 | yes | – | yes | – | – |
| E-AC-3 5.1 | eac3 | – | eac3 | yes | – | yes | – | – |
| DTS 5.1 | dts | – | dts | yes | – | yes | – | – |
| Dolby TrueHD 5.1 | truehd | – | truehd | yes | – | yes | – | – |
| PCM 16-bit / WAV | wav | – | pcm_s16le | yes | – | yes | – | – |
| PCM 24-bit / WAV | wav | – | pcm_s24le | yes | – | yes | – | – |
| WMA v2 / ASF | asf | – | wmav2 | yes | – | yes | – | – |
| WavPack | wv | – | wavpack | yes | – | yes | – | – |
| True Audio | tta | – | tta | yes | – | yes | – | – |
| AMR-NB | amr | – | amr_nb | yes | – | yes | – | – |
| H.264 + AAC / MP4 | mov,mp4,m4a,3gp,3g2,mj2 | h264 (nv12) | aac | yes | yes | yes | – | hardware (vaapi) |
| HEVC 10-bit + E-AC-3 5.1 / Matroska | mkv | hevc (p010) | eac3 | yes | yes | yes | – | hardware (vaapi) |
| VP9 + Opus / WebM | mkv | vp9 (nv12) | opus | yes | yes | yes | – | hardware (vaapi) |
| MPEG-2 + AC-3 / MPEG-TS | mpegts | mpeg2video (yuv420p) | ac3 | yes | yes | yes | – | software |
| WMV 8 + WMA v2 / ASF | asf | wmv2 (yuv420p) | wmav2 | yes | yes | yes | – | software |
| SubRip embedded in Matroska | mkv | h264 (nv12) | – | yes | yes | – | yes | hardware (vaapi) |
| ASS embedded in Matroska | mkv | h264 (nv12) | – | yes | yes | – | yes | hardware (vaapi) |
| mov_text (tx3g) embedded in MP4 | mov,mp4,m4a,3gp,3g2,mj2 | h264 (nv12) | – | yes | yes | – | yes | hardware (vaapi) |
| WebVTT embedded in WebM | mkv | vp8 (yuv420p) | – | yes | yes | – | yes | software |
| External .srt file | mkv | h264 (nv12) | – | yes | yes | – | yes | hardware (vaapi) |
| External .ass file | mkv | h264 (nv12) | – | yes | yes | – | yes | hardware (vaapi) |

63 of 63 generated clips passed; 0 could not be generated by the local ffmpeg.

Notes on reading the table:

- "Video" shows the codec and the format of the decoded picture (inside the
  hardware surface when decoding in hardware).
- "Audio plays": the audio decoder ran and PipeWire opened an output. The
  matrix runs muted, so nothing was listened to; the samples were decoded
  and delivered to the output.
- "Subtitles": the subtitle track was decoded (`sub-text`) and bright text
  was found in the picture.
- `vulkan-copy` (ProRes, FFV1): mpv used FFmpeg's Vulkan decoder with a
  copy back; still correct output.

### Software OpenGL (Xvfb, llvmpipe)

The same matrix under Xvfb, where the renderer is software OpenGL and
hardware decoding is switched off:

All **63 of 63** clips passed: opened, correct picture, audio decoded
(`ao=null`), subtitles drawn, every video decoded in software. A first run
showed 7 scattered failures; they were caused by the test procedure (another
test script killed the matrix's player processes while it ran) and did not
reproduce in two targeted reruns or in a full undisturbed rerun.

### Opening paths

`scripts/e2e-render-check.sh` (Xvfb) checks, by the colour of the rendered
picture, that a clip opened by absolute path with spaces, by a relative
non-ASCII path, and by a `file://` URI with escaped `%` and `#` is drawn,
and that a missing file and a non-media file are reported without the
player exiting. On the real session the open dialog, drag and drop from
the file manager, `file://` URIs, the desktop entry with `%U` and Open URL
(a local HTTP server) were each tested by hand with synthetic input.

## Not testable here (depends on the system)

- Codecs the local ffmpeg cannot **encode** but can decode (VC-1/WMV 9,
  RealVideo 3/4, Cinepak, Indeo, Bink, Dolby Atmos/E-AC-3 JOC metadata,
  DTS-HD MA, …) were not generated, so they are not in the table. They
  play wherever the installed FFmpeg decodes them.
- NVIDIA (NVDEC) and Intel (VA-API) hardware decoding: no such GPU here.
- Encrypted streams (DRM) are not supported by mpv/FFmpeg and never will be.
- Blu-ray/DVD menus: not implemented (menu entries are shown disabled).
- Network protocols beyond plain HTTP were not tested (no servers here).

## Re-running the matrix

```sh
cargo build --release
scripts/codec-matrix.py --session hyprland --out /tmp/matrix.md   # real GPU (Hyprland)
scripts/codec-matrix.py --session xvfb                            # software path, any machine
scripts/e2e-render-check.sh                                       # opening paths
```
