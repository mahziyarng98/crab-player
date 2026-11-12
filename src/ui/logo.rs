//! The Crab Player logo as a crisp texture at any size.
//!
//! A texture that is scaled by the GPU looks soft, and small logos are
//! where that shows most. So the logo is never scaled on screen: for every
//! size it is shown at, a texture with exactly that many *physical* pixels
//! is made (from the icon set where a size matches, otherwise by a
//! high-quality downscale) and drawn 1:1 on the pixel grid. That holds at
//! scale factors 1.0, 1.25, 1.5 and 2.0 alike.

use std::collections::HashMap;

/// The icon set, embedded: `(pixels, PNG)`, ascending. The sizes that were
/// produced for the desktop icon are reused as they are.
const SOURCES: [(u32, &[u8]); 8] = [
    (
        16,
        include_bytes!(
            "../../assets/icons/hicolor/16x16/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
    (
        24,
        include_bytes!(
            "../../assets/icons/hicolor/24x24/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
    (
        32,
        include_bytes!(
            "../../assets/icons/hicolor/32x32/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
    (
        48,
        include_bytes!(
            "../../assets/icons/hicolor/48x48/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
    (
        64,
        include_bytes!(
            "../../assets/icons/hicolor/64x64/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
    (
        128,
        include_bytes!(
            "../../assets/icons/hicolor/128x128/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
    (
        256,
        include_bytes!(
            "../../assets/icons/hicolor/256x256/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
    (
        512,
        include_bytes!(
            "../../assets/icons/hicolor/512x512/apps/io.github.mahziyarng98.crab-player.png"
        ),
    ),
];

/// Icon used for the window itself (X11 `_NET_WM_ICON`).
pub const WINDOW_ICON_PNG: &[u8] = SOURCES[6].1;

/// Corner radius of the logo's plate, as a fraction of its side.
const CORNER: f32 = 0.2;

/// Largest texture ever made; anything bigger is drawn scaled.
const MAX_PIXELS: u32 = 512;

/// Index into [`SOURCES`] of the image a logo of `pixels` is made from:
/// the exact size when there is one, otherwise the next larger (downscaling
/// keeps detail, upscaling invents it), otherwise the largest.
fn source_for(pixels: u32) -> usize {
    SOURCES
        .iter()
        .position(|(size, _)| *size >= pixels)
        .unwrap_or(SOURCES.len() - 1)
}

/// Physical pixels a logo of `points` covers at `pixels_per_point`.
fn pixel_size(points: f32, pixels_per_point: f32) -> u32 {
    ((points * pixels_per_point).round() as u32).clamp(1, MAX_PIXELS)
}

/// RGBA image of the logo at exactly `pixels` × `pixels`.
fn render(pixels: u32) -> Option<image::RgbaImage> {
    let (size, png) = SOURCES[source_for(pixels)];
    let source = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .inspect_err(|err| tracing::warn!(%err, size, "could not decode the embedded logo"))
        .ok()?
        .into_rgba8();
    Some(if size == pixels {
        source
    } else {
        image::imageops::resize(
            &source,
            pixels,
            pixels,
            image::imageops::FilterType::Lanczos3,
        )
    })
}

/// Textures of the logo, one per pixel size in use.
#[derive(Default)]
pub struct Logo {
    textures: HashMap<u32, Option<egui::TextureHandle>>,
}

impl Logo {
    /// Draw the logo `points` wide with its top-left corner at `top_left`
    /// (snapped to the pixel grid). Returns the rectangle it covers.
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        top_left: egui::Pos2,
        points: f32,
        opacity: f32,
    ) -> egui::Rect {
        let scale = painter.ctx().pixels_per_point();
        let pixels = pixel_size(points, scale);
        // One texture pixel per screen pixel: the size in points follows
        // from the pixel size, and the origin sits on a pixel boundary.
        let side = pixels as f32 / scale;
        let snap = |value: f32| (value * scale).round() / scale;
        let rect = egui::Rect::from_min_size(
            egui::pos2(snap(top_left.x), snap(top_left.y)),
            egui::Vec2::splat(side),
        );

        let texture = self.textures.entry(pixels).or_insert_with(|| {
            render(pixels).map(|image| {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                );
                // NEAREST: the texture is drawn 1:1, so no filtering is
                // wanted; a filter could only blur it.
                painter.ctx().load_texture(
                    format!("logo-{pixels}"),
                    image,
                    egui::TextureOptions::NEAREST,
                )
            })
        });
        if let Some(texture) = texture {
            // The artwork is a rounded plate on an opaque dark square; the
            // corners are clipped so only the plate shows on any surface.
            let corner = (side * CORNER).round().clamp(0.0, 255.0) as u8;
            painter.add(
                egui::epaint::RectShape::filled(
                    rect,
                    egui::CornerRadius::same(corner),
                    egui::Color32::WHITE.gamma_multiply(opacity),
                )
                .with_texture(
                    texture.id(),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                ),
            );
        }
        rect
    }

    /// Like [`Self::paint`], centred on `center`.
    pub fn paint_centered(
        &mut self,
        painter: &egui::Painter,
        center: egui::Pos2,
        points: f32,
        opacity: f32,
    ) -> egui::Rect {
        let half = points / 2.0;
        self.paint(
            painter,
            egui::pos2(center.x - half, center.y - half),
            points,
            opacity,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_sizes_follow_the_scale_factor() {
        // The caption logo is 18 points.
        assert_eq!(pixel_size(18.0, 1.0), 18);
        assert_eq!(pixel_size(18.0, 1.25), 23); // 22.5 rounds up
        assert_eq!(pixel_size(18.0, 1.5), 27);
        assert_eq!(pixel_size(18.0, 2.0), 36);
        // Never zero, never larger than the largest source.
        assert_eq!(pixel_size(0.1, 1.0), 1);
        assert_eq!(pixel_size(400.0, 2.0), 512);
    }

    #[test]
    fn sources_are_never_upscaled_when_a_larger_one_exists() {
        for (pixels, expected) in [
            (16, 16),
            (18, 24),
            (23, 24),
            (24, 24),
            (27, 32),
            (36, 48),
            (96, 128),
            (140, 256),
            (300, 512),
            (512, 512),
        ] {
            assert_eq!(SOURCES[source_for(pixels)].0, expected, "{pixels} px");
        }
    }

    #[test]
    fn every_embedded_size_decodes_and_matches_its_label() {
        for (size, png) in SOURCES {
            let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                .expect("embedded PNG decodes");
            assert_eq!((image.width(), image.height()), (size, size));
        }
    }

    #[test]
    fn rendering_gives_exactly_the_requested_size() {
        for pixels in [16, 18, 23, 27, 36, 90, 140] {
            let image = render(pixels).expect("renders");
            assert_eq!((image.width(), image.height()), (pixels, pixels));
            // Not an empty picture: the plate is opaque in the middle.
            assert!(image.get_pixel(pixels / 2, pixels / 2).0[3] > 200);
        }
    }
}
