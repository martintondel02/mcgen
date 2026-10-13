//! Data-driven density functions.
//!
//! In 1.21 the terrain shape is not hard-coded: it is a graph of density
//! functions loaded from datapack JSON (`noise_settings`, `density_function`
//! registries, and whatever mods add on top). Evaluating that graph is the
//! "data-driven" half of a worldgen accelerator — if this were hard-coded to
//! vanilla, every modded pack would generate wrong terrain.
//!
//! This module parses Minecraft's density-function JSON into a tree and
//! evaluates it. Unsupported node types are a hard error rather than being
//! silently approximated, so a partial port fails loudly instead of quietly
//! producing wrong worlds.

use serde_json::Value;
use std::collections::BTreeMap;

use crate::noise::{BlendedNoise, NormalNoise};
use crate::random::XoroshiroRandomSource;

#[derive(Debug)]
pub enum DensityError {
    NotAnObject,
    MissingType,
    UnknownType(String),
    MissingField(&'static str),
    NotANumber(String),
    Unsupported(String),
}

impl std::fmt::Display for DensityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnObject => write!(f, "density function must be a JSON object"),
            Self::MissingType => write!(f, "density function is missing a \"type\" field"),
            Self::UnknownType(t) => write!(f, "unknown density function type: {t}"),
            Self::MissingField(name) => write!(f, "missing required field: {name}"),
            Self::NotANumber(name) => write!(f, "field {name} is not a number"),
            Self::Unsupported(t) => write!(
                f,
                "density function type {t} is not implemented yet \
                 (refusing to guess, as that would generate wrong terrain)"
            ),
        }
    }
}

impl std::error::Error for DensityError {}

/// How a `minecraft:noise` node identifies its noise.
///
/// Vanilla has two encodings, and which one you get depends on the codec:
/// `DIRECT_CODEC` (no registry) inlines the parameters, while the registry-aware
/// codec emits a name reference. Real datapacks use names; the oracle's
/// serialisation of the built-in overworld router uses inline parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum NoiseSpec {
    /// A registry reference, e.g. `"minecraft:temperature"`.
    Named(String),
    /// Parameters inlined by `DIRECT_CODEC`, e.g.
    /// `{"firstOctave":-8,"amplitudes":[1.0]}`.
    Inline {
        first_octave: i32,
        amplitudes: Vec<f64>,
    },
    /// A noise identified by registry name, with its parameters carried alongside.
    ///
    /// The encoded JSON keeps only parameters, and distinct registries routinely share
    /// them — `final_density` collapses 25 noises into 12 parameter sets, with one set
    /// shared by eight different registries. So parameters alone cannot identify a
    /// noise; the name has to come from outside, and this variant pairs the two.
    Seeded {
        name: String,
        first_octave: i32,
        amplitudes: Vec<f64>,
    },
}

impl NoiseSpec {
    /// Build the `NormalNoise` this spec describes.
    ///
    /// The seed rule must match the oracle:
    ///  * named noises derive the seed from the name (vanilla's actual rule is
    ///    more involved and is tracked separately as unverified);
    ///  * inline noises use seed 0, which is what `RouterOracle` does so that
    ///    graph evaluation can be tested independently of seed derivation.
    #[must_use]
    pub fn build(&self) -> NormalNoise {
        let (seed, first_octave, amplitudes) = match self {
            Self::Named(name) => {
                let (lo, _hi) = crate::random::seed_from_hash_of(name);
                (lo, -7, vec![1.0, 1.0, 1.0, 1.0])
            }
            Self::Inline {
                first_octave,
                amplitudes,
            } => (0, *first_octave, amplitudes.clone()),
            Self::Seeded {
                first_octave,
                amplitudes,
                ..
            } => (0, *first_octave, amplitudes.clone()),
        };
        let mut rs = XoroshiroRandomSource::new(seed);
        NormalNoise::new_modern(&mut rs, first_octave, &amplitudes)
    }

    /// Build with the `RandomState` rule, which is what a real chunk uses:
    ///
    /// ```text
    /// NormalNoise.create(source.forkPositional().fromHashOf(location), params)
    /// ```
    ///
    /// The encoded JSON only carries `{firstOctave, amplitudes}`, so the registry
    /// location is lost by the codec. The oracle emits the names in traversal order
    /// and this takes the matching one.
    ///
    /// # Panics
    /// If the spec is a `Named` reference, whose parameters are not carried here.
    #[must_use]
    pub fn build_seeded(&self, world_seed: u64, name: &str) -> NormalNoise {
        let (first_octave, amplitudes) = match self {
            Self::Inline {
                first_octave,
                amplitudes,
            }
            | Self::Seeded {
                first_octave,
                amplitudes,
                ..
            } => (first_octave, amplitudes),
            Self::Named(_) => {
                panic!("build_seeded needs parameters alongside the name, got {self:?}")
            }
        };
        // `RandomState` builds `<algorithm>.newInstance(seed).forkPositional()` and
        // hashes the location into it. The overworld's algorithm is XOROSHIRO — not,
        // as its age suggests, LEGACY. (The legacy factory is kept in `random` because
        // other presets do use it, and its rule is `String.hashCode() ^ seed`.)
        let mut rs = XoroshiroRandomSource::new(world_seed)
            .fork_positional()
            .from_hash_of(name);
        NormalNoise::new_modern(&mut rs, *first_octave, amplitudes)
    }
}

/// `WeirdScaledSampler`'s rarity mapper. Values confirmed from bytecode
/// (`QuantizedSpaghettiRarity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RarityMapper {
    Type1,
    Type2,
}

impl RarityMapper {
    #[must_use]
    pub fn rarity(self, d: f64) -> f64 {
        match self {
            // getSpaghettiRarity3D
            Self::Type1 => {
                if d < -0.5 {
                    0.75
                } else if d < 0.0 {
                    1.0
                } else if d < 0.5 {
                    1.5
                } else {
                    2.0
                }
            }
            // getSphaghettiRarity2D
            Self::Type2 => {
                if d < -0.75 {
                    0.5
                } else if d < -0.5 {
                    0.75
                } else if d < 0.5 {
                    1.0
                } else if d < 0.75 {
                    2.0
                } else {
                    3.0
                }
            }
        }
    }
}

/// One point's value: a constant, or a further spline.
#[derive(Debug, Clone)]
pub enum SplineValue {
    Constant(f32),
    Nested(Box<CubicSpline>),
}

/// Vanilla's `CubicSpline`: piecewise cubic Hermite interpolation over a sorted
/// set of sample points, driven by a coordinate function.
///
/// All arithmetic is `f32`. Confirmed from bytecode: `Multipoint.apply` returns
/// `float`, the derivatives are `float[]`, and the blend uses `Mth.lerp:(FFF)F`.
#[derive(Debug, Clone)]
pub struct CubicSpline {
    coordinate: Box<DensityFunction>,
    locations: Vec<f32>,
    values: Vec<SplineValue>,
    derivatives: Vec<f32>,
}

#[inline]
fn lerp_f32(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// Vanilla's `Mth.binarySearch(0, n, i -> f < locations[i]) - 1`: the index of the
/// last sample point at or below `f`, or `-1` when `f` is below all of them.
fn find_interval_start(locations: &[f32], f: f32) -> i32 {
    let mut start = 0usize;
    let mut len = locations.len();
    while len > 0 {
        let half = len / 2;
        let mid = start + half;
        if f < locations[mid] {
            len = half;
        } else {
            start = mid + 1;
            len -= half + 1;
        }
    }
    start as i32 - 1
}

/// Vanilla's `CubicSpline.linearExtend`: continue along the end derivative.
#[inline]
fn linear_extend(f: f32, locations: &[f32], value: f32, derivatives: &[f32], index: usize) -> f32 {
    let d = derivatives[index];
    if d == 0.0 {
        value
    } else {
        value + d * (f - locations[index])
    }
}

impl CubicSpline {
    fn value_at(&self, v: &SplineValue, noises: &[NormalNoise], x: f64, y: f64, z: f64) -> f32 {
        match v {
            SplineValue::Constant(c) => *c,
            SplineValue::Nested(s) => s.apply(noises, x, y, z),
        }
    }

    /// Evaluate at a position. Mirrors vanilla's branch structure exactly.
    #[must_use]
    pub fn apply(&self, noises: &[NormalNoise], x: f64, y: f64, z: f64) -> f32 {
        let coord = self.coordinate.compute(noises, x, y, z) as f32;
        let idx = find_interval_start(&self.locations, coord);
        let last = self.locations.len() - 1;

        if idx < 0 {
            let v = self.value_at(&self.values[0], noises, x, y, z);
            return linear_extend(coord, &self.locations, v, &self.derivatives, 0);
        }
        if idx as usize == last {
            let v = self.value_at(&self.values[last], noises, x, y, z);
            return linear_extend(coord, &self.locations, v, &self.derivatives, last);
        }

        let i = idx as usize;
        let g = self.locations[i];
        let h = self.locations[i + 1];
        let t = (coord - g) / (h - g);

        let y0 = self.value_at(&self.values[i], noises, x, y, z);
        let y1 = self.value_at(&self.values[i + 1], noises, x, y, z);
        let d0 = self.derivatives[i];
        let d1 = self.derivatives[i + 1];

        let a = d0 * (h - g) - (y1 - y0);
        let b = -d1 * (h - g) + (y1 - y0);

        lerp_f32(t, y0, y1) + t * (1.0 - t) * lerp_f32(t, a, b)
    }

    #[must_use]
    pub fn point_count(&self) -> usize {
        self.locations.len()
    }
}

thread_local! {
    /// Whether `interpolated` markers should behave as `NoiseInterpolator`s; see `chunked`.
    static CHUNKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };

    /// Whether to count node visits; see `with_visit_counting`.
    static VISITS_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };


    /// The eight corner samples of each `interpolated` node, for the cell currently being
    /// walked.
    ///
    /// Only ever holds the handful of `interpolated` nodes in one graph, and is emptied whenever
    /// the cell changes — which is the same lifetime vanilla gives a `NoiseInterpolator`'s
    /// materialised corners. A map keyed by the cell as well was tried first and was worse than
    /// useless: it was looked up once per node per block, 491,520 times for a chunk, to serve
    /// 23,424 distinct cells.
    /// Indexed by slot, and `false` whenever the cell has changed. The earlier version stored
    /// `(node pointer, corners)` pairs and scanned them, which walked six cache lines per lookup
    /// on the hottest path in the project; a slot index is a plain array subscript.
    static CORNERS: std::cell::RefCell<(Vec<[f64; 8]>, bool)> =
        std::cell::RefCell::new((Vec::new(), false));

    /// The quart-corner value of each `flat_cache` / `cache_2d` node, for the chunk being walked.
    ///
    /// `NoiseChunk.FlatCache` keeps a 5 x 5 `double[][]` for the chunk and serves
    /// `values[quartX - firstNoiseX][quartZ - firstNoiseZ]`, so the subtree below the marker is
    /// evaluated 25 times per chunk rather than once per block. Keyed by node address and the
    /// corner's world `(x, z)`; cleared per chunk. A direct-indexed slot array was written to
    /// replace this and was no faster, so this stays.
    static QUART_CACHE: std::cell::RefCell<std::collections::HashMap<(usize, i64, i64), f64>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    /// Noise samples for the chunk being walked, keyed by noise index and the exact `(x, y, z)`.
    ///
    /// A noise is a pure function of its coordinates, and the walk asks for the same coordinates
    /// over and over: the cell corners behind the `interpolated` markers are shared by up to eight
    /// neighbouring cells, and a chunk has only about 1,225 distinct corner positions against
    /// 86,849 noise samples taken. Vanilla does not memoise these and re-evaluates every time,
    /// which is why this is worth doing at all.
    static NOISE_CACHE: std::cell::RefCell<
        std::collections::HashMap<(u32, u64, u64, u64), f64>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());

    /// Noise memo hits and misses, for the diagnostic that asks whether more is recoverable.
    static NOISE_HITS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0; 2]) };

    /// The cell `CORNERS` is currently holding.
    static CORNER_CELL: std::cell::Cell<(i64, i64, i64)> = const { std::cell::Cell::new((i64::MIN, 0, 0)) };

    /// Diagnostic: node visits, so a per-node cost can be worked out instead of guessed at.
    static VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Per-variant visit counts.
    static TALLY: std::cell::Cell<[u64; 24]> = const { std::cell::Cell::new([0; 24]) };


}

/// Turn visit counting on for the duration of `f`.
pub fn with_visit_counting<T>(f: impl FnOnce() -> T) -> T {
    VISITS_ENABLED.with(|c| c.set(true));
    let out = f();
    VISITS_ENABLED.with(|c| c.set(false));
    out
}

/// Whether `interpolated` markers should behave as `NoiseInterpolator`s. See
/// [`DensityFunction::compute_chunked`].
///
/// **Thread-local, and it has to be.** A plain `static` is faster to read and was tried, but the
/// chunked reading is a property of one traversal, and the fill runs one traversal per chunk
/// across a rayon pool — so a global flag lets one thread's chunked walk leak into another
/// thread's raw one. Three FFI tests failed within seconds of the change, and the saving was zero.
#[inline]
fn chunked() -> bool {
    CHUNKED.with(std::cell::Cell::get)
}

/// A small integer per variant, so a diagnostic can say which nodes dominate the walk.
#[must_use]
pub fn tally(self_: &DensityFunction) -> usize {
    match self_ {
        DensityFunction::Constant(_) => 0,
        DensityFunction::YClampedGradient { .. } => 1,
        DensityFunction::Noise { .. } => 2,
        DensityFunction::ShiftedNoise(_) => 3,
        DensityFunction::Shift { .. } => 4,
        DensityFunction::Add(..) => 5,
        DensityFunction::Mul(..) => 6,
        DensityFunction::Min(..) => 7,
        DensityFunction::Max(..) => 8,
        DensityFunction::Clamp { .. } => 9,
        DensityFunction::AbsWrapped(_) => 10,
        DensityFunction::Square(_) => 11,
        DensityFunction::Cube(_) => 12,
        DensityFunction::HalfNegative(_) => 13,
        DensityFunction::QuarterNegative(_) => 14,
        DensityFunction::Squeeze(_) => 15,
        DensityFunction::Invert(_) => 16,
        DensityFunction::Spline(_) => 17,
        DensityFunction::OldBlendedNoise(_) => 18,
        DensityFunction::WeirdScaledSampler { .. } => 19,
        DensityFunction::RangeChoice(_) => 20,
        DensityFunction::PassThrough(_) => 21,
        DensityFunction::QuartSnapped(_) => 22,
        DensityFunction::Interpolated { .. } => 23,
    }
}

/// Names matching [`tally`].
pub const TALLY_NAMES: [&str; 24] = [
    "Constant", "YClampedGradient", "Noise", "ShiftedNoise", "Shift", "Add", "Mul", "Min",
    "Max", "Clamp", "AbsWrapped", "Square", "Cube", "HalfNegative", "QuarterNegative",
    "Squeeze", "Invert", "Spline", "OldBlendedNoise", "WeirdScaledSampler", "RangeChoice",
    "PassThrough", "QuartSnapped", "Interpolated",
];

/// Read and reset the node-visit counter.
/// Read and reset the noise-memo hit/miss counts.
pub fn take_noise_stats() -> [u64; 2] {
    NOISE_HITS.with(|c| c.replace([0; 2]))
}

pub fn take_visits() -> u64 {
    VISITS.with(|v| v.replace(0))
}

/// Count one node visit. Diagnostic only; it is a thread-local increment, so it perturbs the
/// thing it measures, but it answers "how many nodes does a block touch" which is not otherwise
/// observable.
pub fn count_visit() {
    VISITS.with(|v| v.set(v.get() + 1));
}

/// Per-variant visit counts, for diagnosing where a walk's time goes.
pub fn take_tally() -> [u64; 24] {
    TALLY.with(|t| t.replace([0; 24]))
}

pub fn bump_tally(id: usize) {
    TALLY.with(|t| {
        let mut v = t.get();
        v[id] += 1;
        t.set(v);
    });
}

/// Drop the per-cell corner cache. Cheap, and called once per chunk so a fill never inherits
/// another chunk's corners.
pub fn clear_interpolation_cache() {
    CORNERS.with(|c| c.borrow_mut().1 = false);
    QUART_CACHE.with(|c| c.borrow_mut().clear());
    NOISE_CACHE.with(|c| c.borrow_mut().clear());
}

/// Which of the three shift noises a `Shift` node uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKind {
    Shift,
    ShiftA,
    ShiftB,
}

/// The payload of a `RangeChoice`.
///
/// A separate struct only so the variant can be boxed: `DensityFunction` is sized to its widest
/// variant, and this one has five boxed children and two `f64`s. Unboxed it made *every* node in
/// the graph 56 bytes, including a `Constant`, which needs 16.
#[derive(Debug, Clone)]
pub struct RangeChoiceData {
    pub input: Box<DensityFunction>,
    pub min_inclusive: f64,
    pub max_exclusive: f64,
    pub when_in_range: Box<DensityFunction>,
    pub when_out_of_range: Box<DensityFunction>,
}

/// The payload of a `ShiftedNoise`. See [`RangeChoiceData`] for why it is separate.
#[derive(Debug, Clone)]
pub struct ShiftedNoiseData {
    pub noise: usize,
    pub xz_scale: f64,
    pub y_scale: f64,
    pub shift_x: Box<DensityFunction>,
    pub shift_y: Box<DensityFunction>,
    pub shift_z: Box<DensityFunction>,
}

/// A node in the density-function graph.
#[derive(Debug, Clone)]
pub enum DensityFunction {
    Constant(f64),
    YClampedGradient {
        from_y: f64,
        to_y: f64,
        from_value: f64,
        to_value: f64,
    },
    Noise {
        noise: usize,
        xz_scale: f64,
        y_scale: f64,
    },
    ShiftedNoise(Box<ShiftedNoiseData>),
    Shift {
        kind: ShiftKind,
        noise: usize,
    },
    Add(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
    Min(Box<Self>, Box<Self>),
    Max(Box<Self>, Box<Self>),
    Clamp {
        input: Box<Self>,
        min: f64,
        max: f64,
    },
    AbsWrapped(Box<Self>),
    Square(Box<Self>),
    Cube(Box<Self>),
    HalfNegative(Box<Self>),
    QuarterNegative(Box<Self>),
    Squeeze(Box<Self>),
    Invert(Box<Self>),
    Spline(Box<CubicSpline>),
    OldBlendedNoise(Box<BlendedNoise>),
    WeirdScaledSampler {
        input: Box<Self>,
        noise: usize,
        rarity: RarityMapper,
    },
    RangeChoice(Box<RangeChoiceData>),
    /// The `Marker` types `NoiseChunk` wraps in caches. In the graph they are plain
    /// passthroughs; the caching is a property of the chunk, not of the node.
    PassThrough(Box<Self>),
    /// `minecraft:flat_cache` / `minecraft:cache_2d`, read at the quart corner and memoised.
    QuartSnapped(Box<Self>),
    /// `minecraft:interpolated` — where the coarse 4 x 8 x 4 corner sampling happens.
    ///
    /// `slot` indexes the current cell's corner cache. It is assigned at parse time so the lookup
    /// is an array index rather than a scan over `(node, corners)` pairs; that scan, plus the four
    /// thread-local accesses around it, was the single hottest thing in the evaluator.
    Interpolated { input: Box<Self>, slot: u32 },
}

/// Builds a [`DensityFunction`] from Minecraft's JSON, and remembers what it needed.
///
/// The graph is data-driven, so the builder also collects the things the encoding leaves
/// out: which noises the graph references (in the order it meets them) and the clamp bounds,
/// which vanilla recomputes from a static range rather than storing.
pub struct DensityBuilder {
    /// Every noise the graph references, in first-seen order.
    noise_specs: Vec<NoiseSpec>,
    /// Deduplication for the above, keyed by a canonical description of the spec.
    noise_index: BTreeMap<String, usize>,
    /// One `(first_octave, amplitudes, registry name)` per noise occurrence, and a cursor.
    occurrences: Option<(Vec<(i32, Vec<u64>, String)>, usize)>,
    /// The names taken, in order, so the association can be checked occurrence by occurrence.
    consumed: Vec<String>,
    /// Clamp bounds in post-order, and a cursor.
    clamp_bounds: Option<(Vec<(f64, f64)>, usize)>,
    /// The world seed, when the graph being parsed is a real router.
    world_seed: Option<u64>,
    /// How many `interpolated` markers have been seen, so each can own a corner-cache slot.
    interp_slots: u32,
}

impl DensityBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            noise_specs: Vec::new(),
            noise_index: BTreeMap::new(),
            occurrences: None,
            consumed: Vec::new(),
            clamp_bounds: None,
            world_seed: None,
            interp_slots: 0,
        }
    }
}

impl Default for DensityBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DensityBuilder {
    /// Tell the builder which world seed the graph belongs to, so the nodes `RandomState`
    /// re-seeds can be reconstructed. Without this, `old_blended_noise` falls back to the
    /// codec's own seed-0 form.
    pub fn set_world_seed(&mut self, seed: u64) {
        self.world_seed = Some(seed);
    }

    /// The `RandomSource` `RandomState` gives a node seeded from `name`: the same positional
    /// factory the noises use, hashed against `name`.
    fn positional_source(&self, name: &str) -> Option<XoroshiroRandomSource> {
        let seed = self.world_seed?;
        Some(
            XoroshiroRandomSource::new(seed)
                .fork_positional()
                .from_hash_of(name),
        )
    }

    /// Supply one `(min, max)` per `clamp` node, in post-order — the order `mapAll` visits
    /// them, which is also the order this parser finishes each clamp.
    pub fn set_clamp_bounds(&mut self, bounds: Vec<(f64, f64)>) {
        self.clamp_bounds = Some((bounds, 0));
    }

    /// Supply one `(first_octave, amplitudes, registry name)` per noise occurrence, in
    /// the order the oracle walks the graph.
    ///
    /// Every subsequent noise descriptor consumes the next entry and must agree with it
    /// on the parameters, so a traversal-order mismatch fails loudly instead of
    /// silently attaching the wrong noise.
    pub fn set_occurrences(&mut self, occurrences: Vec<(i32, Vec<u64>, String)>) {
        self.occurrences = Some((occurrences, 0));
    }

    /// Every noise the graph references, in first-seen order. Build one
    /// `NormalNoise` per entry and pass them to `compute`.
    #[must_use]
    pub fn noise_specs(&self) -> &[NoiseSpec] {
        &self.noise_specs
    }

    /// The registry names consumed in occurrence mode, in order.
    #[must_use]
    pub fn consumed_occurrences(&self) -> &[String] {
        &self.consumed
    }

    /// Names of the *registry-referenced* noises, in order. Kept for callers that
    /// only ever see the named encoding.
    #[must_use]
    pub fn noise_names(&self) -> Vec<String> {
        self.noise_specs
            .iter()
            .filter_map(|s| match s {
                NoiseSpec::Named(n) => Some(n.clone()),
                NoiseSpec::Seeded { name, .. } => Some(name.clone()),
                NoiseSpec::Inline { .. } => None,
            })
            .collect()
    }

    /// Intern a spec, de-duplicating so repeated references share one instance.
    fn intern_spec(&mut self, spec: NoiseSpec) -> usize {
        let key = match &spec {
            NoiseSpec::Named(n) => format!("named:{n}"),
            NoiseSpec::Inline {
                first_octave,
                amplitudes,
            } => format!("inline:{first_octave}:{amplitudes:?}"),
            NoiseSpec::Seeded { name, .. } => format!("seeded:{name}"),
        }
        .to_ascii_lowercase();
        if let Some(&i) = self.noise_index.get(&key) {
            return i;
        }
        let idx = self.noise_specs.len();
        self.noise_specs.push(spec);
        self.noise_index.insert(key, idx);
        idx
    }

    /// `minecraft:noise` names its noise via `noise`; the shift nodes use
    /// `argument`. Either way the value is a registry id or an inline parameter
    /// object, and both encodings turn up in real graphs.
    fn noise_ref(&mut self, v: &Value) -> Result<usize, DensityError> {
        let descriptor = v.get("noise").ok_or(DensityError::MissingField("noise"))?;
        self.noise_descriptor(descriptor)
    }

    /// Intern the noise described by `descriptor`: a registry name string, or an
    /// inline `{"firstOctave":…,"amplitudes":[…]}` object.
    fn noise_descriptor(&mut self, descriptor: &Value) -> Result<usize, DensityError> {
        if let Some(name) = descriptor.as_str() {
            return Ok(self.intern_spec(NoiseSpec::Named(name.to_owned())));
        }

        let obj = descriptor
            .as_object()
            .ok_or(DensityError::MissingField("noise"))?;
        let first_octave = obj
            .get("firstOctave")
            .and_then(Value::as_i64)
            .ok_or(DensityError::MissingField("firstOctave"))? as i32;

        // `amplitudes` is normally an array but vanilla's codec also accepts a
        // bare number for the single-amplitude case.
        let amplitudes = match obj.get("amplitudes") {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_f64).collect(),
            Some(v) => vec![v.as_f64().ok_or(DensityError::MissingField("amplitudes"))?],
            None => return Err(DensityError::MissingField("amplitudes")),
        };

        if let Some((entries, next)) = self.occurrences.as_mut() {
            let (want_octave, want_bits, name) = entries.get(*next).unwrap_or_else(|| {
                panic!("the graph has more noise occurrences than the oracle reported")
            });
            let got_bits: Vec<u64> = amplitudes.iter().map(|a| a.to_bits()).collect();
            assert!(
                *want_octave == first_octave && *want_bits == got_bits,
                "noise occurrence {next} is out of step with the oracle: \
                 encoded ({first_octave}, {got_bits:x?}) vs oracle ({want_octave}, {want_bits:x?}). \
                 The traversal orders disagree, so names cannot be attached safely."
            );
            let name = name.clone();
            *next += 1;
            self.consumed.push(name.clone());
            return Ok(self.intern_spec(NoiseSpec::Seeded {
                name,
                first_octave,
                amplitudes,
            }));
        }

        Ok(self.intern_spec(NoiseSpec::Inline {
            first_octave,
            amplitudes,
        }))
    }

    fn f64_field(obj: &Value, name: &'static str, default: f64) -> Result<f64, DensityError> {
        match obj.get(name) {
            None => Ok(default),
            Some(v) => v.as_f64().ok_or(DensityError::NotANumber(name.to_owned())),
        }
    }

    /// Recursively parse a density-function node.
    pub fn parse(&mut self, v: &Value) -> Result<DensityFunction, DensityError> {
        // `minecraft:constant` is special: vanilla encodes a constant as a bare
        // JSON number (`1.5`) rather than an object. Accept both forms.
        if let Some(n) = v.as_f64() {
            return Ok(DensityFunction::Constant(n));
        }

        let obj = v.as_object().ok_or(DensityError::NotAnObject)?;
        let ty = obj
            .get("type")
            .and_then(Value::as_str)
            .ok_or(DensityError::MissingType)?;
        // Datapack ids are namespaced; we only care about the path.
        let path = ty.strip_prefix("minecraft:").unwrap_or(ty);

        let arg = |key: &'static str| -> Result<&Value, DensityError> {
            obj.get(key).ok_or(DensityError::MissingField(key))
        };

        Ok(match path {
            // NOTE: the field is `argument`, not `value`. Confirmed by having
            // vanilla serialise its own constant.
            "constant" => DensityFunction::Constant(Self::f64_field(v, "argument", 0.0)?),

            "y_clamped_gradient" => DensityFunction::YClampedGradient {
                from_y: Self::f64_field(v, "from_y", 0.0)?,
                to_y: Self::f64_field(v, "to_y", 0.0)?,
                from_value: Self::f64_field(v, "from_value", 0.0)?,
                to_value: Self::f64_field(v, "to_value", 0.0)?,
            },

            "noise" => {
                let noise = self.noise_ref(v)?;
                DensityFunction::Noise {
                    noise,
                    xz_scale: Self::f64_field(v, "xz_scale", 1.0)?,
                    y_scale: Self::f64_field(v, "y_scale", 1.0)?,
                }
            }

            "shifted_noise" => {
                // Field order matters: `mapAll` walks shiftX, shiftY, shiftZ and only
                // then the noise holder, and the occurrence mode depends on that order.
                let xz_scale = Self::f64_field(v, "xz_scale", 1.0)?;
                let y_scale = Self::f64_field(v, "y_scale", 1.0)?;
                let shift_x = Box::new(self.parse(arg("shift_x")?)?);
                let shift_y = Box::new(self.parse(arg("shift_y")?)?);
                let shift_z = Box::new(self.parse(arg("shift_z")?)?);
                DensityFunction::ShiftedNoise(Box::new(ShiftedNoiseData {
                    noise: self.noise_ref(v)?,
                    xz_scale,
                    y_scale,
                    shift_x,
                    shift_y,
                    shift_z,
                }))
            }

            "shift" | "shift_a" | "shift_b" => {
                let kind = match path {
                    "shift" => ShiftKind::Shift,
                    "shift_a" => ShiftKind::ShiftA,
                    _ => ShiftKind::ShiftB,
                };
                // `argument` here may be either a name or inline parameters; the
                // real overworld router uses the inline form.
                let descriptor = arg("argument")?;
                DensityFunction::Shift {
                    kind,
                    noise: self.noise_descriptor(descriptor)?,
                }
            }

            "add" => DensityFunction::Add(
                Box::new(self.parse(arg("argument1")?)?),
                Box::new(self.parse(arg("argument2")?)?),
            ),
            "mul" => DensityFunction::Mul(
                Box::new(self.parse(arg("argument1")?)?),
                Box::new(self.parse(arg("argument2")?)?),
            ),
            "min" => DensityFunction::Min(
                Box::new(self.parse(arg("argument1")?)?),
                Box::new(self.parse(arg("argument2")?)?),
            ),
            "max" => DensityFunction::Max(
                Box::new(self.parse(arg("argument1")?)?),
                Box::new(self.parse(arg("argument2")?)?),
            ),

            "clamp" => {
                // Consume the bound entry first: the oracle walks pre-order, so the node
                // itself comes before its input.
                let from_table = if let Some((bounds, next)) = self.clamp_bounds.as_mut() {
                    let b = bounds.get(*next).copied().unwrap_or_else(|| {
                        panic!("the graph has more clamp nodes than the oracle reported")
                    });
                    *next += 1;
                    Some(b)
                } else {
                    None
                };
                let input = Box::new(self.parse(arg("input")?)?);
                let (min, max) = match from_table {
                    Some(b) => b,
                    // No side table: fall back to whatever the JSON says, which is nothing.
                    None => (
                        Self::f64_field(v, "min", f64::NEG_INFINITY)?,
                        Self::f64_field(v, "max", f64::INFINITY)?,
                    ),
                };
                DensityFunction::Clamp { input, min, max }
            }

            "range_choice" => DensityFunction::RangeChoice(Box::new(RangeChoiceData {
                input: Box::new(self.parse(arg("input")?)?),
                min_inclusive: Self::f64_field(v, "min_inclusive", f64::NEG_INFINITY)?,
                max_exclusive: Self::f64_field(v, "max_exclusive", f64::INFINITY)?,
                when_in_range: Box::new(self.parse(arg("when_in_range")?)?),
                when_out_of_range: Box::new(self.parse(arg("when_out_of_range")?)?),
            })),

            "abs" => DensityFunction::AbsWrapped(Box::new(self.parse(arg("argument")?)?)),
            "square" => DensityFunction::Square(Box::new(self.parse(arg("argument")?)?)),
            "cube" => DensityFunction::Cube(Box::new(self.parse(arg("argument")?)?)),
            "half_negative" => {
                DensityFunction::HalfNegative(Box::new(self.parse(arg("argument")?)?))
            }
            "quarter_negative" => {
                DensityFunction::QuarterNegative(Box::new(self.parse(arg("argument")?)?))
            }
            "squeeze" => DensityFunction::Squeeze(Box::new(self.parse(arg("argument")?)?)),
            "invert" => DensityFunction::Invert(Box::new(self.parse(arg("argument")?)?)),

            "old_blended_noise" => {
                // The JSON names are the scale factors, not the internally-derived
                // multipliers. The *seed* is not in the encoding at all: the codec's decoder
                // rebuilds `BlendedNoise` from `XoroshiroRandomSource(0)`, while the live graph
                // has whatever `RandomState` gave it.
                let blended = match self.positional_source("minecraft:terrain") {
                    // A real router: `RandomState` re-seeds it, and the encoding does not say so.
                    Some(mut random) => BlendedNoise::new_seeded(
                        &mut random,
                        Self::f64_field(v, "xz_scale", 1.0)?,
                        Self::f64_field(v, "y_scale", 1.0)?,
                        Self::f64_field(v, "xz_factor", 80.0)?,
                        Self::f64_field(v, "y_factor", 160.0)?,
                        Self::f64_field(v, "smear_scale_multiplier", 8.0)?,
                    ),
                    None => BlendedNoise::create_unseeded(
                        Self::f64_field(v, "xz_scale", 1.0)?,
                        Self::f64_field(v, "y_scale", 1.0)?,
                        Self::f64_field(v, "xz_factor", 80.0)?,
                        Self::f64_field(v, "y_factor", 160.0)?,
                        Self::f64_field(v, "smear_scale_multiplier", 8.0)?,
                    ),
                };
                DensityFunction::OldBlendedNoise(Box::new(blended))
            }
            "weird_scaled_sampler" => {
                let rarity = match obj.get("rarity_value_mapper").and_then(Value::as_str) {
                    Some("type_1") => RarityMapper::Type1,
                    Some("type_2") => RarityMapper::Type2,
                    other => {
                        return Err(DensityError::Unsupported(format!(
                            "unknown rarity_value_mapper: {other:?}"
                        )))
                    }
                };
                DensityFunction::WeirdScaledSampler {
                    input: Box::new(self.parse(arg("input")?)?),
                    noise: self.noise_ref(v)?,
                    rarity,
                }
            }

            "spline" => DensityFunction::Spline(Box::new(self.parse_spline(v)?)),

            // The "no blending" variants. With no Blender present — which is the
            // case for a fully generated area, and for the data generator — these
            // are exactly 1.0 and 0.0, confirmed from bytecode (`dconst_1` /
            // `dconst_0`). BlendDensity is then `lerp(1, 0, input) == input`, which
            // is why it can stay a passthrough.
            "blend_alpha" => DensityFunction::Constant(1.0),
            "blend_offset" => DensityFunction::Constant(0.0),

            // `interpolated` is not a passthrough: it is where the coarse cell sampling
            // happens. See the `Interpolated` variant.
            "interpolated" => {
                let slot = self.interp_slots;
                self.interp_slots += 1;
                DensityFunction::Interpolated {
                    input: Box::new(self.parse(arg("argument")?)?),
                    slot,
                }
            }

            // All the cache markers stay passthroughs. See the note on `Cached`: keying on
            // `(x, z)` alone is only sound for nodes that provably do not vary with `y`, and
            // nothing here establishes that, so an attempted cache was both unsound and no
            // faster.
            // All the cache markers stay passthroughs. An O(1), generation-tagged, cell-scoped
            // cache was 2.65x faster (11.1 -> 29.4 chunks/s) and wrong: it changed 620 of the
            // 98,304 chunked column values. The keying does not yet match `FlatCache`, and a
            // faster wrong answer is not worth having. See the README.
            // `flat_cache` / `cache_2d`. `NoiseChunk.FlatCache.compute` returns
            // `values[quartX - firstNoiseX][quartZ - firstNoiseZ]` — a 5 x 5 grid over the whole
            // chunk — so the node reads as the value at the corner of its quart, not at the point
            // asked for, and the subtree below it is evaluated 25 times per chunk. Both halves
            // matter: reading the corner is what the chunk does, and memoising it is what makes
            // it fast.
            "flat_cache" | "cache_2d" => DensityFunction::QuartSnapped(Box::new(self.parse(arg("argument")?)?)),

            "cache_once" | "cache_all_in_cell" | "blend_density" => {
                DensityFunction::PassThrough(Box::new(self.parse(arg("argument")?)?))
            }

            // Deliberately loud: these need real implementations before a world
            // can be generated correctly.
            "end_islands" | "find_top_surface" => {
                return Err(DensityError::Unsupported(ty.to_owned()));
            }

            _ => return Err(DensityError::UnknownType(ty.to_owned())),
        })
    }

    /// Parse a `minecraft:spline` node.
    ///
    /// JSON shape, taken from vanilla's own encoding:
    /// `{"spline":{"coordinate":<density function>,"points":[{"location":f,
    /// "value":f,"derivative":f}, ...]}}`. A point's `value` may also be another
    /// spline, in which case it has no derivative.
    fn parse_spline(&mut self, v: &Value) -> Result<CubicSpline, DensityError> {
        // A spline appears in two shapes, and vanilla accepts both:
        //   * as a typed node: {"type":"minecraft:spline","spline":{...}}
        //   * as a nested point value, where the *bare body* is serialised with
        //     no type wrapper and no "spline" key: {"coordinate":...,"points":[...]}
        // Getting this wrong is easy; only diffing against vanilla revealed it.
        let body = v.get("spline").unwrap_or(v);
        let sp = body.as_object().ok_or(DensityError::NotAnObject)?;

        let coordinate = Box::new(self.parse(
            sp.get("coordinate")
                .ok_or(DensityError::MissingField("coordinate"))?,
        )?);

        let points = sp
            .get("points")
            .and_then(Value::as_array)
            .ok_or(DensityError::MissingField("points"))?;
        if points.is_empty() {
            return Err(DensityError::Unsupported(
                "spline with no points".to_owned(),
            ));
        }

        let mut locations = Vec::with_capacity(points.len());
        let mut values = Vec::with_capacity(points.len());
        let mut derivatives = Vec::with_capacity(points.len());

        for point in points {
            let po = point.as_object().ok_or(DensityError::NotAnObject)?;
            let location = po
                .get("location")
                .and_then(Value::as_f64)
                .ok_or(DensityError::MissingField("location"))?;
            let value = po.get("value").ok_or(DensityError::MissingField("value"))?;

            locations.push(location as f32);

            if let Some(n) = value.as_f64() {
                values.push(SplineValue::Constant(n as f32));
                let d = po
                    .get("derivative")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                derivatives.push(d as f32);
            } else {
                // A nested spline; vanilla gives these no derivative.
                let nested = self.parse_spline(value)?;
                values.push(SplineValue::Nested(Box::new(nested)));
                derivatives.push(0.0);
            }
        }

        // Vanilla's validateSizes also requires strictly increasing locations;
        // silently accepting an unsorted spline would produce nonsense.
        for w in locations.windows(2) {
            if w[1] <= w[0] {
                return Err(DensityError::Unsupported(format!(
                    "spline locations must strictly increase, got {} then {}",
                    w[0], w[1]
                )));
            }
        }

        Ok(CubicSpline {
            coordinate,
            locations,
            values,
            derivatives,
        })
    }

    /// Convenience: parse a whole JSON document (string) into a tree.
    pub fn parse_str(&mut self, json: &str) -> Result<DensityFunction, DensityError> {
        let v: Value = serde_json::from_str(json)
            .map_err(|e| DensityError::Unsupported(format!("invalid JSON: {e}")))?;
        self.parse(&v)
    }
}

impl DensityFunction {
    /// The chunked reading: identical to [`Self::compute`] except that `interpolated` markers
    /// do their corner lerp, as they do inside a `NoiseChunk`.
    #[must_use]
    pub fn compute_chunked(&self, noises: &[NormalNoise], x: f64, y: f64, z: f64) -> f64 {
        // Empty the corner cache when the cell changes. All `interpolated` nodes share the same
        // 4 x 8 x 4 grid, so one check here covers every one of them.
        let cell = ((x / 4.0).floor() as i64, (y / 8.0).floor() as i64, (z / 4.0).floor() as i64);
        if CORNER_CELL.with(std::cell::Cell::get) != cell {
            CORNER_CELL.with(|c| c.set(cell));
            CORNERS.with(|c| c.borrow_mut().1 = false);
        }
        CHUNKED.with(|c| c.set(true));
        let out = self.compute(noises, x, y, z);
        CHUNKED.with(|c| c.set(false));
        out
    }

    #[must_use]
    pub fn compute(&self, noises: &[NormalNoise], x: f64, y: f64, z: f64) -> f64 {
        if VISITS_ENABLED.with(std::cell::Cell::get) {
            count_visit();
            bump_tally(tally(self));
        }
        match self {
            Self::Constant(v) => *v,
            Self::YClampedGradient {
                from_y,
                to_y,
                from_value,
                to_value,
            } => {
                // Vanilla: t = (y - from_y) / (to_y - from_y), clamped to [0,1].
                let t = ((y - from_y) / (to_y - from_y)).clamp(0.0, 1.0);
                from_value + t * (to_value - from_value)
            }
            Self::Noise {
                noise,
                xz_scale,
                y_scale,
            } => {
                let (sx, sy, sz) = (x * xz_scale, y * y_scale, z * xz_scale);
                if !chunked() {
                    return noises[*noise].get(sx, sy, sz);
                }
                let key = (
                    *noise as u32,
                    (sx as i64) as u64 ^ sx.to_bits().rotate_left(1),
                    (sy as i64) as u64 ^ sy.to_bits().rotate_left(7),
                    (sz as i64) as u64 ^ sz.to_bits().rotate_left(13),
                );
                // Only memoise exact integer coordinates: the cell corners behind
                // `interpolated` and `flat_cache` are integers, and non-integer queries are rare
                // enough that hashing them costs more than it saves.
                if sx.fract() == 0.0 && sy.fract() == 0.0 && sz.fract() == 0.0 {
                    // Counted only when the diagnostic is on: the increment is a thread-local
                    // write on the hottest path in the project.
                    let counting = VISITS_ENABLED.with(std::cell::Cell::get);
                    if let Some(v) = NOISE_CACHE.with(|c| c.borrow().get(&key).copied()) {
                        if counting {
                            NOISE_HITS.with(|c| c.set([c.get()[0] + 1, c.get()[1]]));
                        }
                        return v;
                    }
                    if counting {
                        NOISE_HITS.with(|c| c.set([c.get()[0], c.get()[1] + 1]));
                    }
                    let v = noises[*noise].get(sx, sy, sz);
                    NOISE_CACHE.with(|c| c.borrow_mut().insert(key, v));
                    return v;
                }
                noises[*noise].get(sx, sy, sz)
            }
            Self::ShiftedNoise(data) => {
                let ShiftedNoiseData {
                    noise,
                    xz_scale,
                    y_scale,
                    shift_x,
                    shift_y,
                    shift_z,
                } = data.as_ref();
                // Vanilla applies the shift *after* scaling: `coord * scale + shift`.
                // (The shift nodes already include their own factor of 4.)
                let sx = shift_x.compute(noises, x, y, z);
                let sy = shift_y.compute(noises, x, y, z);
                let sz = shift_z.compute(noises, x, y, z);
                noises[*noise].get(
                    x * xz_scale + sx,
                    y * y_scale + sy,
                    z * xz_scale + sz,
                )
            }
            Self::Shift { kind, noise } => {
                // `DensityFunctions.ShiftNoise.compute(x,y,z)` is
                // `offsetNoise.getValue(x*0.25, y*0.25, z*0.25) * 4.0`.
                //
                // The three variants pass different block coordinates. Note the
                // argument order in each — shift_b is `(z, x, 0)`, not `(z, 0, x)`.
                // Getting that wrong is invisible until you diff against vanilla:
                //   shift   -> (x, y, z)
                //   shift_a -> (x, 0, z)
                //   shift_b -> (z, x, 0)
                let n = &noises[*noise];
                let raw = match kind {
                    ShiftKind::Shift => n.get(x * 0.25, y * 0.25, z * 0.25),
                    ShiftKind::ShiftA => n.get(x * 0.25, 0.0, z * 0.25),
                    ShiftKind::ShiftB => n.get(z * 0.25, x * 0.25, 0.0),
                };
                raw * 4.0
            }
            Self::Add(a, b) => a.compute(noises, x, y, z) + b.compute(noises, x, y, z),
            Self::Mul(a, b) => a.compute(noises, x, y, z) * b.compute(noises, x, y, z),
            Self::Min(a, b) => a.compute(noises, x, y, z).min(b.compute(noises, x, y, z)),
            Self::Max(a, b) => a.compute(noises, x, y, z).max(b.compute(noises, x, y, z)),
            Self::Clamp { input, min, max } => input.compute(noises, x, y, z).clamp(*min, *max),
            Self::AbsWrapped(a) => {
                let v = a.compute(noises, x, y, z);
                if v < 0.0 {
                    -v
                } else {
                    v
                }
            }
            Self::Square(a) => {
                let v = a.compute(noises, x, y, z);
                v * v
            }
            Self::Cube(a) => {
                let v = a.compute(noises, x, y, z);
                v * v * v
            }
            Self::HalfNegative(a) => {
                let v = a.compute(noises, x, y, z);
                if v < 0.0 {
                    v * 0.5
                } else {
                    v
                }
            }
            Self::QuarterNegative(a) => {
                let v = a.compute(noises, x, y, z);
                if v < 0.0 {
                    v * 0.25
                } else {
                    v
                }
            }
            Self::Squeeze(a) => {
                // Vanilla: clamp to [-1, 1] then scale by 0.5, using the
                // "weird" blend at the edges.
                let v = a.compute(noises, x, y, z).clamp(-1.0, 1.0);
                v / 2.0 - v * v * v / 24.0
            }
            Self::Invert(a) => -a.compute(noises, x, y, z),
            Self::Spline(spline) => f64::from(spline.apply(noises, x, y, z)),
            Self::OldBlendedNoise(blended) => blended.compute(x, y, z),
            Self::WeirdScaledSampler {
                input,
                noise,
                rarity,
            } => {
                // Note: `blockX / rarity` is a *double* division of the integer
                // block coordinate.
                let r = rarity.rarity(input.compute(noises, x, y, z));
                r * noises[*noise].get(x / r, y / r, z / r).abs()
            }
            Self::RangeChoice(data) => {
                let RangeChoiceData {
                    input,
                    min_inclusive,
                    max_exclusive,
                    when_in_range,
                    when_out_of_range,
                } = data.as_ref();
                // Vanilla: `d >= minInclusive && d < maxExclusive`.
                let d = input.compute(noises, x, y, z);
                if d >= *min_inclusive && d < *max_exclusive {
                    when_in_range.compute(noises, x, y, z)
                } else {
                    when_out_of_range.compute(noises, x, y, z)
                }
            }
            Self::PassThrough(a) => a.compute(noises, x, y, z),
            Self::QuartSnapped(input) => {
                // The raw graph's `Marker.compute` is a passthrough, so a bare `compute` must be
                // one too; only the chunk installs the cache.
                if !chunked() {
                    return input.compute(noises, x, y, z);
                }
                // `QuartPos.fromBlock` is `>> 2`, and the corner is that quart times 4. `y` is
                // passed through untouched: `FlatCache` never reads it.
                let corner_x = (x as i64) >> 2 << 2;
                let corner_z = (z as i64) >> 2 << 2;
                let key = std::ptr::from_ref::<Self>(input.as_ref()) as usize;
                let hit = QUART_CACHE.with(|c| c.borrow().get(&(key, corner_x, corner_z)).copied());
                match hit {
                    Some(v) => v,
                    None => {
                        let v = input.compute(noises, corner_x as f64, y, corner_z as f64);
                        QUART_CACHE.with(|c| {
                            c.borrow_mut().insert((key, corner_x, corner_z), v);
                        });
                        v
                    }
                }
            }
            Self::Interpolated { input, slot } => {
                if !chunked() {
                    return input.compute(noises, x, y, z);
                }
                const CW: f64 = 4.0;
                const CH: f64 = 8.0;
                let slot = *slot as usize;

                let cell_x = (x / CW).floor();
                let cell_y = (y / CH).floor();
                let cell_z = (z / CW).floor();
                let fx = (x - cell_x * CW) / CW;
                let fy = (y - cell_y * CH) / CH;
                let fz = (z - cell_z * CW) / CW;

                // The lerp happens *inside* the borrow. Returning the eight corners instead
                // copies 64 bytes on every hit — 371,625 of them per chunk — which measured
                // slower than the scan it replaced. A closure returning a single `f64` does not.
                let hit = CORNERS.with(|c| {
                    let c = c.borrow();
                    if c.1 && c.0.len() > slot {
                        let n = &c.0[slot];
                        Some(crate::math::lerp3(
                            fx, fy, fz, n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7],
                        ))
                    } else {
                        None
                    }
                });
                if let Some(d) = hit {
                    return d;
                }

                // Miss: compute the corners with no borrow held, because the subtree may contain
                // another `interpolated` node and would otherwise re-enter.
                let (cx, cy, cz) = (cell_x * CW, cell_y * CH, cell_z * CW);
                let mut v = [0.0f64; 8];
                for (i, (dx, dy, dz)) in [
                    (0.0, 0.0, 0.0),
                    (1.0, 0.0, 0.0),
                    (0.0, 1.0, 0.0),
                    (1.0, 1.0, 0.0),
                    (0.0, 0.0, 1.0),
                    (1.0, 0.0, 1.0),
                    (0.0, 1.0, 1.0),
                    (1.0, 1.0, 1.0),
                ]
                .iter()
                .enumerate()
                {
                    v[i] = input.compute(noises, cx + dx * CW, cy + dy * CH, cz + dz * CW);
                }
                CORNERS.with(|c| {
                    let mut c = c.borrow_mut();
                    if c.0.len() <= slot {
                        c.0.resize(slot + 1, [f64::NAN; 8]);
                    }
                    c.0[slot] = v;
                    c.1 = true;
                });
                crate::math::lerp3(
                    fx, fy, fz, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7],
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::NormalNoise;
    use crate::random::LegacyRandomSource;

    fn test_noises(builder: &DensityBuilder) -> Vec<NormalNoise> {
        builder
            .noise_names()
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let mut r = LegacyRandomSource::new(1000 + i as u64);
                NormalNoise::new_legacy(&mut r, -7, &[1.0, 1.0, 1.0, 1.0])
            })
            .collect()
    }

    #[test]
    fn parses_and_evaluates_a_realistic_density_graph() {
        // Shaped like vanilla's overworld skeleton, but small:
        //   density = gradient(y) + noise*0.5
        let json = r#"{
          "type": "minecraft:add",
          "argument1": {
            "type": "minecraft:y_clamped_gradient",
            "from_y": -64, "to_y": 320,
            "from_value": 1.0, "to_value": -1.0
          },
          "argument2": {
            "type": "minecraft:mul",
            "argument1": {
              "type": "minecraft:noise",
              "noise": "minecraft:temperature",
              "xz_scale": 0.25, "y_scale": 0.0
            },
            "argument2": { "type": "minecraft:constant", "argument": 0.5 }
          }
        }"#;

        let mut b = DensityBuilder::new();
        let f = b.parse_str(json).expect("should parse");
        assert_eq!(b.noise_names(), vec!["minecraft:temperature".to_owned()]);

        let noises = test_noises(&b);

        // At y below the gradient start, the gradient term is fully +1.
        let low = f.compute(&noises, 0.0, -100.0, 0.0);
        let high = f.compute(&noises, 0.0, 400.0, 0.0);
        assert!(low > high, "terrain should thin out with height: {low} vs {high}");
        assert!(low.is_finite() && high.is_finite());
    }

    #[test]
    fn y_clamped_gradient_endpoints_and_clamping() {
        let mut b = DensityBuilder::new();
        let f = b
            .parse_str(
                r#"{ "type":"minecraft:y_clamped_gradient",
                     "from_y":0,"to_y":100,"from_value":10.0,"to_value":0.0 }"#,
            )
            .unwrap();
        assert!((f.compute(&[], 0.0, 0.0, 0.0) - 10.0).abs() < 1e-12);
        assert!((f.compute(&[], 0.0, 100.0, 0.0) - 0.0).abs() < 1e-12);
        assert!((f.compute(&[], 0.0, 50.0, 0.0) - 5.0).abs() < 1e-12);
        // Clamped outside the range.
        assert!((f.compute(&[], 0.0, -50.0, 0.0) - 10.0).abs() < 1e-12);
        assert!((f.compute(&[], 0.0, 500.0, 0.0) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn cache_wrappers_are_transparent() {
        let mut b = DensityBuilder::new();
        let wrapped = b
            .parse_str(r#"{ "type":"minecraft:flat_cache",
                            "argument": { "type":"minecraft:constant", "argument": 3.5 } }"#)
            .unwrap();
        assert!((wrapped.compute(&[], 1.0, 2.0, 3.0) - 3.5).abs() < 1e-12);
    }

    #[test]
    fn constant_accepts_both_encodings() {
        // Vanilla serialises a constant as a bare number, but also accepts an
        // object. Guessing only "value" here was an actual bug: real datapack
        // JSON uses "argument" and would not have loaded.
        let mut b = DensityBuilder::new();
        let bare = b.parse_str("1.5").unwrap();
        assert!((bare.compute(&[], 0.0, 0.0, 0.0) - 1.5).abs() < 1e-15);

        let obj = b
            .parse_str(r#"{"type":"minecraft:constant","argument":1.5}"#)
            .unwrap();
        assert!((obj.compute(&[], 0.0, 0.0, 0.0) - 1.5).abs() < 1e-15);
    }

    #[test]
    fn range_choice_branches_on_the_input() {
        let json = r#"{"type":"minecraft:range_choice",
            "input":{"type":"minecraft:constant","argument":0.5},
            "min_inclusive":-1.0,"max_exclusive":1.0,
            "when_in_range":{"type":"minecraft:constant","argument":10.0},
            "when_out_of_range":{"type":"minecraft:constant","argument":20.0}}"#;
        let mut b = DensityBuilder::new();
        let f = b.parse_str(json).unwrap();
        assert!((f.compute(&[], 0.0, 0.0, 0.0) - 10.0).abs() < 1e-15);
    }

    #[test]
    fn range_choice_bounds_are_inclusive_min_exclusive_max() {
        // The boundary case is the whole reason to test this node separately.
        let make = |v: f64| {
            format!(
                r#"{{"type":"minecraft:range_choice",
                   "input":{{"type":"minecraft:constant","argument":{v}}},
                   "min_inclusive":0.0,"max_exclusive":1.0,
                   "when_in_range":{{"type":"minecraft:constant","argument":1.0}},
                   "when_out_of_range":{{"type":"minecraft:constant","argument":0.0}}}}"#
            )
        };
        let mut b = DensityBuilder::new();

        // exactly min -> in range
        let lo = b.parse_str(&make(0.0)).unwrap();
        assert!((lo.compute(&[], 0.0, 0.0, 0.0) - 1.0).abs() < 1e-15);
        // exactly max -> out of range
        let hi = b.parse_str(&make(1.0)).unwrap();
        assert!((hi.compute(&[], 0.0, 0.0, 0.0) - 0.0).abs() < 1e-15);
        // just inside -> in range
        let mid = b.parse_str(&make(0.999)).unwrap();
        assert!((mid.compute(&[], 0.0, 0.0, 0.0) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn spline_interpolates_between_points() {
        // Coordinate is a constant, so the spline is a pure function of the points.
        let json = r#"{"type":"minecraft:spline","spline":{
            "coordinate":{"type":"minecraft:constant","argument":0.5},
            "points":[
              {"location":0.0,"value":0.0,"derivative":0.0},
              {"location":1.0,"value":10.0,"derivative":0.0}]}}"#;
        let mut b = DensityBuilder::new();
        let f = b.parse_str(json).unwrap();
        // With zero derivatives at both ends this degenerates to the linear lerp.
        let v = f.compute(&[], 0.0, 0.0, 0.0);
        assert!((v - 5.0).abs() < 1e-6, "expected 5.0, got {v}");
    }

    #[test]
    fn spline_extrapolates_below_and_above_its_range() {
        let make = |coord: f64| {
            format!(
                r#"{{"type":"minecraft:spline","spline":{{
                   "coordinate":{{"type":"minecraft:constant","argument":{coord}}},
                   "points":[
                     {{"location":0.0,"value":1.0,"derivative":2.0}},
                     {{"location":1.0,"value":3.0,"derivative":5.0}}]}}}}"#
            )
        };
        let mut b = DensityBuilder::new();

        // Below the first point: value + derivative * (coord - location).
        let below = b.parse_str(&make(-1.0)).unwrap();
        let v = below.compute(&[], 0.0, 0.0, 0.0);
        assert!((v - (1.0 + 2.0 * -1.0)).abs() < 1e-6, "got {v}");

        // Above the last point.
        let above = b.parse_str(&make(2.0)).unwrap();
        let v = above.compute(&[], 0.0, 0.0, 0.0);
        assert!((v - (3.0 + 5.0 * 1.0)).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn spline_accepts_a_nested_bare_body_value() {
        // Vanilla serialises a nested point value as the bare spline body, with
        // no type wrapper. Parsing only the wrapped form was a real bug.
        let json = r#"{"type":"minecraft:spline","spline":{
            "coordinate":{"type":"minecraft:constant","argument":0.0},
            "points":[
              {"location":0.0,"value":{
                 "coordinate":{"type":"minecraft:constant","argument":0.0},
                 "points":[{"location":0.0,"value":7.0,"derivative":0.0}]},
               "derivative":0.0},
              {"location":1.0,"value":1.0,"derivative":0.0}]}}"#;
        let mut b = DensityBuilder::new();
        let f = b.parse_str(json).unwrap();
        let v = f.compute(&[], 0.0, 0.0, 0.0);
        assert!((v - 7.0).abs() < 1e-6, "expected the nested spline value, got {v}");
    }

    #[test]
    fn spline_rejects_unsorted_locations() {
        let json = r#"{"type":"minecraft:spline","spline":{
            "coordinate":{"type":"minecraft:constant","argument":0.5},
            "points":[
              {"location":1.0,"value":0.0,"derivative":0.0},
              {"location":0.0,"value":10.0,"derivative":0.0}]}}"#;
        let mut b = DensityBuilder::new();
        let err = b.parse_str(json).unwrap_err().to_string();
        assert!(err.contains("strictly increase"), "unexpected error: {err}");
    }

    #[test]
    fn spline_rejects_an_empty_point_list() {
        let json = r#"{"type":"minecraft:spline","spline":{
            "coordinate":{"type":"minecraft:constant","argument":0.0},"points":[]}}"#;
        let mut b = DensityBuilder::new();
        assert!(b.parse_str(json).is_err());
    }

    #[test]
    fn noise_accepts_inline_parameters() {
        // Vanilla has two encodings for a noise reference. Real datapacks use a
        // name; the serialised built-in router inlines the parameters.
        let json = r#"{"type":"minecraft:noise",
            "noise":{"firstOctave":-8,"amplitudes":[1.0]},
            "xz_scale":1.5,"y_scale":1.0}"#;
        let mut b = DensityBuilder::new();
        b.parse_str(json).unwrap();
        assert_eq!(b.noise_specs().len(), 1);
        match &b.noise_specs()[0] {
            NoiseSpec::Inline {
                first_octave,
                amplitudes,
            } => {
                assert_eq!(*first_octave, -8);
                assert_eq!(amplitudes, &vec![1.0]);
            }
            other => panic!("expected inline parameters, got {other:?}"),
        }
        assert!(b.noise_names().is_empty(), "inline noises have no name");
    }

    #[test]
    fn identical_inline_noises_are_deduplicated() {
        let json = r#"{"type":"minecraft:add",
            "argument1":{"type":"minecraft:noise","noise":{"firstOctave":-8,"amplitudes":[1.0]}},
            "argument2":{"type":"minecraft:noise","noise":{"firstOctave":-8,"amplitudes":[1.0]}}}"#;
        let mut b = DensityBuilder::new();
        b.parse_str(json).unwrap();
        assert_eq!(
            b.noise_specs().len(),
            1,
            "identical parameters must share one instance"
        );
    }

    #[test]
    fn inline_amplitudes_accept_a_bare_number() {
        // Vanilla's codec allows `"amplitudes": 1.0` as shorthand for a one-element
        // list.
        let json = r#"{"type":"minecraft:noise","noise":{"firstOctave":-5,"amplitudes":1.0}}"#;
        let mut b = DensityBuilder::new();
        b.parse_str(json).unwrap();
        match &b.noise_specs()[0] {
            NoiseSpec::Inline { amplitudes, .. } => assert_eq!(amplitudes, &vec![1.0]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn shift_nodes_accept_inline_parameters() {
        let json = r#"{"type":"minecraft:shift_a",
            "argument":{"firstOctave":-3,"amplitudes":[1.0,1.0,1.0,0.0]}}"#;
        let mut b = DensityBuilder::new();
        b.parse_str(json).unwrap();
        assert_eq!(b.noise_specs().len(), 1);
        assert!(matches!(b.noise_specs()[0], NoiseSpec::Inline { .. }));
    }

    #[test]
    fn blend_alpha_and_offset_are_the_no_blender_constants() {
        let mut b = DensityBuilder::new();
        let alpha = b.parse_str(r#"{"type":"minecraft:blend_alpha"}"#).unwrap();
        let offset = b.parse_str(r#"{"type":"minecraft:blend_offset"}"#).unwrap();
        assert!((alpha.compute(&[], 0.0, 0.0, 0.0) - 1.0).abs() < 1e-15);
        assert!((offset.compute(&[], 0.0, 0.0, 0.0) - 0.0).abs() < 1e-15);
    }

    #[test]
    fn rarity_mapper_boundaries_match_vanilla() {
        // Boundaries are inclusive-exclusive in a specific direction; the exact
        // comparison is load-bearing because these feed the ore-vein scaling.
        assert_eq!(RarityMapper::Type1.rarity(-0.500_000_1), 0.75);
        assert_eq!(RarityMapper::Type1.rarity(-0.5), 1.0);
        assert_eq!(RarityMapper::Type1.rarity(0.0), 1.5);
        assert_eq!(RarityMapper::Type1.rarity(0.5), 2.0);

        assert_eq!(RarityMapper::Type2.rarity(-0.750_000_1), 0.5);
        assert_eq!(RarityMapper::Type2.rarity(-0.75), 0.75);
        assert_eq!(RarityMapper::Type2.rarity(-0.5), 1.0);
        assert_eq!(RarityMapper::Type2.rarity(0.5), 2.0);
        assert_eq!(RarityMapper::Type2.rarity(0.75), 3.0);
    }

    #[test]
    fn weird_scaled_sampler_rejects_an_unknown_rarity_mapper() {
        let json = r#"{"type":"minecraft:weird_scaled_sampler",
            "input":{"type":"minecraft:constant","argument":0.0},
            "noise":{"firstOctave":-7,"amplitudes":[1.0]},
            "rarity_value_mapper":"type_99"}"#;
        let mut b = DensityBuilder::new();
        let err = b.parse_str(json).unwrap_err().to_string();
        assert!(err.contains("type_99"), "unexpected error: {err}");
    }

    #[test]
    fn unsupported_types_fail_loudly() {
        let mut b = DensityBuilder::new();
        // Every overworld node type is implemented now; these two remain, for the
        // surface system and the End. They must fail rather than approximate.
        let err = b
            .parse_str(r#"{ "type":"minecraft:end_islands" }"#)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("end_islands"), "unexpected error: {msg}");
        assert!(msg.contains("not implemented"), "unexpected error: {msg}");
    }

    #[test]
    fn unknown_types_are_reported() {
        let mut b = DensityBuilder::new();
        let err = b.parse_str(r#"{ "type":"modname:mystery" }"#).unwrap_err();
        assert!(err.to_string().contains("modname:mystery"));
    }
}
