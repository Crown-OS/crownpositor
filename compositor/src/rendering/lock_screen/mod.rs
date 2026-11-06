//! What an output shows while the built-in lock is up, back to front: an
//! opaque floor, so nothing of the desktop shows through whatever is still
//! loading; the background, zoomed and blurred as the frame says; its
//! darkening; the clock, its digits frosted glass; the background's subject,
//! cut out for depth; and the prompt.
//!
//! Once the password is accepted the floor goes and the desktop is drawn
//! beneath, so the background fades off the real desktop.

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
/// The least of the frost the pyramid may have kept through a darkening it
/// lifted, so brightening it back never divides by a near-black.
const LEAST_FROST_KEPT: f32 = 0.25;
/// Blur radii below this many pixels draw sharp.
const VISIBLE_BLUR: f32 = 0.5;
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
    let visuals = lock.visuals.get(&monitor.id());
    let background = Background {
        lock,
        monitor,
        scale,
        style: frame.wallpaper,
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
            frame.subject(),
            background.subject_zoom(),
            background.logical_size(),
        );
        sprites.layer(Layer::Clock, frame.clock, 0.0);
    }

    if darkening > 0.0 {
        elements.push(solid(&lock.ids.dim, output, [0.0, 0.0, 0.0, darkening]));
    }
    elements.extend(blur.map(|blur| CrownElement::Tile(Wrap::from(blur))));
    background.draw(elements, renderer, visuals);

    if lock.flow.is_unlocking() {
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
