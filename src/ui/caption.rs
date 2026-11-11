//! The caption strip at the top of the window.
//!
//! From the left: the logo, the name "Crab Player", the menu button; the
//! media title in the middle; pin and the window buttons at the right. The
//! strip is also the title bar: dragging it moves the window and a double
//! click maximizes it, because the window has no decorations of its own.
//!
//! Like every UI module it only reports intents.

use crate::command::Command;
use crate::ui::icons::{self, ButtonState, Icon};
use crate::ui::logo::Logo;
use crate::ui::menu::{self, MenuContext};
use crate::ui::palette;

/// Height of the strip in points.
pub const HEIGHT: f32 = 32.0;
const LOGO: f32 = 18.0;
const BUTTON: egui::Vec2 = egui::vec2(34.0, 24.0);

/// Things only the window can do; the app turns them into viewport
/// commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// The user started dragging the strip: let the compositor move the
    /// window.
    Drag,
    ToggleMaximize,
    Minimize,
}

pub struct CaptionInput<'a> {
    pub menu: &'a MenuContext<'a>,
    pub name: &'a str,
    /// Title of the open media, shown in the middle.
    pub title: Option<&'a str>,
    pub maximized: bool,
    pub fullscreen: bool,
    /// Always-on-top cannot be requested on this display server.
    pub on_top_supported: bool,
}

#[derive(Default)]
pub struct CaptionOutput {
    pub command: Option<Command>,
    pub window: Option<WindowAction>,
}

pub fn show(ui: &mut egui::Ui, logo: &mut Logo, input: &CaptionInput<'_>) -> CaptionOutput {
    let mut output = CaptionOutput::default();
    let (rect, strip) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HEIGHT),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter().clone();
    let middle = rect.center().y;

    // The strip itself: title-bar behaviour. Buttons are added afterwards
    // and take the pointer where they are.
    if !input.fullscreen {
        if strip.double_clicked() {
            output.window = Some(WindowAction::ToggleMaximize);
        } else if strip.drag_started_by(egui::PointerButton::Primary) {
            output.window = Some(WindowAction::Drag);
        }
    }

    // --- left: logo, name, menu ---------------------------------------
    let mut x = rect.left() + 10.0;
    let mark = logo.paint(&painter, egui::pos2(x, middle - LOGO / 2.0), LOGO, 1.0);
    x = mark.right() + 7.0;
    let name = painter.layout_no_wrap(
        input.name.to_owned(),
        egui::FontId::proportional(13.5),
        palette::TEXT,
    );
    let name_width = name.size().x;
    painter.galley(
        egui::pos2(x, middle - name.size().y / 2.0),
        name,
        palette::TEXT,
    );
    x += name_width + 8.0;

    let menu_rect = egui::Rect::from_min_size(egui::pos2(x, middle - BUTTON.y / 2.0), BUTTON);
    let menu_button = ui.allocate_rect(menu_rect, egui::Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&menu_button));
    if open {
        painter.rect_filled(menu_rect, egui::CornerRadius::same(5), palette::HOVER);
    } else {
        icons::paint_background(
            ui,
            menu_rect,
            ButtonState::Normal,
            &menu_button,
            palette::HOVER,
        );
    }
    let glyph =
        egui::Rect::from_center_size(menu_rect.center(), egui::Vec2::splat(icons::GLYPH_BOX));
    icons::paint_icon(
        &painter,
        glyph,
        Icon::Menu,
        if open {
            palette::ICON_HOVER
        } else {
            icons::foreground(ButtonState::Normal, &menu_button)
        },
    );
    if let Some(command) = menu::popup(&menu_button, input.menu) {
        output.command = Some(command);
    }
    if !open {
        menu_button.on_hover_text("Menu (right-click the video for the same)");
    }
    let left_end = menu_rect.right() + 10.0;

    // --- right: pin and window buttons --------------------------------
    let mut right = rect.right() - 6.0;
    let mut next = |width: f32| {
        let button = egui::Rect::from_min_size(
            egui::pos2(right - width, middle - BUTTON.y / 2.0),
            egui::vec2(width, BUTTON.y),
        );
        right -= width + 2.0;
        button
    };

    if close_button(ui, next(BUTTON.x)) {
        output.command = Some(Command::Quit);
    }
    if input.fullscreen {
        if icons::button_at(
            ui,
            next(BUTTON.x),
            Icon::Restore,
            "Leave full screen (Esc, F or middle click)",
            ButtonState::Normal,
        ) {
            output.command = Some(Command::Fullscreen);
        }
    } else {
        let (icon, tooltip) = if input.maximized {
            (Icon::Restore, "Restore")
        } else {
            (Icon::Maximize, "Maximize")
        };
        if icons::button_at(ui, next(BUTTON.x), icon, tooltip, ButtonState::Normal) {
            output.window = Some(WindowAction::ToggleMaximize);
        }
        if icons::button_at(
            ui,
            next(BUTTON.x),
            Icon::Minimize,
            "Minimize",
            ButtonState::Normal,
        ) {
            output.window = Some(WindowAction::Minimize);
        }
        right -= 6.0;
        let (state, tooltip) = if !input.on_top_supported {
            (
                ButtonState::Disabled,
                "Always on top is not available on Wayland",
            )
        } else if input.menu.always_on_top {
            (ButtonState::Active, "Always on top: on")
        } else {
            (ButtonState::Normal, "Always on top")
        };
        let next = |width: f32| {
            egui::Rect::from_min_size(
                egui::pos2(right - width, middle - BUTTON.y / 2.0),
                egui::vec2(width, BUTTON.y),
            )
        };
        let pin = next(BUTTON.x);
        right = pin.left() - 2.0;
        if icons::button_at(ui, pin, Icon::Pin, tooltip, state) {
            output.command = Some(Command::ToggleOnTop);
        }
    }
    let right_start = right - 8.0;

    // --- middle: media title -------------------------------------------
    if let Some(title) = input.title.filter(|title| !title.is_empty()) {
        let room = right_start - left_end;
        if room > 60.0 {
            let mut job = egui::text::LayoutJob::simple_singleline(
                title.to_owned(),
                egui::FontId::proportional(12.5),
                palette::TEXT_WEAK,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let galley = painter.layout_job(job);
            // Centred in the window when there is room, otherwise in the
            // gap between the two button groups.
            let ideal = rect.center().x - galley.size().x / 2.0;
            let left = ideal.min(right_start - galley.size().x).max(left_end);
            painter.galley(
                egui::pos2(left, middle - galley.size().y / 2.0),
                galley,
                palette::TEXT_WEAK,
            );
        }
    }

    output
}

/// The close button: like the others, but red under the pointer.
fn close_button(ui: &mut egui::Ui, rect: egui::Rect) -> bool {
    let response = ui.allocate_rect(rect, egui::Sense::click());
    icons::paint_background(ui, rect, ButtonState::Normal, &response, palette::DANGER);
    let hot = response.hovered() || response.is_pointer_button_down_on();
    let glyph = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(icons::GLYPH_BOX));
    icons::paint_icon(
        ui.painter(),
        glyph,
        Icon::Close,
        if hot { palette::WELL } else { palette::ICON },
    );
    response.on_hover_text("Close (Ctrl+Q)").clicked()
}
