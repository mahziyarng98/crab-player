//! Playback backend abstraction.
//!
//! The UI never talks to libmpv directly. Everything goes through [`Player`],
//! which keeps the door open for a pure-Rust engine (alhazen-core) later.
//!
//! The trait is split the same way throughout: *intents* are methods
//! (`load`, `seek`, `select_track`, …) and *state* is read back through
//! [`PlayerSnapshot`], which also carries the track lists.

pub mod null;

#[cfg(feature = "mpv")]
pub mod mpv;
#[cfg(feature = "mpv")]
pub mod mpv_dl;
#[cfg(feature = "mpv")]
mod mpv_shim;

use std::ffi::{CStr, c_void};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Duration;

/// OpenGL symbol loader of the window's GL context, as handed out by eframe's
/// glow backend (`CreationContext::get_proc_address`).
pub type GetProcAddress = Arc<dyn Fn(&CStr) -> *const c_void + Send + Sync>;

/// What a backend needs from the windowing layer to composite video. Must be
/// built on the GL thread while the GL context is current.
#[cfg_attr(not(feature = "mpv"), allow(dead_code))] // Consumed by the mpv backend only.
pub struct VideoInit {
    /// `None` when the window has no GL context; playback is then audio-only.
    pub get_proc_address: Option<GetProcAddress>,
    /// Woken (from any thread) whenever a new video frame is ready or the
    /// backend's state changed.
    pub repaint: egui::Context,
    /// Display connection of the window, for hardware-decoder interop.
    pub native_display: Option<NativeDisplay>,
    /// `GL_RENDERER` of the context, used to recognise software rasterizers.
    pub gl_renderer: Option<String>,
}

/// Native display connection the window lives on. The pointer is owned by
/// the windowing layer and stays valid for the lifetime of the window.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(feature = "mpv"), allow(dead_code))]
pub enum NativeDisplay {
    /// `wl_display *`
    Wayland(NonNull<c_void>),
    /// Xlib `Display *`
    X11(NonNull<c_void>),
}

/// Handle the UI uses to draw video frames.
#[cfg(feature = "mpv")]
pub use mpv::VideoRenderer;

/// UI-only builds have no backend that produces frames.
#[cfg(not(feature = "mpv"))]
pub enum VideoRenderer {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)] // `Ended`/`Error` depend on the active backend.
pub enum PlaybackState {
    #[default]
    Idle,
    Opening,
    Playing,
    Paused,
    Stopped,
    Ended,
    Error,
}

impl PlaybackState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "No media",
            Self::Opening => "Opening…",
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
            Self::Ended => "Finished",
            Self::Error => "Error",
        }
    }
}

/// The three kinds of elementary stream a media file can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackKind {
    Audio,
    Subtitle,
    Video,
}

impl TrackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Subtitle => "Subtitle",
            Self::Video => "Video",
        }
    }
}

/// One selectable stream. `id` is the backend's own track number (unique per
/// [`TrackKind`], starting at 1) and is what [`Player::select_track`] takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub id: i64,
    pub title: String,
    pub language: Option<String>,
    pub codec: Option<String>,
    /// Flagged as the default stream by the container.
    pub default: bool,
    /// Currently being decoded and presented.
    pub selected: bool,
    /// Loaded from a separate file (for example an `.srt` next to the video).
    pub external: bool,
}

impl TrackInfo {
    /// Menu label: `#2  Commentary  [eng]  aac`, leaving out what is unknown.
    pub fn label(&self) -> String {
        let mut label = format!("#{}", self.id);
        if !self.title.is_empty() {
            label.push_str("  ");
            label.push_str(&self.title);
        }
        if let Some(language) = self.language.as_deref().filter(|l| !l.is_empty()) {
            label.push_str(&format!("  [{language}]"));
        }
        if let Some(codec) = self.codec.as_deref().filter(|c| !c.is_empty()) {
            label.push_str("  ");
            label.push_str(codec);
        }
        if self.external {
            label.push_str("  (external)");
        }
        label
    }
}

/// Track lists are shared, immutable slices: a snapshot is taken every frame
/// and must stay cheap to clone.
pub type Tracks = Arc<[TrackInfo]>;

/// Picture controls, each in −100..=100 with 0 as neutral (mpv's
/// `brightness`, `contrast`, `saturation` and `hue`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Picture {
    pub brightness: i32,
    pub contrast: i32,
    pub saturation: i32,
    pub hue: i32,
}

impl Picture {
    pub const RANGE: i32 = 100;

    pub fn is_neutral(self) -> bool {
        self == Self::default()
    }

    /// Every control clamped to `-RANGE..=RANGE`.
    pub fn clamped(self) -> Self {
        let c = |v: i32| v.clamp(-Self::RANGE, Self::RANGE);
        Self {
            brightness: c(self.brightness),
            contrast: c(self.contrast),
            saturation: c(self.saturation),
            hue: c(self.hue),
        }
    }
}

/// A-B repeat points. The loop is active once both are set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AbLoop {
    pub a: Option<Duration>,
    pub b: Option<Duration>,
}

impl AbLoop {
    pub fn is_active(self) -> bool {
        matches!((self.a, self.b), (Some(a), Some(b)) if a < b)
    }
}

/// Normalise a rotation to 0, 90, 180 or 270 degrees clockwise.
pub fn normalize_rotation(degrees: i32) -> u16 {
    // Snap to the nearest quarter turn, then wrap.
    let quarter = (f64::from(degrees) / 90.0).round() as i64;
    (quarter.rem_euclid(4) * 90) as u16
}

/// Track to switch to when cycling forwards through `tracks`. With
/// `allow_none` (subtitles) the cycle passes through "off" after the last
/// track. Returns `None` for "off"; an empty list always yields `None`.
pub fn next_track(tracks: &[TrackInfo], allow_none: bool) -> Option<i64> {
    let current = tracks.iter().position(|track| track.selected);
    let next = match current {
        Some(index) if index + 1 < tracks.len() => Some(index + 1),
        Some(_) if allow_none => None,
        Some(_) | None => (!tracks.is_empty()).then_some(0),
    };
    next.map(|index| tracks[index].id)
}

#[derive(Clone, Debug)]
pub struct PlayerSnapshot {
    pub state: PlaybackState,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
    pub muted: bool,
    pub speed: f64,
    pub title: Option<String>,
    pub media_path: Option<PathBuf>,
    pub video_size: Option<(u32, u32)>,
    pub audio_tracks: Tracks,
    pub subtitle_tracks: Tracks,
    pub video_tracks: Tracks,
    /// Seconds the audio is shifted against the video (positive = later).
    pub audio_delay: f64,
    /// Seconds the subtitles are shifted against the video (positive = later).
    pub subtitle_delay: f64,
    /// Name of the hardware decoder in use (`vaapi`, `nvdec-copy`, …), or
    /// `None` when the video is decoded in software or nothing is decoding.
    pub hardware_decoder: Option<String>,
    /// Display aspect ratio forced by the user; `None` uses the file's.
    pub aspect_override: Option<f64>,
    /// Extra clockwise rotation chosen by the user (0/90/180/270), on top
    /// of any rotation stored in the file.
    pub rotation: u16,
    pub ab_loop: AbLoop,
    pub picture: Picture,
    /// Hardware decoding is switched on (the decoder may still fall back to
    /// software for a given file; see `hardware_decoder`).
    pub hardware_decoding: bool,
    /// Hardware decoding cannot be switched on in this session (no backend
    /// support, or a software OpenGL renderer).
    pub hardware_decoding_locked: bool,
    /// Technical details of the open file, once the backend has been asked
    /// for them with [`Player::refresh_media_info`].
    pub media_info: Option<std::sync::Arc<MediaInfo>>,
}

/// What the Media Info dialog shows: the container and the streams that
/// are being decoded right now, as reported by the playback engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    /// Container (demuxer) name, e.g. `matroska,webm`.
    pub container: Option<String>,
    pub file_size: Option<u64>,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    /// Hardware decoder in use (`vaapi`, …); `None` means software.
    pub hardware_decoder: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoInfo {
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub pixel_format: Option<String>,
    pub size: Option<(u32, u32)>,
    pub fps: Option<f64>,
    /// Bits per second.
    pub bitrate: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioInfo {
    pub codec: Option<String>,
    pub profile: Option<String>,
    /// Channel layout as the engine names it (`stereo`, `5.1`, …).
    pub channels: Option<String>,
    pub sample_rate: Option<u32>,
    /// Bits per second.
    pub bitrate: Option<u64>,
}

impl MediaInfo {
    /// The dialog's rows: `(label, value)`, with `None` separating the
    /// sections. Unknown values are left out rather than shown as blanks.
    pub fn rows(&self) -> Vec<Option<(&'static str, String)>> {
        fn push(
            rows: &mut Vec<Option<(&'static str, String)>>,
            label: &'static str,
            value: Option<String>,
        ) {
            if let Some(value) = value {
                rows.push(Some((label, value)));
            }
        }

        let mut rows = Vec::new();
        push(&mut rows, "Container", self.container.clone());
        push(&mut rows, "File size", self.file_size.map(format_size));

        if let Some(video) = &self.video {
            rows.push(None);
            push(&mut rows, "Video codec", video.codec.clone());
            push(&mut rows, "Profile", video.profile.clone());
            push(&mut rows, "Pixel format", video.pixel_format.clone());
            push(
                &mut rows,
                "Resolution",
                video.size.map(|(w, h)| format!("{w} × {h}")),
            );
            push(&mut rows, "Frame rate", video.fps.map(format_fps));
            push(
                &mut rows,
                "Video bitrate",
                video.bitrate.map(format_bitrate),
            );
            rows.push(Some((
                "Decoder",
                match &self.hardware_decoder {
                    Some(name) => format!("Hardware ({name})"),
                    None => "Software".to_owned(),
                },
            )));
        }

        if let Some(audio) = &self.audio {
            rows.push(None);
            push(&mut rows, "Audio codec", audio.codec.clone());
            push(&mut rows, "Audio profile", audio.profile.clone());
            push(&mut rows, "Channels", audio.channels.clone());
            push(
                &mut rows,
                "Sample rate",
                audio.sample_rate.map(|rate| format!("{rate} Hz")),
            );
            push(
                &mut rows,
                "Audio bitrate",
                audio.bitrate.map(format_bitrate),
            );
        }
        rows
    }
}

/// `1234567` bits per second as `1.23 Mb/s`.
pub fn format_bitrate(bits_per_second: u64) -> String {
    if bits_per_second >= 1_000_000 {
        format!("{:.2} Mb/s", bits_per_second as f64 / 1_000_000.0)
    } else {
        format!("{:.0} kb/s", bits_per_second as f64 / 1000.0)
    }
}

/// A byte count in binary units.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `23.976` rather than `23.976023976`, `30` rather than `30.000`.
pub fn format_fps(fps: f64) -> String {
    let text = format!("{fps:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} fps")
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            state: PlaybackState::Idle,
            position: Duration::ZERO,
            duration: None,
            volume: 85.0,
            muted: false,
            speed: 1.0,
            title: None,
            media_path: None,
            video_size: None,
            audio_tracks: Tracks::default(),
            subtitle_tracks: Tracks::default(),
            video_tracks: Tracks::default(),
            audio_delay: 0.0,
            subtitle_delay: 0.0,
            hardware_decoder: None,
            aspect_override: None,
            rotation: 0,
            ab_loop: AbLoop::default(),
            picture: Picture::default(),
            hardware_decoding: false,
            hardware_decoding_locked: true,
            media_info: None,
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // `FileLoaded`/`Error` are for backends that report asynchronously.
pub enum PlayerEvent {
    FileLoaded { duration: Option<Duration> },
    EndOfFile,
    Error(String),
}

pub trait Player {
    fn load(&mut self, path: &std::path::Path, start_paused: bool) -> anyhow::Result<()>;
    fn play(&mut self) -> anyhow::Result<()>;
    fn pause(&mut self) -> anyhow::Result<()>;
    fn toggle_pause(&mut self) -> anyhow::Result<()>;
    fn stop(&mut self) -> anyhow::Result<()>;
    /// Exact seek to an absolute position.
    fn seek(&mut self, position: Duration) -> anyhow::Result<()>;
    /// Fast, approximate seek used while the seek bar is being dragged; the
    /// drag ends with an exact [`Self::seek`]. Backends without a cheaper
    /// mode can rely on this default.
    fn scrub(&mut self, position: Duration) -> anyhow::Result<()> {
        self.seek(position)
    }
    fn set_volume(&mut self, volume: f32) -> anyhow::Result<()>;
    fn set_muted(&mut self, muted: bool) -> anyhow::Result<()>;
    fn set_speed(&mut self, speed: f64) -> anyhow::Result<()>;
    /// Switch the active track of `kind` to the one with this
    /// [`TrackInfo::id`], or turn that kind off with `None` (the usual way
    /// to disable subtitles). The track lists in the next snapshot reflect
    /// the change.
    fn select_track(&mut self, kind: TrackKind, id: Option<i64>) -> anyhow::Result<()>;
    /// Load a subtitle file next to the current media and select it.
    fn add_subtitle_file(&mut self, path: &std::path::Path) -> anyhow::Result<()>;
    /// Shift the audio against the video; positive values delay the audio.
    fn set_audio_delay(&mut self, seconds: f64) -> anyhow::Result<()>;
    /// Shift the subtitles against the video; positive values delay them.
    fn set_subtitle_delay(&mut self, seconds: f64) -> anyhow::Result<()>;
    /// Force a display aspect ratio (width / height), or `None` for the
    /// file's own.
    fn set_aspect_override(&mut self, ratio: Option<f64>) -> anyhow::Result<()>;
    /// Extra clockwise rotation in degrees; normalised to quarter turns.
    fn set_rotation(&mut self, degrees: i32) -> anyhow::Result<()>;
    /// Loop between `a` and `b`; clearing either point ends the loop.
    fn set_ab_loop(&mut self, ab: AbLoop) -> anyhow::Result<()>;
    fn set_picture(&mut self, picture: Picture) -> anyhow::Result<()>;
    /// Save the current video frame (with subtitles) as an image file. The
    /// format follows the extension (`.png`, `.jpg`, `.webp`).
    fn screenshot(&mut self, path: &std::path::Path) -> anyhow::Result<()>;
    /// Switch hardware decoding on (the configured mode) or off at runtime.
    fn set_hardware_decoding(&mut self, enabled: bool) -> anyhow::Result<()>;
    /// Ask the backend to read the technical details of the open file
    /// again; they appear in [`PlayerSnapshot::media_info`] shortly after.
    /// Bitrates change while playing, so the dialog asks repeatedly.
    fn refresh_media_info(&mut self) {}
    fn snapshot(&mut self) -> PlayerSnapshot;
    fn drain_events(&mut self) -> Vec<PlayerEvent>;
    fn backend_name(&self) -> &'static str;
}

/// Create the best available backend, falling back to a no-op player so the UI
/// still starts on machines without libmpv. The second value is the video
/// renderer, present only when the backend could attach to the GL context.
pub fn create(
    config: &crate::config::PlaybackConfig,
    video: VideoInit,
) -> (Box<dyn Player>, Option<VideoRenderer>) {
    // Silences the "unused variable" warnings in UI-only builds.
    let _ = (config, &video);

    #[cfg(feature = "mpv")]
    if let Err(reason) = mpv_dl::status() {
        tracing::error!(%reason, "libmpv could not be loaded; using the null backend");
        return (Box::new(null::NullPlayer::new()), None);
    }

    #[cfg(feature = "mpv")]
    match mpv::MpvPlayer::new(config, video.repaint.clone()) {
        Ok(mut player) => {
            crate::startup::mark("mpv core created");
            // No GL context, or a context mpv cannot use: keep playing audio.
            let renderer = player
                .attach_video(video)
                .inspect_err(|err| {
                    tracing::error!(
                        err = format!("{err:#}"),
                        "video output unavailable; audio only"
                    );
                })
                .ok();
            return (Box::new(player), renderer);
        }
        Err(err) => {
            tracing::error!(%err, "libmpv unavailable; using the null backend");
        }
    }

    (Box::new(null::NullPlayer::new()), None)
}

/// Why the real playback engine is not available, for the error banner;
/// `None` when it is (or in UI-only builds, which never have it).
pub fn engine_unavailable() -> Option<String> {
    #[cfg(feature = "mpv")]
    if let Err(reason) = mpv_dl::status() {
        return Some(reason);
    }
    None
}

pub fn format_time(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_info_values_are_formatted_for_reading() {
        assert_eq!(format_bitrate(128_000), "128 kb/s");
        assert_eq!(format_bitrate(4_567_000), "4.57 Mb/s");
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(3_800_000), "3.6 MiB");
        assert_eq!(format_size(2_747_535_443), "2.6 GiB");
        assert_eq!(format_fps(23.976_023_976), "23.976 fps");
        assert_eq!(format_fps(30.0), "30 fps");
        assert_eq!(format_fps(59.94), "59.94 fps");
    }

    #[test]
    fn media_info_rows_skip_unknown_values_and_name_the_decoder() {
        let info = MediaInfo {
            container: Some("matroska,webm".to_owned()),
            file_size: None,
            video: Some(VideoInfo {
                codec: Some("hevc".to_owned()),
                profile: Some("Main 10".to_owned()),
                pixel_format: Some("yuv420p10".to_owned()),
                size: Some((1920, 1080)),
                fps: Some(23.976),
                bitrate: None,
            }),
            audio: Some(AudioInfo {
                codec: Some("aac".to_owned()),
                channels: Some("5.1".to_owned()),
                sample_rate: Some(48_000),
                ..Default::default()
            }),
            hardware_decoder: Some("vaapi".to_owned()),
        };
        let rows = info.rows();
        let labels: Vec<&str> = rows
            .iter()
            .map(|row| row.as_ref().map_or("—", |r| r.0))
            .collect();
        assert_eq!(
            labels,
            [
                "Container",
                "—",
                "Video codec",
                "Profile",
                "Pixel format",
                "Resolution",
                "Frame rate",
                "Decoder",
                "—",
                "Audio codec",
                "Channels",
                "Sample rate"
            ]
        );
        let value = |label: &str| {
            rows.iter()
                .flatten()
                .find(|row| row.0 == label)
                .map(|row| row.1.clone())
        };
        assert_eq!(value("Decoder").as_deref(), Some("Hardware (vaapi)"));
        assert_eq!(value("Resolution").as_deref(), Some("1920 × 1080"));
        assert_eq!(value("Sample rate").as_deref(), Some("48000 Hz"));

        let software = MediaInfo {
            video: Some(VideoInfo::default()),
            ..Default::default()
        };
        assert_eq!(
            software.rows(),
            vec![None, Some(("Decoder", "Software".to_owned()))]
        );
        // Audio only: no decoder row at all.
        assert!(MediaInfo::default().rows().is_empty());
    }

    fn track(id: i64, selected: bool) -> TrackInfo {
        TrackInfo {
            id,
            selected,
            ..Default::default()
        }
    }

    #[test]
    fn label_leaves_out_unknown_fields() {
        let full = TrackInfo {
            id: 2,
            title: "Commentary".to_owned(),
            language: Some("eng".to_owned()),
            codec: Some("aac".to_owned()),
            external: true,
            ..Default::default()
        };
        assert_eq!(full.label(), "#2  Commentary  [eng]  aac  (external)");
        assert_eq!(track(1, false).label(), "#1");
    }

    #[test]
    fn next_track_cycles_and_optionally_passes_through_off() {
        let none_selected = [track(1, false), track(2, false)];
        let first = [track(1, true), track(2, false)];
        let last = [track(1, false), track(2, true)];

        assert_eq!(next_track(&none_selected, true), Some(1));
        assert_eq!(next_track(&first, true), Some(2));
        // Subtitles: after the last track comes "off".
        assert_eq!(next_track(&last, true), None);
        // Audio: wraps straight back to the first track.
        assert_eq!(next_track(&last, false), Some(1));
        assert_eq!(next_track(&[], true), None);
        assert_eq!(next_track(&[], false), None);
    }

    /// `create` with whatever the windowing layer could (not) provide.
    #[cfg(feature = "mpv")]
    fn create_with(loader: Option<GetProcAddress>) -> (Box<dyn Player>, Option<VideoRenderer>) {
        // Shows the logged error with `cargo test -- --nocapture`.
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        create(
            &crate::config::PlaybackConfig::default(),
            VideoInit {
                get_proc_address: loader,
                repaint: egui::Context::default(),
                native_display: None,
                gl_renderer: None,
            },
        )
    }

    /// The window has no OpenGL context at all (eframe hands out `None`).
    #[cfg(feature = "mpv")]
    #[test]
    fn missing_gl_context_falls_back_to_audio_only() {
        let (mut player, renderer) = create_with(None);
        assert!(renderer.is_none(), "no render context without a GL context");
        // The backend itself is intact and keeps accepting intents.
        assert_eq!(player.backend_name(), "libmpv");
        assert_eq!(player.snapshot().state, PlaybackState::Idle);
        assert!(player.set_volume(50.0).is_ok());
        assert_eq!(player.snapshot().volume, 50.0);
    }

    /// There is a loader, but it cannot resolve a single GL symbol (a broken
    /// or lost context). mpv must refuse cleanly instead of crashing.
    #[cfg(feature = "mpv")]
    #[test]
    fn unusable_gl_context_falls_back_to_audio_only() {
        let broken: GetProcAddress = Arc::new(|_symbol| std::ptr::null());
        let (mut player, renderer) = create_with(Some(broken));
        assert!(
            renderer.is_none(),
            "mpv must reject a context without symbols"
        );
        assert_eq!(player.backend_name(), "libmpv");
        assert_eq!(player.snapshot().state, PlaybackState::Idle);
    }

    #[test]
    fn rotation_snaps_to_quarter_turns() {
        assert_eq!(normalize_rotation(0), 0);
        assert_eq!(normalize_rotation(90), 90);
        assert_eq!(normalize_rotation(-90), 270);
        assert_eq!(normalize_rotation(360 + 180), 180);
        assert_eq!(normalize_rotation(100), 90);
        assert_eq!(normalize_rotation(-450), 270);
    }

    #[test]
    fn picture_is_clamped_and_neutral_by_default() {
        assert!(Picture::default().is_neutral());
        let wild = Picture {
            brightness: 500,
            contrast: -500,
            saturation: 10,
            hue: 0,
        };
        assert_eq!(
            wild.clamped(),
            Picture {
                brightness: 100,
                contrast: -100,
                saturation: 10,
                hue: 0
            }
        );
    }

    #[test]
    fn ab_loop_needs_both_points_in_order() {
        let s = Duration::from_secs;
        assert!(!AbLoop::default().is_active());
        assert!(
            !AbLoop {
                a: Some(s(5)),
                b: None
            }
            .is_active()
        );
        assert!(
            AbLoop {
                a: Some(s(5)),
                b: Some(s(9))
            }
            .is_active()
        );
        assert!(
            !AbLoop {
                a: Some(s(9)),
                b: Some(s(5))
            }
            .is_active()
        );
    }

    #[test]
    fn format_time_switches_to_hours() {
        assert_eq!(format_time(Duration::from_secs(59)), "0:59");
        assert_eq!(format_time(Duration::from_secs(3723)), "1:02:03");
    }
}
