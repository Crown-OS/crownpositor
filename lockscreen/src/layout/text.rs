use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use tiny_skia::Pixmap;

use super::{FontFamily, TextRun};

/// Shapes and rasterises text: the one place the lock screen touches fonts.
pub(crate) struct Shaper {
    pub(super) fonts: FontSystem,
    pub(super) glyphs: SwashCache,
}

impl Shaper {
    pub fn new(fonts: FontSystem) -> Self {
        Self {
            fonts,
            glyphs: SwashCache::new(),
        }
    }

    fn shape(&mut self, run: &TextRun, scale: f32, width: Option<f32>) -> Buffer {
        let metrics = Metrics::new(run.size * scale, run.line_height * scale);
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        let attrs = Attrs::new()
            .family(family_of(&run.family))
            .weight(Weight(run.weight));
        let align = run.centered.then_some(Align::Center);
        let mut borrowed = buffer.borrow_with(&mut self.fonts);
        borrowed.set_wrap(if run.single_line {
            Wrap::None
        } else {
            Wrap::WordOrGlyph
        });
        borrowed.set_size(width.map(|width| width * scale), None);
        borrowed.set_text(&run.text, &attrs, Shaping::Advanced, align);
        borrowed.shape_until_scroll(false);
        buffer
    }

    /// The logical size `run` takes when wrapped at `width`, or unwrapped.
    pub fn measure(&mut self, run: &TextRun, width: Option<f32>) -> [f32; 2] {
        let buffer = self.shape(run, 1.0, width.filter(|_| !run.single_line));
        let (wide, lines) = buffer
            .layout_runs()
            .fold((0.0_f32, 0usize), |(wide, lines), line| {
                (wide.max(line.line_w), lines + 1)
            });
        [wide.ceil(), (lines.max(1) as f32 * run.line_height).ceil()]
    }

    /// Paints `run` into `pixmap`, `width` logical pixels wide, at `origin`.
    pub fn draw(
        &mut self,
        run: &TextRun,
        scale: f32,
        width: f32,
        pixmap: &mut Pixmap,
        origin: [i32; 2],
    ) {
        let mut buffer = self.shape(run, scale, Some(width));
        let [r, g, b, a] = run
            .color
            .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
        let (pixmap_width, pixmap_height) = (pixmap.width() as i32, pixmap.height() as i32);
        let data = pixmap.data_mut();
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            Color::rgba(r, g, b, a),
            |x, y, w, h, color| {
                let alpha = u32::from(color.a());
                if alpha == 0 {
                    return;
                }
                for dy in 0..h as i32 {
                    for dx in 0..w as i32 {
                        let (px, py) = (origin[0] + x + dx, origin[1] + y + dy);
                        if px < 0 || py < 0 || px >= pixmap_width || py >= pixmap_height {
                            continue;
                        }
                        let index = ((py * pixmap_width + px) * 4) as usize;
                        blend(
                            &mut data[index..index + 4],
                            [color.r(), color.g(), color.b()],
                            alpha,
                        );
                    }
                }
            },
        );
    }
}

pub(super) fn family_of(family: &FontFamily) -> Family<'_> {
    match family {
        FontFamily::Sans(name) | FontFamily::Mono(name) if !name.is_empty() => Family::Name(name),
        FontFamily::Mono(_) => Family::Monospace,
        FontFamily::Sans(_) => Family::SansSerif,
    }
}

/// Source-over of a straight colour at `alpha` onto a premultiplied pixel.
fn blend(pixel: &mut [u8], rgb: [u8; 3], alpha: u32) {
    let keep = 255 - alpha;
    for (channel, source) in pixel.iter_mut().zip(rgb) {
        *channel = ((u32::from(source) * alpha + u32::from(*channel) * keep + 127) / 255) as u8;
    }
    pixel[3] = ((alpha * 255 + u32::from(pixel[3]) * keep + 127) / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blending_is_source_over_premultiplied() {
        let mut pixel = [0, 0, 0, 0];
        blend(&mut pixel, [255, 0, 0], 128);
        assert_eq!(pixel, [128, 0, 0, 128]);
        blend(&mut pixel, [0, 0, 255], 255);
        assert_eq!(pixel, [0, 0, 255, 255]);
    }
}
