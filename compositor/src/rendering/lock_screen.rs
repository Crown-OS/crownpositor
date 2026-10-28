//! What an output shows while the built-in lock is up, front to back: the
//! prompt and clock sprites as their transitions place them, a dim, a blur
//! over the background, the background, and an opaque floor — so nothing of
//! the desktop shows through, whatever is still loading.
//!
//! Once the password is accepted the floor goes and the desktop is drawn
//! beneath, so the lock's blur and dim lift off the real desktop.

use std::time::Instant;

use lockscreen::{
    clock::{GLASS_TINT, Homography, morph},
    layout::{Content, GlyphPaint, Layer, Part, Rect},
    transition::LayerStyle,
};
use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Element, Kind, Wrap, memory::MemoryRenderBufferRenderElement,
            solid::SolidColorRenderElement,
        },
        utils::CommitCounter,
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
    wayland::shell::wlr_layer::Layer as ShellLayer,
};

use crate::{
    rendering::{
        Elements,
        blur::{Glass, Shading},
        decorate::{Backdrop as GlassBackdrop, GlyphDraw, GlyphGlass, TileDecorator},
        element::CrownElement,
        layer_elements,
        lock_glyph::{clock_warp, unwarp, warped_bounds},
        unstretched::Unstretched,
    },
    shell::{
        lock_screen::{ActiveLock, Backdrop, OutputVisuals, PreparedSprite},
        monitor::Monitor,
    },
};

const FLOOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Blur radii below this many pixels draw sharp.
const VISIBLE_BLUR: f32 = 0.5;
/// Logical pixels: how far the lit top edge of a glass digit reaches.
const GLASS_EDGE: f32 = 1.5;
/// Quantises animated alphas into commit counters.
const STEPS: f32 = 1024.0;

/// Pushes the lock for `monitor`. Returns whether the desktop should be drawn
/// beneath it: only while the unlock animates away.
pub fn lock_screen_elements<R, D>(
    elements: &mut Elements<R, D>,
    lock: &ActiveLock,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
) -> bool
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let now = Instant::now();
    let frame = lock.flow.frame(now);
    let unlocking = lock.flow.is_unlocking();
    let output = Rectangle::from_size(monitor.geometry().size.to_physical_precise_round(scale));
    let visuals = lock.visuals.get(&monitor.id());

    let strength = lock.blur * frame.backdrop;
    let lock_glass = strength > 0.0 && decorator.blur_fingerprint().is_some();
    let frost = lock.clock_glass.then(|| {
        if lock_glass {
            lock.dim * frame.backdrop
        } else {
            0.0
        }
    });

    if let Some(visuals) = visuals {
        let mut sprites = Sprites {
            elements: &mut *elements,
            renderer: &mut *renderer,
            decorator: &mut *decorator,
            visuals,
            scale,
            now,
            clock_warp: lock.clock_warp,
            frost,
        };
        sprites.layer(Layer::Prompt, frame.prompt, frame.shake);
        if frame.clock.is_visible() {
            sprites.depth(frame.clock.opacity);
        }
        sprites.layer(Layer::Clock, frame.clock, 0.0);
    }

    let dim = lock.dim * frame.backdrop;
    if dim > 0.0 {
        elements.push(solid(&lock.ids.dim, output, [0.0, 0.0, 0.0, dim]));
    }

    if strength > 0.0 {
        let backdrop = GlassBackdrop {
            id: lock.ids.glass.clone(),
            commit: CommitCounter::from(
                decorator.blur_fingerprint().unwrap_or_default() as usize ^ quantise(strength),
            ),
            geometry: output,
            mask: output,
            radius: 0.0,
            glass: clear_glass(),
            alpha: 1.0,
            strength,
        };
        if let Some(glass) = decorator.backdrop(renderer, backdrop) {
            elements.push(CrownElement::Tile(Wrap::from(glass)));
        }
    }

    let fade = if unlocking { frame.backdrop } else { 1.0 };
    match lock.backdrop {
        Backdrop::Wallpaper if !unlocking => {
            layer_elements(
                elements,
                monitor,
                renderer,
                decorator,
                scale,
                &[ShellLayer::Background],
            );
        }
        Backdrop::Wallpaper => {}
        Backdrop::Picture => {
            if let Some(picture) = visuals.and_then(|visuals| visuals.picture.as_ref())
                && let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    Point::<f64, Physical>::default(),
                    picture,
                    Some(fade),
                    None,
                    None,
                    Kind::Unspecified,
                )
            {
                elements.push(Unstretched::at_own_size(element, scale).into());
            }
        }
        Backdrop::Color([r, g, b, a]) => {
            let alpha = a * fade;
            elements.push(solid(
                &lock.ids.color,
                output,
                [r * alpha, g * alpha, b * alpha, alpha],
            ));
        }
    }

    if unlocking {
        return true;
    }
    elements.push(solid(&lock.ids.base, output, FLOOR));
    false
}

struct Sprites<'a, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    D: TileDecorator<R>,
{
    elements: &'a mut Elements<R, D>,
    renderer: &'a mut R,
    decorator: &'a mut D,
    visuals: &'a OutputVisuals,
    scale: Scale<f64>,
    now: Instant,
    clock_warp: [[f32; 2]; 4],
    /// The dim to take out of the blur behind glass digits; `None` while the
    /// digits are not glass.
    frost: Option<f32>,
}

impl<R, D> Sprites<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    /// One layer's sprites, front to back, scaled about the layer's centre.
    fn layer(&mut self, layer: Layer, style: LayerStyle, shake: f32) {
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
        let sprites: Vec<&PreparedSprite> = in_layer().collect();
        for sprite in sprites.into_iter().rev() {
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

    /// The background's subject, in front of the clock.
    fn depth(&mut self, opacity: f32) {
        let Some(depth) = &self.visuals.depth else {
            return;
        };
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            self.renderer,
            Point::<f64, Physical>::default(),
            depth,
            Some(opacity),
            None,
            None,
            Kind::Unspecified,
        ) {
            self.elements
                .push(Unstretched::at_own_size(element, self.scale).into());
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
    /// blurred and scaled about its own centre by its `morph`.
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
        let glass = self.frost.filter(|_| is_face(sprite));
        if warp.is_none()
            && glass.is_none()
            && (morph.scale - 1.0).abs() < 1e-3
            && morph.blur * (self.scale.x as f32) < VISIBLE_BLUR
        {
            self.sprite(sprite, flat, pivot, 0.0);
            return;
        }
        let scale = self.scale.x as f32;
        let [x, y] = placed(sprite.rect, style, pivot).centre();
        let zoom = combined.scale;
        let [width, height] = sprite.pixels.map(|side| side as f32 * zoom);
        let padded = Rect {
            x: x * scale - width / 2.0,
            y: y * scale - height / 2.0,
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
            None,
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
        let pixels = sprite.pixels[1] as f32;
        let pad = sprite.pad as f32;
        let draw = GlyphDraw {
            unwarp,
            reach: [blur / padded.width, blur / padded.height],
            glass: glass.map(|dim| GlyphGlass {
                id: sprite.glass_id.clone(),
                geometry,
                dim,
                wash: GLASS_TINT,
                rows: [pad / pixels, (pad + sprite.rect.height * scale) / pixels],
                edge: GLASS_EDGE * scale / pixels,
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
        let x = place(sprite.rect.x, pivot[0], style.offset[0] + nudge);
        let y = place(sprite.rect.y, pivot[1], style.offset[1]);
        let pad = sprite.pad as f32 / scale * zoom;
        let origin = Point::<f64, Logical>::from((f64::from(x - pad), f64::from(y - pad)))
            .to_physical(self.scale)
            .to_i32_round::<i32>()
            .to_f64();

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
                None,
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

fn is_face(sprite: &PreparedSprite) -> bool {
    matches!(&sprite.content, Content::Glyph(run) if matches!(run.paint, GlyphPaint::Face { .. }))
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

fn solid<R, E>(
    id: &smithay::backend::renderer::element::Id,
    geometry: Rectangle<i32, Physical>,
    color: [f32; 4],
) -> CrownElement<R, E>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    E: smithay::backend::renderer::element::RenderElement<R>,
{
    let commit = CommitCounter::from(quantise(color[3]) ^ (quantise(color[0]) << 11));
    SolidColorRenderElement::new(id.clone(), geometry, commit, color, Kind::Unspecified).into()
}

fn quantise(value: f32) -> usize {
    (value.clamp(0.0, 1.0) * STEPS) as usize
}

/// Blur and nothing else: no tint, no rim, the backdrop's own colours.
fn clear_glass() -> Glass {
    Glass {
        tint: [0.0; 4],
        tint_inset: 0.0,
        saturation: 1.0,
        rim: 0.0,
        shading: Shading::default(),
    }
}
