//! The animatable properties, indexed the way the protocol numbers them.

pub use crownos_protocols::surface_animation::v1::server::crownos_animated_surface_v1::Property;

pub const PROPERTIES: [Property; 5] = [
    Property::TranslateX,
    Property::TranslateY,
    Property::Scale,
    Property::Rotation,
    Property::Opacity,
];

pub fn index(property: Property) -> usize {
    property as usize
}

pub fn resting_value(property: Property) -> f32 {
    match property {
        Property::Scale | Property::Opacity => 1.0,
        _ => 0.0,
    }
}

/// Rotation is accepted but not drawn: smithay's element wrappers can move,
/// scale and fade a surface, but turning one needs a custom vertex path. Such
/// a property jumps straight to its target and settles at once, so a client
/// waiting on `settled` is not left hanging.
pub fn is_rendered(property: Property) -> bool {
    property != Property::Rotation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_follow_the_protocol_numbering() {
        for (position, property) in PROPERTIES.into_iter().enumerate() {
            assert_eq!(index(property), position);
        }
    }
}
