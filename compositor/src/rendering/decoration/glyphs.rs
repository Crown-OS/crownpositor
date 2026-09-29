//! Glyph rasterisation with the same hinting the CrownOS toolkits use.
//!
//! Swash only runs a font's own bytecode hints, so an unhinted face like Inter
//! comes out soft. Outlines here go through skrifa's autohinter, sit on whole
//! pixels, and grow by a hair so thin stems stay solid — matching what vello
//! paints in every app window. Glyphs with no outline (colour emoji bitmaps)
//! still come from swash.

use std::sync::Arc;

use cosmic_text::{Buffer, CacheKey, Font, FontSystem, LayoutGlyph, SwashCache, fontdb};
use skrifa::{
    FontRef, GlyphId, MetadataProvider, Tag,
    instance::Size,
    outline::{
        DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target,
    },
};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

/// Outline growth per side, in physical pixels.
const STEM_DARKENING: f32 = 0.07;

const HINTING: HintingOptions = HintingOptions {
    engine: Engine::AutoFallback,
    target: Target::Smooth {
        mode: SmoothMode::Lcd,
        symmetric_rendering: false,
        preserve_linear_metrics: true,
    },
};

/// Paints every glyph of `buffer` into `pixmap` in `color`.
pub fn paint(
    buffer: &Buffer,
    fonts: &mut FontSystem,
    swash: &mut SwashCache,
    pixmap: &mut Pixmap,
    color: [u8; 4],
) {
    let [red, green, blue, alpha] = color;
    let mut paint = Paint::default();
    paint.set_color_rgba8(red, green, blue, alpha);
    let darkening = Stroke {
        width: 2.0 * STEM_DARKENING,
        ..Stroke::default()
    };

    let mut hinter: Option<(CacheKey, Arc<Font>, HintingInstance)> = None;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let origin = (
                (glyph.x + glyph.font_size * glyph.x_offset).round(),
                (run.line_y + glyph.y - glyph.font_size * glyph.y_offset).round(),
            );
            let physical = glyph.physical((0.0, run.line_y), 1.0);
            if hinter
                .as_ref()
                .is_none_or(|(key, ..)| !same_face(key, &physical.cache_key))
            {
                hinter = hinting_for(fonts, glyph)
                    .map(|(font, instance)| (physical.cache_key, font, instance));
            }
            let outline = hinter.as_ref().and_then(|(_, font, instance)| {
                outline_path(font, fonts.db(), glyph, instance, origin)
            });
            match outline {
                Some(path) => {
                    pixmap.fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                    pixmap.stroke_path(&path, &paint, &darkening, Transform::identity(), None);
                }
                None => blend_bitmap(fonts, swash, physical, color, pixmap),
            }
        }
    }
}

fn same_face(a: &CacheKey, b: &CacheKey) -> bool {
    a.font_id == b.font_id && a.font_weight == b.font_weight && a.font_size_bits == b.font_size_bits
}

fn face_ref<'a>(font: &'a Font, db: &fontdb::Database, glyph: &LayoutGlyph) -> Option<FontRef<'a>> {
    let index = db.face(glyph.font_id)?.index;
    FontRef::from_index(font.data(), index).ok()
}

fn hinting_for(
    fonts: &mut FontSystem,
    glyph: &LayoutGlyph,
) -> Option<(Arc<Font>, HintingInstance)> {
    let font = fonts.get_font(glyph.font_id, glyph.font_weight)?;
    let face = face_ref(&font, fonts.db(), glyph)?;
    let location = face
        .axes()
        .location([(Tag::new(b"wght"), f32::from(glyph.font_weight.0))]);
    let instance = HintingInstance::new(
        &face.outline_glyphs(),
        Size::new(glyph.font_size),
        &location,
        HINTING,
    )
    .ok()?;
    Some((font, instance))
}

fn outline_path(
    font: &Font,
    db: &fontdb::Database,
    glyph: &LayoutGlyph,
    hinting: &HintingInstance,
    origin: (f32, f32),
) -> Option<tiny_skia::Path> {
    let face = face_ref(font, db, glyph)?;
    let outline = face
        .outline_glyphs()
        .get(GlyphId::new(glyph.glyph_id.into()))?;
    let mut pen = PixelPen {
        path: PathBuilder::new(),
        origin,
    };
    outline
        .draw(DrawSettings::hinted(hinting, false), &mut pen)
        .ok()?;
    pen.path.finish()
}

/// Maps font units (y up, at the hinted size) onto the pixmap (y down).
struct PixelPen {
    path: PathBuilder,
    origin: (f32, f32),
}

impl PixelPen {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (self.origin.0 + x, self.origin.1 - y)
    }
}

impl OutlinePen for PixelPen {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.path.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.path.line_to(x, y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.map(cx0, cy0);
        let (x, y) = self.map(x, y);
        self.path.quad_to(cx0, cy0, x, y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.map(cx0, cy0);
        let (cx1, cy1) = self.map(cx1, cy1);
        let (x, y) = self.map(x, y);
        self.path.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }

    fn close(&mut self) {
        self.path.close();
    }
}

fn blend_bitmap(
    fonts: &mut FontSystem,
    swash: &mut SwashCache,
    glyph: cosmic_text::PhysicalGlyph,
    [red, green, blue, alpha]: [u8; 4],
    pixmap: &mut Pixmap,
) {
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let base = cosmic_text::Color::rgba(red, green, blue, alpha);
    let pixels = pixmap.pixels_mut();
    swash.with_pixels(fonts, glyph.cache_key, base, |x, y, color| {
        let (px, py) = (glyph.x + x, glyph.y + y);
        if px < 0 || py < 0 || px >= width || py >= height || color.a() == 0 {
            return;
        }
        let pixel = &mut pixels[(py * width + px) as usize];
        let coverage = u32::from(color.a());
        let over = |source: u8, dest: u8| {
            (u32::from(source) * coverage / 255 + u32::from(dest) * (255 - coverage) / 255) as u8
        };
        let blended = PremultipliedColorU8::from_rgba(
            over(color.r(), pixel.red()),
            over(color.g(), pixel.green()),
            over(color.b(), pixel.blue()),
            over(255, pixel.alpha()),
        );
        if let Some(blended) = blended {
            *pixel = blended;
        }
    });
}
