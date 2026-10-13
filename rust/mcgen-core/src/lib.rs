//! `mcgen-core` — Minecraft 1.21.1 worldgen primitives in Rust.
//!
//! # What this is
//!
//! The CPU-bound core of a worldgen accelerator, with no dependency on
//! Minecraft, the JVM, or NeoForge. Everything here is testable with
//! `cargo test`, which is the point: the interesting risk in this project is
//! *correctness*, not FFI plumbing.
//!
//! * [`noise`] — `PerlinNoise` / `NormalNoise`, matching vanilla's math.
//! * [`density`] — parses and evaluates datapack density-function JSON.
//! * [`terrain`] — turns density values into chunk block data, in parallel.
//! * [`random`] — `LegacyRandomSource` (the `java.util.Random` LCG).
//! * [`math`] — `wrap`, `smoothstep`, `lerp3` and friends.
//!
//! # What this is not
//!
//! Not a Minecraft server, and not yet a *vanilla-parity* generator. See the
//! repository README for the honest status table.

pub mod density;
pub mod math;
pub mod noise;
pub mod random;
pub mod terrain;

/// FNV-1a over a stream of `f64` bit patterns.
///
/// Used to pin down "this generator produced exactly these values" in tests
/// and across the FFI boundary. Any accidental change to the math — or any
/// nondeterminism from threading — shows up as a changed hash.
#[must_use]
pub fn fnv1a_f64(values: impl Iterator<Item = f64>) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for v in values {
        for byte in v.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

/// FNV-1a over raw bytes (used for chunk fingerprints).
#[must_use]
pub fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_sensitive_to_every_bit() {
        let a = fnv1a_bytes(&[0, 0, 0, 0]);
        let b = fnv1a_bytes(&[0, 0, 0, 1]);
        assert_ne!(a, b);
    }

    #[test]
    fn fnv1a_f64_distinguishes_similar_values() {
        let a = fnv1a_f64([1.0, 2.0, 3.0].into_iter());
        let b = fnv1a_f64([1.0, 2.0, 3.000_000_1].into_iter());
        assert_ne!(a, b);
    }

    #[test]
    fn fnv1a_is_order_sensitive() {
        let a = fnv1a_f64([1.0, 2.0].into_iter());
        let b = fnv1a_f64([2.0, 1.0].into_iter());
        assert_ne!(a, b);
    }
}
