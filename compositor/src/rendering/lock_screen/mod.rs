//! What an output shows while the built-in lock is up, back to front: an
//! opaque floor, so nothing of the desktop shows through whatever is still
//! loading; the background, zoomed and blurred as the frame says; its
//! darkening; the clock, its digits frosted glass; the background's subject,
//! cut out for depth; and the prompt.
//!
//! While the lock arrives, and once the password is accepted, the floor goes
//! and the desktop is drawn beneath: the clock and subject rise in over the
//! real desktop and the background floods out from behind the subject, and
//! on unlock the background fades off it.

mod background;
mod sprites;

use std::time::Instant;

use lockscreen::layout::Layer;
use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{Id, Kind, RenderElement, Wrap, solid::SolidColorRenderElement},
        utils::CommitCounter,
    },
    utils::{Physical, Rectangle, Scale},
};

use crate::{
    rendering::{Elements, decorate::TileDecorator, element::CrownElement},
    shell::{lock_screen::ActiveLock, monitor::Monitor},
};

use background::Background;
use sprites::Sprites;

const FLOOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Where the background floods out from with neither a subject nor a clock
/// to well up behind, as a fraction of the output across and down.
const BOTTOM_MIDDLE: [f32; 2] = [0.5, 1.0];
/// The least of the frost the pyramid may have kept through a darkening it
/// lifted, so brightening it back never divides by a near-black.
const LEAST_FROST_KEPT: f32 = 0.25;
/// Blur radii below this many pixels draw sharp.
const VISIBLE_BLUR: f32 = 0.5;
/// Quantises animated alphas into commit counters.
const STEPS: f32 = 1024.0;

/// Pushes the lock for `monitor`. Returns whether the desktop should be drawn
/// beneath it: only while the lock arrives or the unlock animates away.
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
    let height = monitor.geometry().size.h as f32;
    let frame = lock.flow.frame(now).risen(height);
    let visuals = lock.visuals.get(&monitor.id());
    let [across, down] = visuals
        .and_then(sprites::flood_source)
        .unwrap_or(BOTTOM_MIDDLE);
    let background = Background {
        lock,
        monitor,
        visuals,
        scale,
        style: frame.wallpaper,
        flood: lock.flow.is_arriving(now).then_some(frame.flood),
        source: [across, down + 1.0 - frame.rise],
    };
    let output = background.output();
    let darkening = frame.darkening(lock.dim);
    let blur = background.blur(renderer, decorator);
    let lifted = if blur.is_some() { 0.0 } else { darkening };

    if let Some(visuals) = visuals {
        let mut sprites = Sprites {
            elements: &mut *elements,
            renderer: &mut *renderer,
            decorator: &mut *decorator,
            visuals,
            scale,
            now,
            clock_warp: lock.clock_warp,
            frost_gain: frost_gain(lifted, frame.frost_darkening(lock.dim)),
        };
        sprites.layer(Layer::Prompt, frame.prompt, frame.shake);
        sprites.subject(
            frame.subject,
            background.subject_zoom(),
            background.logical_size(),
        );
        sprites.layer(Layer::Clock, frame.clock, 0.0);
    }

    if darkening > 0.0 {
        elements.push(solid(&lock.ids.dim, output, [0.0, 0.0, 0.0, darkening]));
    }
    elements.extend(blur.map(|blur| CrownElement::Tile(Wrap::from(blur))));
    let floods = background.draw(elements, renderer, decorator);

    if floods || lock.flow.is_unlocking() {
        return true;
    }
    elements.push(solid(&lock.ids.base, output, FLOOR));
    false
}

/// What the frost behind a glass digit is multiplied by to carry `wanted`
/// darkening, when the pyramid lifted it carrying `lifted`: the background
/// blur lifts it from under the darkening, a digit's own glass from over it.
fn frost_gain(lifted: f32, wanted: f32) -> f32 {
    (1.0 - wanted) / (1.0 - lifted).max(LEAST_FROST_KEPT)
}

fn solid<R, E>(id: &Id, geometry: Rectangle<i32, Physical>, color: [f32; 4]) -> CrownElement<R, E>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + 'static,
    E: RenderElement<R>,
{
    let commit = CommitCounter::from(quantise(color[3]) ^ (quantise(color[0]) << 11));
    SolidColorRenderElement::new(id.clone(), geometry, commit, color, Kind::Unspecified).into()
}

fn quantise(value: f32) -> usize {
    (value.clamp(0.0, 1.0) * STEPS) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frost_is_brightened_back_by_what_the_pyramid_lifted() {
        assert_eq!(frost_gain(0.0, 0.0), 1.0);
        assert!((frost_gain(0.285, 0.0) * (1.0 - 0.285) - 1.0).abs() < 1e-6);
        assert!((frost_gain(0.0, 0.4) - 0.6).abs() < 1e-6);
        assert_eq!(frost_gain(1.0, 0.0), 1.0 / LEAST_FROST_KEPT);
    }
}
