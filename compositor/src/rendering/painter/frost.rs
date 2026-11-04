//! The frosted pane a compositor-drawn scene stands on: whatever is under it
//! blurred and dimmed, the overview's wallpaper and the Alt+Tab desktop alike.

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer, element::Id},
    utils::{Physical, Rectangle},
};

use super::Painter;
use crate::rendering::{
    blur::{Glass, GlassKind},
    decorate::{Backdrop, TileDecorator},
};

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    /// `blur` and `dim` are 0 to 1. The blur grows rather than a finished
    /// blur being faded over a sharp scene, which would read as a double
    /// image.
    pub(in crate::rendering) fn frost(
        &mut self,
        id: Id,
        geometry: Rectangle<i32, Physical>,
        blur: f32,
        dim: f32,
    ) {
        if blur <= 0.0 || self.decorator.blur_fingerprint().is_none() {
            return;
        }
        let backdrop = Backdrop {
            id,
            commit: self.glass_commit(),
            geometry,
            mask: geometry,
            radius: 0.0,
            glass: dimmed(self.decorator.glass(self.scale.x, GlassKind::Window), dim),
            alpha: blur,
            strength: blur,
        };
        if let Some(pane) = self.decorator.backdrop(self.renderer, backdrop) {
            self.push(pane);
        }
    }
}

/// `glass` with a black wash of `dim` folded into its tint — the same pixels
/// as a separate quad laid over it, without anything between the pane and the
/// glass in front of it — and no rim, because its edges are the screen's.
fn dimmed(glass: Glass, dim: f32) -> Glass {
    let [red, green, blue, alpha] = glass.tint;
    let washed = 1.0 - (1.0 - alpha) * (1.0 - dim);
    let kept = if washed > 0.0 {
        alpha * (1.0 - dim) / washed
    } else {
        0.0
    };
    Glass {
        tint: [red * kept, green * kept, blue * kept, washed],
        rim: 0.0,
        ..glass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mix(colour, tint.rgb, tint.a)`, as the finish shader applies it.
    fn tinted(colour: f32, tint: [f32; 4]) -> f32 {
        colour * (1.0 - tint[3]) + tint[0] * tint[3]
    }

    #[test]
    fn the_folded_dim_matches_a_wash_laid_over_the_glass() {
        let glass = Glass::default();
        for dim in [0.0, 0.2, 0.4] {
            for colour in [0.0, 0.3, 1.0] {
                let washed = tinted(colour, glass.tint) * (1.0 - dim);
                let folded = tinted(colour, dimmed(glass, dim).tint);
                assert!(
                    (washed - folded).abs() < 1e-6,
                    "{dim} {colour}: {washed} {folded}"
                );
            }
        }
    }

    #[test]
    fn the_pane_has_no_rim() {
        let glass = Glass {
            rim: 3.0,
            ..Glass::default()
        };
        assert_eq!(dimmed(glass, 0.4).rim, 0.0);
    }
}
