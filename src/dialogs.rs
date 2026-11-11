//! Native file dialogs that never block the UI thread.
//!
//! A portal file dialog stays open for as long as the user browses. If the
//! UI thread waited for it, the window would stop answering the
//! compositor's pings, and compositors such as Hyprland then put an
//! "application not responding" prompt on top of the dialog. So every
//! dialog runs on its own thread and reports back through a channel; the
//! UI keeps drawing (and the video keeps playing) meanwhile.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

/// File name extensions offered by the "Media" filter of the Open dialog.
/// The dialog also has an "All files" filter: this list only narrows what
/// is shown, it never limits what can be opened.
const MEDIA_FILTER: &[&str] = &[
    "mkv", "mk3d", "mka", "mp4", "m4v", "mov", "qt", "avi", "divx", "webm", "ts", "mts", "m2ts",
    "mpg", "mpeg", "mpe", "m2v", "vob", "wmv", "asf", "flv", "f4v", "ogv", "ogm", "ogx", "rm",
    "rmvb", "3gp", "3g2", "mxf", "dv", "nut", "y4m", "mp3", "mp2", "flac", "opus", "ogg", "oga",
    "m4a", "m4b", "aac", "ac3", "eac3", "dts", "wav", "aiff", "wma", "ape", "wv", "tta", "alac",
];

/// What a dialog was opened for; decides what happens with the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// File ▸ Open File…: open and play.
    OpenFile,
    /// File ▸ Open Folder…: add the folder and play its first item.
    OpenFolder,
    /// Playlist ▸ Add Files…: append without interrupting playback.
    AddFiles,
    /// Subtitles ▸ Load…: attach to the playing media.
    LoadSubtitle,
}

/// A finished dialog. `paths` is empty when the user cancelled.
#[derive(Debug)]
pub struct Picked {
    pub purpose: Purpose,
    pub paths: Vec<PathBuf>,
}

pub struct Dialogs {
    sender: Sender<Picked>,
    receiver: Receiver<Picked>,
    /// The thread of the dialog that is on screen. A second dialog is not
    /// opened on top of it.
    running: Option<std::thread::JoinHandle<()>>,
}

impl Default for Dialogs {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            running: None,
        }
    }
}

impl Dialogs {
    /// Show a dialog. Returns `false` (and does nothing) while another one
    /// is still open, or if the dialog thread could not be started.
    pub fn open(
        &mut self,
        purpose: Purpose,
        subtitle_extensions: &[&str],
        repaint: &egui::Context,
    ) -> bool {
        if self.is_open() {
            return false;
        }
        let dialog = match purpose {
            Purpose::OpenFile => rfd::AsyncFileDialog::new()
                .set_title("Open File")
                .add_filter("Media", MEDIA_FILTER)
                .add_filter("All files", &["*"]),
            Purpose::OpenFolder => rfd::AsyncFileDialog::new().set_title("Open Folder"),
            Purpose::AddFiles => rfd::AsyncFileDialog::new()
                .set_title("Add Files")
                .add_filter("Media", MEDIA_FILTER)
                .add_filter("All files", &["*"]),
            Purpose::LoadSubtitle => rfd::AsyncFileDialog::new()
                .set_title("Load Subtitle")
                .add_filter("Subtitles", subtitle_extensions)
                .add_filter("All files", &["*"]),
        };

        let (sender, repaint) = (self.sender.clone(), repaint.clone());
        let spawned = std::thread::Builder::new()
            .name("file-dialog".to_owned())
            .spawn(move || {
                let paths = pollster::block_on(async {
                    let one = |handle: Option<rfd::FileHandle>| {
                        handle
                            .map(|handle| vec![handle.path().to_path_buf()])
                            .unwrap_or_default()
                    };
                    match purpose {
                        Purpose::OpenFile | Purpose::LoadSubtitle => one(dialog.pick_file().await),
                        Purpose::OpenFolder => one(dialog.pick_folder().await),
                        Purpose::AddFiles => dialog
                            .pick_files()
                            .await
                            .unwrap_or_default()
                            .iter()
                            .map(|handle| handle.path().to_path_buf())
                            .collect(),
                    }
                });
                let _ = sender.send(Picked { purpose, paths });
                // The UI may be idle; wake it to pick the result up.
                repaint.request_repaint();
            });

        match spawned {
            Ok(handle) => {
                self.running = Some(handle);
                true
            }
            Err(err) => {
                tracing::error!(%err, "could not start the file dialog thread");
                false
            }
        }
    }

    /// The result of a dialog that has been closed since the last call.
    pub fn poll(&mut self) -> Option<Picked> {
        let picked = self.receiver.try_recv().ok()?;
        self.running = None;
        Some(picked)
    }

    /// A dialog is on screen. A dialog thread that died without reporting
    /// does not count, so one failure cannot disable Open for good.
    pub fn is_open(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
}
