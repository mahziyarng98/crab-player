//! Whether egui's system-font fallback can be enabled safely.
//!
//! eframe's `system_fonts` provider enumerates fonts through fontconfig on a
//! background thread, and the enumeration (fontique) panics when fontconfig
//! has no configuration or no fonts at all, which aborts the whole process.
//! Every desktop has both; minimal systems and containers may not. There the
//! player starts with egui's built-in fonts only, instead of crashing.

use std::path::{Path, PathBuf};

/// True when fontconfig is configured and at least one font is installed.
pub fn system_fonts_available() -> bool {
    let configured = fontconfig_configured(
        std::env::var_os("FONTCONFIG_FILE").map(PathBuf::from),
        Path::new("/etc/fonts/fonts.conf"),
    );
    configured && any_font_in(&font_dirs())
}

fn fontconfig_configured(override_file: Option<PathBuf>, default_file: &Path) -> bool {
    override_file.is_some_and(|file| file.is_file()) || default_file.is_file()
}

/// The usual font locations: the XDG data directories and the per-user ones.
fn font_dirs() -> Vec<PathBuf> {
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let mut dirs: Vec<PathBuf> = data_dirs
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join("fonts"))
        .collect();
    if let Some(home) = std::env::home_dir() {
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    dirs
}

/// Whether any of `dirs` holds a font file (searched a few levels deep,
/// stopping at the first one).
fn any_font_in(dirs: &[PathBuf]) -> bool {
    const EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc", "pcf", "gz", "woff", "woff2"];
    dirs.iter().any(|dir| {
        walkdir::WalkDir::new(dir)
            .max_depth(4)
            .into_iter()
            .filter_map(Result::ok)
            .any(|entry| {
                entry.file_type().is_file()
                    && entry
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("crab-player-fonts-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_font_file_somewhere_below_counts() {
        let scratch = Scratch::new("found");
        let nested = scratch.0.join("truetype/dejavu");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("DejaVuSans.TTF"), b"").unwrap();
        assert!(any_font_in(std::slice::from_ref(&scratch.0)));
    }

    #[test]
    fn empty_or_missing_font_directories_do_not() {
        let scratch = Scratch::new("empty");
        std::fs::write(scratch.0.join("fonts.dir"), b"").unwrap();
        assert!(!any_font_in(&[
            scratch.0.clone(),
            scratch.0.join("missing")
        ]));
    }

    #[test]
    fn fontconfig_needs_a_configuration_file() {
        let scratch = Scratch::new("config");
        let conf = scratch.0.join("fonts.conf");
        assert!(!fontconfig_configured(None, &conf));
        assert!(!fontconfig_configured(
            Some(scratch.0.join("nope.conf")),
            &conf
        ));
        std::fs::write(&conf, b"<fontconfig/>").unwrap();
        assert!(fontconfig_configured(None, &conf));
        assert!(fontconfig_configured(
            Some(conf.clone()),
            &scratch.0.join("absent")
        ));
    }
}
