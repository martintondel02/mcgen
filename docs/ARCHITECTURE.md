# How it works

## The shape of it

```
NeoForge server JVM
│
├── mcgen-0.2.0.jar
│     McGenMod        @Mod entry point; loads the library, runs the self-test,
│     │               keeps one native world per seed
│     ├── McGenCommands   the /mcgen tree, registered on NeoForge's game bus
│     └── McGenNative     native method declarations
│
├── libmcgenjni.so
│     jni_shim.c      converts JNI types to the C ABI. Nothing else — no logic,
│                     no string encoding decisions, no allocation policy.
│     └── links libmcgen.a statically
│
└── Rust
      mcgen-ffi       the C ABI: create/destroy, fill region, fill chunk,
      │               fingerprint, noise samples, density-at-a-point, noise names
      └── mcgen-core  math, random, noise, density, terrain
```

The Java side never sees the density graph. It asks for whole chunks. That is deliberate:
a per-block callback across JNI would cost more than the generation it was accelerating.

## What each Rust module does

| Module | Contents |
|---|---|
| `math` | `wrap`, `lerp`/`lerp2`/`lerp3`, `smoothstep`, the 16-entry gradient table |
| `random` | `LegacyRandomSource` (java.util.Random), `XoroshiroRandomSource`, MD5-based `seedFromHashOf` |
| `noise` | `ImprovedNoise`, `PerlinNoise`, `BlendedNoise`, `NormalNoise` |
| `density` | The density-function graph: parse, evaluate, and the chunked reading |
| `terrain` | The cell grid and the chunk fill |

## The two readings of a graph

This distinction is the single most important thing in the codebase, and getting it
wrong is easy.

Vanilla has two different ways to evaluate the same density graph:

1. **The raw reading.** Each node computes itself. A `Marker.Type.Interpolated` node is a
   **passthrough** — it returns its child's value unchanged. This is what Java's
   `Marker.compute` does.

2. **The chunked reading.** Before evaluation, `NoiseChunk` replaces every
   `Marker.Type.Interpolated` node with a `NoiseInterpolator`. That node samples its child
   at the **eight corners of a 4×8×4-block cell** and trilinearly interpolates the block's
   value from them. It is no longer a passthrough.

So the same graph gives different numbers depending on which reading you use, and only the
chunked one produces vanilla terrain. `mcgen-core` models this with two enum variants —
`Interpolated { input, slot }` and `QuartSnapped { .. }` — that pass through under
`compute` and do their real work under `compute_chunked`.

The parity suite checks **both**, because a bug in either is a bug.

## Caching, and where the speed went

Three caches exist, all chunk-scoped, cleared at the chunk boundary:

- **Cell corners** for `interpolated`. Sampled once per cell per node, then reused by the
  64 blocks in that cell.
- **Quart snapping** for `flat_cache` and `cache_2d`. Vanilla's "2D caches" are not caches
  at all — they snap to the quart corner (a 4×4-block grid) and are therefore evaluated 25
  times per chunk instead of once per block. Reproducing that *is* most of the original
  speedup over the naive port.
- **Noise samples**, keyed by noise index and exact coordinates. This is the one place the
  port does something vanilla does not: a chunk takes ~86,000 noise samples but has only
  ~1,225 distinct cell-corner positions, a 70× redundancy that vanilla re-evaluates every
  time. Memoising it was worth 1.37×.

## Why it is still slower than vanilla

The density layer walks ~14.4 nodes per block. A micro-benchmark
(`rust/mcgen-core/examples/microbench.rs`) puts a single dispatch at **2.3 ns** for a leaf
and **3.6–4.2 ns** for an arithmetic node — but the real graph costs **~11 ns** per node.

That gap has the shape of a cache miss: the micro-benchmark measures a 20-node chain that
lives entirely in L1, while the real graph is ~263 KB of boxed nodes scattered across the
heap. Vanilla, running the same graph, gets ~1.5 ns per node out of JIT-inlined
monomorphic call sites.

Twelve optimisations were attempted. **Every one that changed the shape of the code did
nothing** — a smaller enum, an O(1) cache index, a flattened 338-instruction evaluator,
`lto = "fat"`, `target-cpu = "native"`. **Every one that reduced actual work helped** —
quart snapping (1.35×), noise memoisation (1.37×), and lerping inside the borrow instead
of copying 64 bytes out of it (1.20×). The full account is in
[DEVELOPMENT.md](DEVELOPMENT.md).

## The FFI boundary

`mcgen-ffi` exposes a flat C ABI. Three conventions matter:

- **An ABI version**, checked by the mod at load. A mismatch logs and disables the engine
  rather than risking undefined behaviour.
- **Every entry point catches panics** (`catch_unwind`) and converts them to an error
  code, because a panic unwinding into the JVM is undefined behaviour. The release profile
  uses `panic = "unwind"` for exactly this reason — `panic = "abort"` would defeat it.
- **Errors are sticky per thread.** A failing call records a message that
  `mcgen_last_error` retrieves, so the Java side can report *why* rather than a bare
  number.

Return codes are `0` for success and a small positive code otherwise. Nothing returns a
sentinel value mixed into the data.

## Why JNI ships and FFM does not

`java/harness` contains a second, standalone FFM implementation using Java 21's foreign
function API. It is not what the mod uses.

FFM is **final in Java 22 and a preview feature in Java 21**. NeoForge 1.21.1 targets Java
21. Shipping the mod on FFM would mean every server admin adding `--enable-preview` to
their launch arguments, which also makes the JVM warn on every start and can break other
mods. JNI has no such requirement.

The harness is kept because FFM is where this is going, and having both makes the
trade-off visible rather than asserted.
