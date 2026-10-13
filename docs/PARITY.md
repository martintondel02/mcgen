# What is proven

The claim "bit-identical to Minecraft" is only worth anything if it is checkable. This
document says exactly what was compared, against what, and what remains unproven.

## The method

Minecraft's worldgen is deterministic and its numbers are `double`s. So the test is not
"close enough" — it is **raw IEEE-754 bit pattern equality**.

Two Java programs are compiled against the **real Minecraft jar** and run:

- [`tools/oracle/VanillaOracle.java`](../tools/oracle/VanillaOracle.java) — evaluates
  individual primitives: noise, random sources, hashes, math helpers.
- [`tools/oracle/RouterOracle.java`](../tools/oracle/RouterOracle.java) — loads the real
  overworld `RandomState`, extracts all fifteen density functions, and evaluates them.

A Rust example (`rust/mcgen-core/examples/parity_dump.rs`) emits records in the same format
for the same inputs. [`scripts/check-parity.sh`](../scripts/check-parity.sh) runs both and
diffs the raw `f64` bit patterns.

The oracle reads the real classes rather than a reimplementation of them, which is the
point: if the oracle and the engine shared a bug, the check would be worthless.

## The twelve checks

| # | Check | Values | What it covers |
|---|---|---|---|
| 1 | `improved_noise` | 144 | One octave of gradient noise |
| 2 | `xoroshiro` | 288 | XOROSHIRO128++ random source |
| 3 | `normal_noise` | 576 | The modern two-stack `NormalNoise` |
| 4 | `hash` | 10 | `seedFromHashOf`, octave-name derivation |
| 5 | `math` | 144 | `wrap`, `lerp`, `lerp2`, `lerp3`, `smoothstep` |
| 6 | `density` | 432 | Hand-built graphs, node by node |
| 7 | `density_noise` | 168 | Graphs whose leaves are real noises |
| 8 | `density_json` | 960 | Parse a codec-produced JSON, then evaluate |
| 9 | `density_dense` | 14,400 | The same graphs at many coordinates |
| 10 | `density_router` | 360 | All fifteen real overworld router functions |
| 11 | `noise_seed` | 35 | The `RandomState` → noise seed chain |
| 12 | `column` | 1,574,394 | The chunked reading at all 98,304 block positions of a chunk, ×15 routes, against `NoiseChunk` itself |
| | **total** | **1,591,911** | |

Run it with:

```bash
./scripts/check-parity.sh
```

Check 12 is the one that matters most and the one that was added last. It is the only check
that compares against **vanilla's own chunk pipeline** rather than against a
reimplementation of a piece of it, and it has since caught three separate correctness bugs
that the other eleven all passed.

## Verified on a live server

Parity against extracted classes is necessary but not sufficient — the mod has to work in
a running server, with a real world seed, against real generated chunks.

`/mcgen chunk 0 0` on a vanilla NeoForge world:

```
97,985 of 98,304 positions agree (99.675%)
of those, 298 are blocks a later stage placed (trees, carved cave air) and
21 are the density layer's own: 99.979% agreement there.
```

The 298 are spruce leaves, spruce logs, and carved cave air — placed by the feature and
carving stages that run *after* density and that this engine does not implement. The
remaining 21 out of 98,304 are unattributed; they are most likely aquifer edge cases,
which are also unimplemented.

## What is NOT proven

Being explicit about this is part of the point.

- **Speed.** The engine is **roughly 5-6x slower** than vanilla's own code. It accelerates
  nothing. Measured two ways: ~157 ns per block against vanilla's ~30 ns (which is 5.2x,
  though vanilla's figure includes stages this engine does not implement), and ~59 chunks/s
  against an equivalent ~336 chunks/s (5.7x).
- **Block types.** The engine decides solid or not. It does not decide *what* block, so
  there are no aquifers, no ore veins and no surface rules. A position may be correctly
  identified as solid and still get the wrong block in a real generator.
- **Non-overworld dimensions.** Only the overworld router is implemented. The Nether and
  the End have their own density functions and their own noise settings.
- **Modded worldgen.** The engine implements the *vanilla* overworld router. A datapack
  that changes density functions will not be reflected — by design, since the router is
  part of the compiled fixture. `/mcgen chunk` will measure the difference rather than
  reproduce it.
- **Other platforms.** Only 64-bit Linux is built and tested.
- **The FFM harness** is exercised, but it is not what ships in the mod.

## Reproducing the fixture

The FFI library's overworld router is generated, not hand-written:

```bash
./scripts/make-fixture.sh    # writes rust/mcgen-ffi/fixture/overworld.txt
```

The fixture is embedded with `include_str!`, so **the Rust library must be rebuilt after
regenerating it**. The script prints a reminder; this is the kind of thing that silently
produces stale results otherwise.

One subtlety worth recording, because it cost real time: the density codec is **not
injective**. `RandomState` re-seeds every `BlendedNoise` from
`fromHashOf("minecraft:terrain")`, and the codec rebuilds it from `XoroshiroRandomSource(0)`
— so the seed is lost in the encoding. The fixture therefore carries the seed-dependent
information separately rather than round-tripping it through JSON.
