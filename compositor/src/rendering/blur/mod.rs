//! GPU blur for `ext-background-effect-v1` surfaces.
//!
//! Every backdrop blurs what is actually beneath it, at the moment it is drawn.
//! The damage tracker composites back to front, so when a [`BlurBackdrop`]'s
//! `draw` runs the framebuffer already holds everything below it and nothing
//! above: the element blits the damaged part of that into a window-sized scene
//! texture, runs a dual-kawase pyramid over the dirty footprint, and the final
//! upsample *is* the draw.
//!
//! The pyramid outlives the frame ([`BlurCache`], keyed by element id) because
//! only the damaged rectangles are copied. Outside them the framebuffer still
//! holds the previous composite — this window included — and blurring that
//! would feed the effect its own output; what the cache keeps instead is the
//! scene from the frames that did own those pixels.
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
use protocols::background_effect;

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
}

impl From<&Appearance> for BlurConfig {
    /// The file speaks in user units; the pipeline wants what the shader can
    /// hold, so this is where the narrowing and the sanity clamps happen.
    fn from(appearance: &Appearance) -> Self {
        Self {
            enabled: appearance.blur,
            passes: appearance.blur_passes.min(u8::MAX.into()) as u8,
            offset: appearance.blur_size.max(0.0) as f32,
            noise: appearance.blur_noise.clamp(0.0, 1.0) as f32,
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

    /// Identifies the settings a backdrop was drawn with. Everything here is
    /// applied at composite time, so a change to any of it repaints.
    pub fn fingerprint(&self) -> u64 {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        [
            self.passes() as u32,
            self.offset.to_bits(),
            self.noise.to_bits(),
        ]
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, field| {
            (hash ^ u64::from(field)).wrapping_mul(PRIME)
        })
    }
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
    let rect = rect.intersection(surface)?;

    // Rounded as extremities rather than as location-plus-size: at a
    // fractional scale the latter lets two rectangles that shared an edge in
    // logical space end up a pixel apart, and a seam through a translucent
    // backdrop is plainly visible.
    let placed = Rectangle::from_extremities(
        origin + rect.loc.to_physical_precise_round(scale),
        origin + (rect.loc + rect.size.to_point()).to_physical_precise_round(scale),
    );

    let placed = placed.intersection(clip)?;
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
    with_states(surface, |states| {
        let slots = states
            .data_map
            .get_or_insert_threadsafe(BackdropSlots::default);
        let mut slots = slots.0.lock().unwrap();

        if slots.seen != Some((fingerprint, generation)) {
            slots.seen = Some((fingerprint, generation));
            slots.commit.increment();
        }

        // Grown, never shrunk, so a region that gains and loses a rectangle
        // does not hand the same piece a new identity each time. The cap on
        // rectangles caps this too.
        while slots.ids.len() < count {
            slots.ids.push(Id::new());
        }

        (slots.ids[..count].to_vec(), slots.commit)
    })
}

#[derive(Debug, Default)]
struct BackdropSlots(Mutex<BackdropSlotsInner>);

#[derive(Debug, Default)]
struct BackdropSlotsInner {
    ids: Vec<Id>,
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
        ] {
            assert_ne!(config.fingerprint(), changed.fingerprint());
        }
    }
}
