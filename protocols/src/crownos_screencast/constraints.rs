//! What a session's buffers have to look like.

use smithay::{
    backend::allocator::{Buffer, Fourcc, Modifier, dmabuf::Dmabuf},
    utils::{Buffer as BufferCoords, Size},
};

/// One pixel format and the layouts the compositor can render it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatModifiers {
    pub format: Fourcc,
    pub modifiers: Vec<Modifier>,
}

/// The size, formats and modifiers of the buffers a session renders into, as
/// one `constraints` … `constraints_done` sequence advertises them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferConstraints {
    pub size: Size<i32, BufferCoords>,
    pub formats: Vec<FormatModifiers>,
}

impl BufferConstraints {
    /// Whether a client buffer satisfies these constraints and may be
    /// attached.
    pub fn admits(&self, dmabuf: &Dmabuf) -> bool {
        let format = dmabuf.format();
        dmabuf.size() == self.size
            && self.formats.iter().any(|candidate| {
                candidate.format == format.code && candidate.modifiers.contains(&format.modifier)
            })
    }
}

/// Splits a modifier into the two halves the protocol carries.
pub fn modifier_halves(modifier: Modifier) -> (u32, u32) {
    let raw = u64::from(modifier);
    ((raw >> 32) as u32, raw as u32)
}

/// Joins the two halves of a 64-bit protocol value.
pub fn join_halves(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_trip() {
        let modifier = Modifier::from(0x0200_0000_1234_5678_u64);
        let (hi, lo) = modifier_halves(modifier);
        assert_eq!((hi, lo), (0x0200_0000, 0x1234_5678));
        assert_eq!(join_halves(hi, lo), u64::from(modifier));
    }

    #[test]
    fn linear_is_zero_and_invalid_is_the_reserved_code() {
        assert_eq!(modifier_halves(Modifier::Linear), (0, 0));
        assert_eq!(modifier_halves(Modifier::Invalid), (0x00ff_ffff, u32::MAX));
    }
}
