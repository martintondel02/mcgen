//! Scalar helpers shared by the noise and density code.
//!
//! These mirror Minecraft's `Mth` helpers. All math is done in `f64` to match
//! vanilla's `ImprovedNoise`/`NormalNoise`, which are `double` throughout.

/// `2^25` — the wrap-around period used by vanilla's coordinate folding.
pub const ROUND_OFF: f64 = 33_554_432.0;

/// Slightly-less-than `2^24`. Written as a bit pattern to match vanilla exactly
/// (it is *not* exactly `16777216.0`).
pub const HALF_ROUND_OFF: f64 = f64::from_bits(0x416F_FFFF_FFFF_FFFF);

/// Vanilla's `PerlinNoise` offsets are drawn in `[0, 256)`.
pub const NOISE_OFFSET_SCALE: f64 = 256.0;

/// Fold an unbounded coordinate into the range vanilla wraps at.
#[inline]
#[must_use]
pub fn wrap(x: f64) -> f64 {
    if (-HALF_ROUND_OFF..HALF_ROUND_OFF).contains(&x) {
        x
    } else {
        x - (x / ROUND_OFF + 0.5).floor() * ROUND_OFF
    }
}

/// Vanilla's `Mth.smoothstep`.
#[inline]
#[must_use]
pub fn smoothstep(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
#[must_use]
pub fn lerp(alpha: f64, a: f64, b: f64) -> f64 {
    a + alpha * (b - a)
}

#[inline]
#[must_use]
pub fn lerp2(alpha_x: f64, alpha_y: f64, d00: f64, d10: f64, d01: f64, d11: f64) -> f64 {
    lerp(alpha_y, lerp(alpha_x, d00, d10), lerp(alpha_x, d01, d11))
}

/// Trilinear interpolation over the 8 cube corners, in vanilla's argument order.
#[inline]
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn lerp3(
    alpha_x: f64,
    alpha_y: f64,
    alpha_z: f64,
    d000: f64,
    d100: f64,
    d010: f64,
    d110: f64,
    d001: f64,
    d101: f64,
    d011: f64,
    d111: f64,
) -> f64 {
    lerp(
        alpha_z,
        lerp2(alpha_x, alpha_y, d000, d100, d010, d110),
        lerp2(alpha_x, alpha_y, d001, d101, d011, d111),
    )
}

/// Vanilla's 16-entry gradient table used by `ImprovedNoise.gradDot`.
pub const GRADIENTS: [[f64; 3]; 16] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
    [1.0, 1.0, 0.0],
    [0.0, -1.0, 1.0],
    [-1.0, 1.0, 0.0],
    [0.0, -1.0, -1.0],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_is_identity_inside_the_band() {
        for x in [-1000.0, -1.0, 0.0, 0.5, 1000.0] {
            assert_eq!(wrap(x), x, "wrap({x}) should be identity");
        }
    }

    #[test]
    fn wrap_folds_large_coordinates() {
        let x = 1.0e9;
        let w = wrap(x);
        assert!((-HALF_ROUND_OFF..HALF_ROUND_OFF).contains(&w), "got {w}");
    }

    #[test]
    fn smoothstep_matches_known_values() {
        assert!((smoothstep(0.0) - 0.0).abs() < 1e-15);
        assert!((smoothstep(1.0) - 1.0).abs() < 1e-15);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-15);
    }

    #[test]
    fn lerp_endpoints() {
        assert!((lerp(0.0, 3.0, 7.0) - 3.0).abs() < 1e-15);
        assert!((lerp(1.0, 3.0, 7.0) - 7.0).abs() < 1e-15);
    }
}
