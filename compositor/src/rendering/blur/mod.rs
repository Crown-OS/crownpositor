//! GPU blur for `ext-background-effect-v1` surfaces.
//!
//! Every backdrop blurs what is actually beneath it, at the moment it is drawn.
//! The damage tracker composites back to front, so when a [`BlurBackdrop`]'s
//! `draw` runs the framebuffer already holds everything below it and nothing
//! above: the element blits the damaged part of that into the output's scene
//! texture, runs a dual-kawase pyramid over the dirty footprint, and the final
//! upsample *is* the draw.
//!
//! One scene and one pyramid serve the whole output, not one per backdrop, and
//! they outlive the frame ([`BlurCache`]) because only the damaged rectangles
//! are copied. Outside them the framebuffer still holds the previous composite
//! — this window included — and blurring that would feed the effect its own
//! output; what the cache keeps instead is the scene from the frames that did
//! own those pixels.
//!
//! Sharing them is what makes overlapping glass one layer of blur rather than
//! two. A backdrop lifts its damage into the scene *minus* the rectangles this
//! frame's earlier backdrops already covered, so a popup over a bar blurs the
//! desktop the bar blurred, not the bar's own glass; glass writes opaque, so
//! the upper piece simply wins where they meet. The same sharing is why two
//! pieces that merely touch blur across their shared edge instead of each
//! clamping its taps at it.
//!
//! A pass spreads a changed pixel further than the rectangle it arrived in, so
//! a draw leaves a stale band around its own damage. The cache offers that band
//! back as the next frame's damage, and the backend is asked for that frame
//! through [`BlurCache::wants_redraw`]. An offer nobody claims — a backdrop the
//! tracker has since hidden behind an opaque window — expires with the frame.
//!
//! It degrades: no shaders, no blit capability, an allocation failure — the
//! backdrop element simply isn't emitted and windows draw as before.
//!
//! Everything here talks to a plain [`GlesRenderer`]; on the KMS backend that
//! is the render node's GLES context under the multi-GPU wrapper (the same one
//! the rounded-corner shader binds through), so the pyramid lives exactly where
//! the output's composition happens.
//!
//! [`GlesRenderer`]: smithay::backend::renderer::gles::GlesRenderer

mod backdrop;
mod cache;
mod scene;

pub use backdrop::BlurBackdrop;
pub use cache::{BlurCache, BlurSession};

use std::sync::Mutex;

use smithay::{
    backend::renderer::{
        element::Id,
        utils::{CommitCounter, with_renderer_surface_state},
    },
    desktop::{Window, WindowSurface},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Scale},
    wayland::compositor::with_states,
};

use config::Appearance;
use protocols::{background_effect, crownos_background_effects as crownos};

/// Runtime knobs for the blur pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurConfig {
    pub enabled: bool,
    /// Downsample depth of the kawase pyramid. Each pass halves the
    /// resolution, so perceived radius grows exponentially with this.
    pub passes: u8,
    /// Kawase tap spread, in (level-local) pixels. Fractional values are the
    /// point: the taps land between texels and the bilinear filter does the
    /// averaging.
    pub offset: f32,
    /// Dither strength applied when compositing, to hide gradient banding.
    pub noise: f32,
    /// Chroma multiplier about the luma. Above 1.0 is the vibrancy that keeps
    /// colour alive through a heavy blur instead of letting it wash to grey.
    pub vibrancy: f32,
    /// Width of the refractive rim inside a piece of glass's edge, in *logical*
    /// pixels. Matched to the border width, because the rim is what a border on
    /// glass actually looks like.
    pub rim: f32,
}

/// The vibrancy the compositor's own glass is drawn with: 130%, which is where
/// a wallpaper's colour still reads through frosted glass without the whole
/// surface turning into a stained-glass window.
const VIBRANCY: f32 = 1.3;

/// A whisper of white over the blur, so glass over a dark backdrop still reads
/// as a surface rather than as a hole.
const SHEEN: [f32; 4] = [1.0, 1.0, 1.0, 0.06];

impl From<&Appearance> for BlurConfig {
    /// The file speaks in user units; the pipeline wants what the shader can
    /// hold, so this is where the narrowing and the sanity clamps happen.
    fn from(appearance: &Appearance) -> Self {
        Self {
            enabled: appearance.blur,
            passes: appearance.blur_passes.min(u8::MAX.into()) as u8,
            offset: appearance.blur_size.max(0.0) as f32,
            noise: appearance.blur_noise.clamp(0.0, 1.0) as f32,
            vibrancy: VIBRANCY,
            rim: appearance.border_width as f32,
        }
    }
}

impl Default for BlurConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            passes: 3,
            offset: 1.5,
            noise: 0.01,
            vibrancy: VIBRANCY,
            rim: 2.0,
        }
    }
}

impl BlurConfig {
    fn passes(&self) -> usize {
        // Below 1 the pyramid does not exist; above 8 the smallest level of
        // any real output is a pixel and the extra passes only burn time.
        self.passes.clamp(1, 8) as usize
    }

    /// How far a pixel's influence spreads, in full-resolution pixels: every
    /// halving doubles the tap spread, and the last upsample doubles it once
    /// more. The footprint the pyramid is rerun over grows by this.
    fn radius(&self) -> i32 {
        (self.offset.max(0.0) * (1u32 << (self.passes() + 1)) as f32).ceil() as i32
    }

    /// The material the compositor's own glass — window frames, menus, window
    /// previews — is made of, at one output's scale.
    pub fn glass(&self, scale: f64) -> Glass {
        Glass {
            tint: SHEEN,
            saturation: self.vibrancy,
            rim: (self.rim as f64 * scale) as f32,
        }
    }

    /// Identifies the settings a backdrop was drawn with. Everything here is
    /// applied at composite time, so a change to any of it repaints.
    pub fn fingerprint(&self) -> u64 {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        [
            self.passes() as u32,
            self.offset.to_bits(),
            self.noise.to_bits(),
            self.vibrancy.to_bits(),
            self.rim.to_bits(),
        ]
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, field| {
            (hash ^ u64::from(field)).wrapping_mul(PRIME)
        })
    }
}

/// The material one piece of glass is made of.
///
/// Separate from [`BlurConfig`] because the blur pipeline's settings are the
/// compositor's and the same for every surface on screen, while this is per
/// surface: a client that asked for its own tint and vibrancy through
/// `crownos_background_effects` gets them, and everything else gets the
/// compositor's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glass {
    /// Straight RGBA, mixed over the blurred backdrop.
    pub tint: [f32; 4],
    /// Chroma multiplier about the luma, applied before the tint.
    pub saturation: f32,
    /// Width of the refractive rim just inside the shape's edge, in physical
    /// pixels. Zero leaves the edge flat.
    pub rim: f32,
}

impl Default for Glass {
    /// Plain glass: the compositor's own sheen and vibrancy, with no rim. What
    /// a decorator that has no blur pipeline would draw with if it drew
    /// anything at all.
    fn default() -> Self {
        Self {
            tint: SHEEN,
            saturation: VIBRANCY,
            rim: 0.0,
        }
    }
}

/// One piece of glass on the output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassPiece {
    /// The rectangle to fill, already clipped to what is actually on screen.
    pub geometry: Rectangle<i32, Physical>,
    /// The rounded rectangle the edge is cut from, *un*clipped — so a shape
    /// running off the surface still curves against itself rather than growing
    /// four new corners at the clip.
    pub mask: Rectangle<i32, Physical>,
    pub radius: f32,
}

/// One blurred silhouette cast under a surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowPiece {
    /// The element's own rect: the silhouette, offset and then grown to hold
    /// the gaussian's tail.
    pub geometry: Rectangle<i32, Physical>,
    /// The silhouette itself, in the same output-local space.
    pub shape: Rectangle<i32, Physical>,
    pub radius: f32,
    /// Standard deviation of the gaussian, in physical pixels.
    pub sigma: f32,
    /// Premultiplied, as the shader expects.
    pub color: [f32; 4],
}

/// Everything one surface asked `crownos_background_effects` for, placed on the
/// output.
#[derive(Debug, Clone)]
pub struct SurfaceEffects {
    /// Bumped when the client committed different effects, so the elements
    /// built from this repaint.
    pub generation: u32,
    pub glass: Glass,
    pub pieces: Vec<GlassPiece>,
    pub shadows: Vec<ShadowPiece>,
}

/// How many standard deviations of the gaussian a shadow's element has to hold
/// before its tail is below one step of an 8-bit channel.
const SHADOW_TAIL: f32 = 3.0;

/// A blur radius as the protocol states it, in standard deviations. The
/// convention every toolkit uses: the visible edge of a gaussian blur sits at
/// about twice its sigma.
fn sigma(radius: u32) -> f32 {
    radius as f32 * 0.5
}

/// The corner radius a surface asked for through
/// `crownos_background_effects.set_corner_radius`, in physical pixels.
///
/// `None` when it asked for none, which leaves the compositor's own radius in
/// charge. Read separately from the rest of the effects because it clips the
/// client's own texture, which happens whether or not there is a blur pipeline
/// to draw glass with.
pub fn surface_corner_radius(surface: &WlSurface, scale: f64) -> Option<f32> {
    let committed = with_states(surface, crownos::surface_effects)?;
    let radius = committed.effects.corner_radius;
    (radius > 0).then_some(radius as f32 * scale as f32)
}

/// Places a surface's committed `crownos_background_effects` on the output.
///
/// `origin` is where the surface's own `(0, 0)` lands in output-local physical
/// coordinates, and `clip` bounds the glass — the shadow is deliberately not
/// clipped, because it is drawn outside the surface by design.
///
/// `None` when the surface committed no effects at all, which is not the same
/// as coming back with no pieces: a client may legitimately set a shape that
/// covers nothing.
pub fn place_surface_effects(
    surface: &WlSurface,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    clip: Rectangle<i32, Physical>,
) -> Option<SurfaceEffects> {
    let committed = with_states(surface, crownos::surface_effects)?;
    let effects = &committed.effects;

    // The surface size is only known once a buffer is attached; before that
    // there is nothing to draw behind anyway, so an absent size means an empty
    // placement rather than an unclipped one.
    let surface_size = with_renderer_surface_state(surface, |state| state.surface_size()).flatten();
    let bounds = Rectangle::from_size(surface_size.unwrap_or_default());

    let whole = crownos::RoundedRect::new(bounds, effects.corner_radius);
    let primitives = |shape: &Option<crownos::Primitives>| match shape {
        Some(shape) => shape.to_vec(),
        None => vec![whole],
    };

    let glass = Glass {
        tint: effects
            .blur
            .as_ref()
            .map_or([0.0; 4], |blur| blur.tint.channels()),
        saturation: effects
            .blur
            .as_ref()
            .map_or(1.0, |blur| blur.saturation as f32),
        rim: (effects.border_width as f64 * scale.x) as f32,
    };

    let pieces = effects
        .blur
        .as_ref()
        .map(|blur| {
            primitives(&blur.shape)
                .iter()
                .filter_map(|primitive| place_piece(primitive, origin, scale, bounds, clip))
                .collect()
        })
        .unwrap_or_default();

    let shadows = effects
        .shadow
        .as_ref()
        .map(|shadow| {
            let sigma = sigma(shadow.radius) * scale.x as f32;
            let color = shadow.color.premultiplied();
            primitives(&shadow.shape)
                .iter()
                .filter_map(|primitive| {
                    place_shadow(primitive, shadow.offset, origin, scale, sigma, color)
                })
                .collect()
        })
        .unwrap_or_default();

    Some(SurfaceEffects {
        generation: committed.generation,
        glass,
        pieces,
        shadows,
    })
}

/// Places one primitive of a blur shape. `None` when nothing of it survives the
/// surface and the clip.
fn place_piece(
    primitive: &crownos::RoundedRect,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    surface: Rectangle<i32, Logical>,
    clip: Rectangle<i32, Physical>,
) -> Option<GlassPiece> {
    // The mask is the whole primitive and the geometry is the part of it that
    // is on screen. That split is the clipping the protocol asks for: the shape
    // keeps its own curve, and what falls outside the surface is simply not
    // drawn.
    let mask = place(primitive.rect, origin, scale);
    let geometry = mask
        .intersection(place(surface, origin, scale))?
        .intersection(clip)?;

    (!geometry.is_empty()).then_some(GlassPiece {
        geometry,
        mask,
        radius: primitive.clamped_radius() * scale.x as f32,
    })
}

/// Places one primitive of a shadow shape, offset and grown for its blur.
fn place_shadow(
    primitive: &crownos::RoundedRect,
    offset: Point<i32, Logical>,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    sigma: f32,
    color: [f32; 4],
) -> Option<ShadowPiece> {
    let shape = place(
        Rectangle::new(primitive.rect.loc + offset, primitive.rect.size),
        origin,
        scale,
    );
    if shape.is_empty() {
        return None;
    }

    let tail = (sigma * SHADOW_TAIL).ceil() as i32;
    let margin = Point::from((tail, tail));
    let geometry = Rectangle::from_extremities(
        shape.loc - margin,
        shape.loc + shape.size.to_point() + margin,
    );

    Some(ShadowPiece {
        geometry,
        shape,
        radius: primitive.clamped_radius() * scale.x as f32,
        sigma,
        color,
    })
}

/// A surface-local rectangle in output-local physical coordinates.
///
/// Rounded as extremities rather than as location-plus-size: at a fractional
/// scale the latter lets two rectangles that shared an edge in logical space
/// end up a pixel apart, and a seam through translucent glass is plainly
/// visible.
fn place(
    rect: Rectangle<i32, Logical>,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
) -> Rectangle<i32, Physical> {
    Rectangle::from_extremities(
        origin + rect.loc.to_physical_precise_round(scale),
        origin + (rect.loc + rect.size.to_point()).to_physical_precise_round(scale),
    )
}

/// The `wl_surface` a window draws through, if it is a Wayland one.
///
/// X11 windows have no `wl_surface` of their own to hang a blur region on, so
/// they simply never blur.
pub fn window_surface(window: &Window) -> Option<WlSurface> {
    match window.underlying_surface() {
        WindowSurface::Wayland(toplevel) => Some(toplevel.wl_surface().clone()),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Places a surface's committed blur region on the output, in `out`.
///
/// `origin` is where the surface's own `(0, 0)` lands in output-local physical
/// coordinates, and `clip` bounds the result. Returns the region's generation,
/// or `None` when the surface committed no region at all — which is not the
/// same as `out` coming back empty, since a client may legitimately set a
/// region that covers nothing.
pub fn place_blur_region(
    surface: &WlSurface,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    clip: Rectangle<i32, Physical>,
    out: &mut Vec<Rectangle<i32, Physical>>,
) -> Option<u32> {
    out.clear();

    let region = with_states(surface, background_effect::blur_region)?;

    // "The blur region is specified in the surface-local coordinates, and
    // clipped by the compositor to the surface size." The size is only known
    // once a buffer is attached; before that there is nothing to draw behind
    // anyway, so an absent size means an empty placement rather than an
    // unclipped one.
    let surface_size = with_renderer_surface_state(surface, |state| state.surface_size()).flatten();
    let Some(surface_size) = surface_size else {
        return Some(region.generation);
    };
    let surface_rect = Rectangle::from_size(surface_size);

    out.extend(
        region
            .rects
            .iter()
            .filter_map(|rect| place_rect(*rect, origin, scale, surface_rect, clip)),
    );

    Some(region.generation)
}

/// Places one surface-local rectangle on the output, or drops it if nothing of
/// it survives the surface and the clip.
fn place_rect(
    rect: Rectangle<i32, Logical>,
    origin: Point<i32, Physical>,
    scale: Scale<f64>,
    surface: Rectangle<i32, Logical>,
    clip: Rectangle<i32, Physical>,
) -> Option<Rectangle<i32, Physical>> {
    let placed = place(rect.intersection(surface)?, origin, scale).intersection(clip)?;
    (!placed.is_empty()).then_some(placed)
}

/// Element identities and a commit counter for one surface's backdrops.
///
/// The ids have to be stable across frames or the damage tracker repaints the
/// surface's area every frame — and, since a pyramid is cached under its
/// element id, a new id every frame would also throw the scene copy away. The
/// counter has to advance exactly when the pixels change for a reason the
/// tracker cannot see for itself, which is when the blur settings changed
/// (`fingerprint`) or the client committed a different region (`generation`).
///
/// Both live on the surface, so a surface visited by two outputs in one frame
/// would make the counter oscillate. Nothing does that here: a window belongs
/// to one workspace on one monitor, and a layer surface to one output.
pub fn backdrop_slots(
    surface: &WlSurface,
    count: usize,
    fingerprint: u64,
    generation: u32,
) -> (Vec<Id>, CommitCounter) {
    slots(surface, count, fingerprint, generation, |slots| {
        &mut slots.backdrops
    })
}

/// The same, for the shadows a surface casts. A separate set of identities
/// because a shadow and the glass above it are separate elements that come and
/// go independently.
pub fn shadow_slots(
    surface: &WlSurface,
    count: usize,
    fingerprint: u64,
    generation: u32,
) -> (Vec<Id>, CommitCounter) {
    slots(surface, count, fingerprint, generation, |slots| {
        &mut slots.shadows
    })
}

fn slots(
    surface: &WlSurface,
    count: usize,
    fingerprint: u64,
    generation: u32,
    pick: impl FnOnce(&mut BackdropSlotsInner) -> &mut Vec<Id>,
) -> (Vec<Id>, CommitCounter) {
    with_states(surface, |states| {
        let slots = states
            .data_map
            .get_or_insert_threadsafe(BackdropSlots::default);
        let Ok(mut slots) = slots.0.lock() else {
            return (Vec::new(), CommitCounter::default());
        };

        if slots.seen != Some((fingerprint, generation)) {
            slots.seen = Some((fingerprint, generation));
            slots.commit.increment();
        }
        let commit = slots.commit;

        // Grown, never shrunk, so a shape that gains and loses a primitive does
        // not hand the same piece a new identity each time. The cap on
        // primitives caps this too.
        let ids = pick(&mut slots);
        while ids.len() < count {
            ids.push(Id::new());
        }

        (ids[..count].to_vec(), commit)
    })
}

#[derive(Debug, Default)]
struct BackdropSlots(Mutex<BackdropSlotsInner>);

#[derive(Debug, Default)]
struct BackdropSlotsInner {
    backdrops: Vec<Id>,
    shadows: Vec<Id>,
    commit: CommitCounter,
    seen: Option<(u64, u32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_clamps_passes() {
        let config = BlurConfig {
            passes: 0,
            ..Default::default()
        };
        assert_eq!(config.passes(), 1);
        let config = BlurConfig {
            passes: 40,
            ..Default::default()
        };
        assert_eq!(config.passes(), 8);
    }

    fn logical(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn physical(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_region_covering_the_surface_covers_the_whole_backdrop() {
        let placed = place_rect(
            logical(0, 0, 100, 50),
            (10, 20).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(10, 20, 100, 50),
        );
        assert_eq!(placed, Some(physical(10, 20, 100, 50)));
    }

    #[test]
    fn a_region_larger_than_the_surface_is_clipped_to_it() {
        // "The blur region is ... clipped by the compositor to the surface
        // size" — a client asking for more must not get more.
        let placed = place_rect(
            logical(-50, -50, 400, 400),
            (0, 0).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(0, 0, 1000, 1000),
        );
        assert_eq!(placed, Some(physical(0, 0, 100, 50)));
    }

    #[test]
    fn a_region_outside_the_surface_is_dropped() {
        let placed = place_rect(
            logical(200, 200, 10, 10),
            (0, 0).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(0, 0, 1000, 1000),
        );
        assert_eq!(placed, None);
    }

    #[test]
    fn a_region_outside_the_clip_is_dropped() {
        // A window mid-shrink still holds its old buffer; its blur must not
        // spill over the neighbour it is uncovering.
        let placed = place_rect(
            logical(0, 0, 100, 50),
            (0, 0).into(),
            Scale::from(1.0),
            logical(0, 0, 100, 50),
            physical(500, 500, 100, 50),
        );
        assert_eq!(placed, None);
    }

    #[test]
    fn adjacent_rectangles_stay_adjacent_at_a_fractional_scale() {
        // Rounding location and size separately would put a one-pixel seam
        // between these two, and a seam through translucent glass shows.
        let surface = logical(0, 0, 100, 100);
        let clip = physical(0, 0, 1000, 1000);
        let scale = Scale::from(1.25);

        for edge in 1..100 {
            let left = place_rect(logical(0, 0, edge, 10), (0, 0).into(), scale, surface, clip)
                .expect("left half is on screen");
            let right = place_rect(
                logical(edge, 0, 100 - edge, 10),
                (0, 0).into(),
                scale,
                surface,
                clip,
            )
            .expect("right half is on screen");

            assert_eq!(
                left.loc.x + left.size.w,
                right.loc.x,
                "seam at logical x={edge}"
            );
        }
    }

    fn primitive(x: i32, y: i32, w: i32, h: i32, radius: u32) -> crownos::RoundedRect {
        crownos::RoundedRect::new(logical(x, y, w, h), radius)
    }

    /// The point of keeping the mask whole: a shape running off its surface has
    /// to be cut by a straight line at the surface's edge, not grow four new
    /// corners there.
    #[test]
    fn a_clipped_primitive_keeps_its_own_curve() {
        let piece = place_piece(
            &primitive(-20, 0, 100, 40, 20),
            (0, 0).into(),
            Scale::from(1.0),
            logical(0, 0, 200, 40),
            physical(0, 0, 1000, 1000),
        )
        .expect("most of it is on the surface");

        assert_eq!(piece.mask, physical(-20, 0, 100, 40));
        assert_eq!(piece.geometry, physical(0, 0, 80, 40));
        assert_eq!(piece.radius, 20.0);
    }

    #[test]
    fn a_primitive_entirely_off_the_surface_is_dropped() {
        assert!(
            place_piece(
                &primitive(500, 0, 10, 10, 0),
                (0, 0).into(),
                Scale::from(1.0),
                logical(0, 0, 100, 100),
                physical(0, 0, 1000, 1000),
            )
            .is_none()
        );
    }

    #[test]
    fn a_pill_shaped_primitive_rounds_to_a_stadium_at_any_scale() {
        // The radius the client asked for is absurd; what draws is half the
        // shorter side, scaled with the output.
        let piece = place_piece(
            &primitive(0, 0, 200, 60, 9999),
            (0, 0).into(),
            Scale::from(2.0),
            logical(0, 0, 200, 60),
            physical(0, 0, 1000, 1000),
        )
        .expect("on screen");

        assert_eq!(piece.geometry, physical(0, 0, 400, 120));
        assert_eq!(piece.radius, 60.0);
    }

    #[test]
    fn a_shadow_is_offset_and_grown_to_hold_its_tail() {
        let piece = place_shadow(
            &primitive(0, 0, 100, 50, 8),
            (4, 10).into(),
            (0, 0).into(),
            Scale::from(1.0),
            8.0,
            [0.0, 0.0, 0.0, 0.5],
        )
        .expect("a shadow with a silhouette");

        assert_eq!(piece.shape, physical(4, 10, 100, 50));
        // Three standard deviations on every side, which is where an 8-bit
        // channel has nothing left to show.
        let tail = (8.0 * SHADOW_TAIL) as i32;
        assert_eq!(
            piece.geometry,
            physical(4 - tail, 10 - tail, 100 + 2 * tail, 50 + 2 * tail)
        );
    }

    #[test]
    fn a_shadow_is_not_clipped_to_the_surface() {
        // It is drawn outside the surface by design, so an offset that takes it
        // clear of the surface still produces an element.
        let piece = place_shadow(
            &primitive(0, 0, 40, 40, 0),
            (400, 400).into(),
            (0, 0).into(),
            Scale::from(1.0),
            2.0,
            [0.0, 0.0, 0.0, 1.0],
        );
        assert!(piece.is_some());
    }

    #[test]
    fn a_blur_radius_is_twice_its_standard_deviation() {
        // The convention every toolkit uses, and the one the protocol states.
        assert_eq!(sigma(16), 8.0);
        assert_eq!(sigma(0), 0.0);
    }

    #[test]
    fn the_rim_scales_with_the_output() {
        let config = BlurConfig {
            rim: 2.0,
            ..Default::default()
        };
        assert_eq!(config.glass(1.0).rim, 2.0);
        assert_eq!(config.glass(2.0).rim, 4.0);
        assert_eq!(config.glass(1.0).saturation, VIBRANCY);
    }

    #[test]
    fn fingerprint_tracks_the_knobs_that_invalidate_pixels() {
        let config = BlurConfig::default();

        // Every knob is applied while compositing, dither included, so each one
        // changing has to repaint the backdrops already on screen.
        for changed in [
            BlurConfig {
                noise: 0.5,
                ..config
            },
            BlurConfig {
                offset: 3.0,
                ..config
            },
            BlurConfig {
                passes: 5,
                ..config
            },
            // Both of these are applied in the same composite as the blur, so
            // changing either has to repaint the glass already on screen.
            BlurConfig {
                vibrancy: 1.6,
                ..config
            },
            BlurConfig { rim: 6.0, ..config },
        ] {
            assert_ne!(config.fingerprint(), changed.fingerprint());
        }
    }
}
