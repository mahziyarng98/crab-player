//! Transport bar: time on the left, the transport group in the middle,
//! volume, the decoder badge, playlist and full screen on the right.
//!
//! Layout (`docs/ui-spec.md` §6) is computed by [`plan`] from the bar's
//! width alone, so it can be tested without a UI:
//!
//! - the transport group (previous, play/pause, stop, next) is always
//!   centred in the bar, with play/pause as the largest button;
//! - the right cluster keeps mute, the decoder badge, playlist and full
//!   screen; the volume slider shrinks and finally disappears;
//! - the optional toggles (repeat, subtitles, A-B, shuffle, capture) sit
//!   left of the volume and drop out, least important first, when the
//!   window is narrow, so nothing ever overlaps.
//!
//! Icons come from [`crate::ui::icons`]; the bar only reports intents.

use crate::command::{AbStep, Command, RepeatMode};
use crate::player::{PlaybackState, PlayerSnapshot, TrackKind, format_time};
use crate::ui::icons::{self, ButtonState, Icon};
use crate::ui::palette;

/// The bar only reports intents; the slider is no exception and yields
/// `Command::SetVolume`.
#[derive(Default)]
pub struct ControlBarOutput {
    pub command: Option<Command>,
}

/// Everything the bar shows besides the snapshot.
pub struct ControlBarInput<'a> {
    pub snapshot: &'a PlayerSnapshot,
    pub show_remaining: bool,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub playlist_open: bool,
    pub fullscreen: bool,
}

/// Optional toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Toggle {
    Repeat,
    Subtitles,
    AbRepeat,
    Shuffle,
    Capture,
}

/// Height of the bar in points.
pub const HEIGHT: f32 = 46.0;
const EDGE: f32 = 10.0;
const GAP: f32 = 2.0;
const BUTTON: egui::Vec2 = egui::vec2(32.0, 30.0);
/// Play/pause: a circle of this diameter.
const PLAY: f32 = 38.0;
const BADGE_WIDTH: f32 = 40.0;
const AB_WIDTH: f32 = 38.0;
const VOLUME_MAX: f32 = 96.0;
/// Below this the slider is too short to aim at and is left out.
const VOLUME_MIN: f32 = 44.0;
/// previous + play + stop + next with their gaps.
const CENTER_WIDTH: f32 = 3.0 * BUTTON.x + PLAY + 3.0 * 6.0;
/// Clear space kept on both sides of the transport group.
const CENTER_MARGIN: f32 = 14.0;
/// Right cluster without slider and toggles: mute, badge, playlist, full
/// screen and the gaps between them.
const RIGHT_FIXED: f32 = 3.0 * BUTTON.x + BADGE_WIDTH + 2.0 * 6.0 + GAP;

fn toggle_width(toggle: Toggle) -> f32 {
    match toggle {
        Toggle::AbRepeat => AB_WIDTH,
        _ => BUTTON.x,
    }
}

/// What fits into a bar of a given width.
#[derive(Debug, PartialEq)]
struct Plan {
    /// Width of the volume slider; `0.0` when it is left out.
    volume: f32,
    /// Toggles that fit, most important first.
    toggles: Vec<Toggle>,
    /// Whether the open button fits next to the time.
    open: bool,
}

/// Decide what is shown in a bar `width` points wide whose time label is
/// `time_width` wide. The transport group and the fixed right cluster
/// always fit at the minimum window width.
fn plan(width: f32, time_width: f32) -> Plan {
    // Most important first.
    const PRIORITY: [Toggle; 5] = [
        Toggle::Repeat,
        Toggle::Subtitles,
        Toggle::AbRepeat,
        Toggle::Shuffle,
        Toggle::Capture,
    ];
    let side = width / 2.0 - CENTER_WIDTH / 2.0 - CENTER_MARGIN - EDGE;

    let mut budget = side - RIGHT_FIXED;
    let volume = (budget - 4.0).min(VOLUME_MAX);
    let volume = if volume >= VOLUME_MIN { volume } else { 0.0 };
    if volume > 0.0 {
        budget -= volume + 4.0;
    }
    // The slider is worth more than any toggle: toggles only get the room
    // left once it has its full width.
    let toggles = if volume < VOLUME_MAX {
        Vec::new()
    } else {
        PRIORITY
            .into_iter()
            .take_while(|toggle| {
                budget -= toggle_width(*toggle) + GAP;
                budget >= 0.0
            })
            .collect()
    };

    Plan {
        volume,
        toggles,
        open: side >= time_width + 12.0 + BUTTON.x,
    }
}

pub fn show(ui: &mut egui::Ui, input: &ControlBarInput<'_>) -> ControlBarOutput {
    let snapshot = input.snapshot;
    let mut output = ControlBarOutput::default();
    let playing = snapshot.state == PlaybackState::Playing;
    let has_media = snapshot.media_path.is_some();

    // The bar takes clicks itself, so that in full screen, where it lies
    // over the video, a click between two buttons does not pause playback.
    let (bar, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HEIGHT),
        egui::Sense::click(),
    );
    let middle = bar.center().y;
    let painter = ui.painter().clone();
    let mut press = |command| output.command = Some(command);
    let slot = |left: f32, width: f32| {
        egui::Rect::from_min_size(
            egui::pos2(left, middle - BUTTON.y / 2.0),
            egui::vec2(width, BUTTON.y),
        )
    };

    // --- left: time (and speed, when it is not 1×) ----------------------
    let time = painter.layout_no_wrap(
        time_text(snapshot, input.show_remaining),
        egui::FontId::monospace(12.5),
        palette::TEXT,
    );
    let layout = plan(bar.width(), time.size().x);
    let time_rect = egui::Rect::from_min_size(
        egui::pos2(bar.left() + EDGE + 4.0, middle - time.size().y / 2.0),
        time.size(),
    );
    let time_response = ui.allocate_rect(
        time_rect.expand2(egui::vec2(4.0, 6.0)),
        egui::Sense::click(),
    );
    painter.galley(time_rect.min, time, palette::TEXT);
    if time_response
        .on_hover_text("Click to switch between total and remaining time")
        .clicked()
    {
        press(Command::ToggleTimeDisplay);
    }
    let mut x = time_rect.right() + 10.0;
    if layout.open {
        if icons::button_at(
            ui,
            slot(x, BUTTON.x),
            Icon::Open,
            "Open file (Ctrl+O)",
            ButtonState::Normal,
        ) {
            press(Command::OpenFile);
        }
        x += BUTTON.x + 6.0;
    }
    if (snapshot.speed - 1.0).abs() > f64::EPSILON {
        let speed = painter.layout_no_wrap(
            format!("{:.2}×", snapshot.speed),
            egui::FontId::proportional(12.0),
            palette::ACCENT_LIGHT,
        );
        // Only where it cannot run into the transport group.
        if x + speed.size().x < bar.center().x - CENTER_WIDTH / 2.0 - 6.0 {
            painter.galley(
                egui::pos2(x, middle - speed.size().y / 2.0),
                speed,
                palette::ACCENT_LIGHT,
            );
        }
    }

    // --- centre: previous, play/pause, stop, next -----------------------
    let mut x = bar.center().x - CENTER_WIDTH / 2.0;
    if icons::button_at(
        ui,
        slot(x, BUTTON.x),
        Icon::Previous,
        "Previous (PgUp)",
        ButtonState::Normal,
    ) {
        press(Command::Previous);
    }
    x += BUTTON.x + 6.0;
    let (icon, tooltip) = if playing {
        (Icon::PauseFilled, "Pause (Space)")
    } else {
        (Icon::PlayFilled, "Play (Space)")
    };
    if play_button(ui, egui::pos2(x + PLAY / 2.0, middle), icon, tooltip) {
        press(Command::PlayPause);
    }
    x += PLAY + 6.0;
    if icons::button_at(
        ui,
        slot(x, BUTTON.x),
        Icon::Stop,
        "Stop (S)",
        ButtonState::enabled(has_media),
    ) {
        press(Command::Stop);
    }
    x += BUTTON.x + 6.0;
    if icons::button_at(
        ui,
        slot(x, BUTTON.x),
        Icon::Next,
        "Next (PgDn)",
        ButtonState::Normal,
    ) {
        press(Command::Next);
    }

    // --- right, from the edge inwards -----------------------------------
    let mut right = bar.right() - EDGE;
    let mut take = |width: f32, gap: f32| {
        let rect = slot(right - width, width);
        right -= width + gap;
        rect
    };
    let (icon, tooltip) = if input.fullscreen {
        (Icon::Restore, "Leave full screen (F, Esc or middle click)")
    } else {
        (Icon::Fullscreen, "Full screen (F or middle click)")
    };
    if icons::button_at(ui, take(BUTTON.x, GAP), icon, tooltip, ButtonState::Normal) {
        press(Command::Fullscreen);
    }
    if icons::button_at(
        ui,
        take(BUTTON.x, 6.0),
        Icon::Playlist,
        "Playlist (F9)",
        ButtonState::active(input.playlist_open),
    ) {
        press(Command::TogglePlaylist);
    }
    if let Some(command) = decoder_badge(ui, take(BADGE_WIDTH, 6.0), snapshot) {
        press(command);
    }
    if layout.volume > 0.0 {
        let rect = take(layout.volume, 4.0);
        if let Some(volume) = volume_slider(ui, rect, snapshot.volume) {
            press(Command::SetVolume(volume));
        }
    }
    let mute_icon = if snapshot.muted {
        Icon::SpeakerMuted
    } else {
        Icon::Speaker
    };
    if icons::button_at(
        ui,
        take(BUTTON.x, GAP),
        mute_icon,
        "Mute (M)",
        ButtonState::Normal,
    ) {
        press(Command::Mute);
    }

    for toggle in &layout.toggles {
        let rect = take(toggle_width(*toggle), GAP);
        let command = match toggle {
            Toggle::Repeat => {
                let (icon, state, tooltip) = match input.repeat {
                    RepeatMode::Off => (Icon::Repeat, ButtonState::Normal, "Repeat: off"),
                    RepeatMode::Playlist => (Icon::Repeat, ButtonState::Active, "Repeat: playlist"),
                    RepeatMode::File => (Icon::RepeatOne, ButtonState::Active, "Repeat: this file"),
                };
                icons::button_at(ui, rect, icon, tooltip, state)
                    .then_some(Command::SetRepeat(next_repeat(input.repeat)))
            }
            Toggle::Subtitles => subtitles_button(ui, rect, snapshot),
            Toggle::AbRepeat => ab_button(ui, rect, snapshot),
            Toggle::Shuffle => icons::button_at(
                ui,
                rect,
                Icon::Shuffle,
                "Shuffle",
                ButtonState::active(input.shuffle),
            )
            .then_some(Command::ToggleShuffle),
            Toggle::Capture => icons::button_at(
                ui,
                rect,
                Icon::Capture,
                "Save image (Ctrl+Shift+S)",
                ButtonState::enabled(snapshot.video_size.is_some()),
            )
            .then_some(Command::Screenshot),
        };
        if let Some(command) = command {
            press(command);
        }
    }

    output
}

/// The play/pause button: a filled glyph in a circle, larger than the
/// rest, with the accent colour under the pointer.
fn play_button(ui: &mut egui::Ui, center: egui::Pos2, icon: Icon, tooltip: &str) -> bool {
    let rect = egui::Rect::from_center_size(center, egui::Vec2::splat(PLAY));
    let response = ui.allocate_rect(rect, egui::Sense::click());
    let pressed = response.is_pointer_button_down_on();
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        response.hovered() || pressed,
        icons::HOVER_FADE,
    );
    let radius = PLAY / 2.0 - 1.0;
    let fill = if pressed {
        palette::ACCENT
    } else {
        palette::HOVER.lerp_to_gamma(palette::ACCENT_DEEP, hover)
    };
    let painter = ui.painter();
    painter.circle_filled(center, radius, fill);
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            1.0,
            palette::SEPARATOR.lerp_to_gamma(palette::ACCENT_LIGHT, hover),
        ),
    );
    let glyph = egui::Rect::from_center_size(center, egui::Vec2::splat(30.0));
    icons::paint_icon(painter, glyph, icon, palette::TEXT);
    response.on_hover_text(tooltip).clicked()
}

/// A slim horizontal slider for the volume, 0..=100. Click or drag
/// anywhere on it. Returns the new value while it is being changed.
fn volume_slider(ui: &mut egui::Ui, rect: egui::Rect, volume: f32) -> Option<f32> {
    const THUMB: f32 = 5.0;
    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left() + THUMB, rect.center().y - 1.5),
        egui::pos2(rect.right() - THUMB, rect.center().y + 1.5),
    );

    let mut value = volume.clamp(0.0, 100.0);
    let mut changed = None;
    if (response.dragged() || response.clicked())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        value = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0) * 100.0;
        changed = Some(value);
    }

    let active = response.hovered() || response.dragged();
    let grow =
        ui.ctx()
            .animate_bool_with_time(response.id.with("hover"), active, icons::HOVER_FADE);
    let x = track.left() + track.width() * value / 100.0;
    let painter = ui.painter();
    painter.rect_filled(track, egui::CornerRadius::same(2), palette::TRACK);
    painter.rect_filled(
        egui::Rect::from_min_max(track.min, egui::pos2(x, track.max.y)),
        egui::CornerRadius::same(2),
        palette::ACCENT,
    );
    painter.circle_filled(
        egui::pos2(x, track.center().y),
        THUMB + grow,
        palette::THUMB.lerp_to_gamma(palette::ICON_HOVER, grow),
    );

    // The percentage follows the handle while dragging, not only after
    // the usual hover delay.
    let percent = format!("Volume: {value:.0}%");
    if response.dragged() {
        response.show_tooltip_text(percent);
    } else {
        response.on_hover_text(percent);
    }
    changed
}

/// Repeat button cycle: off → playlist → this file → off.
fn next_repeat(mode: RepeatMode) -> RepeatMode {
    match mode {
        RepeatMode::Off => RepeatMode::Playlist,
        RepeatMode::Playlist => RepeatMode::File,
        RepeatMode::File => RepeatMode::Off,
    }
}

/// `0:42 / 1:58:07`, or `0:42 / -1:57:25` when showing the remaining time.
/// Without a known duration (idle, live streams) the right side is `--:--`.
fn time_text(snapshot: &PlayerSnapshot, show_remaining: bool) -> String {
    let position = format_time(snapshot.position);
    match snapshot.duration {
        Some(duration) if show_remaining => format!(
            "{position} / -{}",
            format_time(duration.saturating_sub(snapshot.position))
        ),
        Some(duration) => format!("{position} / {}", format_time(duration)),
        None => format!("{position} / --:--"),
    }
}

/// `HW` (accent) while a hardware decoder is active, `SW` otherwise. A
/// click switches hardware decoding on or off.
fn decoder_badge(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    snapshot: &PlayerSnapshot,
) -> Option<Command> {
    let decoder = snapshot.hardware_decoder.as_deref();
    let text = if decoder.is_some() { "HW" } else { "SW" };
    let tooltip = match (
        decoder,
        snapshot.hardware_decoding_locked,
        snapshot.hardware_decoding,
    ) {
        (Some(name), _, _) => format!("Hardware decoding ({name}) · click to turn off"),
        (None, true, _) => "Software decoding · hardware decoding is not available".to_owned(),
        (None, false, true) => "Software decoding (no hardware decoder for this file)".to_owned(),
        (None, false, false) => "Software decoding · click to turn hardware decoding on".to_owned(),
    };
    let state = if snapshot.hardware_decoding_locked {
        ButtonState::Disabled
    } else {
        ButtonState::active(decoder.is_some())
    };
    icons::badge_at(ui, rect, text, &tooltip, state).then_some(Command::ToggleHardwareDecoding)
}

/// Subtitles on/off: turns them off when a track is shown, otherwise shows
/// the first (or next) track.
fn subtitles_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    snapshot: &PlayerSnapshot,
) -> Option<Command> {
    let tracks = &snapshot.subtitle_tracks;
    let showing = tracks.iter().any(|track| track.selected);
    let state = if tracks.is_empty() {
        ButtonState::Disabled
    } else {
        ButtonState::active(showing)
    };
    let tooltip = if showing {
        "Subtitles on · click to hide (J cycles tracks)"
    } else {
        "Subtitles off · click to show (J cycles tracks)"
    };
    icons::button_at(ui, rect, Icon::Subtitles, tooltip, state).then_some(if showing {
        Command::SelectTrack(TrackKind::Subtitle, None)
    } else {
        Command::CycleTrack(TrackKind::Subtitle)
    })
}

/// One button for A-B repeat: sets A, then B, then clears (like the `L`
/// key). The badge shows how far along it is.
fn ab_button(ui: &mut egui::Ui, rect: egui::Rect, snapshot: &PlayerSnapshot) -> Option<Command> {
    let ab = snapshot.ab_loop;
    let (text, state, tooltip) = match (ab.a, ab.b) {
        _ if snapshot.duration.is_none() => ("A-B", ButtonState::Disabled, "A-B repeat"),
        (None, _) => ("A-B", ButtonState::Normal, "A-B repeat: click to set A (L)"),
        (Some(_), None) => ("A-", ButtonState::Active, "A-B repeat: click to set B (L)"),
        (Some(_), Some(_)) => (
            "A-B",
            ButtonState::Active,
            "A-B repeat on · click to clear (L)",
        ),
    };
    icons::badge_at(ui, rect, text, tooltip, state).then_some(Command::AbRepeat(AbStep::Cycle))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn snapshot(position: u64, duration: Option<u64>) -> PlayerSnapshot {
        PlayerSnapshot {
            position: Duration::from_secs(position),
            duration: duration.map(Duration::from_secs),
            ..Default::default()
        }
    }

    #[test]
    fn time_text_shows_total_or_remaining() {
        let playing = snapshot(42, Some(7087));
        assert_eq!(time_text(&playing, false), "0:42 / 1:58:07");
        assert_eq!(time_text(&playing, true), "0:42 / -1:57:25");
    }

    #[test]
    fn repeat_button_cycles_through_all_modes() {
        assert_eq!(next_repeat(RepeatMode::Off), RepeatMode::Playlist);
        assert_eq!(next_repeat(RepeatMode::Playlist), RepeatMode::File);
        assert_eq!(next_repeat(RepeatMode::File), RepeatMode::Off);
    }

    #[test]
    fn everything_fits_in_a_wide_bar() {
        let wide = plan(1400.0, 120.0);
        assert_eq!(wide.volume, VOLUME_MAX);
        assert_eq!(wide.toggles.len(), 5);
        assert!(wide.open);
    }

    #[test]
    fn the_minimum_window_keeps_transport_volume_and_the_fixed_buttons() {
        let width = crate::app::MIN_WINDOW_SIZE[0];
        let narrow = plan(width, 120.0);
        // No optional toggle, but still a usable volume slider.
        assert!(narrow.toggles.is_empty());
        assert!(narrow.volume >= VOLUME_MIN, "{narrow:?}");
        // The fixed right cluster and the slider fit beside the centred
        // transport group without touching it.
        let side = width / 2.0 - CENTER_WIDTH / 2.0 - CENTER_MARGIN - EDGE;
        assert!(RIGHT_FIXED + narrow.volume + 4.0 <= side);
        // And so does the longest time label on the left.
        assert!(120.0 <= side);
    }

    #[test]
    fn toggles_drop_least_important_first_and_never_flicker() {
        let mut previous = plan(500.0, 120.0);
        for width in (500..1500).step_by(5) {
            let current = plan(width as f32, 120.0);
            // Monotonic: more room never hides anything.
            assert!(current.toggles.len() >= previous.toggles.len(), "{width}");
            assert!(current.volume >= previous.volume, "{width}");
            // Always a prefix of the priority order.
            let order = [
                Toggle::Repeat,
                Toggle::Subtitles,
                Toggle::AbRepeat,
                Toggle::Shuffle,
                Toggle::Capture,
            ];
            assert_eq!(
                current.toggles[..],
                order[..current.toggles.len()],
                "{width}"
            );
            // The slider comes before any toggle.
            assert!(
                current.toggles.is_empty() || current.volume == VOLUME_MAX,
                "{width}"
            );
            previous = current;
        }
    }

    #[test]
    fn a_very_narrow_bar_drops_the_slider_but_not_the_buttons() {
        let tiny = plan(520.0, 120.0);
        assert_eq!(tiny.volume, 0.0);
        assert!(tiny.toggles.is_empty());
    }

    #[test]
    fn time_text_without_duration_and_past_the_end() {
        assert_eq!(time_text(&snapshot(0, None), true), "0:00 / --:--");
        // A position slightly past the reported duration must not underflow.
        assert_eq!(time_text(&snapshot(65, Some(60)), true), "1:05 / -0:00");
    }
}
