//! The one place colours are defined.
//!
//! Crab Player's palette comes from its logo: a violet crab on a purple-black
//! plate. Every colour used anywhere in the UI is a token in [`palette`];
//! widgets never construct colours themselves. The contrast tests at the
//! bottom keep body text at WCAG AA (4.5:1) on every surface it appears on.

use egui::Color32;

/// Colour tokens.
pub mod palette {
    use egui::Color32;

    /// Behind the video. Pure black, so letterbox bars disappear.
    pub const CANVAS: Color32 = Color32::BLACK;
    /// Text fields, deepest surfaces.
    pub const WELL: Color32 = Color32::from_rgb(0x0c, 0x06, 0x18);
    /// Side panels (playlist) and scrollbar tracks.
    pub const PANEL: Color32 = Color32::from_rgb(0x10, 0x0a, 0x1d);
    /// Menu bar, control bar, menus, dialogs.
    pub const CHROME: Color32 = Color32::from_rgb(0x15, 0x0f, 0x24);
    /// Hover fill behind buttons and menu rows.
    pub const HOVER: Color32 = Color32::from_rgb(0x26, 0x1b, 0x40);
    /// Pressed buttons.
    pub const PRESSED: Color32 = Color32::from_rgb(0x3a, 0x2a, 0x61);
    /// Separators, outlines, scrollbar thumbs.
    pub const SEPARATOR: Color32 = Color32::from_rgb(0x33, 0x27, 0x52);

    /// Body text: near-white with a hint of lavender.
    pub const TEXT: Color32 = Color32::from_rgb(0xf5, 0xf3, 0xff);
    /// Secondary text: desaturated lavender-grey.
    pub const TEXT_WEAK: Color32 = Color32::from_rgb(0xa9, 0xa3, 0xc2);
    /// Icon at rest.
    pub const ICON: Color32 = Color32::from_rgb(0x9a, 0x93, 0xb8);
    /// Icon under the pointer.
    pub const ICON_HOVER: Color32 = TEXT;
    /// Icon or placeholder that cannot be used right now.
    pub const ICON_DISABLED: Color32 = Color32::from_rgb(0x54, 0x4a, 0x75);

    /// The accent: progress, active toggles, selection. The logo's mid
    /// violet, kept inside the brand range `#7C3AED`..`#C084FC`.
    pub const ACCENT: Color32 = Color32::from_rgb(0x91, 0x46, 0xf5);
    /// Darker end of the brand range: selection fills behind text.
    pub const ACCENT_DEEP: Color32 = Color32::from_rgb(0x7c, 0x3a, 0xed);
    /// Lighter end: accent text and active-and-hovered toggles.
    pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0xc0, 0x84, 0xfc);
    /// Neon rim, used sparingly: the seek thumb's hover halo and focus rings.
    pub const NEON: Color32 = Color32::from_rgb(0xe8, 0x79, 0xf9);
    /// Seek thumb at rest.
    pub const THUMB: Color32 = Color32::from_rgb(0xdd, 0xd6, 0xfe);
    /// Unplayed part of the seek track.
    pub const TRACK: Color32 = Color32::from_rgb(0x2a, 0x1f, 0x48);
    /// Errors and the muted cross.
    pub const DANGER: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);

    /// Translucent black plate behind text drawn over the video.
    pub const fn scrim(alpha: u8) -> Color32 {
        Color32::from_black_alpha(alpha)
    }
}

/// Dark theme built from [`palette`].
pub fn install_theme(ctx: &egui::Context) {
    use palette::*;

    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = CHROME;
        v.window_fill = CHROME;
        v.window_stroke = egui::Stroke::new(1.0, SEPARATOR);
        v.extreme_bg_color = WELL;
        v.faint_bg_color = PANEL;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT_DEEP;
        v.selection.stroke = egui::Stroke::new(1.0, NEON);
        v.hyperlink_color = ACCENT_LIGHT;
        // Hover states are fills, not colour shifts, so the accent keeps
        // its meaning (ui-spec §15).
        v.widgets.inactive.weak_bg_fill = PANEL;
        v.widgets.inactive.bg_fill = SEPARATOR;
        v.widgets.hovered.weak_bg_fill = HOVER;
        v.widgets.hovered.bg_fill = PRESSED;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, PRESSED);
        v.widgets.active.weak_bg_fill = PRESSED;
        v.widgets.active.bg_fill = ACCENT;
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, SEPARATOR);
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
        v.widgets.open.weak_bg_fill = HOVER;
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.scroll.bar_width = 10.0;
    });
}

/// WCAG 2.x relative luminance of an opaque colour.
#[cfg_attr(not(test), allow(dead_code))]
fn luminance(color: Color32) -> f64 {
    let channel = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

/// WCAG contrast ratio between two opaque colours, 1.0..=21.0.
#[cfg_attr(not(test), allow(dead_code))]
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::palette::*;
    use super::*;

    const SURFACES: [(&str, Color32); 5] = [
        ("well", WELL),
        ("panel", PANEL),
        ("chrome", CHROME),
        ("hover", HOVER),
        ("pressed", PRESSED),
    ];

    #[test]
    fn contrast_matches_known_values() {
        assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 1e-9);
        assert!((contrast(Color32::BLACK, Color32::BLACK) - 1.0).abs() < 1e-9);
        // #767676 on white is the classic "just passes AA" grey.
        let grey = Color32::from_rgb(0x76, 0x76, 0x76);
        assert!((contrast(grey, Color32::WHITE) - 4.54).abs() < 0.01);
    }

    #[test]
    fn body_text_meets_aa_on_every_surface() {
        for (name, surface) in SURFACES {
            for (label, text) in [("text", TEXT), ("weak text", TEXT_WEAK)] {
                let ratio = contrast(text, surface);
                assert!(ratio >= 4.5, "{label} on {name}: {ratio:.2}:1");
            }
        }
        // Selected rows and text selections.
        assert!(contrast(TEXT, ACCENT_DEEP) >= 4.5);
        // Accent-coloured text (speed badge, links) on the bars.
        assert!(contrast(ACCENT_LIGHT, CHROME) >= 4.5);
        assert!(contrast(ACCENT_LIGHT, PANEL) >= 4.5);
    }

    #[test]
    fn graphics_meet_the_three_to_one_rule() {
        // Icons and state indicators are graphics, not text (WCAG 1.4.11).
        for (name, surface) in [("chrome", CHROME), ("hover", HOVER)] {
            for (label, colour) in [("icon", ICON), ("accent", ACCENT), ("neon", NEON)] {
                let ratio = contrast(colour, surface);
                assert!(ratio >= 3.0, "{label} on {name}: {ratio:.2}:1");
            }
        }
        assert!(contrast(ACCENT, TRACK) >= 3.0, "played vs unplayed track");
        assert!(contrast(DANGER, CHROME) >= 4.5);
    }

    #[test]
    fn disabled_is_visibly_dimmer_than_enabled() {
        assert!(contrast(ICON_DISABLED, CHROME) < contrast(ICON, CHROME));
        assert!(contrast(ICON_DISABLED, CHROME) >= 1.5, "still discernible");
    }
}
