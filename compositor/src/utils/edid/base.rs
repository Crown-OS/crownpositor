//! The 128-byte EDID base block.
//!
//! Layout is E-EDID 1.4 §3. Everything here reads from a `&[u8; 128]` that the
//! caller has already length- and checksum-checked, so the offsets below cannot
//! panic and the parser has no error path of its own — a field that is absent
//! or nonsensical comes back as `None`.

use super::vendors;

/// The four CIE 1931 xy pairs from bytes 25..35.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chromaticity {
    pub red: (f32, f32),
    pub green: (f32, f32),
    pub blue: (f32, f32),
    pub white: (f32, f32),
}

impl Chromaticity {
    /// Whether these primaries describe a display that could exist.
    ///
    /// A surprising number of panels report zeroes, duplicated points or a
    /// white point nowhere near the daylight locus. Colour management derives
    /// an RGB→XYZ matrix from this, and a degenerate triangle inverts to
    /// garbage, so the caller drops the whole set rather than propagating it.
    pub fn is_plausible(&self) -> bool {
        let area = {
            let (rx, ry) = self.red;
            let (gx, gy) = self.green;
            let (bx, by) = self.blue;
            ((gx - rx) * (by - ry) - (bx - rx) * (gy - ry)).abs()
        };
        // sRGB's triangle has an area of ~0.1124; a tenth of that is still a
        // far narrower gamut than any real display and safely invertible.
        if area < 0.01 {
            return false;
        }

        let (wx, wy) = self.white;
        if !(0.0..=1.0).contains(&wx) || !(0.0..=1.0).contains(&wy) {
            return false;
        }

        // CIE daylight locus, valid for 4000K..25000K. Anything far off it is
        // a misreported white point rather than an exotic display.
        let locus_y = -3.0 * wx * wx + 2.87 * wx - 0.275;
        (wy - locus_y).abs() <= 0.05
    }
}

/// A display descriptor's monitor range limits, byte 5 onwards of a `0xFD` tag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeLimits {
    pub min_vertical_hz: u16,
    pub max_vertical_hz: u16,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BaseBlock {
    pub manufacturer: String,
    pub product_code: u16,
    pub numeric_serial: u32,
    pub monitor_name: Option<String>,
    pub serial_string: Option<String>,
    pub physical_size_mm: Option<(u16, u16)>,
    pub gamma: Option<f32>,
    pub chromaticity: Option<Chromaticity>,
    pub range_limits: Option<RangeLimits>,
    pub extension_count: u8,
}

const MAGIC: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// Where the four 18-byte descriptors start.
const DESCRIPTORS: [usize; 4] = [54, 72, 90, 108];
const DESCRIPTOR_LEN: usize = 18;

pub fn parse(block: &[u8; 128]) -> Option<BaseBlock> {
    if block[..8] != MAGIC {
        return None;
    }

    let mut parsed = BaseBlock {
        manufacturer: manufacturer(u16::from_be_bytes([block[8], block[9]])),
        product_code: u16::from_le_bytes([block[10], block[11]]),
        numeric_serial: u32::from_le_bytes([block[12], block[13], block[14], block[15]]),
        gamma: (block[23] != 0xFF).then(|| (block[23] as f32 + 100.0) / 100.0),
        chromaticity: chromaticity(block),
        extension_count: block[126],
        ..BaseBlock::default()
    };

    // Bytes 21/22 are whole centimetres, so they are only the fallback; a
    // detailed timing descriptor carries millimetres.
    if block[21] != 0 && block[22] != 0 {
        parsed.physical_size_mm = Some((block[21] as u16 * 10, block[22] as u16 * 10));
    }

    for start in DESCRIPTORS {
        let descriptor = &block[start..start + DESCRIPTOR_LEN];
        match descriptor_tag(descriptor) {
            Some(0xFC) => parsed.monitor_name = descriptor_text(descriptor),
            Some(0xFF) => parsed.serial_string = descriptor_text(descriptor),
            Some(0xFD) => parsed.range_limits = range_limits(descriptor),
            // Not a display descriptor, so it is a detailed timing, whose
            // millimetre image size beats the base block's centimetres.
            None => {
                if let Some(size) = timing_physical_size(descriptor) {
                    parsed.physical_size_mm = Some(size);
                }
            }
            _ => {}
        }
    }

    Some(parsed)
}

/// Three 5-bit letters packed big-endian, `A` == 1, top bit reserved.
fn manufacturer(packed: u16) -> String {
    let letters = [
        ((packed >> 10) & 0x1F) as u8,
        ((packed >> 5) & 0x1F) as u8,
        (packed & 0x1F) as u8,
    ];
    if letters.iter().any(|letter| !(1..=26).contains(letter)) {
        return String::new();
    }

    let code = letters.map(|letter| b'A' + letter - 1);
    vendors::name_for(&code).map_or_else(
        || String::from_utf8_lossy(&code).into_owned(),
        str::to_owned,
    )
}

/// Ten bytes of 10-bit fixed point, the low two bits of every coordinate packed
/// into the first two bytes.
fn chromaticity(block: &[u8; 128]) -> Option<Chromaticity> {
    let low = |byte: u8, shift: u8| ((byte >> shift) & 0x03) as u16;
    let coordinate = |high: u8, low_bits: u16| (((high as u16) << 2) | low_bits) as f32 / 1024.0;

    let parsed = Chromaticity {
        red: (
            coordinate(block[27], low(block[25], 6)),
            coordinate(block[28], low(block[25], 4)),
        ),
        green: (
            coordinate(block[29], low(block[25], 2)),
            coordinate(block[30], low(block[25], 0)),
        ),
        blue: (
            coordinate(block[31], low(block[26], 6)),
            coordinate(block[32], low(block[26], 4)),
        ),
        white: (
            coordinate(block[33], low(block[26], 2)),
            coordinate(block[34], low(block[26], 0)),
        ),
    };

    parsed.is_plausible().then_some(parsed)
}

/// A display descriptor is `00 00 00 <tag> 00`; anything else is a timing.
fn descriptor_tag(descriptor: &[u8]) -> Option<u8> {
    (descriptor[0] == 0 && descriptor[1] == 0 && descriptor[2] == 0 && descriptor[4] == 0)
        .then(|| descriptor[3])
}

/// Thirteen bytes, terminated by `0x0A` and padded with spaces.
///
/// Monitors put arbitrary bytes here, so anything outside printable ASCII is
/// dropped rather than carried into a log line or a config file.
fn descriptor_text(descriptor: &[u8]) -> Option<String> {
    let payload = &descriptor[5..DESCRIPTOR_LEN];
    let end = payload.iter().position(|byte| *byte == 0x0A).unwrap_or(payload.len());
    let text: String = payload[..end]
        .iter()
        .copied()
        .filter(|byte| (0x20..0x7F).contains(byte))
        .map(char::from)
        .collect();

    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn range_limits(descriptor: &[u8]) -> Option<RangeLimits> {
    // EDID 1.4 byte 4 says whether 255 has to be added back to the rates that
    // would otherwise not fit in a byte.
    let offset = if descriptor[4] & 0x03 == 0x03 { 255u16 } else { 0 };
    let min = descriptor[5] as u16 + offset;
    let max = descriptor[6] as u16 + offset;

    (min > 0 && max >= min).then_some(RangeLimits {
        min_vertical_hz: min,
        max_vertical_hz: max,
    })
}

/// Bytes 12..15 of a detailed timing: two low bytes plus a shared high nibble.
fn timing_physical_size(descriptor: &[u8]) -> Option<(u16, u16)> {
    let width = descriptor[12] as u16 | ((descriptor[14] as u16 & 0xF0) << 4);
    let height = descriptor[13] as u16 | ((descriptor[14] as u16 & 0x0F) << 8);
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_manufacturer_becomes_a_registered_name() {
        // "DEL" == 0b00100 00101 01100
        assert_eq!(manufacturer(0b0_00100_00101_01100), "Dell Inc.");
    }

    #[test]
    fn an_unregistered_manufacturer_keeps_its_raw_code() {
        // "ZZZ"
        assert_eq!(manufacturer(0b0_11010_11010_11010), "ZZZ");
    }

    #[test]
    fn a_manufacturer_with_a_letter_out_of_range_is_dropped() {
        assert!(manufacturer(0).is_empty());
    }

    #[test]
    fn descriptor_text_stops_at_the_terminator_and_drops_junk() {
        let mut descriptor = [0x20u8; DESCRIPTOR_LEN];
        descriptor[3] = 0xFC;
        descriptor[5..12].copy_from_slice(b"U2720Q\x0A");
        descriptor[12] = 0x01;
        assert_eq!(descriptor_text(&descriptor).as_deref(), Some("U2720Q"));
    }

    #[test]
    fn an_empty_descriptor_string_is_none() {
        let mut descriptor = [0x20u8; DESCRIPTOR_LEN];
        descriptor[3] = 0xFC;
        assert_eq!(descriptor_text(&descriptor), None);
    }

    #[test]
    fn srgb_primaries_are_plausible_and_zeroes_are_not() {
        let srgb = Chromaticity {
            red: (0.64, 0.33),
            green: (0.30, 0.60),
            blue: (0.15, 0.06),
            white: (0.3127, 0.3290),
        };
        assert!(srgb.is_plausible());

        let degenerate = Chromaticity {
            red: (0.0, 0.0),
            green: (0.0, 0.0),
            blue: (0.0, 0.0),
            white: (0.0, 0.0),
        };
        assert!(!degenerate.is_plausible());
    }

    #[test]
    fn a_white_point_off_the_daylight_locus_is_rejected() {
        let wrong_white = Chromaticity {
            red: (0.64, 0.33),
            green: (0.30, 0.60),
            blue: (0.15, 0.06),
            white: (0.1, 0.8),
        };
        assert!(!wrong_white.is_plausible());
    }

    #[test]
    fn range_limits_apply_the_edid_1_4_offset() {
        let mut descriptor = [0u8; DESCRIPTOR_LEN];
        descriptor[3] = 0xFD;
        descriptor[4] = 0x03;
        descriptor[5] = 1;
        descriptor[6] = 105;
        let limits = range_limits(&descriptor).expect("a valid range");
        assert_eq!(limits.min_vertical_hz, 256);
        assert_eq!(limits.max_vertical_hz, 360);
    }
}
