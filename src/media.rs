//! Turning what a user hands us (a command-line argument, a `file://` URI
//! from a file manager, a dropped item, text typed into Open URL) into
//! something the player can open, and saying why when it cannot.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// One thing to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A local file or folder, always absolute.
    Path(PathBuf),
    /// A stream, passed to the playback engine as it is.
    Url(String),
}

/// Why something could not be opened. The text is shown to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    NotFound(PathBuf),
    PermissionDenied(PathBuf),
    /// A folder without a single media file in it.
    NoMediaInFolder(PathBuf),
    /// A `file:` URI that cannot be turned into a path.
    BadUri(String),
    Empty,
}

impl OpenError {
    /// Short headline for the error banner.
    pub fn title(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "File not found",
            Self::PermissionDenied(_) => "Permission denied",
            Self::NoMediaInFolder(_) => "No media files in this folder",
            Self::BadUri(_) => "Invalid file address",
            Self::Empty => "Nothing to open",
        }
    }

    /// What the error is about: the path or the text that was given.
    pub fn subject(&self) -> String {
        match self {
            Self::NotFound(path) | Self::PermissionDenied(path) | Self::NoMediaInFolder(path) => {
                path.display().to_string()
            }
            Self::BadUri(text) => text.clone(),
            Self::Empty => String::new(),
        }
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let subject = self.subject();
        if subject.is_empty() {
            f.write_str(self.title())
        } else {
            write!(f, "{}: {subject}", self.title())
        }
    }
}

/// Interpret one argument. `base` is the directory relative paths are
/// resolved against (the working directory).
///
/// - `file:` URIs (what desktop entries with `%U` and file managers pass)
///   are percent-decoded into a path;
/// - anything else with a `scheme://` is a stream URL, whatever the scheme:
///   the playback engine decides what it can open, not a list here;
/// - everything else is a path.
pub fn parse(text: &str, base: &Path) -> Result<Source, OpenError> {
    let text = text.trim_matches(['\r', '\n']);
    if text.trim().is_empty() {
        return Err(OpenError::Empty);
    }
    if let Some(rest) = strip_scheme(text, "file") {
        return file_uri_path(rest)
            .map(Source::Path)
            .ok_or_else(|| OpenError::BadUri(text.to_owned()));
    }
    if is_url(text) {
        return Ok(Source::Url(text.trim().to_owned()));
    }
    let path = Path::new(text);
    Ok(Source::Path(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }))
}

/// `scheme://…` with a syntactically valid scheme (RFC 3986), other than a
/// one-letter one, which is more likely a Windows drive than a protocol.
pub fn is_url(text: &str) -> bool {
    let Some((scheme, _)) = text.trim_start().split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    scheme.len() > 1
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// `rest` of `scheme:rest`, matching the scheme without regard to case.
fn strip_scheme<'a>(text: &'a str, scheme: &str) -> Option<&'a str> {
    let (head, rest) = text.split_once(':')?;
    head.eq_ignore_ascii_case(scheme).then_some(rest)
}

/// The path of a `file:` URI, given everything after `file:`. Accepts
/// `///path`, `//localhost/path` and `/path`; other hosts are refused,
/// because the file is then not on this machine.
fn file_uri_path(rest: &str) -> Option<PathBuf> {
    let path = if let Some(after) = rest.strip_prefix("//") {
        let slash = after.find('/')?;
        let (host, path) = after.split_at(slash);
        if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
            return None;
        }
        path
    } else {
        rest
    };
    if !path.starts_with('/') {
        return None;
    }
    // A query or fragment is not part of a file name in a URI; a literal
    // `?` or `#` in a file name arrives percent-encoded.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let bytes = percent_decode(path)?;
    if bytes.contains(&0) {
        return None;
    }
    Some(PathBuf::from(os_string(bytes)))
}

/// Decode `%XX` escapes. `None` for a truncated or non-hex escape.
fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Some(out)
}

#[cfg(unix)]
fn os_string(bytes: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt as _;
    OsString::from_vec(bytes)
}

#[cfg(not(unix))]
fn os_string(bytes: Vec<u8>) -> OsString {
    OsString::from(String::from_utf8_lossy(&bytes).into_owned())
}

/// A URL segment as people read it: `%20` becomes a space. Text that is
/// not valid percent-encoded UTF-8 is returned unchanged.
pub fn readable(text: &str) -> String {
    percent_decode(text)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| text.to_owned())
}

/// The entries of a `text/uri-list` (what a drag from a file manager
/// carries): one URI per line, CRLF separated, `#` lines are comments.
pub fn parse_uri_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Check that a local path can be opened at all, so the user gets the real
/// reason instead of a generic playback error.
pub fn check_path(path: &Path) -> Result<(), OpenError> {
    use std::io::ErrorKind;

    let classify = |err: std::io::Error| match err.kind() {
        ErrorKind::PermissionDenied => OpenError::PermissionDenied(path.to_path_buf()),
        _ => OpenError::NotFound(path.to_path_buf()),
    };
    let metadata = std::fs::metadata(path).map_err(classify)?;
    if metadata.is_dir() {
        std::fs::read_dir(path).map(drop).map_err(classify)
    } else if metadata.is_file() {
        std::fs::File::open(path).map(drop).map_err(classify)
    } else {
        // FIFOs and devices can be valid inputs; let the engine try.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> Result<Source, OpenError> {
        parse(text, Path::new("/work dir"))
    }

    #[test]
    fn plain_paths_stay_paths_and_relative_ones_are_anchored() {
        assert_eq!(
            path("/media/My Film (2024).mkv"),
            Ok(Source::Path(PathBuf::from("/media/My Film (2024).mkv")))
        );
        assert_eq!(
            path("clips/فیلم تست.mp4"),
            Ok(Source::Path(PathBuf::from("/work dir/clips/فیلم تست.mp4")))
        );
        // A percent sign in a plain path is just a character.
        assert_eq!(
            path("/media/100%20real.mkv"),
            Ok(Source::Path(PathBuf::from("/media/100%20real.mkv")))
        );
        // A colon alone does not make a URL.
        assert_eq!(
            path("/media/Part 2: The End.mkv"),
            Ok(Source::Path(PathBuf::from("/media/Part 2: The End.mkv")))
        );
    }

    #[test]
    fn file_uris_are_decoded() {
        assert_eq!(
            path("file:///home/me/My%20Videos/a%20b.mkv"),
            Ok(Source::Path(PathBuf::from("/home/me/My Videos/a b.mkv")))
        );
        assert_eq!(
            path("file:///v/%D9%81%DB%8C%D9%84%D9%85%20%D8%AA%D8%B3%D8%AA%201.mp4"),
            Ok(Source::Path(PathBuf::from("/v/فیلم تست 1.mp4")))
        );
        // Reserved characters that file managers escape.
        assert_eq!(
            path("file:///v/50%25%20%23hash%20%3Fq%20%26%20%5Bx%5D.mkv"),
            Ok(Source::Path(PathBuf::from("/v/50% #hash ?q & [x].mkv")))
        );
        assert_eq!(
            path("file://localhost/v/a.mkv"),
            Ok(Source::Path(PathBuf::from("/v/a.mkv")))
        );
        assert_eq!(
            path("FILE:/v/a.mkv"),
            Ok(Source::Path(PathBuf::from("/v/a.mkv")))
        );
    }

    #[test]
    fn file_uris_keep_non_utf8_names() {
        use std::os::unix::ffi::OsStrExt as _;
        let Ok(Source::Path(decoded)) = path("file:///v/caf%E9.avi") else {
            panic!("not a path");
        };
        assert_eq!(decoded.as_os_str().as_bytes(), b"/v/caf\xe9.avi");
    }

    #[test]
    fn broken_file_uris_are_reported() {
        for bad in [
            "file://otherhost/v/a.mkv",
            "file:///v/a%2",
            "file:///v/a%zz.mkv",
            "file:relative.mkv",
            "file:///v/a%00b.mkv",
        ] {
            assert_eq!(path(bad), Err(OpenError::BadUri(bad.to_owned())), "{bad}");
        }
        assert_eq!(path("  "), Err(OpenError::Empty));
    }

    #[test]
    fn any_scheme_is_a_stream() {
        for url in [
            "http://example.com/a.mp4",
            "https://example.com/a%20b.m3u8?x=1",
            "rtsp://cam/live",
            "rtmp://host/app",
            "mms://host/a",
            "udp://239.0.0.1:1234",
            "srt://host:9000",
            "smb://nas/share/a.mkv",
            "dvd://",
        ] {
            assert_eq!(path(url), Ok(Source::Url(url.to_owned())), "{url}");
            assert!(is_url(url));
        }
        assert!(!is_url("/media/movie.mkv"));
        assert!(!is_url("movie.mkv"));
        assert!(!is_url("c://odd"));
        assert!(!is_url("no scheme://x"));
    }

    #[test]
    fn url_segments_are_shown_decoded() {
        assert_eq!(readable("2%20second.mkv"), "2 second.mkv");
        assert_eq!(readable("%D9%81%DB%8C%D9%84%D9%85.mp4"), "فیلم.mp4");
        // Not decodable: shown as it is.
        assert_eq!(readable("100%.mp4"), "100%.mp4");
        assert_eq!(readable("%FF.mp4"), "%FF.mp4");
    }

    #[test]
    fn uri_lists_skip_comments_and_blank_lines() {
        assert_eq!(
            parse_uri_list("# from a file manager\r\nfile:///a%20b.mkv\r\n\r\nfile:///c.mp4\r\n"),
            vec!["file:///a%20b.mkv".to_owned(), "file:///c.mp4".to_owned()]
        );
        assert!(parse_uri_list("").is_empty());
    }

    #[test]
    fn missing_and_unreadable_paths_are_told_apart() {
        let dir = std::env::temp_dir().join(format!("crab-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip with space.mkv");
        std::fs::write(&file, b"x").unwrap();

        assert_eq!(check_path(&file), Ok(()));
        assert_eq!(check_path(&dir), Ok(()));
        let missing = dir.join("nope.mkv");
        assert_eq!(check_path(&missing), Err(OpenError::NotFound(missing)));

        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root can read anything; the check is meaningless there.
        if std::fs::File::open(&file).is_err() {
            assert_eq!(
                check_path(&file),
                Err(OpenError::PermissionDenied(file.clone()))
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(
            OpenError::NotFound(PathBuf::from("/v/a.mkv")).to_string(),
            "File not found: /v/a.mkv"
        );
        assert_eq!(OpenError::Empty.to_string(), "Nothing to open");
    }
}
