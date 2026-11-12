# Architecture

## 1. Goals and constraints

| Goal | Consequence |
| --- | --- |
| Native Linux only | X11 and Wayland first-class; no Wine, no embedded Windows runtime, no wrapper processes. |
| Small and efficient | One process, one UI thread, no webview, no Electron. Idle cost must stay in the low tens of MB and near-zero CPU. |
| Instant startup | No media framework initialisation before the first frame; the backend is created lazily enough that the window appears immediately. |
| Real playback features | Hardware decoding, subtitle rendering, network streams and audio filters are non-negotiable. |
| Packagable | Arch, Debian and Flatpak from one source tree, no distro-specific patches. |

Non-goals for Phase 1: DVD/BD navigation, VR/3D presentation modes, live TV
tuners, in-app subtitle download services.

## 2. Layers

```
┌──────────────────────────────────────────────────────────────┐
│ ui/            egui widgets, menus, panels, OSD, theme       │
├──────────────────────────────────────────────────────────────┤
│ app.rs         layout, command dispatch, autohide, persistence│
├──────────────────────────────────────────────────────────────┤
│ command.rs     one Command enum shared by menus/keys/buttons  │
├──────────────────────────────────────────────────────────────┤
│ player::Player trait  +  PlayerSnapshot  +  PlayerEvent       │
├───────────────────────────────┬──────────────────────────────┤
│ player::mpv (libmpv FFI)      │ player::null (virtual clock) │
└───────────────────────────────┴──────────────────────────────┘
```

The UI never calls libmpv. It reads a `PlayerSnapshot` once per frame and
issues intents through the `Player` trait. This keeps the door open for a
second backend (for example a pure-Rust decode pipeline) without touching a
single widget, and it makes the UI fully testable with the `null` backend.

`PlayerSnapshot` is a plain value: state, position, duration, volume, mute,
speed, title, path, video size, track lists, audio/subtitle delay and the
active hardware decoder. `PlayerEvent` carries the few things that are
edge-triggered: file loaded, end of file, error.

### The `Player` trait

Intents are methods; state comes back only through the snapshot, so every
method can return as soon as the intent is queued.

| Group | Methods |
| --- | --- |
| Transport | `load`, `play`, `pause`, `toggle_pause`, `stop` |
| Position | `seek` (exact), `scrub` (fast, keyframe; defaults to `seek`) |
| Mixer | `set_volume`, `set_muted`, `set_speed` |
| Tracks | `select_track(kind, Option<id>)`, `add_subtitle_file` |
| Sync | `set_audio_delay`, `set_subtitle_delay` |
| Picture | `set_aspect_override`, `set_rotation`, `set_picture`, `set_ab_loop`, `screenshot` |
| Decoding | `set_hardware_decoding` |
| State | `snapshot`, `drain_events`, `backend_name` |

Tracks are `TrackInfo { id, title, language, codec, default, selected,
external }`, grouped by `TrackKind` (audio, subtitle, video) into
`Arc<[TrackInfo]>` slices, so cloning a snapshot every frame stays cheap.
`select_track(kind, None)` turns a kind off, which is how subtitles are
disabled. In the mpv backend the ids are mpv's own track ids and selection
maps to the `aid`/`sid`/`vid` properties; delays map to `audio-delay` and
`sub-delay`. The `null` backend has no tracks and rejects selection with an
error instead of pretending.

## 3. Why egui + libmpv

**egui (0.37, `eframe` with the glow renderer).** Immediate mode suits a
player: chrome is a function of state, and there is no widget tree to
reconcile at 60 Hz. It is MIT/Apache-2.0, has no runtime dependency beyond
winit/glutin, and — critically — exposes the underlying OpenGL context, which
is what the mpv render API needs. Slint was evaluated and rejected: its
GPLv3/attribution terms for non-commercial use add friction, and its scene
graph gives no advantage for a mostly custom-painted transport bar.

**libmpv.** It is the only mature Linux playback library that covers the
required matrix in one dependency: FFmpeg demux/decode, VA-API/NVDEC/VDPAU,
subtitle rendering (ASS/SSA), audio filters, network protocols, and a stable
render API. Linking against the system `libmpv.so.2` keeps the package small
and lets distro security updates apply. `libmpv2` crates supply the safe
property and command API.

**alhazen-core** (pure Rust, VA-API/NVDEC) is tracked as an experimental
future backend. It is not used in Phase 1: no DVD/BD, no advanced subtitle
renderer, no shader pipeline, and a young API. The `Player` trait is the seam
where it could be added.

## 4. Frame and threading model

```
UI thread (winit, GL)            mpv-control thread           mpv internal threads
---------------------            ------------------           --------------------
eframe → PlayerApp::ui           recv() ◄─ Request ─┐         demux / decode / audio
  ├─ tick(): snapshot + events     ├─ loadfile, seek, set…    core, VO
  ├─ keyboard / menus → Command    └─ drain mpv events ◄─ wakeup callback
  ├─ Player intent ── Request ───────────┘   │
  │                                          ▼
  ├─ read State ◄───────── Mutex<State> ◄── fold events / observed properties
  └─ painter (GL) ◄── repaint ── render update callback (§5)
```

- All windowing and GL calls stay on the UI thread, as required by both
  Wayland and OpenGL. Every `mpv_render_*` call happens there too, which
  makes the UI thread mpv's *render thread*.
- libmpv forbids a render thread from calling its synchronous client API
  (`mpv_command`, `mpv_get_property`, …): the core may be waiting for a frame
  at that moment, and the two only come apart after a 200 ms timeout. Before
  Phase 4 the UI thread did exactly that, and every seek stalled the UI for
  about 200 ms (`mpv_render_context_render() not being called or stuck` in
  the mpv log). All client-API traffic therefore runs on a dedicated
  **mpv-control** thread:
  - `MpvPlayer` (UI side) turns intents into `Request`s on a channel and
    updates `State` optimistically, so a button flips at once.
  - The control thread sleeps in `recv()`. It is woken by a request or by
    mpv's wakeup callback, executes requests, and folds mpv's events into
    `State`. Queued seeks are coalesced to the newest one.
  - It never holds the state lock across an mpv call, so the UI thread never
    waits on the mpv core.
- State changes arrive as `mpv_observe_property` notifications (`time-pos`,
  `duration`, `pause`, `eof-reached`, `dwidth`/`dheight`, `video-params/
  rotate`, `track-list`, `hwdec-current`, …) instead of per-frame polling.
  mpv only notifies a value that differs from the last one it sent, so these
  fields are mirrors that are never reset locally; what must not leak into
  the next item is masked while a load is pending.
- While a video is playing, repaints are driven by mpv: one per new frame,
  requested from mpv's thread (§5). There is no fixed-rate repaint timer.
  Audio-only playback and the `null` backend have no frames to wake on and
  keep a 16 ms timer for the seek bar.
- Idle or paused, the app wakes at 4 Hz, which is enough for the OSD fade
  and the clock. Measured cost is about 0.2 % of one core.

## 5. Video output

The picture is drawn by libmpv's render API (`vo=libmpv`) and composited into
the egui window by a paint callback. The `Player` trait is not involved: the
render context is an inherent feature of the mpv backend
(`MpvPlayer::attach_video`), and `ui/video.rs` only sees a small
`VideoRenderer` handle.

```
GL context (eframe/glow)
  └─ CreationContext::get_proc_address + native display handle
       └─ mpv_render_context_create            MpvPlayer::attach_video
            └─ update callback  (mpv thread)   "a new frame is ready"
                 └─ egui::Context::request_repaint
                      └─ egui::PaintCallback   VideoSurface::paint
                           ├─ mpv_render_context_update   new frame?
                           ├─ mpv_render_context_render → offscreen FBO
                           │                              (physical pixels)
                           ├─ GL state reset
                           └─ glBlitFramebuffer → eframe's framebuffer
```

### Pipeline

1. **GL context → `get_proc_address`.** `PlayerApp::new` runs inside eframe's
   app creator, on the main thread with the GL context current. It builds a
   `VideoInit` for `player::create`:
   - `get_proc_address`, the loader of eframe's glow context;
   - a clone of the `egui::Context`, for repaints;
   - the window's **native display** (`wl_display *` on Wayland, Xlib
     `Display *` on X11), read through `raw-window-handle` from the
     `CreationContext`;
   - `GL_RENDERER`, to recognise software rasterizers.
2. **Render context.** `MpvPlayer::attach_video` calls
   `mpv_render_context_create` with `MPV_RENDER_API_TYPE_OPENGL`,
   `MPV_RENDER_PARAM_OPENGL_INIT_PARAMS` built from that loader, and
   `MPV_RENDER_PARAM_WAYLAND_DISPLAY` or `MPV_RENDER_PARAM_X11_DISPLAY`.
   Before `mpv_initialize` the backend sets `vo=libmpv`,
   `video-timing-offset=0` (so `mpv_render_context_render` never sleeps on
   the UI thread; egui owns vsync) and `keepaspect=no` (see *HiDPI*). If
   there is no GL context, or mpv rejects it, the error is logged
   (`video output unavailable; audio only`) and playback continues without
   video; nothing panics. Both cases are covered by unit tests.
3. **Update callback → `request_repaint`.** mpv invokes the update callback
   on one of its own threads whenever a new frame is ready. The callback does
   exactly one thing: `egui::Context::request_repaint()`, which is
   thread-safe and wakes the event loop. No mpv or GL call is made there.
4. **`PaintCallback`.** `ui::video::show` paints the black canvas, then queues
   an `egui::PaintCallback` (`egui_glow::CallbackFn`) covering the canvas,
   then the OSD and statistics so they stay on top. The callback is queued
   whenever media is open, even before mpv has reported a picture size. mpv
   hands each frame to the render thread and waits for it to be taken; if
   nothing took frames until the size arrived, the core would stall on its
   200 ms timeout exactly while trying to report that size. Such early
   frames are rendered into a 16 × 16 scratch target and never shown.
5. **Render in physical pixels.** Inside the callback the destination
   rectangle is computed in device pixels (see *HiDPI* below), and an
   offscreen FBO of exactly that size is created or resized. The callback
   calls `mpv_render_context_update` and renders only if
   `MPV_RENDER_UPDATE_FRAME` is set, or if the FBO was just (re)created and
   holds no picture yet. Any other repaint (pointer movement, OSD, menus)
   reuses the cached frame. The target mirrors the window's orientation, so
   mpv renders with `flip_y = 1`.
6. **State reset and blit.** libmpv expects standard GL defaults on entry and
   leaves its own state behind, while egui_glow paints with scissor test and
   blending enabled. Before the render the callback disables scissor/blend
   and unbinds egui's program, VAO, array buffer and texture. Afterwards it
   resets program, VAO, array and pixel pack/unpack buffers, active texture
   and 2D binding, pixel-store modes, depth/stencil/cull, colour mask, blend
   function, scissor box and viewport, and rebinds the framebuffer. The frame
   is then copied 1:1 with `glBlitFramebuffer` into the framebuffer that was
   bound when the callback started. That binding is queried
   (`GL_DRAW_FRAMEBUFFER_BINDING`), never assumed to be 0. The scissor box is
   egui's clip rectangle, so the picture cannot spill over the chrome.

The offscreen FBO is not optional. The render API has no destination
rectangle: mpv takes ownership of the whole framebuffer it is handed and
clears it, so rendering straight into eframe's framebuffer would wipe the
caption strip and panels. The FBO also makes step 5 possible: egui clears and
redraws the window on every repaint, so without a cached frame every hover
would force a full mpv render pass.

### Threading model

| Thread | Does | Never does |
| --- | --- | --- |
| UI (winit/GL) | Creates, uses and frees the render context; all GL calls; all egui painting; queues `Request`s. | Calls the synchronous mpv client API. |
| mpv-control | Runs `Request`s (`loadfile`, `seek`, property writes, `sub-add`); drains mpv events into `State`. | Touches GL; holds the state lock across an mpv call. |
| mpv wakeup / render-update callbacks | Queue one `Request::Wake`; call `request_repaint()`. | Call into mpv or GL. |
| mpv decode/audio threads | Demux, decode, audio out. | Anything of ours. |

`egui_glow::CallbackFn` requires `Send + Sync`, but an `mpv_render_context`
is bound to the GL thread. The handle therefore lives in a small wrapper
(`GlBound` in `player/mpv.rs`) that records the creating thread and refuses
access from any other; that wrapper carries the only `unsafe impl Send/Sync`
in the code base. The remaining `unsafe` is the lifetime erasure next to it
and the raw `glow` calls in the private `ui::video::surface` module. Each
block has a `SAFETY` comment.

### Lifetime and drop order

`mpv_render_context_free` must run before `mpv_terminate_destroy`, on the GL
thread, with the context current. Three layers guarantee the order:

1. `eframe::App::on_exit` (GL context current) calls `VideoSurface::destroy`,
   which frees the render context and deletes the FBO and its texture. This
   is the normal path.
2. `MpvPlayer` implements `Drop`: it releases the render context, then sends
   `Request::Shutdown` and joins the control thread, so that thread's handle
   clone is gone too. The `video` field is also declared before the `mpv`
   handle.
3. The shared state behind `VideoRenderer` holds its own `Arc<Mpv>` in a
   field declared after the render-context slot. Even if a queued paint
   callback is the last owner, the context is freed first and the handle
   last.

Releasing is idempotent; after it, `VideoRenderer::render` is a no-op.

### HiDPI and fractional scaling

egui lays out in logical points; the compositor scale (`pixels_per_point`)
can be fractional (1.25, 1.5, …). Everything about the picture is computed in
physical pixels, at paint time, from the values egui_glow passes to the
callback:

- `physical_rect` multiplies the canvas rectangle by `pixels_per_point` and
  rounds each **edge** to a device pixel (rounding the size instead would
  drift by one pixel against neighbouring panels), then clamps to the screen.
- `letterbox` fits the video's display aspect ratio inside that pixel
  rectangle with integer arithmetic and centres it. The bars are simply the
  black canvas the UI already painted; mpv fills the computed sub-rectangle.
  The aspect comes from mpv's `dwidth`/`dheight` (pixel aspect ratio
  applied), swapped for 90°/270° rotation metadata.
- The FBO has exactly the size of that sub-rectangle and is blitted without
  scaling, so mpv's scaler is the only resampling step and the picture is as
  sharp as the display allows.
- mpv runs with `keepaspect=no`. The aspect ratio has already been applied
  by `letterbox`; left to its own logic, mpv rounded the picture size
  independently and could leave a one-pixel black column or row inside the
  target (seen at an 803 × 600 window). Measured after the change with
  pure-white clips: the picture fills its target exactly at 22 window sizes
  and two aspect ratios, at egui zoom levels 0.8–2.0, and at the native
  1.25 Wayland scale.

Because the size is derived on every paint, a window resize or a scale
change (moving between monitors, or egui zoom) just produces a different
pixel rectangle; the FBO is recreated and mpv redraws the current frame at
the new size, also while paused. Both functions are pure and covered by unit
tests. Rotation metadata (90°/180°/270°) and non-square pixels (720 × 480 at
SAR 32:27, shown 853 × 480) were checked visually on the real GPU.

### Hardware decoding

`hwdec=auto-safe` lets mpv choose a hardware decoder and fall back to
software by itself when none works. The active decoder (`hwdec-current`, or
"software") is shown in the Statistics overlay.

Passing the native display handle matters on multi-GPU machines. Without
it, mpv's VA-API interop opens the first DRM render node, which can be a
different GPU from the one the compositor and our GL context use. With it,
libva opens the compositor's display.

| Session | Result (Radeon 680M, Mesa radeonsi, H.264 and HEVC 1080p30) |
| --- | --- |
| Native Wayland | `Using hardware decoding (vaapi)`: zero-copy dmabuf import via EGL, about 5 % of one core. |
| X11 / Xwayland | `vulkan-copy`: hardware decoding with a copy back. eframe creates a GLX context (glutin `FallbackEgl` is hard-coded), and mpv's VA-API GL interop needs EGL. |
| Software GL (llvmpipe, Xvfb) | Software decoding, enforced (see below). |

A software rasterizer is detected from `GL_RENDERER` (llvmpipe, softpipe,
swrast, SwiftShader, lavapipe), and two workarounds are applied:

- `hwdec=no`, because VA-API interop "succeeds" there and then presents a
  black picture. The HW/SW badge is then locked to software.
- `dither-depth=no` and bilinear `scale`/`dscale`/`cscale`, because with
  mpv's lookup textures (dither matrix, scaler weights) llvmpipe
  intermittently (about 2 of 3 starts) drew a grid of black rows and columns,
  or a black picture, through the render API. Disabling dithering alone left
  1 of 10 starts affected; with both changes 0 of 16 were. Standalone mpv on
  the same stack and every real-GPU run were clean, and the Mesa-side cause
  is not known. Both only refine the picture, and bilinear is far cheaper on
  a CPU rasterizer.

Render-context creation probes every hwdec interop mpv was built with, on
the UI thread. The CUDA probe took 1-2 s on a machine with CUDA libraries
but no NVIDIA driver (it runs `nvidia-modprobe`). When no NVIDIA kernel
driver is present, the backend therefore starts mpv with
`gpu-hwdec-interop=vaapi` (x86 only; ARM keeps the default for `drmprime`).
With the driver present nothing is restricted, so NVDEC keeps working.

Fallback if FBO interop fails on a driver: surface-embed via
`--wid`/`--vo=gpu-next` on X11 only, with a visible warning. Wayland has no
equivalent escape hatch, which is why the FBO path is the primary design.

Colour management, HDR (`target-colorspace-hint`), 10-bit output and
debanding are mpv properties; the UI only needs to expose them later.

### Diagnostics

`CRAB_PLAYER_MPV_OPTIONS` passes extra mpv options as a comma-separated
`name=value` list, applied last, for example
`CRAB_PLAYER_MPV_OPTIONS="log-file=/tmp/mpv.log,msg-level=all=v"`.
`vo` is refused because the render API depends on `vo=libmpv`, and an
invalid option is logged and skipped rather than preventing startup.

### Opening media

Every way of opening something ends in `PlayerApp::open_entries`: command
line arguments, the desktop entry (`%U`), File ▸ Open, the playlist's Add
button, Open URL, and drops. `src/media.rs` turns each entry into a
`Source`:

- `file:` URIs (what file managers and `%U` pass) are percent-decoded into
  a path, byte for byte, so spaces, `%`, `#`, non-ASCII and even non-UTF-8
  names survive; URIs naming another host are refused;
- anything else with a valid `scheme://` is a stream URL and goes to mpv
  unchanged, whatever the scheme;
- everything else is a path, made absolute against the working directory.

Before a local path is queued, `media::check_path` tells "not found",
"permission denied" and "folder without media" apart, so the user sees the
real reason. A file named explicitly is never filtered by its extension;
only folder scans use the media extension list.

**File dialogs never block the UI thread** (`src/dialogs.rs`): each portal
dialog runs on its own thread with `rfd::AsyncFileDialog` and reports back
through a channel. A dialog stays open while the user browses; if the UI
thread waited for it, the window would stop answering the compositor's
pings and Hyprland would put an "application not responding" prompt over
the dialog. This was the visible symptom of "videos do not open" through
File ▸ Open on Wayland.

**Drops on native Wayland** (`src/wayland_dnd.rs`): winit 0.30 only
delivers dropped files on X11. On Wayland the module attaches a second
event queue to the window's own `wl_display` (`Backend::from_foreign_display`)
and binds `wl_data_device_manager` there. It accepts offers that carry
`text/uri-list`, reads the payload from a pipe on a worker thread and hands
the URIs to `open_entries`. winit's loop reads the socket; libwayland sorts
our objects' events into our queue, which is dispatched once per frame.
eframe's `clipboard` feature is off on purpose: it pulls in
smithay-clipboard, whose own data device made Hyprland send all drag events
there and none to ours. The clipboard comes from arboard (data-control
protocol / X11 selections) through `egui-winit` instead.

**Failures are reported, never swallowed.** mpv's `MPV_EVENT_END_FILE`
error code is mapped to a reason (`failure_reason` in `player/mpv.rs`);
errors during loading *and* in the middle of playback reach the UI as
`PlayerEvent::Error` and show as a banner. A file that fails to load with
hardware decoding enabled is loaded once more with `hwdec=no` before an
error is shown (not for "cannot open"/"unknown format", where no decoder
ran); the mode is restored for the next file and the HW/SW badge shows
what mpv actually uses. mpv's own `auto-safe` mode already falls back to
software per stream when the hardware decoder rejects a codec or profile.

**Settings never break startup.** A missing config is silent; an unreadable
or invalid one falls back to defaults, is kept as `config.invalid.toml`
(so the save on exit does not destroy hand edits), and a banner says so.

## 6. Provenance

Crab Player is an independent, clean-room implementation. Its layout is the
familiar desktop-player shape (caption strip with menu, video, transport bar, docked
playlist) and its feature set is what users of such players expect; none of
it is taken from another program.

- **No third-party code.** Everything outside the crates in `Cargo.toml` and
  the system `libmpv` is written for this project.
- **No third-party assets.** No icons, bitmaps, strings, skins or resource
  dumps from other software are in the repository or the binary. Control
  icons are drawn with `egui::Painter` primitives; the only image asset is
  the project's own logo under `assets/`. The classic-player look of the
  caption strip, transport bar and playlist was designed from general
  knowledge of the genre, not from another player's resources.
- **Colours** come from that logo and are defined once, in `ui::theme`.

## 7. Resource budgets

Measured values are from [`performance.md`](performance.md) (2026-11-11,
Ryzen 5 7535HS, Radeon 680M, native Wayland).

| Metric | Target | Measured |
| --- | --- | --- |
| Warm start to first video frame | < 250 ms | 185-199 ms (1080p), 245 ms (2160p) |
| Cold start to first video frame | < 500 ms | 360 ms (best-effort cache drop) |
| Idle RSS (no media) | < 70 MB | 172 MB: over budget, mostly mapped shared libraries |
| 1080p H.264 playback CPU (VA-API) | < 8 % of one core | 6.0 % |
| 2160p HEVC playback CPU (VA-API) | < 15 % of one core | 7.3 % |
| Idle CPU | near zero | 0.1 % of one core |
| Dropped frames, 60 s of 1080p | 0 | 0 |
| Binary size, release + strip | <= 13 MB (glow, no wgpu) | 12.3 MB |

`Cargo.toml` release profile: fat LTO, one codegen unit, stripped, abort on
panic. wgpu is deliberately not compiled in; glow is smaller and matches the
mpv render API.

## 8. Packaging

Formats, build scripts and test results are in
[`packaging.md`](packaging.md). Two design points matter for the code:

- **libmpv is opened at runtime.** `player::mpv_shim` (generated by
  `scripts/gen-mpv-shim.py` from the libmpv2-sys bindings) defines every
  `mpv_*` function as a `#[no_mangle]` forwarder that resolves the real
  symbol with `dlsym` on first use. The linker therefore satisfies libmpv2's
  references inside the binary and `--as-needed` drops `libmpv.so.2` from
  `DT_NEEDED`. `player::mpv_dl::load` runs first thing in `main`: it
  `dlopen`s `libmpv.so.2` (or `$CRAB_PLAYER_LIBMPV`) and checks for client
  API 2. On failure `main` prints per-distribution install advice,
  `player::create` picks the null backend and the app shows the same advice
  in its error banner, so a missing library never stops the window from
  opening. Calling an mpv function before loading, or one the installed
  libmpv lacks, aborts with a message rather than jumping to null.
- **No fonts, no crash.** eframe's system-font provider (fontique) panics
  on a background thread when fontconfig has no configuration or no fonts,
  and the release profile aborts on panic. `fonts::system_fonts_available`
  enables `system_font_fallback` only when both exist; otherwise egui's
  built-in fonts are used and a warning is logged.
- **One application id.** All targets install
  `io.github.mahziyarng98.crab-player.desktop`, the AppStream file
  `io.github.mahziyarng98.crab-player.metainfo.xml` and the hicolor icons
  from `assets/icons/`. The desktop file name, its `StartupWMClass`, the
  icon name, the Wayland app id, the X11 class, the Flatpak id and the
  AppStream id are all `io.github.mahziyarng98.crab-player` (`app::APP_ID`,
  checked by a unit test), which is what lets compositors and software
  centres show the right icon. The binary, the config folder and the data
  folder (including eframe's window state, pinned with `persistence_path`)
  keep the short name `crab-player`.

## 9. Roadmap

| Phase | Content |
| --- | --- |
| 1 (done) | Shell, commands, playlist, libmpv audio, config, packaging skeleton. |
| 2 (done) | mpv render API → egui FBO, letterbox/aspect, track selection, external subtitles, audio/subtitle delay, VA-API with display handle. |
| 3 | Options parity: equalizer, audio transform, shaders, subtitle style, sync. |
| 4 | Capture (PNG/JPEG, thumbnails, GIF), A-B repeat, chapters, bookmarks. |
| 5 | Network UX: URL dialog, yt-dlp integration (external binary, opt-in). |
| 6 (done) | Opening fixed on every path (non-blocking dialogs, Wayland drops, URI decoding), error banners, software retry, Media Info, codec matrix, caption strip with logo, centred transport, user-level installer. |
| 7 (done) | Release 0.1.0: one reverse-DNS app id, AppStream data, middle-click full screen, libmpv loaded at runtime, AppImage, .deb, .rpm, Arch, Flatpak and tarball packaging, CI and release workflows. |
| 7.1 (done) | Release 0.1.1: app id matching the repository URL, save dialog for images, narrower Flatpak sandbox, Flathub manifest prepared. |
| 7.2 (done) | Release 0.1.2: hand-drawn 16×16 and 24×24 px icons. |
| 8 | Accessibility pass, translations, Flathub submission and AUR. |
