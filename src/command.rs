//! Commands are the single vocabulary shared by menus, buttons, sliders and
//! the keyboard map. Adding a feature starts here.

use std::time::Duration;

use crate::player::TrackKind;
use crate::playlist::SortOrder;

/// Layout presets (View ▸ Presets); see `docs/ui-spec.md` §8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Minimal,
    Compact,
    Normal,
}

/// What happens when the current item reaches its end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    /// Advance through the playlist once, then stop.
    #[default]
    Off,
    /// Replay the current file.
    File,
    /// Wrap around to the first item after the last one.
    Playlist,
}

/// What happens once nothing is left to play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AfterPlayback {
    #[default]
    Nothing,
    /// Continue with the next media file in the current file's folder.
    NextInFolder,
    Exit,
}

/// Quarter-turn rotation of the picture (View ▸ Rotate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotateStep {
    Clockwise,
    CounterClockwise,
    Reset,
}

/// A-B repeat (Play ▸ A-B Repeat). Set
/// each point, a 10-second section from here, and on/off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbStep {
    SetA,
    SetB,
    /// A = now, B = now + 10 s.
    TenSeconds,
    /// A, then B, then off (one key does everything).
    Cycle,
    Clear,
}

/// One of mpv's picture controls (Video ▸ Picture).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureControl {
    Brightness,
    Contrast,
    Saturation,
    Hue,
}

impl PictureControl {
    pub const ALL: [Self; 4] = [
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Hue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Brightness => "Brightness",
            Self::Contrast => "Contrast",
            Self::Saturation => "Saturation",
            Self::Hue => "Hue",
        }
    }
}

/// Step of one picture-control adjustment.
pub const PICTURE_STEP: i32 = 5;

/// One step of an audio or subtitle delay adjustment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DelayStep {
    /// Present the stream earlier (delay decreases).
    Earlier,
    /// Present the stream later (delay increases).
    Later,
    Reset,
}

// `PartialEq` only: `SetVolume` carries a float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    OpenFile,
    OpenFolder,
    AddFiles,
    ClearPlaylist,
    CopyLocation,
    CloseMedia,
    Quit,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekForward,
    SeekBackward,
    SeekForwardLarge,
    SeekBackwardLarge,
    /// Exact seek to an absolute position (click, drag release, Go To).
    SeekTo(Duration),
    /// Intermediate target while the seek bar is being dragged. Dispatch
    /// rate-limits these and seeks to the nearest keyframe.
    Scrub(Duration),
    /// Absolute volume, 0..=100 (the slider).
    SetVolume(f32),
    VolumeUp,
    VolumeDown,
    Mute,
    SpeedUp,
    SpeedDown,
    SpeedReset,
    Fullscreen,
    GoTo,
    SetRepeat(RepeatMode),
    SetAfterPlayback(AfterPlayback),
    /// Activate the track with this id, or turn the kind off with `None`.
    SelectTrack(TrackKind, Option<i64>),
    /// Switch to the next track of a kind (subtitles pass through "off").
    CycleTrack(TrackKind),
    LoadSubtitle,
    AdjustDelay(TrackKind, DelayStep),
    ToggleMenuBar,
    ToggleSeekBar,
    ToggleControls,
    TogglePlaylist,
    ToggleStatistics,
    ToggleTimeDisplay,
    ToggleOnTop,
    ApplyPreset(Preset),
    /// Open a network stream or a typed path (File ▸ Open URL…).
    OpenUrl,
    /// Force a display aspect ratio, or `None` for the file's own.
    SetAspect(Option<f64>),
    Rotate(RotateStep),
    AbRepeat(AbStep),
    /// Change a picture control by this many units (−100..=100 range).
    AdjustPicture(PictureControl, i32),
    ResetPicture,
    /// Save the current frame as an image (Capture ▸ Save Image).
    Screenshot,
    SortPlaylist(SortOrder),
    ToggleShuffle,
    /// Resize the window so the video shows at this percentage of its
    /// size (View ▸ Zoom 50/100/200 %).
    ZoomWindow(u16),
    /// Toggle switching hardware decoding on and off (the HW/SW badge).
    ToggleHardwareDecoding,
    /// Renderer Settings ▸ Display current time / file name overlays.
    ToggleClockOverlay,
    ToggleFileNameOverlay,
    Options,
    Properties,
    About,
}

/// Default key map for Phase 1.
///
/// These bindings follow common desktop-player conventions and are stored
/// here so they can later be loaded from config.
pub fn from_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<Command> {
    use egui::Key;

    if modifiers.command {
        return match key {
            Key::O => Some(Command::OpenFile),
            Key::D => Some(Command::OpenFolder),
            Key::W => Some(Command::CloseMedia),
            Key::Q => Some(Command::Quit),
            Key::P => Some(Command::Properties),
            Key::Comma => Some(Command::Options),
            Key::M => Some(Command::ToggleMenuBar),
            Key::G => Some(Command::GoTo),
            Key::Num0 => Some(Command::ToggleMenuBar),
            Key::Num1 => Some(Command::ToggleSeekBar),
            Key::Num2 => Some(Command::ToggleControls),
            Key::S if modifiers.shift => Some(Command::Screenshot),
            Key::U => Some(Command::OpenUrl),
            _ => None,
        };
    }

    match key {
        Key::Space | Key::K => Some(Command::PlayPause),
        // Shift variants first: the plain keys below would shadow them.
        Key::A if modifiers.shift => Some(Command::SetAspect(None)),
        Key::R if modifiers.shift => Some(Command::Rotate(RotateStep::CounterClockwise)),
        Key::R => Some(Command::Rotate(RotateStep::Clockwise)),
        Key::L => Some(Command::AbRepeat(AbStep::Cycle)),
        Key::A => Some(Command::CycleTrack(TrackKind::Audio)),
        Key::J => Some(Command::CycleTrack(TrackKind::Subtitle)),
        // Z/X shift the subtitles; with Shift they move the audio.
        Key::Z if modifiers.shift => {
            Some(Command::AdjustDelay(TrackKind::Audio, DelayStep::Earlier))
        }
        Key::X if modifiers.shift => Some(Command::AdjustDelay(TrackKind::Audio, DelayStep::Later)),
        Key::Z => Some(Command::AdjustDelay(
            TrackKind::Subtitle,
            DelayStep::Earlier,
        )),
        Key::X => Some(Command::AdjustDelay(TrackKind::Subtitle, DelayStep::Later)),
        Key::ArrowLeft if modifiers.shift => Some(Command::SeekBackwardLarge),
        Key::ArrowRight if modifiers.shift => Some(Command::SeekForwardLarge),
        Key::ArrowLeft => Some(Command::SeekBackward),
        Key::ArrowRight => Some(Command::SeekForward),
        Key::ArrowUp => Some(Command::VolumeUp),
        Key::ArrowDown => Some(Command::VolumeDown),
        Key::M => Some(Command::Mute),
        Key::F | Key::Enter => Some(Command::Fullscreen),
        Key::Escape => Some(Command::Fullscreen),
        Key::F9 => Some(Command::TogglePlaylist),
        Key::PageUp => Some(Command::Previous),
        Key::PageDown => Some(Command::Next),
        Key::OpenBracket => Some(Command::SpeedDown),
        Key::CloseBracket => Some(Command::SpeedUp),
        Key::Backspace => Some(Command::SpeedReset),
        Key::S => Some(Command::Stop),
        // Presets stay reachable from the keyboard because Minimal hides the
        // menu bar that would otherwise be the only way back.
        Key::Num1 => Some(Command::ApplyPreset(Preset::Minimal)),
        Key::Num2 => Some(Command::ApplyPreset(Preset::Compact)),
        Key::Num3 => Some(Command::ApplyPreset(Preset::Normal)),
        _ => None,
    }
}

/// Human-readable shortcut shown in menus.
pub fn label(command: Command) -> &'static str {
    match command {
        Command::OpenFile => "Ctrl+O",
        Command::OpenFolder => "Ctrl+D",
        Command::CloseMedia => "Ctrl+W",
        Command::Quit => "Ctrl+Q",
        Command::PlayPause => "Space",
        Command::Stop => "S",
        Command::Next => "PgDn",
        Command::Previous => "PgUp",
        Command::SeekForward => "Right",
        Command::SeekBackward => "Left",
        Command::SeekForwardLarge => "Shift+Right",
        Command::SeekBackwardLarge => "Shift+Left",
        Command::VolumeUp => "Up",
        Command::VolumeDown => "Down",
        Command::Mute => "M",
        Command::SpeedUp => "]",
        Command::SpeedDown => "[",
        Command::SpeedReset => "Backspace",
        Command::Fullscreen => "F",
        Command::GoTo => "Ctrl+G",
        Command::ToggleMenuBar => "Ctrl+M",
        Command::ToggleSeekBar => "Ctrl+1",
        Command::ToggleControls => "Ctrl+2",
        Command::TogglePlaylist => "F9",
        Command::ApplyPreset(Preset::Minimal) => "1",
        Command::ApplyPreset(Preset::Compact) => "2",
        Command::ApplyPreset(Preset::Normal) => "3",
        Command::CycleTrack(TrackKind::Audio) => "A",
        Command::CycleTrack(TrackKind::Subtitle) => "J",
        Command::AdjustDelay(TrackKind::Subtitle, DelayStep::Earlier) => "Z",
        Command::AdjustDelay(TrackKind::Subtitle, DelayStep::Later) => "X",
        Command::AdjustDelay(TrackKind::Audio, DelayStep::Earlier) => "Shift+Z",
        Command::AdjustDelay(TrackKind::Audio, DelayStep::Later) => "Shift+X",
        Command::OpenUrl => "Ctrl+U",
        Command::Screenshot => "Ctrl+Shift+S",
        Command::SetAspect(None) => "Shift+A",
        Command::Rotate(RotateStep::Clockwise) => "R",
        Command::Rotate(RotateStep::CounterClockwise) => "Shift+R",
        Command::AbRepeat(AbStep::Cycle) => "L",
        Command::Options => "Ctrl+,",
        Command::Properties => "Ctrl+P",
        Command::About
        | Command::AddFiles
        | Command::ClearPlaylist
        | Command::CopyLocation
        | Command::SetRepeat(_)
        | Command::SetAfterPlayback(_)
        | Command::SeekTo(_)
        | Command::Scrub(_)
        | Command::SetVolume(_)
        | Command::SelectTrack(..)
        | Command::LoadSubtitle
        | Command::CycleTrack(TrackKind::Video)
        | Command::AdjustDelay(TrackKind::Video, _)
        | Command::AdjustDelay(_, DelayStep::Reset)
        | Command::ToggleStatistics
        | Command::ToggleTimeDisplay
        | Command::ToggleOnTop
        | Command::SetAspect(Some(_))
        | Command::Rotate(RotateStep::Reset)
        | Command::AbRepeat(_)
        | Command::AdjustPicture(..)
        | Command::ResetPicture
        | Command::SortPlaylist(_)
        | Command::ToggleShuffle
        | Command::ZoomWindow(_)
        | Command::ToggleHardwareDecoding
        | Command::ToggleClockOverlay
        | Command::ToggleFileNameOverlay => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_modifiers_beat_plain_keys() {
        let ctrl_o = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        assert_eq!(from_key(egui::Key::O, ctrl_o), Some(Command::OpenFile));

        let none = egui::Modifiers::default();
        assert_eq!(from_key(egui::Key::Space, none), Some(Command::PlayPause));
        assert_eq!(from_key(egui::Key::Y, none), None);
    }

    #[test]
    fn digits_select_presets_and_ctrl_digits_toggle_bars() {
        let none = egui::Modifiers::default();
        let ctrl = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        assert_eq!(
            from_key(egui::Key::Num3, none),
            Some(Command::ApplyPreset(Preset::Normal))
        );
        assert_eq!(
            from_key(egui::Key::Num1, ctrl),
            Some(Command::ToggleSeekBar)
        );
        assert_eq!(from_key(egui::Key::G, ctrl), Some(Command::GoTo));
    }

    #[test]
    fn every_bound_key_is_advertised_by_its_label() {
        // A command reachable from the keyboard must show a shortcut hint.
        let none = egui::Modifiers::default();
        let ctrl = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        for key in egui::Key::ALL {
            for modifiers in [none, ctrl] {
                if let Some(command) = from_key(*key, modifiers) {
                    assert!(!label(command).is_empty(), "{command:?} has no label");
                }
            }
        }
    }

    #[test]
    fn track_and_delay_keys() {
        let none = egui::Modifiers::default();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            from_key(egui::Key::A, none),
            Some(Command::CycleTrack(TrackKind::Audio))
        );
        assert_eq!(
            from_key(egui::Key::J, none),
            Some(Command::CycleTrack(TrackKind::Subtitle))
        );
        assert_eq!(
            from_key(egui::Key::X, none),
            Some(Command::AdjustDelay(TrackKind::Subtitle, DelayStep::Later))
        );
        assert_eq!(
            from_key(egui::Key::X, shift),
            Some(Command::AdjustDelay(TrackKind::Audio, DelayStep::Later))
        );
    }

    #[test]
    fn picture_rotation_and_ab_keys() {
        let none = egui::Modifiers::default();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let ctrl_shift = egui::Modifiers {
            command: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            from_key(egui::Key::R, none),
            Some(Command::Rotate(RotateStep::Clockwise))
        );
        assert_eq!(
            from_key(egui::Key::R, shift),
            Some(Command::Rotate(RotateStep::CounterClockwise))
        );
        assert_eq!(
            from_key(egui::Key::A, shift),
            Some(Command::SetAspect(None))
        );
        // Plain A still cycles audio tracks.
        assert_eq!(
            from_key(egui::Key::A, none),
            Some(Command::CycleTrack(TrackKind::Audio))
        );
        assert_eq!(
            from_key(egui::Key::L, none),
            Some(Command::AbRepeat(AbStep::Cycle))
        );
        assert_eq!(
            from_key(egui::Key::S, ctrl_shift),
            Some(Command::Screenshot)
        );
        // Plain S stays Stop.
        assert_eq!(from_key(egui::Key::S, none), Some(Command::Stop));
    }

    #[test]
    fn shift_changes_the_seek_step() {
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            from_key(egui::Key::ArrowRight, shift),
            Some(Command::SeekForwardLarge)
        );
        assert_eq!(
            from_key(egui::Key::ArrowRight, egui::Modifiers::default()),
            Some(Command::SeekForward)
        );
    }
}
