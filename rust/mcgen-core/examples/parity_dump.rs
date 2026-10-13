//! Emits parity records for diffing against the real Minecraft classes.
//!
//! Paired with `tools/oracle/VanillaOracle.java`, which prints the same records
//! by calling the actual 1.21.1 code. `scripts/check-parity.sh` diffs the two.
//!
//! The format is deliberately trivial:
//!
//! ```text
//! <kind> <seed> <index> <16 hex digits of the raw f64 bits>
//! ```
//!
//! Raw bits, not formatted decimals: the question is bit-exactness, and `%f`
//! would hide a last-mantissa-bit difference — which is exactly the kind of
//! difference that shows up as visible chunk seams.

use mcgen_core::density::{DensityBuilder, NoiseSpec};
use mcgen_core::noise::{ImprovedNoise, NormalNoise};
use mcgen_core::random::{
    java_string_hash_code, seed_from_hash_of, LegacyRandomSource, XoroshiroRandomSource,
};

/// Must match `VanillaOracle.SEEDS`.
const SEEDS: [i64; 6] = [0, 1, 42, 123_456_789, 1_779_920_288_596_261_407, -1];

/// Must match `VanillaOracle.SAMPLES`.
const SAMPLES: i32 = 24;

// Must match VanillaOracle.sampleX/Y/Z exactly. These are the same IEEE-754
// operations in both languages, so the inputs are identical bit for bit.
fn sample_x(i: i32) -> f64 {
    f64::from(i - 12) * 0.37
}
fn sample_y(i: i32) -> f64 {
    f64::from(i) * 0.125 + 0.5
}
fn sample_z(i: i32) -> f64 {
    f64::from(12 - i) * 0.53
}

fn emit(kind: &str, seed: i64, index: i32, value: f64) {
    println!("{kind} {seed} {index} {:016x}", value.to_bits());
}

/// Single-octave Perlin against the legacy RNG. Isolates the core noise math.
fn improved_noise() {
    for &seed in &SEEDS {
        let mut rs = LegacyRandomSource::new(seed as u64);
        let noise = ImprovedNoise::new(&mut rs);
        for i in 0..SAMPLES {
            emit(
                "improved_noise",
                seed,
                i,
                noise.noise(sample_x(i), sample_y(i), sample_z(i)),
            );
        }
    }
}

/// Raw RNG output from the modern generator.
fn xoroshiro() {
    for &seed in &SEEDS {
        let mut rs = XoroshiroRandomSource::new(seed as u64);
        for i in 0..SAMPLES {
            emit("xoroshiro_long", seed, i, rs.next_long() as f64);
        }
    }
    for &seed in &SEEDS {
        let mut rs = XoroshiroRandomSource::new(seed as u64);
        for i in 0..SAMPLES {
            emit("xoroshiro_double", seed, i, rs.next_double());
        }
    }
}

/// The normalized multi-octave noise that density functions actually sample.
///
/// This is the modern splitter path, so it exercises the RNG derivation *and*
/// the octave stacking and normalization at once. Getting this to match means
/// the seed-derivation gap is closed.
fn normal_noise() {
    // Must match VanillaOracle.normalNoise's amplitudeSets.
    const AMPLITUDE_SETS: [&[f64]; 4] = [
        &[1.0],
        &[1.0, 1.0],
        &[1.0, 1.0, 1.0, 1.0],
        &[1.5, 0.0, 1.0, 0.0, 0.6],
    ];

    for &seed in &SEEDS {
        for (a, amps) in AMPLITUDE_SETS.iter().enumerate() {
            let mut rs = XoroshiroRandomSource::new(seed as u64);
            // Vanilla's NormalNoise.create(rs, firstOctave, amplitudes...).
            let noise = NormalNoise::new_modern(&mut rs, -7, amps);
            for i in 0..SAMPLES {
                emit(
                    &format!("normal_noise_a{a}"),
                    seed,
                    i,
                    noise.get(sample_x(i), sample_y(i), sample_z(i)),
                );
            }
        }
    }
}

/// `RandomSupport.seedFromHashOf` in isolation. Must match
/// `VanillaOracle.HASH_NAMES` order.
fn hash_strings() {
    const NAMES: [&str; 10] = [
        "octave_-7",
        "octave_-6",
        "octave_-5",
        "octave_-4",
        "octave_-3",
        "octave_-2",
        "octave_-1",
        "octave_0",
        "TEST STRING",
        "",
    ];
    for (i, name) in NAMES.iter().enumerate() {
        let (lo, hi) = seed_from_hash_of(name);
        println!("hash {i} {lo:016x} {hi:016x}");
    }
}

// ---- density-function layer ---------------------------------------------
//
// Vanilla evaluates density functions at *integer block coordinates*
// (FunctionContext.blockX/Y/Z), not at block centres. Must match
// VanillaOracle.blockX/Y/Z exactly.

fn block_x(i: i32) -> i32 {
    (i - 12) * 7 + 3
}
fn block_y(i: i32) -> i32 {
    i * 5 - 40
}
fn block_z(i: i32) -> i32 {
    (12 - i) * 11 - 3
}

/// Mirror of `VanillaOracle.density`, expressed as the datapack JSON our real
/// parser consumes. Every node type that does not need a registry.
fn density() {
    const CASES: [(&str, &str); 18] = [
        ("constant", r#"{"type":"minecraft:constant","argument":1.5}"#),
        (
            "y_clamped_gradient",
            r#"{"type":"minecraft:y_clamped_gradient","from_y":-64,"to_y":320,"from_value":2.0,"to_value":-3.0}"#,
        ),
        (
            "add",
            r#"{"type":"minecraft:add","argument1":{"type":"minecraft:constant","argument":1.0},"argument2":{"type":"minecraft:constant","argument":2.0}}"#,
        ),
        (
            "mul",
            r#"{"type":"minecraft:mul","argument1":{"type":"minecraft:constant","argument":3.0},"argument2":{"type":"minecraft:constant","argument":-4.0}}"#,
        ),
        (
            "min",
            r#"{"type":"minecraft:min","argument1":{"type":"minecraft:constant","argument":1.0},"argument2":{"type":"minecraft:constant","argument":2.0}}"#,
        ),
        (
            "max",
            r#"{"type":"minecraft:max","argument1":{"type":"minecraft:constant","argument":1.0},"argument2":{"type":"minecraft:constant","argument":2.0}}"#,
        ),
        (
            "abs",
            r#"{"type":"minecraft:abs","argument":{"type":"minecraft:constant","argument":-0.5}}"#,
        ),
        (
            "square",
            r#"{"type":"minecraft:square","argument":{"type":"minecraft:constant","argument":-0.5}}"#,
        ),
        (
            "cube",
            r#"{"type":"minecraft:cube","argument":{"type":"minecraft:constant","argument":-0.5}}"#,
        ),
        (
            "half_negative",
            r#"{"type":"minecraft:half_negative","argument":{"type":"minecraft:constant","argument":-0.5}}"#,
        ),
        (
            "quarter_negative",
            r#"{"type":"minecraft:quarter_negative","argument":{"type":"minecraft:constant","argument":-0.5}}"#,
        ),
        (
            "squeeze",
            r#"{"type":"minecraft:squeeze","argument":{"type":"minecraft:constant","argument":-0.5}}"#,
        ),
        (
            "abs_positive",
            r#"{"type":"minecraft:abs","argument":{"type":"minecraft:constant","argument":0.0}}"#,
        ),
        (
            "clamp",
            r#"{"type":"minecraft:clamp","input":{"type":"minecraft:constant","argument":5.0},"min":-1.0,"max":1.0}"#,
        ),
        (
            "flat_cache",
            r#"{"type":"minecraft:flat_cache","argument":{"type":"minecraft:constant","argument":2.5}}"#,
        ),
        (
            "cache_once",
            r#"{"type":"minecraft:cache_once","argument":{"type":"minecraft:constant","argument":2.5}}"#,
        ),
        (
            "cache_2d",
            r#"{"type":"minecraft:cache_2d","argument":{"type":"minecraft:constant","argument":2.5}}"#,
        ),
        (
            "nested_add_mul",
            r#"{"type":"minecraft:add",
                "argument1":{"type":"minecraft:y_clamped_gradient","from_y":-64,"to_y":320,"from_value":2.0,"to_value":-3.0},
                "argument2":{"type":"minecraft:mul",
                    "argument1":{"type":"minecraft:constant","argument":0.5},
                    "argument2":{"type":"minecraft:squeeze","argument":{"type":"minecraft:constant","argument":-0.5}}}}"#,
        ),
    ];

    for (name, json) in CASES {
        let mut builder = DensityBuilder::new();
        let f = builder.parse_str(json).unwrap_or_else(|e| panic!("{name}: {e}"));
        for i in 0..SAMPLES {
            emit(
                name,
                0,
                i,
                f.compute(
                    &[],
                    f64::from(block_x(i)),
                    f64::from(block_y(i)),
                    f64::from(block_z(i)),
                ),
            );
        }
    }
}

/// JSON-driven density parity: both sides consume the *same* file, so this
/// exercises the parser and the evaluator together.
///
/// Noise-bearing nodes are excluded here (they need a holder); those live in the
/// `density_noise` kind.
fn density_json(path: &str) {
    let text = std::fs::read_to_string(path).expect("could not read the case file");

    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, json) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("expected name<TAB>json, got: {line}"));

        let mut builder = DensityBuilder::new();
        let f = builder.parse_str(json).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The oracle initialises every noise holder with seed 0, so inline specs
        // resolve to the same instances here.
        let noises: Vec<NormalNoise> =
            builder.noise_specs().iter().map(NoiseSpec::build).collect();

        for i in 0..SAMPLES {
            emit(
                name,
                0,
                i,
                f.compute(
                    &noises,
                    f64::from(block_x(i)),
                    f64::from(block_y(i)),
                    f64::from(block_z(i)),
                ),
            );
        }
    }
}

/// Density nodes carrying noise. Must match `VanillaOracle.densityNoise`, which
/// hands each `NoiseHolder` the same `NormalNoise` this builds.
fn density_noise() {
    const SEED: u64 = 4242;

    const CASES: [(&str, &str); 7] = [
        (
            "noise_default",
            r#"{"type":"minecraft:noise","noise":"test:main"}"#,
        ),
        (
            "noise_scaled",
            r#"{"type":"minecraft:noise","noise":"test:main","xz_scale":0.25,"y_scale":0.0}"#,
        ),
        (
            "noise_xz_y",
            r#"{"type":"minecraft:noise","noise":"test:main","xz_scale":0.02,"y_scale":0.06}"#,
        ),
        (
            "shift_a",
            r#"{"type":"minecraft:shift_a","argument":"test:main"}"#,
        ),
        (
            "shift_b",
            r#"{"type":"minecraft:shift_b","argument":"test:main"}"#,
        ),
        (
            "shift",
            r#"{"type":"minecraft:shift","argument":"test:main"}"#,
        ),
        (
            "shifted_noise_2d",
            r#"{"type":"minecraft:shifted_noise","noise":"test:main",
                "xz_scale":0.25,"y_scale":0.0,
                "shift_x":{"type":"minecraft:shift_a","argument":"test:main"},
                "shift_y":{"type":"minecraft:constant","argument":0.0},
                "shift_z":{"type":"minecraft:shift_b","argument":"test:main"}}"#,
        ),
    ];

    for (name, json) in CASES {
        let mut builder = DensityBuilder::new();
        let f = builder
            .parse_str(json)
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        // One normal noise per referenced name, built exactly as the oracle does.
        // Must match VanillaOracle.densityNoise, which uses seed 4242 and a fixed
        // 4-octave parameter set for every noise.
        let noises: Vec<NormalNoise> = builder
            .noise_specs()
            .iter()
            .map(|_| {
                let mut rs = XoroshiroRandomSource::new(SEED);
                NormalNoise::new_modern(&mut rs, -7, &[1.0, 1.0, 1.0, 1.0])
            })
            .collect();

        for i in 0..SAMPLES {
            emit(
                name,
                0,
                i,
                f.compute(
                    &noises,
                    f64::from(block_x(i)),
                    f64::from(block_y(i)),
                    f64::from(block_z(i)),
                ),
            );
        }
    }
}

/// The **real** 1.21.1 overworld router, serialised to JSON by
/// `tools/oracle/RouterOracle.java` from Minecraft's own data generators.
///
/// This is the strongest check in the repo: the graph is not hand-written, it is
/// the one a real overworld uses, down to `router_final_density`.
///
/// Noise parameters arrive *inlined* (that is how `DIRECT_CODEC` writes them), so
/// both sides build the same `NormalNoise` with seed 0. That deliberately isolates
/// graph evaluation from seed derivation, which is still unverified.
fn density_router(path: &str) {
    let text = std::fs::read_to_string(path).expect("could not read the router corpus");

    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, json) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("expected name<TAB>json, got: {line}"));

        let mut builder = DensityBuilder::new();
        let f = builder
            .parse_str(json)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let noises: Vec<NormalNoise> =
            builder.noise_specs().iter().map(NoiseSpec::build).collect();

        for i in 0..SAMPLES {
            emit(
                name,
                0,
                i,
                f.compute(
                    &noises,
                    f64::from(block_x(i)),
                    f64::from(block_y(i)),
                    f64::from(block_z(i)),
                ),
            );
        }
    }
}

/// 6 x 6 x 10 grid of chunk-local-ish coordinates, matching `VanillaOracle`.
const DENSE_SAMPLES: i32 = 360;
const DENSE_X: [i32; 6] = [0, 1, 2, 3, 7, 15];
const DENSE_Y: [i32; 10] = [-64, -32, 0, 39, 43, 60, 63, 100, 200, 250];
const DENSE_Z: [i32; 6] = [0, 1, 2, 3, 7, 15];

/// `density_json`, but over a dense grid of chunk-local coordinates. The shared sparse
/// coordinates never land in the region a chunk generator walks, so a node type checked
/// only through them is checked only loosely.
fn density_dense(path: &str) {
    let text = std::fs::read_to_string(path).expect("could not read the case file");

    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, json) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("expected name<TAB>json, got: {line}"));

        let mut builder = DensityBuilder::new();
        let f = builder.parse_str(json).unwrap_or_else(|e| panic!("{name}: {e}"));
        let noises: Vec<NormalNoise> = builder
            .noise_specs()
            .iter()
            .map(NoiseSpec::build)
            .collect();

        for i in 0..DENSE_SAMPLES {
            let x = f64::from(DENSE_X[(i % 6) as usize]);
            let z = f64::from(DENSE_Z[((i / 6) % 6) as usize]);
            let y = f64::from(DENSE_Y[(i / 36) as usize]);
            emit(name, 0, i, f.compute(&noises, x, y, z));
        }
    }
}

/// The interpolation primitives the cell grid is built on. Must match
/// `VanillaOracle.math`.
fn math() {
    use mcgen_core::math::{lerp, lerp2, lerp3, smoothstep, wrap};

    fn clamped_lerp(start: f64, end: f64, delta: f64) -> f64 {
        if delta < 0.0 {
            start
        } else if delta > 1.0 {
            end
        } else {
            lerp(delta, start, end)
        }
    }

    for i in 0..24 {
        let fi = f64::from(i);
        let t = fi * 0.0416666;

        emit("math_lerp", 0, i, lerp(t, -1.5 + fi, 2.25 - fi));
        emit("math_smoothstep", 0, i, smoothstep(t));
        emit("math_clamped_lerp", 0, i, clamped_lerp(-3.0 + fi, 5.0 - fi, t));
        emit("math_wrap", 0, i, wrap(t * 4.0e7 - 2.0e7));

        let ax = fi * 0.037;
        let ay = fi * 0.061;
        let az = fi * 0.017;
        let d000 = 1.0 + fi;
        let d100 = -2.0 - fi;
        let d010 = 3.5 * fi;
        let d110 = 0.25 - fi;
        let d001 = -1.0 + 2.0 * fi;
        let d101 = 7.0 - fi;
        let d011 = -0.5 * fi;
        let d111 = 11.0 + fi;

        emit(
            "math_lerp3",
            0,
            i,
            lerp3(ax, ay, az, d000, d100, d010, d110, d001, d101, d011, d111),
        );
        emit("math_lerp2", 0, i, lerp2(ax, ay, d000, d100, d010, d110));
    }
}

/// One `(first_octave, amplitudes, name)` per noise occurrence, grouped by route, in the
/// order `RouterOracle column` walks each graph.
///
/// Names are needed because the encoded JSON keeps only parameters, and distinct
/// registries routinely share them.
fn noise_names() -> std::collections::HashMap<String, Vec<(i32, Vec<u64>, String)>> {
    let path = std::env::var("MCGEN_NOISE_NAMES").unwrap_or_else(|_| {
        panic!("set MCGEN_NOISE_NAMES to the file written by `RouterOracle column`")
    });
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {path}: {e}"));

    let mut map: std::collections::HashMap<String, Vec<(i32, Vec<u64>, String)>> =
        std::collections::HashMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.split(' ');
        let route = parts.next().expect("route").to_owned();
        let first_octave: i32 = parts.next().unwrap().parse().expect("firstOctave");
        let amplitudes: Vec<u64> = parts
            .next()
            .unwrap()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| u64::from_str_radix(s, 16).expect("amplitude bits"))
            .collect();
        let name = parts.next().expect("name").to_owned();
        map.entry(route)
            .or_default()
            .push((first_octave, amplitudes, name));
    }
    map
}

/// The world seed the oracle uses.
const COLUMN_SEED: u64 = 0;

/// Chunk-level parity: the raw density with `RandomState`'s own seeding, plus the
/// per-block solid/air decision taken from a real `NoiseChunk`.
/// Which slice of the chunk-level probe to emit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColumnPart {
    /// The `RandomState` derivation chain only. Bit-exact, and checked in CI.
    NoiseSeeds,
    /// The derivation chain plus one value per constructed `NormalNoise`.
    NoiseValues,
    /// Everything, including the cell grid. Still being brought into line.
    Everything,
}

/// Parse one router graph and build its noises with the real `RandomState` seeding.
/// Clamp bounds per route, in post-order, written by `RouterOracle column`.
fn clamp_bounds() -> std::collections::HashMap<String, Vec<(f64, f64)>> {
    let Ok(path) = std::env::var("MCGEN_CLAMPS") else {
        return std::collections::HashMap::new();
    };
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let mut map: std::collections::HashMap<String, Vec<(f64, f64)>> =
        std::collections::HashMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.split(' ');
        let route = parts.next().expect("route").to_owned();
        let min = f64::from_bits(u64::from_str_radix(parts.next().expect("min"), 16).expect("min"));
        let max = f64::from_bits(u64::from_str_radix(parts.next().expect("max"), 16).expect("max"));
        map.entry(route).or_default().push((min, max));
    }
    map
}

fn build_seeded(
    route: &str,
    json: &str,
    names: &std::collections::HashMap<String, Vec<(i32, Vec<u64>, String)>>,
    clamps: &std::collections::HashMap<String, Vec<(f64, f64)>>,
) -> (mcgen_core::density::DensityFunction, Vec<NormalNoise>) {
    let mut builder = DensityBuilder::new();
    // The router graphs are the ones `RandomState` re-seeds, so they need the world seed.
    builder.set_world_seed(COLUMN_SEED);
    builder.set_clamp_bounds(clamps.get(route).cloned().unwrap_or_default());
    builder.set_occurrences(
        names
            .get(route)
            .unwrap_or_else(|| panic!("no noise occurrences reported for {route}"))
            .clone(),
    );
    let density = builder
        .parse_str(json)
        .unwrap_or_else(|e| panic!("{route}: {e}"));

    let noises: Vec<NormalNoise> = builder
        .noise_specs()
        .iter()
        .map(|spec| {
            let NoiseSpec::Seeded { name, .. } = spec else {
                panic!("occurrence mode should produce Seeded specs, got {spec:?}");
            };
            spec.build_seeded(COLUMN_SEED, name)
        })
        .collect();
    (density, noises)
}

fn column(part: ColumnPart) {
    let names = noise_names();
    let clamps = clamp_bounds();

    let router_path = std::env::var("MCGEN_ROUTER_JSON")
        .expect("set MCGEN_ROUTER_JSON to the router corpus");
    let text = std::fs::read_to_string(&router_path)
        .unwrap_or_else(|e| panic!("could not read {router_path}: {e}"));

    let json_for = |name: &str| {
        text.lines()
            .find_map(|l| l.split_once('\t').filter(|(n, _)| *n == name))
            .map(|(_, j)| j)
            .unwrap_or_else(|| panic!("router.jsonl has no {name} line"))
    };

    // Distinct registry names with their parameters, sorted, so these probes do not
    // depend on traversal order.
    let mut by_name: std::collections::BTreeMap<&str, (i32, Vec<u64>)> =
        std::collections::BTreeMap::new();
    for occurrences in names.values() {
        for (first_octave, amplitudes, name) in occurrences {
            by_name
                .entry(name)
                .or_insert_with(|| (*first_octave, amplitudes.clone()));
        }
    }

    // (0) The derivation chain alone.
    // This loop emits `noise_seed` (the derivation chain), not `noise_value_rs`.
    // `noise_value_rs` is retired — see the note on the oracle side; the routes prove the
    // same thing more strongly.
    let mut seed_names: Vec<&str> = by_name.keys().copied().collect();
    seed_names.sort();
    for name in seed_names {
        let mut rs = XoroshiroRandomSource::new(COLUMN_SEED)
            .fork_positional()
            .from_hash_of(name);
        emit(
            "noise_seed",
            COLUMN_SEED as i64,
            java_string_hash_code(name),
            f64::from_bits(rs.next_long() as u64),
        );
    }
    if part == ColumnPart::NoiseSeeds {
        return;
    }

    // (0b) One value per noise, isolating construction from the graph.
    for (name, params) in &by_name {
        let (first_octave, amplitude_bits) = params;
        let amplitudes: Vec<f64> = amplitude_bits.iter().map(|b| f64::from_bits(*b)).collect();
        let mut rs = XoroshiroRandomSource::new(COLUMN_SEED)
            .fork_positional()
            .from_hash_of(name);
        let nn = NormalNoise::new_modern(&mut rs, *first_octave, &amplitudes);
        emit(
            "noise_value",
            COLUMN_SEED as i64,
            java_string_hash_code(name),
            nn.get(0.0, 0.0, 0.0),
        );
    }
    if part == ColumnPart::NoiseValues {
        return;
    }

    // The same, but only for the names `final_density` actually uses, because the oracle
    // can only ask `RandomState` about the keys it finds in that graph.
    let fd_names: std::collections::BTreeSet<&str> = names
        .get("router_final_density")
        .map(|o| o.iter().map(|(_, _, n)| n.as_str()).collect())
        .unwrap_or_default();
    for (name, params) in by_name.iter().filter(|(n, _)| fd_names.contains(*n)) {
        let (first_octave, amplitude_bits) = params;
        let amplitudes: Vec<f64> = amplitude_bits.iter().map(|b| f64::from_bits(*b)).collect();
        let mut rs = XoroshiroRandomSource::new(COLUMN_SEED)
            .fork_positional()
            .from_hash_of(name);
        let nn = NormalNoise::new_modern(&mut rs, *first_octave, &amplitudes);
        emit(
            "noise_value_rs",
            COLUMN_SEED as i64,
            java_string_hash_code(name),
            nn.get(0.0, 0.0, 0.0),
        );
    }

    // (0c) The order this parser first meets each distinct noise in `final_density`,
    // compared against the oracle's own walk.
    {
        let mut builder = DensityBuilder::new();
        builder.set_occurrences(
            names
                .get("router_final_density")
                .expect("no occurrences for final_density")
                .clone(),
        );
        builder
            .parse_str(json_for("router_final_density"))
            .expect("final_density");
        let mut index = 0;
        // Every occurrence, not just the first sight of each name: two registries that
        // share parameters are only distinguishable here.
        for name in builder.consumed_occurrences() {
            emit(
                "fd_occ_order",
                COLUMN_SEED as i64,
                index,
                f64::from(java_string_hash_code(name)),
            );
            index += 1;
        }
    }

    const ROUTES: [&str; 15] = [
        "router_temperature",
        "router_vegetation",
        "router_continents",
        "router_erosion",
        "router_depth",
        "router_ridges",
        "router_barrier",
        "router_fluid_floodedness",
        "router_fluid_spread",
        "router_lava",
        "router_vein_toggle",
        "router_vein_ridged",
        "router_vein_gap",
        "router_initial_density",
        "router_final_density",
    ];
    let built: Vec<(&str, mcgen_core::density::DensityFunction, Vec<NormalNoise>)> = ROUTES
        .iter()
        .map(|r| {
            let (f, n) = build_seeded(r, json_for(r), &names, &clamps);
            (*r, f, n)
        })
        .collect();

    // (0d) Every route at the original sparse coordinates.
    for (route, f, n) in &built {
        for i in 0..SAMPLES {
            emit(
                route,
                COLUMN_SEED as i64,
                i,
                f.compute(
                    n,
                    f64::from(block_x(i)),
                    f64::from(block_y(i)),
                    f64::from(block_z(i)),
                ),
            );
        }
    }

    // (0e) Every route at chunk-local coordinates. Whichever route diverges first
    // localises the fault without guesswork.
    let heights = mcgen_core::terrain::HEIGHT;
    // The corner and quart caches are per chunk. `fill_chunk_chunked` announces the boundary
    // itself; this loop walks a chunk by hand, so it has to as well.
    mcgen_core::density::clear_interpolation_cache();
    let mut column_index = 0;
    for lx in 0usize..16 {
        for lz in 0usize..16 {
            for local_y in 0..heights {
                let wy = mcgen_core::terrain::MIN_Y + local_y as i32;
                let index = (column_index * heights + local_y) as i32;
                let (x, y, z) = (lx as f64, f64::from(wy), lz as f64);
                for (route, f, n) in &built {
                    emit(
                        &format!("rawcol_{route}"),
                        COLUMN_SEED as i64,
                        index,
                        f.compute(n, x, y, z),
                    );
                }
            }
            column_index += 1;
        }
    }

    // (1) The cell grid, through vanilla's own interpolation layout.
    let (_, final_density, final_noises) = built
        .iter()
        .find(|(r, _, _)| *r == "router_final_density")
        .expect("final_density");
    let wg = mcgen_core::terrain::WorldGen {
        density: final_density,
        noises: final_noises,
    };
    let grid = wg.sample_cells(0, 0);

    // Time the chunked evaluation on its own. This is the number to compare against a
    // generator: 98,304 block evaluations of the real router, one chunk's worth.
    let timed_blocks = 16 * 16 * heights as usize;

    // The flattened evaluator must agree with the tree on every input, and be worth having.


    // Cell order, matching `fill_chunk_chunked`. With `z, x, y` order the corner caches are
    // emptied every eight blocks and the measurement describes something the fill never does.
    let cell_order = |sink: &mut f64| {
        for cell_z in 0..mcgen_core::terrain::CHUNK_SIZE / mcgen_core::terrain::CELL_WIDTH {
            for cell_x in 0..mcgen_core::terrain::CHUNK_SIZE / mcgen_core::terrain::CELL_WIDTH {
                for cell_y in 0..heights / mcgen_core::terrain::CELL_HEIGHT {
                    for dy in 0..mcgen_core::terrain::CELL_HEIGHT {
                        let y = cell_y * mcgen_core::terrain::CELL_HEIGHT + dy;
                        let wy = mcgen_core::terrain::MIN_Y + y as i32;
                        for dz in 0..mcgen_core::terrain::CELL_WIDTH {
                            let z = cell_z * mcgen_core::terrain::CELL_WIDTH + dz;
                            for dx in 0..mcgen_core::terrain::CELL_WIDTH {
                                let x = cell_x * mcgen_core::terrain::CELL_WIDTH + dx;
                                *sink += final_density.compute_chunked(
                                    final_noises,
                                    x as f64,
                                    f64::from(wy),
                                    z as f64,
                                );
                            }
                        }
                    }
                }
            }
        }
    };

    let t0 = std::time::Instant::now();
    {
        let mut sink = 0.0f64;
        cell_order(&mut sink);
        std::hint::black_box(sink);
    }
    let ns_per = t0.elapsed().as_nanos() as f64 / timed_blocks as f64;
    eprintln!(
        "RUST chunked: {timed_blocks} evals in {} ms ({ns_per:.1} ns/eval)",
        t0.elapsed().as_millis()
    );

    // How many nodes does a block actually touch? Without this, "30 ns per node visit" is a
    // guess.
    let mut tally_out = [0u64; 24];
    let visits = mcgen_core::density::with_visit_counting(|| {
        let mut sink = 0.0f64;
        cell_order(&mut sink);
        std::hint::black_box(sink);
        tally_out = mcgen_core::density::take_tally();
        mcgen_core::density::take_visits()
    });
    let mut ranked: Vec<(usize, u64)> = tally_out.iter().copied().enumerate().collect();
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let total: u64 = tally_out.iter().sum();
    for (id, n) in ranked.iter().take(24) {
        if *n > 0 {
            eprintln!(
                "RUST tally: {:<18} {:>10} ({:>5.2}%)",
                mcgen_core::density::TALLY_NAMES[*id],
                n,
                100.0 * *n as f64 / total as f64
            );
        }
    }
    let noise_stats = mcgen_core::density::take_noise_stats();
    let _ = &noise_stats;
    eprintln!(
        "RUST noise memo: {} hits, {} misses ({:.1}% hit rate)",
        noise_stats[0],
        noise_stats[1],
        100.0 * noise_stats[0] as f64 / (noise_stats[0] + noise_stats[1]).max(1) as f64
    );
    eprintln!(
        "RUST visits: {visits} for {timed_blocks} blocks = {:.1} node visits/block ({:.1} ns/visit)",
        visits as f64 / timed_blocks as f64,
        ns_per / (visits as f64 / timed_blocks as f64)
    );


    let mut column_index = 0;
    for lx in 0usize..16 {
        for lz in 0usize..16 {
            for local_y in 0..heights {
                // The real chunked value, with `interpolated` nodes doing their own corner
                // lerp. `grid.interpolate` was a different algorithm: it coarse-sampled the
                // whole density instead of only the subtrees vanilla marks.
                let wy = mcgen_core::terrain::MIN_Y + local_y as i32;
                let d = final_density.compute_chunked(
                    final_noises,
                    lx as f64,
                    f64::from(wy),
                    lz as f64,
                );
                let solid = d > 0.0;
                let index = (column_index * heights + local_y) * 2 + usize::from(solid);
                emit(
                    "column",
                    COLUMN_SEED as i64,
                    index as i32,
                    if solid { 1.0 } else { 0.0 },
                );
            }
            column_index += 1;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Kinds, plus any per-kind argument (e.g. the shared JSON corpus path).
    let mut kinds: Vec<String> = args.iter().skip(1).cloned().collect();
    if kinds.is_empty() {
        kinds = vec![
            "improved_noise".to_owned(),
            "xoroshiro".to_owned(),
            "normal_noise".to_owned(),
            "hash".to_owned(),
            "density".to_owned(),
            "density_noise".to_owned(),
        ];
    }

    let mut i = 0;
    while i < kinds.len() {
        let kind = kinds[i].clone();
        match kind.as_str() {
            "improved_noise" => improved_noise(),
            "xoroshiro" => xoroshiro(),
            "normal_noise" => normal_noise(),
            "hash" => hash_strings(),
            "math" => math(),
            "column" => column(ColumnPart::Everything),
            "noise_seed" => column(ColumnPart::NoiseSeeds),
            "noise_value" => column(ColumnPart::NoiseValues),
            "density" => density(),
            "density_noise" => density_noise(),
            "density_router" => {
                let path = kinds
                    .get(i + 1)
                    .unwrap_or_else(|| panic!("density_router needs a corpus path"));
                density_router(path);
                i += 1; // consumed the path
            }
            "density_json" => {
                let path = kinds
                    .get(i + 1)
                    .unwrap_or_else(|| panic!("density_json needs a case-file path"));
                density_json(path);
                i += 1; // consumed the path
            }
            "density_dense" => {
                let path = kinds
                    .get(i + 1)
                    .unwrap_or_else(|| panic!("density_dense needs a case-file path"));
                density_dense(path);
                i += 1; // consumed the path
            }
            other => {
                eprintln!("unknown kind: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
}
