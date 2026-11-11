//! Seek bar: a custom-painted timeline (progress, hover preview, drag-to-seek).
//!
//! The bar spans the full width of the window and sits flush on top of the
//! control bar: a 14-point hit area with a 3-point track that thickens
//! smoothly to 6 points under the pointer, and a round thumb. Hovering
//! shows a guide line and a time bubble straight away (no tooltip delay);
//! the A-B repeat section is shaded.
//!
//! The bar never touches the player. It reports a [`SeekGesture`], which the
//! app turns into `Command::Scrub` (while dragging) or `Command::SeekTo`
//! (click or release). [`ScrubThrottle`] is the rate limiter the dispatcher
//! puts in front of the backend for the former.

use std::time::{Duration, Instant};

use crate::player::{PlayerSnapshot, format_time};

/// What the user did on the seek bar this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekGesture {
    /// The handle is being dragged and is currently at this position.
    Scrub(Duration),
    /// A click, or the end of a drag: seek exactly here.
    Commit(Duration),
}

/// Smallest gap between two scrub seeks: at most 20 per second reach the
/// backend, however fast the pointer moves or the UI repaints.
pub const SCRUB_INTERVAL: Duration = Duration::from_millis(50);

/// Coalesces the stream of drag targets into a bounded stream of seeks.
///
/// Only the latest target matters: a target that arrives too soon replaces
/// the one being held back, and is released by [`Self::poll`] once the
/// interval has passed. The exact seek on release is not throttled; the
/// caller issues it directly and calls [`Self::finish`].
#[derive(Debug, Default)]
pub struct ScrubThrottle {
    last_sent_at: Option<Instant>,
    last_sent: Option<Duration>,
    held_back: Option<Duration>,
    latest: Option<Duration>,
}

impl ScrubThrottle {
    /// A new drag target. Returns it if a seek should be issued right now.
    pub fn scrub(&mut self, target: Duration, now: Instant) -> Option<Duration> {
        self.latest = Some(target);
        if self.last_sent == Some(target) {
            // The pointer is resting: nothing new to seek to.
            self.held_back = None;
            return None;
        }
        if self.is_due(now) {
            self.send(target, now)
        } else {
            self.held_back = Some(target);
            None
        }
    }

    /// Call once per frame: releases the held-back target when it is due.
    pub fn poll(&mut self, now: Instant) -> Option<Duration> {
        let target = self.held_back.filter(|_| self.is_due(now))?;
        self.send(target, now)
    }

    /// How long until [`Self::poll`] has something to release, if anything
    /// is being held back. Used to schedule a repaint for that moment.
    pub fn wake_in(&self, now: Instant) -> Option<Duration> {
        self.held_back?;
        let elapsed = self
            .last_sent_at
            .map_or(SCRUB_INTERVAL, |at| now.saturating_duration_since(at));
        Some(SCRUB_INTERVAL.saturating_sub(elapsed))
    }

    /// Where the handle is while a drag is in progress. The UI shows this
    /// instead of the backend position, which lags behind during scrubbing.
    pub fn target(&self) -> Option<Duration> {
        self.latest
    }

    /// The drag ended, or an exact seek superseded it: drop all state.
    pub fn finish(&mut self) {
        *self = Self::default();
    }

    fn is_due(&self, now: Instant) -> bool {
        self.last_sent_at
            .is_none_or(|at| now.saturating_duration_since(at) >= SCRUB_INTERVAL)
    }

    fn send(&mut self, target: Duration, now: Instant) -> Option<Duration> {
        self.last_sent_at = Some(now);
        self.last_sent = Some(target);
        self.held_back = None;
        Some(target)
    }
}

use crate::player::AbLoop;
use crate::ui::palette;

/// Height of the hit area in points.
pub const HEIGHT: f32 = 14.0;
/// The track is thin at rest and thickens under the pointer.
const TRACK_HEIGHT: f32 = 3.0;
const TRACK_HEIGHT_HOVER: f32 = 6.0;
const HANDLE_RADIUS: f32 = 4.5;
const HANDLE_RADIUS_HOVER: f32 = 6.5;
/// Seconds the track takes to thicken or thin.
const GROW_TIME: f32 = 0.12;

pub fn show(ui: &mut egui::Ui, snapshot: &PlayerSnapshot) -> Option<SeekGesture> {
    let size = egui::vec2(ui.available_width(), HEIGHT);
    let Some(duration) = snapshot.duration.filter(|d| !d.is_zero()) else {
        // No seekable media: the empty track keeps its place, so the
        // control bar does not jump when a file is opened.
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().rect_filled(
            egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), TRACK_HEIGHT)),
            egui::CornerRadius::ZERO,
            palette::TRACK,
        );
        return None;
    };

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter();
    let active = response.hovered() || response.dragged();
    // 0 at rest, 1 under the pointer, eased in between.
    let grow = ui
        .ctx()
        .animate_bool_with_time(response.id.with("grow"), active, GROW_TIME);
    let thickness = TRACK_HEIGHT + (TRACK_HEIGHT_HOVER - TRACK_HEIGHT) * grow;

    // Edge to edge: the track is as wide as the window.
    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.center().y - thickness / 2.0),
        egui::pos2(rect.right(), rect.center().y + thickness / 2.0),
    );
    let radius = egui::CornerRadius::ZERO;
    painter.rect_filled(track, radius, palette::TRACK);

    let fraction_of =
        |at: Duration| (at.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0) as f32;
    let x_of = |at: Duration| track.left() + track.width() * fraction_of(at);
    let handle_x = x_of(snapshot.position);
    painter.rect_filled(
        egui::Rect::from_min_max(track.min, egui::pos2(handle_x, track.max.y)),
        radius,
        palette::ACCENT,
    );

    paint_ab_section(painter, rect, track, snapshot.ab_loop, x_of);

    let scrubbing = response.dragged() || response.clicked() || response.drag_stopped();
    let pointer = response
        .interact_pointer_pos()
        .filter(|_| scrubbing)
        .or_else(|| response.hover_pos());

    // The last drag target is remembered so that a release always commits
    // to where the handle was, even if the pointer position is no longer
    // reported on the release frame.
    let memory = response.id.with("drag_target");
    let mut gesture = None;
    if let Some(pointer) = pointer {
        let fraction = fraction_at(pointer.x, track.left(), track.width());
        let time = time_at(fraction, duration);
        let x = track.left() + track.width() * fraction;

        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, palette::NEON.gamma_multiply(0.7)),
        );

        let text = if scrubbing {
            format!("{} / {}", format_time(time), format_time(duration))
        } else {
            format_time(time)
        };
        paint_time_bubble(ui, response.id, rect, x, text);

        if response.dragged() {
            ui.data_mut(|data| data.insert_temp(memory, time));
            gesture = Some(SeekGesture::Scrub(time));
        } else if response.clicked() || response.drag_stopped() {
            gesture = Some(SeekGesture::Commit(time));
        }
    }
    if response.drag_stopped() {
        let last = ui.data_mut(|data| data.remove_temp::<Duration>(memory));
        gesture = gesture.or(last.map(SeekGesture::Commit));
    }

    // Thumb states: pale at rest, white
    // under the pointer, accent while held.
    let handle_radius = HANDLE_RADIUS + (HANDLE_RADIUS_HOVER - HANDLE_RADIUS) * grow;
    let handle_color = if response.dragged() {
        palette::ACCENT
    } else {
        palette::THUMB.lerp_to_gamma(palette::ICON_HOVER, grow)
    };
    // The thumb stays whole at both ends of the full-width track.
    let center = egui::pos2(
        handle_x.clamp(
            rect.left() + HANDLE_RADIUS_HOVER,
            (rect.right() - HANDLE_RADIUS_HOVER).max(rect.left() + HANDLE_RADIUS_HOVER),
        ),
        rect.center().y,
    );
    if grow > 0.0 {
        painter.circle_filled(
            center,
            handle_radius + 3.0,
            palette::NEON.gamma_multiply(0.35 * grow),
        );
    }
    painter.circle_filled(center, handle_radius, handle_color);

    gesture
}

/// Shade the A-B repeat section and mark both points; a lone A is a tick.
fn paint_ab_section(
    painter: &egui::Painter,
    bar: egui::Rect,
    track: egui::Rect,
    ab: AbLoop,
    x_of: impl Fn(Duration) -> f32,
) {
    if ab.is_active()
        && let (Some(a), Some(b)) = (ab.a, ab.b)
    {
        let section = egui::Rect::from_min_max(
            egui::pos2(x_of(a), track.top() - 2.0),
            egui::pos2(x_of(b), track.bottom() + 2.0),
        );
        painter.rect_filled(
            section,
            egui::CornerRadius::same(2),
            palette::ACCENT_LIGHT.gamma_multiply(0.35),
        );
    }
    for (point, label) in [(ab.a, "A"), (ab.b, "B")] {
        let Some(at) = point else { continue };
        let x = x_of(at);
        painter.line_segment(
            [
                egui::pos2(x, bar.top() + 1.0),
                egui::pos2(x, bar.bottom() - 1.0),
            ],
            egui::Stroke::new(1.5, palette::ACCENT_LIGHT),
        );
        painter.text(
            egui::pos2(x + 2.0, bar.top() - 1.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(9.0),
            palette::ACCENT_LIGHT,
        );
    }
}

/// Position along the track as 0..=1, clamped so dragging past either end
/// seeks to the start or the end.
fn fraction_at(pointer_x: f32, track_left: f32, track_width: f32) -> f32 {
    if track_width <= 0.0 {
        return 0.0;
    }
    ((pointer_x - track_left) / track_width).clamp(0.0, 1.0)
}

fn time_at(fraction: f32, duration: Duration) -> Duration {
    duration.mul_f64(f64::from(fraction.clamp(0.0, 1.0)))
}

/// Time label above the bar, centred on `x` and kept inside the bar's width.
/// Painted on the tooltip layer so it overlaps the video instead of being
/// clipped by the bottom panel.
fn paint_time_bubble(ui: &egui::Ui, id: egui::Id, bar: egui::Rect, x: f32, text: String) {
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("time")));
    let galley = painter.layout_no_wrap(text, egui::FontId::monospace(12.0), palette::TEXT);
    let padding = egui::vec2(6.0, 3.0);
    let size = galley.size() + padding * 2.0;
    let left = (x - size.x / 2.0)
        .min(bar.right() - size.x - 4.0)
        .max(bar.left() + 4.0);
    let plate = egui::Rect::from_min_size(egui::pos2(left, bar.top() - size.y - 4.0), size);

    painter.rect_filled(plate, egui::CornerRadius::same(5), palette::scrim(215));
    painter.galley(plate.min + padding, galley, palette::TEXT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_is_linear_and_clamped() {
        assert_eq!(fraction_at(100.0, 100.0, 400.0), 0.0);
        assert_eq!(fraction_at(300.0, 100.0, 400.0), 0.5);
        assert_eq!(fraction_at(500.0, 100.0, 400.0), 1.0);
        // Dragging beyond the ends pins to the ends.
        assert_eq!(fraction_at(-50.0, 100.0, 400.0), 0.0);
        assert_eq!(fraction_at(9000.0, 100.0, 400.0), 1.0);
        // A collapsed track must not divide by zero.
        assert_eq!(fraction_at(10.0, 0.0, 0.0), 0.0);
    }

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn first_scrub_goes_out_immediately() {
        let mut throttle = ScrubThrottle::default();
        let t0 = Instant::now();
        assert_eq!(throttle.scrub(10 * MS, t0), Some(10 * MS));
        assert_eq!(throttle.target(), Some(10 * MS));
    }

    #[test]
    fn rapid_targets_coalesce_to_the_latest() {
        let mut throttle = ScrubThrottle::default();
        let t0 = Instant::now();
        assert_eq!(throttle.scrub(1000 * MS, t0), Some(1000 * MS));

        // A burst of pointer events inside one interval: none is sent…
        for (i, offset) in [5u32, 10, 20, 30, 40].into_iter().enumerate() {
            let target = (2000 + i as u32 * 100) * MS;
            assert_eq!(throttle.scrub(target, t0 + offset * MS), None);
            assert_eq!(throttle.poll(t0 + offset * MS), None);
            assert_eq!(throttle.target(), Some(target), "UI follows the pointer");
        }
        // …and when the interval is over only the last one is.
        assert_eq!(throttle.poll(t0 + SCRUB_INTERVAL), Some(2400 * MS));
        assert_eq!(throttle.poll(t0 + SCRUB_INTERVAL), None, "sent only once");
    }

    #[test]
    fn at_most_one_seek_per_interval() {
        let mut throttle = ScrubThrottle::default();
        let t0 = Instant::now();
        let mut sent = 0;
        // 1 second of 1 kHz pointer events, polled every event.
        for i in 0..1000u32 {
            let now = t0 + i * MS;
            sent += u32::from(throttle.scrub(i * MS, now).is_some());
            sent += u32::from(throttle.poll(now).is_some());
        }
        let limit = 1000 / SCRUB_INTERVAL.as_millis() as u32 + 1;
        assert!(sent <= limit, "{sent} seeks in one second");
        assert!(sent >= limit - 2, "throttle must not starve: {sent}");
    }

    #[test]
    fn resting_pointer_sends_nothing_more() {
        let mut throttle = ScrubThrottle::default();
        let t0 = Instant::now();
        assert_eq!(throttle.scrub(500 * MS, t0), Some(500 * MS));
        for i in 1..10u32 {
            let now = t0 + i * SCRUB_INTERVAL;
            assert_eq!(throttle.scrub(500 * MS, now), None);
            assert_eq!(throttle.poll(now), None);
            assert_eq!(throttle.wake_in(now), None);
        }
        // Moving away and back to the sent target cancels the held-back one.
        let now = t0 + 10 * SCRUB_INTERVAL + MS;
        assert_eq!(throttle.scrub(600 * MS, now), Some(600 * MS));
        assert_eq!(throttle.scrub(700 * MS, now + MS), None);
        assert_eq!(throttle.scrub(600 * MS, now + 2 * MS), None);
        assert_eq!(throttle.poll(now + SCRUB_INTERVAL), None);
    }

    #[test]
    fn wake_in_counts_down_to_the_release() {
        let mut throttle = ScrubThrottle::default();
        let t0 = Instant::now();
        assert_eq!(throttle.wake_in(t0), None);
        throttle.scrub(100 * MS, t0);
        assert_eq!(throttle.wake_in(t0), None, "nothing held back yet");
        throttle.scrub(200 * MS, t0 + 10 * MS);
        assert_eq!(throttle.wake_in(t0 + 10 * MS), Some(40 * MS));
        assert_eq!(throttle.wake_in(t0 + 60 * MS), Some(Duration::ZERO));
    }

    #[test]
    fn finish_forgets_the_drag() {
        let mut throttle = ScrubThrottle::default();
        let t0 = Instant::now();
        throttle.scrub(100 * MS, t0);
        throttle.scrub(200 * MS, t0 + MS);
        throttle.finish();
        assert_eq!(throttle.target(), None);
        assert_eq!(throttle.poll(t0 + SCRUB_INTERVAL), None);
        // A new drag starts unthrottled.
        assert_eq!(throttle.scrub(300 * MS, t0 + 2 * MS), Some(300 * MS));
    }

    #[test]
    fn time_maps_onto_the_duration() {
        let duration = Duration::from_secs(7200);
        assert_eq!(time_at(0.0, duration), Duration::ZERO);
        assert_eq!(time_at(0.25, duration), Duration::from_secs(1800));
        assert_eq!(time_at(1.0, duration), duration);
        assert_eq!(time_at(1.5, duration), duration);
    }
}
