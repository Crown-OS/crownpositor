//! The colour state of one output: what the CRTC's gamma ramp does to it,
//! the depth it is scanned out at, and what its display can take.
//!
//! On SDR the night light and the dim are a gamma ramp after encoding, so
//! they change no description. The HDR follow-up changes
//! [`OutputColor::description`] and moves both into the shader pass.

use protocols::{color_management::record::DescriptionKind, gamma_control::GammaRamps};
use smithay::backend::allocator::Fourcc;

use super::{night_light, pipeline::output_encoding};

/// Bits per colour channel in the buffers an output scans out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanoutDepth {
    #[default]
    Eight,
    Ten,
}

impl ScanoutDepth {
    pub fn of_format(format: Fourcc) -> Self {
        match format {
            Fourcc::Abgr2101010
            | Fourcc::Xbgr2101010
            | Fourcc::Argb2101010
            | Fourcc::Xrgb2101010 => Self::Ten,
            _ => Self::Eight,
        }
    }

    pub const fn bits(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Ten => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputColor {
    /// Night light, from 0.0 (neutral) to 1.0 (warmest).
    pub warmth: f64,
    /// The share of full brightness the output keeps; 1.0 is not dimmed.
    pub dim: f64,
    pub depth: ScanoutDepth,
    /// The EDID claims a PQ or HLG transfer function.
    pub hdr_capable: bool,
}

impl OutputColor {
    pub fn new(hdr_capable: bool) -> Self {
        Self {
            warmth: 0.0,
            dim: 1.0,
            depth: ScanoutDepth::default(),
            hdr_capable,
        }
    }

    /// What the output scans out, and so what its surfaces should render
    /// into.
    pub fn description(&self) -> DescriptionKind {
        output_encoding()
    }

    /// Whether clients watching the output must hear of a new description.
    pub fn describes_differently(&self, other: &Self) -> bool {
        self.description() != other.description()
    }

    /// The gamma ramps of `size` entries for this output, or `None` when it
    /// asks for nothing and a gamma client may keep the output.
    pub fn ramps(&self, size: u32) -> Option<GammaRamps> {
        night_light::ramps(size, self.warmth, self.dim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_bit_formats_scan_out_at_ten_bits() {
        assert_eq!(ScanoutDepth::of_format(Fourcc::Xbgr2101010).bits(), 10);
        assert_eq!(ScanoutDepth::of_format(Fourcc::Abgr2101010).bits(), 10);
        assert_eq!(ScanoutDepth::of_format(Fourcc::Abgr8888).bits(), 8);
        assert_eq!(ScanoutDepth::of_format(Fourcc::Argb8888).bits(), 8);
    }

    #[test]
    fn night_light_and_dim_change_no_sdr_description() {
        let neutral = OutputColor::new(false);
        let evening = OutputColor {
            warmth: 0.6,
            dim: 0.5,
            depth: ScanoutDepth::Ten,
            ..neutral
        };
        assert!(!evening.describes_differently(&neutral));
    }

    #[test]
    fn a_neutral_output_asks_for_no_ramp() {
        assert!(OutputColor::new(true).ramps(256).is_none());
    }
}
