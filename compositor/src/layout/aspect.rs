//! Keeping a window's width to height ratio, for Android apps that resize
//! only in proportion.

use protocols::crownos_android_shell::AspectRatio;
use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    utils::{Logical, Size},
};

/// Which side of a window a resize sets, the other following from the ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Width,
    Height,
}

impl Driver {
    /// The side a drag on `edges` drives. A corner drives whichever side the
    /// drag changed more, relative to the size it started from.
    pub fn of(edges: ResizeEdge, from: Size<i32, Logical>, to: Size<i32, Logical>) -> Self {
        match edges {
            ResizeEdge::Left | ResizeEdge::Right => Self::Width,
            ResizeEdge::Top | ResizeEdge::Bottom => Self::Height,
            _ => {
                let change =
                    |from: i32, to: i32| f64::from((to - from).abs()) / f64::from(from.max(1));
                if change(from.h, to.h) > change(from.w, to.w) {
                    Self::Height
                } else {
                    Self::Width
                }
            }
        }
    }
}

/// `size` with the side `driver` names kept and the other one set by `ratio`.
pub fn follow(ratio: AspectRatio, size: Size<i32, Logical>, driver: Driver) -> Size<i32, Logical> {
    match driver {
        Driver::Width => Size::from((size.w, scale(size.w, ratio.height.get(), ratio.width.get()))),
        Driver::Height => {
            Size::from((scale(size.h, ratio.width.get(), ratio.height.get()), size.h))
        }
    }
}

/// The largest size with `ratio` that fits inside `bounds`.
pub fn fit_within(ratio: AspectRatio, bounds: Size<i32, Logical>) -> Size<i32, Logical> {
    let by_width = follow(ratio, bounds, Driver::Width);
    if by_width.h <= bounds.h {
        by_width
    } else {
        follow(ratio, bounds, Driver::Height)
    }
}

/// `length * numerator / denominator`, rounded, and never below a pixel.
fn scale(length: i32, numerator: u32, denominator: u32) -> i32 {
    let scaled = (f64::from(length) * f64::from(numerator) / f64::from(denominator)).round();
    (scaled as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portrait() -> AspectRatio {
        AspectRatio::new(9, 16).expect("a ratio")
    }

    fn size(w: i32, h: i32) -> Size<i32, Logical> {
        Size::from((w, h))
    }

    #[test]
    fn a_side_drag_keeps_the_ratio_from_the_dragged_side() {
        assert_eq!(
            follow(portrait(), size(450, 300), Driver::Width),
            size(450, 800)
        );
        assert_eq!(
            follow(portrait(), size(10, 1600), Driver::Height),
            size(900, 1600)
        );
    }

    #[test]
    fn a_corner_follows_the_side_that_moved_more() {
        let from = size(450, 800);
        assert_eq!(
            Driver::of(ResizeEdge::BottomRight, from, size(500, 1200)),
            Driver::Height
        );
        assert_eq!(
            Driver::of(ResizeEdge::TopLeft, from, size(900, 820)),
            Driver::Width
        );
        assert_eq!(
            Driver::of(ResizeEdge::Top, from, size(900, 820)),
            Driver::Height
        );
    }

    #[test]
    fn a_slot_is_filled_as_far_as_the_ratio_allows() {
        assert_eq!(fit_within(portrait(), size(1000, 800)), size(450, 800));
        assert_eq!(fit_within(portrait(), size(450, 2000)), size(450, 800));
        let square = AspectRatio::new(1, 1).expect("a ratio");
        assert_eq!(fit_within(square, size(1280, 720)), size(720, 720));
    }

    #[test]
    fn nothing_shrinks_below_a_pixel() {
        let wide = AspectRatio::new(10_000, 1).expect("a ratio");
        assert_eq!(follow(wide, size(100, 50), Driver::Width), size(100, 1));
    }
}
