pub mod caption;
pub mod control_bar;
pub mod icons;
pub mod logo;
pub mod menu;
pub mod notice;
pub mod playlist_panel;
pub mod seek_bar;
pub mod theme;
pub mod video;

use std::time::{Duration, Instant};

pub use theme::{install_theme, palette};

use crate::command::{AfterPlayback, Preset, RepeatMode};

#[derive(Default)]
pub struct UiState {
    pub show_menu_bar: bool,
    pub show_seek_bar: bool,
    pub show_controls: bool,
    pub show_playlist: bool,
    pub show_goto: bool,
    /// Text being edited in the Go To dialog.
    pub goto_text: String,
    /// Set for one frame so the Go To field grabs keyboard focus.
    pub goto_focus: bool,
    pub show_options: bool,
    pub show_about: bool,
    pub show_properties: bool,
    pub show_statistics: bool,
    pub fullscreen: bool,
    pub controls_visible: bool,
    /// Time display shows the remaining time instead of the total.
    pub show_remaining: bool,
    pub repeat: RepeatMode,
    pub after_playback: AfterPlayback,
    pub osd: Option<(String, Instant)>,
    /// Playback order is random (the shuffle button).
    pub shuffle: bool,
    /// Renderer Settings ▸ Display current time / file name.
    pub show_clock: bool,
    pub show_file_name: bool,
    /// File ▸ Open URL… dialog.
    pub show_open_url: bool,
    pub url_text: String,
    pub url_focus: bool,
    /// Size of the video canvas in points, from the last frame (for the
    /// zoom-to-window commands).
    pub canvas_size: Option<egui::Vec2>,
    /// The error banner over the video; see [`UiState::error`].
    pub notice: Option<Notice>,
    /// A drag carrying files is over the window.
    pub drop_hover: bool,
    /// Vertical extent of the seek bar in the last frame, in points.
    pub seek_band: Option<std::ops::RangeInclusive<f32>>,
}

/// A problem the user has to know about (a file that would not open, a
/// decoder error). Shown as a banner that does not block anything and goes
/// away by itself or when dismissed.
pub struct Notice {
    pub title: String,
    /// What it is about, usually a file name. May be empty.
    pub detail: String,
    /// Further problems reported while this one was on screen.
    pub more: usize,
    pub since: Instant,
}

impl Notice {
    /// How long a banner stays up when it is not dismissed.
    pub const LIFETIME: Duration = Duration::from_secs(12);
}

impl UiState {
    /// Start from the persisted layout.
    pub fn new(config: &crate::config::UiConfig) -> Self {
        Self {
            show_menu_bar: config.show_menu_bar,
            show_seek_bar: config.show_seek_bar,
            show_controls: config.show_controls,
            show_playlist: config.show_playlist,
            show_remaining: config.show_remaining_time,
            controls_visible: true,
            ..Self::default()
        }
    }

    /// Write the layout back so it is saved with the config on exit.
    pub fn store(&self, config: &mut crate::config::UiConfig) {
        config.show_menu_bar = self.show_menu_bar;
        config.show_seek_bar = self.show_seek_bar;
        config.show_controls = self.show_controls;
        config.show_playlist = self.show_playlist;
        config.show_remaining_time = self.show_remaining;
    }

    /// Presets are layout shortcuts: they only set the panel flags.
    ///
    /// The spec table lists Compact and Normal with identical panels; Compact
    /// keeps the Phase 1 behaviour of dropping the playlist so the two differ.
    pub fn apply_preset(&mut self, preset: Preset) {
        let (menu, seek, playlist) = match preset {
            Preset::Minimal => (false, false, false),
            Preset::Compact => (true, true, false),
            Preset::Normal => (true, true, true),
        };
        self.show_menu_bar = menu;
        self.show_seek_bar = seek;
        self.show_controls = true;
        self.show_playlist = playlist;
    }

    /// True while a dialog window is open; chrome must not auto-hide then.
    pub fn any_window_open(&self) -> bool {
        self.show_options
            || self.show_about
            || self.show_properties
            || self.show_goto
            || self.show_open_url
    }

    /// Report a problem. Unlike [`UiState::flash`] this stays readable for
    /// a while and names the reason; it never takes focus or blocks input.
    pub fn error(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        // Several failures at once (a multi-file drop): show the first and
        // count the rest, instead of flickering through all of them.
        if let Some(notice) = &mut self.notice
            && notice.since.elapsed() < Duration::from_millis(1500)
        {
            notice.more += 1;
            return;
        }
        self.notice = Some(Notice {
            title: title.into(),
            detail: detail.into(),
            more: 0,
            since: Instant::now(),
        });
    }

    /// The banner to draw, if one is still current.
    pub fn current_notice(&mut self) -> Option<&Notice> {
        if self
            .notice
            .as_ref()
            .is_some_and(|notice| notice.since.elapsed() >= Notice::LIFETIME)
        {
            self.notice = None;
        }
        self.notice.as_ref()
    }

    pub fn flash(&mut self, text: impl Into<String>) {
        self.osd = Some((text.into(), Instant::now()));
    }

    pub fn osd_text(&self) -> Option<&str> {
        self.osd
            .as_ref()
            .filter(|(_, at)| at.elapsed().as_secs_f32() < 1.6)
            .map(|(text, _)| text.as_str())
    }
}

/// Dark, low-chrome theme: near-black canvas, neutral chrome, one accent.
pub fn dimmed_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .size(12.0)
            .color(palette::TEXT_WEAK),
    );
}

/// Whole wheel notches scrolled this frame: positive is "wheel up".
///
/// egui's `smooth_scroll_delta` spreads one notch over several frames, which
/// would turn a single click of the wheel into several volume steps, so the
/// raw wheel events are counted instead. Touchpads report in points; those
/// only count once the movement is clearly intentional.
pub fn wheel_notches(ctx: &egui::Context) -> i32 {
    ctx.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => Some((*unit, delta.y)),
                _ => None,
            })
            .map(|(unit, dy)| notch(unit, dy))
            .sum()
    })
}

fn notch(unit: egui::MouseWheelUnit, dy: f32) -> i32 {
    let threshold = match unit {
        egui::MouseWheelUnit::Point => 8.0,
        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => 0.0,
    };
    if dy > threshold {
        1
    } else if dy < -threshold {
        -1
    } else {
        0
    }
}

/// Parse what a user types into "Go To": plain seconds (`90`, `12.5`),
/// `m:ss` or `h:mm:ss`. Minutes and seconds may exceed 59 only in the
/// leading field, as on a stopwatch.
pub fn parse_timecode(text: &str) -> Option<Duration> {
    let fields: Vec<&str> = text.trim().split(':').map(str::trim).collect();
    if fields.is_empty() || fields.len() > 3 || fields.iter().any(|f| f.is_empty()) {
        return None;
    }

    let (last, leading) = fields.split_last()?;
    let seconds: f64 = last.parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 || (!leading.is_empty() && seconds >= 60.0) {
        return None;
    }

    let mut total = 0u64;
    for (index, field) in leading.iter().enumerate() {
        let value: u64 = field.parse().ok()?;
        if index > 0 && value >= 60 {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(value)?;
    }
    let total = total.checked_mul(60)? as f64 + seconds;
    Duration::try_from_secs_f64(total).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_accepts_seconds_and_clock_forms() {
        assert_eq!(parse_timecode("90"), Some(Duration::from_secs(90)));
        assert_eq!(parse_timecode(" 1:30 "), Some(Duration::from_secs(90)));
        assert_eq!(parse_timecode("1:02:03"), Some(Duration::from_secs(3723)));
        assert_eq!(parse_timecode("75:00"), Some(Duration::from_secs(4500)));
        assert_eq!(
            parse_timecode("0:12.5"),
            Some(Duration::from_millis(12_500))
        );
    }

    #[test]
    fn timecode_rejects_garbage() {
        for bad in [
            "", ":", "1:", ":30", "1:60", "1:75:00", "-5", "abc", "1:2:3:4", "nan", "inf",
        ] {
            assert_eq!(parse_timecode(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn wheel_notches_ignore_touchpad_jitter() {
        use egui::MouseWheelUnit::{Line, Point};
        assert_eq!(notch(Line, 1.0), 1);
        assert_eq!(notch(Line, -3.0), -1);
        assert_eq!(notch(Line, 0.0), 0);
        assert_eq!(notch(Point, 2.0), 0);
        assert_eq!(notch(Point, -20.0), -1);
    }

    #[test]
    fn presets_map_to_panel_flags() {
        let mut state = UiState::new(&crate::config::UiConfig::default());
        assert!(state.show_menu_bar && state.show_seek_bar && state.show_controls);
        state.apply_preset(Preset::Minimal);
        assert!(!state.show_menu_bar && !state.show_seek_bar && !state.show_playlist);
        assert!(state.show_controls);

        state.apply_preset(Preset::Normal);
        assert!(state.show_menu_bar && state.show_seek_bar && state.show_playlist);
    }

    #[test]
    fn layout_is_restored_from_and_stored_to_the_config() {
        let mut config = crate::config::UiConfig {
            show_menu_bar: false,
            show_remaining_time: true,
            ..Default::default()
        };
        let mut state = UiState::new(&config);
        assert!(!state.show_menu_bar && state.show_remaining && state.show_seek_bar);

        state.apply_preset(Preset::Compact);
        state.show_controls = false;
        state.store(&mut config);
        assert!(config.show_menu_bar && !config.show_playlist && !config.show_controls);
    }
}
