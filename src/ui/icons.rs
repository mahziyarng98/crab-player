//! Icons and icon buttons, all drawn with painter primitives.
//!
//! One outline style (1.5-point strokes) throughout. Buttons have four
//! states: rest, hover, pressed and disabled, plus an accent colour for
//! toggles that are on. Hover fills fade in and out over
//! [`HOVER_FADE`] seconds instead of switching.

use crate::ui::palette;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Open,
    Previous,
    PlayFilled,
    PauseFilled,
    Stop,
    Next,
    Speaker,
    SpeakerMuted,
    Playlist,
    Fullscreen,
    Repeat,
    RepeatOne,
    Shuffle,
    Subtitles,
    Capture,
    Pin,
    Menu,
    Minimize,
    Maximize,
    Restore,
    Close,
    Plus,
    Trash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    /// A toggle that is on: drawn in the accent colour.
    Active,
    Disabled,
}

impl ButtonState {
    pub fn enabled(yes: bool) -> Self {
        if yes { Self::Normal } else { Self::Disabled }
    }

    pub fn active(on: bool) -> Self {
        if on { Self::Active } else { Self::Normal }
    }
}

/// Seconds a hover fill takes to fade in or out.
pub const HOVER_FADE: f32 = 0.12;
/// Side of the box a standard glyph is drawn in (the glyph is half of it).
pub const GLYPH_BOX: f32 = 28.0;

/// Foreground colour for each button state.
pub fn foreground(state: ButtonState, response: &egui::Response) -> egui::Color32 {
    let hot = response.hovered() || response.is_pointer_button_down_on();
    match (state, hot) {
        (ButtonState::Disabled, _) => palette::ICON_DISABLED,
        (ButtonState::Active, false) => palette::ACCENT,
        (ButtonState::Active, true) => palette::ACCENT_LIGHT,
        (ButtonState::Normal, false) => palette::ICON,
        (ButtonState::Normal, true) => palette::ICON_HOVER,
    }
}

/// Background: nothing at rest, a fill that fades in on hover, a darker
/// one while pressed. `hover_fill` lets a button use its own hover colour
/// (the close button turns red).
pub fn paint_background(
    ui: &egui::Ui,
    rect: egui::Rect,
    state: ButtonState,
    response: &egui::Response,
    hover_fill: egui::Color32,
) {
    if state == ButtonState::Disabled {
        return;
    }
    let pressed = response.is_pointer_button_down_on();
    let amount = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        response.hovered() || pressed,
        HOVER_FADE,
    );
    if amount <= 0.0 {
        return;
    }
    let fill = if pressed && hover_fill == palette::HOVER {
        palette::PRESSED
    } else {
        hover_fill
    };
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(5),
        fill.gamma_multiply(amount),
    );
}

fn sense(state: ButtonState) -> egui::Sense {
    if state == ButtonState::Disabled {
        egui::Sense::hover()
    } else {
        egui::Sense::click()
    }
}

/// An icon button occupying exactly `rect`. Returns whether it was clicked.
pub fn button_at(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: Icon,
    tooltip: &str,
    state: ButtonState,
) -> bool {
    let response = ui.allocate_rect(rect, sense(state));
    paint_background(ui, rect, state, &response, palette::HOVER);
    let glyph = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(GLYPH_BOX));
    paint_icon(ui.painter(), glyph, icon, foreground(state, &response));
    response.on_hover_text(tooltip).clicked()
}

/// Outlined text badge (`HW`/`SW`, `A-B`): a state indicator that is also
/// a button.
pub fn badge_at(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: &str,
    tooltip: &str,
    state: ButtonState,
) -> bool {
    let response = ui.allocate_rect(rect, sense(state));
    paint_background(ui, rect, state, &response, palette::HOVER);
    let color = foreground(state, &response);
    let frame = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width() - 8.0, 16.0));
    ui.painter().rect_stroke(
        frame,
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.3, color),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        frame.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::monospace(10.0),
        color,
    );
    response.on_hover_text(tooltip).clicked()
}

/// Draw `icon` centred in `rect`. The glyph fills about half of the
/// smaller side, so a 28-point box gives the standard 14-point glyph.
pub fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let c = rect.center();
    let s = rect.height().min(rect.width()) * 0.5;
    let stroke = egui::Stroke::new(1.5, color);
    let p = |x: f32, y: f32| egui::pos2(c.x + s * x, c.y + s * y);
    let outline = |points: Vec<egui::Pos2>| {
        painter.add(egui::Shape::closed_line(points, stroke));
    };

    match icon {
        // Thin outlines throughout.
        Icon::Stop => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-0.4, -0.4), p(0.4, 0.4)),
                egui::CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Previous | Icon::Next => {
            let d = if icon == Icon::Next { 1.0 } else { -1.0 };
            outline(vec![
                p(-0.3 * d, -0.42),
                p(-0.3 * d, 0.42),
                p(0.36 * d, 0.0),
            ]);
            painter.line_segment([p(0.42 * d, -0.42), p(0.42 * d, 0.42)], stroke);
        }
        Icon::Open => {
            // Eject: triangle over a bar.
            outline(vec![p(-0.42, 0.08), p(0.42, 0.08), p(0.0, -0.42)]);
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-0.42, 0.24), p(0.42, 0.4)),
                egui::CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Speaker | Icon::SpeakerMuted => {
            outline(vec![
                p(-0.5, -0.16),
                p(-0.22, -0.16),
                p(0.08, -0.44),
                p(0.08, 0.44),
                p(-0.22, 0.16),
                p(-0.5, 0.16),
            ]);
            if icon == Icon::SpeakerMuted {
                let cross = egui::Stroke::new(1.5, palette::DANGER);
                painter.line_segment([p(0.22, -0.18), p(0.52, 0.18)], cross);
                painter.line_segment([p(0.22, 0.18), p(0.52, -0.18)], cross);
            } else {
                painter.add(egui::Shape::line(
                    vec![p(0.22, -0.2), p(0.3, 0.0), p(0.22, 0.2)],
                    stroke,
                ));
                painter.add(egui::Shape::line(
                    vec![p(0.36, -0.36), p(0.5, 0.0), p(0.36, 0.36)],
                    stroke,
                ));
            }
        }
        Icon::Playlist => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-0.42, -0.46), p(0.42, 0.46)),
                egui::CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Middle,
            );
            for y in [-0.2, 0.0, 0.2] {
                painter.line_segment([p(-0.24, y), p(0.24, y)], stroke);
            }
        }
        Icon::Fullscreen => {
            let (arm, corner) = (0.3, 0.46);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let q = p(sx * corner, sy * corner);
                painter.line_segment([q, egui::pos2(q.x - sx * s * arm, q.y)], stroke);
                painter.line_segment([q, egui::pos2(q.x, q.y - sy * s * arm)], stroke);
            }
        }
        Icon::Repeat | Icon::RepeatOne => {
            // Two opposed arrows around a rounded loop.
            painter.add(egui::Shape::line(
                vec![p(-0.36, 0.12), p(-0.36, -0.22), p(0.36, -0.22)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.22, -0.36), p(0.36, -0.22), p(0.22, -0.08)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.36, -0.12), p(0.36, 0.22), p(-0.36, 0.22)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(-0.22, 0.08), p(-0.36, 0.22), p(-0.22, 0.36)],
                stroke,
            ));
            if icon == Icon::RepeatOne {
                painter.text(
                    p(0.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    "1",
                    egui::FontId::monospace(8.0),
                    color,
                );
            }
        }
        Icon::Shuffle => {
            painter.add(egui::Shape::line(
                vec![
                    p(-0.46, -0.24),
                    p(-0.12, -0.24),
                    p(0.16, 0.24),
                    p(0.42, 0.24),
                ],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    p(-0.46, 0.24),
                    p(-0.12, 0.24),
                    p(0.16, -0.24),
                    p(0.42, -0.24),
                ],
                stroke,
            ));
            for y in [-0.24, 0.24] {
                painter.add(egui::Shape::line(
                    vec![p(0.3, y - 0.12), p(0.44, y), p(0.3, y + 0.12)],
                    stroke,
                ));
            }
        }
        Icon::Subtitles => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-0.48, -0.34), p(0.48, 0.34)),
                egui::CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([p(-0.28, 0.06), p(0.04, 0.06)], stroke);
            painter.line_segment([p(0.14, 0.06), p(0.28, 0.06)], stroke);
            painter.line_segment([p(-0.28, 0.2), p(-0.12, 0.2)], stroke);
            painter.line_segment([p(-0.02, 0.2), p(0.28, 0.2)], stroke);
        }
        Icon::Capture => {
            // Viewfinder corners around a dot.
            let (arm, corner) = (0.22, 0.42);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let q = p(sx * corner, sy * corner * 0.8);
                painter.line_segment([q, egui::pos2(q.x - sx * s * arm, q.y)], stroke);
                painter.line_segment([q, egui::pos2(q.x, q.y - sy * s * arm)], stroke);
            }
            painter.circle_stroke(c, s * 0.14, stroke);
        }
        Icon::Pin => {
            // Push pin: head, body and needle, tilted.
            outline(vec![
                p(0.06, -0.48),
                p(0.48, -0.06),
                p(0.3, 0.02),
                p(-0.02, -0.3),
            ]);
            outline(vec![
                p(0.1, -0.2),
                p(0.2, -0.1),
                p(-0.06, 0.24),
                p(-0.24, 0.06),
            ]);
            painter.line_segment([p(-0.14, 0.14), p(-0.46, 0.46)], stroke);
        }
        Icon::Menu => {
            for y in [-0.28, 0.0, 0.28] {
                painter.line_segment([p(-0.4, y), p(0.4, y)], stroke);
            }
        }
        Icon::Minimize => {
            painter.line_segment([p(-0.36, 0.3), p(0.36, 0.3)], stroke);
        }
        Icon::Maximize => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-0.36, -0.34), p(0.36, 0.34)),
                egui::CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Restore => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-0.4, -0.2), p(0.2, 0.4)),
                egui::CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.add(egui::Shape::line(
                vec![
                    p(-0.2, -0.3),
                    p(-0.2, -0.4),
                    p(0.4, -0.4),
                    p(0.4, 0.2),
                    p(0.3, 0.2),
                ],
                stroke,
            ));
        }
        Icon::Close => {
            painter.line_segment([p(-0.32, -0.32), p(0.32, 0.32)], stroke);
            painter.line_segment([p(-0.32, 0.32), p(0.32, -0.32)], stroke);
        }
        Icon::Plus => {
            painter.line_segment([p(-0.36, 0.0), p(0.36, 0.0)], stroke);
            painter.line_segment([p(0.0, -0.36), p(0.0, 0.36)], stroke);
        }
        Icon::Trash => {
            painter.line_segment([p(-0.42, -0.3), p(0.42, -0.3)], stroke);
            painter.line_segment([p(-0.14, -0.44), p(0.14, -0.44)], stroke);
            outline(vec![
                p(-0.3, -0.3),
                p(-0.24, 0.44),
                p(0.24, 0.44),
                p(0.3, -0.3),
            ]);
        }
        Icon::PlayFilled => {
            painter.add(egui::Shape::convex_polygon(
                vec![p(-0.3, -0.46), p(-0.3, 0.46), p(0.46, 0.0)],
                color,
                egui::Stroke::NONE,
            ));
        }
        Icon::PauseFilled => {
            for x in [-0.34, 0.1] {
                painter.rect_filled(
                    egui::Rect::from_min_max(p(x, -0.42), p(x + 0.24, 0.42)),
                    egui::CornerRadius::same(1),
                    color,
                );
            }
        }
    }
}
