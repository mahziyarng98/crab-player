//! The error banner: a small card at the top of the video area that names
//! what went wrong. It never blocks input and can be dismissed.

use super::{Notice, palette};

/// Draw `notice` at the top centre of `canvas`. Returns `true` when the
/// user dismissed it.
pub fn show(ctx: &egui::Context, canvas: egui::Rect, notice: &Notice) -> bool {
    let mut dismissed = false;
    let max_width = (canvas.width() - 32.0).clamp(160.0, 560.0);

    // Fade out over the last second instead of vanishing.
    let left = Notice::LIFETIME.saturating_sub(notice.since.elapsed());
    let opacity = left.as_secs_f32().clamp(0.0, 1.0);

    egui::Area::new(egui::Id::unique("error_notice"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(canvas.center().x, canvas.top() + 14.0))
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            egui::Frame::new()
                .fill(palette::PANEL)
                .stroke(egui::Stroke::new(1.0, palette::DANGER))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_max_width(max_width);
                    ui.horizontal(|ui| {
                        let (mark, _) =
                            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(mark.center(), 9.0, palette::DANGER);
                        ui.painter().text(
                            mark.center(),
                            egui::Align2::CENTER_CENTER,
                            "!",
                            egui::FontId::proportional(13.0),
                            palette::PANEL,
                        );

                        ui.vertical(|ui| {
                            ui.set_max_width(max_width - 70.0);
                            ui.label(
                                egui::RichText::new(&notice.title)
                                    .size(14.0)
                                    .strong()
                                    .color(palette::TEXT),
                            );
                            if !notice.detail.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&notice.detail)
                                            .size(12.5)
                                            .color(palette::TEXT_WEAK),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&notice.detail);
                            }
                            if notice.more > 0 {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "and {} more problem{}",
                                        notice.more,
                                        if notice.more == 1 { "" } else { "s" }
                                    ))
                                    .size(12.0)
                                    .color(palette::TEXT_WEAK),
                                );
                            }
                        });

                        let close = ui.add(
                            egui::Button::new(
                                egui::RichText::new("✕")
                                    .size(12.0)
                                    .color(palette::TEXT_WEAK),
                            )
                            .frame(false),
                        );
                        dismissed = close.on_hover_text("Dismiss").clicked();
                    });
                });
        });

    // Keep the countdown and the fade moving while nothing else repaints.
    ctx.request_repaint_after(std::time::Duration::from_millis(if opacity < 1.0 {
        33
    } else {
        250
    }));
    dismissed
}
