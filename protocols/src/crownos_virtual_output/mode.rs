//! The validation rules `crownos_virtual_output_v1` states for names and modes.

use smithay::utils::{Physical, Size};

pub const MAX_NAME_LEN: usize = 32;
pub const MAX_DIMENSION: i32 = 16384;
pub const SCALE_DENOMINATOR: u32 = 120;
pub const MIN_SCALE_120: u32 = 60;
pub const MAX_SCALE_120: u32 = 480;

/// Prefixed to every client-chosen name, so a virtual head can never be
/// mistaken for — or collide with — a connector.
pub const NAME_PREFIX: &str = "VIRTUAL-";

/// A mode a client asked a virtual output to have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualMode {
    pub size: Size<i32, Physical>,
    pub refresh_mhz: u32,
    pub scale_120: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ModeError {
    #[error("{0}x{1} is outside 1..={MAX_DIMENSION}")]
    Size(i32, i32),
    #[error("the refresh rate cannot be zero")]
    Refresh,
    #[error("scale {0}/120 is outside {MIN_SCALE_120}..={MAX_SCALE_120}")]
    Scale(u32),
}

impl VirtualMode {
    pub fn new(
        width: i32,
        height: i32,
        refresh_mhz: u32,
        scale_120: u32,
    ) -> Result<Self, ModeError> {
        let dimension = 1..=MAX_DIMENSION;
        if !dimension.contains(&width) || !dimension.contains(&height) {
            return Err(ModeError::Size(width, height));
        }
        if refresh_mhz == 0 {
            return Err(ModeError::Refresh);
        }
        if !(MIN_SCALE_120..=MAX_SCALE_120).contains(&scale_120) {
            return Err(ModeError::Scale(scale_120));
        }
        Ok(Self {
            size: Size::from((width, height)),
            refresh_mhz,
            scale_120,
        })
    }

    pub fn scale(&self) -> f64 {
        f64::from(self.scale_120) / f64::from(SCALE_DENOMINATOR)
    }

    /// Millihertz as smithay's `Mode::refresh` counts it.
    pub fn refresh(&self) -> i32 {
        i32::try_from(self.refresh_mhz).unwrap_or(i32::MAX)
    }
}

/// 1 to 32 characters of `[A-Za-z0-9_-]`.
pub fn is_valid_name(name: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// The output's full name: what `created` reports, `wl_output.name` says and
/// output management keys the head on.
pub fn full_name(name: &str) -> String {
    format!("{NAME_PREFIX}{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_and_plain() {
        assert!(is_valid_name("tablet"));
        assert!(is_valid_name("Pixel_Tab-2"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("has space"));
        assert!(!is_valid_name("slash/no"));
        assert!(!is_valid_name("ünicode"));
        assert!(!is_valid_name(&"x".repeat(MAX_NAME_LEN + 1)));
        assert!(is_valid_name(&"x".repeat(MAX_NAME_LEN)));
    }

    #[test]
    fn the_full_name_is_namespaced() {
        assert_eq!(full_name("tablet"), "VIRTUAL-tablet");
    }

    #[test]
    fn modes_are_range_checked() {
        let mode = VirtualMode::new(2560, 1600, 60_000, 180).expect("a valid mode");
        assert_eq!(mode.scale(), 1.5);
        assert_eq!(mode.refresh(), 60_000);

        assert_eq!(
            VirtualMode::new(0, 1080, 60_000, 120),
            Err(ModeError::Size(0, 1080))
        );
        assert_eq!(
            VirtualMode::new(1920, MAX_DIMENSION + 1, 60_000, 120),
            Err(ModeError::Size(1920, MAX_DIMENSION + 1))
        );
        assert_eq!(
            VirtualMode::new(1920, 1080, 0, 120),
            Err(ModeError::Refresh)
        );
        assert_eq!(
            VirtualMode::new(1920, 1080, 60_000, 59),
            Err(ModeError::Scale(59))
        );
        assert_eq!(
            VirtualMode::new(1920, 1080, 60_000, 481),
            Err(ModeError::Scale(481))
        );
    }

    #[test]
    fn a_refresh_beyond_i32_saturates() {
        let mode = VirtualMode::new(1, 1, u32::MAX, 120).expect("a valid mode");
        assert_eq!(mode.refresh(), i32::MAX);
    }
}
