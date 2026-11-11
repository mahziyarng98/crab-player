//! Video surface.
//!
//! Paints the black canvas, the libmpv picture, the OSD and the "no media"
//! placeholder. The picture is composited by [`VideoSurface`]: an
//! `egui::PaintCallback` asks the backend's `VideoRenderer` to draw the
//! current frame into an offscreen FBO sized in physical pixels, then blits
//! it into the framebuffer eframe is painting to. See `docs/architecture.md`
//! §5 for the whole pipeline.
//!
//! All `unsafe` here is raw OpenGL through `glow`, confined to the private
//! `surface` module.

use crate::player::{PlaybackState, PlayerSnapshot, format_time};
use crate::ui::palette;

#[cfg(feature = "mpv")]
pub use surface::VideoSurface;

/// UI-only builds have no video renderer, so the surface can never exist.
#[cfg(not(feature = "mpv"))]
pub enum VideoSurface {}

#[cfg(not(feature = "mpv"))]
impl VideoSurface {
    pub fn new(renderer: crate::player::VideoRenderer) -> Self {
        match renderer {}
    }

    pub fn paint(&self, _painter: &egui::Painter, _rect: egui::Rect) {
        match *self {}
    }

    pub fn display_size(&self) -> Option<(u32, u32)> {
        match *self {}
    }

    pub fn has_picture(&self) -> bool {
        match *self {}
    }

    pub fn destroy(&self, _gl: Option<&eframe::glow::Context>) {
        match *self {}
    }
}

/// `GL_RENDERER` of the context (for example `AMD Radeon 680M` or
/// `llvmpipe`). Must be called with `gl` current, which eframe guarantees
/// inside the app creator.
pub fn gl_renderer_name(gl: &eframe::glow::Context) -> String {
    use eframe::glow::HasContext as _;
    // SAFETY: a plain string query on the context that is current on this
    // thread; glow copies the result into an owned `String`.
    unsafe { gl.get_parameter_string(eframe::glow::RENDERER) }
}

/// Axis-aligned rectangle in physical pixels, origin at the top-left of the
/// window (egui's convention).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "mpv"), allow(dead_code))]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg_attr(not(feature = "mpv"), allow(dead_code))]
impl PixelRect {
    pub fn is_empty(self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// Y of the bottom edge measured from the bottom of a framebuffer that is
    /// `screen_height` pixels tall, i.e. the OpenGL window-space origin.
    pub fn gl_bottom(self, screen_height: i32) -> i32 {
        screen_height - self.top - self.height
    }
}

/// Convert a rectangle in egui points to whole physical pixels.
///
/// Each *edge* is rounded independently (rather than rounding the size), so
/// the result lands on the same device pixels egui uses for the neighbouring
/// panels at fractional scale factors such as 1.25 or 1.5: no gap, no overlap,
/// and a 1:1 blit with no resampling. The result is clamped to the screen.
#[cfg_attr(not(feature = "mpv"), allow(dead_code))]
pub fn physical_rect(rect: egui::Rect, pixels_per_point: f32, screen_px: [u32; 2]) -> PixelRect {
    let screen_width = i32::try_from(screen_px[0]).unwrap_or(i32::MAX);
    let screen_height = i32::try_from(screen_px[1]).unwrap_or(i32::MAX);
    if !(pixels_per_point.is_finite() && pixels_per_point > 0.0 && rect.is_finite()) {
        return PixelRect {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
        };
    }

    // `as` saturates on overflow, and the clamp keeps everything on screen.
    let edge = |points: f32, max: i32| ((points * pixels_per_point).round() as i32).clamp(0, max);
    let left = edge(rect.min.x, screen_width);
    let top = edge(rect.min.y, screen_height);
    let right = edge(rect.max.x, screen_width).max(left);
    let bottom = edge(rect.max.y, screen_height).max(top);

    PixelRect {
        left,
        top,
        width: right - left,
        height: bottom - top,
    }
}

/// Largest rectangle with the aspect ratio of `video` (display width ×
/// height) that fits inside `canvas`, centred. The bars around it are left to
/// the UI, which has already painted the canvas black.
#[cfg_attr(not(feature = "mpv"), allow(dead_code))]
pub fn letterbox(canvas: PixelRect, video: (u32, u32)) -> PixelRect {
    let (video_width, video_height) = (u64::from(video.0), u64::from(video.1));
    if canvas.is_empty() || video_width == 0 || video_height == 0 {
        return PixelRect {
            width: 0,
            height: 0,
            ..canvas
        };
    }
    let (canvas_width, canvas_height) = (canvas.width as u64, canvas.height as u64);

    // Integer cross-multiplication avoids float drift; `+ divisor / 2` rounds
    // to the nearest pixel.
    let (width, height) = if canvas_width * video_height > canvas_height * video_width {
        // Canvas is wider than the video: bars left and right (pillarbox).
        let width = (canvas_height * video_width + video_height / 2) / video_height;
        (width.clamp(1, canvas_width), canvas_height)
    } else {
        // Canvas is taller than the video: bars above and below (letterbox).
        let height = (canvas_width * video_height + video_width / 2) / video_width;
        (canvas_width, height.clamp(1, canvas_height))
    };
    let (width, height) = (width as i32, height as i32);

    PixelRect {
        left: canvas.left + (canvas.width - width) / 2,
        top: canvas.top + (canvas.height - height) / 2,
        width,
        height,
    }
}

#[cfg(feature = "mpv")]
mod surface {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use anyhow::{Result, anyhow, bail};
    use eframe::egui_glow;
    use eframe::glow::{self, HasContext as _};

    use super::{PixelRect, letterbox, physical_rect};
    use crate::player::VideoRenderer;

    /// Side length of the target used to consume frames before the picture
    /// size is known.
    const SCRATCH_SIZE: i32 = 16;

    /// Offscreen render target mpv draws into.
    struct Target {
        framebuffer: glow::Framebuffer,
        texture: glow::Texture,
        width: i32,
        height: i32,
        /// False until mpv has rendered into this target at least once.
        has_frame: bool,
    }

    struct Inner {
        renderer: VideoRenderer,
        target: Mutex<Option<Target>>,
        /// Rendering errors repeat every frame; log only the first.
        error_logged: AtomicBool,
    }

    /// GL-side compositor for the video picture. Cheap to keep around; GL
    /// objects are created lazily inside the paint callback.
    pub struct VideoSurface {
        inner: Arc<Inner>,
    }

    impl VideoSurface {
        pub fn new(renderer: VideoRenderer) -> Self {
            Self {
                inner: Arc::new(Inner {
                    renderer,
                    target: Mutex::new(None),
                    error_logged: AtomicBool::new(false),
                }),
            }
        }

        /// Queue the video picture for `rect` (the whole canvas, in points).
        ///
        /// The callback is queued even while the picture size is not known
        /// yet. mpv hands frames to the render thread and waits for each one
        /// to be taken; if nothing took them until the size arrived, the
        /// core would stall on its 200 ms timeout exactly when it is trying
        /// to report that size. Such frames are consumed without being shown.
        /// Size the picture is shown at (aspect and rotation applied), in
        /// video pixels, or `None` before mpv has reported it.
        pub fn display_size(&self) -> Option<(u32, u32)> {
            self.inner.renderer.display_size()
        }

        /// True once a frame of the current media has been presented at its
        /// real size. From then on mpv's update callback drives repaints.
        pub fn has_picture(&self) -> bool {
            self.inner.renderer.display_size().is_some()
                && self
                    .inner
                    .target
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                    .is_some_and(|target| target.has_frame)
        }

        pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
            let inner = Arc::clone(&self.inner);
            painter.add(egui::PaintCallback {
                rect,
                callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                    inner.draw(painter.gl(), &info);
                })),
            });
        }

        /// Free the mpv render context and the GL objects. Call on the GL
        /// thread with the context current (eframe's `on_exit`). Without a
        /// GL context only the render context is released; the GL objects
        /// die with the context.
        pub fn destroy(&self, gl: Option<&glow::Context>) {
            self.inner.renderer.release();
            let target = self
                .inner
                .target
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let (Some(gl), Some(target)) = (gl, target) {
                // SAFETY: `gl` is the context the objects were created in and
                // it is current on this thread (eframe's `on_exit` contract).
                unsafe { delete_target(gl, &target) };
            }
        }
    }

    impl Inner {
        /// Runs inside egui_glow's paint pass: GL context current, eframe's
        /// framebuffer bound, scissor set to the clip rectangle.
        fn draw(&self, gl: &glow::Context, info: &egui::PaintCallbackInfo) {
            static FIRST_PAINT: AtomicBool = AtomicBool::new(false);
            crate::startup::mark_once(&FIRST_PAINT, "first paint callback");
            let screen = info.screen_size_px;
            let canvas = physical_rect(info.viewport, info.pixels_per_point, screen);
            // Read at paint time, so the size is as fresh as the frame. With
            // no size (or no room) there is nothing to show, only to consume.
            let dst = self
                .renderer
                .display_size()
                .map(|video| letterbox(canvas, video))
                .filter(|dst| !dst.is_empty());
            let clip = physical_rect(info.clip_rect, info.pixels_per_point, screen);

            let mut target = self.target.lock().unwrap_or_else(PoisonError::into_inner);
            // SAFETY: egui_glow invokes paint callbacks on the GL thread with
            // `gl` current, and `target` only ever holds objects of this
            // context.
            let result = unsafe { composite(gl, &mut target, &self.renderer, dst, clip, screen) };
            if let Err(err) = result
                && !self.error_logged.swap(true, Ordering::Relaxed)
            {
                tracing::error!(%err, "video compositing failed");
            }
        }
    }

    /// Render the current mpv frame (if new) into the offscreen target and
    /// blit it to `dst` in the framebuffer that was bound on entry. With
    /// `dst == None` the frame is only taken off mpv's hands: it is rendered
    /// into whatever target exists (or a tiny scratch one) and not shown.
    ///
    /// # Safety
    /// `gl` must be current on the calling thread, and `target` must only
    /// contain objects created in that context.
    unsafe fn composite(
        gl: &glow::Context,
        target: &mut Option<Target>,
        renderer: &VideoRenderer,
        dst: Option<PixelRect>,
        clip: PixelRect,
        screen: [u32; 2],
    ) -> Result<()> {
        // `glBlitFramebuffer` and separate read/draw bindings need GL 3.0 or
        // GLES 3.0. Everything eframe creates on Linux qualifies.
        if gl.version().major < 3 {
            bail!("OpenGL 3.0 / OpenGL ES 3.0 or newer is required for video output");
        }
        let screen_width = i32::try_from(screen[0]).unwrap_or(i32::MAX);
        let screen_height = i32::try_from(screen[1]).unwrap_or(i32::MAX);

        // SAFETY: plain state queries on the current context. This is the
        // framebuffer eframe wants the picture in; it is not assumed to be 0.
        let (draw_fbo, read_fbo) = unsafe {
            (
                gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
                gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING),
            )
        };

        let (width, height) = match dst {
            Some(dst) => (dst.width, dst.height),
            // Consuming only: any target will do.
            None => target
                .as_ref()
                .map_or((SCRATCH_SIZE, SCRATCH_SIZE), |t| (t.width, t.height)),
        };

        // SAFETY: forwarded caller contract (current context, own objects).
        let rendered = unsafe { ensure_target(gl, target, width, height) }.and_then(|t| {
            // The target mirrors the window's orientation (GL origin at the
            // bottom-left, like the default framebuffer), so mpv has to flip
            // and the blit below is a straight 1:1 copy.
            let flip_y = true;
            let fbo = i32::try_from(t.framebuffer.0.get())
                .map_err(|_| anyhow!("framebuffer id out of range"))?;
            // SAFETY: current context (caller contract).
            unsafe { enter_mpv_state(gl) };
            // A shown picture is redrawn when the target was just created;
            // a consumed one only when mpv actually has a frame waiting.
            let force = dst.is_some() && !t.has_frame;
            let fresh = renderer.render(fbo, t.width, t.height, flip_y, force)?;
            match dst {
                Some(_) => {
                    t.has_frame |= fresh;
                    if fresh {
                        crate::startup::first_frame();
                    }
                }
                // No size yet (startup, or a new file is opening): whatever
                // the target holds is not the current picture. It is redrawn
                // once the real size is known.
                None => t.has_frame = false,
            }
            Ok(())
        });

        // SAFETY: current context; `draw_fbo`/`read_fbo` were just queried
        // from it and the target objects belong to it. mpv leaves arbitrary
        // state behind, so everything egui relies on is put back first, even
        // when rendering failed.
        unsafe {
            reset_state(gl, clip, screen_width, screen_height);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw_fbo);

            if let (Some(dst), Some(t)) = (dst, target.as_ref().filter(|t| t.has_frame)) {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.framebuffer));
                let (x, y) = (dst.left, dst.gl_bottom(screen_height));
                gl.blit_framebuffer(
                    0,
                    0,
                    t.width,
                    t.height,
                    x,
                    y,
                    x + t.width,
                    y + t.height,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
            }
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read_fbo);
        }

        rendered
    }

    /// Make sure `target` is a complete FBO of exactly `width` × `height`
    /// pixels, recreating it after a window resize or scale-factor change.
    ///
    /// # Safety
    /// Same contract as [`composite`]. Leaves `TEXTURE_2D` and the
    /// framebuffer bindings modified; the caller restores them.
    unsafe fn ensure_target<'t>(
        gl: &glow::Context,
        target: &'t mut Option<Target>,
        width: i32,
        height: i32,
    ) -> Result<&'t mut Target> {
        if let Some(old) = target.take_if(|t| t.width != width || t.height != height) {
            // SAFETY: `old` was created in this context (caller contract).
            unsafe { delete_target(gl, &old) };
        }
        if target.is_none() {
            // SAFETY: object creation on the current context. On failure the
            // partially built objects are deleted before returning.
            let created = unsafe {
                let texture = gl.create_texture().map_err(|e| anyhow!(e))?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                for (name, value) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
                }
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    width,
                    height,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );

                let framebuffer = match gl.create_framebuffer() {
                    Ok(framebuffer) => framebuffer,
                    Err(err) => {
                        gl.delete_texture(texture);
                        bail!(err);
                    }
                };
                let created = Target {
                    framebuffer,
                    texture,
                    width,
                    height,
                    has_frame: false,
                };
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
                let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
                if status != glow::FRAMEBUFFER_COMPLETE {
                    delete_target(gl, &created);
                    bail!("video framebuffer incomplete (status {status:#x})");
                }
                created
            };
            tracing::debug!(width, height, "video render target (re)created");
            *target = Some(created);
        }
        target
            .as_mut()
            .ok_or_else(|| anyhow!("video render target missing"))
    }

    /// # Safety
    /// `gl` must be current and must be the context `target` was created in.
    unsafe fn delete_target(gl: &glow::Context, target: &Target) {
        // SAFETY: caller contract; deleting a bound object just unbinds it.
        unsafe {
            gl.delete_framebuffer(target.framebuffer);
            gl.delete_texture(target.texture);
        }
    }

    /// libmpv expects "reasonably standard" GL defaults on entry
    /// (`mpv/render_gl.h`, "OpenGL state"), while egui_glow paints with the
    /// scissor test and blending enabled and its own program/VAO bound.
    /// Rendering with egui's scissor box active clips the picture away.
    ///
    /// # Safety
    /// `gl` must be current on the calling thread.
    unsafe fn enter_mpv_state(gl: &glow::Context) {
        // SAFETY: state-setting calls with constant, valid enums on the
        // current context.
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.use_program(None);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        }
    }

    /// Put back every piece of GL state that `mpv_render_context_render` may
    /// have changed and that egui_glow or the blit depend on. egui_glow
    /// re-applies its own blend/program/VAO setup after each callback, but
    /// not buffer bindings, pixel-store modes or the masks, so all of it is
    /// reset here rather than relying on that.
    ///
    /// # Safety
    /// `gl` must be current on the calling thread (GL 3.0 / GLES 3.0+).
    unsafe fn reset_state(
        gl: &glow::Context,
        clip: PixelRect,
        screen_width: i32,
        screen_height: i32,
    ) {
        // SAFETY: state-setting calls with constant, valid enums on the
        // current context; no pointers or object lifetimes are involved.
        unsafe {
            gl.use_program(None);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);

            // GL defaults; a stale row length or unpack buffer would corrupt
            // egui's next font-atlas upload.
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 0);
            gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 0);
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);

            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.disable(glow::CULL_FACE);
            gl.color_mask(true, true, true, true);

            // What egui_glow paints with: premultiplied-alpha blending.
            gl.enable(glow::BLEND);
            gl.blend_equation_separate(glow::FUNC_ADD, glow::FUNC_ADD);
            gl.blend_func_separate(
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE_MINUS_DST_ALPHA,
                glow::ONE,
            );

            // The blit honours the scissor box, which keeps the picture
            // inside egui's clip rectangle.
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left,
                clip.gl_bottom(screen_height),
                clip.width,
                clip.height,
            );
            gl.viewport(0, 0, screen_width, screen_height);
        }
    }
}

#[derive(Default)]
pub struct VideoOutput {
    /// The canvas response, for the right-click context menu.
    pub response: Option<egui::Response>,
    /// Canvas size in points.
    pub size: egui::Vec2,
    pub clicked: bool,
    pub double_clicked: bool,
    /// The primary button started dragging the canvas.
    pub drag_started: bool,
    /// Wheel notches over the canvas (positive = up); the app maps each to
    /// a volume step.
    pub wheel: i32,
}

/// Persistent corner overlays (Renderer Settings ▸ Display current time /
/// file name).
#[derive(Default)]
pub struct Overlays {
    /// Top right: `position / duration`.
    pub clock: Option<String>,
    /// Top centre: the media title.
    pub file_name: Option<String>,
    /// A drag carrying files is over the window: show where it will land.
    pub drop_hint: bool,
}

pub fn show(
    ui: &mut egui::Ui,
    snapshot: &PlayerSnapshot,
    surface: Option<&VideoSurface>,
    osd: Option<&str>,
    statistics: Option<&[String]>,
    overlays: &Overlays,
    logo: &mut crate::ui::logo::Logo,
) -> VideoOutput {
    let size = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, egui::CornerRadius::ZERO, palette::CANVAS);

    if snapshot.media_path.is_none() {
        if !overlays.drop_hint {
            paint_welcome(&painter, rect, logo, crate::app::APP_NAME);
        }
    } else if let Some(surface) = surface {
        // Queued after the black fill and before the overlays, so OSD and
        // statistics draw on top of the picture.
        surface.paint(&painter, rect);
    }

    if snapshot.media_path.is_some() && snapshot.state == PlaybackState::Opening {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Opening…",
            egui::FontId::proportional(18.0),
            palette::TEXT_WEAK,
        );
    }

    if let Some(lines) = statistics {
        paint_statistics(&painter, rect, lines);
    }
    if let Some(text) = &overlays.clock {
        paint_corner(&painter, rect, egui::Align2::RIGHT_TOP, text);
    }
    if let Some(text) = &overlays.file_name {
        paint_corner(&painter, rect, egui::Align2::CENTER_TOP, text);
    }

    if let Some(text) = osd {
        paint_osd(&painter, rect, text);
    }
    if overlays.drop_hint {
        paint_drop_hint(&painter, rect);
    }

    // Keep the pointer out of the way while a video is playing: a player is
    // not a document viewer. Never while a menu is open over the video.
    if response.hovered()
        && snapshot.state == PlaybackState::Playing
        && osd.is_none()
        && !egui::Popup::is_any_open(ui.ctx())
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    }

    VideoOutput {
        size: rect.size(),
        clicked: response.clicked(),
        double_clicked: response.double_clicked(),
        drag_started: response.drag_started_by(egui::PointerButton::Primary),
        wheel: if response.hovered() {
            crate::ui::wheel_notches(ui.ctx())
        } else {
            0
        },
        response: Some(response),
    }
}

/// Shown while files are dragged over the window.
fn paint_drop_hint(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, egui::CornerRadius::ZERO, palette::scrim(140));
    painter.rect_stroke(
        rect.shrink(10.0),
        egui::CornerRadius::same(10),
        egui::Stroke::new(2.0, palette::ACCENT),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to play",
        egui::FontId::proportional(20.0),
        palette::TEXT,
    );
}

/// Small translucent label pinned to a corner of the canvas.
fn paint_corner(painter: &egui::Painter, rect: egui::Rect, align: egui::Align2, text: &str) {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::monospace(13.0),
        palette::TEXT,
    );
    let margin = 12.0;
    let anchor = match align.x() {
        egui::Align::Min => egui::pos2(rect.left() + margin, rect.top() + margin),
        egui::Align::Center => egui::pos2(rect.center().x, rect.top() + margin),
        egui::Align::Max => egui::pos2(rect.right() - margin, rect.top() + margin),
    };
    let text_rect = align.anchor_size(anchor, galley.size());
    painter.rect_filled(
        text_rect.expand2(egui::vec2(8.0, 4.0)),
        egui::CornerRadius::same(5),
        palette::scrim(150),
    );
    painter.galley(text_rect.min, galley, palette::TEXT);
}

/// The welcome screen shown while nothing is open: the logo, the name and
/// how to open something. Everything scales down in a small window.
fn paint_welcome(
    painter: &egui::Painter,
    rect: egui::Rect,
    logo: &mut crate::ui::logo::Logo,
    name: &str,
) {
    let center = rect.center();
    // Leave room for the two lines of text below the logo.
    let side = (rect.height() * 0.34)
        .min(rect.width() * 0.4)
        .clamp(48.0, 128.0);
    let compact = rect.height() < 200.0;
    let top = center.y - (side + if compact { 34.0 } else { 78.0 }) / 2.0;

    let mark = logo.paint_centered(painter, egui::pos2(center.x, top + side / 2.0), side, 1.0);
    painter.text(
        egui::pos2(center.x, mark.bottom() + 12.0),
        egui::Align2::CENTER_TOP,
        name,
        egui::FontId::proportional(if compact { 18.0 } else { 24.0 }),
        palette::TEXT,
    );
    if compact {
        return;
    }
    painter.text(
        egui::pos2(center.x, mark.bottom() + 48.0),
        egui::Align2::CENTER_TOP,
        "Drop a video here or press Ctrl+O",
        egui::FontId::proportional(14.0),
        palette::TEXT_WEAK,
    );
    painter.text(
        egui::pos2(center.x, mark.bottom() + 70.0),
        egui::Align2::CENTER_TOP,
        "Right-click for the menu  ·  Ctrl+D opens a folder  ·  Ctrl+U a URL",
        egui::FontId::proportional(12.0),
        palette::ICON_DISABLED.lerp_to_gamma(palette::TEXT_WEAK, 0.55),
    );
}

fn paint_osd(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(15.0),
        palette::TEXT,
    );
    let anchor = egui::pos2(rect.center().x, rect.bottom() - 56.0);
    let text_rect = egui::Align2::CENTER_CENTER.anchor_size(anchor, galley.size());

    painter.rect_filled(
        text_rect.expand2(egui::vec2(14.0, 8.0)),
        egui::CornerRadius::same(6),
        palette::scrim(190),
    );
    painter.galley(text_rect.min, galley, palette::TEXT);
}

fn paint_statistics(painter: &egui::Painter, rect: egui::Rect, lines: &[String]) {
    let mut y = rect.top() + 12.0;
    let x = rect.left() + 12.0;
    let width = 340.0;
    let height = lines.len() as f32 * 16.0 + 16.0;

    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(x - 8.0, y - 8.0), egui::vec2(width, height)),
        egui::CornerRadius::same(6),
        palette::scrim(165),
    );

    for line in lines {
        painter.text(
            egui::pos2(x, y),
            egui::Align2::LEFT_TOP,
            line,
            egui::FontId::monospace(12.0),
            palette::TEXT,
        );
        y += 16.0;
    }
}

/// Lines shown by "Display Statistics". Kept here so the overlay and the
/// properties window agree on formatting.
pub fn statistics_lines(snapshot: &PlayerSnapshot, backend: &str) -> Vec<String> {
    let mut lines = vec![
        format!("state      {}", snapshot.state.label()),
        format!("backend    {backend}"),
        format!("position   {}", format_time(snapshot.position)),
        format!(
            "duration   {}",
            snapshot
                .duration
                .map(format_time)
                .unwrap_or_else(|| "--:--".to_owned())
        ),
    ];
    if let Some((w, h)) = snapshot.video_size {
        lines.push(format!("video      {w}×{h}"));
    }
    if snapshot.video_size.is_some() {
        lines.push(format!(
            "decoder    {}",
            snapshot.hardware_decoder.as_deref().unwrap_or("software")
        ));
    }
    lines.push(format!(
        "audio      {} track(s)  ·  subtitle {} track(s)",
        snapshot.audio_tracks.len(),
        snapshot.subtitle_tracks.len()
    ));
    lines.push(format!("volume     {:.0}%", snapshot.volume));
    lines.push(format!("speed      {:.2}×", snapshot.speed));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: [u32; 2] = [4000, 3000];

    fn points(left: f32, top: f32, right: f32, bottom: f32) -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom))
    }

    fn px(left: i32, top: i32, width: i32, height: i32) -> PixelRect {
        PixelRect {
            left,
            top,
            width,
            height,
        }
    }

    #[test]
    fn physical_rect_is_identity_at_scale_one() {
        let rect = physical_rect(points(10.0, 20.0, 650.0, 380.0), 1.0, SCREEN);
        assert_eq!(rect, px(10, 20, 640, 360));
    }

    #[test]
    fn physical_rect_scales_fractionally() {
        // 1120 × 700 points at 1.25 and 1.5 are exact pixel counts.
        let rect = points(0.0, 0.0, 1120.0, 700.0);
        assert_eq!(physical_rect(rect, 1.25, SCREEN), px(0, 0, 1400, 875));
        assert_eq!(physical_rect(rect, 1.5, SCREEN), px(0, 0, 1680, 1050));
    }

    #[test]
    fn physical_rect_rounds_edges_not_sizes() {
        // 22.2 pt → 27.75 px → 28; 633.4 pt → 791.75 px → 792. Rounding the
        // size (611.2 pt → 764 px) would land the right edge on the same
        // pixel here, but edge rounding is what guarantees it in general.
        let rect = physical_rect(points(22.2, 0.0, 633.4, 100.0), 1.25, SCREEN);
        assert_eq!((rect.left, rect.left + rect.width), (28, 792));
    }

    #[test]
    fn adjacent_rects_share_an_edge_at_any_scale() {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            for i in 0..2000 {
                let split = i as f32 * 0.37;
                let a = physical_rect(points(0.0, 0.0, split, 10.0), scale, SCREEN);
                let b = physical_rect(points(split, 0.0, 900.0, 10.0), scale, SCREEN);
                assert_eq!(a.left + a.width, b.left, "scale {scale}, split {split}");
            }
        }
    }

    #[test]
    fn physical_rect_clamps_to_screen_and_rejects_garbage() {
        let rect = physical_rect(points(-50.0, -50.0, 5000.0, 5000.0), 2.0, [800, 600]);
        assert_eq!(rect, px(0, 0, 800, 600));

        assert!(physical_rect(points(0.0, 0.0, 10.0, 10.0), 0.0, SCREEN).is_empty());
        assert!(physical_rect(points(0.0, 0.0, 10.0, 10.0), f32::NAN, SCREEN).is_empty());
        assert!(physical_rect(points(0.0, 0.0, f32::NAN, 10.0), 1.0, SCREEN).is_empty());
        // Inverted rectangle collapses instead of going negative.
        assert!(physical_rect(points(10.0, 10.0, 5.0, 5.0), 1.0, SCREEN).is_empty());
    }

    #[test]
    fn gl_bottom_flips_the_origin() {
        assert_eq!(px(0, 0, 100, 50).gl_bottom(600), 550);
        assert_eq!(px(0, 550, 100, 50).gl_bottom(600), 0);
    }

    #[test]
    fn letterbox_exact_fit_fills_the_canvas() {
        let canvas = px(7, 9, 1280, 720);
        assert_eq!(letterbox(canvas, (1920, 1080)), canvas);
    }

    #[test]
    fn letterbox_wide_video_gets_bars_above_and_below() {
        // 2.40:1 film in a 16:9 canvas.
        let rect = letterbox(px(0, 0, 1920, 1080), (1920, 800));
        assert_eq!(rect, px(0, 140, 1920, 800));
    }

    #[test]
    fn letterbox_narrow_video_gets_bars_left_and_right() {
        // 4:3 in a 16:9 canvas, with a canvas offset.
        let rect = letterbox(px(100, 50, 1920, 1080), (640, 480));
        assert_eq!(rect, px(340, 50, 1440, 1080));
    }

    #[test]
    fn letterbox_portrait_video() {
        let rect = letterbox(px(0, 0, 1000, 1000), (1080, 1920));
        // 1000 × 1080 / 1920 = 562.5 → 563 px wide; the odd spare pixel goes right.
        assert_eq!(rect, px(218, 0, 563, 1000));
    }

    #[test]
    fn letterbox_stays_inside_canvas_and_keeps_aspect() {
        let videos = [(1920, 1080), (720, 576), (1080, 1920), (3840, 1600), (1, 1)];
        for canvas in [px(3, 5, 1399, 873), px(0, 0, 641, 400), px(0, 0, 37, 911)] {
            for video in videos {
                let rect = letterbox(canvas, video);
                assert!(rect.left >= canvas.left && rect.top >= canvas.top);
                assert!(rect.left + rect.width <= canvas.left + canvas.width);
                assert!(rect.top + rect.height <= canvas.top + canvas.height);
                // One axis is filled completely…
                assert!(rect.width == canvas.width || rect.height == canvas.height);
                // …and the other is within half a pixel of the true aspect.
                let ideal = rect.height as f64 * video.0 as f64 / video.1 as f64;
                assert!(
                    (rect.width as f64 - ideal).abs() <= (video.0 as f64 / video.1 as f64).max(1.0),
                    "{canvas:?} {video:?} -> {rect:?}"
                );
            }
        }
    }

    #[test]
    fn letterbox_degenerate_inputs_are_empty() {
        assert!(letterbox(px(0, 0, 0, 100), (16, 9)).is_empty());
        assert!(letterbox(px(0, 0, 100, 100), (0, 9)).is_empty());
        assert!(letterbox(px(0, 0, 100, 100), (16, 0)).is_empty());
        // Extreme aspect never collapses to zero pixels.
        assert_eq!(letterbox(px(0, 0, 10, 10), (10_000, 1)).height, 1);
    }
}
