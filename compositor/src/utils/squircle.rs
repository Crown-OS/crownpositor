//! The CPU side of the corner shape every shader draws.

/// How far along each edge a squircle corner of radius `r` reaches, as a
/// multiple of `r`. Mirrors `SQUIRCLE_EXTENT` in
/// `shaders/common/rounded_box.glsl`; geometry that assumes a corner stays
/// inside its radius has to reach this far instead.
pub const EXTENT: f32 = 1.5;

/// The distance from a corner along either edge at which the curve begins.
pub fn reach(radius: f32) -> f32 {
    radius.max(0.0) * EXTENT
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_rust_and_glsl_extents_agree() {
        let glsl = include_str!("../shaders/common/rounded_box.glsl");
        assert!(glsl.contains(&format!(
            "const float SQUIRCLE_EXTENT = {:?};",
            super::EXTENT
        )));
    }
}
