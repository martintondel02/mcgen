//! Isolate the per-node cost of the tree evaluator.
//!
//! Four optimisations failed for want of this measurement. The full router walks ~19.8 nodes per
//! block at ~13 ns each, but that average mixes dispatch, noise sampling, the corner caches and
//! cache misses together, so it cannot say which one costs. These three cases separate them.

use mcgen_core::density::DensityFunction;
use mcgen_core::noise::{NormalNoise, PerlinNoise};
use mcgen_core::random::XoroshiroRandomSource;

fn chain(depth: usize) -> DensityFunction {
    let mut f = DensityFunction::Constant(0.5);
    for _ in 0..depth {
        f = DensityFunction::Add(Box::new(f), Box::new(DensityFunction::Constant(0.25)));
    }
    f
}

/// `depth` noise samples summed, so the walk is `depth` nodes deep and every leaf samples.
fn noise_chain(depth: usize) -> DensityFunction {
    let mut f = DensityFunction::Noise {
        noise: 0,
        xz_scale: 1.0,
        y_scale: 1.0,
    };
    for _ in 0..depth {
        f = DensityFunction::Add(
            Box::new(f),
            Box::new(DensityFunction::Noise {
                noise: 0,
                xz_scale: 1.0,
                y_scale: 1.0,
            }),
        );
    }
    f
}

fn make_noise() -> NormalNoise {
    let mut rs = XoroshiroRandomSource::new(0);
    NormalNoise::new_modern(&mut rs, -7, &[1.0, 1.0, 1.0, 1.0])
}

/// Time `f`, which is handed a per-iteration index.
///
/// The index matters. A first version took a nullary closure and reported
/// `ImprovedNoise::noise` at 0.4 ns — because with constant arguments LLVM hoisted the call
/// straight out of the loop and measured nothing. Every number here has to depend on the
/// counter or it is not a measurement.
fn time<F: FnMut(u64) -> f64>(iters: u64, mut f: F) -> f64 {
    for i in 0..5 {
        std::hint::black_box(f(i));
    }
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t0 = std::time::Instant::now();
        let mut sink = 0.0;
        for i in 0..iters {
            sink += f(i);
        }
        std::hint::black_box(sink);
        let ns = t0.elapsed().as_nanos() as f64 / iters as f64;
        if ns < best {
            best = ns;
        }
    }
    best
}

fn main() {
    let noise = make_noise();
    let noises = vec![noise];
    let iters = 20_000_000u64;

    let c20 = chain(20);
    let ns = time(iters, |i| {
        c20.compute(&noises, 1.0 + i as f64 * 1e-6, 2.0, 3.0)
    });
    println!(
        "chain of 20 Adds : {ns:7.1} ns/call = {:.2} ns/node",
        ns / 21.0
    );

    let c60 = chain(60);
    let ns = time(iters, |i| {
        c60.compute(&noises, 1.0 + i as f64 * 1e-6, 2.0, 3.0)
    });
    println!(
        "chain of 60 Adds : {ns:7.1} ns/call = {:.2} ns/node",
        ns / 61.0
    );

    let n = noise_chain(8);
    let ns = time(iters, |i| {
        n.compute(&noises, 1.0 + i as f64 * 1e-6, 2.0, 3.0)
    });
    println!(
        "8 noises + 8 Adds: {ns:7.1} ns/call = {:.2} ns/node",
        ns / 17.0
    );

    let ns = time(iters, |i| noises[0].get(1.0 + i as f64 * 1e-6, 2.0, 3.0));
    println!("NormalNoise::get : {ns:7.1} ns/call");

    // A leaf with no children, to price the dispatch alone.
    let k = DensityFunction::Constant(1.0);
    let ns = time(iters, |i| {
        k.compute(&noises, 1.0 + i as f64 * 1e-6, 2.0, 3.0)
    });
    println!("Constant leaf    : {ns:7.1} ns/call");

    let p = PerlinNoise::create_legacy(
        &mut XoroshiroRandomSource::new(0),
        -7,
        &[1.0, 1.0, 1.0, 1.0],
    );
    let ns = time(iters, |i| p.get_value(1.0 + i as f64 * 1e-6, 2.0, 3.0));
    println!(
        "PerlinNoise (4 octaves): {ns:7.1} ns/call = {:.1} ns/octave",
        ns / 4.0
    );

    // One octave on its own, to price the gradient noise rather than the octave loop.
    let octave = p.octave_noise(0).expect("octave 0");
    let ns = time(iters, |i| octave.noise(1.0 + i as f64 * 1e-6, 2.0, 3.0));
    println!("ImprovedNoise::noise     : {ns:7.1} ns/call");

    let ns = time(iters, |i| mcgen_core::math::wrap(1234.5 + i as f64 * 1e-6));
    println!("wrap (fast path)         : {ns:7.1} ns/call");
}
