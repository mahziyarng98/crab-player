//! Startup timing marks.
//!
//! `RUST_LOG=crab_player::startup=debug` prints the time since process start
//! at each milestone up to the first video frame, which is how the numbers
//! in `docs/performance.md` are taken.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);

/// Call first thing in `main`.
pub fn begin() {
    START.get_or_init(Instant::now);
}

/// Log a milestone with the milliseconds elapsed since [`begin`].
pub fn mark(milestone: &str) {
    if let Some(start) = START.get() {
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(ms = format!("{ms:.1}"), milestone);
    }
}

/// Log a milestone the first time it is reached; `flag` remembers that.
pub fn mark_once(flag: &AtomicBool, milestone: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        mark(milestone);
    }
}

/// Log the first video frame reaching the screen; later calls do nothing.
#[cfg_attr(not(feature = "mpv"), allow(dead_code))]
pub fn first_frame() {
    mark_once(&FIRST_FRAME, "first video frame");
}
