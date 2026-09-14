//! CTA-861 extension blocks: HDR capability and wide-gamut colorimetry.
//!
//! An EDID extension is another 128 bytes whose first byte is a tag; `0x02` is
//! CTA-861. Byte 2 is where the detailed timings start, so bytes 4 up to that
//! offset are a collection of data blocks, each `<3-bit tag><5-bit length>`
//! followed by its payload. Everything this compositor wants lives behind tag
//! 7 ("extended"), which spends a further byte on an extended tag code.

use std::ops::BitOr;

const CTA_TAG: u8 = 0x02;
const EXTENDED_TAG: u8 = 7;
const COLORIMETRY: u8 = 5;
const HDR_STATIC_METADATA: u8 = 6;

/// Which electro-optical transfer functions the display says it can accept.
///
/// A display that does not claim ST 2084 must not be driven with a PQ
/// `HDR_OUTPUT_METADATA` infoframe, so this is the gate on HDR output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EotfSupport {
    pub traditional_sdr: bool,
    pub traditional_hdr: bool,
    pub st2084_pq: bool,
    pub hlg: bool,
}

impl EotfSupport {
    fn from_bits(bits: u8) -> Self {
        Self {
            traditional_sdr: bits & 0x01 != 0,
            traditional_hdr: bits & 0x02 != 0,
            st2084_pq: bits & 0x04 != 0,
            hlg: bits & 0x08 != 0,
        }
    }

    /// Whether driving this display in BT.2100 is meaningful at all.
    pub fn supports_hdr(&self) -> bool {
        self.st2084_pq || self.hlg
    }
}

/// The HDR Static Metadata Data Block, CTA-861.3 §4.2.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HdrStaticMetadata {
    pub eotf: EotfSupport,
    /// cd/m². The three luminances are optional even when the block is present.
    pub max_luminance: Option<f32>,
    pub max_frame_average_luminance: Option<f32>,
    pub min_luminance: Option<f32>,
}

/// The Colorimetry Data Block's wide-gamut bits, CTA-861 §7.5.5.
///
/// `BT2020_RGB` is the licence to set the `Colorspace` connector property; the
/// YCC variants are only meaningful for YUV output, which this compositor does
/// not drive, but they are cheap to carry and useful in logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Colorimetry {
    pub bt2020_rgb: bool,
    pub bt2020_ycc: bool,
    pub bt2020_cycc: bool,
    pub adobe_rgb: bool,
    pub dci_p3: bool,
}

impl BitOr for Colorimetry {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            bt2020_rgb: self.bt2020_rgb | other.bt2020_rgb,
            bt2020_ycc: self.bt2020_ycc | other.bt2020_ycc,
            bt2020_cycc: self.bt2020_cycc | other.bt2020_cycc,
            adobe_rgb: self.adobe_rgb | other.adobe_rgb,
            dci_p3: self.dci_p3 | other.dci_p3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CtaBlock {
    pub hdr: Option<HdrStaticMetadata>,
    pub colorimetry: Colorimetry,
}

/// Reads one 128-byte extension. Returns `None` for extensions that are not
/// CTA-861, or that carry no data block collection.
pub fn parse(block: &[u8; 128]) -> Option<CtaBlock> {
    if block[0] != CTA_TAG {
        return None;
    }

    // Byte 2 is the offset of the first detailed timing. 0 means "no timings
    // and no data blocks"; anything below 4 would overlap the header itself.
    let timings_at = block[2] as usize;
    if timings_at < 4 || timings_at > block.len() {
        return None;
    }

    let mut parsed = CtaBlock::default();
    let mut cursor = 4;

    while cursor < timings_at {
        let length = (block[cursor] & 0x1F) as usize;
        let tag = block[cursor] >> 5;
        let payload_at = cursor + 1;
        let next = payload_at + length;
        // A length running past the collection means the block is malformed;
        // stop rather than reinterpreting timing bytes as data blocks.
        if next > timings_at {
            break;
        }

        if tag == EXTENDED_TAG && length >= 1 {
            let payload = &block[payload_at + 1..next];
            match block[payload_at] {
                COLORIMETRY => parsed.colorimetry = parsed.colorimetry | colorimetry(payload),
                HDR_STATIC_METADATA => parsed.hdr = hdr_static_metadata(payload),
                _ => {}
            }
        }

        cursor = next;
    }

    Some(parsed)
}

fn colorimetry(payload: &[u8]) -> Colorimetry {
    let Some(&supported) = payload.first() else {
        return Colorimetry::default();
    };
    // The second payload byte's high nibble carries the DCI-P3 bit in
    // CTA-861.3 and later; absent in older revisions, hence the `and_then`.
    let extended = payload.get(1).copied().unwrap_or(0);

    Colorimetry {
        bt2020_cycc: supported & 0x20 != 0,
        bt2020_ycc: supported & 0x40 != 0,
        bt2020_rgb: supported & 0x80 != 0,
        adobe_rgb: supported & 0x10 != 0,
        dci_p3: extended & 0x80 != 0,
    }
}

fn hdr_static_metadata(payload: &[u8]) -> Option<HdrStaticMetadata> {
    let &eotf = payload.first()?;
    // The three luminance bytes are appended in order and any suffix may be
    // absent, so each is read independently rather than gating on a length.
    Some(HdrStaticMetadata {
        eotf: EotfSupport::from_bits(eotf),
        max_luminance: payload.get(2).copied().map(decode_luminance),
        max_frame_average_luminance: payload.get(3).copied().map(decode_luminance),
        min_luminance: payload
            .get(4)
            .copied()
            .zip(payload.get(2).copied())
            .map(|(min, max)| decode_min_luminance(min, decode_luminance(max))),
    })
}

/// CTA-861.3: `50 · 2^(code / 32)` cd/m².
fn decode_luminance(code: u8) -> f32 {
    50.0 * (code as f32 / 32.0).exp2()
}

/// CTA-861.3: `max · (code / 255)² / 100` cd/m².
fn decode_min_luminance(code: u8, max: f32) -> f32 {
    let fraction = code as f32 / 255.0;
    max * fraction * fraction / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CTA block whose collection holds `blocks`, laid out the way a real
    /// extension is: header, then the collection, then timings.
    fn extension(blocks: &[&[u8]]) -> [u8; 128] {
        let mut block = [0u8; 128];
        block[0] = CTA_TAG;
        block[1] = 3;

        let mut cursor = 4;
        for payload in blocks {
            block[cursor..cursor + payload.len()].copy_from_slice(payload);
            cursor += payload.len();
        }
        block[2] = cursor as u8;
        block
    }

    #[test]
    fn a_non_cta_extension_is_ignored() {
        let mut block = [0u8; 128];
        block[0] = 0xF0;
        assert_eq!(parse(&block), None);
    }

    #[test]
    fn hdr_static_metadata_decodes_eotfs_and_luminances() {
        // tag 7, length 5: extended tag 6, eotf bits, sm bits, max, avg, min.
        let parsed = parse(&extension(&[&[0xE6, HDR_STATIC_METADATA, 0x06, 0x01, 0xBE, 0xAF, 0x08]]))
            .expect("a CTA block");
        let hdr = parsed.hdr.expect("an HDR block");

        assert!(hdr.eotf.traditional_hdr);
        assert!(hdr.eotf.st2084_pq);
        assert!(!hdr.eotf.hlg);
        assert!(hdr.eotf.supports_hdr());

        let max = hdr.max_luminance.expect("a max luminance");
        assert!((max - 50.0 * (190.0f32 / 32.0).exp2()).abs() < 0.01);
        assert!(hdr.min_luminance.is_some_and(|min| min > 0.0 && min < max));
    }

    #[test]
    fn colorimetry_reports_bt2020_rgb() {
        let parsed =
            parse(&extension(&[&[0xE3, COLORIMETRY, 0xC0, 0x00]])).expect("a CTA block");
        assert!(parsed.colorimetry.bt2020_rgb);
        assert!(parsed.colorimetry.bt2020_ycc);
        assert!(!parsed.colorimetry.bt2020_cycc);
    }

    #[test]
    fn both_blocks_are_read_from_one_collection() {
        let parsed = parse(&extension(&[
            &[0xE3, COLORIMETRY, 0xC0, 0x00],
            &[0xE3, HDR_STATIC_METADATA, 0x0C, 0x01],
        ]))
        .expect("a CTA block");

        assert!(parsed.colorimetry.bt2020_rgb);
        assert!(parsed.hdr.is_some_and(|hdr| hdr.eotf.hlg));
    }

    #[test]
    fn a_data_block_running_past_the_collection_stops_the_walk() {
        let mut block = [0u8; 128];
        block[0] = CTA_TAG;
        block[2] = 8;
        // Claims 31 bytes of payload inside an 8-byte collection.
        block[4] = 0xFF;
        assert_eq!(parse(&block), Some(CtaBlock::default()));
    }

    #[test]
    fn a_collection_that_overlaps_the_header_is_refused() {
        let mut block = [0u8; 128];
        block[0] = CTA_TAG;
        block[2] = 2;
        assert_eq!(parse(&block), None);
    }
}
