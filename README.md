<h1 align="center">
  <img src="assets/logo/crab-player.png" alt="" width="96" align="middle">
  Crab Player
</h1>

A fast, native media player for Linux with a classic desktop-player layout:
a slim caption strip with the logo and menu, the video, a full-width seek
bar over a centred transport bar, and a docked playlist. Written in Rust, with
[egui](https://github.com/emilk/egui) for the interface and
[libmpv](https://mpv.io) for demuxing, decoding and rendering.

Crab Player is an independent project. It contains no code or assets from
other players.

## Screenshots

Taken from the running app on Hyprland at 1.25× scale.

| Playing | Welcome |
| --- | --- |
| ![Playing a video](docs/screenshots/playing.png) | ![Welcome screen](docs/screenshots/welcome.png) |
| **Main menu** | **Full screen (controls shown)** |
| ![Main menu](docs/screenshots/menu.png) | ![Full screen](docs/screenshots/fullscreen.png) |
| **Error banner** | **Minimum window size** |
| ![A file that cannot be opened](docs/screenshots/error.png) | ![640 × 400 window](docs/screenshots/small.png) |

## Features

- Plays everything the installed FFmpeg decodes: local files, folders
  (added recursively) and network streams (any URL mpv accepts:
  `http(s)`, `rtsp`, `rtmp`, `udp`, `srt`, …). See
  [`docs/codec-support.md`](docs/codec-support.md) for what was tested.
- Opens files every way you expect: command line, file manager / "Open
  with" (`file://` URIs with spaces and non-Latin names), File ▸ Open (a
  native portal dialog that never freezes the window), drag and drop
  (including on native Wayland), Open URL, the playlist's Add button.
- Clear, non-blocking error messages when a file cannot be opened or
  decoded (not found, permission denied, unsupported format, decoder
  error); hardware-decoding failures retry in software automatically.
- Video rendered inside the window through mpv's render API, sharp at
  fractional scale factors, letterboxed, with rotation and aspect-ratio
  override.
- Hardware decoding: VA-API zero-copy on Wayland, with an HW/SW badge that
  shows the active decoder and switches it.
- Media Info dialog: container, codecs, profile, pixel format, resolution,
  frame rate, bitrates, audio channels and sample rate, and the decoder in
  use.
- Audio, subtitle and video track selection; external subtitle files;
  audio and subtitle delay.
- A-B repeat, repeat file / playlist, shuffle, playback speed, go to time.
- Picture controls (brightness, contrast, saturation, hue), save image.
- Playlist with natural sorting, by name or folder, and random order.
- Caption strip with the logo, a main menu and the window buttons; a
  right-click menu on the video; every action also has a command behind
  it, so menus, buttons and keys behave the same.
- Full screen with fading, auto-hiding controls drawn over the picture;
  layout presets; always on top (X11).
- Layout and settings persist in a TOML file.

Planned, and shown disabled in the menus: equalizer, shaders, thumbnails
and GIF capture, DVD/BD navigation, a shortcut editor.

## Install

### Which package should I choose?

| You use | Pick | Why |
| --- | --- | --- |
| Arch, Manjaro, EndeavourOS | the Arch package (`PKGBUILD`, AUR once published) | native, uses the system mpv and FFmpeg |
| Debian 12+, Ubuntu 24.04+ and derivatives | the `.deb` | native, updates with the system libraries |
| Fedora, openSUSE Tumbleweed | the `.rpm` | native; add RPM Fusion / Packman for all codecs |
| Ubuntu 22.04, RHEL 9 / Alma / Rocky, openSUSE Leap 15, or anything else | the Flatpak, or the AppImage | carry their own libmpv and FFmpeg |
| You want nothing installed system-wide | the AppImage | one file, runs from anywhere |
| A distribution with its own libmpv but no package above | the tarball + `install.sh` | prebuilt binary, uses the system libmpv |

Release files appear on the
[releases page](https://github.com/mahziyarng98/crab-player/releases) once a
version is tagged. Every package can also be built from this checkout; see
[`docs/packaging.md`](docs/packaging.md) for how each one is made and what
was tested.

### Arch Linux

```sh
packaging/arch/build-local.sh              # builds target/arch/crab-player-<version>-1-x86_64.pkg.tar.zst
sudo pacman -U target/arch/crab-player-*-x86_64.pkg.tar.zst
```

### Debian and Ubuntu

```sh
sudo apt install ./crab-player_0.1.0-1_amd64.deb
sudo apt install libavcodec-extra          # optional: the codecs Debian ships separately
```

Needs `libmpv2` (Debian 12+, Ubuntu 23.04+). Ubuntu 22.04 only has
`libmpv1`, which is too old: use the Flatpak or the AppImage there.

### Fedora and openSUSE

```sh
sudo dnf install ./crab-player-0.1.0-1.fc45.x86_64.rpm          # Fedora
sudo zypper install ./crab-player-0.1.0-1.fc45.x86_64.rpm       # openSUSE Tumbleweed
```

Fedora's own FFmpeg leaves out H.264, HEVC and other patented codecs.
Enable [RPM Fusion](https://rpmfusion.org) and run
`sudo dnf swap ffmpeg-free ffmpeg --allowerasing` for all of them; on
openSUSE the [Packman](https://en.opensuse.org/Additional_package_repositories#Packman)
repository does the same.

### Flatpak

```sh
flatpak install --user ./Crab_Player-0.1.0-x86_64.flatpak
flatpak run io.github.mahziyarng98.CrabPlayer
```

The Freedesktop runtime it uses brings FFmpeg, and its `codecs-extra`
extension the patented codecs; both are installed from Flathub.

### AppImage

```sh
chmod +x Crab_Player-0.1.0-x86_64.AppImage
./Crab_Player-0.1.0-x86_64.AppImage movie.mkv
```

Runs on any x86_64 distribution with glibc 2.35 or newer, with or without
mpv installed. Graphics and VA-API drivers are the system's own.

### Tarball, or from this checkout

```sh
tar xf crab-player-0.1.0-linux-x86_64.tar.gz && cd crab-player-0.1.0-linux-x86_64
scripts/install.sh                         # for your user, into ~/.local (no root)
sudo scripts/install.sh --prefix /usr/local  # or system-wide
scripts/uninstall.sh                       # removes exactly what install.sh put there
```

From a checkout, `scripts/install.sh` builds the release binary first (Rust
1.95 or newer). For a system-wide install from a checkout, build as yourself
and then install: `cargo build --release --locked && sudo scripts/install.sh
--prefix /usr/local --no-build`. Either way Crab Player shows up in the
launcher and under "Open with" for video and audio files; your default
applications are not changed.

### Runtime dependencies

The `.deb`, `.rpm` and Arch packages pull these in; the Flatpak and the
AppImage carry libmpv and FFmpeg themselves. "All codecs" means everything
the installed FFmpeg decodes.

| | Debian / Ubuntu | Fedora | openSUSE | Arch |
| --- | --- | --- | --- | --- |
| libmpv (client API 2, mpv 0.35+) | `libmpv2` | `mpv-libs` | `libmpv2` | `mpv` |
| FFmpeg | pulled in by `libmpv2`; `libavcodec-extra` for extra codecs | `ffmpeg-free`; `ffmpeg-libs` from RPM Fusion for H.264/HEVC | Packman's `libavcodec` for patented codecs | `ffmpeg` (full build) |
| OpenGL / EGL (Mesa) | `libgl1`, `libegl1`, `libgl1-mesa-dri` | `mesa-libGL`, `mesa-libEGL`, `mesa-dri-drivers` | `Mesa-libGL1`, `Mesa-libEGL1`, `Mesa-dri` | `libglvnd`, `mesa` |
| VA-API | `libva2` | `libva` | `libva2` | `libva` |
| VA-API driver, AMD | `mesa-va-drivers` | `mesa-va-drivers` (`-freeworld` from RPM Fusion for H.264/HEVC) | `Mesa-libva` | `libva-mesa-driver` |
| VA-API driver, Intel | `intel-media-va-driver` (`-non-free` for encoding) | `intel-media-driver` (RPM Fusion) | `intel-media-driver` | `intel-media-driver` |
| NVIDIA (NVDEC) | proprietary driver | proprietary driver (RPM Fusion) | proprietary driver | `nvidia-utils` |
| File dialogs | `xdg-desktop-portal` + `-gtk` or `-kde` | same | same | same |

If libmpv is missing, Crab Player still starts and says which package to
install, on the terminal and in the window.

### Build and run from the checkout

```sh
cargo build --release
./target/release/crab-player /path/to/movie.mkv
```

Build dependencies: Rust 1.95+, a C compiler, `pkg-config` and the libmpv
development files (`libmpv-dev`, `mpv-devel`, or `mpv` on Arch). A UI-only
build without libmpv, for working on the interface:

```sh
cargo build --release --no-default-features
```

### Run

```sh
crab-player                           # welcome screen; drop files on it
crab-player movie.mkv                 # a file or a URL
crab-player ~/Videos                  # a folder, added recursively
crab-player movie.mkv movie.srt       # media plus an external subtitle file
crab-player a.mkv b.mkv               # queue several items
crab-player --version
```

Settings live in `$XDG_CONFIG_HOME/crab-player/config.toml` (usually
`~/.config/crab-player/config.toml`) and are written on exit. A broken
settings file does not stop the player: it starts with defaults, keeps the
file as `config.invalid.toml` and says so. Saved images go to
`~/Pictures/crab-player/`.

## Supported formats

"All codecs" means **everything the FFmpeg installed on your system can
decode**: Crab Player adds no whitelist of its own, and mpv/FFmpeg pick the
demuxer and decoder. On Arch the `ffmpeg` package is a full build. On
Debian/Ubuntu install `libavcodec-extra` for the codecs that are packaged
separately.

Tested here (62 generated clips, all passing): H.264 8/10-bit and 4:2:2 /
4:4:4, HEVC 8/10-bit and 4:4:4, VP8, VP9 (incl. 10-bit), AV1 (incl. 10-bit),
MPEG-1, MPEG-2 (incl. interlaced), MPEG-4 Part 2 / Xvid / DivX 3, H.263,
Theora, ProRes, Motion JPEG, WMV 7/8, Sorenson Spark, FFV1, HuffYUV, DV,
RealVideo; AAC, MP3, MP2, Opus, Vorbis, FLAC, ALAC, AC-3, E-AC-3, DTS,
TrueHD, PCM, WMA, WavPack, TTA, AMR; in Matroska, MP4, WebM, AVI, MOV, MPEG-TS,
MPEG-PS, FLV, Ogg, ASF/WMV, MXF, 3GP, RealMedia and raw DV; SubRip, ASS,
mov_text and WebVTT subtitles, embedded and external. Details, including
which ones decode in hardware, are in
[`docs/codec-support.md`](docs/codec-support.md).

Hardware decoding needs the right VA-API driver: `libva-mesa-driver` (AMD:
H.264, HEVC, VP9, and AV1 on RDNA2 and newer), `intel-media-driver` (Intel
Broadwell and newer), or the NVIDIA driver for NVDEC. Anything the GPU
cannot decode is decoded in software automatically.

## Keyboard and mouse

| Key | Action |
| --- | --- |
| `Space`, `K` | Play / pause |
| `S` | Stop |
| `←` / `→` | Seek 5 s |
| `Shift+←` / `Shift+→` | Seek 60 s |
| `↑` / `↓` | Volume ±5 % |
| `M` | Mute |
| `[` / `]` | Speed down / up |
| `Backspace` | Reset speed |
| `PgUp` / `PgDn` | Previous / next item |
| `F`, `Enter`, middle click | Toggle full screen (double click on the video too) |
| `Esc` | Leave full screen |
| `F9` | Toggle playlist |
| `A` / `J` | Next audio / subtitle track |
| `Z` / `X` | Subtitle delay −/+ 100 ms |
| `Shift+Z` / `Shift+X` | Audio delay −/+ 100 ms |
| `L` | A-B repeat: set A, set B, off |
| `R` / `Shift+R` | Rotate clockwise / counter-clockwise |
| `Shift+A` | Aspect ratio back to the file's |
| `1` / `2` / `3` | Layout preset Minimal / Compact / Normal |
| `Ctrl+M` | Toggle the caption strip and menu (also `Ctrl+0`) |
| `Ctrl+1` / `Ctrl+2` | Toggle seek bar / controls |
| `Ctrl+G` | Go to time |
| `Ctrl+O` / `Ctrl+D` | Open file / folder |
| `Ctrl+U` | Open URL |
| `Ctrl+Shift+S` | Save image |
| `Ctrl+W` | Close media |
| `Ctrl+,` | Options |
| `Ctrl+P` | Media Info |
| `Ctrl+Q` | Quit |

Mouse:

| Input | Where | Action |
| --- | --- | --- |
| Click | video | Play / pause |
| Double click | video | Toggle full screen |
| **Middle click** (wheel button) | video, caption strip, bars, playlist, empty areas | Toggle full screen |
| Wheel | video, control bar | Volume ±2 % |
| Right click | video | Context menu |
| Drag | video (windowed) | Move the window |

A middle click does nothing over a menu or dialog, while a text field is
being edited (Go To, Open URL) or while the seek bar or volume slider is
being dragged.

Bindings are defined in `src/command.rs`.

## Hardware decoding

The default mode is mpv's `auto-safe`; change it in Options or in the config
file (`[playback] hwdec`). The HW/SW badge in the control bar and the
Statistics overlay show what is actually in use.

| Session | What you get |
| --- | --- |
| Wayland | VA-API with zero-copy (`vaapi`). 1080p H.264 measured at 6 % of one core. |
| X11 / Xwayland | Hardware decoding with a copy back (`vulkan-copy` on the test machine), not zero-copy. eframe creates a GLX context on X11, and mpv's VA-API interop needs EGL. Startup is also slower there (about 1.7 s to the first frame on the test machine). |
| Software OpenGL (llvmpipe) | Software decoding, enforced, with simpler scaling. |
| NVIDIA | NVDEC is left to mpv's defaults and has not been tested. |

For diagnostics, extra mpv options can be passed through the environment:

```sh
CRAB_PLAYER_MPV_OPTIONS="log-file=/tmp/mpv.log,msg-level=all=v" crab-player movie.mkv
RUST_LOG=crab_player=debug crab-player movie.mkv
```

## Performance

Measured on a Ryzen 5 7535HS with a Radeon 680M: 0.19 s from launch to the
first video frame, 6 % of one core for 1080p H.264 and 7 % for 4K HEVC with
VA-API, no dropped frames at 1080p over 60 s, and a 12.2 MB binary. Details and
method are in [`docs/performance.md`](docs/performance.md).

## Known limitations

- Idle memory is about 170 MB, above the project's 70 MB target.
- Zero-copy hardware decoding works on Wayland only (see above).
- Always on top depends on the window manager and does nothing on native
  Wayland, which has no protocol for it.
- The 16 px icon is too small for the logo's detail: the crab reads as a
  purple shape and the play button is lost.
- Track selection, delays and picture settings are not remembered per file.
- No subtitle styling options, equalizer, shaders or thumbnail capture yet.
- Packages were built and install-tested in containers and on one Arch
  machine; see [`docs/packaging.md`](docs/packaging.md) for exactly what was
  tested where. Nothing has been published yet (no release, AUR or Flathub
  entry).
- In the Flatpak the clipboard is unavailable (the sandbox does not expose
  Wayland's data-control protocol), so "Copy location" does nothing there.
- On a system without fontconfig or without any installed fonts, the
  player uses egui's built-in fonts only, so file names in scripts they do
  not cover (Persian, CJK, …) show as boxes.
- Always on top cannot work on native Wayland (no protocol); the pin is
  disabled there.
- Settings from the project's earlier working name are copied to the new
  location on first start; the old files are left in place.

## Project layout

```
src/
  main.rs            CLI parsing and eframe bootstrap
  app.rs             window layout, command dispatch, persistence
  media.rs           paths, file:// URIs and URLs; open errors
  dialogs.rs         native file dialogs off the UI thread
  wayland_dnd.rs     file drops on native Wayland
  command.rs         command vocabulary and default key map
  config.rs          TOML settings, migration from the old name
  playlist.rs        playlist model, sorting, folder scanning
  startup.rs         startup timing marks
  player/            Player trait, libmpv backend, null backend
  ui/                theme, logo, icons, caption, menus, video canvas,
                     seek bar, control bar, playlist, error banner
assets/              logo and application icons
docs/                architecture, UI specification, performance, codecs,
                     screenshots
packaging/           desktop entry, AppStream data, AppImage, Debian, RPM,
                     Arch, Flatpak, tarball, container install test
scripts/             install/uninstall, end-to-end render check, codec
                     matrix, middle-click check, libmpv shim generator
.github/workflows/   CI (every push) and release builds (v* tags)
```

## License

MIT. See [LICENSE](LICENSE).
