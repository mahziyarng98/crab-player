//! A dependency-free backend used for UI development and for graceful
//! degradation when libmpv is not present. It advances a virtual clock and
//! supports every transport command, but produces no audio or video.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{AbLoop, Picture, PlaybackState, Player, PlayerEvent, PlayerSnapshot, TrackKind};

pub struct NullPlayer {
    state: PlaybackState,
    path: Option<PathBuf>,
    position: Duration,
    last_tick: Instant,
    volume: f32,
    muted: bool,
    speed: f64,
    audio_delay: f64,
    subtitle_delay: f64,
    aspect_override: Option<f64>,
    rotation: u16,
    ab_loop: AbLoop,
    picture: Picture,
}

impl NullPlayer {
    pub fn new() -> Self {
        Self {
            state: PlaybackState::Idle,
            path: None,
            position: Duration::ZERO,
            last_tick: Instant::now(),
            volume: 85.0,
            muted: false,
            speed: 1.0,
            audio_delay: 0.0,
            subtitle_delay: 0.0,
            aspect_override: None,
            rotation: 0,
            ab_loop: AbLoop::default(),
            picture: Picture::default(),
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        if self.state == PlaybackState::Playing {
            self.position = self.position.saturating_add(elapsed.mul_f64(self.speed));
        }
    }
}

impl Default for NullPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Player for NullPlayer {
    fn load(&mut self, path: &Path, start_paused: bool) -> anyhow::Result<()> {
        self.path = Some(path.to_path_buf());
        self.position = Duration::ZERO;
        self.last_tick = Instant::now();
        self.state = if start_paused {
            PlaybackState::Paused
        } else {
            PlaybackState::Playing
        };
        Ok(())
    }

    fn play(&mut self) -> anyhow::Result<()> {
        if self.path.is_some() {
            self.state = PlaybackState::Playing;
        }
        Ok(())
    }

    fn pause(&mut self) -> anyhow::Result<()> {
        if self.path.is_some() {
            self.state = PlaybackState::Paused;
        }
        Ok(())
    }

    fn toggle_pause(&mut self) -> anyhow::Result<()> {
        match self.state {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Paused => self.play(),
            _ => Ok(()),
        }
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        self.tick();
        self.state = PlaybackState::Stopped;
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> anyhow::Result<()> {
        self.position = position;
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> anyhow::Result<()> {
        self.volume = volume.clamp(0.0, 100.0);
        Ok(())
    }

    fn set_muted(&mut self, muted: bool) -> anyhow::Result<()> {
        self.muted = muted;
        Ok(())
    }

    fn set_speed(&mut self, speed: f64) -> anyhow::Result<()> {
        self.speed = speed;
        Ok(())
    }

    fn select_track(&mut self, kind: TrackKind, _id: Option<i64>) -> anyhow::Result<()> {
        // There is nothing to decode, so there are no tracks to choose from.
        anyhow::bail!(
            "the null backend has no {} tracks",
            kind.label().to_lowercase()
        )
    }

    fn add_subtitle_file(&mut self, _path: &Path) -> anyhow::Result<()> {
        anyhow::bail!("the null backend cannot load subtitles")
    }

    fn set_audio_delay(&mut self, seconds: f64) -> anyhow::Result<()> {
        self.audio_delay = seconds;
        Ok(())
    }

    fn set_subtitle_delay(&mut self, seconds: f64) -> anyhow::Result<()> {
        self.subtitle_delay = seconds;
        Ok(())
    }

    // Picture settings are remembered so the UI stays consistent, even
    // though there is no picture to apply them to.
    fn set_aspect_override(&mut self, ratio: Option<f64>) -> anyhow::Result<()> {
        self.aspect_override = ratio;
        Ok(())
    }

    fn set_rotation(&mut self, degrees: i32) -> anyhow::Result<()> {
        self.rotation = super::normalize_rotation(degrees);
        Ok(())
    }

    fn set_ab_loop(&mut self, ab: AbLoop) -> anyhow::Result<()> {
        self.ab_loop = ab;
        Ok(())
    }

    fn set_picture(&mut self, picture: Picture) -> anyhow::Result<()> {
        self.picture = picture.clamped();
        Ok(())
    }

    fn screenshot(&mut self, _path: &Path) -> anyhow::Result<()> {
        anyhow::bail!("the null backend has no video to capture")
    }

    fn set_hardware_decoding(&mut self, _enabled: bool) -> anyhow::Result<()> {
        anyhow::bail!("the null backend does not decode")
    }

    fn snapshot(&mut self) -> PlayerSnapshot {
        self.tick();
        PlayerSnapshot {
            state: self.state,
            position: self.position,
            duration: None,
            volume: self.volume,
            muted: self.muted,
            speed: self.speed,
            title: self
                .path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned()),
            media_path: self.path.clone(),
            audio_delay: self.audio_delay,
            subtitle_delay: self.subtitle_delay,
            aspect_override: self.aspect_override,
            rotation: self.rotation,
            ab_loop: self.ab_loop,
            picture: self.picture,
            ..PlayerSnapshot::default()
        }
    }

    fn drain_events(&mut self) -> Vec<PlayerEvent> {
        Vec::new()
    }

    fn backend_name(&self) -> &'static str {
        "null"
    }
}
