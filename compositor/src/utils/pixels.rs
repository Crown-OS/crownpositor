/// Copies `[r, g, b, a]` pixels into `[b, g, r, a]` order, or back: the
/// difference between what GL reads out (`Abgr8888`) and what `wl_shm` and
/// DRM `Argb8888` lay out in memory.
pub fn copy_swapping_red_blue(dst: &mut [u8], src: &[u8]) {
    for (dst, src) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_and_blue_trade_places() {
        let mut dst = [0; 8];
        copy_swapping_red_blue(&mut dst, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(dst, [3, 2, 1, 4, 7, 6, 5, 8]);
    }
}
