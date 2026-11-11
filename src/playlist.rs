//! Playlist model: local files, folders (scanned for media) and stream
//! URLs, which are passed to libmpv verbatim.

use std::path::{Path, PathBuf};

/// Extensions picked up when a *folder* is added, so that covers, text
/// files and the like stay out of the playlist. A file that is opened
/// explicitly is never checked against this list.
const MEDIA_EXTENSIONS: &[&str] = &[
    "mkv", "mk3d", "mka", "mp4", "m4v", "mov", "qt", "avi", "divx", "webm", "ts", "mts", "m2ts",
    "m2t", "mpg", "mpeg", "mpe", "m1v", "m2v", "vob", "wmv", "asf", "flv", "f4v", "ogv", "ogm",
    "ogx", "rm", "rmvb", "3gp", "3g2", "mxf", "dv", "nut", "y4m", "wtv", "mp3", "mp2", "flac",
    "opus", "ogg", "oga", "m4a", "m4b", "aac", "ac3", "eac3", "dts", "wav", "aiff", "aif", "wma",
    "ape", "wv", "tta", "alac", "amr", "weba",
];

#[derive(Clone, Debug)]
pub struct PlaylistItem {
    pub path: PathBuf,
    pub title: String,
}

impl PlaylistItem {
    fn from_path(path: PathBuf) -> Self {
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self { path, title }
    }
}

/// Playlists ▸ Sort orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    NameAscending,
    NameDescending,
    FolderAscending,
    FolderDescending,
    Random,
}

/// Small xorshift generator: enough to shuffle a playlist, no dependency.
#[derive(Clone, Debug)]
pub struct Shuffle(u64);

impl Shuffle {
    pub fn seeded(seed: u64) -> Self {
        // xorshift must never hold zero.
        Self(seed | 1)
    }

    /// Seed from the clock; playlists do not need cryptographic randomness.
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64);
        Self::seeded(nanos)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform-enough index in `0..len`; `len` must be non-zero.
    pub fn below(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }
}

/// Case-insensitive natural comparison key: `ep2` sorts before `ep10`.
fn natural_key(text: &str) -> Vec<(u8, u128, String)> {
    let mut key = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut digits = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                digits.push(d);
                chars.next();
            }
            // Very long digit runs saturate rather than overflow.
            key.push((
                0,
                digits.parse::<u128>().unwrap_or(u128::MAX),
                String::new(),
            ));
        } else {
            let mut word = String::new();
            while let Some(&w) = chars.peek().filter(|w| !w.is_ascii_digit()) {
                word.extend(w.to_lowercase());
                chars.next();
            }
            key.push((1, 0, word));
        }
    }
    key
}

/// The media file that follows `current` in its folder, by natural name
/// order (After Playback ▸ Play next file in the folder). `siblings` is the
/// folder listing; non-media entries are ignored.
pub fn next_in_folder(current: &Path, siblings: &[PathBuf]) -> Option<PathBuf> {
    let name_key = |p: &Path| {
        natural_key(
            &p.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default(),
        )
    };
    let current_key = name_key(current);
    siblings
        .iter()
        .filter(|path| is_media(path) && path.as_path() != current)
        .filter(|path| name_key(path) > current_key)
        .min_by(|a, b| name_key(a).cmp(&name_key(b)))
        .cloned()
}

#[derive(Default)]
pub struct Playlist {
    items: Vec<PlaylistItem>,
    selected: Option<usize>,
}

impl Playlist {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[PlaylistItem] {
        &self.items
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.filter(|i| *i < self.items.len());
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.selected = None;
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.selected = match self.selected {
                Some(selected) if selected == index => None,
                Some(selected) if selected > index => Some(selected - 1),
                other => other,
            };
        }
    }

    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.items.iter().position(|item| item.path == path)
    }

    /// Add a network stream. libmpv resolves the URL itself, so no probing is
    /// done here.
    pub fn add_url(&mut self, url: &str) {
        // The last path segment, without query or fragment, decoded.
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let title = path
            .rsplit('/')
            .next()
            .filter(|segment| !segment.is_empty())
            .map_or_else(|| url.to_owned(), crate::media::readable);
        self.items.push(PlaylistItem {
            path: PathBuf::from(url),
            title,
        });
    }

    /// Add one item exactly as given, without looking at its extension or
    /// at the file system.
    pub fn add_file(&mut self, path: &Path) {
        self.items.push(PlaylistItem::from_path(path.to_path_buf()));
    }

    /// Add a file, or recursively add all media files inside a directory.
    pub fn add_path(&mut self, path: &Path) {
        if path.is_dir() {
            for entry in walkdir::WalkDir::new(path)
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
            {
                if entry.file_type().is_file() && is_media(entry.path()) {
                    self.items.push(PlaylistItem::from_path(entry.into_path()));
                }
            }
        } else if path.is_file() {
            self.items.push(PlaylistItem::from_path(path.to_path_buf()));
        }
    }

    pub fn next_index(&self) -> Option<usize> {
        match self.selected {
            Some(index) if index + 1 < self.items.len() => Some(index + 1),
            Some(_) if !self.items.is_empty() => Some(0),
            None if !self.items.is_empty() => Some(0),
            _ => None,
        }
    }

    pub fn previous_index(&self) -> Option<usize> {
        match self.selected {
            Some(0) => self.items.len().checked_sub(1),
            Some(index) => Some(index - 1),
            None if !self.items.is_empty() => Some(self.items.len() - 1),
            _ => None,
        }
    }

    pub fn get(&self, index: usize) -> Option<&PlaylistItem> {
        self.items.get(index)
    }

    /// Reorder the entries. The selected item stays selected wherever it
    /// lands, so sorting never interrupts playback.
    pub fn sort(&mut self, order: SortOrder, rng: &mut Shuffle) {
        let selected = self.selected.map(|index| self.items[index].path.clone());
        match order {
            SortOrder::NameAscending | SortOrder::NameDescending => {
                self.items
                    .sort_by_cached_key(|item| natural_key(&item.title));
            }
            SortOrder::FolderAscending | SortOrder::FolderDescending => {
                self.items.sort_by_cached_key(|item| {
                    let folder = item.path.parent().map(|p| p.to_string_lossy().into_owned());
                    (
                        natural_key(&folder.unwrap_or_default()),
                        natural_key(&item.title),
                    )
                });
            }
            SortOrder::Random => {
                // Fisher–Yates.
                for i in (1..self.items.len()).rev() {
                    let j = rng.below(i + 1);
                    self.items.swap(i, j);
                }
            }
        }
        if matches!(
            order,
            SortOrder::NameDescending | SortOrder::FolderDescending
        ) {
            self.items.reverse();
        }
        self.selected = selected.and_then(|path| self.index_of(&path));
    }

    /// A random entry other than the selected one (shuffle playback).
    pub fn random_index(&self, rng: &mut Shuffle) -> Option<usize> {
        match (self.items.len(), self.selected) {
            (0, _) => None,
            (1, _) => Some(0),
            (len, Some(current)) => {
                // Draw from the other len - 1 entries.
                let pick = rng.below(len - 1);
                Some(if pick >= current { pick + 1 } else { pick })
            }
            (len, None) => Some(rng.below(len)),
        }
    }
}

fn is_media(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(is_media_extension)
}

/// Whether `ext` (without the dot, any case) is a playable media extension.
pub fn is_media_extension(ext: &str) -> bool {
    MEDIA_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> PlaylistItem {
        PlaylistItem::from_path(PathBuf::from(format!("/media/{name}.mkv")))
    }

    #[test]
    fn navigation_wraps_both_ways() {
        let mut playlist = Playlist {
            items: vec![item("a"), item("b"), item("c")],
            ..Playlist::default()
        };

        playlist.set_selected(None);
        assert_eq!(playlist.next_index(), Some(0));
        assert_eq!(playlist.previous_index(), Some(2));

        playlist.set_selected(Some(2));
        assert_eq!(playlist.next_index(), Some(0));
        assert_eq!(playlist.previous_index(), Some(1));
    }

    #[test]
    fn removal_keeps_a_valid_selection() {
        let mut playlist = Playlist {
            items: vec![item("a"), item("b"), item("c")],
            ..Playlist::default()
        };
        playlist.set_selected(Some(2));

        playlist.remove(0);
        assert_eq!(playlist.selected(), Some(1));
        assert_eq!(playlist.len(), 2);
    }

    fn titles(playlist: &Playlist) -> Vec<&str> {
        playlist
            .items()
            .iter()
            .map(|item| item.title.as_str())
            .collect()
    }

    #[test]
    fn name_sort_is_natural_and_keeps_the_selection() {
        let mut playlist = Playlist {
            items: vec![item("Ep10"), item("ep2"), item("Ep1"), item("bonus")],
            ..Playlist::default()
        };
        playlist.set_selected(Some(1)); // ep2
        let mut rng = Shuffle::seeded(7);

        playlist.sort(SortOrder::NameAscending, &mut rng);
        assert_eq!(titles(&playlist), ["bonus", "Ep1", "ep2", "Ep10"]);
        assert_eq!(playlist.selected(), Some(2), "ep2 is still selected");

        playlist.sort(SortOrder::NameDescending, &mut rng);
        assert_eq!(titles(&playlist), ["Ep10", "ep2", "Ep1", "bonus"]);
        assert_eq!(playlist.selected(), Some(1));
    }

    #[test]
    fn folder_sort_groups_by_folder_then_name() {
        let at = |path: &str| PlaylistItem::from_path(PathBuf::from(path));
        let mut playlist = Playlist {
            items: vec![
                at("/b/x.mkv"),
                at("/a/z.mkv"),
                at("/b/a.mkv"),
                at("/a/b.mkv"),
            ],
            ..Playlist::default()
        };
        playlist.sort(SortOrder::FolderAscending, &mut Shuffle::seeded(1));
        assert_eq!(titles(&playlist), ["b", "z", "a", "x"]);
        playlist.sort(SortOrder::FolderDescending, &mut Shuffle::seeded(1));
        assert_eq!(titles(&playlist), ["x", "a", "z", "b"]);
    }

    #[test]
    fn random_sort_is_a_permutation() {
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let mut playlist = Playlist {
            items: names.iter().map(|n| item(n)).collect(),
            ..Playlist::default()
        };
        playlist.sort(SortOrder::Random, &mut Shuffle::seeded(42));
        let mut shuffled: Vec<&str> = titles(&playlist);
        assert_ne!(shuffled, names, "seed 42 must actually move something");
        shuffled.sort_unstable();
        assert_eq!(shuffled, names);
    }

    #[test]
    fn shuffle_never_repeats_the_current_item() {
        let mut playlist = Playlist {
            items: vec![item("a"), item("b"), item("c")],
            ..Playlist::default()
        };
        let mut rng = Shuffle::seeded(3);
        let mut seen = [false; 3];
        for _ in 0..200 {
            playlist.set_selected(Some(1));
            let next = playlist.random_index(&mut rng).expect("non-empty");
            assert_ne!(next, 1);
            seen[next] = true;
        }
        assert!(seen[0] && seen[2], "both other items get picked");
        assert_eq!(Playlist::default().random_index(&mut rng), None);
    }

    #[test]
    fn next_in_folder_uses_natural_order_and_skips_non_media() {
        let dir = |names: &[&str]| -> Vec<PathBuf> {
            names
                .iter()
                .map(|n| PathBuf::from(format!("/show/{n}")))
                .collect()
        };
        let files = dir(&[
            "ep10.mkv",
            "ep2.mkv",
            "ep1.mkv",
            "notes.txt",
            "ep3.srt",
            "ep11.mp4",
        ]);
        let next = |name: &str| next_in_folder(Path::new(&format!("/show/{name}")), &files);
        assert_eq!(next("ep1.mkv"), Some(PathBuf::from("/show/ep2.mkv")));
        assert_eq!(next("ep2.mkv"), Some(PathBuf::from("/show/ep10.mkv")));
        assert_eq!(next("ep10.mkv"), Some(PathBuf::from("/show/ep11.mp4")));
        assert_eq!(next("ep11.mp4"), None);
    }

    #[test]
    fn urls_get_a_readable_title() {
        let mut playlist = Playlist::default();
        playlist.add_url("https://example.invalid/videos/example-stream.m3u8");
        assert_eq!(playlist.items()[0].title, "example-stream.m3u8");
    }
}
