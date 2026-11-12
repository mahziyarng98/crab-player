//! Application shell: owns the player, the playlist and all UI state.
//!
//! The layout is the classic desktop player shape: menu bar on top, video in the middle, transport bar at the
//! bottom, playlist docked on the right. Chrome hides itself in full screen.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::command::{
    self, AbStep, AfterPlayback, Command, DelayStep, PictureControl, RepeatMode, RotateStep,
};
use crate::config::AppConfig;
use crate::dialogs::{Dialogs, Purpose};
use crate::media::{self, OpenError, Source};
use crate::player::{self, PlaybackState, Player, PlayerEvent, PlayerSnapshot, TrackKind};
use crate::playlist::{Playlist, Shuffle};
use crate::ui::playlist_panel::PlaylistAction;
use crate::ui::seek_bar::{ScrubThrottle, SeekGesture};
use crate::ui::{self, UiState};

/// Product name, as shown to people.
pub const APP_NAME: &str = "Crab Player";
/// Name used for the binary, directories and the desktop entry.
pub const APP_SLUG: &str = "crab-player";
/// Reverse-DNS application id: the Wayland `app_id`, the X11 class, the
/// desktop file name (`<APP_ID>.desktop`), its `StartupWMClass`, the icon
/// name, the Flatpak id and the AppStream id. Compositors find the icon
/// through the desktop file of the same name, so all of them must stay
/// identical (see docs/packaging.md, "Application id").
pub const APP_ID: &str = "io.github.mahziyarng98.crab-player";
/// Application icon, embedded so the window needs no files.
pub const ICON_PNG: &[u8] = ui::logo::WINDOW_ICON_PNG;
/// Smallest window, in points (ui-spec §2). Zoom never goes below it.
pub const MIN_WINDOW_SIZE: [f32; 2] = [640.0, 400.0];

const SEEK_SMALL: Duration = Duration::from_secs(5);
const SEEK_LARGE: Duration = Duration::from_secs(60);
/// One step of an audio/subtitle delay adjustment, in seconds.
const DELAY_STEP: f64 = 0.1;

const SUBTITLE_EXTENSIONS: &[&str] = &[
    "srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi", "lrc",
];

pub struct PlayerApp {
    config: AppConfig,
    player: Box<dyn Player>,
    snapshot: PlayerSnapshot,
    playlist: Playlist,
    ui: UiState,
    /// Media passed on the command line, opened on the first frame.
    pending: Vec<String>,
    /// Subtitle files waiting for the backend to finish loading the media
    /// they belong to (a subtitle cannot be attached before that).
    pending_subtitles: Vec<PathBuf>,
    last_activity: Instant,
    /// Rate limiter between seek-bar drags and the backend.
    scrub: ScrubThrottle,
    /// Random source for shuffle playback and random sorting.
    rng: Shuffle,
    backend: &'static str,
    /// GL-side video compositor; `None` for audio-only/null backends.
    video: Option<ui::video::VideoSurface>,
    /// The logo: caption strip, welcome screen and About box.
    logo: ui::logo::Logo,
    /// Native file dialogs, running off the UI thread.
    dialogs: Dialogs,
    /// Frame captured to a temporary file while the save dialog is open.
    pending_screenshot: Option<PathBuf>,
    /// A screenshot mpv is still writing; the save dialog opens only once
    /// the file is complete, and not at all if the capture failed.
    pending_capture: Option<PendingCapture>,
    /// When the backend was last asked for the Media Info details.
    media_info_asked: Option<Instant>,
    /// File drops on native Wayland, which the windowing library does not
    /// deliver itself. `None` on X11, where it does.
    drop_target: Option<crate::wayland_dnd::WaylandDrop>,
}

/// A screenshot mpv is still writing.
struct PendingCapture {
    /// Where mpv writes the PNG.
    temp: PathBuf,
    /// Suggested file name for the save dialog.
    name: String,
    /// Give up and report failure after this point.
    deadline: Instant,
}

/// What adding one local path to the playlist led to.
enum Added {
    /// A playlist item at this index (new, or already present).
    Item(usize),
    /// A subtitle file: belongs to media, not to the playlist.
    Subtitle(PathBuf),
}

impl PlayerApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: AppConfig,
        config_warning: Option<String>,
        media: Vec<String>,
    ) -> Self {
        ui::install_theme(&cc.egui_ctx);

        crate::startup::mark("window and GL context ready");
        // The creator runs on the GL thread with the context current, which
        // is what the mpv render context needs.
        let (mut player, video) = player::create(
            &config.playback,
            player::VideoInit {
                get_proc_address: cc.get_proc_address.clone(),
                repaint: cc.egui_ctx.clone(),
                native_display: native_display(cc),
                gl_renderer: cc.gl.as_deref().map(ui::video::gl_renderer_name),
            },
        );
        crate::startup::mark("playback backend ready");
        let video = video.map(ui::video::VideoSurface::new);
        let backend = player.backend_name();
        // Apply persisted mixer state to the backend before the first frame.
        let _ = player.set_volume(config.volume);
        let _ = player.set_muted(config.muted);

        let snapshot = player::PlayerSnapshot {
            volume: config.volume,
            muted: config.muted,
            ..Default::default()
        };

        if config.ui.always_on_top {
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::AlwaysOnTop,
                ));
        }

        let drop_target = wayland_display(cc).map(|display| {
            // SAFETY: this is the display the window lives on, and the
            // receiver is dropped in `on_exit`, before the window goes away.
            unsafe { crate::wayland_dnd::WaylandDrop::new(display, cc.egui_ctx.clone()) }
        });

        let ui_state = UiState::new(&config.ui);
        let mut app = Self {
            config,
            player,
            video,
            logo: ui::logo::Logo::default(),
            snapshot,
            playlist: Playlist::default(),
            ui: ui_state,
            pending: media,
            pending_subtitles: Vec::new(),
            last_activity: Instant::now(),
            scrub: ScrubThrottle::default(),
            rng: Shuffle::from_time(),
            backend,
            dialogs: Dialogs::default(),
            pending_screenshot: None,
            pending_capture: None,
            media_info_asked: None,
            drop_target,
        };

        if !app.ui.show_menu_bar {
            // The layout is persisted; say how to get the menu back.
            app.ui.flash("Menu hidden · Ctrl+M brings it back");
        }
        if let Some(reason) = player::engine_unavailable() {
            let (title, detail) = if reason.contains("libmpv") {
                (
                    "Playback is unavailable: libmpv is not installed",
                    format!(
                        "Install libmpv2 (Debian, Ubuntu, openSUSE), mpv-libs (Fedora) or \
                         mpv (Arch), then restart Crab Player. ({reason})"
                    ),
                )
            } else {
                (
                    "Playback is unavailable: a library libmpv needs is missing",
                    format!("Install the missing library, then restart Crab Player. ({reason})"),
                )
            };
            app.ui.error(title, detail);
        }
        if let Some(warning) = config_warning {
            app.ui
                .error("Settings were invalid; using defaults", warning);
        }
        app.open_pending();
        app
    }

    // ---------------------------------------------------------------- media

    fn open_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let media = std::mem::take(&mut self.pending);
        self.open_entries(&media, true);
    }

    /// Open what the user handed over, whichever way it came in (command
    /// line, desktop entry, drop, dialog, Open URL): paths, `file:` URIs
    /// and stream URLs. New items are appended to the playlist and, with
    /// `play`, the first of them starts. Returns how many entries led to a
    /// playlist item; every entry that did not is reported to the user.
    fn open_entries(&mut self, entries: &[String], play: bool) -> usize {
        let base = std::env::current_dir().unwrap_or_default();
        let mut first = None;
        let mut opened = 0;
        let mut subtitles = Vec::new();

        for entry in entries {
            let added = match media::parse(entry, &base) {
                Ok(Source::Url(url)) => {
                    self.playlist.add_url(&url);
                    Ok(Added::Item(self.playlist.len() - 1))
                }
                Ok(Source::Path(path)) => self.add_local(&path),
                Err(err) => Err(err),
            };
            match added {
                Ok(Added::Item(index)) => {
                    opened += 1;
                    first.get_or_insert(index);
                }
                Ok(Added::Subtitle(path)) => subtitles.push(path),
                Err(err) => self.report_open_error(&err),
            }
        }

        let starting = play && first.is_some();
        for path in subtitles {
            if starting {
                // `crab-player movie.mkv movie.srt`: attached once the
                // media it came with has loaded.
                self.pending_subtitles.push(path);
            } else if self.snapshot.media_path.is_some() {
                self.load_subtitle(&path);
            } else {
                self.ui.error(
                    "Open a video first",
                    format!("{} is a subtitle file", display_name(&path)),
                );
            }
        }

        if let Some(index) = first.filter(|_| play) {
            self.play_index(index);
        }
        opened
    }

    /// Add one local file or folder to the playlist.
    fn add_local(&mut self, path: &Path) -> Result<Added, OpenError> {
        media::check_path(path)?;
        if path.is_dir() {
            let before = self.playlist.len();
            self.playlist.add_path(path);
            return if self.playlist.len() > before {
                Ok(Added::Item(before))
            } else {
                Err(OpenError::NoMediaInFolder(path.to_path_buf()))
            };
        }
        if is_subtitle(path) {
            return Ok(Added::Subtitle(path.to_path_buf()));
        }
        // A file named explicitly is never filtered by its extension: the
        // playback engine decides whether it can be played.
        Ok(Added::Item(self.playlist.index_of(path).unwrap_or_else(
            || {
                self.playlist.add_file(path);
                self.playlist.len() - 1
            },
        )))
    }

    fn report_open_error(&mut self, err: &OpenError) {
        tracing::warn!(%err, "could not open");
        self.ui.error(err.title(), err.subject());
    }

    /// The result of a native file dialog.
    fn dialog_finished(&mut self, picked: crate::dialogs::Picked) {
        if picked.purpose == Purpose::SaveImage {
            self.save_screenshot(picked.paths.first());
            return;
        }
        let entries: Vec<String> = picked
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        if entries.is_empty() {
            return; // cancelled
        }
        match picked.purpose {
            Purpose::OpenFile | Purpose::OpenFolder => {
                self.open_entries(&entries, true);
            }
            Purpose::AddFiles => {
                let added = self.open_entries(&entries, false);
                if added > 0 {
                    self.ui.flash(format!("Added {added} to the playlist"));
                }
            }
            Purpose::LoadSubtitle => {
                for path in &picked.paths {
                    self.load_subtitle(path);
                }
            }
            Purpose::SaveImage => unreachable!("handled above"),
        }
    }

    /// Finish Capture ▸ Save Image: copy the captured frame to the file the
    /// portal dialog returned, then drop the temporary copy.
    fn save_screenshot(&mut self, target: Option<&PathBuf>) {
        let Some(temp) = self.pending_screenshot.take() else {
            return;
        };
        let copied = target.map(|target| {
            std::fs::copy(&temp, target)
                .inspect_err(
                    |err| tracing::warn!(%err, path = %target.display(), "cannot save the image"),
                )
                .ok()
        });
        let _ = std::fs::remove_file(&temp);
        match (target, copied) {
            (Some(_), Some(_)) => {
                let name = target
                    .and_then(|path| path.file_name())
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                self.ui.flash(format!("Saved {name}"));
            }
            (Some(_), None) => self.ui.flash("Could not save the image"),
            (None, _) => self.ui.flash("Save cancelled"),
        }
    }

    fn open_dialog(&mut self, purpose: Purpose, ctx: &egui::Context) {
        if self.dialogs.is_open() {
            self.ui.flash("A file dialog is already open");
        } else if !self.dialogs.open(purpose, SUBTITLE_EXTENSIONS, None, ctx) {
            self.ui.error("Could not open the file dialog", "");
        }
    }

    fn play_index(&mut self, index: usize) {
        let Some(path) = self.playlist.get(index).map(|item| item.path.clone()) else {
            return;
        };
        self.playlist.set_selected(Some(index));

        match self.player.load(&path, !self.config.autoplay) {
            Ok(()) => {
                self.snapshot = self.player.snapshot();
                self.snapshot.media_path = Some(path.clone());
            }
            Err(err) => {
                tracing::error!(%err, path = %path.display(), "failed to load media");
                self.ui
                    .error("Could not open the file", display_name(&path));
            }
        }
    }

    fn play_selected_or_first(&mut self) {
        let index = self
            .playlist
            .selected()
            .filter(|index| *index < self.playlist.len())
            .or_else(|| (!self.playlist.is_empty()).then_some(0));

        match index {
            Some(index) => self.play_index(index),
            None => self.ui.flash("Playlist is empty"),
        }
    }

    fn refresh(&mut self) {
        self.snapshot = self.player.snapshot();
        // While the seek bar is being dragged the handle follows the
        // pointer; the backend position lags behind the throttled seeks.
        if let Some(target) = self.scrub.target() {
            self.snapshot.position = target;
        }
    }

    /// Release a drag target that the throttle was holding back, and make
    /// sure a frame is scheduled for the moment the next one is due.
    fn pump_scrub(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(target) = self.scrub.poll(now) {
            let _ = self.player.scrub(target);
        }
        if let Some(wait) = self.scrub.wake_in(now) {
            ctx.request_repaint_after(wait);
        }
    }

    // ------------------------------------------------------------ lifecycle

    fn tick(&mut self, ctx: &egui::Context) {
        self.refresh();
        self.pump_scrub(ctx);
        self.poll_capture(ctx);
        if let Some(picked) = self.dialogs.poll() {
            self.dialog_finished(picked);
        }

        for event in self.player.drain_events() {
            match event {
                PlayerEvent::EndOfFile => self.advance_after_eof(ctx),
                PlayerEvent::Error(message) => {
                    let subject = self
                        .snapshot
                        .media_path
                        .as_deref()
                        .map(display_name)
                        .unwrap_or_default();
                    tracing::warn!(%message, %subject, "playback error");
                    self.ui.error(message, subject);
                }
                PlayerEvent::FileLoaded { duration } => {
                    if let Some(duration) = duration {
                        self.snapshot.duration = Some(duration);
                    }
                    for path in std::mem::take(&mut self.pending_subtitles) {
                        self.load_subtitle(&path);
                    }
                }
            }
        }

        let busy = matches!(
            self.snapshot.state,
            PlaybackState::Playing | PlaybackState::Opening
        );
        // With a picture on screen, mpv's render update callback requests a
        // repaint for every new frame, so no fixed-rate timer is needed.
        // That only holds once a picture is on screen: until then (startup,
        // a new file) the timer keeps running, otherwise the first frame
        // can sit unrendered until some unrelated event causes a repaint.
        let frame_driven = self
            .video
            .as_ref()
            .is_some_and(ui::video::VideoSurface::has_picture);
        if busy && !frame_driven {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.ui.osd.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// The current item finished: apply Repeat, then the playlist order,
    /// then After Playback once nothing is left.
    fn advance_after_eof(&mut self, ctx: &egui::Context) {
        let Some(current) = self.playlist.selected() else {
            return;
        };
        let last = current + 1 >= self.playlist.len();

        match self.ui.repeat {
            RepeatMode::File => self.play_index(current),
            // Shuffle keeps drawing random items; it has no "end".
            _ if self.ui.shuffle => {
                if let Some(index) = self.playlist.random_index(&mut self.rng) {
                    self.play_index(index);
                }
            }
            RepeatMode::Playlist if last => self.play_index(0),
            _ if !last => self.play_index(current + 1),
            _ => match self.ui.after_playback {
                AfterPlayback::Nothing => self.ui.flash("End of playlist"),
                AfterPlayback::NextInFolder => self.play_next_in_folder(),
                AfterPlayback::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            },
        }
    }

    /// Track user activity and decide whether the chrome may auto-hide.
    ///
    /// Chrome hides only in full screen, while playing, after `autohide_ms`
    /// without pointer, key or wheel input. It never hides while paused or
    /// loading, while a dialog is open, or while a menu popup is open.
    fn update_activity(&mut self, ctx: &egui::Context) {
        let active = ctx.input(|i| {
            i.pointer.delta() != egui::Vec2::ZERO
                || i.pointer.any_click()
                || i.pointer.any_pressed()
                || i.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key { .. }
                            | egui::Event::Text(_)
                            | egui::Event::MouseWheel { .. }
                            | egui::Event::Touch { .. }
                    )
                })
        });
        if active {
            self.last_activity = Instant::now();
        }

        let may_hide = self.ui.fullscreen
            && self.config.ui.controls_autohide
            && self.snapshot.state == PlaybackState::Playing
            && !self.ui.any_window_open()
            && !egui::Popup::is_any_open(ctx);
        let timeout = Duration::from_millis(self.config.ui.autohide_ms);
        let idle = self.last_activity.elapsed();

        let visible = !(may_hide && idle >= timeout);
        if visible != self.ui.controls_visible {
            tracing::debug!(visible, "full-screen chrome visibility changed");
        }
        self.ui.controls_visible = visible;
        if may_hide && idle < timeout {
            // Wake up exactly when the chrome is due to disappear; video
            // frames are not guaranteed to arrive at that moment.
            ctx.request_repaint_after(timeout - idle);
        }
    }

    /// The wheel button toggles full screen, through the same command as F,
    /// Enter and a double click. Runs before anything is drawn, so menus,
    /// focus and drags are as they were when the button was clicked.
    fn handle_middle_click(&mut self, ctx: &egui::Context) {
        let (clicked, pos, other_button_down) = ctx.input(|i| {
            (
                i.pointer.button_clicked(egui::PointerButton::Middle),
                i.pointer.interact_pos(),
                i.pointer.primary_down() || i.pointer.secondary_down(),
            )
        });
        if !clicked {
            return;
        }
        let click = command::MiddleClick {
            clicked,
            // Panels, the video and the full-screen chrome all live on the
            // background layer; dialogs, menus and tooltips float above it.
            over_main_surface: pos
                .and_then(|pos| ctx.layer_id_at(pos))
                .is_none_or(|layer| layer.order == egui::Order::Background),
            popup_open: egui::Popup::is_any_open(ctx),
            editing_text: ctx.egui_wants_keyboard_input()
                || self.ui.show_goto
                || self.ui.show_open_url,
            // A held left or right button means a drag (seek bar, volume
            // slider, panel edge, window move) is under way, even before
            // egui has decided it is one.
            dragging: ctx.dragged_id().is_some() || other_button_down,
        };
        if command::middle_click_toggles_fullscreen(click) {
            self.run(Command::Fullscreen, ctx);
        } else {
            tracing::debug!(?click, "middle click ignored");
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Never steal keys from a focused text field (e.g. the options page).
        if ctx.egui_wants_keyboard_input() {
            return;
        }

        let mut commands = Vec::new();
        ctx.input(|i| {
            for event in &i.events {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                else {
                    continue;
                };

                if *key == egui::Key::Escape {
                    // Escape only leaves full screen; it does not enter it.
                    if self.ui.fullscreen {
                        commands.push(Command::Fullscreen);
                    }
                    continue;
                }

                if let Some(command) = command::from_key(*key, *modifiers) {
                    commands.push(command);
                }
            }
        });

        for command in commands {
            self.run(command, ctx);
        }
    }

    // ------------------------------------------------------------- commands

    fn run(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::OpenFile => self.open_dialog(Purpose::OpenFile, ctx),
            Command::OpenFolder => self.open_dialog(Purpose::OpenFolder, ctx),
            Command::AddFiles => self.open_dialog(Purpose::AddFiles, ctx),
            Command::ClearPlaylist => {
                self.playlist.clear();
                let _ = self.player.stop();
                self.refresh();
            }
            Command::CopyLocation => {
                if let Some(path) = &self.snapshot.media_path {
                    ctx.copy_text(path.display().to_string());
                    self.ui.flash("Location copied");
                }
            }
            Command::CloseMedia => {
                if self.player.stop().is_ok() {
                    self.playlist.set_selected(None);
                    self.refresh();
                    self.ui.flash("Closed");
                }
            }
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Command::PlayPause => match self.snapshot.state {
                PlaybackState::Idle | PlaybackState::Stopped | PlaybackState::Ended => {
                    self.play_selected_or_first();
                }
                _ => {
                    let _ = self.player.toggle_pause();
                    self.refresh();
                }
            },
            Command::Stop => {
                let _ = self.player.stop();
                self.refresh();
            }
            Command::Next => {
                let index = if self.ui.shuffle {
                    self.playlist.random_index(&mut self.rng)
                } else {
                    self.playlist.next_index()
                };
                if let Some(index) = index {
                    self.play_index(index);
                }
            }
            Command::Previous => match self.playlist.selected() {
                // At the start of the playlist, Previous restarts the item
                // instead of wrapping around to the end.
                Some(0) => self.play_index(0),
                _ => {
                    if let Some(index) = self.playlist.previous_index() {
                        self.play_index(index);
                    }
                }
            },
            Command::SeekForward => self.seek_by(SEEK_SMALL.as_secs() as i64),
            Command::SeekBackward => self.seek_by(-(SEEK_SMALL.as_secs() as i64)),
            Command::SeekForwardLarge => self.seek_by(SEEK_LARGE.as_secs() as i64),
            Command::SeekBackwardLarge => self.seek_by(-(SEEK_LARGE.as_secs() as i64)),
            Command::SeekTo(target) => self.seek_to(target),
            Command::Scrub(target) => {
                let Some(duration) = self.snapshot.duration else {
                    return;
                };
                let target = target.min(duration);
                if let Some(target) = self.scrub.scrub(target, Instant::now()) {
                    let _ = self.player.scrub(target);
                }
                self.snapshot.position = target;
            }
            Command::SetVolume(volume) => self.set_volume(volume),
            Command::VolumeUp => self.change_volume(5.0),
            Command::VolumeDown => self.change_volume(-5.0),
            Command::Mute => {
                let muted = !self.snapshot.muted;
                let _ = self.player.set_muted(muted);
                self.snapshot.muted = muted;
                self.config.muted = muted;
                self.ui.flash(if muted { "Muted" } else { "Unmuted" });
            }
            Command::SpeedUp => self.change_speed(1.25),
            Command::SpeedDown => self.change_speed(0.8),
            Command::SpeedReset => self.set_speed(1.0),
            Command::Fullscreen => {
                self.ui.fullscreen = !self.ui.fullscreen;
                tracing::debug!(fullscreen = self.ui.fullscreen, "fullscreen toggled");
                self.last_activity = Instant::now();
                self.ui.controls_visible = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.ui.fullscreen));
            }
            Command::GoTo => {
                if self.snapshot.duration.is_some() {
                    self.ui.goto_text = player::format_time(self.snapshot.position);
                    self.ui.goto_focus = true;
                    self.ui.show_goto = true;
                }
            }
            Command::SetRepeat(mode) => {
                self.ui.repeat = mode;
                self.ui.flash(match mode {
                    RepeatMode::Off => "Repeat off",
                    RepeatMode::File => "Repeat file",
                    RepeatMode::Playlist => "Repeat playlist",
                });
            }
            Command::SetAfterPlayback(action) => self.ui.after_playback = action,
            Command::ToggleMenuBar => {
                self.ui.show_menu_bar = !self.ui.show_menu_bar;
                if !self.ui.show_menu_bar {
                    self.ui.flash("Menu hidden · Ctrl+M brings it back");
                }
            }
            Command::ToggleSeekBar => self.ui.show_seek_bar = !self.ui.show_seek_bar,
            Command::ToggleControls => self.ui.show_controls = !self.ui.show_controls,
            Command::TogglePlaylist => {
                self.ui.show_playlist = !self.ui.show_playlist;
                self.config.ui.show_playlist = self.ui.show_playlist;
            }
            Command::ToggleStatistics => self.ui.show_statistics = !self.ui.show_statistics,
            Command::ToggleTimeDisplay => self.ui.show_remaining = !self.ui.show_remaining,
            Command::ToggleOnTop => {
                let on_top = !self.config.ui.always_on_top;
                self.config.ui.always_on_top = on_top;
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
                self.ui.flash(if on_top {
                    "Always on top"
                } else {
                    "On top: default"
                });
            }
            Command::ApplyPreset(preset) => {
                self.ui.apply_preset(preset);
                self.config.ui.show_playlist = self.ui.show_playlist;
                self.ui.flash(format!("Preset: {preset:?}"));
            }
            Command::SelectTrack(kind, id) => self.select_track(kind, id),
            Command::CycleTrack(kind) => {
                let tracks = self.tracks(kind);
                if tracks.is_empty() {
                    self.ui
                        .flash(format!("No {} tracks", kind.label().to_lowercase()));
                } else {
                    // Only subtitles have an "off" position in the cycle.
                    let next = player::next_track(tracks, kind == TrackKind::Subtitle);
                    self.select_track(kind, next);
                }
            }
            Command::LoadSubtitle => {
                if self.snapshot.media_path.is_none() {
                    return;
                }
                self.open_dialog(Purpose::LoadSubtitle, ctx);
            }
            Command::AdjustDelay(kind, step) => self.adjust_delay(kind, step),
            Command::OpenUrl => {
                self.ui.url_focus = true;
                self.ui.show_open_url = true;
            }
            Command::SetAspect(ratio) => {
                if self.player.set_aspect_override(ratio).is_ok() {
                    self.refresh();
                    self.ui
                        .flash(format!("Aspect ratio: {}", aspect_label(ratio)));
                }
            }
            Command::Rotate(step) => {
                let degrees = match step {
                    RotateStep::Clockwise => i32::from(self.snapshot.rotation) + 90,
                    RotateStep::CounterClockwise => i32::from(self.snapshot.rotation) - 90,
                    RotateStep::Reset => 0,
                };
                if self.player.set_rotation(degrees).is_ok() {
                    self.refresh();
                    self.ui
                        .flash(format!("Rotation {}°", self.snapshot.rotation));
                }
            }
            Command::AbRepeat(step) => {
                let Some(duration) = self.snapshot.duration else {
                    return;
                };
                let ab = next_ab(
                    self.snapshot.ab_loop,
                    step,
                    self.snapshot.position,
                    duration,
                );
                if self.player.set_ab_loop(ab).is_ok() {
                    self.refresh();
                    self.ui.flash(describe_ab(ab));
                }
            }
            Command::AdjustPicture(control, delta) => {
                let mut picture = self.snapshot.picture;
                let value = match control {
                    PictureControl::Brightness => &mut picture.brightness,
                    PictureControl::Contrast => &mut picture.contrast,
                    PictureControl::Saturation => &mut picture.saturation,
                    PictureControl::Hue => &mut picture.hue,
                };
                *value += delta;
                if self.player.set_picture(picture.clamped()).is_ok() {
                    self.refresh();
                    let applied = match control {
                        PictureControl::Brightness => self.snapshot.picture.brightness,
                        PictureControl::Contrast => self.snapshot.picture.contrast,
                        PictureControl::Saturation => self.snapshot.picture.saturation,
                        PictureControl::Hue => self.snapshot.picture.hue,
                    };
                    self.ui.flash(format!("{} {applied:+}", control.label()));
                }
            }
            Command::ResetPicture => {
                if self.player.set_picture(player::Picture::default()).is_ok() {
                    self.refresh();
                    self.ui.flash("Picture reset");
                }
            }
            Command::Screenshot => self.screenshot(ctx),
            Command::SortPlaylist(order) => {
                self.playlist.sort(order, &mut self.rng);
                self.ui.flash("Playlist sorted");
            }
            Command::ToggleShuffle => {
                self.ui.shuffle = !self.ui.shuffle;
                self.ui.flash(if self.ui.shuffle {
                    "Shuffle on"
                } else {
                    "Shuffle off"
                });
            }
            Command::ZoomWindow(percent) => self.zoom_window(ctx, percent),
            Command::ToggleHardwareDecoding => {
                let enable = !self.snapshot.hardware_decoding;
                match self.player.set_hardware_decoding(enable) {
                    Ok(()) => {
                        self.refresh();
                        self.ui.flash(if enable {
                            "Hardware decoding on"
                        } else {
                            "Hardware decoding off"
                        });
                    }
                    Err(err) => {
                        tracing::info!(%err, "hardware decoding toggle refused");
                        self.ui.flash("Hardware decoding is not available here");
                    }
                }
            }
            Command::ToggleClockOverlay => self.ui.show_clock = !self.ui.show_clock,
            Command::ToggleFileNameOverlay => {
                self.ui.show_file_name = !self.ui.show_file_name;
            }
            Command::Options => self.ui.show_options = true,
            Command::Properties => self.ui.show_properties = true,
            Command::About => self.ui.show_about = true,
        }
    }

    /// After Playback ▸ Play next file in the folder.
    fn play_next_in_folder(&mut self) {
        let Some(current) = self.snapshot.media_path.clone() else {
            return;
        };
        let siblings: Vec<PathBuf> = current
            .parent()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .map(|entries| entries.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        match crate::playlist::next_in_folder(&current, &siblings) {
            Some(next) => {
                self.open_entries(&[next.to_string_lossy().into_owned()], true);
            }
            None => self.ui.flash("No more files in this folder"),
        }
    }

    /// Capture ▸ Save Image: capture the current frame to a temporary file
    /// and let the user choose where to keep it with the save dialog.
    fn screenshot(&mut self, ctx: &egui::Context) {
        if self.snapshot.video_size.is_none() {
            self.ui.flash("Nothing to capture");
            return;
        }
        if self.dialogs.is_open() {
            self.ui.flash("A file dialog is already open");
            return;
        }
        if self.pending_capture.is_some() {
            self.ui.flash("Still capturing the image");
            return;
        }
        // The frame is written now, not when the dialog is confirmed, so the
        // saved image is the one that was on screen when Save Image was
        // chosen. The sandbox's private temp directory always accepts it;
        // the portal save dialog returns a writable path for the real file.
        // mpv runs the screenshot command asynchronously, so the dialog is
        // deferred until the file is complete (see `poll_capture`).
        let temp =
            std::env::temp_dir().join(format!("crab-player-frame-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&temp);
        if let Err(err) = self.player.screenshot(&temp) {
            tracing::warn!(%err, "screenshot failed");
            self.ui.flash("Could not capture the image");
            return;
        }
        let title = self.snapshot.title.as_deref().unwrap_or("frame");
        self.pending_capture = Some(PendingCapture {
            temp,
            name: screenshot_name(title, self.snapshot.position),
            // Generous for a 4K PNG with subtitles; only reached when the
            // capture never completes.
            deadline: Instant::now() + Duration::from_secs(3),
        });
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    /// Open the save dialog once the requested screenshot has been written.
    /// A failed capture (for example a frame the GPU could not read back)
    /// leaves no complete PNG, and then no dialog is shown at all.
    fn poll_capture(&mut self, ctx: &egui::Context) {
        let Some(capture) = &self.pending_capture else {
            return;
        };
        if !capture_complete(&capture.temp) {
            if Instant::now() < capture.deadline {
                ctx.request_repaint_after(Duration::from_millis(30));
                return;
            }
            let Some(capture) = self.pending_capture.take() else {
                return;
            };
            tracing::warn!(path = %capture.temp.display(), "screenshot was not written");
            let _ = std::fs::remove_file(&capture.temp);
            self.ui.flash("Could not capture the image");
            return;
        }
        let Some(capture) = self.pending_capture.take() else {
            return;
        };
        if !self.dialogs.open(
            Purpose::SaveImage,
            SUBTITLE_EXTENSIONS,
            Some(&capture.name),
            ctx,
        ) {
            let _ = std::fs::remove_file(&capture.temp);
            self.ui.error("Could not open the save dialog", "");
            return;
        }
        self.pending_screenshot = Some(capture.temp);
    }

    /// View ▸ Zoom: size the window so the picture shows at `percent` of its
    /// own pixel size, keeping all chrome where it is.
    fn zoom_window(&mut self, ctx: &egui::Context, percent: u16) {
        if self.ui.fullscreen {
            return;
        }
        let video = self
            .video
            .as_ref()
            .and_then(|surface| surface.display_size());
        let window = ctx.input(|i| i.viewport().inner_rect.map(|rect| rect.size()));
        let (Some(video), Some(window), Some(canvas)) = (video, window, self.ui.canvas_size) else {
            self.ui.flash("Nothing to zoom");
            return;
        };
        let mut size = zoom_window_size(window, canvas, video, ctx.pixels_per_point(), percent);
        // Never larger than the monitor; the picture is letterboxed instead.
        if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
            size = size.min(monitor);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        self.ui.flash(format!("Zoom {percent} %"));
    }

    fn tracks(&self, kind: TrackKind) -> &[player::TrackInfo] {
        match kind {
            TrackKind::Audio => &self.snapshot.audio_tracks,
            TrackKind::Subtitle => &self.snapshot.subtitle_tracks,
            TrackKind::Video => &self.snapshot.video_tracks,
        }
    }

    fn select_track(&mut self, kind: TrackKind, id: Option<i64>) {
        match self.player.select_track(kind, id) {
            Ok(()) => {
                // The snapshot taken right after the call already carries
                // the new selection, so the menu check mark moves at once.
                self.refresh();
                let name = id
                    .and_then(|id| self.tracks(kind).iter().find(|track| track.id == id))
                    .map(player::TrackInfo::label);
                self.ui.flash(match name {
                    Some(name) => format!("{}: {name}", kind.label()),
                    None => format!("{}: off", kind.label()),
                });
            }
            Err(err) => {
                tracing::warn!(%err, ?kind, ?id, "track selection failed");
                self.ui.flash(format!(
                    "Could not switch the {} track",
                    kind.label().to_lowercase()
                ));
            }
        }
    }

    fn load_subtitle(&mut self, path: &Path) {
        match self.player.add_subtitle_file(path) {
            Ok(()) => {
                self.refresh();
                self.ui.flash(format!("Subtitles: {}", display_name(path)));
            }
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "could not load subtitles");
                self.ui
                    .flash(format!("Could not load {}", display_name(path)));
            }
        }
    }

    fn adjust_delay(&mut self, kind: TrackKind, step: DelayStep) {
        let current = match kind {
            TrackKind::Audio => self.snapshot.audio_delay,
            TrackKind::Subtitle => self.snapshot.subtitle_delay,
            // Video is the reference clock; it has no delay of its own.
            TrackKind::Video => return,
        };
        let target = next_delay(current, step);
        let result = match kind {
            TrackKind::Audio => self.player.set_audio_delay(target),
            _ => self.player.set_subtitle_delay(target),
        };
        match result {
            Ok(()) => {
                self.refresh();
                let applied = match kind {
                    TrackKind::Audio => self.snapshot.audio_delay,
                    _ => self.snapshot.subtitle_delay,
                };
                self.ui.flash(format!(
                    "{} delay {}",
                    kind.label(),
                    ui::menu::format_delay(applied)
                ));
            }
            Err(err) => tracing::warn!(%err, ?kind, "could not change the delay"),
        }
    }

    fn seek_by(&mut self, seconds: i64) {
        let Some(duration) = self.snapshot.duration else {
            return;
        };
        let target = if seconds >= 0 {
            self.snapshot
                .position
                .saturating_add(Duration::from_secs(seconds.unsigned_abs()))
        } else {
            self.snapshot
                .position
                .saturating_sub(Duration::from_secs(seconds.unsigned_abs()))
        }
        .min(duration);

        self.seek_to(target);
        self.flash_position(target, duration);
    }

    /// Exact seek; the single place every precise seek goes through. It
    /// ends any drag in progress, so no stale scrub can follow it.
    fn seek_to(&mut self, target: Duration) {
        let Some(duration) = self.snapshot.duration else {
            return;
        };
        let target = target.min(duration);
        self.scrub.finish();
        let _ = self.player.seek(target);
        self.snapshot.position = target;
    }

    fn flash_position(&mut self, position: Duration, duration: Duration) {
        self.ui.flash(format!(
            "{} / {}",
            player::format_time(position),
            player::format_time(duration)
        ));
    }

    /// Absolute volume without OSD: the slider shows its own tooltip.
    fn set_volume(&mut self, volume: f32) {
        if !volume.is_finite() {
            return;
        }
        let volume = volume.clamp(0.0, 100.0);
        let _ = self.player.set_volume(volume);
        self.snapshot.volume = volume;
        self.config.volume = volume;
    }

    /// Relative volume step from keys, menus and the wheel, echoed on the OSD.
    fn change_volume(&mut self, delta: f32) {
        if delta == 0.0 {
            return;
        }
        self.set_volume(self.snapshot.volume + delta);
        if self.snapshot.muted && delta > 0.0 {
            let _ = self.player.set_muted(false);
            self.snapshot.muted = false;
        }
        self.ui
            .flash(format!("Volume {:.0}%", self.snapshot.volume));
    }

    fn change_speed(&mut self, factor: f64) {
        self.set_speed((self.snapshot.speed * factor).clamp(0.25, 4.0));
    }

    fn set_speed(&mut self, speed: f64) {
        let _ = self.player.set_speed(speed);
        self.snapshot.speed = speed;
        self.ui.flash(format!("Speed {speed:.2}×"));
    }

    // ------------------------------------------------------------------ UI

    /// The caption strip as a top panel (windowed mode).
    fn caption_panel(&mut self, ui: &mut egui::Ui) {
        if !self.ui.show_menu_bar {
            return;
        }
        egui::Panel::top("caption")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(ui::palette::CHROME))
            .show(ui, |ui| self.caption_contents(ui));
    }

    fn caption_contents(&mut self, ui: &mut egui::Ui) {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        let output = ui::caption::show(
            ui,
            &mut self.logo,
            &ui::caption::CaptionInput {
                menu: &ui::menu::MenuContext {
                    state: &self.ui,
                    snapshot: &self.snapshot,
                    always_on_top: self.config.ui.always_on_top,
                    playlist_len: self.playlist.len(),
                },
                name: APP_NAME,
                title: self
                    .snapshot
                    .media_path
                    .as_ref()
                    .and(self.snapshot.title.as_deref()),
                maximized,
                fullscreen: self.ui.fullscreen,
                // Wayland has no protocol for it; say so instead of
                // offering a button that does nothing.
                on_top_supported: self.drop_target.is_none(),
            },
        );

        let ctx = ui.ctx().clone();
        match output.window {
            Some(ui::caption::WindowAction::Drag) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            Some(ui::caption::WindowAction::ToggleMaximize) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            Some(ui::caption::WindowAction::Minimize) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            None => {}
        }
        if let Some(command) = output.command {
            self.run(command, &ctx);
        }
    }

    /// Height of the seek bar plus the control bar, as currently enabled.
    fn bottom_height(&self) -> f32 {
        let seek = if self.ui.show_seek_bar {
            ui::seek_bar::HEIGHT
        } else {
            0.0
        };
        let controls = if self.ui.show_controls {
            ui::control_bar::HEIGHT
        } else {
            0.0
        };
        seek + controls
    }

    /// Seek bar and control bar as a bottom panel (windowed mode).
    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        if self.bottom_height() <= 0.0 {
            return;
        }
        egui::Panel::bottom("controls")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(ui::palette::CHROME))
            .show(ui, |ui| self.bottom_contents(ui));
    }

    /// The seek bar, flush on top of the control bar; no margins, so the
    /// seek bar spans the full width.
    fn bottom_contents(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let top = ui.cursor().top();
        let mut seek = None;
        let mut command = None;
        if self.ui.show_seek_bar {
            seek = ui::seek_bar::show(ui, &self.snapshot).map(|gesture| match gesture {
                SeekGesture::Scrub(target) => Command::Scrub(target),
                SeekGesture::Commit(target) => Command::SeekTo(target),
            });
            // Remembered so the window-resize zones leave the ends of the
            // seek bar alone.
            self.ui.seek_band = Some(top..=ui.cursor().top());
        } else {
            self.ui.seek_band = None;
        }
        if self.ui.show_controls {
            let input = ui::control_bar::ControlBarInput {
                snapshot: &self.snapshot,
                show_remaining: self.ui.show_remaining,
                repeat: self.ui.repeat,
                shuffle: self.ui.shuffle,
                playlist_open: self.ui.show_playlist,
                fullscreen: self.ui.fullscreen,
            };
            command = ui::control_bar::show(ui, &input).command;
        }
        let area = egui::Rect::from_min_max(
            egui::pos2(ui.max_rect().left(), top),
            egui::pos2(ui.max_rect().right(), ui.cursor().top()),
        );

        // Seek-bar drags and the volume slider take the same path as every
        // key press and menu item.
        for command in [seek, command].into_iter().flatten() {
            self.run(command, ui.ctx());
        }

        // Wheel anywhere over the bar adjusts the volume, as over the video.
        if ui.rect_contains_pointer(area) {
            self.change_volume(ui::wheel_notches(ui.ctx()) as f32 * 2.0);
        }
    }

    fn playlist_panel(&mut self, ui: &mut egui::Ui) {
        if !self.ui.show_playlist {
            return;
        }

        let mut action = None;
        let playing = matches!(
            self.snapshot.state,
            PlaybackState::Playing | PlaybackState::Opening
        );
        egui::Panel::right("playlist")
            .resizable(true)
            .default_size(280.0)
            .min_size(180.0)
            // The video keeps most of a small window.
            .max_size((ui.available_width() * 0.4).max(180.0))
            .frame(egui::Frame::new().fill(ui::palette::PANEL))
            .show(ui, |ui| {
                action = ui::playlist_panel::show(ui, &mut self.playlist, playing);
            });

        match action {
            Some(PlaylistAction::Play(index)) => self.play_index(index),
            Some(PlaylistAction::Remove(index)) => {
                if self.playlist.selected() == Some(index) {
                    self.playlist.remove(index);
                    let _ = self.player.stop();
                    self.refresh();
                } else {
                    self.playlist.remove(index);
                }
            }
            Some(PlaylistAction::Clear) => self.run(Command::ClearPlaylist, ui.ctx()),
            Some(PlaylistAction::AddFiles) => self.run(Command::AddFiles, ui.ctx()),
            None => {}
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let stats = self
            .ui
            .show_statistics
            .then(|| ui::video::statistics_lines(&self.snapshot, self.backend));

        let has_media = self.snapshot.media_path.is_some();
        let overlays = ui::video::Overlays {
            clock: (self.ui.show_clock && has_media).then(|| {
                let duration = self
                    .snapshot
                    .duration
                    .map_or_else(|| "--:--".to_owned(), player::format_time);
                format!(
                    "{} / {duration}",
                    player::format_time(self.snapshot.position)
                )
            }),
            file_name: self
                .ui
                .show_file_name
                .then(|| self.snapshot.title.clone())
                .flatten(),
            drop_hint: self.ui.drop_hover,
        };

        let mut output = ui::video::VideoOutput::default();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui::palette::CANVAS))
            .show(ui, |ui| {
                output = ui::video::show(
                    ui,
                    &self.snapshot,
                    self.video.as_ref(),
                    self.ui.osd_text(),
                    stats.as_deref(),
                    &overlays,
                    &mut self.logo,
                );
                if self.ui.fullscreen
                    && let Some(canvas) = output.response.as_ref().map(|response| response.rect)
                {
                    self.fullscreen_chrome(ui, canvas);
                }
            });
        self.ui.canvas_size = Some(output.size);

        // The error banner floats over the top of the video area.
        let canvas = output.response.as_ref().map(|response| response.rect);
        // Below the caption when that overlays the video.
        let top = if self.ui.fullscreen && self.ui.show_menu_bar {
            ui::caption::HEIGHT
        } else {
            0.0
        };
        let dismissed = match (canvas, self.ui.current_notice()) {
            (Some(canvas), Some(notice)) => {
                ui::notice::show(ui.ctx(), canvas.translate(egui::vec2(0.0, top)), notice)
            }
            _ => false,
        };
        if dismissed {
            self.ui.notice = None;
        }
        if self.ui.show_clock && self.snapshot.state == PlaybackState::Playing {
            // Audio-only files have no frames to drive repaints.
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }

        // Right-click menu on the video.
        let context = output.response.as_ref().and_then(|response| {
            ui::menu::context_menu(
                response,
                &ui::menu::MenuContext {
                    state: &self.ui,
                    snapshot: &self.snapshot,
                    always_on_top: self.config.ui.always_on_top,
                    playlist_len: self.playlist.len(),
                },
            )
        });
        if let Some(command) = context {
            self.run(command, ui.ctx());
        }

        if output.double_clicked {
            self.run(Command::Fullscreen, ui.ctx());
        } else if output.clicked {
            self.run(Command::PlayPause, ui.ctx());
        } else if output.drag_started && !self.ui.fullscreen {
            // Dragging the picture moves the window, as on a classic
            // player; the window has no title bar when the caption is off.
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        self.change_volume(output.wheel as f32 * 2.0);
    }

    /// In full screen the caption and the bars are drawn *over* the video
    /// and fade in and out, so the picture never moves or rescales when
    /// they appear.
    fn fullscreen_chrome(&mut self, ui: &mut egui::Ui, canvas: egui::Rect) {
        let fade = ui.ctx().animate_bool_with_time(
            egui::Id::unique("fullscreen_chrome"),
            self.ui.controls_visible,
            0.2,
        );
        if fade <= 0.0 {
            return;
        }
        let plate = ui::palette::CHROME.gamma_multiply(0.9);

        if self.ui.show_menu_bar {
            let rect = egui::Rect::from_min_size(
                canvas.min,
                egui::vec2(canvas.width(), ui::caption::HEIGHT),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_opacity(fade);
                    ui.painter()
                        .rect_filled(rect, egui::CornerRadius::ZERO, plate);
                    self.caption_contents(ui);
                },
            );
        }

        let height = self.bottom_height();
        if height > 0.0 {
            let rect = egui::Rect::from_min_max(
                egui::pos2(canvas.left(), canvas.bottom() - height),
                canvas.max,
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_opacity(fade);
                    // The seek bar's own strip stays clear, so the picture
                    // shows through around the thin track.
                    let bar = egui::Rect::from_min_max(
                        egui::pos2(
                            rect.left(),
                            rect.bottom() - height
                                + if self.ui.show_seek_bar {
                                    ui::seek_bar::HEIGHT / 2.0
                                } else {
                                    0.0
                                },
                        ),
                        rect.max,
                    );
                    ui.painter()
                        .rect_filled(bar, egui::CornerRadius::ZERO, plate);
                    self.bottom_contents(ui);
                },
            );
        }
    }

    /// The window has no decorations, so its edges are resize handles:
    /// the pointer changes near an edge and a press starts a resize.
    fn resize_edges(&self, ctx: &egui::Context) {
        const MARGIN: f32 = 5.0;
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if self.ui.fullscreen || maximized {
            return;
        }
        let rect = ctx.content_rect();
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        if ctx.input(|i| i.pointer.any_down() && !i.pointer.primary_pressed()) {
            return; // a drag that started elsewhere
        }
        let on_seek_bar = self
            .ui
            .seek_band
            .as_ref()
            .is_some_and(|band| band.contains(&pos.y));
        let west = pos.x < rect.left() + MARGIN && !on_seek_bar;
        let east = pos.x > rect.right() - MARGIN && !on_seek_bar;
        let north = pos.y < rect.top() + MARGIN;
        let south = pos.y > rect.bottom() - MARGIN;

        use egui::{CursorIcon, ResizeDirection};
        let (direction, cursor) = match (north, south, west, east) {
            (true, _, true, _) => (ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
            (true, _, _, true) => (ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
            (_, true, true, _) => (ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
            (_, true, _, true) => (ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
            (true, ..) => (ResizeDirection::North, CursorIcon::ResizeNorth),
            (_, true, ..) => (ResizeDirection::South, CursorIcon::ResizeSouth),
            (_, _, true, _) => (ResizeDirection::West, CursorIcon::ResizeWest),
            (_, _, _, true) => (ResizeDirection::East, CursorIcon::ResizeEast),
            _ => return,
        };
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        if self.ui.show_options {
            let mut open = true;
            egui::Window::new("Options")
                .open(&mut open)
                .default_width(420.0)
                .resizable(true)
                .show(ctx, |ui| self.options_ui(ui, ctx));
            self.ui.show_options = open;
        }

        if self.ui.show_goto {
            self.goto_window(ctx);
        }

        if self.ui.show_open_url {
            self.open_url_window(ctx);
        }

        if self.ui.show_properties {
            // Bitrates and the active decoder change while playing: ask the
            // backend again about once a second while the dialog is open.
            if self
                .media_info_asked
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
            {
                self.player.refresh_media_info();
                self.media_info_asked = Some(Instant::now());
            }
            ctx.request_repaint_after(Duration::from_millis(500));

            let mut open = true;
            egui::Window::new("Media Info")
                .open(&mut open)
                .default_width(430.0)
                .show(ctx, |ui| self.properties_ui(ui));
            self.ui.show_properties = open;
        }

        if self.ui.show_about {
            let mut open = true;
            egui::Window::new("About").open(&mut open).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (mark, _) =
                        ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
                    self.logo.paint(ui.painter(), mark.min, 72.0, 1.0);
                    ui.vertical(|ui| {
                        ui.heading(APP_NAME);
                        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                        ui.add_space(6.0);
                        ui.label("A fast, native media player for Linux.");
                        ui.label("Built with Rust, egui and libmpv.");
                    });
                });
                ui.add_space(8.0);
                ui.separator();
                ui::dimmed_label(ui, "An independent, open-source project.");
            });
            self.ui.show_about = open;
        }
    }

    /// File ▸ Open URL…: a stream URL (`https://`, `rtsp://`, …) or a
    /// local path, added to the playlist and played.
    fn open_url_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut submit = false;
        egui::Window::new("Open URL")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.ui.url_text)
                            .desired_width(330.0)
                            .hint_text("https://… or /path/to/file"),
                    );
                    if std::mem::take(&mut self.ui.url_focus) {
                        field.request_focus();
                    }
                    submit |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    submit |= ui.button("Open").clicked();
                });
                ui::dimmed_label(
                    ui,
                    "Any address libmpv can open: http(s), rtsp, rtmp, udp, … or a local path.",
                );
            });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        if submit {
            let entry = self.ui.url_text.trim().to_owned();
            if entry.is_empty() {
                self.ui.flash("Enter a URL or a path");
            } else {
                // Failures are reported by `open_entries`; the dialog
                // stays open so the text can be corrected.
                open = self.open_entries(&[entry], true) == 0;
            }
        }
        self.ui.show_open_url = open;
    }

    /// Navigate ▸ Go To…: jump to a typed timecode.
    fn goto_window(&mut self, ctx: &egui::Context) {
        let Some(duration) = self.snapshot.duration else {
            self.ui.show_goto = false;
            return;
        };

        let mut open = true;
        let mut submit = false;
        egui::Window::new("Go To")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Time");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.ui.goto_text)
                            .desired_width(110.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if std::mem::take(&mut self.ui.goto_focus) {
                        field.request_focus();
                    }
                    submit |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    submit |= ui.button("Go").clicked();
                });
                ui::dimmed_label(
                    ui,
                    format!(
                        "Seconds, m:ss or h:mm:ss · up to {}",
                        player::format_time(duration)
                    ),
                );
            });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        if submit {
            match ui::parse_timecode(&self.ui.goto_text) {
                Some(target) => {
                    let target = target.min(duration);
                    self.run(Command::SeekTo(target), ctx);
                    self.flash_position(target, duration);
                    open = false;
                }
                None => self.ui.flash("Not a valid time"),
            }
        }
        self.ui.show_goto = open;
    }

    fn options_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(4.0);
        ui.heading("Playback");
        egui::Grid::new("options_playback")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Hardware decoding");
                egui::ComboBox::from_id_salt("hwdec")
                    .selected_text(&self.config.playback.hwdec)
                    .show_ui(ui, |ui| {
                        for mode in ["auto-safe", "auto", "vaapi", "nvdec", "vdpau", "no"] {
                            ui.selectable_value(
                                &mut self.config.playback.hwdec,
                                mode.to_owned(),
                                mode,
                            );
                        }
                    });
                ui.end_row();

                ui.label("Audio device");
                ui.add(
                    egui::TextEdit::singleline(&mut self.config.playback.audio_device)
                        .hint_text("default"),
                );
                ui.end_row();

                ui.label("Deinterlace");
                ui.checkbox(&mut self.config.playback.deinterlace, "Enable");
                ui.end_row();
            });
        ui::dimmed_label(ui, "Hardware decoding changes apply after a restart.");

        ui.add_space(8.0);
        ui.heading("Interface");
        ui.checkbox(
            &mut self.config.autoplay,
            "Play immediately when media is opened",
        );
        ui.checkbox(
            &mut self.config.resume_playback,
            "Resume from the last position",
        );
        ui.checkbox(
            &mut self.config.ui.controls_autohide,
            "Hide controls while playing in full screen",
        );

        let mut autohide = self.config.ui.autohide_ms;
        if ui
            .add(
                egui::Slider::new(&mut autohide, 500..=10_000)
                    .text("Auto-hide delay (ms)")
                    .logarithmic(true),
            )
            .changed()
        {
            self.config.ui.autohide_ms = autohide;
        }

        if ui
            .checkbox(&mut self.ui.show_playlist, "Show playlist panel")
            .changed()
        {
            self.config.ui.show_playlist = self.ui.show_playlist;
        }

        if ui
            .checkbox(&mut self.config.ui.always_on_top, "Always on top")
            .changed()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                if self.config.ui.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                },
            ));
        }

        ui.add_space(10.0);
        ui.separator();
        ui::dimmed_label(ui, "Config is saved to your XDG config directory on exit.");
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("properties")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Title");
                ui.label(self.snapshot.title.as_deref().unwrap_or("—"));
                ui.end_row();

                ui.label("File");
                let path = self
                    .snapshot
                    .media_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "—".to_owned());
                ui.add(egui::Label::new(path).truncate());
                ui.end_row();

                ui.label("State");
                ui.label(self.snapshot.state.label());
                ui.end_row();

                ui.label("Resolution");
                ui.label(
                    self.snapshot
                        .video_size
                        .map(|(w, h)| format!("{w} × {h}"))
                        .unwrap_or_else(|| "—".to_owned()),
                );
                ui.end_row();

                ui.label("Duration");
                ui.label(
                    self.snapshot
                        .duration
                        .map(player::format_time)
                        .unwrap_or_else(|| "—".to_owned()),
                );
                ui.end_row();

                ui.label("Backend");
                ui.label(self.backend);
                ui.end_row();

                ui.label("Audio tracks");
                ui.label(self.snapshot.audio_tracks.len().to_string());
                ui.end_row();

                ui.label("Subtitle tracks");
                ui.label(self.snapshot.subtitle_tracks.len().to_string());
                ui.end_row();

                // Container, codecs and the decoder in use, from the engine.
                let rows = self
                    .snapshot
                    .media_info
                    .as_ref()
                    .map(|info| info.rows())
                    .unwrap_or_default();
                for row in rows {
                    match row {
                        Some((label, value)) => {
                            ui.label(label);
                            ui.label(value);
                        }
                        None => {
                            ui.separator();
                            ui.separator();
                        }
                    }
                    ui.end_row();
                }
            });

        if [TrackKind::Video, TrackKind::Audio, TrackKind::Subtitle]
            .iter()
            .any(|kind| !self.tracks(*kind).is_empty())
        {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
        }
        for kind in [TrackKind::Video, TrackKind::Audio, TrackKind::Subtitle] {
            for track in self.tracks(kind) {
                let active = if track.selected { "  ✔" } else { "" };
                ui::dimmed_label(
                    ui,
                    format!("{}  ·  {}{active}", kind.label(), track.label()),
                );
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // X11: the windowing library reports dropped paths.
        let mut dropped: Vec<String> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path())
                .map(|path| path.to_string_lossy().into_owned())
                .collect()
        });
        // Wayland: our own receiver hands over `text/uri-list` payloads.
        if let Some(target) = &mut self.drop_target {
            for list in target.poll() {
                dropped.extend(media::parse_uri_list(&list));
            }
        }
        self.ui.drop_hover = ctx.input(|i| !i.raw.hovered_files.is_empty())
            || self
                .drop_target
                .as_ref()
                .is_some_and(crate::wayland_dnd::WaylandDrop::hovering);

        if !dropped.is_empty() {
            tracing::debug!(count = dropped.len(), "files dropped on the window");
            self.open_entries(&dropped, true);
        }
    }

    fn save_config(&mut self) {
        self.ui.store(&mut self.config.ui);
        if let Err(err) = self.config.save() {
            tracing::warn!(%err, "could not save the config");
        }
    }
}

impl eframe::App for PlayerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        static FIRST_PASS: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        crate::startup::mark_once(&FIRST_PASS, "first UI pass");

        self.tick(&ctx);
        self.update_activity(&ctx);
        self.handle_middle_click(&ctx);
        self.handle_keys(&ctx);
        self.handle_dropped_files(&ctx);

        if self.ui.fullscreen {
            // Nothing but the picture; the chrome is overlaid on it.
            self.central(ui);
        } else {
            self.caption_panel(ui);
            self.bottom_bar(ui);
            self.playlist_panel(ui);
            self.central(ui);
        }
        self.windows(&ctx);
        self.resize_edges(&ctx);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.save_config();
        self.drop_target = None;
        // Last point where the GL context is guaranteed current: free the
        // mpv render context and our FBO here, before the player is dropped.
        if let Some(video) = &self.video {
            video.destroy(gl);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// The display connection the window was created on, for hardware-decoder
/// interop. `None` on platforms or backends that do not expose one.
fn native_display(cc: &eframe::CreationContext<'_>) -> Option<player::NativeDisplay> {
    use raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle};

    match cc.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(handle) => Some(player::NativeDisplay::Wayland(handle.display)),
        RawDisplayHandle::Xlib(handle) => handle.display.map(player::NativeDisplay::X11),
        _ => None,
    }
}

/// The window's `wl_display`, when the session is native Wayland.
fn wayland_display(
    cc: &eframe::CreationContext<'_>,
) -> Option<std::ptr::NonNull<std::ffi::c_void>> {
    use raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle};

    match cc.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(handle) => Some(handle.display),
        _ => None,
    }
}

fn is_subtitle(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUBTITLE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The A-B points after one step at `position`. B is kept after A; a step
/// that would invert them starts over from the new point.
fn next_ab(
    ab: player::AbLoop,
    step: AbStep,
    position: Duration,
    duration: Duration,
) -> player::AbLoop {
    let at = position.min(duration);
    let cleared = player::AbLoop::default();
    match step {
        AbStep::Clear => cleared,
        AbStep::SetA => player::AbLoop {
            a: Some(at),
            b: ab.b.filter(|b| *b > at),
        },
        AbStep::SetB => match ab.a {
            Some(a) if a < at => player::AbLoop {
                a: Some(a),
                b: Some(at),
            },
            // B before A (or no A): loop from the start up to here.
            _ => player::AbLoop {
                a: Some(Duration::ZERO),
                b: Some(at).filter(|b| !b.is_zero()),
            },
        },
        AbStep::TenSeconds => player::AbLoop {
            a: Some(at),
            b: Some((at + Duration::from_secs(10)).min(duration)).filter(|b| *b > at),
        },
        AbStep::Cycle => match (ab.a, ab.b) {
            (None, _) => next_ab(ab, AbStep::SetA, position, duration),
            (Some(_), None) => next_ab(ab, AbStep::SetB, position, duration),
            (Some(_), Some(_)) => cleared,
        },
    }
}

fn describe_ab(ab: player::AbLoop) -> String {
    let at = |point: Option<Duration>| point.map_or_else(|| "—".to_owned(), player::format_time);
    match (ab.a, ab.b) {
        (None, None) => "A-B repeat off".to_owned(),
        (Some(_), None) => format!("A-B repeat: A = {}", at(ab.a)),
        _ => format!("A-B repeat: {} – {}", at(ab.a), at(ab.b)),
    }
}

/// `4:3`-style label for an aspect ratio menu value.
pub fn aspect_label(ratio: Option<f64>) -> String {
    const NAMED: [(f64, &str); 6] = [
        (4.0 / 3.0, "4:3"),
        (5.0 / 4.0, "5:4"),
        (16.0 / 9.0, "16:9"),
        (2.35, "2.35:1"),
        (1.85, "1.85:1"),
        (1.0, "1:1"),
    ];
    match ratio {
        None => "Default".to_owned(),
        Some(ratio) => NAMED
            .iter()
            .find(|(value, _)| (value - ratio).abs() < 1e-3)
            .map_or_else(|| format!("{ratio:.2}:1"), |(_, name)| (*name).to_owned()),
    }
}

/// Whether the PNG mpv was asked to write is complete. The screenshot
/// command runs asynchronously, so the file's PNG trailer is used as the
/// completion signal: a failed encode leaves no trailer.
fn capture_complete(path: &Path) -> bool {
    const PNG_TRAILER: [u8; 8] = *b"IEND\xaeB`\x82";
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return false;
    };
    if len < PNG_TRAILER.len() as u64 {
        return false;
    }
    if file
        .seek(SeekFrom::End(-(PNG_TRAILER.len() as i64)))
        .is_err()
    {
        return false;
    }
    let mut trailer = [0u8; PNG_TRAILER.len()];
    file.read_exact(&mut trailer).is_ok() && trailer == PNG_TRAILER
}

/// `Movie Title 0-12-34.png`: readable, sortable, and safe on any file
/// system (no `/`, `:` or control characters).
fn screenshot_name(title: &str, position: Duration) -> String {
    // mpv's `media-title` is the file name when the file has no title tag.
    let title = Path::new(title)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| crate::playlist::is_media_extension(ext))
        .and_then(|_| Path::new(title).file_stem()?.to_str())
        .unwrap_or(title);
    let clean: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(80)
        .collect();
    let clean = if clean.is_empty() {
        "frame".to_owned()
    } else {
        clean
    };
    let total = position.as_millis();
    let (h, m, s, ms) = (
        total / 3_600_000,
        total / 60_000 % 60,
        total / 1000 % 60,
        total % 1000,
    );
    format!("{clean} {h}-{m:02}-{s:02}.{ms:03}.png")
}

/// Window inner size (points) that shows a `video` (pixels) picture at
/// `percent` of its size, given the current window and video canvas sizes.
/// Everything that is not canvas (menus, bars, playlist) keeps its size.
fn zoom_window_size(
    window: egui::Vec2,
    canvas: egui::Vec2,
    video: (u32, u32),
    pixels_per_point: f32,
    percent: u16,
) -> egui::Vec2 {
    let scale = f32::from(percent) / 100.0 / pixels_per_point.max(0.1);
    let picture = egui::vec2(video.0 as f32, video.1 as f32) * scale;
    let chrome = (window - canvas).max(egui::Vec2::ZERO);
    // Below the minimum the picture is letterboxed inside the larger canvas.
    (chrome + picture)
        .round()
        .max(egui::Vec2::from(MIN_WINDOW_SIZE))
}

/// Delay after applying one step, snapped to the step grid so repeated
/// presses never accumulate float error (0.1 + 0.2 != 0.3).
fn next_delay(current: f64, step: DelayStep) -> f64 {
    let steps = (current / DELAY_STEP).round();
    match step {
        DelayStep::Earlier => (steps - 1.0) * DELAY_STEP,
        DelayStep::Later => (steps + 1.0) * DELAY_STEP,
        DelayStep::Reset => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_matches_the_packaging() {
        // The taskbar icon is found through the desktop file named after the
        // app id; the AppStream and Flatpak ids must be the same string.
        let desktop = include_str!("../packaging/io.github.mahziyarng98.crab-player.desktop");
        assert!(desktop.contains(&format!("\nIcon={APP_ID}\n")));
        assert!(desktop.contains(&format!("\nStartupWMClass={APP_ID}\n")));
        let metainfo = include_str!("../packaging/io.github.mahziyarng98.crab-player.metainfo.xml");
        assert!(metainfo.contains(&format!("<id>{APP_ID}</id>")));
        assert!(metainfo.contains(&format!(
            "<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>"
        )));
        let flatpak = include_str!("../packaging/flatpak/io.github.mahziyarng98.crab-player.yml");
        assert!(flatpak.contains(&format!("\napp-id: {APP_ID}\n")));
    }

    #[test]
    fn delay_steps_stay_on_the_grid() {
        let mut delay = 0.0;
        for _ in 0..3 {
            delay = next_delay(delay, DelayStep::Later);
        }
        assert!((delay - 0.3).abs() < 1e-9);
        for _ in 0..5 {
            delay = next_delay(delay, DelayStep::Earlier);
        }
        assert!((delay + 0.2).abs() < 1e-9);
        assert_eq!(next_delay(delay, DelayStep::Reset), 0.0);
        // An off-grid value (set elsewhere) snaps before stepping.
        assert!((next_delay(0.26, DelayStep::Later) - 0.4).abs() < 1e-9);
    }

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn ab(a: Option<u64>, b: Option<u64>) -> player::AbLoop {
        player::AbLoop {
            a: a.map(S),
            b: b.map(S),
        }
    }

    #[test]
    fn ab_cycle_sets_a_then_b_then_clears() {
        let d = S(100);
        let first = next_ab(ab(None, None), AbStep::Cycle, S(10), d);
        assert_eq!(first, ab(Some(10), None));
        let second = next_ab(first, AbStep::Cycle, S(25), d);
        assert_eq!(second, ab(Some(10), Some(25)));
        assert!(second.is_active());
        assert_eq!(next_ab(second, AbStep::Cycle, S(30), d), ab(None, None));
    }

    #[test]
    fn ab_points_never_invert() {
        let d = S(100);
        // B before A: loop from the start instead.
        assert_eq!(
            next_ab(ab(Some(50), None), AbStep::SetB, S(20), d),
            ab(Some(0), Some(20))
        );
        // Moving A past B drops B.
        assert_eq!(
            next_ab(ab(Some(10), Some(20)), AbStep::SetA, S(30), d),
            ab(Some(30), None)
        );
        // Moving A but staying before B keeps it.
        assert_eq!(
            next_ab(ab(Some(10), Some(20)), AbStep::SetA, S(15), d),
            ab(Some(15), Some(20))
        );
    }

    #[test]
    fn ten_second_section_is_clipped_to_the_end() {
        let d = S(100);
        assert_eq!(
            next_ab(ab(None, None), AbStep::TenSeconds, S(40), d),
            ab(Some(40), Some(50))
        );
        assert_eq!(
            next_ab(ab(None, None), AbStep::TenSeconds, S(95), d),
            ab(Some(95), Some(100))
        );
        // At the very end there is no section to loop.
        assert_eq!(
            next_ab(ab(None, None), AbStep::TenSeconds, S(100), d),
            ab(Some(100), None)
        );
        assert_eq!(
            next_ab(ab(Some(1), Some(2)), AbStep::Clear, S(5), d),
            ab(None, None)
        );
    }

    #[test]
    fn aspect_labels_name_the_common_ratios() {
        assert_eq!(aspect_label(None), "Default");
        assert_eq!(aspect_label(Some(16.0 / 9.0)), "16:9");
        assert_eq!(aspect_label(Some(2.35)), "2.35:1");
        assert_eq!(aspect_label(Some(1.6)), "1.60:1");
    }

    #[test]
    fn screenshot_names_are_safe_and_sortable() {
        let at = Duration::from_millis(3_723_045);
        assert_eq!(screenshot_name("Film", at), "Film 1-02-03.045.png");
        // A file-name title loses its media extension, other dots stay.
        assert_eq!(screenshot_name("base.mp4", at), "base 1-02-03.045.png");
        assert_eq!(screenshot_name("Dr. No", at), "Dr. No 1-02-03.045.png");
        assert_eq!(
            screenshot_name("a/b:c*d?\"e\"<f>|g", Duration::ZERO),
            "a_b_c_d__e__f__g 0-00-00.000.png"
        );
        assert_eq!(
            screenshot_name("   ", Duration::ZERO),
            "frame 0-00-00.000.png"
        );
        assert!(screenshot_name(&"x".repeat(300), Duration::ZERO).len() < 100);
    }

    #[test]
    fn capture_is_complete_only_with_the_png_trailer() {
        let dir = std::env::temp_dir().join(format!("crab-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("frame.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nnot written yet").unwrap();
        assert!(!capture_complete(&path));
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n\x00\x00\x00\x00IEND\xaeB`\x82").unwrap();
        assert!(capture_complete(&path));
        assert!(!capture_complete(&dir.join("missing.png")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zoom_keeps_the_chrome_and_scales_the_picture() {
        let window = egui::vec2(1120.0, 700.0);
        let canvas = egui::vec2(813.0, 610.0); // menu, bar and playlist take the rest
        // 640 × 480 at 100 % on a 1.0 display: canvas becomes 640 × 480.
        assert_eq!(
            zoom_window_size(window, canvas, (640, 480), 1.0, 100),
            egui::vec2(947.0, 570.0)
        );
        // 200 %, and on a 2.0 display 1 video pixel is half a point.
        assert_eq!(
            zoom_window_size(window, canvas, (640, 480), 2.0, 200),
            egui::vec2(947.0, 570.0)
        );
        // 50 % of 640 × 480 would need a 563 × 282 window: clamped to the
        // minimum window size instead.
        assert_eq!(
            zoom_window_size(window, canvas, (640, 480), 1.25, 50),
            egui::vec2(640.0, 400.0)
        );
        assert_eq!(
            zoom_window_size(window, canvas, (1920, 1080), 1.0, 50),
            egui::vec2(1267.0, 630.0)
        );
    }

    #[test]
    fn subtitles_are_recognised_by_extension() {
        assert!(is_subtitle(Path::new("/films/movie.srt")));
        assert!(is_subtitle(Path::new("MOVIE.ASS")));
        assert!(!is_subtitle(Path::new("movie.mkv")));
        assert!(!is_subtitle(Path::new("srt")));
    }
}
