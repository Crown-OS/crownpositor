//! EDID parsing, vendored.
//!
//! `smithay-drm-extras` would do this through `libdisplay-info`, but that
//! crate's `-sys` shim rejects the 0.3 series this machine ships, so the
//! workspace builds it with `default-features = false` and gets no display
//! information at all. The subset a compositor actually needs is small and
//! entirely self-contained, so it lives here instead of behind a system
//! library: identity for config matching, physical size for a scale guess,
//! and the colorimetry and HDR capability that colour management needs.
//!
//! The bytes come from a monitor over a cable, so every field is treated as
//! hostile: blocks are checksummed, the extension count is bounded by the
//! actual blob length, and no offset is read without a length check.

mod base;
mod cta;
mod vendors;

#[allow(unused_imports)]
pub use base::{Chromaticity, RangeLimits};
#[allow(unused_imports)]
pub use cta::{Colorimetry, EotfSupport, HdrStaticMetadata};

const BLOCK_LEN: usize = 128;

/// Everything this compositor reads out of an EDID blob.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EdidInfo {
    /// The registered vendor name, or the raw PnP code when unregistered.
    pub make: String,
    /// The `0xFC` monitor-name descriptor, else the hex product code.
    pub model: String,
    /// The `0xFF` serial descriptor, else the numeric serial, else `None`.
    ///
    /// `None` rather than a placeholder, because `zwlr_output_head_v1` must
    /// simply not send the event when there is nothing to say.
    pub serial: Option<String>,
    pub physical_size_mm: Option<(u16, u16)>,
    pub gamma: Option<f32>,
    /// Absent when the panel reported something that could not be a display.
    pub chromaticity: Option<Chromaticity>,
    /// The VRR floor: refresh may not be stretched below this.
    pub min_vertical_hz: Option<u16>,
    pub hdr: Option<HdrStaticMetadata>,
    pub colorimetry: Colorimetry,
}

impl EdidInfo {
    /// Whether this display can be driven in BT.2100.
    pub fn supports_hdr(&self) -> bool {
        self.hdr.is_some_and(|hdr| hdr.eotf.supports_hdr())
    }

    /// `"MAKE MODEL SERIAL"`, the identity a config entry may be keyed on.
    pub fn identity(&self) -> String {
        match self.serial.as_deref() {
            Some(serial) => format!("{} {} {}", self.make, self.model, serial),
            None => format!("{} {}", self.make, self.model),
        }
    }
}

/// Parses an EDID blob. `None` when it is not one.
///
/// Extension blocks that fail their checksum are skipped rather than failing
/// the whole blob: a bad CTA block costs HDR, not the monitor's name.
pub fn parse(bytes: &[u8]) -> Option<EdidInfo> {
    let first: &[u8; BLOCK_LEN] = bytes.get(..BLOCK_LEN)?.try_into().ok()?;
    if !checksum_ok(first) {
        return None;
    }

    let base = base::parse(first)?;

    let mut info = EdidInfo {
        model: base
            .monitor_name
            .clone()
            .unwrap_or_else(|| format!("{:04X}", base.product_code)),
        serial: base.serial_string.clone().or_else(|| {
            (base.numeric_serial != 0).then(|| base.numeric_serial.to_string())
        }),
        make: base.manufacturer,
        physical_size_mm: base.physical_size_mm,
        gamma: base.gamma,
        chromaticity: base.chromaticity,
        min_vertical_hz: base.range_limits.map(|limits| limits.min_vertical_hz),
        ..EdidInfo::default()
    };

    // The header's count is what the monitor claims; the blob is what it
    // actually sent. Trust whichever is smaller.
    let available = bytes.len() / BLOCK_LEN - 1;
    for index in 0..available.min(base.extension_count as usize) {
        let start = (index + 1) * BLOCK_LEN;
        let Some(block) = bytes.get(start..start + BLOCK_LEN) else {
            break;
        };
        let Ok(block) = <&[u8; BLOCK_LEN]>::try_from(block) else {
            break;
        };
        if !checksum_ok(block) {
            continue;
        }
        if let Some(extension) = cta::parse(block) {
            info.colorimetry = info.colorimetry | extension.colorimetry;
            info.hdr = extension.hdr.or(info.hdr);
        }
    }

    Some(info)
}

/// Every block's 128 bytes sum to zero mod 256.
fn checksum_ok(block: &[u8; BLOCK_LEN]) -> bool {
    block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A base block with a valid header, checksum and the fields the tests
    /// care about; `extensions` is appended and counted.
    fn edid(extensions: &[[u8; BLOCK_LEN]]) -> Vec<u8> {
        let mut base = [0u8; BLOCK_LEN];
        base[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        // "DEL"
        base[8..10].copy_from_slice(&0b0_00100_00101_01100u16.to_be_bytes());
        base[10..12].copy_from_slice(&0x2720u16.to_le_bytes());
        base[12..16].copy_from_slice(&0x0001_E240u32.to_le_bytes());
        base[21] = 60;
        base[22] = 34;
        base[23] = 120;

        // sRGB-ish chromaticity, so it survives the plausibility check.
        base[25] = 0b01_01_10_10;
        base[26] = 0b01_11_01_11;
        base[27..35].copy_from_slice(&[0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54]);

        // A 0xFC monitor-name descriptor.
        base[54 + 3] = 0xFC;
        base[59..66].copy_from_slice(b"U2720Q\x0A");
        for byte in &mut base[66..72] {
            *byte = 0x20;
        }

        base[126] = extensions.len() as u8;
        base[127] = checksum_for(&base);

        let mut blob = base.to_vec();
        for extension in extensions {
            blob.extend_from_slice(extension);
        }
        blob
    }

    fn checksum_for(block: &[u8; BLOCK_LEN]) -> u8 {
        block[..BLOCK_LEN - 1]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
            .wrapping_neg()
    }

    fn hdr_extension() -> [u8; BLOCK_LEN] {
        let mut block = [0u8; BLOCK_LEN];
        block[0] = 0x02;
        block[1] = 3;
        block[2] = 12;
        block[4..8].copy_from_slice(&[0xE3, 5, 0xC0, 0x00]);
        block[8..12].copy_from_slice(&[0xE3, 6, 0x0C, 0x01]);
        block[127] = checksum_for(&block);
        block
    }

    #[test]
    fn a_base_block_yields_identity_and_physical_size() {
        let info = parse(&edid(&[])).expect("a valid EDID");

        assert_eq!(info.make, "Dell Inc.");
        assert_eq!(info.model, "U2720Q");
        assert_eq!(info.serial.as_deref(), Some("123456"));
        assert_eq!(info.identity(), "Dell Inc. U2720Q 123456");
        assert_eq!(info.physical_size_mm, Some((600, 340)));
        assert!(info.gamma.is_some_and(|gamma| (gamma - 2.2).abs() < 0.01));
        assert!(info.chromaticity.is_some());
        assert!(!info.supports_hdr());
    }

    #[test]
    fn a_cta_extension_adds_hdr_and_colorimetry() {
        let info = parse(&edid(&[hdr_extension()])).expect("a valid EDID");

        assert!(info.supports_hdr());
        assert!(info.colorimetry.bt2020_rgb);
    }

    #[test]
    fn a_bad_base_checksum_rejects_the_blob() {
        let mut blob = edid(&[]);
        blob[127] = blob[127].wrapping_add(1);
        assert_eq!(parse(&blob), None);
    }

    #[test]
    fn a_bad_extension_checksum_costs_only_the_extension() {
        let mut extension = hdr_extension();
        extension[127] = extension[127].wrapping_add(1);

        let info = parse(&edid(&[extension])).expect("the base block still parses");
        assert_eq!(info.model, "U2720Q");
        assert!(!info.supports_hdr());
    }

    #[test]
    fn a_claimed_extension_that_is_not_there_is_not_read() {
        let mut blob = edid(&[]);
        blob[126] = 4;
        blob[127] = checksum_for(blob[..BLOCK_LEN].try_into().expect("a full block"));

        let info = parse(&blob).expect("a valid EDID");
        assert_eq!(info.model, "U2720Q");
    }

    #[test]
    fn a_truncated_blob_is_refused() {
        assert_eq!(parse(&[0u8; 64]), None);
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn a_blob_without_the_header_magic_is_refused() {
        let mut blob = edid(&[]);
        blob[1] = 0;
        blob[127] = checksum_for(blob[..BLOCK_LEN].try_into().expect("a full block"));
        assert_eq!(parse(&blob), None);
    }

    #[test]
    fn a_monitor_with_no_name_falls_back_to_its_product_code() {
        let mut blob = edid(&[]);
        blob[54 + 3] = 0xFE;
        blob[127] = checksum_for(blob[..BLOCK_LEN].try_into().expect("a full block"));

        let info = parse(&blob).expect("a valid EDID");
        assert_eq!(info.model, "2720");
    }
}
