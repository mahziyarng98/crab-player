//! Persisted application settings.
//!
//! Stored as TOML in the XDG config directory so that it is editable by hand
//! and survives across sessions. Keys are intentionally flat and stable.

use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::app::APP_SLUG;

/// Directory names used before the product was named Crab Player: the
/// config folder, and the folder eframe derived from the old app id for
/// window state. Kept only so existing settings can be migrated.
const LEGACY_CONFIG_DIR: &str = "player-crafted";
const LEGACY_DATA_DIR: &str = "io.github.playercrafted.playercrafted";
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not determine the XDG config directory")]
    NoConfigDir,
    #[error("could not read or write the config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not serialize the config file: {0}")]
    Toml(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 0..=100, applied to newly created player instances.
    pub volume: f32,
    pub muted: bool,
    /// Start playback immediately when a file is opened from the CLI or a dialog.
    pub autoplay: bool,
    /// Remember the last position per file and resume there.
    pub resume_playback: bool,
    pub ui: UiConfig,
    pub playback: PlaybackConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            volume: 85.0,
            muted: false,
            autoplay: true,
            resume_playback: false,
            ui: UiConfig::default(),
            playback: PlaybackConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub show_playlist: bool,
    pub controls_autohide: bool,
    pub autohide_ms: u64,
    pub always_on_top: bool,
    // Panel visibility (View menu, presets). Added after the first release:
    // `#[serde(default)]` on the struct fills them in for older files.
    pub show_menu_bar: bool,
    pub show_seek_bar: bool,
    pub show_controls: bool,
    /// Time display shows the remaining time instead of the total.
    pub show_remaining_time: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_playlist: true,
            controls_autohide: true,
            autohide_ms: 2_500,
            always_on_top: false,
            show_menu_bar: true,
            show_seek_bar: true,
            show_controls: true,
            show_remaining_time: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackConfig {
    /// libmpv hardware decoding mode: auto-safe, auto, vaapi, nvdec, no.
    pub hwdec: String,
    /// Audio output driver; empty means "let libmpv decide".
    pub audio_device: String,
    /// Enable the deinterlacer (`--deinterlace=auto` when true).
    pub deinterlace: bool,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            hwdec: "auto-safe".to_owned(),
            audio_device: String::new(),
            deinterlace: false,
        }
    }
}

impl AppConfig {
    /// `$XDG_CONFIG_HOME/crab-player/config.toml`.
    pub fn path() -> Option<PathBuf> {
        BaseDirs::new().map(|dirs| dirs.config_dir().join(APP_SLUG).join("config.toml"))
    }

    /// Never fails: a missing or malformed file yields defaults, which is the
    /// behaviour users expect from a media player. The second value is a
    /// warning for the user when the file existed but could not be used.
    pub fn load() -> (Self, Option<String>) {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => (Self::default(), None),
        }
    }

    fn load_from(path: &Path) -> (Self, Option<String>) {
        let text = match fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "could not read the config");
                return (
                    Self::default(),
                    Some(format!("{} could not be read: {err}", path.display())),
                );
            }
        };
        match Self::parse(&text) {
            Ok(config) => (config, None),
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "ignoring invalid config");
                // Settings are written back on exit, which would replace
                // the broken file. Keep a copy so nothing typed by hand is
                // lost.
                let backup = path.with_extension("invalid.toml");
                let kept = fs::copy(path, &backup).is_ok();
                let advice = if kept {
                    format!("The file was kept as {}", backup.display())
                } else {
                    format!("{} is not valid", path.display())
                };
                (Self::default(), Some(advice))
            }
        }
    }

    /// Parse a config file. Unknown keys are ignored and missing ones take
    /// their defaults, so files written by older and newer versions load.
    fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut config = toml::from_str::<Self>(text)?;
        config.volume = if config.volume.is_finite() {
            config.volume.clamp(0.0, 100.0)
        } else {
            Self::default().volume
        };
        Ok(config)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }
}

/// What [`migrate_dir`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Migration {
    /// There was nothing under the old name.
    NothingToMigrate,
    /// The new location already exists; it wins and nothing was touched.
    AlreadyPresent,
    /// This many files were copied to the new location.
    Copied(usize),
}

/// Copy `old` to `new` if `new` does not exist yet. The old directory is
/// left exactly as it was, so an older build keeps working and nothing can
/// be lost; if both exist, the new one is authoritative.
pub fn migrate_dir(old: &Path, new: &Path) -> std::io::Result<Migration> {
    if !old.is_dir() {
        return Ok(Migration::NothingToMigrate);
    }
    if new.exists() {
        return Ok(Migration::AlreadyPresent);
    }
    // Copy into a sibling first and rename, so an interrupted copy never
    // leaves a half-filled directory that would count as "already present".
    let staging = new.with_extension("migrating");
    let _ = fs::remove_dir_all(&staging);
    let copied = copy_tree(old, &staging).inspect_err(|_| {
        let _ = fs::remove_dir_all(&staging);
    })?;
    fs::rename(&staging, new)?;
    Ok(Migration::Copied(copied))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<usize> {
    fs::create_dir_all(to)?;
    let mut copied = 0;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
            copied += 1;
        }
        // Symlinks and special files are not ours; leave them behind.
    }
    Ok(copied)
}

/// Carry settings and window state over from the earlier working name.
/// Failures are logged and otherwise ignored: the player then simply starts
/// with defaults.
pub fn migrate_legacy() {
    let Some(dirs) = BaseDirs::new() else {
        return;
    };
    for (kind, old, new) in [
        (
            "config",
            dirs.config_dir().join(LEGACY_CONFIG_DIR),
            dirs.config_dir().join(APP_SLUG),
        ),
        (
            "window state",
            dirs.data_dir().join(LEGACY_DATA_DIR),
            dirs.data_dir().join(APP_SLUG),
        ),
    ] {
        match migrate_dir(&old, &new) {
            Ok(Migration::Copied(files)) => {
                tracing::info!(kind, files, from = %old.display(), to = %new.display(), "migrated");
            }
            Ok(_) => {}
            Err(err) => tracing::warn!(%err, kind, from = %old.display(), "migration failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory under the system temp dir, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "crab-player-test-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create scratch dir");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_config_is_not_a_warning() {
        let scratch = Scratch::new("missing");
        let (config, warning) = AppConfig::load_from(&scratch.0.join("config.toml"));
        assert_eq!(warning, None);
        assert_eq!(config.volume, AppConfig::default().volume);
    }

    #[test]
    fn a_corrupted_config_falls_back_to_defaults_and_is_kept() {
        let scratch = Scratch::new("corrupt");
        let path = scratch.0.join("config.toml");
        for garbage in [
            &b"volume = \"loud\"\n[ui\nshow_playlist = maybe"[..],
            &b"\xff\xfe\x00binary\x00junk"[..],
            &b"[playback]\nhwdec = 3\n"[..],
        ] {
            fs::write(&path, garbage).unwrap();
            let (config, warning) = AppConfig::load_from(&path);
            assert_eq!(config.volume, AppConfig::default().volume);
            assert_eq!(config.playback.hwdec, "auto-safe");
            let warning = warning.expect("a warning for the user");
            assert!(warning.contains("config.invalid.toml"), "{warning}");
            // The broken file survives the save on exit.
            assert_eq!(
                fs::read(scratch.0.join("config.invalid.toml")).unwrap(),
                garbage
            );
        }
    }

    #[test]
    fn an_old_config_with_unknown_and_missing_keys_loads() {
        let scratch = Scratch::new("old");
        let path = scratch.0.join("config.toml");
        fs::write(
            &path,
            "volume = 40.0\nsome_removed_key = true\n[ui]\nshow_playlist = false\nlegacy = 1\n",
        )
        .unwrap();
        let (config, warning) = AppConfig::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(config.volume, 40.0);
        assert!(!config.ui.show_playlist);
        assert!(config.ui.show_menu_bar);
        assert_eq!(config.playback.hwdec, "auto-safe");
    }

    #[test]
    fn migration_copies_everything_and_keeps_the_old_files() {
        let scratch = Scratch::new("copy");
        let (old, new) = (
            scratch.0.join("player-crafted"),
            scratch.0.join("crab-player"),
        );
        fs::create_dir_all(old.join("nested")).expect("mkdir");
        fs::write(old.join("config.toml"), PHASE_3_CONFIG).expect("write");
        fs::write(old.join("nested/app.ron"), "(window: 1)").expect("write");

        assert_eq!(
            migrate_dir(&old, &new).expect("migrate"),
            Migration::Copied(2)
        );

        // The migrated config loads with every old setting intact…
        let text = fs::read_to_string(new.join("config.toml")).expect("read new");
        let config = AppConfig::parse(&text).expect("parse migrated config");
        assert_eq!(config.volume, 40.0);
        assert!(config.muted && !config.ui.show_playlist && config.ui.always_on_top);
        assert_eq!(config.playback.hwdec, "vaapi");
        assert_eq!(
            fs::read_to_string(new.join("nested/app.ron")).expect("read"),
            "(window: 1)"
        );
        // …and the originals are untouched.
        assert_eq!(
            fs::read_to_string(old.join("config.toml")).expect("read old"),
            PHASE_3_CONFIG
        );
        assert!(old.join("nested/app.ron").is_file());
        assert!(!scratch.0.join("crab-player.migrating").exists());
    }

    #[test]
    fn migration_never_overwrites_an_existing_new_directory() {
        let scratch = Scratch::new("keep");
        let (old, new) = (scratch.0.join("old"), scratch.0.join("new"));
        fs::create_dir_all(&old).expect("mkdir");
        fs::create_dir_all(&new).expect("mkdir");
        fs::write(old.join("config.toml"), "volume = 10.0").expect("write");
        fs::write(new.join("config.toml"), "volume = 90.0").expect("write");

        assert_eq!(
            migrate_dir(&old, &new).expect("migrate"),
            Migration::AlreadyPresent
        );
        assert_eq!(
            fs::read_to_string(new.join("config.toml")).expect("read"),
            "volume = 90.0"
        );
        // Running it again is harmless.
        assert_eq!(
            migrate_dir(&old, &new).expect("migrate"),
            Migration::AlreadyPresent
        );
    }

    #[test]
    fn migration_without_old_settings_does_nothing() {
        let scratch = Scratch::new("none");
        let new = scratch.0.join("new");
        assert_eq!(
            migrate_dir(&scratch.0.join("missing"), &new).expect("migrate"),
            Migration::NothingToMigrate
        );
        assert!(!new.exists());
    }

    /// A config file exactly as the Phase 3 build wrote it.
    const PHASE_3_CONFIG: &str = r#"
volume = 40.0
muted = true
autoplay = false
resume_playback = false

[ui]
show_playlist = false
controls_autohide = true
autohide_ms = 4000
always_on_top = true

[playback]
hwdec = "vaapi"
audio_device = ""
deinterlace = false
"#;

    #[test]
    fn old_config_files_still_load_and_new_keys_default() {
        let config = AppConfig::parse(PHASE_3_CONFIG).expect("old config must parse");
        assert_eq!(config.volume, 40.0);
        assert!(config.muted && !config.autoplay);
        assert!(!config.ui.show_playlist && config.ui.always_on_top);
        assert_eq!(config.ui.autohide_ms, 4000);
        assert_eq!(config.playback.hwdec, "vaapi");
        // Keys that did not exist yet.
        assert!(config.ui.show_menu_bar && config.ui.show_seek_bar && config.ui.show_controls);
        assert!(!config.ui.show_remaining_time);
    }

    #[test]
    fn empty_and_partial_files_take_defaults() {
        let empty = AppConfig::parse("").expect("empty config must parse");
        assert_eq!(empty.volume, 85.0);
        assert!(empty.ui.show_menu_bar);

        let partial = AppConfig::parse("[ui]\nshow_seek_bar = false\n").expect("partial");
        assert!(!partial.ui.show_seek_bar);
        assert!(partial.ui.show_menu_bar && partial.ui.show_playlist);
    }

    #[test]
    fn unknown_keys_and_out_of_range_values_are_tolerated() {
        let config =
            AppConfig::parse("volume = 250.0\nfuture_option = 1\n[ui]\nnew_panel = true\n")
                .expect("unknown keys must not break loading");
        assert_eq!(config.volume, 100.0);
        assert!(AppConfig::parse("volume = \"loud\"").is_err());
    }

    #[test]
    fn panel_flags_round_trip() {
        let mut config = AppConfig::default();
        config.ui.show_menu_bar = false;
        config.ui.show_seek_bar = false;
        config.ui.show_controls = false;
        config.ui.show_remaining_time = true;

        let text = toml::to_string_pretty(&config).expect("serialize");
        let back = AppConfig::parse(&text).expect("parse back");
        assert!(!back.ui.show_menu_bar && !back.ui.show_seek_bar && !back.ui.show_controls);
        assert!(back.ui.show_remaining_time);
    }
}
