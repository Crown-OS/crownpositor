//! Sprites as pixels. Each is rasterised once, on the CPU, when its content
//! changes; everything that moves — transitions, the shake — happens on the
//! GPU in the host.

mod glyph;
mod icon;
mod shape;

use cosmic_text::FontSystem;
use lockscene::LockScene;
use tiny_skia::{Paint, Pixmap, Point, Transform};

use crate::{
    layout::{self, Content, Context, Images, Live, Shaper, Sprite, TextRun},
    palette::Palette,
};

use shape::{fill, gradient_fill, rounded_rect, stroke};

const DOT_RADIUS: f32 = 4.5;
const DOT_SPACING: f32 = 15.0;
const BUSY_DOT_ALPHA: f32 = 0.45;

pub struct Painter {
    shaper: Shaper,
}

impl Painter {
    /// `fonts` should already hold any font the host bundles.
    pub fn new(fonts: FontSystem) -> Self {
        Self {
            shaper: Shaper::new(fonts),
        }
    }

    /// Lays `scene` out over an output `size` logical pixels big.
    pub fn layout(
        &mut self,
        scene: &LockScene,
        live: Live<'_>,
        images: &dyn Images,
        size: [f32; 2],
    ) -> Vec<Sprite> {
        let theme_font = &scene.theme.font;
        let context = Context {
            scene,
            live,
            palette: Palette::new(&scene.theme),
            images,
            family: scene.config.font.as_deref().unwrap_or(&theme_font.family),
            mono: &theme_font.mono,
            text_scale: theme_font.scale.clamp(0.5, 3.0),
        };
        layout::sprites(&mut self.shaper, &context, size)
    }

    /// `sprite` at `scale` physical pixels per logical one, with `pad`
    /// transparent pixels around it for a blur to spread into.
    pub fn paint(
        &mut self,
        sprite: &Sprite,
        scale: f32,
        pad: u32,
        images: &dyn Images,
    ) -> Option<Pixmap> {
        let width = (sprite.rect.width * scale).ceil() as u32;
        let height = (sprite.rect.height * scale).ceil() as u32;
        let mut pixmap = Pixmap::new(width + 2 * pad, height + 2 * pad)?;
        let origin = pad as f32;
        let (w, h) = (width as f32, height as f32);
        match &sprite.content {
            Content::Text(run) => {
                self.shaper
                    .draw(run, scale, sprite.rect.width, &mut pixmap, [pad as i32; 2]);
            }
            Content::Glyph(run) => glyph::draw(&mut self.shaper, &mut pixmap, run, scale, origin),
            Content::Icon { body, color } => icon::draw(&mut pixmap, body, *color, origin, w),
            Content::Image { asset, radius } => {
                let image = images.get(asset)?;
                draw_image(&mut pixmap, image, origin, w, h, radius * scale);
            }
            Content::Panel {
                fill: inside,
                border,
                radius,
            } => {
                let path = rounded_rect(origin, origin, w, h, radius * scale)?;
                if let Some(color) = inside {
                    fill(&mut pixmap, &path, *color);
                }
                if let Some((line, color)) = border {
                    let inset = line * scale / 2.0;
                    let ring = rounded_rect(
                        origin + inset,
                        origin + inset,
                        w - 2.0 * inset,
                        h - 2.0 * inset,
                        radius * scale - inset,
                    )?;
                    stroke(&mut pixmap, &ring, line * scale, *color);
                }
            }
            Content::Divider { color } => {
                fill(
                    &mut pixmap,
                    &rounded_rect(origin, origin, w, h, 0.0)?,
                    *color,
                );
            }
            Content::Progress {
                value,
                track,
                fill: done,
            } => {
                let radius = h / 2.0;
                fill(
                    &mut pixmap,
                    &rounded_rect(origin, origin, w, h, radius)?,
                    *track,
                );
                if *value > 0.0 {
                    let bar = rounded_rect(origin, origin, (w * value).max(h), h, radius)?;
                    fill(&mut pixmap, &bar, *done);
                }
            }
            Content::Field {
                dots,
                busy,
                hint,
                fill: inside,
                border,
                dot,
            } => {
                let path = rounded_rect(origin, origin, w, h, h / 2.0)?;
                fill(&mut pixmap, &path, *inside);
                stroke(&mut pixmap, &path, scale, *border);
                if *dots == 0 {
                    self.centred_text(&mut pixmap, hint, scale, sprite.rect.width, pad, h);
                } else {
                    let alpha = if *busy { BUSY_DOT_ALPHA } else { 1.0 };
                    draw_dots(&mut pixmap, *dots, *dot, alpha, scale, origin, w, h);
                }
            }
            Content::Avatar {
                initial,
                fill: stops,
            } => {
                let path = rounded_rect(origin, origin, w, h, w / 2.0)?;
                gradient_fill(
                    &mut pixmap,
                    &path,
                    Point::from_xy(origin, origin),
                    Point::from_xy(origin + w, origin + h),
                    *stops,
                );
                self.centred_text(&mut pixmap, initial, scale, sprite.rect.width, pad, h);
            }
        }
        Some(pixmap)
    }

    fn centred_text(
        &mut self,
        pixmap: &mut Pixmap,
        run: &TextRun,
        scale: f32,
        width: f32,
        pad: u32,
        height: f32,
    ) {
        let top = pad as f32 + (height - run.line_height * scale) / 2.0;
        self.shaper
            .draw(run, scale, width, pixmap, [pad as i32, top.round() as i32]);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_dots(
    pixmap: &mut Pixmap,
    count: usize,
    color: [f32; 4],
    alpha: f32,
    scale: f32,
    origin: f32,
    width: f32,
    height: f32,
) {
    let spacing = DOT_SPACING * scale;
    let fits = ((width - height) / spacing).floor().max(1.0) as usize;
    let shown = count.min(fits);
    let span = (shown - 1) as f32 * spacing;
    let start = origin + (width - span) / 2.0;
    let [r, g, b, a] = color;
    let color = [r, g, b, a * alpha];
    let centre_y = origin + height / 2.0;
    for index in 0..shown {
        let centre_x = start + index as f32 * spacing;
        let radius = DOT_RADIUS * scale;
        if let Some(path) = rounded_rect(
            centre_x - radius,
            centre_y - radius,
            2.0 * radius,
            2.0 * radius,
            radius,
        ) {
            fill(pixmap, &path, color);
        }
    }
}

fn draw_image(
    pixmap: &mut Pixmap,
    image: &Pixmap,
    origin: f32,
    width: f32,
    height: f32,
    radius: f32,
) {
    let Some(path) = rounded_rect(origin, origin, width, height, radius) else {
        return;
    };
    let scale = (width / image.width() as f32).max(height / image.height() as f32);
    let dx = origin + (width - image.width() as f32 * scale) / 2.0;
    let dy = origin + (height - image.height() as f32 * scale) / 2.0;
    let shader = tiny_skia::Pattern::new(
        image.as_ref(),
        tiny_skia::SpreadMode::Pad,
        tiny_skia::FilterQuality::Bicubic,
        1.0,
        Transform::from_row(scale, 0.0, 0.0, scale, dx, dy),
    );
    let paint = Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    };
    pixmap.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
}

#[cfg(test)]
mod tests;
