//! The clock and prompt layers' sprites, and the background's subject
//! between them, as the frame places them.

use std::time::Instant;

use lockscreen::{
    clock::{Glass, Homography, frost_sigma, morph, rim_reach},
    layout::{Content, Layer, Part, Rect},
    transition::LayerStyle,
};
use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{Element, Kind, Wrap, memory::MemoryRenderBufferRenderElement},
    },
    utils::{Logical, Physical, Point, Scale, Size},
};

use super::{VISIBLE_BLUR, background::covering};
use crate::{
    rendering::{
        Elements,
        decorate::{GlyphDraw, GlyphGlass, TileDecorator},
        element::CrownElement,
        lock_glyph::{clock_warp, unwarp, warped_bounds},
        unstretched::Unstretched,
    },
    shell::lock_screen::{OutputVisuals, PreparedSprite},
};

pub(super) struct Sprites<'a, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    D: TileDecorator<R>,
{
    pub elements: &'a mut Elements<R, D>,
    pub renderer: &'a mut R,
    pub decorator: &'a mut D,
    pub visuals: &'a OutputVisuals,
    pub scale: Scale<f64>,
    pub now: Instant,
    pub clock_warp: [[f32; 2]; 4],
    /// What the frost behind glass digits is multiplied by, so it carries
    /// the darkening the glass wants rather than what the pyramid lifted.
    pub frost_gain: f32,
}

impl<R, D> Sprites<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    /// One layer's sprites, front to back, scaled about the layer's centre.
    pub fn layer(&mut self, layer: Layer, style: LayerStyle, shake: f32) {
        if !style.is_visible() {
            return;
        }
        let visuals = self.visuals;
        let in_layer = || {
            visuals
                .sprites
                .iter()
                .filter(move |sprite| sprite.layer == layer)
        };
        let Some(pivot) = centre(in_layer().map(|sprite| sprite.rect)) else {
            return;
        };
        let warp = self.warp(in_layer(), style, pivot);
        for sprite in front_to_back(in_layer()) {
            if matches!(sprite.part, Some(Part::ClockGlyph(_))) {
                let arriving = sprite.born.map_or(LayerStyle::REST, |born| {
                    morph(self.now.saturating_duration_since(born)).1
                });
                self.glyph(sprite, style, arriving, pivot, warp.as_ref());
                continue;
            }
            let nudge = if sprite.part == Some(Part::SecureField) {
                shake
            } else {
                0.0
            };
            self.sprite(sprite, style, pivot, nudge);
        }
        if layer == Layer::Clock {
            for (sprite, since) in &visuals.leaving {
                let going = morph(self.now.saturating_duration_since(*since)).0;
                self.glyph(sprite, style, going, pivot, warp.as_ref());
            }
        }
    }

    /// The background's subject in front of the clock, framed over the
    /// `size` output at `zoom` exactly as the background is.
    pub fn subject(&mut self, style: LayerStyle, zoom: f32, size: Size<i32, Logical>) {
        if !style.is_visible() {
            return;
        }
        let Some(subject) = self
            .visuals
            .subject
            .as_ref()
            .filter(|subject| !subject.hides_clock)
        else {
            return;
        };
        let Some(element) = covering(self.renderer, &subject.picture, size, zoom, style.opacity)
        else {
            return;
        };
        let blur = style.blur * self.scale.x as f32;
        if blur < VISIBLE_BLUR {
            self.elements.push(Unstretched::from(element).into());
        } else if let Some(blurred) = self.decorator.blurred_sprite(self.renderer, element, blur) {
            self.elements.push(CrownElement::Tile(Wrap::from(blurred)));
        }
    }

    /// The clock's warp, in physical pixels, about its glyphs as the layer
    /// places them.
    fn warp<'s>(
        &self,
        sprites: impl Iterator<Item = &'s PreparedSprite>,
        style: LayerStyle,
        pivot: [f32; 2],
    ) -> Option<Homography> {
        let glyphs = sprites
            .filter(|sprite| matches!(sprite.part, Some(Part::ClockGlyph(_))))
            .map(|sprite| placed(sprite.rect, style, pivot));
        let clock = physical(bounds(glyphs)?, self.scale.x as f32);
        clock_warp(clock, self.clock_warp)
    }

    /// A clock glyph: placed by its layer's `style`, bent by the warp, and
    /// blurred and scaled about its own centre by its `morph`. A face is
    /// frosted glass.
    fn glyph(
        &mut self,
        sprite: &PreparedSprite,
        style: LayerStyle,
        morph: LayerStyle,
        pivot: [f32; 2],
        warp: Option<&Homography>,
    ) {
        let combined = style.then(morph);
        if !combined.is_visible() {
            return;
        }
        let flat = LayerStyle {
            opacity: combined.opacity,
            ..style
        };
        let face = face_glass(sprite);
        if warp.is_none()
            && face.is_none()
            && (morph.scale - 1.0).abs() < 1e-3
            && morph.blur * (self.scale.x as f32) < VISIBLE_BLUR
        {
            self.sprite(sprite, flat, pivot, 0.0);
            return;
        }
        let scale = self.scale.x as f32;
        let rest = placed(sprite.rect, style, pivot);
        let [left, top] = rest.padded_origin(scale, sprite.pad as f32 * style.scale);
        let [x, y] = rest.centre().map(|at| at * scale);
        let zoom = combined.scale;
        let [width, height] = sprite.pixels.map(|side| side as f32 * zoom);
        let padded = Rect {
            x: x + (left - x) * morph.scale,
            y: y + (top - y) * morph.scale,
            width,
            height,
        };
        let warp = warp.copied().unwrap_or(Homography::IDENTITY);
        let Some(bounds) = warped_bounds(padded, &warp) else {
            return;
        };
        let element = MemoryRenderBufferRenderElement::from_buffer(
            self.renderer,
            bounds.loc.to_f64(),
            &sprite.buffer,
            Some(combined.opacity),
            Some(sprite.whole_buffer()),
            Some(Size::<i32, Logical>::from((
                (bounds.size.w as f32 / scale).round() as i32,
                (bounds.size.h as f32 / scale).round() as i32,
            ))),
            Kind::Unspecified,
        );
        let Ok(element) = element else {
            return;
        };
        let geometry = element.geometry(self.scale);
        let Some(unwarp) = unwarp(geometry, padded, &warp) else {
            return;
        };
        let blur = combined.blur * scale;
        let [columns, rows] = sprite.pixels.map(|side| side as f32);
        let pad = sprite.pad as f32;
        let glyph_height = sprite.rect.height * scale;
        let draw = GlyphDraw {
            unwarp,
            reach: [blur / padded.width, blur / padded.height],
            glass: face.map(|face| GlyphGlass {
                id: sprite.glass_id.clone(),
                geometry,
                frost_gain: self.frost_gain,
                frost: frost_sigma(glyph_height * zoom),
                face,
                rows: [pad / rows, (pad + glyph_height) / rows],
                rim: [columns, rows].map(|side| rim_reach(glyph_height) / side),
            }),
        };
        if let Some(bent) = self.decorator.lock_glyph(self.renderer, element, draw) {
            self.elements.push(CrownElement::Tile(Wrap::from(bent)));
            return;
        }
        self.sprite(sprite, flat, pivot, 0.0);
    }

    fn sprite(&mut self, sprite: &PreparedSprite, style: LayerStyle, pivot: [f32; 2], nudge: f32) {
        let scale = self.scale.x as f32;
        let zoom = style.scale;
        let place = |at: f32, centre: f32, offset: f32| centre + (at + offset - centre) * zoom;
        let shown = Rect {
            x: place(sprite.rect.x, pivot[0], style.offset[0] + nudge),
            y: place(sprite.rect.y, pivot[1], style.offset[1]),
            ..sprite.rect
        };
        let [x, y] = shown.padded_origin(scale, sprite.pad as f32 * zoom);
        let origin = Point::<f64, Physical>::from((f64::from(x), f64::from(y)));

        let blur = style.blur * scale;
        let exact = (zoom - 1.0).abs() < 1e-3 && blur < VISIBLE_BLUR;
        let size = (!exact).then(|| {
            Size::<i32, Logical>::from((
                (sprite.pixels[0] as f32 * zoom / scale).round() as i32,
                (sprite.pixels[1] as f32 * zoom / scale).round() as i32,
            ))
        });
        let element = |renderer: &mut R| {
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                origin,
                &sprite.buffer,
                Some(style.opacity),
                size.map(|_| sprite.whole_buffer()),
                size,
                Kind::Unspecified,
            )
            .ok()
        };
        let Some(sharp) = element(self.renderer) else {
            return;
        };
        if exact {
            self.elements
                .push(Unstretched::at_own_size(sharp, self.scale).into());
            return;
        }
        if blur >= VISIBLE_BLUR
            && let Some(blurred) = self.decorator.blurred_sprite(self.renderer, sharp, blur)
        {
            self.elements.push(CrownElement::Tile(Wrap::from(blurred)));
            return;
        }
        if let Some(sharp) = element(self.renderer) {
            self.elements.push(Unstretched::from(sharp).into());
        }
    }
}

/// A layer's `sprites`, laid out back to front, in the order elements are
/// pushed: front to back, the date in front of all the rest, so the frost
/// glass digits lift from under them is the background alone.
fn front_to_back<'s>(
    sprites: impl DoubleEndedIterator<Item = &'s PreparedSprite>,
) -> Vec<&'s PreparedSprite> {
    let mut ordered: Vec<_> = sprites.rev().collect();
    ordered.sort_by_key(|sprite| sprite.part != Some(Part::ClockDate));
    ordered
}

/// The glass a face is drawn with; `None` for anything else.
fn face_glass(sprite: &PreparedSprite) -> Option<Glass> {
    match &sprite.content {
        Content::Glyph(run) => run.paint.glass(),
        _ => None,
    }
}

/// Where `style` puts `rect`, scaled about `pivot`, in logical pixels.
fn placed(rect: Rect, style: LayerStyle, pivot: [f32; 2]) -> Rect {
    let place = |at: f32, centre: f32, offset: f32| centre + (at + offset - centre) * style.scale;
    Rect {
        x: place(rect.x, pivot[0], style.offset[0]),
        y: place(rect.y, pivot[1], style.offset[1]),
        width: rect.width * style.scale,
        height: rect.height * style.scale,
    }
}

fn physical(rect: Rect, scale: f32) -> Rect {
    Rect {
        x: rect.x * scale,
        y: rect.y * scale,
        width: rect.width * scale,
        height: rect.height * scale,
    }
}

/// The box around `rects`.
fn bounds(rects: impl Iterator<Item = Rect>) -> Option<Rect> {
    let [left, top, right, bottom] = rects.fold(None, |bounds: Option<[f32; 4]>, rect| {
        let [left, top, right, bottom] = bounds.unwrap_or([f32::MAX, f32::MAX, f32::MIN, f32::MIN]);
        Some([
            left.min(rect.x),
            top.min(rect.y),
            right.max(rect.x + rect.width),
            bottom.max(rect.y + rect.height),
        ])
    })?;
    Some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn centre(rects: impl Iterator<Item = Rect>) -> Option<[f32; 2]> {
    bounds(rects).map(|rect| rect.centre())
}
