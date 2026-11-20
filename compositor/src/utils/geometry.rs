use smithay::utils::{Logical, Point, Rectangle};

pub fn centre(rectangle: Rectangle<i32, Logical>) -> Point<f64, Logical> {
    let rectangle = rectangle.to_f64();
    rectangle.loc + rectangle.size.downscale(2.0).to_point()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centre_of_an_offset_rectangle_is_offset_too() {
        let rectangle = Rectangle::new((1920, 100).into(), (1280, 720).into());
        assert_eq!(centre(rectangle), Point::from((2560.0, 460.0)));
    }
}
