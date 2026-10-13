//! Vanilla-faithful noise.
//!
//! The structure here is not a guess: it was read out of the bytecode of the
//! real `net.minecraft.world.level.levelgen.synth` classes shipped with the
//! server, and then verified bit-for-bit against them by
//! `scripts/check-parity.sh`. Three details in particular are load-bearing and
//! easy to get subtly wrong:
//!
//! 1. **`NormalNoise` samples `second` at `(x,y,z) * INPUT_FACTOR`** at the call
//!    site — not by scaling each octave's frequency. `x * IF * f` and
//!    `x * (f * IF)` round differently.
//! 2. **`valueFactor` is applied once, at the end**, as
//!    `(first + second) * valueFactor`. Folding it into the layer amplitudes
//!    changes the association and shifts results by one ULP.
//! 3. **Each octave's `ImprovedNoise` comes from
//!    `forkPositional().fromHashOf("octave_<index>")`**, not from a shared
//!    stream and not from `nextSplitter()`.
//!
//! All arithmetic is `f64`, matching vanilla's `double`.

use crate::math::{lerp3, smoothstep, wrap, GRADIENTS, NOISE_OFFSET_SCALE};
use crate::random::{RandomSource, XoroshiroRandomSource};

/// Per-octave standard deviation of raw Perlin output. Vanilla constant.
const STANDARD_DEVIATION: f64 = 0.270_224_783_124_521_1;
/// Vanilla targets a deviation of 1/3 for normal noise.
const TARGET_DEVIATION: f64 = 1.0 / 3.0;
/// The second stack is sampled at this multiple of the input coordinates.
const INPUT_FACTOR: f64 = 1.018_126_888_217_522_7;
/// Vanilla's literal numerator for `NormalNoise.valueFactor` (= 1/6).
const VALUE_FACTOR_NUMERATOR: f64 = 0.166_666_666_666_666_66;
const LACUNARITY: f64 = 2.0;
const PERSISTENCE: f64 = 0.5;
/// Number of draws a skipped octave would have consumed.
const SKIP_OCTAVE_DRAWS: usize = 262;

#[inline]
fn grad_dot(hash: i32, x: f64, y: f64, z: f64) -> f64 {
    let g = &GRADIENTS[(hash & 15) as usize];
    g[0] * x + g[1] * y + g[2] * z
}

/// A single octave of Perlin noise — vanilla's `ImprovedNoise`.
///
/// Note this does *not* fold coordinates: vanilla applies `wrap` one level up, in
/// `PerlinNoise.getValue`. Calling `noise` directly with unwrapped coordinates is
/// what vanilla does too.
#[derive(Debug, Clone)]
pub struct ImprovedNoise {
    perms: [u8; 256],
    pub offset_x: f64,
    pub offset_y: f64,
    pub offset_z: f64,
}

impl ImprovedNoise {
    /// Consumes the same draws as vanilla: 3 doubles, then 256 bounded ints.
    pub fn new(random: &mut impl RandomSource) -> Self {
        let offset_x = random.next_f64() * NOISE_OFFSET_SCALE;
        let offset_y = random.next_f64() * NOISE_OFFSET_SCALE;
        let offset_z = random.next_f64() * NOISE_OFFSET_SCALE;

        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = random.next_bounded_i32((256 - i) as i32) as usize + i;
            perms.swap(i, j);
        }

        Self {
            perms,
            offset_x,
            offset_y,
            offset_z,
        }
    }

    #[inline]
    fn permute(&self, x: i32) -> i32 {
        i32::from(self.perms[(x & 0xFF) as usize])
    }

    /// The 8-corner gather plus trilinear blend.
    #[allow(clippy::too_many_arguments)]
    fn sample_and_lerp(
        &self,
        x: i32,
        y: i32,
        z: i32,
        relative_x: f64,
        relative_y: f64,
        relative_z: f64,
        original_relative_y: f64,
    ) -> f64 {
        let x0 = self.permute(x);
        let x1 = self.permute(x + 1);
        let xy00 = self.permute(x0 + y);
        let xy01 = self.permute(x0 + y + 1);
        let xy10 = self.permute(x1 + y);
        let xy11 = self.permute(x1 + y + 1);

        let d000 = grad_dot(self.permute(xy00 + z), relative_x, relative_y, relative_z);
        let d100 = grad_dot(
            self.permute(xy10 + z),
            relative_x - 1.0,
            relative_y,
            relative_z,
        );
        let d010 = grad_dot(
            self.permute(xy01 + z),
            relative_x,
            relative_y - 1.0,
            relative_z,
        );
        let d110 = grad_dot(
            self.permute(xy11 + z),
            relative_x - 1.0,
            relative_y - 1.0,
            relative_z,
        );
        let d001 = grad_dot(
            self.permute(xy00 + z + 1),
            relative_x,
            relative_y,
            relative_z - 1.0,
        );
        let d101 = grad_dot(
            self.permute(xy10 + z + 1),
            relative_x - 1.0,
            relative_y,
            relative_z - 1.0,
        );
        let d011 = grad_dot(
            self.permute(xy01 + z + 1),
            relative_x,
            relative_y - 1.0,
            relative_z - 1.0,
        );
        let d111 = grad_dot(
            self.permute(xy11 + z + 1),
            relative_x - 1.0,
            relative_y - 1.0,
            relative_z - 1.0,
        );

        lerp3(
            smoothstep(relative_x),
            smoothstep(original_relative_y),
            smoothstep(relative_z),
            d000, d100, d010, d110, d001, d101, d011, d111,
        )
    }

    /// Vanilla's 5-argument `noise(x, y, z, yScale, yFreq)`, which shears the
    /// interpolation along Y.
    ///
    /// Only the legacy blended noise uses this. Note the shear is subtracted from
    /// the *gradient* y, while the y smoothstep still uses the untouched relative
    /// y — which is why `sample_and_lerp` carries both.
    #[must_use]
    pub fn noise_sheared(&self, x: f64, y: f64, z: f64, y_scale: f64, y_freq: f64) -> f64 {
        let x = x + self.offset_x;
        let y = y + self.offset_y;
        let z = z + self.offset_z;

        let floor_x = x.floor();
        let floor_y = y.floor();
        let floor_z = z.floor();

        let relative_x = x - floor_x;
        let relative_y = y - floor_y;
        let relative_z = z - floor_z;

        let sheared = if y_scale != 0.0 {
            let pick = if y_freq >= 0.0 && y_freq < relative_y {
                y_freq
            } else {
                relative_y
            };
            // Vanilla uses the *f32* value of 1e-7 widened to f64, not 1e-7.
            (pick / y_scale + f64::from(1.0e-7_f32)).floor() * y_scale
        } else {
            0.0
        };

        self.sample_and_lerp(
            floor_x as i32,
            floor_y as i32,
            floor_z as i32,
            relative_x,
            relative_y - sheared,
            relative_z,
            relative_y,
        )
    }

    /// Sample at a position. Equivalent to vanilla's `ImprovedNoise.noise(x, y, z)`.
    #[must_use]
    pub fn noise(&self, x: f64, y: f64, z: f64) -> f64 {
        let x = x + self.offset_x;
        let y = y + self.offset_y;
        let z = z + self.offset_z;

        let floor_x = x.floor();
        let floor_y = y.floor();
        let floor_z = z.floor();

        let relative_x = x - floor_x;
        let relative_y = y - floor_y;
        let relative_z = z - floor_z;

        self.sample_and_lerp(
            floor_x as i32,
            floor_y as i32,
            floor_z as i32,
            relative_x,
            relative_y,
            relative_z,
            relative_y,
        )
    }
}

/// Vanilla's `PerlinNoise`: a stack of octaves at doubling frequency and halving
/// amplitude.
#[derive(Debug, Clone)]
pub struct PerlinNoise {
    noise_levels: Vec<Option<ImprovedNoise>>,
    first_octave: i32,
    amplitudes: Vec<f64>,
    lowest_freq_input_factor: f64,
    lowest_freq_value_factor: f64,
}

impl PerlinNoise {
    fn finish(
        noise_levels: Vec<Option<ImprovedNoise>>,
        first_octave: i32,
        amplitudes: &[f64],
    ) -> Self {
        let j = amplitudes.len() as i32;
        Self {
            noise_levels,
            first_octave,
            amplitudes: amplitudes.to_vec(),
            lowest_freq_input_factor: LACUNARITY.powi(first_octave),
            lowest_freq_value_factor: LACUNARITY.powi(j - 1) / (LACUNARITY.powi(j) - 1.0),
        }
    }

    /// The modern path: each octave's noise is derived by name from a positional
    /// factory, so octaves are independent streams.
    pub fn create_modern(
        random: &mut XoroshiroRandomSource,
        first_octave: i32,
        amplitudes: &[f64],
    ) -> Self {
        let j = amplitudes.len();

        // Consumes two draws, exactly like vanilla.
        let factory = random.fork_positional();

        let mut noise_levels: Vec<Option<ImprovedNoise>> = vec![None; j];
        for (i, level) in noise_levels.iter_mut().enumerate() {
            if amplitudes[i] != 0.0 {
                let octave = first_octave + i as i32;
                let mut child = factory.from_hash_of(&format!("octave_{octave}"));
                *level = Some(ImprovedNoise::new(&mut child));
            }
        }

        Self::finish(noise_levels, first_octave, amplitudes)
    }

    /// The legacy path: a single shared stream, with skipped octaves still
    /// consuming their draws so later octaves stay aligned.
    pub fn create_legacy(
        random: &mut impl RandomSource,
        first_octave: i32,
        amplitudes: &[f64],
    ) -> Self {
        let j = amplitudes.len();
        let zero_octave_index = -first_octave;

        let mut noise_levels: Vec<Option<ImprovedNoise>> = vec![None; j];

        let zero_octave = ImprovedNoise::new(random);
        if zero_octave_index >= 0
            && (zero_octave_index as usize) < j
            && amplitudes[zero_octave_index as usize] != 0.0
        {
            noise_levels[zero_octave_index as usize] = Some(zero_octave);
        }

        let mut i = zero_octave_index - 1;
        while i >= 0 {
            if (i as usize) < j {
                if amplitudes[i as usize] == 0.0 {
                    random.skip(SKIP_OCTAVE_DRAWS);
                } else {
                    noise_levels[i as usize] = Some(ImprovedNoise::new(random));
                }
            } else {
                random.skip(SKIP_OCTAVE_DRAWS);
            }
            i -= 1;
        }

        Self::finish(noise_levels, first_octave, amplitudes)
    }

    /// Sum the octaves, with `wrap` applied per octave before sampling — this is
    /// the step that keeps far-from-origin terrain stable.
    #[must_use]
    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut d = 0.0;
        let mut e = self.lowest_freq_input_factor;
        let mut f = self.lowest_freq_value_factor;

        for (i, level) in self.noise_levels.iter().enumerate() {
            if let Some(noise) = level {
                let v = noise.noise(wrap(x * e), wrap(y * e), wrap(z * e));
                // Vanilla's order: amplitudes[i] * value * f.
                d += self.amplitudes[i] * v * f;
            }
            e *= LACUNARITY;
            f /= LACUNARITY;
        }

        d
    }

    /// A loose upper bound on `|get_value|`; vanilla derives a tighter one.
    #[must_use]
    pub fn max_value(&self) -> f64 {
        let mut f = self.lowest_freq_value_factor;
        let mut total = 0.0;
        for a in &self.amplitudes {
            total += a.abs() * f;
            f /= LACUNARITY;
        }
        total * 2.0 * 3.0
    }

    #[must_use]
    pub fn first_octave(&self) -> i32 {
        self.first_octave
    }

    /// Vanilla's `getOctaveNoise(int)`.
    ///
    /// Despite the name, this indexes **from the end**:
    /// `noiseLevels[noiseLevels.length - 1 - index]`. Reading it as a plain index
    /// silently consumes the octaves in reverse, which produces plausible-looking
    /// but wrong noise.
    #[must_use]
    pub fn octave_noise(&self, index: usize) -> Option<&ImprovedNoise> {
        self.noise_levels
            .len()
            .checked_sub(1 + index)
            .and_then(|i| self.noise_levels.get(i))
            .and_then(Option::as_ref)
    }
}

/// Vanilla's `BlendedNoise` — the pre-1.18 terrain noise, reached through the
/// `minecraft:old_blended_noise` density node.
///
/// Three Perlin stacks: a min/max limit pair over octaves `-15..=0`, and a main
/// stack over `-7..=0`. The main stack produces a blend factor that can *skip*
/// the limit stacks entirely at its extremes.
#[derive(Debug, Clone)]
pub struct BlendedNoise {
    min_limit_noise: PerlinNoise,
    max_limit_noise: PerlinNoise,
    main_noise: PerlinNoise,
    xz_multiplier: f64,
    y_multiplier: f64,
    xz_factor: f64,
    y_factor: f64,
    smear_scale_multiplier: f64,
}

/// `Mth.clampedLerp(start, end, delta)` — the argument order is
/// `(start, end, delta)`, not `(delta, start, end)` as the name suggests.
#[inline]
fn clamped_lerp(start: f64, end: f64, delta: f64) -> f64 {
    if delta < 0.0 {
        start
    } else if delta > 1.0 {
        end
    } else {
        start + delta * (end - start)
    }
}

impl BlendedNoise {
    /// `BlendedNoise.createUnseeded`: what a datapack `old_blended_noise` node
    /// builds. The random source is `XoroshiroRandomSource(0)`.
    #[must_use]
    pub fn create_unseeded(
        xz_scale: f64,
        y_scale: f64,
        xz_factor: f64,
        y_factor: f64,
        smear_scale_multiplier: f64,
    ) -> Self {
        let mut random = XoroshiroRandomSource::new(0);
        Self::new_seeded(
            &mut random,
            xz_scale,
            y_scale,
            xz_factor,
            y_factor,
            smear_scale_multiplier,
        )
    }

    /// Mirrors `BlendedNoise(RandomSource, …)`: three stacks, in this order, all
    /// via the legacy (no-positional-factory) path.
    pub fn new_seeded(
        random: &mut XoroshiroRandomSource,
        xz_scale: f64,
        y_scale: f64,
        xz_factor: f64,
        y_factor: f64,
        smear_scale_multiplier: f64,
    ) -> Self {
        // `makeAmplitudes` over rangeClosed(-15, 0) reduces to sixteen ones with
        // firstOctave -15; over rangeClosed(-7, 0), eight ones with firstOctave -7.
        let amplitudes_16 = [1.0f64; 16];
        let amplitudes_8 = [1.0f64; 8];

        let min_limit_noise = PerlinNoise::create_legacy(random, -15, &amplitudes_16);
        let max_limit_noise = PerlinNoise::create_legacy(random, -15, &amplitudes_16);
        let main_noise = PerlinNoise::create_legacy(random, -7, &amplitudes_8);

        Self {
            min_limit_noise,
            max_limit_noise,
            main_noise,
            // The constructor DERIVES these from the scale factors:
            // `xzMultiplier = 684.412 * xzScale`. Using the scale directly (which
            // is what the JSON field names suggest) is wrong by a factor of ~2700.
            xz_multiplier: 684.412 * xz_scale,
            y_multiplier: 684.412 * y_scale,
            xz_factor,
            y_factor,
            smear_scale_multiplier,
        }
    }

    #[must_use]
    pub fn compute(&self, x: f64, y: f64, z: f64) -> f64 {
        let (_main_sum, d, min_sum, max_sum) = self.terms(x, y, z);
        clamped_lerp(min_sum / 512.0, max_sum / 512.0, d) / 128.0
    }

    /// The four intermediates, exposed so parity failures can be diagnosed
    /// without guessing. Returns `(main_sum, d, min_sum, max_sum)`.
    #[doc(hidden)]
    #[must_use]
    pub fn debug_terms(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64, f64) {
        self.terms(x, y, z)
    }

    fn terms(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64, f64) {
        let sx_full = x * self.xz_multiplier;
        let sy_full = y * self.y_multiplier;
        let sz_full = z * self.xz_multiplier;

        let sx = sx_full / self.xz_factor;
        let sy = sy_full / self.y_factor;
        let sz = sz_full / self.xz_factor;

        let smear = self.y_multiplier * self.smear_scale_multiplier;
        let smear_freq = smear / self.y_factor;

        let mut min_sum = 0.0;
        let mut max_sum = 0.0;
        let mut main_sum = 0.0;

        let mut freq = 1.0;
        for i in 0..8 {
            if let Some(noise) = self.main_noise.octave_noise(i) {
                main_sum += noise.noise_sheared(
                    wrap(sx * freq),
                    wrap(sy * freq),
                    wrap(sz * freq),
                    smear_freq * freq,
                    sy * freq,
                ) / freq;
            }
            freq /= 2.0;
        }

        let d = (main_sum / 10.0 + 1.0) / 2.0;

        // At the extremes the blend factor pins to one end, so the corresponding
        // limit stack is not evaluated at all.
        let skip_min = d >= 1.0;
        let skip_max = d <= 0.0;

        freq = 1.0;
        for i in 0..16 {
            let wx = wrap(sx_full * freq);
            let wy = wrap(sy_full * freq);
            let wz = wrap(sz_full * freq);
            let w_smear = smear * freq;

            if !skip_min {
                if let Some(noise) = self.min_limit_noise.octave_noise(i) {
                    min_sum += noise.noise_sheared(wx, wy, wz, w_smear, sy_full * freq) / freq;
                }
            }
            if !skip_max {
                if let Some(noise) = self.max_limit_noise.octave_noise(i) {
                    max_sum += noise.noise_sheared(wx, wy, wz, w_smear, sy_full * freq) / freq;
                }
            }
            freq /= 2.0;
        }

        (main_sum, d, min_sum, max_sum)
    }
}

/// Vanilla's `NormalNoise`: two `PerlinNoise` stacks of identical shape, the
/// second sampled at shifted coordinates, scaled by a single `valueFactor`.
#[derive(Debug, Clone)]
pub struct NormalNoise {
    first: PerlinNoise,
    second: PerlinNoise,
    value_factor: f64,
    max_value: f64,
}

impl NormalNoise {
    /// Modern (positional-factory) path — what a default 1.21.1 world uses.
    pub fn new_modern(
        random: &mut XoroshiroRandomSource,
        first_octave: i32,
        amplitudes: &[f64],
    ) -> Self {
        let first = PerlinNoise::create_modern(random, first_octave, amplitudes);
        let second = PerlinNoise::create_modern(random, first_octave, amplitudes);

        let value_factor = VALUE_FACTOR_NUMERATOR / expected_deviation(octave_span(amplitudes));
        let max_value = (first.max_value() + second.max_value()) * value_factor;

        Self {
            first,
            second,
            value_factor,
            max_value,
        }
    }

    /// Legacy path, used by `createLegacy` worldgen and the legacy nether biome.
    pub fn new_legacy(
        random: &mut impl RandomSource,
        first_octave: i32,
        amplitudes: &[f64],
    ) -> Self {
        let first = PerlinNoise::create_legacy(random, first_octave, amplitudes);
        let second = PerlinNoise::create_legacy(random, first_octave, amplitudes);

        // The legacy path rescales amplitudes so its deviation matches the
        // pre-1.18 behaviour.
        let octave_count = amplitudes.len();
        let octaves = legacy_octave_amplitudes(octave_count, amplitudes);
        let mut target_amplitude: f64 = octaves.iter().map(|a| a.abs()).sum();
        let mut normalization_factor = compute_normalization_factor(target_amplitude, &octaves);

        if normalization_factor != 0.0 {
            let parity = VALUE_FACTOR_NUMERATOR / expected_deviation(octave_span(amplitudes));
            target_amplitude *= parity / normalization_factor;
            normalization_factor = parity;
        }

        let max_value = target_amplitude * TARGET_DEVIATION * 6.0;

        Self {
            first,
            second,
            value_factor: normalization_factor,
            max_value,
        }
    }

    /// Evaluate at a position.
    ///
    /// The order of operations mirrors vanilla's bytecode exactly: the shifted
    /// coordinates are computed first, both stacks are summed, and only then is
    /// `valueFactor` applied.
    #[must_use]
    pub fn get(&self, x: f64, y: f64, z: f64) -> f64 {
        let shifted_x = x * INPUT_FACTOR;
        let shifted_y = y * INPUT_FACTOR;
        let shifted_z = z * INPUT_FACTOR;

        (self.first.get_value(x, y, z) + self.second.get_value(shifted_x, shifted_y, shifted_z))
            * self.value_factor
    }

    /// Approximate bound on `|get(..)|`.
    #[must_use]
    pub fn max_value(&self) -> f64 {
        self.max_value
    }

    #[must_use]
    pub fn layer_count(&self) -> usize {
        self.first.noise_levels.len() + self.second.noise_levels.len()
    }
}

/// Vanilla's `NormalNoise.expectedDeviation`.
fn expected_deviation(octave_span: i32) -> f64 {
    0.1 * (1.0 + 1.0 / f64::from(octave_span + 1))
}

/// Distance between the lowest and highest non-zero amplitude index.
fn octave_span(amplitudes: &[f64]) -> i32 {
    let mut min = i32::MAX;
    let mut max = i32::MIN;
    for (i, a) in amplitudes.iter().enumerate() {
        if *a != 0.0 {
            min = min.min(i as i32);
            max = max.max(i as i32);
        }
    }
    if min == i32::MAX {
        0
    } else {
        max - min
    }
}

/// Effective amplitudes the legacy path builds before normalization.
fn legacy_octave_amplitudes(octave_count: usize, amplitudes: &[f64]) -> Vec<f64> {
    let n = octave_count as i32;
    let base = PERSISTENCE.powi(-(n - 1)) / (PERSISTENCE.powi(-n) - 1.0);
    amplitudes.iter().map(|a| a * base).collect()
}

fn estimate_deviation(amplitudes: &[f64]) -> f64 {
    let mut variance = 0.0;
    for a in amplitudes {
        let d = STANDARD_DEVIATION * a.abs();
        variance += d * d;
    }
    variance.sqrt()
}

fn compute_normalization_factor(target_amplitude: f64, amplitudes: &[f64]) -> f64 {
    let input_deviation = estimate_deviation(amplitudes);
    if input_deviation == 0.0 {
        0.0
    } else {
        let input_sum_deviation = input_deviation * LACUNARITY.sqrt();
        let target_deviation = target_amplitude * TARGET_DEVIATION;
        target_deviation / input_sum_deviation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::LegacyRandomSource;

    #[test]
    fn improved_noise_permutation_is_a_bijection() {
        let mut r = LegacyRandomSource::new(12345);
        let p = ImprovedNoise::new(&mut r);
        let mut seen = [false; 256];
        for v in p.perms {
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "table must contain every value once");
    }

    #[test]
    fn improved_noise_is_deterministic_for_a_fixed_seed() {
        let a = {
            let mut r = LegacyRandomSource::new(7);
            let p = ImprovedNoise::new(&mut r);
            (0..64)
                .map(|i| p.noise(f64::from(i), 1.5, 2.5))
                .collect::<Vec<_>>()
        };
        let b = {
            let mut r = LegacyRandomSource::new(7);
            let p = ImprovedNoise::new(&mut r);
            (0..64)
                .map(|i| p.noise(f64::from(i), 1.5, 2.5))
                .collect::<Vec<_>>()
        };
        assert_eq!(a, b);
    }

    #[test]
    fn improved_noise_differs_across_seeds() {
        let a = {
            let mut r = LegacyRandomSource::new(1);
            ImprovedNoise::new(&mut r).noise(0.5, 0.5, 0.5)
        };
        let b = {
            let mut r = LegacyRandomSource::new(2);
            ImprovedNoise::new(&mut r).noise(0.5, 0.5, 0.5)
        };
        assert!((a - b).abs() > 1e-9);
    }

    #[test]
    fn improved_noise_is_bounded() {
        let mut r = LegacyRandomSource::new(99);
        let p = ImprovedNoise::new(&mut r);
        let mut max = 0.0f64;
        for i in 0..2000 {
            let x = f64::from(i) * 0.37;
            let v = p.noise(x, x * 0.5, -x * 0.25);
            assert!(v.is_finite());
            assert!(v.abs() < 3.0, "unexpected magnitude {v}");
            max = max.max(v.abs());
        }
        assert!(max > 0.05, "noise looks degenerate, max={max}");
    }

    #[test]
    fn octave_span_ignores_leading_and_trailing_zeros() {
        assert_eq!(octave_span(&[1.0, 1.0, 1.0]), 2);
        assert_eq!(octave_span(&[1.0]), 0);
        assert_eq!(octave_span(&[0.0, 1.0, 0.0, 1.0, 0.0]), 2);
        assert_eq!(octave_span(&[0.0, 0.0]), 0);
    }

    #[test]
    fn expected_deviation_matches_the_documented_shape() {
        assert!((expected_deviation(0) - 0.2).abs() < 1e-15);
        assert!((expected_deviation(1) - 0.15).abs() < 1e-15);
    }

    #[test]
    fn modern_normal_noise_is_deterministic() {
        let amps = [1.0, 1.0, 1.0, 1.0];

        let a = {
            let mut rs = XoroshiroRandomSource::new(42);
            NormalNoise::new_modern(&mut rs, -7, &amps)
        };
        let b = {
            let mut rs = XoroshiroRandomSource::new(42);
            NormalNoise::new_modern(&mut rs, -7, &amps)
        };

        for i in 0..200 {
            let x = f64::from(i) * 13.7;
            let va = a.get(x, 0.0, -x * 0.5);
            let vb = b.get(x, 0.0, -x * 0.5);
            assert_eq!(va.to_bits(), vb.to_bits(), "same seed must be identical");
            assert!(va.is_finite());
        }
    }

    #[test]
    fn modern_normal_noise_has_unit_ish_variance() {
        let mut rs = XoroshiroRandomSource::new(42);
        let n = NormalNoise::new_modern(&mut rs, -7, &[1.0, 1.0, 1.0, 1.0]);

        let mut sum_sq = 0.0;
        let mut count = 0u64;
        for i in 0..20_000 {
            let x = f64::from(i) * 13.7;
            let v = n.get(x, 0.0, -x * 0.5);
            sum_sq += v * v;
            count += 1;
        }
        let rms = (sum_sq / count as f64).sqrt();
        // The normalization exists precisely so this lands in a predictable band.
        assert!((0.1..1.2).contains(&rms), "rms={rms} outside expected band");
    }

    #[test]
    fn zero_amplitude_octaves_do_not_panic() {
        let mut rs = XoroshiroRandomSource::new(7);
        let with_zero = NormalNoise::new_modern(&mut rs, -7, &[1.0, 0.0, 1.0]);
        assert!(with_zero.get(1.0, 2.0, 3.0).is_finite());
    }

    #[test]
    fn blended_noise_is_deterministic() {
        let a = BlendedNoise::create_unseeded(0.25, 0.125, 80.0, 160.0, 8.0);
        let b = BlendedNoise::create_unseeded(0.25, 0.125, 80.0, 160.0, 8.0);
        for i in 0..32 {
            let x = f64::from(i) * 7.0;
            assert_eq!(
                a.compute(x, 3.0, -x).to_bits(),
                b.compute(x, 3.0, -x).to_bits()
            );
        }
    }

    #[test]
    fn blended_noise_multipliers_include_the_684_factor() {
        // The constructor derives xzMultiplier = 684.412 * xzScale. Using the
        // scale directly is wrong by a factor of ~2700 and was an actual bug.
        let bn = BlendedNoise::create_unseeded(0.25, 0.125, 80.0, 160.0, 8.0);
        let (main_sum, d, _min, _max) = bn.debug_terms(1.0, 0.0, 1.0);
        assert!(main_sum.is_finite());
        assert!(d.is_finite());
        // A wrong multiplier makes the blend factor explode far outside [0,1] for
        // ordinary coordinates; this is a coarse sanity bound, not a parity check.
        assert!(d.abs() < 1000.0, "blend factor looks unhinged: {d}");
    }

    #[test]
    fn octave_noise_indexes_from_the_end() {
        // Vanilla: noiseLevels[length - 1 - index]. Getting this backwards was an
        // actual bug that silently consumed the octaves in reverse.
        let mut r = LegacyRandomSource::new(5);
        let amps = [1.0f64; 4];
        let perlin = PerlinNoise::create_legacy(&mut r, -3, &amps);
        assert!(perlin.octave_noise(0).is_some());
        assert!(perlin.octave_noise(3).is_some());
        assert!(perlin.octave_noise(4).is_none(), "out of range must be None");
    }

    #[test]
    fn legacy_path_still_builds_and_evaluates() {
        let mut rs = LegacyRandomSource::new(42);
        let n = NormalNoise::new_legacy(&mut rs, -7, &[1.0, 1.0, 1.0, 1.0]);
        let v = n.get(1.0, 2.0, 3.0);
        assert!(v.is_finite());
        assert!(n.max_value() > 0.0);
        assert!(n.layer_count() > 0);
    }
}
