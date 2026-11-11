//! Menus: the main menu behind the caption's menu button and the
//! right-click context menu.
//!
//! The main menu has the classic desktop-player groups (File / Play / View /
//! Audio / Subtitles / Navigate / Capture / Playlists) as submenus; the
//! context menu offers the same features grouped for quick access. The two
//! share their submenu builders, so an item
//! behaves the same wherever it appears. Items that are not functional yet
//! are listed disabled so the roadmap is visible in-app.
//!
//! Menus never mutate application state: every click becomes a [`Command`]
//! that `PlayerApp::run` dispatches, exactly like a key press.

use crate::command::{
    self, AbStep, AfterPlayback, Command, DelayStep, PICTURE_STEP, PictureControl, Preset,
    RepeatMode, RotateStep,
};
use crate::player::{PlayerSnapshot, TrackInfo, TrackKind};
use crate::playlist::SortOrder;
use crate::ui::UiState;

/// Read-only inputs the menus need to draw check marks and enabled states.
pub struct MenuContext<'a> {
    pub state: &'a UiState,
    pub snapshot: &'a PlayerSnapshot,
    pub always_on_top: bool,
    pub playlist_len: usize,
}

impl MenuContext<'_> {
    fn has_media(&self) -> bool {
        self.snapshot.media_path.is_some()
    }

    fn has_video(&self) -> bool {
        self.snapshot.video_size.is_some()
    }

    fn seekable(&self) -> bool {
        self.snapshot.duration.is_some()
    }
}

/// egui closes a menu on any click inside it by default. Check and radio
/// items must toggle in place (ui-spec §3), so menus only close on an outside
/// click and plain actions close them explicitly via `ui.close()`.
fn menu_config() -> egui::containers::menu::MenuConfig {
    egui::containers::menu::MenuConfig::default()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// The main menu, opened by the caption's menu button: the classic groups
/// as submenus of one popup.
pub fn popup(button: &egui::Response, cx: &MenuContext<'_>) -> Option<Command> {
    let mut out = None;
    let info = egui::UiStackInfo::new(egui::UiKind::Menu).with_tag_value(
        egui::containers::menu::MenuConfig::MENU_CONFIG_TAG,
        menu_config(),
    );
    egui::Popup::menu(button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .gap(4.0)
        .info(info)
        .show(|ui| {
            ui.set_min_width(200.0);
            let out = &mut out;
            item(ui, "Open File…", Command::OpenFile, out);
            ui.separator();
            ui.menu_button("File", |ui| file_menu(ui, cx, out));
            ui.menu_button("Play", |ui| play_menu(ui, cx, out));
            ui.menu_button("View", |ui| view_menu(ui, cx, out));
            ui.menu_button("Audio", |ui| audio_menu(ui, cx, out));
            ui.menu_button("Subtitles", |ui| subtitles_menu(ui, cx, out));
            ui.menu_button("Navigate", |ui| navigate_menu(ui, cx, out));
            ui.menu_button("Capture", |ui| capture_menu(ui, cx, out));
            ui.menu_button("Playlists", |ui| playlists_menu(ui, cx, out));
            ui.separator();
            item(ui, "Options…", Command::Options, out);
            placeholder(ui, "Keyboard Shortcuts…");
            item(ui, "Media Info", Command::Properties, out);
            ui.separator();
            item(ui, "About Crab Player", Command::About, out);
            item(ui, "Exit", Command::Quit, out);
        });
    out
}

/// Right-click menu on the video: open actions first, then Play / View / Video / Audio /
/// Subtitles groups, the three track lists, navigation, capture, playlists,
/// and the dialogs.
pub fn context_menu(response: &egui::Response, cx: &MenuContext<'_>) -> Option<Command> {
    let mut out = None;
    let info = egui::UiStackInfo::new(egui::UiKind::Menu).with_tag_value(
        egui::containers::menu::MenuConfig::MENU_CONFIG_TAG,
        menu_config(),
    );
    egui::Popup::context_menu(response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .info(info)
        .show(|ui| {
            ui.set_min_width(220.0);
            let out = &mut out;
            item(ui, "Open File…", Command::OpenFile, out);
            ui.menu_button("Open", |ui| {
                item(ui, "Open Folder…", Command::OpenFolder, out);
                item(ui, "Open URL…", Command::OpenUrl, out);
                item_enabled(
                    ui,
                    cx.has_media(),
                    "Load Subtitles…",
                    Command::LoadSubtitle,
                    out,
                );
                ui.separator();
                placeholder(ui, "Open Disc…");
                placeholder(ui, "Open Device…");
                ui.separator();
                item_enabled(ui, cx.has_media(), "Close", Command::CloseMedia, out);
                item(ui, "Media Info", Command::Properties, out);
            });
            ui.separator();
            ui.menu_button("Play", |ui| {
                item(ui, "Play / Pause", Command::PlayPause, out);
                item(ui, "Stop", Command::Stop, out);
                volume_menu(ui, cx, out);
                ui.separator();
                item(ui, "Decrease Rate", Command::SpeedDown, out);
                item(ui, "Reset Rate", Command::SpeedReset, out);
                item(ui, "Increase Rate", Command::SpeedUp, out);
                ab_repeat_menu(ui, cx, out);
                repeat_menu(ui, cx, out);
                after_playback_menu(ui, cx, out);
            });
            ui.menu_button("View", |ui| {
                zoom_items(ui, cx, out);
                fullscreen_item(ui, cx, out);
                ui.separator();
                on_top_menu(ui, cx, out);
                ui.separator();
                item_enabled(ui, cx.has_video(), "Save Image", Command::Screenshot, out);
                check(
                    ui,
                    cx.state.show_statistics,
                    "Display Statistics",
                    Command::ToggleStatistics,
                    out,
                );
                aspect_menu(ui, cx, out);
                rotate_menu(ui, cx, out);
                renderer_menu(ui, cx, out);
            });
            ui.menu_button("Video", |ui| {
                picture_menu(ui, cx, out);
                placeholder(ui, "Shaders");
                ui.separator();
                placeholder(ui, "VR");
                placeholder(ui, "3D Display");
            });
            ui.menu_button("Audio", |ui| {
                volume_menu(ui, cx, out);
                delay_menu(ui, cx, TrackKind::Audio, out);
                ui.separator();
                placeholder(ui, "Output Speakers");
                placeholder(ui, "Use Equalizer");
            });
            ui.menu_button("Subtitles", |ui| {
                item_enabled(
                    ui,
                    cx.has_media(),
                    "Load Subtitles…",
                    Command::LoadSubtitle,
                    out,
                );
                delay_menu(ui, cx, TrackKind::Subtitle, out);
                ui.separator();
                placeholder(ui, "Download Subtitles…");
                placeholder(ui, "Browse / Edit / Sync");
            });
            ui.separator();
            track_menu(ui, cx, TrackKind::Video, out);
            track_menu(ui, cx, TrackKind::Audio, out);
            track_menu(ui, cx, TrackKind::Subtitle, out);
            ui.separator();
            ui.menu_button("Navigate", |ui| navigate_menu(ui, cx, out));
            ui.menu_button("Capture", |ui| capture_menu(ui, cx, out));
            ui.separator();
            ui.menu_button("Playlists", |ui| playlists_menu(ui, cx, out));
            ui.separator();
            item(ui, "Options…", Command::Options, out);
            item(ui, "Media Info", Command::Properties, out);
            ui.separator();
            item(ui, "Exit", Command::Quit, out);
        });
    out
}

fn file_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    item(ui, "Open File…", Command::OpenFile, out);
    item(ui, "Open Folder…", Command::OpenFolder, out);
    item(ui, "Open URL…", Command::OpenUrl, out);
    placeholder(ui, "Open Disc…");
    placeholder(ui, "Open Device…");
    ui.menu_button("Recent Files", |ui| placeholder(ui, "(empty)"));
    ui.separator();
    item_enabled(ui, cx.has_media(), "Close", Command::CloseMedia, out);
    ui.separator();
    placeholder(ui, "Save a Copy…");
    item_enabled(ui, cx.has_video(), "Save Image", Command::Screenshot, out);
    placeholder(ui, "Save Thumbnails…");
    ui.menu_button("Subtitles", |ui| {
        item_enabled(ui, cx.has_media(), "Load…", Command::LoadSubtitle, out);
        placeholder(ui, "Save…");
        placeholder(ui, "Download…");
        placeholder(ui, "Upload…");
    });
    ui.separator();
    item(ui, "Media Info", Command::Properties, out);
    ui.separator();
    item(ui, "Exit", Command::Quit, out);
}

fn play_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    item(ui, "Play / Pause", Command::PlayPause, out);
    item(ui, "Stop", Command::Stop, out);
    placeholder(ui, "Frame Step");
    repeat_menu(ui, cx, out);
    ab_repeat_menu(ui, cx, out);
    ui.separator();

    item(ui, "Increase Rate", Command::SpeedUp, out);
    item(ui, "Decrease Rate", Command::SpeedDown, out);
    item(ui, "Reset Rate", Command::SpeedReset, out);
    ui.separator();

    placeholder(ui, "Filters");
    placeholder(ui, "Shaders");
    track_menu(ui, cx, TrackKind::Audio, out);
    track_menu(ui, cx, TrackKind::Subtitle, out);
    track_menu(ui, cx, TrackKind::Video, out);
    ui.separator();

    volume_menu(ui, cx, out);
    after_playback_menu(ui, cx, out);
    ui.separator();
    item_enabled(
        ui,
        cx.has_media(),
        "Copy Location",
        Command::CopyLocation,
        out,
    );
}

fn view_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let state = cx.state;

    check(
        ui,
        state.show_menu_bar,
        "Caption & Menu",
        Command::ToggleMenuBar,
        out,
    );
    check(
        ui,
        state.show_seek_bar,
        "Seek Bar",
        Command::ToggleSeekBar,
        out,
    );
    check(
        ui,
        state.show_controls,
        "Controls",
        Command::ToggleControls,
        out,
    );
    check(
        ui,
        state.show_playlist,
        "Playlist",
        Command::TogglePlaylist,
        out,
    );
    for label in [
        "MusicCTRL",
        "Subresync",
        "Capture",
        "Navigation",
        "Debug Shaders",
    ] {
        placeholder(ui, label);
    }
    ui.menu_button("Presets", |ui| {
        for (label, preset) in [
            ("Minimal", Preset::Minimal),
            ("Compact", Preset::Compact),
            ("Normal", Preset::Normal),
        ] {
            item(ui, label, Command::ApplyPreset(preset), out);
        }
    });
    ui.separator();

    fullscreen_item(ui, cx, out);
    ui.menu_button("Zoom", |ui| {
        zoom_items(ui, cx, out);
        placeholder(ui, "Auto Fit");
        placeholder(ui, "Auto Fit (larger only)");
    });
    ui.separator();
    renderer_menu(ui, cx, out);
    ui.menu_button("Video Frame", |ui| {
        for label in [
            "Half Size",
            "Normal Size",
            "Double Size",
            "Stretch to Window",
            "Touch Window from Inside",
            "Touch Window from Outside",
        ] {
            placeholder(ui, label);
        }
        ui.separator();
        aspect_menu(ui, cx, out);
    });
    rotate_menu(ui, cx, out);
    picture_menu(ui, cx, out);
    ui.menu_button("Pan & Scan", |ui| {
        for label in ["Increase Size", "Decrease Size", "Centre", "Reset"] {
            placeholder(ui, label);
        }
    });
    ui.separator();
    on_top_menu(ui, cx, out);
    item(ui, "Options…", Command::Options, out);
}

fn audio_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    item(ui, "Volume Up", Command::VolumeUp, out);
    item(ui, "Volume Down", Command::VolumeDown, out);
    check(ui, cx.snapshot.muted, "Mute", Command::Mute, out);
    ui.separator();
    track_menu(ui, cx, TrackKind::Audio, out);
    delay_menu(ui, cx, TrackKind::Audio, out);
    ui.menu_button("Output Speakers", |ui| {
        for label in ["Auto", "1.0", "2.0", "2.1", "4.0", "5.1", "7.1"] {
            placeholder(ui, label);
        }
    });
    placeholder(ui, "Use Equalizer");
    placeholder(ui, "Presets");
}

fn subtitles_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    item_enabled(
        ui,
        cx.has_media(),
        "Load Subtitles…",
        Command::LoadSubtitle,
        out,
    );
    placeholder(ui, "Save Subtitles…");
    ui.separator();
    placeholder(ui, "Download Subtitles…");
    placeholder(ui, "Upload Subtitles…");
    placeholder(ui, "Browse / Edit / Sync");
    ui.separator();
    track_menu(ui, cx, TrackKind::Subtitle, out);
    delay_menu(ui, cx, TrackKind::Subtitle, out);
    placeholder(ui, "Default Style…");
}

fn navigate_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let has_items = cx.playlist_len > 0;

    item_enabled(ui, has_items, "Previous", Command::Previous, out);
    item_enabled(ui, has_items, "Next", Command::Next, out);
    item_enabled(ui, cx.seekable(), "Go To…", Command::GoTo, out);
    ui.separator();
    for label in [
        "Title Menu",
        "Root Menu",
        "Subtitle Menu",
        "Audio Menu",
        "Angle Menu",
        "Chapter Menu",
    ] {
        placeholder(ui, label);
    }
    ui.separator();
    placeholder(ui, "Favorites");
}

fn capture_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    item_enabled(ui, cx.has_video(), "Save Image", Command::Screenshot, out);
    placeholder(ui, "Save Thumbnails…");
    placeholder(ui, "Advanced Capture…");
    placeholder(ui, "Save to Clipboard");
    placeholder(ui, "Save GIF…");
}

// ------------------------------------------------------- shared submenus

fn volume_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    ui.menu_button("Volume", |ui| {
        item(ui, "Up", Command::VolumeUp, out);
        item(ui, "Down", Command::VolumeDown, out);
        check(ui, cx.snapshot.muted, "Mute", Command::Mute, out);
    });
}

fn repeat_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    ui.menu_button("Repeat", |ui| {
        for (label, mode) in [
            ("Off", RepeatMode::Off),
            ("File", RepeatMode::File),
            ("Playlist", RepeatMode::Playlist),
        ] {
            radio(
                ui,
                cx.state.repeat == mode,
                label,
                Command::SetRepeat(mode),
                out,
            );
        }
        ui.separator();
        check(ui, cx.state.shuffle, "Shuffle", Command::ToggleShuffle, out);
    });
}

/// Play ▸ A-B Repeat, with the points shown so the state is visible.
fn ab_repeat_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let ab = cx.snapshot.ab_loop;
    let seekable = cx.seekable();
    ui.menu_button("A-B Repeat", |ui| {
        let point = |p: Option<std::time::Duration>| {
            p.map_or_else(|| "—".to_owned(), crate::player::format_time)
        };
        placeholder(ui, &format!("A {}   B {}", point(ab.a), point(ab.b)));
        ui.separator();
        item_enabled(
            ui,
            seekable,
            "Repeat 10 s From Here",
            Command::AbRepeat(AbStep::TenSeconds),
            out,
        );
        item_enabled(
            ui,
            seekable,
            "Set Start Point A",
            Command::AbRepeat(AbStep::SetA),
            out,
        );
        item_enabled(
            ui,
            seekable,
            "Set End Point B",
            Command::AbRepeat(AbStep::SetB),
            out,
        );
        item_enabled(
            ui,
            ab.a.is_some() || ab.b.is_some(),
            "Clear",
            Command::AbRepeat(AbStep::Clear),
            out,
        );
    });
}

fn after_playback_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    ui.menu_button("After Playback", |ui| {
        for (label, action) in [
            ("Do Nothing", AfterPlayback::Nothing),
            ("Play Next File in Folder", AfterPlayback::NextInFolder),
            ("Exit", AfterPlayback::Exit),
        ] {
            radio(
                ui,
                cx.state.after_playback == action,
                label,
                Command::SetAfterPlayback(action),
                out,
            );
        }
        ui.separator();
        for label in [
            "Turn Off the Monitor",
            "Stand By",
            "Hibernate",
            "Shut Down",
            "Log Off",
            "Lock",
        ] {
            placeholder(ui, label);
        }
    });
}

fn fullscreen_item(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let label = if cx.state.fullscreen {
        "Exit Full Screen"
    } else {
        "Full Screen"
    };
    item(ui, label, Command::Fullscreen, out);
}

fn zoom_items(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let enabled = cx.has_video() && !cx.state.fullscreen;
    for percent in [50, 100, 200] {
        item_enabled(
            ui,
            enabled,
            &format!("{percent} %"),
            Command::ZoomWindow(percent),
            out,
        );
    }
}

fn on_top_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    ui.menu_button("On Top", |ui| {
        radio(ui, !cx.always_on_top, "Default", Command::ToggleOnTop, out);
        radio(ui, cx.always_on_top, "Always", Command::ToggleOnTop, out);
        placeholder(ui, "While Playing");
        placeholder(ui, "While Playing Video");
    });
}

/// Aspect ratios on offer: the file's own, 4:3, 5:4, 16:9,
/// 2.35:1, 1.85:1, and square pixels (1:1).
pub const ASPECT_RATIOS: [Option<f64>; 7] = [
    None,
    Some(4.0 / 3.0),
    Some(5.0 / 4.0),
    Some(16.0 / 9.0),
    Some(2.35),
    Some(1.85),
    Some(1.0),
];

fn aspect_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let current = cx.snapshot.aspect_override;
    let enabled = cx.has_video();
    ui.menu_button("Aspect Ratio", |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            for ratio in ASPECT_RATIOS {
                let selected = match (current, ratio) {
                    (None, None) => true,
                    (Some(a), Some(b)) => (a - b).abs() < 1e-6,
                    _ => false,
                };
                radio(
                    ui,
                    selected,
                    &crate::app::aspect_label(ratio),
                    Command::SetAspect(ratio),
                    out,
                );
            }
        });
    });
}

fn rotate_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let enabled = cx.has_video();
    ui.menu_button("Rotate", |ui| {
        placeholder(ui, &format!("Current: {}°", cx.snapshot.rotation));
        ui.separator();
        item_enabled(
            ui,
            enabled,
            "Rotate Clockwise (90°)",
            Command::Rotate(RotateStep::Clockwise),
            out,
        );
        item_enabled(
            ui,
            enabled,
            "Rotate Counter-clockwise",
            Command::Rotate(RotateStep::CounterClockwise),
            out,
        );
        item_enabled(
            ui,
            enabled && cx.snapshot.rotation != 0,
            "Reset",
            Command::Rotate(RotateStep::Reset),
            out,
        );
        ui.separator();
        placeholder(ui, "Flip Vertically");
        placeholder(ui, "Mirror");
    });
}

/// Video ▸ Picture: brightness, contrast, saturation and hue in ±5 steps,
/// with the current value next to each control. Steps keep the menu open.
fn picture_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let picture = cx.snapshot.picture;
    let enabled = cx.has_video();
    ui.menu_button("Picture", |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            egui::Grid::new("picture_controls")
                .num_columns(4)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for control in PictureControl::ALL {
                        let value = match control {
                            PictureControl::Brightness => picture.brightness,
                            PictureControl::Contrast => picture.contrast,
                            PictureControl::Saturation => picture.saturation,
                            PictureControl::Hue => picture.hue,
                        };
                        ui.label(control.label());
                        ui.label(egui::RichText::new(format!("{value:+}")).monospace());
                        if ui.small_button("−").clicked() {
                            *out = Some(Command::AdjustPicture(control, -PICTURE_STEP));
                        }
                        if ui.small_button("+").clicked() {
                            *out = Some(Command::AdjustPicture(control, PICTURE_STEP));
                        }
                        ui.end_row();
                    }
                });
            ui.separator();
            if ui
                .add_enabled(!picture.is_neutral(), egui::Button::new("Reset Colours"))
                .clicked()
            {
                *out = Some(Command::ResetPicture);
            }
        });
    });
}

fn renderer_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    ui.menu_button("Renderer Settings", |ui| {
        check(
            ui,
            cx.state.show_statistics,
            "Display Statistics",
            Command::ToggleStatistics,
            out,
        );
        check(
            ui,
            cx.state.show_clock,
            "Display Current Time",
            Command::ToggleClockOverlay,
            out,
        );
        check(
            ui,
            cx.state.show_file_name,
            "Display File Name",
            Command::ToggleFileNameOverlay,
            out,
        );
        let mut hwdec = cx.snapshot.hardware_decoding;
        if ui
            .add_enabled(
                !cx.snapshot.hardware_decoding_locked,
                egui::Checkbox::new(&mut hwdec, "Hardware Decoding"),
            )
            .changed()
        {
            *out = Some(Command::ToggleHardwareDecoding);
        }
        ui.separator();
        for label in ["Output Range", "Colour Management", "VSync", "Reset"] {
            placeholder(ui, label);
        }
    });
}

fn playlists_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, out: &mut Option<Command>) {
    let has_items = cx.playlist_len > 0;
    check(
        ui,
        cx.state.show_playlist,
        "Playlist Editor",
        Command::TogglePlaylist,
        out,
    );
    ui.separator();
    item(ui, "Add Files…", Command::AddFiles, out);
    item(ui, "Add Folder…", Command::OpenFolder, out);
    item_enabled(ui, has_items, "Clear Playlist", Command::ClearPlaylist, out);
    ui.menu_button("Sort", |ui| {
        ui.add_enabled_ui(cx.playlist_len > 1, |ui| {
            for (label, order) in [
                ("By Name, Ascending", SortOrder::NameAscending),
                ("By Name, Descending", SortOrder::NameDescending),
                ("By Folder, Ascending", SortOrder::FolderAscending),
                ("By Folder, Descending", SortOrder::FolderDescending),
                ("Random", SortOrder::Random),
            ] {
                item(ui, label, Command::SortPlaylist(order), out);
            }
        });
    });
}

fn tracks(snapshot: &PlayerSnapshot, kind: TrackKind) -> &[TrackInfo] {
    match kind {
        TrackKind::Audio => &snapshot.audio_tracks,
        TrackKind::Subtitle => &snapshot.subtitle_tracks,
        TrackKind::Video => &snapshot.video_tracks,
    }
}

/// "<Kind> Track" submenu: every track of the kind with a check mark on the
/// active one. Subtitles additionally offer "Off"; audio and video always
/// keep one track active.
fn track_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, kind: TrackKind, out: &mut Option<Command>) {
    let tracks = tracks(cx.snapshot, kind);
    let cycle = Command::CycleTrack(kind);

    ui.menu_button(format!("{} Track", kind.label()), |ui| {
        if tracks.is_empty() {
            placeholder(ui, "(none)");
            return;
        }
        if kind == TrackKind::Subtitle {
            let off = !tracks.iter().any(|track| track.selected);
            check(ui, off, "Off", Command::SelectTrack(kind, None), out);
            ui.separator();
        }
        for track in tracks {
            check(
                ui,
                track.selected,
                &track.label(),
                Command::SelectTrack(kind, Some(track.id)),
                out,
            );
        }
        if tracks.len() > 1 || kind == TrackKind::Subtitle {
            ui.separator();
            item(ui, "Next Track", cycle, out);
        }
    });
}

/// "<Kind> Delay" submenu with the current offset as its first line.
fn delay_menu(ui: &mut egui::Ui, cx: &MenuContext<'_>, kind: TrackKind, out: &mut Option<Command>) {
    let (delay, has_tracks) = match kind {
        TrackKind::Audio => (
            cx.snapshot.audio_delay,
            !cx.snapshot.audio_tracks.is_empty(),
        ),
        TrackKind::Subtitle => (
            cx.snapshot.subtitle_delay,
            !cx.snapshot.subtitle_tracks.is_empty(),
        ),
        TrackKind::Video => return,
    };

    ui.menu_button(format!("{} Delay", kind.label()), |ui| {
        placeholder(ui, &format!("Current: {}", format_delay(delay)));
        ui.separator();
        for (label, step) in [
            ("Later (+100 ms)", DelayStep::Later),
            ("Earlier (−100 ms)", DelayStep::Earlier),
            ("Reset", DelayStep::Reset),
        ] {
            // Steps keep the menu open so several can be applied in a row.
            let command = Command::AdjustDelay(kind, step);
            let button = egui::Button::new(label).shortcut_text(command::label(command));
            if ui.add_enabled(has_tracks, button).clicked() {
                *out = Some(command);
            }
        }
    });
}

/// `+250 ms`, `−100 ms`, `0 ms`: signed, in whole milliseconds.
pub fn format_delay(seconds: f64) -> String {
    let millis = (seconds * 1000.0).round() as i64;
    match millis {
        0 => "0 ms".to_owned(),
        m if m > 0 => format!("+{m} ms"),
        m => format!("−{} ms", m.unsigned_abs()),
    }
}

/// Plain action: closes the menu on selection, shortcut right-aligned.
fn item(ui: &mut egui::Ui, label: &str, command: Command, out: &mut Option<Command>) {
    item_enabled(ui, true, label, command, out);
}

fn item_enabled(
    ui: &mut egui::Ui,
    enabled: bool,
    label: &str,
    command: Command,
    out: &mut Option<Command>,
) {
    let button = egui::Button::new(label).shortcut_text(command::label(command));
    if ui.add_enabled(enabled, button).clicked() {
        ui.close();
        *out = Some(command);
    }
}

/// Checkable item: toggles in place and keeps the menu open.
fn check(
    ui: &mut egui::Ui,
    checked: bool,
    label: &str,
    command: Command,
    out: &mut Option<Command>,
) {
    let mut value = checked;
    let response = ui.checkbox(&mut value, label);
    let shortcut = command::label(command);
    let response = if shortcut.is_empty() {
        response
    } else {
        response.on_hover_text(shortcut)
    };
    if response.changed() {
        *out = Some(command);
    }
}

/// One choice of an exclusive group; keeps the menu open like a check item.
fn radio(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    command: Command,
    out: &mut Option<Command>,
) {
    if ui.radio(selected, label).clicked() && !selected {
        *out = Some(command);
    }
}

/// Planned item that is not functional yet.
fn placeholder(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_signed_whole_milliseconds() {
        assert_eq!(format_delay(0.0), "0 ms");
        assert_eq!(format_delay(0.25), "+250 ms");
        assert_eq!(format_delay(-0.1), "−100 ms");
        // Float noise from repeated 0.1 s steps must not show up.
        assert_eq!(format_delay(0.1 + 0.1 + 0.1), "+300 ms");
        assert_eq!(format_delay(0.1 - 0.1 - 1e-12), "0 ms");
    }

    #[test]
    fn tracks_are_looked_up_by_kind() {
        let track = |id| TrackInfo {
            id,
            ..Default::default()
        };
        let snapshot = PlayerSnapshot {
            audio_tracks: vec![track(1), track(2)].into(),
            subtitle_tracks: vec![track(7)].into(),
            ..Default::default()
        };
        assert_eq!(tracks(&snapshot, TrackKind::Audio).len(), 2);
        assert_eq!(tracks(&snapshot, TrackKind::Subtitle)[0].id, 7);
        assert!(tracks(&snapshot, TrackKind::Video).is_empty());
    }
}
