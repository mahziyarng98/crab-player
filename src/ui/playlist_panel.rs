//! Playlist panel: the docked list of queued media.
//!
//! A header with the item count and two icon buttons (add, clear), then
//! numbered rows. The current item carries the accent bar and a play mark
//! instead of its number; rows fill softly under the pointer. Double-click
//! plays, right-click offers Play and Remove.

use crate::playlist::Playlist;
use crate::ui::icons::{self, ButtonState, Icon};
use crate::ui::palette;

#[derive(Clone, Copy, Debug)]
pub enum PlaylistAction {
    Play(usize),
    Remove(usize),
    Clear,
    AddFiles,
}

const ROW_HEIGHT: f32 = 26.0;
const HEADER_HEIGHT: f32 = 30.0;
/// Width of the column with the item number.
const NUMBER_WIDTH: f32 = 30.0;

/// Draw the panel. Selection changes are applied to `playlist` directly;
/// everything that touches playback is returned for the app to dispatch.
/// `playing` says whether the selected item is being played right now.
pub fn show(ui: &mut egui::Ui, playlist: &mut Playlist, playing: bool) -> Option<PlaylistAction> {
    let mut action = None;

    // --- header ---------------------------------------------------------
    let (header, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HEADER_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter().clone();
    let title = painter.layout_no_wrap(
        "PLAYLIST".to_owned(),
        egui::FontId::proportional(11.0),
        palette::TEXT_WEAK,
    );
    let title_pos = egui::pos2(
        header.left() + 12.0,
        header.center().y - title.size().y / 2.0,
    );
    let title_width = title.size().x;
    painter.galley(title_pos, title, palette::TEXT_WEAK);
    if !playlist.is_empty() {
        painter.text(
            egui::pos2(title_pos.x + title_width + 8.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            playlist.len().to_string(),
            egui::FontId::proportional(11.0),
            palette::ACCENT_LIGHT,
        );
    }
    let button = |index: f32| {
        egui::Rect::from_center_size(
            egui::pos2(header.right() - 22.0 - index * 30.0, header.center().y),
            egui::vec2(28.0, 24.0),
        )
    };
    if icons::button_at(
        ui,
        button(0.0),
        Icon::Trash,
        "Clear the playlist",
        ButtonState::enabled(!playlist.is_empty()),
    ) {
        action = Some(PlaylistAction::Clear);
    }
    if icons::button_at(
        ui,
        button(1.0),
        Icon::Plus,
        "Add files…",
        ButtonState::Normal,
    ) {
        action = Some(PlaylistAction::AddFiles);
    }
    painter.hline(
        header.x_range(),
        header.bottom(),
        egui::Stroke::new(1.0, palette::SEPARATOR),
    );

    if playlist.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(28.0);
            ui.label(
                egui::RichText::new("The playlist is empty")
                    .size(13.0)
                    .color(palette::TEXT_WEAK),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Drop files here or press Ctrl+O")
                    .size(12.0)
                    .color(palette::ICON_DISABLED.lerp_to_gamma(palette::TEXT_WEAK, 0.5)),
            );
        });
        return action;
    }

    let mut clicked: Option<usize> = None;
    let mut double_clicked: Option<usize> = None;
    let mut remove: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt("playlist_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            for (index, item) in playlist.items().iter().enumerate() {
                let selected = playlist.selected() == Some(index);
                let (row, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                    egui::Sense::click(),
                );
                if ui.is_rect_visible(row) {
                    paint_row(ui, row, &response, index, &item.title, selected, playing);
                }

                if response.clicked() {
                    clicked = Some(index);
                }
                if response.double_clicked() {
                    double_clicked = Some(index);
                }

                response
                    .on_hover_text(item.path.display().to_string())
                    .context_menu(|ui| {
                        if ui.button("Play").clicked() {
                            double_clicked = Some(index);
                            ui.close();
                        }
                        if ui.button("Remove from playlist").clicked() {
                            remove = Some(index);
                            ui.close();
                        }
                    });
            }
        });

    if let Some(index) = clicked {
        playlist.set_selected(Some(index));
    }
    if let Some(index) = remove {
        action = Some(PlaylistAction::Remove(index));
    }
    if let Some(index) = double_clicked {
        action = Some(PlaylistAction::Play(index));
    }

    action
}

fn paint_row(
    ui: &egui::Ui,
    row: egui::Rect,
    response: &egui::Response,
    index: usize,
    title: &str,
    selected: bool,
    playing: bool,
) {
    let painter = ui.painter();
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        response.hovered(),
        icons::HOVER_FADE,
    );
    let plate = row.shrink2(egui::vec2(4.0, 1.0));
    if selected {
        painter.rect_filled(
            plate,
            egui::CornerRadius::same(4),
            palette::ACCENT_DEEP.gamma_multiply(0.38 + 0.12 * hover),
        );
        painter.rect_filled(
            egui::Rect::from_min_size(plate.min, egui::vec2(3.0, plate.height())),
            egui::CornerRadius::same(2),
            palette::ACCENT_LIGHT,
        );
    } else if hover > 0.0 {
        painter.rect_filled(
            plate,
            egui::CornerRadius::same(4),
            palette::HOVER.gamma_multiply(hover),
        );
    }

    // Number column: a play mark for the item that is playing.
    let number = egui::pos2(row.left() + NUMBER_WIDTH - 4.0, row.center().y);
    if selected && playing {
        let c = egui::pos2(row.left() + NUMBER_WIDTH - 10.0, row.center().y);
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(c.x - 3.5, c.y - 4.5),
                egui::pos2(c.x - 3.5, c.y + 4.5),
                egui::pos2(c.x + 4.5, c.y),
            ],
            palette::ACCENT_LIGHT,
            egui::Stroke::NONE,
        ));
    } else {
        painter.text(
            number,
            egui::Align2::RIGHT_CENTER,
            (index + 1).to_string(),
            egui::FontId::monospace(11.0),
            if selected {
                palette::ACCENT_LIGHT
            } else {
                palette::ICON_DISABLED.lerp_to_gamma(palette::TEXT_WEAK, 0.6)
            },
        );
    }

    let left = row.left() + NUMBER_WIDTH + 6.0;
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        egui::FontId::proportional(13.0),
        if selected {
            palette::TEXT
        } else {
            palette::TEXT_WEAK.lerp_to_gamma(palette::TEXT, 0.35 + 0.65 * hover)
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((row.right() - 10.0 - left).max(10.0));
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(left, row.center().y - galley.size().y / 2.0),
        galley,
        palette::TEXT,
    );
}
