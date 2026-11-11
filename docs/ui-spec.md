# UI specification

What Crab Player's interface contains and how it behaves. The feature
inventory is the one users expect from a classic desktop media player;
each table says what is implemented and what is planned.

Keyboard bindings are defaults; a shortcut editor is planned.

## 1. Layout

```
┌───────────────────────────────────────────────────────────────┐
│ 🦀 Crab Player  ☰          movie.mkv            📌  –  ▢  ✕   │  caption strip
├───────────────────────────────────────────────┬───────────────┤
│                                               │ PLAYLIST 3 + 🗑│
│                                               │───────────────│
│                 video canvas                  │ ▶ item        │
│              (letterboxed, black)             │ 2 item        │
│                                               │ 3 item        │
├───────────────────────────────────────────────┴───────────────┤
│━━━━━━━━━━━━━━━━━━●────────────────────────────────────────────│  seek bar (full width)
│ 0:42 / 1:58:07 ⏏      ⏮  (⏸)  ⏹  ⏭      ↻ ㏄ A-B 🔊▬▬▬ HW ☰ ⛶ │  control bar
└───────────────────────────────────────────────────────────────┘
```

Zones, in panel order (egui evaluates outermost first): top caption strip →
bottom seek bar + control bar → right playlist → central video. In full
screen only the video is a panel; caption and bars are drawn over it (see
§6), so the picture never moves when they appear or hide.

Screenshots: `docs/screenshots/`.

## 2. Window and caption

`src/ui/caption.rs`.

- The window has no decorations of its own (`with_decorations(false)`): the
  caption strip is the title bar. This looks the same on every compositor,
  including tiling ones such as Hyprland that draw no title bars at all.
- Caption, left to right: the logo (18 pt, crisp, see §15), the name
  "Crab Player", the menu button (☰, the full menu as one popup); the media
  title centred (truncated, never overlapping the buttons); at the right
  Always on top (pin), Minimize, Maximize/Restore, Close (red under the
  pointer). In full screen the window buttons become "Leave full screen".
- Dragging the strip moves the window (`ViewportCommand::StartDrag`); a
  double click maximizes or restores. Dragging the video moves the window
  too, as on classic players, so the window stays movable with the caption
  hidden.
- The window edges (5 pt) are resize handles with the matching cursors
  (`ViewportCommand::BeginResize`), except where the seek bar reaches the
  edge.
- View ▸ "Caption & Menu" (`Ctrl+M`) hides and shows the strip.
- App id `crab-player` (Wayland `app_id` and X11 class), matching
  `crab-player.desktop`; the window title is "Crab Player".
- Minimum size 640×400; default 1120×700.
- Full screen is a viewport command, so the compositor handles workspace
  and focus correctly.
- Always on top uses `ViewportCommand::WindowLevel`. Wayland has no protocol
  for it, so on native Wayland the pin button is shown disabled with a
  tooltip saying so.

## 3. Menus

Implemented in `src/ui/menu.rs`. The menu button in the caption opens one
popup with the classic groups as submenus (File, Play, View, Audio,
Subtitles, Navigate, Capture, Playlists) plus Options, Media Info, About and
Exit. The right-click menu on the video is the main interaction surface and
shares the same submenu builders. Items that are not functional yet are
shown disabled so the roadmap is visible in the application.
The "Planned inventory" column is the target feature set.

| Menu | Planned inventory | Implemented |
| --- | --- | --- |
| **File** | Quick Open, Open File, Open DVD/BD, Open Device, Open Directory, Open Disc, Recent Files, Close, Save a Copy, Save Image, Save Thumbnails, Subtitles ▸ (Load / Save / Download / Upload), Properties, Exit | Open File, Open Folder, Open URL… (`Ctrl+U`), Close, Save Image, Subtitles ▸ Load, Media Info, Exit |
| **Play** | Play/Pause, Stop, Frame Step, Repeat ▸ (Forever / File / Playlist), Decrease/Increase/Reset Rate, Filters, Shaders, Audio Track, Subtitle Track, Video Track, Volume ▸ (Up/Down/Mute), After Playback ▸ (Nothing / Next file / Monitor off / Exit / Stand By / Hibernate / Shutdown / Log Off / Lock), Copy URL | Play/Pause, Stop, Repeat ▸ (Off / File / Playlist, Shuffle), A-B Repeat ▸ (10 s from here, set A, set B, clear), rate up/down/reset, Volume ▸, After Playback ▸ (Do Nothing / Play Next File in Folder / Exit), Copy Location; Audio / Subtitle / Video Track ▸ with check marks |
| **View** | Caption&Menu, Seek Bar, Controls, MusicCTRL, CaptionBar, Subresync, Playlist, Capture, Navigation, Debug Shaders, Presets ▸ (Minimal / Compact / Normal), Full Screen, Zoom ▸ (50/100/200 %, Auto Fit, Auto Fit larger only), Renderer Settings ▸ (tearing test, statistics, current time, file name, output range, presentation, colour management, VSync, GPU control, reset), Video Frame ▸ (half/normal/double, stretch, touch inside/outside, zoom 1/2, aspect ratio), Pan&Scan ▸ (size/width/height ±, move, centre, reset), On Top ▸ (Default / Always / While playing / While playing video), Options | Caption & Menu, Seek Bar, Controls, Playlist toggles, Presets ▸, Full Screen, Zoom ▸ 50/100/200 %, Renderer Settings ▸ (statistics, current time, file name, hardware decoding), Video Frame ▸ Aspect Ratio ▸, Rotate ▸, Picture ▸ (brightness / contrast / saturation / hue), On Top ▸ (Default / Always), Options |
| **Audio** | Output Speakers ▸ (Auto, 1.0 … 7.1), Internal Audio Renderer, Use Equalizer, Presets, audio track selection | Volume, mute, Audio Track ▸ (real tracks, check mark on the active one, Next Track), Audio Delay ▸ (±100 ms, reset, current value) |
| **Subtitles** | Load, Save, Download, Upload, Browse/Edit/Sync, subtitle track, default style | Load Subtitles… (external file, selected on load), Subtitle Track ▸ (Off + real tracks, Next Track), Subtitle Delay ▸ (±100 ms, reset); Save/Download/Upload/Sync/Style placeholders |
| **Navigate** | Previous, Next, Go To…, Title/Root/Subtitle/Audio/Angle/Chapter menu, Favorites | Previous, Next, Go To… (seconds, `m:ss` or `h:mm:ss`) |
| **Capture** | Save Image, Save Thumbnails, Advanced Capture, Save to Clipboard, GIF | Save Image (`Ctrl+Shift+S`, PNG at the video's resolution, with subtitles, into `~/Pictures/crab-player/`); the rest placeholders |
| **Playlists** | Playlist Editor, Add File(s), Add Folder, Clear Playlist, Sort ▸ (name/folder asc/desc, random) | In the right-click menu: all of them; names sort naturally (`ep2` before `ep10`) and the playing item stays selected |
| **Options** | Options…, Keyboard Shortcuts… | Options… (subset) |
| **Help** | Home Page, Contact, About, Crash reporter | About |

Menu behaviour rules:

- Menu closes on selection; checkable and radio items (panel toggles,
  tracks, Repeat, After Playback, On Top) and delay steps toggle in place and
  do not close the menu. egui closes a menu on any inner click by default,
  so the bar uses `CloseOnClickOutside` and plain actions close explicitly.
- Track submenus list what the backend reports (`#id  title  [lang]  codec`,
  "(external)" for loaded files), refreshed on every file load and on
  mpv's `track-list` change notification. Audio and video always keep one
  track active; subtitles also offer "Off".
- A shortcut hint is right-aligned.
- The menu only emits `Command` values; it never mutates state itself, so a
  menu click, a key press and a button all take the same path through
  `PlayerApp::run`.
- Menu access is keyboard-reachable (Alt opens, arrows navigate) once egui's
  accesskit integration is enabled.
- The caption (and with it the menu button) hides in full screen when the
  chrome is auto-hidden.

## 4. Video surface

`src/ui/video.rs`.

- Fills all space not used by panels; black background, no border.
- Media is letterboxed, never stretched, unless the user picks a Video Frame
  mode (Phase 3).
- **Single click**: play/pause. **Double click**: toggle full screen.
  **Wheel**: volume ±2 % per notch (matches common player behaviour).
- **Right click**: the context menu: Open File, Open ▸, Play ▸, View ▸, Video ▸,
  Audio ▸, Subtitles ▸, the three track lists, Navigate ▸, Capture ▸,
  Playlists ▸, Options…, Media Info, Exit. It shares its submenus with the
  main menu, keeps check items open, and the pointer never auto-hides while
  it is open.
- **Overlays** (Renderer Settings): current time `position / duration` top
  right, file name top centre, in monospace on a translucent plate.
- **Drag the picture**: moves the window (not in full screen).
- **Drag and drop**: files, folders and URLs dropped on the window are
  appended to the playlist and the first new item plays. While a drag with
  files is over the window the canvas shows a "Drop to play" frame. On X11
  winit delivers the drop; on native Wayland winit does not, so
  `src/wayland_dnd.rs` binds its own `wl_data_device` on the window's
  connection and reads the `text/uri-list` (see architecture §5).
- **Errors** (`src/ui/notice.rs`): a file that cannot be opened or played
  shows a banner at the top of the canvas naming the reason (file not
  found, permission denied, no media in the folder, unrecognised format,
  not supported by the installed FFmpeg, decoder error, …) and the file. It
  never blocks input, fades after 12 s and has a close button; several
  failures at once are counted ("and 2 more problems").
- Cursor hides while playing with no OSD visible; any pointer movement brings
  chrome back.
- Welcome state (nothing open): the logo (up to 128 pt), "Crab Player",
  "Drop a video here or press Ctrl+O" and a hint line; everything scales
  down in a small window. The surface stays a drop target for the whole
  window.
- Statistics overlay (View ▸ Display Statistics): translucent top-left panel
  with state, backend, position, duration, resolution, track counts, volume
  and speed, in monospace 12 px.

## 5. Seek bar

`src/ui/seek_bar.rs` — custom painted, not a stock slider.

- Full window width, flush on top of the control bar, no side margins.
- 14 pt hit area. The track is 3 pt at rest and grows smoothly (120 ms) to
  6 pt under the pointer, dark violet (`#2A1F48`) with a violet played part.
  The round thumb grows with it: pale at rest, white under the pointer
  (with a neon halo), accent while dragged; it stays whole at both ends.
- An active A-B section is shaded on the track, with A and B ticks.
- Hover shows a vertical guide and a time bubble above the bar, without the
  usual tooltip delay; the bubble reads `position / duration` when dragging.
- The bar never calls the player. It reports a gesture: a click or the end
  of a drag becomes `Command::SeekTo` (exact seek); a drag in progress
  becomes `Command::Scrub`. The dispatcher puts `ScrubThrottle` in front of
  the backend: at most one scrub every 50 ms reaches mpv, a target that
  arrives too soon replaces the one being held back, and a resting pointer
  sends nothing. Scrubs are keyframe seeks so the picture can keep up, and
  the release always ends with one exact seek.
- Without a duration (idle, live streams) the empty track keeps its place,
  so nothing jumps when a file opens.
- Keyboard: ←/→ = 5 s, Shift+←/→ = 60 s. The OSD echoes the new time.
- Planned: chapter ticks, buffered ranges, thumbnail preview on hover.

## 6. Control bar

`src/ui/control_bar.rs`, icons in `src/ui/icons.rs`.

Icons are drawn with painter primitives in one thin outline style. Every
button has four states: lavender-grey at rest, near-white on a fill that
fades in (120 ms) under the pointer, a darker fill while pressed, dimmed
when disabled. Toggles that are on are drawn in the accent colour.

Layout, computed from the bar width alone (`plan`, unit-tested):

- **Left**: time (`m:ss / h:mm:ss`, monospace; a click switches total and
  remaining), the Open button, and the speed badge when the speed is not
  1.00×.
- **Centre**, always centred in the window: Previous, **Play/Pause** (a
  38 pt filled circle, the largest control, accent under the pointer),
  Stop, Next.
- **Right**, from the edge inwards: Full screen, Playlist (accent while the
  panel is open), `HW`/`SW` badge, volume slider, Mute.
- **Optional toggles**, left of Mute: Repeat, Subtitles, `A-B`, Shuffle,
  Save image. When the window narrows they drop out from the end of that
  list; only once they are all gone does the volume slider shrink (to
  44 pt) and finally disappear. The transport group and the fixed right
  buttons always fit at the minimum window width.

| Control | Behaviour |
| --- | --- |
| Previous / Next | Playlist previous / next item. |
| Play / Pause | Single toggle; filled glyph reflects state. Starts the selected playlist item when idle. |
| Stop | Stops playback, keeps the item selected. |
| Volume icon | Click mutes/unmutes; muted shows a red cross. |
| Volume slider | 0–100, up to 96 pt, custom painted (thin track, round thumb that grows under the pointer); click or drag; tooltip shows the percentage while dragging. Wheel over the bar adjusts by 2 %. |
| `HW`/`SW` badge | Accent `HW` while a hardware decoder runs, `SW` otherwise; the tooltip names the decoder. It shows what mpv reports (`hwdec-current`), including after an automatic software retry. A click switches hardware decoding on or off. Locked to `SW` on software OpenGL. |
| `A-B` badge | Click sets A, then B, then clears (like `L`). |
| Repeat | Cycles off → playlist → this file; a "1" for this file. |
| Shuffle | Random playback order. |
| Subtitles | Hides the shown subtitles, or shows the next track; disabled without tracks. |
| Playlist | Toggles the right panel (also F9). |
| Full screen | Toggles full screen (also F/Enter, double click; Esc leaves). |

Full screen: the caption and the bars are drawn over the video on a 90 %
opaque plate and fade (200 ms). They hide after `autohide_ms` (default
2 500 ms) without pointer, key or wheel input **while playing**; any input
brings them back. They never hide when paused, while loading, or while a
window or menu is open. Clicks on the bars never reach the video below.

## 7. Playlist panel

`src/ui/playlist_panel.rs`.

- Header: "PLAYLIST" with the item count in the accent colour, and two icon
  buttons: add files (+) and clear (bin, disabled when empty).
- Rows (26 pt): number column, title truncated to the width, full path in
  the tooltip. The current item has an accent bar, a tinted plate and a
  play mark instead of its number while playing. Rows fill softly under the
  pointer.
- Single click selects, double click plays, right click offers Play / Remove.
- Empty state explains drop and Ctrl+O.
- Width: 280 pt by default, resizable between 180 pt and 40 % of the
  window, so the video keeps most of a small window.
- Folder loads recurse (without following symlinks) and keep media files
  only; a file opened explicitly is never filtered by its extension.
- Planned: drag to reorder, multi-select, save/load `.m3u`, keyboard
  navigation.

## 8. Presets

| Preset | Caption | Playlist | Control bar | Seek bar |
| --- | --- | --- | --- | --- |
| Minimal | hidden | hidden | visible | hidden |
| Compact | visible | hidden | visible | visible |
| Normal | visible | shown | visible | visible |

Presets are layout shortcuts, not separate windows; they map onto the same
panel visibility flags (`UiState::apply_preset`). Keys `1`/`2`/`3` select
them, so Minimal (no caption) always has a way back.

The layout persists: caption, seek bar, controls, playlist and the
total/remaining time display are saved in the `[ui]` table of
`config.toml` on exit and restored at startup. The keys were added with
`#[serde(default)]`, so config files written by earlier versions still load
(missing keys take their defaults, unknown keys are ignored). When the menu
strip starts hidden, the OSD says how to bring it back.

## 9. OSD

- Bottom-centre, 56 px above the control bar, rounded translucent black plate.
- Triggered by: volume, mute, speed, seek, track change, file loaded, errors.
- Lives 1.6 s; a new message replaces the old one and restarts the timer.
- Never blocks clicks; painted into the video canvas below chrome.
- Phase 3: configurable position/size, "display current time" and "show file
  name" overlay options.

## 10. Panels and dialogs

Planned panels and dialogs:

| Panel / dialog | Purpose | Phase 1 |
| --- | --- | --- |
| Playlist | Queue management | yes |
| MusicCTRL | Compact audio-only bar with visualisation | no |
| Capture | Snapshot, thumbnails, GIF, clipboard | no |
| Navigation | DVD/BD title/chapter menus, favourites | no |
| Subresync | Subtitle timing editor | no |
| Debug Shaders | Shader compile/test UI | no |
| Open URL | URL + subtitle URL + "add without opening" | URL or path, any scheme mpv accepts; errors keep the dialog open |
| Go To | Timecode or frame number | no |
| Media Info (was Properties) | File/media technical summary | yes: title, file, state, duration, track counts, and from mpv: container, file size, video codec, profile, pixel format, resolution, frame rate, bitrate, the decoder in use (hardware name or software), audio codec, profile, channels, sample rate, bitrate; refreshed every second while open |
| About | Version, build, license | yes |
| Warning | Detailed playback failure | error banner with the reason (§4) |
| Download / Upload subtitles | Provider list, progress | no |
| Pan&Scan presets | Named crop/zoom presets | no |
| Favourites | Add/organise shortcuts | no |
| Advanced capture | Timed/batch capture | no |
| Tuner scan | BDA/DVB channel scan | out of scope |
| Update checker | Version check | no (package manager owns updates) |
| Crash reporter | Minidump upload | no (use systemd-coredump) |

## 11. Options pages

Options pages, in the order they are planned.

| Page | Contents | Status |
| --- | --- | --- |
| Playback | hwdec mode, audio device, deinterlace | **Phase 1** |
| Player | language, single instance, autoplay, resume | partial |
| Player ▸ Keys | shortcut editor | Phase 3 |
| Player ▸ Formats | file associations | package-level |
| Player ▸ Logo | logo overlay | Phase 4 |
| Player ▸ Web Interface | remote control HTTP server | Phase 5 (opt-in) |
| Subtitles / Default Style | font, size, colour, outline, position | Phase 2 |
| Subtitles ▸ Download | providers, languages | Phase 5 |
| Internal Filters ▸ Audio Equalizer | 10-band EQ | Phase 3 |
| Internal Filters ▸ Audio Transform | normalise, channel mapping | Phase 3 |
| Internal Filters ▸ Audio Renderer | device, exclusive mode, passthrough | Phase 3 (PipeWire) |
| Playback ▸ Output | renderer, colour range, HDR, VSync | Phase 2 |
| Playback ▸ Shaders | pre/post-resize shader lists | Phase 3 |
| Playback ▸ Fullscreen | monitor, mode, exclusive | Phase 3 |
| Playback ▸ Sync Renderer | frame timing, offsets | Phase 2 (mpv properties) |
| External Filters | filter graph editor | not planned; mpv `--vf/--af` |
| Miscellaneous / Tweaks / Advanced | cache, threads, logging | Phase 3 |
| Capture | output folder, format, naming | Phase 4 |
| MediaInfo | technical summary | Phase 4 |

## 12. Keyboard map

Implemented in `src/command.rs`; stored as `Command` values so a future
editor can rebind without touching call sites.

| Command | Binding |
| --- | --- |
| Play/Pause | Space, K |
| Stop | S |
| Previous / Next | PgUp / PgDn |
| Seek 5 s | ← / → |
| Seek 60 s | Shift+← / Shift+→ |
| Volume ± | ↑ / ↓ |
| Mute | M |
| Speed − / + | [ / ] |
| Speed reset | Backspace |
| Full screen | F, Enter |
| Leave full screen | Esc |
| Playlist | F9 |
| Caption & Menu / Seek Bar / Controls | Ctrl+M (also Ctrl+0) / Ctrl+1 / Ctrl+2 |
| Preset Minimal / Compact / Normal | 1 / 2 / 3 |
| Go To… | Ctrl+G |
| Open URL… | Ctrl+U |
| Save image | Ctrl+Shift+S |
| Rotate clockwise / counter-clockwise | R / Shift+R |
| Aspect ratio back to the file's | Shift+A |
| A-B repeat: set A → set B → off | L |
| Next audio / subtitle track | A / J (subtitles cycle through "off") |
| Subtitle delay −/+ 100 ms | Z / X |
| Audio delay −/+ 100 ms | Shift+Z / Shift+X |
| Open file / folder | Ctrl+O / Ctrl+D |
| Close media | Ctrl+W |
| Options | Ctrl+, |
| Media Info | Ctrl+P |
| Quit | Ctrl+Q |

Rules: shortcuts are suppressed while a text field has focus; they work in
full screen; they are not active while a menu popup is open (the popup owns
the keys).

Still to come: frame step (`.`/`,`), screenshot (`Ctrl+Shift+S`), cycle
aspect ratio, chapter skip.

## 13. Mouse, wheel and touch

| Input | Target | Action |
| --- | --- | --- |
| Left click | video | Play/pause |
| Double click | video | Full screen |
| Right click | video | Context menu (§4) |
| Wheel | video | Volume ±2 per notch |
| Wheel | control bar | Volume ±2 |
| Click / drag | seek bar | Seek |
| Double click | playlist row | Play |
| Drag | panel edge | Resize playlist |
| Touch tap | video | Play/pause (single), full screen (double) |
| Drag down | video | Dismiss (Phase 3, touch only) |

## 14. States

| State | Video area | Control bar | Notes |
| --- | --- | --- | --- |
| Idle | placeholder | enabled, play starts first item | default on launch |
| Opening | "Opening…" | enabled | pending load |
| Playing | picture | autohide allowed | 16 ms repaint |
| Paused | picture (last frame) | always visible | 4 Hz repaint |
| Stopped | black + title | enabled | selection kept |
| Ended | black + title | visible | auto-advances if a later item exists |
| Error | black + OSD reason | enabled | next/previous still work |

## 15. Visual language

All colours are tokens in `ui::theme::palette`; widgets never define their
own. The palette comes from the Crab Player logo.

| Token | Value | Use |
| --- | --- | --- |
| Canvas | `#000000` | behind the video |
| Well | `#0C0618` | text fields |
| Panel | `#100A1D` | playlist, scrollbar tracks |
| Chrome | `#150F24` | caption strip, control bar, menus, dialogs |
| Hover / Pressed | `#261B40` / `#3A2A61` | button and row fills |
| Separator | `#332752` | lines, outlines |
| Text / weak text | `#F5F3FF` / `#A9A3C2` | body and secondary text |
| Icon / disabled | `#9A93B8` / `#544A75` | glyphs |
| Accent | `#9146F5` (deep `#7C3AED`, light `#C084FC`) | progress, active toggles, selection |
| Neon | `#E879F9` | seek-thumb hover halo, selection outline; used sparingly |
| Track | `#2A1F48` | unplayed part of the seek bar |
| Danger | `#F87171` | errors, muted cross |

Contrast is enforced by unit tests: body and weak text are at least 4.5:1
on every surface they appear on (weak text on Pressed is the tightest),
icons and accents at least 3:1.

- Text: 12–13.5 pt UI, 15 pt OSD, 24 pt welcome heading, monospace for
  timecodes and statistics.
- 1 pt separators, 4–8 pt corner radii, no shadows, no gradients.
- Hover states are fills that fade in over 120 ms, not colour shifts, so
  the accent keeps its meaning. Seek-bar thickness and the full-screen
  chrome are animated too; nothing changes size in the layout, so nothing
  jumps.
- No icons are bundled as images: every icon is drawn with `egui::Painter`
  primitives (`src/ui/icons.rs`), so it scales and themes for free.
- The logo is the only image. `src/ui/logo.rs` never lets the GPU scale it:
  for each size it is shown at, a texture with exactly that many physical
  pixels is made (from the icon set when a size matches, otherwise a
  Lanczos downscale of the next larger one), snapped to the pixel grid and
  drawn 1:1, so it is sharp at 1.0, 1.25, 1.5 and 2.0. Its dark square
  corners are clipped to the rounded plate.

## 16. Accessibility and localisation

- Accesskit support (egui feature): names, roles, and focus order
  for all buttons and menu items (planned).
- Every control has a tooltip and a keyboard path today; contrast of the
  accent on near-black passes WCAG AA for large text, and body text uses
  `#C8CCD2` or lighter.
- No text baked into images or shaders.
- All user-facing strings will move to a single catalog before translations
  land; Phase 1 keeps them inline to avoid premature abstraction.
