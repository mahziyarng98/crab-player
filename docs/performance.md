# Performance

Measured numbers, not estimates. Budgets live in `architecture.md` §7.

## Test conditions

| | |
| --- | --- |
| Date | 2026-11-11 |
| Build | `cargo build --release` (fat LTO, one codegen unit, stripped), Crab Player 0.1.0 |
| Machine | AMD Ryzen 5 7535HS (12 threads), 14 GB RAM, Radeon 680M integrated GPU (a second, discrete Radeon is present but idle) |
| System | Linux 7.2.15, Mesa 26.2.5 (radeonsi), mpv/libmpv 0.41.0, Hyprland 0.56.2 |
| Display | 1920 × 1080 at 144 Hz, scale 1.25, native Wayland unless noted |
| Clips | generated locally with ffmpeg from `testsrc2`, 30 fps, 70 s, no audio: 1080p H.264 High (libx264) and 2160p HEVC Main (libx265) |
| Audio | `ao=null` (no audio device involved) |
| Config | default settings, empty config directory, `hwdec=auto-safe` |

Method:

- **Startup**: wall-clock time from spawning the process to the log
  timestamp of the first video frame presented
  (`RUST_LOG=crab_player::startup=debug`). The in-process marks give the
  breakdown.
- **Memory**: `VmRSS` from `/proc/<pid>/status` after 60 s of playback
  (8 s for idle).
- **CPU**: user + system time of the whole process over 60 s, as a
  percentage of one core.
- **Frame pacing**: over 60 s, through mpv's JSON IPC
  (`input-ipc-server`): the counters `frame-drop-count`,
  `decoder-frame-drop-count` and `vo-delayed-frame-count`, and the spacing
  of `time-pos` change notifications (one per frame mpv hands to the
  renderer). The spacing reflects mpv's frame scheduling; it is not a
  photodiode measurement of what reached the screen.

## Startup

| Case | Spawn → first video frame |
| --- | --- |
| 1080p H.264, warm (5 runs) | 185–199 ms |
| 2160p HEVC, warm (2 runs) | 244–245 ms |
| 1080p H.264, cold (1 run) | 360 ms |
| 1080p H.264, warm, **before** the Phase 5 fixes (3 runs) | 1677–3022 ms |
| 1080p H.264 under Xwayland (1 run) | about 1.7 s (see below) |

"Cold" means the page cache was dropped (`posix_fadvise(DONTNEED)`) for the
binary, the clip and all 204 shared libraries it loads. Without root this is
best effort: libraries mapped by other running programs stay cached, so a
start after a reboot will be slower than this.

Where a warm start goes (1080p, milliseconds since process start):

| Milestone | Time |
| --- | --- |
| Config loaded | 0.1 |
| Window and GL context ready | 65–68 |
| mpv core created | 68–72 |
| Playback backend ready (render context attached) | 78–82 |
| First UI pass | 79–83 |
| First video frame presented | 129–134 |

The remaining ~55 ms of wall time is process spawn and dynamic linking
before `main`.

Two problems were found and fixed in Phase 5:

1. **CUDA probe (950–2200 ms).** Creating the mpv render context probes every
   hardware-decoding interop. The CUDA one calls `cuInit`, which on a
   machine with CUDA libraries but no NVIDIA driver launches
   `nvidia-modprobe` and waits for it to fail. Fix: when no NVIDIA kernel
   driver is present (`/sys/module/nvidia`, `/proc/driver/nvidia` and
   `/dev/nvidiactl` all missing), mpv is started with
   `gpu-hwdec-interop=vaapi`, so only the VA-API interop is loaded. With an
   NVIDIA driver present nothing is restricted and NVDEC keeps working. The
   restriction applies on x86 only, because ARM boards use the `drmprime`
   interop. The NVIDIA branch is covered by a unit test but could not be
   exercised on real hardware here.
2. **Missed first repaint (about 550 ms, intermittent).** Once a file was
   loaded, repaints were left to mpv's frame notifications, and the first
   frame could sit unrendered until an unrelated event caused a repaint
   (mpv logged two 200 ms render timeouts). Fix: the short repaint timer
   keeps running until a picture has actually been presented.

Under X11/Xwayland the first frame takes about 1.7 s. The time goes to
Vulkan initialisation for the `vulkan-copy` decoder (standalone mpv on this
machine also reports 1.4 s "enumerating instance extensions"). Not fixed.

### Re-check after Phase 6

Release build with the Phase 6 changes (non-blocking dialogs, Wayland drop
receiver, crisp logo, undecorated window), 1080p H.264, warm, 5 runs, same
machine:

| Milestone (ms since process start) | Phase 5 | Phase 6 |
| --- | --- | --- |
| Window and GL context ready | 65–68 | 68–100 |
| First video frame presented | 129–134 | 118–183 |

Wall time from spawn to the first frame was 212–279 ms, measured with a
shell polling loop that adds its own latency (Phase 5 used log timestamps
and measured 185–199 ms), so the two wall-time figures are not directly
comparable. The spread comes from window creation, which varied more in
this run; nothing in the playback path was changed. Memory and CPU were not
re-measured.

## Memory

| State | RSS |
| --- | --- |
| Idle, no media | 172 MB |
| 1080p H.264, VA-API | 212 MB |
| 1080p H.264, software decoding | 295 MB |
| 2160p HEVC, VA-API | 276 MB |
| 2160p HEVC, software decoding | 609 MB |

Idle memory is well above the 70 MB budget. Most of it is mapped shared
libraries (Mesa, libmpv and FFmpeg, 204 libraries in total), which RSS
counts in full. It has not been broken down further.

## CPU

| State | CPU (one core = 100 %) |
| --- | --- |
| Idle, no media | 0.1 % |
| 1080p H.264, VA-API (zero-copy) | 6.0 % |
| 1080p H.264, software decoding | 16.8 % |
| 2160p HEVC, VA-API (zero-copy) | 7.3 % |
| 2160p HEVC, software decoding | 58.2 % |

## Frame pacing (60 s, 30 fps clips)

| Case | Frames | Dropped (VO) | Dropped (decoder) | Delayed | Interval mean | Std dev | 99th pct | Max |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1080p H.264, VA-API | 1802 | 0 | 0 | 0 | 33.32 ms | 0.75 ms | 33.50 ms | 34.18 ms |
| 1080p H.264, software | 1802 | 0 | 0 | 0 | 33.33 ms | 0.12 ms | 33.53 ms | 33.82 ms |
| 2160p HEVC, VA-API | 1802 | 1 | 0 | 0 | 33.33 ms | 0.10 ms | 33.53 ms | 33.73 ms |
| 2160p HEVC, software | 1802 | 0 | 0 | 0 | 33.32 ms | 0.61 ms | 33.46 ms | 33.62 ms |

No interval exceeded 1.5 × the frame time in any run. mpv's
`mistimed-frame-count` and `display-fps` are unavailable with the render
API, so display-synchronised jitter was not measured.

## Binary size

| Build | Size |
| --- | --- |
| Release, thin LTO (before) | 13,087,608 bytes (13.1 MB) |
| Release, fat LTO (Phase 5) | 12,214,480 bytes (12.2 MB) |
| Release, fat LTO (Phase 6: Wayland drops, logo, Media Info) | 12,269,040 bytes (12.3 MB) |
| As packaged by `makepkg` (stripped again) | 11,809,088 bytes (11.8 MB) |
| Arch package (`.pkg.tar.zst`, with icons) | 5.3 MB |

`panic = "abort"` was already set and is kept. Fat LTO lengthens a release
build from about 55 s to about 2–3 min on this machine.
