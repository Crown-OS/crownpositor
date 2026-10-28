/// How a layer is drawn at one instant: what transitions animate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerStyle {
    pub opacity: f32,
    /// Logical pixels.
    pub offset: [f32; 2],
    /// About the layer's own centre.
    pub scale: f32,
    /// Blur radius in logical pixels.
    pub blur: f32,
}

impl LayerStyle {
    pub const REST: Self = Self {
        opacity: 1.0,
        offset: [0.0; 2],
        scale: 1.0,
        blur: 0.0,
    };

    pub const HIDDEN: Self = Self {
        opacity: 0.0,
        ..Self::REST
    };

    /// Both displacements at once, as when a layer leaves mid-entrance.
    pub fn then(self, other: Self) -> Self {
        Self {
            opacity: self.opacity * other.opacity,
            offset: [
                self.offset[0] + other.offset[0],
                self.offset[1] + other.offset[1],
            ],
            scale: self.scale * other.scale,
            blur: self.blur + other.blur,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.opacity > 1.0 / 512.0
    }
}
