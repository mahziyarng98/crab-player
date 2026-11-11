//! Runtime loading of libmpv.
//!
//! libmpv2-sys links `-lmpv`, which would make `libmpv.so.2` a hard
//! dependency of the executable: without it the dynamic loader refuses to
//! start the program and prints a line most people never see. Instead,
//! `mpv_shim` defines every libmpv function in the binary itself as a
//! forwarder, the linker satisfies all references locally, and
//! `--as-needed` drops the library from DT_NEEDED. [`load`] then opens
//! libmpv with `dlopen` at startup, so a missing library becomes a clear
//! message on stderr and in the window, and the player still starts (with
//! the null backend).
//!
//! `CRAB_PLAYER_LIBMPV` overrides the library to open (a file name or a
//! path), for unusual installs and for testing the missing-library path.

use std::ffi::{CStr, CString, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};

/// The soname of the libmpv client API 2.x that libmpv2-sys targets.
const SONAME: &str = "libmpv.so.2";
/// Major version of the client API the bindings were generated for.
const API_MAJOR: u64 = 2;

/// Handle from `dlopen`, or why there is none.
static LIBRARY: OnceLock<Result<usize, String>> = OnceLock::new();

/// Open libmpv; later calls return the first result. Call before any
/// libmpv function is used.
pub fn load() -> Result<(), String> {
    LIBRARY
        .get_or_init(|| {
            let name = std::env::var("CRAB_PLAYER_LIBMPV").unwrap_or_else(|_| SONAME.to_owned());
            let handle = open(&name)?;
            // SAFETY: the library was just opened and exports this
            // function in every libmpv release.
            let version = unsafe { libmpv2_sys::mpv_client_api_version() } as u64;
            if version >> 16 != API_MAJOR {
                return Err(format!(
                    "{name} implements client API {}.{}, but {API_MAJOR}.x is required \
                     (mpv 0.35 or newer)",
                    version >> 16,
                    version & 0xffff
                ));
            }
            Ok(handle)
        })
        .clone()
        .map(|_| ())
}

/// Whether libmpv was loaded successfully.
pub fn status() -> Result<(), String> {
    match LIBRARY.get() {
        Some(result) => result.clone().map(|_| ()),
        None => load(),
    }
}

fn open(name: &str) -> Result<usize, String> {
    let c_name = CString::new(name).map_err(|_| format!("invalid library name {name:?}"))?;
    // SAFETY: plain dlopen of a library by name; RTLD_LOCAL keeps its
    // symbols out of the global namespace.
    let handle = unsafe { libc::dlopen(c_name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return Err(last_dl_error().unwrap_or_else(|| format!("{name}: cannot open")));
    }
    // Published before `load` asks for the API version below.
    HANDLE.store(handle, Ordering::Release);
    Ok(handle as usize)
}

/// The handle `open` produced, readable without waiting for `LIBRARY`.
static HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn last_dl_error() -> Option<String> {
    // SAFETY: dlerror returns null or a NUL-terminated string.
    let error = unsafe { libc::dlerror() };
    (!error.is_null()).then(|| {
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    })
}

/// One libmpv function pointer, resolved on first use.
pub struct Slot {
    name: &'static CStr,
    pointer: AtomicPtr<c_void>,
}

impl Slot {
    pub const fn new(name: &'static CStr) -> Self {
        Self {
            name,
            pointer: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// The function's address. Aborts if libmpv is not loaded or lacks the
    /// symbol: the bindings promise it exists, so calling on is not an
    /// option.
    pub fn get(&self) -> *mut c_void {
        let pointer = self.pointer.load(Ordering::Acquire);
        if !pointer.is_null() {
            return pointer;
        }
        let handle = HANDLE.load(Ordering::Acquire);
        if handle.is_null() {
            fatal(&format!(
                "libmpv function {} called before libmpv was loaded",
                self.name.to_string_lossy()
            ));
        }
        // SAFETY: valid handle from dlopen and a NUL-terminated name.
        let pointer = unsafe { libc::dlsym(handle, self.name.as_ptr()) };
        if pointer.is_null() {
            fatal(&format!(
                "the installed libmpv has no {}; it is older than this build of Crab Player expects",
                self.name.to_string_lossy()
            ));
        }
        self.pointer.store(pointer, Ordering::Release);
        pointer
    }
}

fn fatal(message: &str) -> ! {
    eprintln!("crab-player: {message}");
    std::process::abort();
}

/// What to tell people when libmpv cannot be loaded: the reason, then the
/// package to install on each distribution family.
pub fn missing_help(reason: &str) -> String {
    // libmpv itself was found, but a library it links could not be loaded:
    // installing libmpv again would not help.
    if !reason.contains("libmpv") {
        return format!(
            "Crab Player found libmpv ({SONAME}), but a library it needs is missing:\n  \
             {reason}\n\n\
             Install that library with your package manager, or reinstall your \
             distribution's mpv/libmpv package so its dependencies are pulled in."
        );
    }
    format!(
        "Crab Player needs libmpv ({SONAME}) for playback, but it could not be loaded:\n  \
         {reason}\n\n\
         Install it with your package manager:\n  \
         Debian / Ubuntu:   sudo apt install libmpv2\n  \
         Fedora:            sudo dnf install mpv-libs   (RPM Fusion for all codecs)\n  \
         openSUSE:          sudo zypper install libmpv2\n  \
         Arch / Manjaro:    sudo pacman -S mpv\n\n\
         The AppImage and the Flatpak bundle libmpv and need nothing extra."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_names_the_package_for_every_family() {
        let help = missing_help("libmpv.so.2: cannot open shared object file");
        for package in [
            "apt install libmpv2",
            "dnf install mpv-libs",
            "zypper install libmpv2",
            "pacman -S mpv",
        ] {
            assert!(help.contains(package), "{package} missing from:\n{help}");
        }
        assert!(help.contains("cannot open shared object file"));
    }

    #[test]
    fn a_missing_dependency_of_libmpv_is_named_as_such() {
        let help = missing_help("libasound.so.2: cannot open shared object file");
        assert!(help.contains("found libmpv"), "{help}");
        assert!(help.contains("libasound.so.2"), "{help}");
        assert!(!help.contains("apt install libmpv2"), "{help}");
    }

    #[test]
    fn an_unloadable_library_is_an_error_not_a_crash() {
        let error = open("/nonexistent/libmpv.so.2").unwrap_err();
        assert!(error.contains("/nonexistent/libmpv.so.2"), "{error}");
    }
}
