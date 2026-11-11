//! libmpv backend.
//!
//! libmpv does demuxing, decoding (VA-API/NVDEC through `hwdec=auto-safe`),
//! audio output, subtitles and network streams. Video goes through the mpv
//! render API (`vo=libmpv`): [`MpvPlayer::attach_video`] creates an
//! `mpv_render_context` on the GL thread and hands out a [`VideoRenderer`],
//! which `ui::video` drives from an egui paint callback.
//!
//! # Threads
//!
//! The UI thread is mpv's *render thread*, and libmpv forbids a render
//! thread from calling its synchronous client API (`mpv_command`,
//! `mpv_get_property`, …): the core may be waiting for a frame to be
//! rendered at that very moment, and the two would only be released by a
//! 200 ms timeout. The backend is therefore split in two:
//!
//! - [`MpvPlayer`] (UI thread) never talks to the mpv core. Intents are sent
//!   as [`Request`]s over a channel; state is read from a mutex-protected
//!   [`State`]. The only libmpv calls on this thread are `mpv_render_*`.
//! - The **control thread** owns all client-API traffic: it executes the
//!   requests and folds mpv's events and observed properties into `State`.
//!   It sleeps in `recv()` and is woken by a request or by mpv's wakeup
//!   callback, so it costs nothing while idle.
//!
//! The control thread never holds the state lock across an mpv call, which
//! is what keeps the UI thread free of any wait on the mpv core.
//!
//! All `unsafe` in this file is confined to the [`GlBound`] wrapper and the
//! lifetime erasure in [`MpvPlayer::attach_video`].

use std::ffi::{CString, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use anyhow::{Context, Result};
use libmpv2::events::{Event, PropertyData};
use libmpv2::render::{
    OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType, mpv_render_update,
};
use libmpv2::{Format, Mpv};

use super::{
    AbLoop, AudioInfo, GetProcAddress, MediaInfo, NativeDisplay, Picture, PlaybackState, Player,
    PlayerEvent, PlayerSnapshot, TrackInfo, TrackKind, Tracks, VideoInfo, VideoInit,
};

/// Environment variable with extra mpv options for diagnostics, as a
/// comma-separated `name=value` list, e.g.
/// `CRAB_PLAYER_MPV_OPTIONS="log-file=/tmp/mpv.log,msg-level=all=v,ao=null"`.
pub const MPV_OPTIONS_ENV: &str = "CRAB_PLAYER_MPV_OPTIONS";

/// Split [`MPV_OPTIONS_ENV`] into `(name, value)` pairs. Entries without a
/// name or `=` are dropped; `vo` is refused because the render API depends
/// on `vo=libmpv`.
fn parse_extra_options(text: &str) -> Vec<(String, String)> {
    text.split(',')
        .filter_map(|entry| entry.split_once('='))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .filter(|(name, _)| !name.is_empty() && name != "vo")
        .collect()
}

/// Delays are clamped to what is still plausibly a sync correction.
const MAX_DELAY_SECONDS: f64 = 600.0;

/// mpv's `track-list`, split by kind.
#[derive(Debug, Default, PartialEq, Eq)]
struct TrackLists {
    audio: Vec<TrackInfo>,
    subtitle: Vec<TrackInfo>,
    video: Vec<TrackInfo>,
}

/// Build the track lists from mpv's `track-list/N/<field>` sub-properties.
///
/// `field(index, name)` returns the property the way mpv prints it as a
/// string (`yes`/`no` for flags, decimal for numbers) or `None` when it is
/// unavailable. Entries without a usable id or with an unknown type are
/// skipped rather than guessed at.
fn parse_track_list(count: usize, field: impl Fn(usize, &str) -> Option<String>) -> TrackLists {
    let mut lists = TrackLists::default();
    for index in 0..count {
        let text = |name: &str| field(index, name).filter(|value| !value.is_empty());
        let flag = |name: &str| field(index, name).as_deref() == Some("yes");

        let Some(id) = text("id").and_then(|id| id.parse::<i64>().ok()) else {
            continue;
        };
        let list = match text("type").as_deref() {
            Some("audio") => &mut lists.audio,
            Some("sub") => &mut lists.subtitle,
            Some("video") => &mut lists.video,
            _ => continue,
        };
        list.push(TrackInfo {
            id,
            title: text("title").unwrap_or_default(),
            language: text("lang"),
            codec: text("codec"),
            default: flag("default"),
            selected: flag("selected"),
            external: flag("external"),
        });
    }
    lists
}

/// mpv property that selects the active track of a kind.
fn track_property(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "aid",
        TrackKind::Subtitle => "sid",
        TrackKind::Video => "vid",
    }
}

/// Value for [`track_property`]: the track id, or `no` to disable the kind.
fn track_value(id: Option<i64>) -> String {
    id.map_or_else(|| "no".to_owned(), |id| id.to_string())
}

/// `hwdec-current` is `no` (or empty/unavailable) while decoding in software.
fn hardware_decoder(hwdec_current: Option<&str>) -> Option<String> {
    hwdec_current
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "no")
        .map(str::to_owned)
}

/// mpv's `MPV_ERROR_*` codes that describe why a file ended with an error.
mod code {
    pub const LOADING_FAILED: i32 = -13;
    pub const AO_INIT_FAILED: i32 = -14;
    pub const VO_INIT_FAILED: i32 = -15;
    pub const NOTHING_TO_PLAY: i32 = -16;
    pub const UNKNOWN_FORMAT: i32 = -17;
    pub const UNSUPPORTED: i32 = -18;
}

/// Why a file could not be played, in words a user can act on.
fn failure_reason(error: i32) -> &'static str {
    match error {
        code::LOADING_FAILED => "The file could not be opened or read",
        code::NOTHING_TO_PLAY => "No audio or video stream was found",
        code::UNKNOWN_FORMAT => "Unrecognised file format",
        code::UNSUPPORTED => "This format is not supported by the installed FFmpeg",
        code::AO_INIT_FAILED => "The audio output could not be started",
        code::VO_INIT_FAILED => "The video output could not be started",
        _ => "Playback failed: decoder error",
    }
}

/// Whether a failed load is worth one more attempt with software decoding.
/// Not when the file could not be opened or recognised at all: no decoder
/// was involved then, and the second attempt would only double the wait.
fn retry_in_software(error: i32, hwdec: &str) -> bool {
    hwdec_enabled(hwdec) && !matches!(error, code::LOADING_FAILED | code::UNKNOWN_FORMAT)
}

/// `video-aspect-override` value: a ratio, or `no` for the file's own.
fn aspect_value(ratio: Option<f64>) -> Result<String> {
    match ratio {
        None => Ok("no".to_owned()),
        Some(ratio) if ratio.is_finite() && ratio > 0.0 => Ok(format!("{ratio:.6}")),
        Some(ratio) => anyhow::bail!("invalid aspect ratio {ratio}"),
    }
}

/// `ab-loop-a` / `ab-loop-b` value: seconds, or `no` when unset.
fn ab_value(point: Option<Duration>) -> String {
    point.map_or_else(|| "no".to_owned(), |at| format!("{:.3}", at.as_secs_f64()))
}

/// True when an NVIDIA kernel driver is loaded, which is the precondition
/// for NVDEC/CUDA to work at all.
fn nvidia_driver_present() -> bool {
    [
        "/sys/module/nvidia",
        "/proc/driver/nvidia",
        "/dev/nvidiactl",
    ]
    .iter()
    .any(|path| Path::new(path).exists())
}

/// Value for mpv's `gpu-hwdec-interop`, or `None` to leave mpv's default
/// (probe every interop it was built with).
///
/// Creating the render context probes all interops on the UI thread. The
/// CUDA one calls `cuInit`, which on a machine that has CUDA libraries but
/// no NVIDIA driver runs `nvidia-modprobe` and takes 1–2 s before failing.
/// Without the driver CUDA cannot work, so only VA-API is loaded there.
/// With the driver present nothing is restricted, and NVDEC keeps working.
/// Limited to x86: ARM boards rely on the `drmprime` interop instead.
fn hwdec_interop(nvidia_present: bool, x86: bool) -> Option<&'static str> {
    (x86 && !nvidia_present).then_some("vaapi")
}

/// Whether an mpv `hwdec` mode asks for hardware decoding at all.
fn hwdec_enabled(mode: &str) -> bool {
    !matches!(mode.trim(), "" | "no")
}

fn clamp_delay(seconds: f64) -> Result<f64> {
    anyhow::ensure!(seconds.is_finite(), "delay must be a finite number");
    Ok(seconds.clamp(-MAX_DELAY_SECONDS, MAX_DELAY_SECONDS))
}

/// An `mpv_render_context` pinned to the thread that created it.
///
/// libmpv requires every `mpv_render_*` call, including the final free, to
/// happen on the thread where the GL context is current. `RenderContext` is
/// neither `Send` nor `Sync`, but egui's paint callback must be both, so this
/// wrapper asserts them and enforces the thread rule at runtime instead.
struct GlBound {
    context: Option<RenderContext<'static>>,
    owner: ThreadId,
}

// SAFETY: the inner `RenderContext` is only ever reachable through
// `GlBound::get`, which returns `None` on any thread other than the one that
// created it, and `Drop` leaks rather than frees it on a foreign thread. The
// value is therefore never *used* off its owner thread; moving or sharing the
// inert pointer itself is harmless.
unsafe impl Send for GlBound {}
// SAFETY: see `Send` above; every access is thread-checked, and the wrapper
// additionally lives behind a `Mutex`.
unsafe impl Sync for GlBound {}

impl GlBound {
    fn get(&mut self) -> Option<&mut RenderContext<'static>> {
        if thread::current().id() == self.owner {
            self.context.as_mut()
        } else {
            None
        }
    }
}

impl Drop for GlBound {
    fn drop(&mut self) {
        if thread::current().id() != self.owner
            && let Some(context) = self.context.take()
        {
            // Freeing off the GL thread is undefined behaviour in libmpv;
            // leaking is the only safe option.
            tracing::error!("mpv render context dropped off the GL thread; leaking it");
            std::mem::forget(context);
        }
    }
}

struct RendererShared {
    // Field order is drop order: the render context must be freed before the
    // last reference to the mpv handle goes away.
    slot: Mutex<Option<GlBound>>,
    /// Display size published by the control thread.
    shared: Arc<Shared>,
    /// Keeps the handle alive for as long as a render context can exist.
    _mpv: Arc<Mpv>,
}

/// Cheap, clonable handle to the mpv render context.
///
/// It is the only thing the UI layer needs to composite video: it does not
/// expose libmpv types, and every method is a no-op once [`Self::release`]
/// has run.
#[derive(Clone)]
pub struct VideoRenderer {
    shared: Arc<RendererShared>,
}

impl VideoRenderer {
    fn slot(&self) -> MutexGuard<'_, Option<GlBound>> {
        // A poisoned lock only means a paint callback panicked; the slot
        // itself is still a valid `Option`.
        self.shared
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Render the current video frame into `fbo` (`width` × `height` physical
    /// pixels) if mpv reports a new frame, or unconditionally when `force` is
    /// set (the target was just (re)created and holds no picture yet).
    ///
    /// Returns `Ok(true)` when the target now holds a fresh frame. Must be
    /// called on the GL thread with the GL context current; mpv changes GL
    /// state, which the caller has to reset afterwards.
    pub fn render(
        &self,
        fbo: i32,
        width: i32,
        height: i32,
        flip_y: bool,
        force: bool,
    ) -> Result<bool> {
        let mut slot = self.slot();
        let Some(context) = slot.as_mut().and_then(GlBound::get) else {
            return Ok(false);
        };

        let flags = context
            .update()
            .context("mpv_render_context_update failed")?;
        if !force && flags & mpv_render_update::Frame == 0 {
            return Ok(false);
        }

        context
            .render::<()>(fbo, width, height, flip_y)
            .context("mpv_render_context_render failed")?;
        Ok(true)
    }

    /// Size of the video as it should be displayed (pixel aspect ratio and
    /// rotation applied), or `None` when nothing with a picture is loaded.
    pub fn display_size(&self) -> Option<(u32, u32)> {
        let packed = self.shared.shared.display_size.load(Ordering::Relaxed);
        let (width, height) = ((packed >> 32) as u32, packed as u32);
        (width > 0 && height > 0).then_some((width, height))
    }

    /// Free the mpv render context. Must run on the GL thread with the GL
    /// context current, before the mpv handle is destroyed. Idempotent.
    pub fn release(&self) {
        // Take it out first so the lock is not held while mpv tears down.
        let bound = self.slot().take();
        drop(bound);
    }
}

/// Software rasterizers cannot import hardware-decoded surfaces: mpv's
/// VA-API interop "succeeds" on llvmpipe and then shows a black picture.
/// Matching the `GL_RENDERER` string is how such a context is recognised.
fn is_software_renderer(gl_renderer: &str) -> bool {
    let name = gl_renderer.to_ascii_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swrast",
        "software rasterizer",
        "swiftshader",
        "lavapipe",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}

// ---------------------------------------------------------------- state

/// Properties the control thread observes. The discriminant is the
/// `reply_userdata` tag passed to `mpv_observe_property`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observed {
    TrackList = 1,
    HwdecCurrent,
    TimePos,
    Duration,
    EofReached,
    Pause,
    Width,
    Height,
    DisplayWidth,
    DisplayHeight,
    Rotate,
    MediaTitle,
}

impl Observed {
    const ALL: [Self; 12] = [
        Self::TrackList,
        Self::HwdecCurrent,
        Self::TimePos,
        Self::Duration,
        Self::EofReached,
        Self::Pause,
        Self::Width,
        Self::Height,
        Self::DisplayWidth,
        Self::DisplayHeight,
        Self::Rotate,
        Self::MediaTitle,
    ];

    fn from_id(id: u64) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|property| *property as u64 == id)
    }

    /// mpv property name and the format it is observed in. Node-typed
    /// properties are observed as strings: libmpv2 cannot decode node change
    /// events, and for `track-list` only the notification matters anyway.
    fn spec(self) -> (&'static str, Format) {
        match self {
            Self::TrackList => ("track-list", Format::String),
            Self::HwdecCurrent => ("hwdec-current", Format::String),
            Self::TimePos => ("time-pos", Format::Double),
            Self::Duration => ("duration", Format::Double),
            Self::EofReached => ("eof-reached", Format::Flag),
            Self::Pause => ("pause", Format::Flag),
            Self::Width => ("width", Format::Int64),
            Self::Height => ("height", Format::Int64),
            Self::DisplayWidth => ("dwidth", Format::Int64),
            Self::DisplayHeight => ("dheight", Format::Int64),
            Self::Rotate => ("video-params/rotate", Format::Int64),
            Self::MediaTitle => ("media-title", Format::String),
        }
    }
}

/// Owned copy of an observed property value (mpv's event data is borrowed).
#[derive(Clone, Debug, PartialEq)]
enum Value {
    /// The property is currently unavailable.
    None,
    Flag(bool),
    Int(i64),
    Double(f64),
    Text(String),
}

impl From<PropertyData<'_>> for Value {
    fn from(data: PropertyData<'_>) -> Self {
        match data {
            PropertyData::Flag(flag) => Self::Flag(flag),
            PropertyData::Int64(int) => Self::Int(int),
            PropertyData::Double(double) => Self::Double(double),
            PropertyData::Str(text) | PropertyData::OsdStr(text) => Self::Text(text.to_owned()),
            PropertyData::None => Self::None,
        }
    }
}

impl Value {
    fn int(&self) -> i64 {
        match self {
            Self::Int(int) => *int,
            _ => 0,
        }
    }
}

/// Where the current item is in its life cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// `loadfile` was issued; mpv has not reported `FileLoaded` yet.
    Loading,
    Loaded,
    Ended,
    Stopped,
    Error,
}

/// Everything the UI reads, written by both threads under one mutex.
///
/// The UI thread writes *optimistically* when it issues an intent (so a
/// button flips at once); the control thread then confirms or corrects it
/// from mpv's events.
///
/// Fields fed by observed properties are *mirrors* and are never reset
/// locally: mpv only notifies when a value differs from the one it sent
/// last, so two files with the same width or duration produce no second
/// event. What must not leak from one item into the next is masked by
/// [`State::has_media`] instead.
#[derive(Debug)]
struct State {
    phase: Phase,
    paused: bool,
    path: Option<PathBuf>,
    position: Duration,
    duration: Option<Duration>,
    /// A seek is in flight: `position` holds its target and `time-pos`
    /// updates (which may still describe the old position) are ignored.
    seeking: bool,
    volume: f32,
    muted: bool,
    speed: f64,
    title: Option<String>,
    width: i64,
    height: i64,
    display_width: i64,
    display_height: i64,
    rotate: i64,
    audio_tracks: Tracks,
    subtitle_tracks: Tracks,
    video_tracks: Tracks,
    audio_delay: f64,
    subtitle_delay: f64,
    hardware_decoder: Option<String>,
    aspect_override: Option<f64>,
    rotation: u16,
    ab_loop: AbLoop,
    picture: Picture,
    hardware_decoding: bool,
    hardware_decoding_locked: bool,
    /// Read on request for the Media Info dialog; dropped with the file.
    media_info: Option<Arc<MediaInfo>>,
    events: Vec<PlayerEvent>,
}

impl State {
    fn new(volume: f32) -> Self {
        Self {
            phase: Phase::Idle,
            paused: false,
            path: None,
            position: Duration::ZERO,
            duration: None,
            seeking: false,
            volume,
            muted: false,
            speed: 1.0,
            title: None,
            width: 0,
            height: 0,
            display_width: 0,
            display_height: 0,
            rotate: 0,
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
            hardware_decoding_locked: false,
            media_info: None,
            events: Vec::new(),
        }
    }

    /// Fold one observed property change into the state. Returns whether
    /// the UI should repaint for it; position updates do not, because video
    /// frames (or the audio-only timer) already drive those repaints.
    fn apply(&mut self, property: Observed, value: Value) -> bool {
        match property {
            Observed::TimePos => {
                if let Value::Double(seconds) = value
                    && !self.seeking
                    && seconds.is_finite()
                    && seconds >= 0.0
                {
                    self.position = Duration::from_secs_f64(seconds);
                }
                return false;
            }
            Observed::Duration => {
                self.duration = match value {
                    Value::Double(seconds) if seconds.is_finite() && seconds > 0.0 => {
                        Some(Duration::from_secs_f64(seconds))
                    }
                    _ => None,
                };
            }
            Observed::EofReached => match (value, self.phase) {
                // Only a loaded file can end. While a load is pending the
                // flag still describes the previous file.
                (Value::Flag(true), Phase::Loaded) => {
                    self.phase = Phase::Ended;
                    self.events.push(PlayerEvent::EndOfFile);
                }
                // Seeking back from the end resumes the same file.
                (Value::Flag(false), Phase::Ended) => self.phase = Phase::Loaded,
                _ => return false,
            },
            Observed::Pause => match value {
                Value::Flag(paused) => self.paused = paused,
                _ => return false,
            },
            Observed::Width => self.width = value.int(),
            Observed::Height => self.height = value.int(),
            Observed::DisplayWidth => self.display_width = value.int(),
            Observed::DisplayHeight => self.display_height = value.int(),
            Observed::Rotate => self.rotate = value.int(),
            Observed::MediaTitle => {
                self.title = match value {
                    Value::Text(title) if !title.is_empty() => Some(title),
                    _ => None,
                };
            }
            Observed::HwdecCurrent => {
                self.hardware_decoder = match value {
                    Value::Text(name) => hardware_decoder(Some(&name)),
                    _ => None,
                };
            }
            // Handled by the control thread, which re-reads the list.
            Observed::TrackList => {}
        }
        true
    }

    /// mpv finished opening the file that was requested last. `duration`
    /// was read at that moment and replaces the mirrored value, which may
    /// still belong to the previous file.
    fn file_loaded(&mut self, duration: Option<Duration>) {
        if self.phase == Phase::Loading {
            self.phase = Phase::Loaded;
        }
        self.seeking = false;
        self.duration = duration;
        self.events.push(PlayerEvent::FileLoaded { duration });
    }

    /// True while a file is open. Everything describing "the current media"
    /// is only reported in that case.
    fn has_media(&self) -> bool {
        matches!(self.phase, Phase::Loaded | Phase::Ended)
    }

    /// mpv ended the current file with an error: while it was being opened,
    /// or later, in the middle of playback. Errors that arrive when nothing
    /// is open or loading (a stale report for a file that was replaced) are
    /// ignored. Returns whether the state changed.
    fn failed(&mut self, reason: &str) -> bool {
        if !matches!(self.phase, Phase::Loading | Phase::Loaded) {
            return false;
        }
        self.phase = Phase::Error;
        self.events.push(PlayerEvent::Error(reason.to_owned()));
        true
    }

    /// A new item is starting (or playback stopped): rewind our own
    /// bookkeeping. The property mirrors stay as they are.
    fn rewind(&mut self) {
        self.position = Duration::ZERO;
        self.seeking = false;
    }

    fn playback_state(&self) -> PlaybackState {
        match self.phase {
            Phase::Idle => PlaybackState::Idle,
            Phase::Loading => PlaybackState::Opening,
            Phase::Loaded if self.paused => PlaybackState::Paused,
            Phase::Loaded => PlaybackState::Playing,
            Phase::Ended => PlaybackState::Ended,
            Phase::Stopped => PlaybackState::Stopped,
            Phase::Error => PlaybackState::Error,
        }
    }

    /// Coded size of the video stream.
    fn video_size(&self) -> Option<(u32, u32)> {
        positive_size(self.width, self.height).filter(|_| self.has_media())
    }

    /// Size the picture should be shown at: `dwidth`/`dheight` include the
    /// pixel aspect ratio, and 90°/270° rotation metadata swaps the axes.
    fn display_size(&self) -> Option<(u32, u32)> {
        if !self.has_media() {
            return None;
        }
        let (width, height) = positive_size(self.display_width, self.display_height)?;
        Some(if self.rotate.rem_euclid(180) == 90 {
            (height, width)
        } else {
            (width, height)
        })
    }

    fn tracks_mut(&mut self, kind: TrackKind) -> &mut Tracks {
        match kind {
            TrackKind::Audio => &mut self.audio_tracks,
            TrackKind::Subtitle => &mut self.subtitle_tracks,
            TrackKind::Video => &mut self.video_tracks,
        }
    }

    fn snapshot(&self) -> PlayerSnapshot {
        let title = self.title.clone().filter(|_| self.has_media()).or_else(|| {
            self.path
                .as_ref()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
        });
        let media = self.has_media();
        let tracks = |tracks: &Tracks| {
            if media {
                Arc::clone(tracks)
            } else {
                Tracks::default()
            }
        };
        PlayerSnapshot {
            state: self.playback_state(),
            position: self.position,
            duration: self.duration.filter(|_| media),
            volume: self.volume,
            muted: self.muted,
            speed: self.speed,
            title,
            media_path: self.path.clone(),
            video_size: self.video_size(),
            audio_tracks: tracks(&self.audio_tracks),
            subtitle_tracks: tracks(&self.subtitle_tracks),
            video_tracks: tracks(&self.video_tracks),
            audio_delay: self.audio_delay,
            subtitle_delay: self.subtitle_delay,
            hardware_decoder: self.hardware_decoder.clone().filter(|_| media),
            aspect_override: self.aspect_override,
            rotation: self.rotation,
            ab_loop: self.ab_loop,
            picture: self.picture,
            hardware_decoding: self.hardware_decoding,
            hardware_decoding_locked: self.hardware_decoding_locked,
            media_info: self.media_info.clone().filter(|_| media),
        }
    }
}

fn positive_size(width: i64, height: i64) -> Option<(u32, u32)> {
    let width = u32::try_from(width).ok().filter(|v| *v > 0)?;
    let height = u32::try_from(height).ok().filter(|v| *v > 0)?;
    Some((width, height))
}

/// Mark `id` as the selected track (or none), leaving the rest untouched.
fn with_selection(tracks: &[TrackInfo], id: Option<i64>) -> Tracks {
    tracks
        .iter()
        .map(|track| TrackInfo {
            selected: Some(track.id) == id,
            ..track.clone()
        })
        .collect()
}

/// State shared by the UI thread, the control thread and the renderer.
struct Shared {
    state: Mutex<State>,
    /// Packed display size (`width << 32 | height`), 0 when there is no
    /// video. An atomic so the paint path never touches the state lock.
    display_size: AtomicU64,
    /// Set by mpv's wakeup callback until the control thread picks it up,
    /// so a burst of wakeups queues a single [`Request::Wake`].
    wake_pending: AtomicBool,
    /// Woken when the control thread changes something the UI shows.
    repaint: egui::Context,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // A poisoned lock only means another thread panicked mid-update;
        // the state is plain data and stays usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish_display_size(&self, size: Option<(u32, u32)>) {
        let packed = size.map_or(0, |(w, h)| (u64::from(w) << 32) | u64::from(h));
        self.display_size.store(packed, Ordering::Relaxed);
    }
}

// ------------------------------------------------------- control thread

/// Work for the control thread.
#[derive(Clone, Debug, PartialEq)]
enum Request {
    /// mpv has events waiting.
    Wake,
    Load {
        path: String,
        paused: bool,
    },
    SetPause(bool),
    Stop,
    Seek {
        seconds: f64,
        exact: bool,
    },
    SetProperty {
        name: &'static str,
        value: PropertyValue,
    },
    AddSubtitle(String),
    Screenshot(String),
    /// Read the properties behind the Media Info dialog.
    ReadMediaInfo,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
enum PropertyValue {
    Flag(bool),
    Double(f64),
    Text(String),
}

/// Reduce a batch of queued requests to what still matters: of several
/// seeks only the last one is worth executing, and one wake-up drains all
/// pending events. The order of everything else is preserved.
fn coalesce(batch: Vec<Request>) -> Vec<Request> {
    let last_seek = batch
        .iter()
        .rposition(|request| matches!(request, Request::Seek { .. }));
    let last_wake = batch.iter().rposition(|request| *request == Request::Wake);
    batch
        .into_iter()
        .enumerate()
        .filter(|(index, request)| match request {
            Request::Seek { .. } => Some(*index) == last_seek,
            Request::Wake => Some(*index) == last_wake,
            _ => true,
        })
        .map(|(_, request)| request)
        .collect()
}

/// What an mpv event means to us, detached from mpv's borrowed data.
enum Incoming {
    Property(Observed, Value),
    FileLoaded,
    PlaybackRestart,
    /// The current file ended with this `MPV_ERROR_*` code.
    Failed(i32),
    Shutdown,
}

struct Control {
    mpv: Arc<Mpv>,
    shared: Arc<Shared>,
    /// Set while a file is being retried with software decoding: the
    /// `hwdec` mode to put back before the next file is loaded.
    restore_hwdec: std::cell::RefCell<Option<String>>,
}

impl Control {
    fn run(self, requests: Receiver<Request>) {
        while let Ok(first) = requests.recv() {
            let mut batch = vec![first];
            batch.extend(requests.try_iter());
            for request in coalesce(batch) {
                match request {
                    Request::Shutdown => return,
                    Request::Wake => {
                        // Re-arm before draining, so an event that arrives
                        // while draining queues another wake-up.
                        self.shared.wake_pending.store(false, Ordering::Release);
                        if !self.drain_events() {
                            return;
                        }
                    }
                    request => self.execute(request),
                }
            }
        }
    }

    /// Run one request against the mpv core. This may block for as long as
    /// the core needs, which is exactly why it does not run on the UI thread.
    fn execute(&self, request: Request) {
        let result = match &request {
            Request::Load { path, paused } => {
                // A software retry applied to one file only.
                if let Some(mode) = self.restore_hwdec.borrow_mut().take() {
                    let _ = self.mpv.set_property("hwdec", mode.as_str());
                }
                self.mpv
                    .command("loadfile", &[path, "replace"])
                    .and_then(|()| self.mpv.set_property("pause", *paused))
            }
            Request::SetPause(paused) => self.mpv.set_property("pause", *paused),
            Request::Stop => self.mpv.command("stop", &[]),
            Request::Seek { seconds, exact } => {
                // Keyframe seeks decode a single frame, so the picture can
                // follow the pointer while scrubbing.
                let flags = if *exact {
                    "absolute"
                } else {
                    "absolute+keyframes"
                };
                self.mpv.command("seek", &[&seconds.to_string(), flags])
            }
            Request::SetProperty { name, value } => match value {
                PropertyValue::Flag(flag) => self.mpv.set_property(name, *flag),
                PropertyValue::Double(double) => self.mpv.set_property(name, *double),
                PropertyValue::Text(text) => self.mpv.set_property(name, text.as_str()),
            },
            Request::AddSubtitle(path) => self.mpv.command("sub-add", &[path, "select"]),
            // `subtitles`: the decoded frame plus subtitles, at the video's
            // own resolution, independent of the window.
            Request::Screenshot(path) => {
                self.mpv.command("screenshot-to-file", &[path, "subtitles"])
            }
            Request::ReadMediaInfo => {
                let info = self.read_media_info();
                self.shared.state().media_info = Some(Arc::new(info));
                self.shared.repaint.request_repaint();
                Ok(())
            }
            Request::Wake | Request::Shutdown => Ok(()),
        };

        let Err(err) = result else { return };
        tracing::warn!(%err, ?request, "mpv rejected a request");
        let reported = {
            let mut state = self.shared.state();
            match request {
                Request::Load { .. } => state.failed(failure_reason(code::LOADING_FAILED)),
                Request::AddSubtitle(_) => {
                    state.events.push(PlayerEvent::Error(
                        "Could not load the subtitle file".to_owned(),
                    ));
                    true
                }
                Request::Screenshot(_) => {
                    state
                        .events
                        .push(PlayerEvent::Error("Could not save the image".to_owned()));
                    true
                }
                // A seek that fails (nothing loaded) must not freeze the
                // position display.
                Request::Seek { .. } => {
                    state.seeking = false;
                    false
                }
                _ => false,
            }
        };
        if reported {
            self.shared.repaint.request_repaint();
        }
    }

    /// Pull every pending mpv event and fold it into the shared state.
    /// Returns `false` once mpv is shutting down.
    fn drain_events(&self) -> bool {
        // 1. Collect, without the state lock: these are mpv calls.
        let mut incoming = Vec::new();
        while let Some(event) = self.mpv.wait_event(0.0) {
            match event {
                Ok(Event::PropertyChange {
                    reply_userdata,
                    change,
                    ..
                }) => {
                    if let Some(property) = Observed::from_id(reply_userdata) {
                        incoming.push(Incoming::Property(property, change.into()));
                    }
                }
                Ok(Event::FileLoaded) => incoming.push(Incoming::FileLoaded),
                Ok(Event::PlaybackRestart) => incoming.push(Incoming::PlaybackRestart),
                Ok(Event::Shutdown) => incoming.push(Incoming::Shutdown),
                Ok(_) => {}
                // The only failing event in use is the end of a file. A
                // string that is not UTF-8 (a title, say) is not a failure.
                Err(libmpv2::Error::Raw(error)) => incoming.push(Incoming::Failed(error)),
                Err(err) => tracing::debug!(%err, "ignoring an undecodable mpv event"),
            }
        }
        if incoming.is_empty() {
            return true;
        }

        // 2. Apply under the lock; note what needs a follow-up read.
        let (mut repaint, mut tracks, mut loaded, mut restarted) = (false, false, false, false);
        let mut alive = true;
        let mut retry = None;
        {
            let mut state = self.shared.state();
            for event in incoming {
                match event {
                    Incoming::Property(Observed::TrackList, _) => tracks = true,
                    Incoming::Property(property, value) => {
                        repaint |= state.apply(property, value);
                    }
                    Incoming::FileLoaded => (loaded, tracks) = (true, true),
                    Incoming::PlaybackRestart => restarted = true,
                    Incoming::Failed(error) => {
                        let reason = failure_reason(error);
                        // Hardware decoding must never be why a file does
                        // not play: try once more without it.
                        let again = (state.phase == Phase::Loading
                            && self.restore_hwdec.borrow().is_none())
                        .then(|| state.path.clone())
                        .flatten();
                        if let Some(path) = again {
                            retry = Some((path, error, state.paused));
                        } else if state.failed(reason) {
                            tracing::warn!(error, reason, "mpv could not play the file");
                            repaint = true;
                        }
                    }
                    Incoming::Shutdown => alive = false,
                }
            }
            self.shared.publish_display_size(state.display_size());
        }

        if let Some((path, error, paused)) = retry {
            repaint |= self.retry_or_fail(&path, error, paused);
        }

        // 3. Follow-up reads, again without the lock.
        let duration = loaded.then(|| self.read_seconds("duration")).flatten();
        let position = (loaded || restarted)
            .then(|| self.read_seconds("time-pos"))
            .flatten();
        let lists = tracks.then(|| self.read_tracks());

        if loaded || restarted || lists.is_some() {
            let mut state = self.shared.state();
            if loaded {
                state.file_loaded(duration);
            }
            if loaded || restarted {
                state.seeking = false;
                if let Some(position) = position {
                    state.position = position;
                }
            }
            if let Some(lists) = lists {
                state.audio_tracks = lists.audio.into();
                state.subtitle_tracks = lists.subtitle.into();
                state.video_tracks = lists.video.into();
            }
            // `file_loaded` may just have made the picture size reportable.
            self.shared.publish_display_size(state.display_size());
            repaint = true;
        }

        if repaint {
            self.shared.repaint.request_repaint();
        }
        alive
    }

    /// A load failed. Load the file again with software decoding if that
    /// can make a difference; otherwise report the failure. Returns whether
    /// the UI should repaint.
    fn retry_or_fail(&self, path: &Path, error: i32, paused: bool) -> bool {
        let mode = self.mpv.get_property::<String>("hwdec").unwrap_or_default();
        if retry_in_software(error, &mode)
            && let Some(text) = path.to_str()
            && self.mpv.set_property("hwdec", "no").is_ok()
        {
            tracing::warn!(
                error,
                path = %path.display(),
                "the file failed to load; retrying with software decoding"
            );
            *self.restore_hwdec.borrow_mut() = Some(mode);
            let reloaded = self
                .mpv
                .command("loadfile", &[text, "replace"])
                .and_then(|()| self.mpv.set_property("pause", paused));
            if reloaded.is_ok() {
                return false;
            }
        }
        let reason = failure_reason(error);
        tracing::warn!(error, reason, path = %path.display(), "mpv could not play the file");
        self.shared.state().failed(reason)
    }

    fn text(&self, property: &str) -> Option<String> {
        self.mpv
            .get_property::<String>(property)
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    }

    fn number(&self, property: &str) -> Option<u64> {
        self.mpv
            .get_property::<i64>(property)
            .ok()
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
    }

    /// Everything the Media Info dialog shows, straight from mpv. A
    /// property that is unavailable (no such stream, not known yet) simply
    /// stays `None`.
    fn read_media_info(&self) -> MediaInfo {
        let video = self.text("video-format").map(|codec| VideoInfo {
            codec: Some(codec),
            profile: self.text("current-tracks/video/codec-profile"),
            // With a hardware decoder `pixelformat` is the surface type
            // (`vaapi`); the format of the picture inside is reported apart.
            pixel_format: self
                .text("video-params/hw-pixelformat")
                .or_else(|| self.text("video-params/pixelformat")),
            size: self
                .number("video-params/w")
                .zip(self.number("video-params/h"))
                .and_then(|(w, h)| Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?))),
            fps: self
                .mpv
                .get_property::<f64>("container-fps")
                .ok()
                .or_else(|| self.mpv.get_property::<f64>("estimated-vf-fps").ok())
                .filter(|fps| fps.is_finite() && *fps > 0.0),
            bitrate: self.number("video-bitrate"),
        });
        let audio = self.text("audio-codec-name").map(|codec| AudioInfo {
            codec: Some(codec),
            profile: self.text("current-tracks/audio/codec-profile"),
            channels: self.text("audio-params/channels"),
            sample_rate: self
                .number("audio-params/samplerate")
                .and_then(|rate| u32::try_from(rate).ok()),
            bitrate: self.number("audio-bitrate"),
        });
        MediaInfo {
            container: self.text("file-format"),
            file_size: self.number("file-size"),
            hardware_decoder: hardware_decoder(self.text("hwdec-current").as_deref()),
            video,
            audio,
        }
    }

    fn read_seconds(&self, property: &str) -> Option<Duration> {
        self.mpv
            .get_property::<f64>(property)
            .ok()
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(Duration::from_secs_f64)
    }

    fn read_tracks(&self) -> TrackLists {
        let count = self
            .mpv
            .get_property::<i64>("track-list/count")
            .ok()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0);
        parse_track_list(count, |index, name| {
            self.mpv
                .get_property::<String>(&format!("track-list/{index}/{name}"))
                .ok()
        })
    }
}

// ------------------------------------------------------------- UI side

pub struct MpvPlayer {
    // Field order is drop order: `video` (the render context) goes first,
    // the mpv handle after it. `Drop` below also releases explicitly, because
    // a queued paint callback may still hold a `VideoRenderer` clone.
    video: Option<VideoRenderer>,
    mpv: Arc<Mpv>,
    shared: Arc<Shared>,
    requests: Sender<Request>,
    control: Option<JoinHandle<()>>,
    /// The configured hardware decoding mode, restored when switched on.
    hwdec_mode: String,
}

impl MpvPlayer {
    /// Create the mpv core and start the control thread. `repaint` is woken
    /// whenever mpv changes something the UI shows.
    pub fn new(config: &crate::config::PlaybackConfig, repaint: egui::Context) -> Result<Self> {
        let hwdec = config.hwdec.clone();
        let audio_device = config.audio_device.clone();
        let deinterlace = config.deinterlace;
        let volume = 85.0f64;
        let extra_options = std::env::var(MPV_OPTIONS_ENV)
            .map(|text| parse_extra_options(&text))
            .unwrap_or_default();

        let mut mpv = Mpv::with_initializer(|init| {
            // `libmpv` is the video output used together with the render API.
            init.set_property("vo", "libmpv")?;
            // egui owns vsync. Without this, `mpv_render_context_render`
            // sleeps until the frame's display time and stalls the UI thread.
            init.set_property("video-timing-offset", 0.0)?;
            // The UI computes the letterbox rectangle itself and hands mpv a
            // target of exactly that size. Left to its own aspect logic, mpv
            // would round the picture independently and could leave a
            // one-pixel black column or row inside the target.
            init.set_property("keepaspect", "no")?;
            init.set_property("idle", "yes")?;
            init.set_property("keep-open", "yes")?;
            init.set_property("osd-level", 0)?;
            init.set_property("hwdec", hwdec.as_str())?;
            if let Some(interop) = hwdec_interop(
                nvidia_driver_present(),
                cfg!(any(target_arch = "x86", target_arch = "x86_64")),
            ) {
                init.set_property("gpu-hwdec-interop", interop)?;
            }
            init.set_property("volume", volume)?;
            if !audio_device.is_empty() {
                init.set_property("audio-device", audio_device.as_str())?;
            }
            if deinterlace {
                init.set_property("deinterlace", "auto")?;
            }
            // Diagnostics last, so they can override the defaults above. A
            // typo must not prevent the player from starting.
            for (name, value) in &extra_options {
                if let Err(err) = init.set_property(name, value.as_str()) {
                    tracing::warn!(%err, option = %name, "ignoring invalid mpv option");
                }
            }
            Ok(())
        })
        .context("failed to create the libmpv context")?;

        let shared = Arc::new(Shared {
            state: Mutex::new(State::new(volume as f32)),
            display_size: AtomicU64::new(0),
            wake_pending: AtomicBool::new(false),
            repaint,
        });
        let (requests, receiver) = mpsc::channel();

        // Runs on arbitrary mpv threads and must not call into mpv: it only
        // queues a wake-up for the control thread.
        let (wake, flag) = (requests.clone(), Arc::clone(&shared));
        mpv.set_wakeup_callback(move || {
            if !flag.wake_pending.swap(true, Ordering::AcqRel) {
                let _ = wake.send(Request::Wake);
            }
        });

        // Change notifications instead of polling: mpv sends the current
        // value once after registration and then on every change.
        for property in Observed::ALL {
            let (name, format) = property.spec();
            if let Err(err) = mpv.observe_property(name, format, property as u64) {
                tracing::warn!(%err, property = name, "could not observe mpv property");
            }
        }

        let mpv = Arc::new(mpv);
        let control = Control {
            mpv: Arc::clone(&mpv),
            shared: Arc::clone(&shared),
            restore_hwdec: std::cell::RefCell::new(None),
        };
        let control = thread::Builder::new()
            .name("mpv-control".to_owned())
            .spawn(move || control.run(receiver))
            .context("failed to start the mpv control thread")?;

        {
            let mut state = shared.state();
            state.hardware_decoding = hwdec_enabled(&hwdec);
        }

        Ok(Self {
            video: None,
            mpv,
            shared,
            requests,
            control: Some(control),
            hwdec_mode: hwdec,
        })
    }

    fn send(&self, request: Request) -> Result<()> {
        self.requests
            .send(request)
            .map_err(|_| anyhow::anyhow!("the mpv control thread has stopped"))
    }

    fn set_property(&self, name: &'static str, value: PropertyValue) -> Result<()> {
        self.send(Request::SetProperty { name, value })
    }

    fn seek_with(&mut self, position: Duration, exact: bool) -> Result<()> {
        {
            let mut state = self.shared.state();
            if matches!(state.phase, Phase::Loaded | Phase::Ended) {
                state.position = position;
                state.seeking = true;
            }
        }
        self.send(Request::Seek {
            seconds: position.as_secs_f64(),
            exact,
        })
    }

    /// Create the mpv render context for OpenGL and return the handle the UI
    /// uses to draw frames.
    ///
    /// Must be called on the GL thread with the GL context current (eframe
    /// guarantees both inside the app creator). `init.get_proc_address` is
    /// the loader of that context; without one there is no GL context and
    /// the call fails cleanly, leaving playback audio-only.
    pub fn attach_video(&mut self, init: VideoInit) -> Result<VideoRenderer> {
        if let Some(video) = &self.video {
            return Ok(video.clone());
        }
        let loader = init
            .get_proc_address
            .context("the window has no OpenGL context")?;

        // Software rasterizers need two workarounds:
        // - Hardware frames cannot be shown there; mpv's VA-API interop
        //   "succeeds" and then presents a black picture. Decode in software.
        // - With mpv's lookup textures (dither matrix, scaler weights),
        //   llvmpipe intermittently (about 2 of 3 starts) renders a grid of
        //   black rows and columns through the render API. Standalone mpv on
        //   the same stack and every real GPU tested are clean; the exact
        //   Mesa-side cause is not known. Dithering and high-quality scalers
        //   only refine the picture, which a software display does not need.
        if let Some(renderer) = init
            .gl_renderer
            .as_deref()
            .filter(|r| is_software_renderer(r))
        {
            tracing::info!(
                %renderer,
                "software OpenGL: hardware decoding, dithering and scaler LUTs disabled"
            );
            self.set_property("hwdec", PropertyValue::Text("no".to_owned()))?;
            // Bilinear scaling needs no weight-lookup texture; with only the
            // dither texture removed the grid still appeared in 1 of 10
            // starts. It is also far cheaper on a CPU rasterizer.
            for (name, value) in [
                ("dither-depth", "no"),
                ("scale", "bilinear"),
                ("dscale", "bilinear"),
                ("cscale", "bilinear"),
            ] {
                self.set_property(name, PropertyValue::Text(value.to_owned()))?;
            }
            let mut state = self.shared.state();
            state.hardware_decoding = false;
            state.hardware_decoding_locked = true;
        }

        fn load_symbol(loader: &GetProcAddress, name: &str) -> *mut c_void {
            match CString::new(name) {
                Ok(name) => loader(&name).cast_mut(),
                Err(_) => std::ptr::null_mut(),
            }
        }

        let mut params = vec![
            RenderParam::ApiType(RenderParamApiType::OpenGl),
            RenderParam::InitParams(OpenGLInitParams {
                get_proc_address: load_symbol,
                ctx: loader,
            }),
        ];
        // The native display lets mpv's hardware decoders (VA-API) open the
        // same display connection as the window, which zero-copy interop
        // needs. mpv only reads the pointer while the render context exists,
        // and that is freed in `on_exit`, before eframe closes the display.
        match init.native_display {
            Some(NativeDisplay::Wayland(display)) => {
                params.push(RenderParam::WaylandDisplay(display.as_ptr().cast_const()));
            }
            Some(NativeDisplay::X11(display)) => {
                params.push(RenderParam::X11Display(display.as_ptr().cast_const()));
            }
            None => {}
        }

        let mut context = self
            .mpv
            .create_render_context(params)
            .context("failed to create the mpv render context")?;

        // Runs on an mpv thread. `request_repaint` is thread-safe and is the
        // only thing done here: no mpv or GL call is allowed in this callback.
        let repaint = init.repaint;
        context.set_update_callback(move || repaint.request_repaint());

        // SAFETY: this only erases the borrow of `self.mpv` from the type.
        // The borrowed `Mpv` lives on the heap behind an `Arc`, so its
        // address is stable, and `RendererShared` keeps a clone of that `Arc`
        // in a field declared *after* the slot holding the context, so the
        // handle is guaranteed to outlive the render context no matter which
        // `VideoRenderer` clone is dropped last.
        let context: RenderContext<'static> =
            unsafe { std::mem::transmute::<RenderContext<'_>, RenderContext<'static>>(context) };

        let video = VideoRenderer {
            shared: Arc::new(RendererShared {
                slot: Mutex::new(Some(GlBound {
                    context: Some(context),
                    owner: thread::current().id(),
                })),
                shared: Arc::clone(&self.shared),
                _mpv: Arc::clone(&self.mpv),
            }),
        };
        self.video = Some(video.clone());
        Ok(video)
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        // The render context must be gone before the mpv handle is destroyed.
        if let Some(video) = &self.video {
            video.release();
        }
        // Stop the control thread so its handle clone is released too. It
        // is either idle in `recv()` or finishing one request.
        let _ = self.requests.send(Request::Shutdown);
        if let Some(control) = self.control.take() {
            let _ = control.join();
        }
    }
}

impl Player for MpvPlayer {
    fn load(&mut self, path: &Path, start_paused: bool) -> Result<()> {
        let text = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("media path is not valid UTF-8"))?;

        {
            let mut state = self.shared.state();
            state.rewind();
            state.phase = Phase::Loading;
            state.paused = start_paused;
            state.path = Some(path.to_path_buf());
            state.media_info = None;
        }
        // Do not keep showing the previous picture at the new aspect ratio.
        self.shared.publish_display_size(None);
        self.send(Request::Load {
            path: text.to_owned(),
            paused: start_paused,
        })
    }

    fn play(&mut self) -> Result<()> {
        self.shared.state().paused = false;
        self.send(Request::SetPause(false))
    }

    fn pause(&mut self) -> Result<()> {
        self.shared.state().paused = true;
        self.send(Request::SetPause(true))
    }

    fn toggle_pause(&mut self) -> Result<()> {
        let paused = self.shared.state().paused;
        if paused { self.play() } else { self.pause() }
    }

    fn stop(&mut self) -> Result<()> {
        {
            let mut state = self.shared.state();
            state.rewind();
            state.phase = Phase::Stopped;
        }
        self.shared.publish_display_size(None);
        self.send(Request::Stop)
    }

    fn seek(&mut self, position: Duration) -> Result<()> {
        self.seek_with(position, true)
    }

    fn scrub(&mut self, position: Duration) -> Result<()> {
        self.seek_with(position, false)
    }

    fn set_volume(&mut self, volume: f32) -> Result<()> {
        let volume = volume.clamp(0.0, 100.0);
        self.shared.state().volume = volume;
        self.set_property("volume", PropertyValue::Double(f64::from(volume)))
    }

    fn set_muted(&mut self, muted: bool) -> Result<()> {
        self.shared.state().muted = muted;
        self.set_property("mute", PropertyValue::Flag(muted))
    }

    fn set_speed(&mut self, speed: f64) -> Result<()> {
        let speed = speed.clamp(0.25, 4.0);
        self.shared.state().speed = speed;
        self.set_property("speed", PropertyValue::Double(speed))
    }

    fn select_track(&mut self, kind: TrackKind, id: Option<i64>) -> Result<()> {
        {
            let mut state = self.shared.state();
            let tracks = state.tracks_mut(kind);
            if let Some(id) = id {
                anyhow::ensure!(
                    tracks.iter().any(|track| track.id == id),
                    "there is no {} track #{id}",
                    kind.label().to_lowercase()
                );
            }
            // Optimistic: the check mark moves now; mpv's `track-list`
            // notification confirms (or corrects) it shortly after.
            *tracks = with_selection(tracks, id);
        }
        self.set_property(track_property(kind), PropertyValue::Text(track_value(id)))
    }

    fn add_subtitle_file(&mut self, path: &Path) -> Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("subtitle path is not valid UTF-8"))?;
        // The new track shows up through the `track-list` notification; a
        // failure comes back as `PlayerEvent::Error`.
        self.send(Request::AddSubtitle(path.to_owned()))
    }

    fn set_audio_delay(&mut self, seconds: f64) -> Result<()> {
        let seconds = clamp_delay(seconds)?;
        self.shared.state().audio_delay = seconds;
        self.set_property("audio-delay", PropertyValue::Double(seconds))
    }

    fn set_subtitle_delay(&mut self, seconds: f64) -> Result<()> {
        let seconds = clamp_delay(seconds)?;
        self.shared.state().subtitle_delay = seconds;
        self.set_property("sub-delay", PropertyValue::Double(seconds))
    }

    fn set_aspect_override(&mut self, ratio: Option<f64>) -> Result<()> {
        let value = aspect_value(ratio)?;
        self.shared.state().aspect_override = ratio;
        // mpv answers with new `dwidth`/`dheight`, which re-letterboxes.
        self.set_property("video-aspect-override", PropertyValue::Text(value))
    }

    fn set_rotation(&mut self, degrees: i32) -> Result<()> {
        let rotation = super::normalize_rotation(degrees);
        self.shared.state().rotation = rotation;
        // The renderer rotates; `video-params/rotate` then reports the
        // file's rotation plus this one, which `display_size` accounts for.
        self.set_property("video-rotate", PropertyValue::Text(rotation.to_string()))
    }

    fn set_ab_loop(&mut self, ab: AbLoop) -> Result<()> {
        self.shared.state().ab_loop = ab;
        self.set_property("ab-loop-a", PropertyValue::Text(ab_value(ab.a)))?;
        self.set_property("ab-loop-b", PropertyValue::Text(ab_value(ab.b)))
    }

    fn set_picture(&mut self, picture: Picture) -> Result<()> {
        let picture = picture.clamped();
        self.shared.state().picture = picture;
        for (name, value) in [
            ("brightness", picture.brightness),
            ("contrast", picture.contrast),
            ("saturation", picture.saturation),
            ("hue", picture.hue),
        ] {
            self.set_property(name, PropertyValue::Double(f64::from(value)))?;
        }
        Ok(())
    }

    fn set_hardware_decoding(&mut self, enabled: bool) -> Result<()> {
        let mode = {
            let mut state = self.shared.state();
            anyhow::ensure!(
                !(enabled && state.hardware_decoding_locked),
                "hardware decoding is not available with this OpenGL renderer"
            );
            state.hardware_decoding = enabled;
            // A configured "no" still means "on" when the user asks for it.
            match (enabled, hwdec_enabled(&self.hwdec_mode)) {
                (false, _) => "no".to_owned(),
                (true, true) => self.hwdec_mode.clone(),
                (true, false) => "auto-safe".to_owned(),
            }
        };
        // mpv re-creates the decoder; `hwdec-current` follows.
        self.set_property("hwdec", PropertyValue::Text(mode))
    }

    fn refresh_media_info(&mut self) {
        let _ = self.send(Request::ReadMediaInfo);
    }

    fn screenshot(&mut self, path: &Path) -> Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("image path is not valid UTF-8"))?;
        self.send(Request::Screenshot(path.to_owned()))
    }

    fn snapshot(&mut self) -> PlayerSnapshot {
        self.shared.state().snapshot()
    }

    fn drain_events(&mut self) -> Vec<PlayerEvent> {
        std::mem::take(&mut self.shared.state().events)
    }

    fn backend_name(&self) -> &'static str {
        "libmpv"
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// A fake `track-list` in mpv's property form.
    fn properties(rows: &[&[(&str, &str)]]) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            for (name, value) in row.iter() {
                map.insert(format!("{index}/{name}"), (*value).to_owned());
            }
        }
        map
    }

    fn parse(rows: &[&[(&str, &str)]]) -> TrackLists {
        let map = properties(rows);
        parse_track_list(rows.len(), |index, name| {
            map.get(&format!("{index}/{name}")).cloned()
        })
    }

    #[test]
    fn track_list_is_split_by_kind_with_all_fields() {
        let lists = parse(&[
            &[
                ("id", "1"),
                ("type", "video"),
                ("codec", "h264"),
                ("selected", "yes"),
            ],
            &[
                ("id", "1"),
                ("type", "audio"),
                ("title", "Main"),
                ("lang", "eng"),
                ("codec", "aac"),
                ("default", "yes"),
                ("selected", "yes"),
            ],
            &[
                ("id", "2"),
                ("type", "audio"),
                ("lang", "fra"),
                ("codec", "ac3"),
            ],
            &[
                ("id", "1"),
                ("type", "sub"),
                ("lang", "eng"),
                ("codec", "subrip"),
                ("external", "yes"),
                ("selected", "no"),
            ],
        ]);

        assert_eq!(lists.video.len(), 1);
        assert_eq!(
            lists.audio,
            vec![
                TrackInfo {
                    id: 1,
                    title: "Main".to_owned(),
                    language: Some("eng".to_owned()),
                    codec: Some("aac".to_owned()),
                    default: true,
                    selected: true,
                    external: false,
                },
                TrackInfo {
                    id: 2,
                    language: Some("fra".to_owned()),
                    codec: Some("ac3".to_owned()),
                    ..Default::default()
                },
            ]
        );
        assert_eq!(lists.subtitle.len(), 1);
        assert!(lists.subtitle[0].external && !lists.subtitle[0].selected);
        assert_eq!(lists.subtitle[0].title, "");
    }

    #[test]
    fn malformed_entries_are_skipped() {
        let lists = parse(&[
            &[("type", "audio")],                   // no id
            &[("id", "x"), ("type", "audio")],      // id is not a number
            &[("id", "3"), ("type", "attachment")], // unknown kind
            &[("id", "4")],                         // no kind
            &[("id", "5"), ("type", "sub"), ("title", "")],
        ]);
        assert!(lists.audio.is_empty() && lists.video.is_empty());
        assert_eq!(lists.subtitle.len(), 1);
        assert_eq!(lists.subtitle[0].id, 5);
        assert_eq!(parse(&[]), TrackLists::default());
    }

    #[test]
    fn selection_maps_to_mpv_properties() {
        assert_eq!(track_property(TrackKind::Audio), "aid");
        assert_eq!(track_property(TrackKind::Subtitle), "sid");
        assert_eq!(track_property(TrackKind::Video), "vid");
        assert_eq!(track_value(Some(3)), "3");
        assert_eq!(track_value(None), "no");
    }

    #[test]
    fn hwdec_current_distinguishes_software() {
        assert_eq!(hardware_decoder(Some("vaapi")), Some("vaapi".to_owned()));
        assert_eq!(
            hardware_decoder(Some("vaapi-copy")),
            Some("vaapi-copy".to_owned())
        );
        assert_eq!(hardware_decoder(Some("no")), None);
        assert_eq!(hardware_decoder(Some("")), None);
        assert_eq!(hardware_decoder(None), None);
    }

    #[test]
    fn extra_options_are_split_and_vo_is_refused() {
        let options = parse_extra_options("log-file=/tmp/a=b.log, ao = null ,vo=gpu,,junk,=x");
        assert_eq!(
            options,
            vec![
                ("log-file".to_owned(), "/tmp/a=b.log".to_owned()),
                ("ao".to_owned(), "null".to_owned()),
            ]
        );
        assert!(parse_extra_options("").is_empty());
    }

    fn loaded_state() -> State {
        let mut state = State::new(85.0);
        state.phase = Phase::Loading;
        state.file_loaded(Some(Duration::from_secs(60)));
        state.events.clear();
        state
    }

    #[test]
    fn stale_eof_is_ignored_while_a_load_is_pending() {
        let mut state = State::new(85.0);
        state.phase = Phase::Loading;
        // Still describes the previous file.
        assert!(!state.apply(Observed::EofReached, Value::Flag(true)));
        assert_eq!(state.playback_state(), PlaybackState::Opening);
        assert!(state.events.is_empty());

        state.file_loaded(Some(Duration::from_secs(10)));
        assert_eq!(state.playback_state(), PlaybackState::Playing);
        assert_eq!(state.duration, Some(Duration::from_secs(10)));
        assert!(matches!(state.events[..], [PlayerEvent::FileLoaded { .. }]));
    }

    #[test]
    fn end_of_file_is_reported_once_and_seeking_back_resumes() {
        let mut state = loaded_state();
        assert!(state.apply(Observed::EofReached, Value::Flag(true)));
        assert_eq!(state.playback_state(), PlaybackState::Ended);
        // A repeated notification must not advance the playlist twice.
        assert!(!state.apply(Observed::EofReached, Value::Flag(true)));
        assert!(matches!(state.events[..], [PlayerEvent::EndOfFile]));

        assert!(state.apply(Observed::EofReached, Value::Flag(false)));
        assert_eq!(state.playback_state(), PlaybackState::Playing);
    }

    #[test]
    fn pause_flag_maps_to_playing_and_paused() {
        let mut state = loaded_state();
        assert!(state.apply(Observed::Pause, Value::Flag(true)));
        assert_eq!(state.playback_state(), PlaybackState::Paused);
        assert!(state.apply(Observed::Pause, Value::Flag(false)));
        assert_eq!(state.playback_state(), PlaybackState::Playing);
        // Idle stays idle whatever the pause flag says.
        let mut idle = State::new(85.0);
        idle.apply(Observed::Pause, Value::Flag(true));
        assert_eq!(idle.playback_state(), PlaybackState::Idle);
    }

    #[test]
    fn position_updates_are_held_back_during_a_seek() {
        let mut state = loaded_state();
        assert!(!state.apply(Observed::TimePos, Value::Double(12.5)));
        assert_eq!(state.position, Duration::from_millis(12_500));

        // The UI seeks to 40 s; an old position still in the queue must not
        // drag the handle back.
        state.position = Duration::from_secs(40);
        state.seeking = true;
        state.apply(Observed::TimePos, Value::Double(12.6));
        assert_eq!(state.position, Duration::from_secs(40));

        state.seeking = false;
        state.apply(Observed::TimePos, Value::Double(40.1));
        assert_eq!(state.position, Duration::from_millis(40_100));
        // Garbage never becomes a position.
        state.apply(Observed::TimePos, Value::Double(f64::NAN));
        state.apply(Observed::TimePos, Value::Double(-1.0));
        state.apply(Observed::TimePos, Value::None);
        assert_eq!(state.position, Duration::from_millis(40_100));
    }

    #[test]
    fn display_size_applies_rotation_and_needs_both_axes() {
        let mut state = loaded_state();
        assert_eq!(state.display_size(), None);
        state.apply(Observed::DisplayWidth, Value::Int(1920));
        assert_eq!(state.display_size(), None);
        state.apply(Observed::DisplayHeight, Value::Int(1080));
        assert_eq!(state.display_size(), Some((1920, 1080)));

        for (rotate, expected) in [(90, (1080, 1920)), (180, (1920, 1080)), (270, (1080, 1920))] {
            state.apply(Observed::Rotate, Value::Int(rotate));
            assert_eq!(state.display_size(), Some(expected), "rotate {rotate}");
        }
        // The property going away (file closed) removes the picture.
        state.apply(Observed::DisplayWidth, Value::None);
        assert_eq!(state.display_size(), None);
    }

    #[test]
    fn mirrors_survive_a_reload_and_are_masked_meanwhile() {
        // First file: 640 × 360, 60 s.
        let mut state = loaded_state();
        state.apply(Observed::DisplayWidth, Value::Int(640));
        state.apply(Observed::DisplayHeight, Value::Int(360));
        state.apply(Observed::Width, Value::Int(640));
        state.apply(Observed::Height, Value::Int(360));
        assert_eq!(state.display_size(), Some((640, 360)));

        // The UI loads a second file.
        state.rewind();
        state.phase = Phase::Loading;
        assert_eq!(state.display_size(), None, "no stale picture while opening");
        let opening = state.snapshot();
        assert_eq!(opening.duration, None);
        assert_eq!(opening.video_size, None);

        // It is 640 × 480 and also 60 s long: mpv only notifies the height.
        state.file_loaded(Some(Duration::from_secs(60)));
        state.apply(Observed::DisplayHeight, Value::Int(480));
        state.apply(Observed::Height, Value::Int(480));
        assert_eq!(state.display_size(), Some((640, 480)));
        assert_eq!(state.snapshot().duration, Some(Duration::from_secs(60)));
        assert_eq!(state.snapshot().video_size, Some((640, 480)));
    }

    #[test]
    fn failures_are_reported_while_loading_or_playing_only() {
        let mut state = State::new(85.0);
        state.phase = Phase::Loading;
        assert!(state.failed(failure_reason(code::UNKNOWN_FORMAT)));
        assert_eq!(state.playback_state(), PlaybackState::Error);
        assert!(
            matches!(&state.events[..], [PlayerEvent::Error(text)] if text == "Unrecognised file format")
        );
        // Reported once: a second error for the same file changes nothing.
        assert!(!state.failed("again"));
        assert_eq!(state.events.len(), 1);

        // A decoder that gives up in the middle of a file.
        let mut playing = loaded_state();
        assert!(playing.failed(failure_reason(-20)));
        assert_eq!(playing.playback_state(), PlaybackState::Error);

        // Nothing open: a stale error is not ours.
        for phase in [Phase::Idle, Phase::Stopped, Phase::Ended] {
            let mut state = State::new(85.0);
            state.phase = phase;
            assert!(!state.failed("stale"));
            assert!(state.events.is_empty());
        }
    }

    #[test]
    fn error_codes_map_to_reasons() {
        assert_eq!(
            failure_reason(code::LOADING_FAILED),
            "The file could not be opened or read"
        );
        assert_eq!(
            failure_reason(code::NOTHING_TO_PLAY),
            "No audio or video stream was found"
        );
        assert_eq!(
            failure_reason(code::UNKNOWN_FORMAT),
            "Unrecognised file format"
        );
        assert!(failure_reason(code::UNSUPPORTED).contains("FFmpeg"));
        assert!(failure_reason(code::AO_INIT_FAILED).contains("audio output"));
        assert!(failure_reason(code::VO_INIT_FAILED).contains("video output"));
        assert_eq!(failure_reason(-20), "Playback failed: decoder error");
    }

    #[test]
    fn software_retry_only_where_a_decoder_was_involved() {
        assert!(retry_in_software(code::UNSUPPORTED, "auto-safe"));
        assert!(retry_in_software(code::NOTHING_TO_PLAY, "vaapi"));
        assert!(retry_in_software(-20, "auto"));
        // Already in software: nothing left to try.
        assert!(!retry_in_software(code::UNSUPPORTED, "no"));
        // Not found / not a media file: no decoder ever ran.
        assert!(!retry_in_software(code::LOADING_FAILED, "auto-safe"));
        assert!(!retry_in_software(code::UNKNOWN_FORMAT, "auto-safe"));
    }

    #[test]
    fn title_and_decoder_follow_their_properties() {
        let mut state = loaded_state();
        state.path = Some(PathBuf::from("/films/holiday.mkv"));
        assert_eq!(state.snapshot().title.as_deref(), Some("holiday"));
        state.apply(Observed::MediaTitle, Value::Text("Holiday 2024".to_owned()));
        assert_eq!(state.snapshot().title.as_deref(), Some("Holiday 2024"));

        state.apply(Observed::HwdecCurrent, Value::Text("vaapi".to_owned()));
        assert_eq!(state.snapshot().hardware_decoder.as_deref(), Some("vaapi"));
        state.apply(Observed::HwdecCurrent, Value::Text("no".to_owned()));
        assert_eq!(state.snapshot().hardware_decoder, None);
    }

    #[test]
    fn observed_ids_round_trip() {
        for property in Observed::ALL {
            assert_eq!(Observed::from_id(property as u64), Some(property));
        }
        assert_eq!(Observed::from_id(0), None);
        assert_eq!(Observed::from_id(999), None);
    }

    #[test]
    fn selection_is_applied_optimistically() {
        let tracks = [
            TrackInfo {
                id: 1,
                selected: true,
                ..Default::default()
            },
            TrackInfo {
                id: 2,
                ..Default::default()
            },
        ];
        let second = with_selection(&tracks, Some(2));
        assert!(!second[0].selected && second[1].selected);
        let off = with_selection(&tracks, None);
        assert!(off.iter().all(|track| !track.selected));
    }

    fn seek(seconds: f64, exact: bool) -> Request {
        Request::Seek { seconds, exact }
    }

    #[test]
    fn queued_seeks_collapse_to_the_last_one() {
        let batch = vec![
            seek(1.0, false),
            Request::Wake,
            seek(2.0, false),
            Request::SetPause(true),
            Request::Wake,
            seek(3.0, true),
        ];
        assert_eq!(
            coalesce(batch),
            vec![Request::SetPause(true), Request::Wake, seek(3.0, true)]
        );
    }

    #[test]
    fn coalescing_keeps_everything_else_in_order() {
        let load = Request::Load {
            path: "a.mkv".to_owned(),
            paused: false,
        };
        let batch = vec![
            Request::Stop,
            load.clone(),
            seek(5.0, true),
            Request::Shutdown,
        ];
        assert_eq!(coalesce(batch.clone()), batch);
        assert!(coalesce(Vec::new()).is_empty());
    }

    #[test]
    fn cuda_probe_is_skipped_only_without_an_nvidia_driver() {
        // No NVIDIA driver on x86: load VA-API only.
        assert_eq!(hwdec_interop(false, true), Some("vaapi"));
        // NVIDIA driver present: mpv's default, so NVDEC keeps working.
        assert_eq!(hwdec_interop(true, true), None);
        // ARM: never restricted (drmprime decoders).
        assert_eq!(hwdec_interop(false, false), None);
        assert_eq!(hwdec_interop(true, false), None);
    }

    #[test]
    fn hwdec_modes_are_classified() {
        assert!(hwdec_enabled("auto-safe"));
        assert!(hwdec_enabled("vaapi"));
        assert!(!hwdec_enabled("no"));
        assert!(!hwdec_enabled(""));
    }

    #[test]
    fn aspect_and_ab_values_map_to_mpv() {
        assert_eq!(aspect_value(None).ok().as_deref(), Some("no"));
        assert_eq!(
            aspect_value(Some(4.0 / 3.0)).ok().as_deref(),
            Some("1.333333")
        );
        assert!(aspect_value(Some(0.0)).is_err());
        assert!(aspect_value(Some(f64::NAN)).is_err());
        assert_eq!(ab_value(None), "no");
        assert_eq!(ab_value(Some(Duration::from_millis(12_345))), "12.345");
    }

    #[test]
    fn software_rasterizers_are_recognised() {
        assert!(is_software_renderer("llvmpipe (LLVM 23.1.3, 256 bits)"));
        assert!(is_software_renderer("softpipe"));
        assert!(is_software_renderer("Google SwiftShader"));
        assert!(!is_software_renderer(
            "AMD Radeon 680M (radeonsi, rembrandt)"
        ));
        assert!(!is_software_renderer("Mesa Intel(R) Xe Graphics (TGL GT2)"));
        assert!(!is_software_renderer(""));
    }

    #[test]
    fn delays_are_clamped_and_must_be_finite() {
        assert_eq!(clamp_delay(0.25).ok(), Some(0.25));
        assert_eq!(clamp_delay(1e9).ok(), Some(MAX_DELAY_SECONDS));
        assert_eq!(clamp_delay(-1e9).ok(), Some(-MAX_DELAY_SECONDS));
        assert!(clamp_delay(f64::NAN).is_err());
    }
}
