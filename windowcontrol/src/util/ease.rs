/// `t` clamped to 0..1, eased in and out.
pub fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_holds_still_past_either_end() {
        assert_eq!(smoothstep(-1.0), 0.0);
        assert_eq!(smoothstep(0.5), 0.5);
        assert_eq!(smoothstep(2.0), 1.0);
    }
}
