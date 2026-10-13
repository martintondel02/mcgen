//! Random number sources.
//!
//! Minecraft has two families of `RandomSource`:
//!
//! * [`LegacyRandomSource`] — a 48-bit LCG, byte-for-byte `java.util.Random`.
//!   This is what vanilla uses for the `legacy`/`createLegacy` worldgen path.
//! * `XoroshiroRandomSource` — the modern default, which additionally supports
//!   *splitters* (`nextSplitter` + `splitString`) so that each octave of a
//!   noise gets an independent stream.
//!
//! Only the legacy source is implemented so far. That makes the legacy noise
//! path complete and self-consistent; see `README` for what that does and does
//! not prove about parity with a default 1.21.1 world.

/// The primitives the noise code needs from a random source.
pub trait RandomSource {
    /// Uniform in `[0, 1)`, matching `RandomSource.nextDouble()`.
    fn next_f64(&mut self) -> f64;

    /// Uniform in `[0, bound)`, matching `RandomSource.nextInt(int)` (which
    /// includes `java.util.Random`'s rejection-to-avoid-bias loop).
    fn next_bounded_i32(&mut self, bound: i32) -> i32;

    /// Advance the stream by `count` draws, matching `consumeCount`.
    fn skip(&mut self, count: usize);
}

/// 48-bit linear congruential generator, identical to `java.util.Random` and to
/// Minecraft's `LegacyRandomSource`.
#[derive(Debug, Clone)]
pub struct LegacyRandomSource {
    seed: u64,
}

const LCG_MULTIPLIER: u64 = 0x5DEECE66D;
const LCG_ADDEND: u64 = 0xB;
const LCG_MASK: u64 = (1 << 48) - 1;

impl LegacyRandomSource {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed: (seed ^ LCG_MULTIPLIER) & LCG_MASK,
        }
    }

    /// `LegacyRandomSource.forkPositional()`. Note this **consumes** a `nextLong()`:
    /// the factory seed is a draw from the source, not the source's own seed.
    pub fn fork_positional(&mut self) -> LegacyPositionalRandomFactory {
        LegacyPositionalRandomFactory::new(self.next_long())
    }

    /// `java.util.Random.nextLong()`: two 32-bit draws, high word first.
    pub fn next_long(&mut self) -> i64 {
        let hi = i64::from(self.next(32));
        let lo = i64::from(self.next(32));
        (hi << 32).wrapping_add(lo)
    }

    /// `java.util.Random.next(int)`: returns the top `bits` bits of the new state.
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = self
            .seed
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_ADDEND)
            & LCG_MASK;
        (self.seed >> (48 - bits)) as i32
    }
}

impl RandomSource for LegacyRandomSource {
    fn next_f64(&mut self) -> f64 {
        // (((long) next(26) << 27) + next(27)) * 0x1.0p-53
        let hi = i64::from(self.next(26));
        let lo = i64::from(self.next(27));
        ((hi << 27) + lo) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    fn next_bounded_i32(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive, got {bound}");

        // Power-of-two fast path, exactly as java.util.Random does it.
        if bound & (bound - 1) == 0 {
            let bits = i64::from(self.next(31));
            return ((i64::from(bound) * bits) >> 31) as i32;
        }

        loop {
            let bits = self.next(31);
            let val = bits % bound;
            // Reject if the value would make the distribution non-uniform.
            if i64::from(bits) - i64::from(val) + i64::from(bound - 1) >= 0 {
                return val;
            }
        }
    }

    fn skip(&mut self, count: usize) {
        for _ in 0..count {
            self.next(1);
        }
    }
}

/// The modern default: XOROSHIRO128++ as used by Minecraft's
/// `XoroshiroRandomSource`.
///
/// This is the source vanilla actually uses for a default 1.21.1 world, and the
/// one that supports *splitters* — independent per-octave streams derived from a
/// parent. The legacy LCG above cannot do that, which is why the legacy path
/// alone cannot reach vanilla parity.
#[derive(Debug, Clone)]
pub struct XoroshiroRandomSource {
    lo: u64,
    hi: u64,
}

/// 0x9E3779B97F4A7C15 — the golden-ratio gamma of SplitMix64.
const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
/// 0x6A09E667F3BCC909 — the second SplitMix64 increment.
const SILVER: u64 = 0x6A09_E667_F3BC_C909;
/// 0xBF58476D1CE4E5B9
const MIX_A: u64 = 0xBF58_476D_1CE4_E5B9;
/// 0x94D049BB133111EB
const MIX_B: u64 = 0x94D0_49BB_1331_11EB;

/// SplitMix64's finalizer, which Minecraft calls `mixStafford13`.
#[inline]
fn mix_stafford13(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(MIX_A);
    value = (value ^ (value >> 27)).wrapping_mul(MIX_B);
    value ^ (value >> 31)
}

/// Expand a single 64-bit seed into the 128-bit state, matching
/// `RandomSupport.upgradeSeedTo128bit`.
#[inline]
fn upgrade_seed_to_128bit(seed: u64) -> (u64, u64) {
    let l = seed ^ SILVER;
    let m = l.wrapping_add(GOLDEN_GAMMA);
    (mix_stafford13(l), mix_stafford13(m))
}

impl XoroshiroRandomSource {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let (lo, hi) = upgrade_seed_to_128bit(seed);
        Self::from_state(lo, hi)
    }

    /// Build directly from a 128-bit state, as `XoroshiroRandomSource(long,long)` does.
    #[must_use]
    pub fn from_state(lo: u64, hi: u64) -> Self {
        // Vanilla guards against the all-zero state, which xorshiro cannot
        // escape from.
        if (lo | hi) == 0 {
            Self {
                lo: GOLDEN_GAMMA,
                hi: SILVER,
            }
        } else {
            Self { lo, hi }
        }
    }

    /// The generator's primitive; everything else is derived from it.
    #[inline]
    pub fn next_long(&mut self) -> i64 {
        let l = self.lo;
        let m = self.hi;
        let out = l.wrapping_add(m).rotate_left(17).wrapping_add(l);
        let x = m ^ l;
        self.lo = l.rotate_left(49) ^ x ^ (x << 21);
        self.hi = x.rotate_left(28);
        out as i64
    }

    /// `(nextLong() >>> 11) * 2^-53`.
    #[inline]
    pub fn next_double(&mut self) -> f64 {
        ((self.next_long() as u64) >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// `(int) nextLong()`.
    #[inline]
    pub fn next_i32(&mut self) -> i32 {
        self.next_long() as i32
    }

    /// Vanilla's multiply-shift bounded int, including the rejection step that
    /// removes modulo bias. This is *not* `java.util.Random`'s algorithm.
    pub fn next_bounded_i32(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive, got {bound}");
        let bound_u = bound as u64;

        let mut l = (self.next_i32() as u64) & 0xFFFF_FFFF;
        let mut m = l.wrapping_mul(bound_u);
        let mut n = m & 0xFFFF_FFFF;

        if n < bound_u {
            // Equivalent to `Integer.remainderUnsigned(-bound, bound)`.
            let threshold = (!bound).wrapping_add(1) as u64 % bound_u;
            while n < threshold {
                l = (self.next_i32() as u64) & 0xFFFF_FFFF;
                m = l.wrapping_mul(bound_u);
                n = m & 0xFFFF_FFFF;
            }
        }

        (m >> 32) as i32
    }

    /// Derive a positional factory, as `forkPositional()` does.
    ///
    /// This is the capability the legacy LCG lacks, and it is what gives every
    /// octave of a `PerlinNoise` its own independent stream.
    pub fn fork_positional(&mut self) -> XoroshiroPositionalFactory {
        XoroshiroPositionalFactory {
            lo: self.next_long() as u64,
            hi: self.next_long() as u64,
        }
    }
}

impl RandomSource for XoroshiroRandomSource {
    fn next_f64(&mut self) -> f64 {
        self.next_double()
    }

    fn next_bounded_i32(&mut self, bound: i32) -> i32 {
        Self::next_bounded_i32(self, bound)
    }

    fn skip(&mut self, count: usize) {
        for _ in 0..count {
            self.next_long();
        }
    }
}

/// `RandomSupport.seedFromHashOf`: MD5 the UTF-8 name and split the digest into
/// two big-endian halves.
///
/// Vanilla uses MD5 here specifically, so this is not a detail we can swap for a
/// "better" hash — a different hash gives different worlds.
#[must_use]
pub fn seed_from_hash_of(name: &str) -> (u64, u64) {
    use md5::{Digest, Md5};

    let digest = Md5::digest(name.as_bytes());
    let mut lo_bytes = [0u8; 8];
    let mut hi_bytes = [0u8; 8];
    lo_bytes.copy_from_slice(&digest[0..8]);
    hi_bytes.copy_from_slice(&digest[8..16]);
    (u64::from_be_bytes(lo_bytes), u64::from_be_bytes(hi_bytes))
}

/// Java's `String.hashCode()` over UTF-16 code units — a 31-based rolling hash, not
/// a digest. Vanilla uses this to seed positional factories by name.
#[must_use]
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(unit));
    }
    h
}

/// `LegacyRandomSource.LegacyPositionalRandomFactory`.
#[derive(Debug, Clone, Copy)]
pub struct LegacyPositionalRandomFactory {
    seed: i64,
}

impl LegacyPositionalRandomFactory {
    #[must_use]
    pub fn new(seed: i64) -> Self {
        Self { seed }
    }

    /// `new LegacyRandomSource((long) name.hashCode() ^ this.seed)`.
    ///
    /// This is what `RandomState` uses for every noise in the overworld, and it is
    /// nothing like the Xoroshiro path: no MD5, no 128-bit seed.
    #[must_use]
    pub fn from_hash_of(&self, name: &str) -> LegacyRandomSource {
        LegacyRandomSource::new(((i64::from(java_string_hash_code(name))) ^ self.seed) as u64)
    }
}

/// Derives child generators by name, matching `XoroshiroRandomSource`'s
/// positional factory (what `forkPositional()` returns).
#[derive(Debug, Clone)]
pub struct XoroshiroPositionalFactory {
    lo: u64,
    hi: u64,
}

impl XoroshiroPositionalFactory {
    /// `fromHashOf`: XOR the name's hash into the factory state.
    ///
    /// This is how vanilla gives each octave of a `PerlinNoise` its own stream:
    /// the octave name is `"octave_<index>"`.
    #[must_use]
    pub fn from_hash_of(&self, name: &str) -> XoroshiroRandomSource {
        let (lo, hi) = seed_from_hash_of(name);
        XoroshiroRandomSource::from_state(lo ^ self.lo, hi ^ self.hi)
    }
}

#[cfg(test)]
mod legacy_factory_tests {
    use super::*;

    #[test]
    fn java_string_hash_code_matches_the_jvm() {
        // Pinned against Java's String.hashCode, which is a 31-based rolling hash
        // over UTF-16 code units. Values from a real JVM.
        assert_eq!(java_string_hash_code("minecraft:offset"), -920_384_768);
        assert_eq!(java_string_hash_code("minecraft:ridge"), -1_135_210_850);
        assert_eq!(java_string_hash_code("minecraft:temperature"), -549_971_161);
        assert_eq!(java_string_hash_code("octave_0"), 1_261_148_513);
        assert_eq!(java_string_hash_code(""), 0);
    }

    #[test]
    fn fork_positional_consumes_a_draw() {
        // The subtle one: `forkPositional()` calls nextLong(), so the factory seed
        // is a draw from the source rather than the source's own seed. Two factories
        // forked in a row must therefore differ.
        let mut a = LegacyRandomSource::new(0);
        let first = a.fork_positional();
        let second = a.fork_positional();
        let mut r1 = first.from_hash_of("minecraft:offset");
        let mut r2 = second.from_hash_of("minecraft:offset");
        assert_ne!(r1.next_long(), r2.next_long());
    }

    #[test]
    fn legacy_factory_from_hash_of_is_hash_xor_seed() {
        let f = LegacyPositionalRandomFactory::new(12345);
        let mut got = f.from_hash_of("minecraft:offset");
        let mut want = LegacyRandomSource::new(
            (i64::from(java_string_hash_code("minecraft:offset")) ^ 12345) as u64,
        );
        assert_eq!(got.next_long(), want.next_long());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values produced by `java.util.Random(42).nextDouble()` etc. This anchors
    /// our LCG to the real JVM implementation rather than to our own reading of it.
    #[test]
    fn matches_java_util_random_seeded_with_42() {
        let mut r = LegacyRandomSource::new(42);
        let a = r.next_f64();
        let b = r.next_f64();
        let c = r.next_f64();
        // java.util.Random(42): 0.7275636800328681, 0.6832234717598454, 0.30871945533265976
        assert!((a - 0.727_563_680_032_868_1).abs() < 1e-15, "a={a}");
        assert!((b - 0.683_223_471_759_845_4).abs() < 1e-15, "b={b}");
        assert!((c - 0.308_719_455_332_659_76).abs() < 1e-15, "c={c}");
    }

    /// Ground truth from the JVM (`new Random(7).nextInt(256)`), captured by
    /// `tools/javaref/RefValues.java`. This pins the bounded-int path,
    /// including the power-of-two fast path used by the permutation shuffle.
    #[test]
    fn matches_java_util_random_next_int_256_seeded_with_7() {
        let mut r = LegacyRandomSource::new(7);
        let got: Vec<i32> = (0..8).map(|_| r.next_bounded_i32(256)).collect();
        assert_eq!(got, vec![187, 163, 191, 2, 89, 125, 229, 190]);
    }

    #[test]
    fn matches_java_util_random_next_int_256_seeded_with_42() {
        let mut r = LegacyRandomSource::new(42);
        let got: Vec<i32> = (0..8).map(|_| r.next_bounded_i32(256)).collect();
        assert_eq!(got, vec![186, 13, 174, 12, 79, 241, 70, 181]);
    }

    #[test]
    fn next_int_is_in_range_and_reproducible() {
        let mut a = LegacyRandomSource::new(7);
        let mut b = LegacyRandomSource::new(7);
        for _ in 0..1000 {
            let x = a.next_bounded_i32(256);
            let y = b.next_bounded_i32(256);
            assert_eq!(x, y);
            assert!((0..256).contains(&x));
        }
    }

    #[test]
    fn power_of_two_path_matches_modulo_path_for_small_bounds() {
        // 256 is a power of two so it takes the fast path; check it still stays
        // in range and is uniform-ish across the whole interval.
        let mut seen = [false; 256];
        let mut r = LegacyRandomSource::new(1234);
        for _ in 0..20_000 {
            seen[r.next_bounded_i32(256) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "every bucket should be hit");
    }
}
