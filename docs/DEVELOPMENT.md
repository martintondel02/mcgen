# Development log

This is the chronological record of building the engine: what was measured, what was
assumed, what turned out to be wrong, and how each error was found. It is kept because
the reasoning is more useful than the conclusions — several rounds were spent on
optimisations that did nothing, and knowing *why* they did nothing is the point.

It is not the place to start. Start with the [README](../README.md) for what the project
is, [ARCHITECTURE.md](ARCHITECTURE.md) for how it works, and [PARITY.md](PARITY.md) for
what has actually been proven.

---

# Rust worldgen accelerator for NeoForge 1.21.1

A Rust native library that runs **inside a real NeoForge 21.1.257 / Minecraft
1.21.1 server**, plus the FFI plumbing, benchmarks and tests that prove it works.

**Read this first: it is correct, and it is not an accelerator.** The density layer
reproduces vanilla bit for bit — 1,591,911 values across twelve checks, including all
98,304 block positions of a chunk against `NoiseChunk` itself — and the NeoForge mod loads
it, generates the real overworld router, and reports. But it runs at about **60 chunks/s
where vanilla's own path takes the equivalent of ~336**, so it is roughly **5.7x slower**
than the thing it was meant to replace. Twelve optimisations were attempted; the ones that
reduced *work* helped (5.3x in total) and every one that changed the *shape* of the code
did nothing. The measurement, the failures and the residual diagnosis are all below rather
than buried.

What is genuinely proven: the noise, the seeding, the density graph, the chunked reading
and solid/air placement. What is not: block *types* — there are no aquifers, ore veins or
surface rules — and any claim of speed.

---

## Status

| Component | State |
|---|---|
| `ImprovedNoise` (single octave) | ✅ **bit-exact vs Minecraft 1.21.1** — 144/144 samples |
| `PerlinNoise` (octave stack) | ✅ **bit-exact** — real structure read from vanilla bytecode |
| `NormalNoise` (modern path) | ✅ **bit-exact** — 576/576 samples |
| `XoroshiroRandomSource` (XOROSHIRO128++) | ✅ **bit-exact** — 288/288 values |
| `seedFromHashOf` / octave-name derivation | ✅ **bit-exact** — 10/10 |
| `RandomState` noise derivation | ✅ **bit-exact** — 35/35 across the whole router; the real seed chain |
| `LegacyRandomSource` | ✅ bit-exact vs `java.util.Random`, verified against the real JVM |
| Density-function JSON parse + eval | ✅ **bit-exact** — 27 node types, on hand-built graphs and on the real overworld router |
| Unknown / modded density types | ✅ hard error, never silently approximated |
| Density evaluation coordinates | ✅ integer block coords, matching `FunctionContext.blockX/Y/Z` |
| Density at chunk-local coords, all 15 routes | ✅ **bit-exact** — 98,304 values per route (1,474,560 total) |
| Chunked evaluation | ✅ **`NoiseChunk`'s own block verdict reproduced** at all 98,304 block positions of a chunk, and `fill_chunk_chunked` matches it |
| Terrain fill (density → blocks) | ✅ **the real chunked model**, per-cell memoised — no aquifers, ore veins or surface rules |
| Fill throughput | ⚠️ **60 chunks/s serial, 491 on 16 cores** — real router, real seeding, terrain byte-identical; about **5.7x slower than vanilla** |
| Parallel chunk fill | ✅ deterministic; serial and parallel paths verified identical |
| FFI via FFM (`java.lang.foreign`) | ✅ standalone harness, Java 21 preview flags |
| FFI via JNI | ✅ in-server, no preview flags needed |
| Panic isolation | ✅ `catch_unwind` at every entry point; tested |
| NeoForge 21.1.257 mod | ✅ loads, self-tests, reports; now generates the **real overworld router** — **does not yet replace chunk generation** |
| **Noise-layer bit-parity** | ✅ **proven against the real classes** |
| **Real overworld router** | ✅ **all 15 functions bit-exact** — 98,304 values each, 1,474,560 total, against the live graph |
| **World-layer parity** | ⚠️ **solid/air proven, block types not** — the verdict matches vanilla at 98,304 points; aquifers, ore veins and surface rules are absent, so the blocks placed are not |

### What is proven, and what is not

**Proven: the noise layer is bit-identical to Minecraft 1.21.1.** Not
"statistically similar" — bit-for-bit, compared on raw `f64` bit patterns against
the real classes shipped in the server install. That covers the single-octave
Perlin, the octave stack, the normalization, the modern XOROSHIRO128++ source,
the MD5 name derivation, and the full `NormalNoise` assembly.

Getting there required three non-obvious details, each of which cost a round of
bytecode reading and diffing:

1. `NormalNoise` samples its `second` stack at `(x,y,z) * INPUT_FACTOR` at the
   **call site**, not by scaling each octave's frequency. `x * IF * f` and
   `x * (f * IF)` round differently.
2. `valueFactor` is applied **once at the end** — `(first + second) * valueFactor`.
   Folding it into per-octave amplitudes shifts every result by exactly one ULP.
3. Each octave's `ImprovedNoise` comes from
   `forkPositional().fromHashOf("octave_<index>")` — *not* from a shared stream,
   and not from `nextSplitter()`.

**Also proven: the density layer.** Twenty-seven node types evaluate
bit-identically to vanilla, *at vanilla's integer block coordinates*, and parse
from vanilla's own JSON — verified both on hand-built graphs and on **fourteen of
the fifteen functions of the real overworld router**, produced by Minecraft's own
data generators. So the noise, the graph that consumes it, and the encoding they
arrive in are all exact.

Two precision details were load-bearing and are easy to get backwards:
`CubicSpline` is `f32` throughout (vanilla's `apply` returns `float`, widened to
`double` only at the end), while `NormalNoise` is `f64` throughout.

**Cell-grid interpolation agrees by construction, not by oracle — yet.** Vanilla
does not evaluate the density function per block: it samples a coarse grid and
trilinearly interpolates. The spacing is not a guess (`RouterOracle settings` prints
vanilla's own `cellWidth=4 cellHeight=8`), and the `lerp3` arithmetic is bit-exact via
the `math` check. What is still open is the surrounding bookkeeping.

A real `NoiseChunk` probe now exists — `./scripts/check-parity.sh column` drives the
actual chunk pipeline exactly as `NoiseBasedChunkGenerator.iterateNoiseColumn` does,
and compares the per-block solid/air decision across 25 columns spanning every cell
residue. It is deliberately *not* in the default check list so it cannot paint the suite
red, and it currently stops early rather than comparing: it cannot resolve a registry
name for one of the noises the other routes use, which is the lossy-corpus problem
showing up as a hard stop instead of a wrong number.

What it has already established is in the section above, plus this: with names scoped
to a single graph, four of the fifteen routes (`temperature`, `continents`, `depth`,
`ridges`) evaluate **24/24 bit-exact against real `RandomState` seeding**, and all
sixteen noises construct **16/16 bit-exact**. Only `final_density` fails — and it fails
for a reason that is now understood exactly.

Where that probe has already got to is worth recording, because it closed a
long-standing gap:

* The `RandomState` derivation chain is **proven bit-exact** (`noise_seed`, 12/12).
  Getting there corrected two assumptions that had been wrong since the beginning:
  `fromHashOf` is not an MD5 digest but Java's `String.hashCode() ^ seed`, and
  `forkPositional()` *consumes* a `nextLong()` rather than reading the current seed.
  The overworld's random algorithm also turned out to be `XOROSHIRO`, not the `LEGACY`
  its age suggests.
* With seeding proven, the remaining disagreement in `column` is narrowed to
  `NormalNoise` construction or the graph the router hands to `RandomState`. The
  isolation harness is in place to finish it.

A pleasant side effect: switching to the interpolated path made generation **~30x
faster** — 2,222 chunks/s serial and 16,869 chunks/s parallel, up from 70 and 599 —
because a chunk now costs ~1,225 density evaluations instead of 98,304.

### The last two levers, and where the residual actually is

With the work-reduction route exhausted, two build-level levers were left untried:

| lever | result |
|---|---|
| `lto = "fat"` (was `"thin"`) | 0% (161.0 against 157.1 ns/block) |
| `target-cpu = "native"` | 0% (160.5 ns/block), and it costs portability |

Both reverted. That is twelve distinct optimisations attempted, and the tally is unambiguous: the
cost is not the code's shape, not its build, and not its memory layout.

What the diagnostics now say:

```
RUST chunked:    98304 evals in 15 ms (159.2 ns/eval)
RUST noise memo: 125426 hits, 8226 misses (93.8% hit rate)
RUST visits:     1416682 for 98304 blocks = 14.4 node visits/block (11.0 ns/visit)
```

The noise memo is doing its job — 93.8% of samples are served from cache, and the 8,226 misses
account for only about 7% of the chunk. It also cut the visit count from 19.8 to 14.4 per block,
because a cache hit returns without descending.

That leaves **14.4 visits at 11 ns each**. The micro-benchmark put dispatch at 2.3 to 4.2 ns, but it
measured a chain of 20 to 60 nodes — entirely L1-resident. The real graph is about 263 KB, so the
extra ~7 ns per visit has the shape of a cache miss. That is also consistent with the flattened
evaluator failing to help: it removed the call and the `Box`, but it still scattered 338
instructions across a large working set.

Which means the one thing that would plausibly close the rest is not a micro-optimisation at all:
getting the whole graph into a contiguous, walk-order layout, which nothing tried so far has
actually achieved.

### Ten attempts, one rule: only work counts

Round 37's micro-benchmark finally separated the two things that had been confounded for four
rounds — **dispatch is cheap** (2.3 ns for a leaf, 3.6 to 4.2 ns for an arithmetic node), and **noise
sampling is expensive** (131.5 ns). That reframes every failed optimisation as the same failure:

| change | intent | result |
|---|---|---|
| `RangeChoice`/`ShiftedNoise` boxed, enum 56 to 40 bytes | less memory | +1.7% |
| quart cache keyed by slot instead of a map | O(1) lookup | 0% |
| `CHUNKED`/visit flags as plain statics | cheaper flags | 0%, and broke threading |
| flattened evaluator, 338 instructions | no call, no dispatch | 0% |
| corner cache keyed by slot, not scanned | O(1) lookup | 0% |

Every one of those changes the **shape** of the code. None of them changes the **work**. And the one
thing that has ever helped is reducing work:

| change | intent | result |
|---|---|---|
| `flat_cache` snapped to the quart corner and memoised 25 per chunk | fewer evaluations | **1.35x** |
| noise samples memoised by index and coordinates | fewer samples | **1.37x** |
| the eight corners lerped inside the borrow instead of copied out | less copying | **1.20x** |

That last one is worth spelling out, because it was hiding behind the fifth failure. Making the
corner cache O(1) did nothing on its own — but the obvious way to write it returns `[f64; 8]` out
of the `RefCell`, and that is a **64-byte copy on every hit**, 371,625 times per chunk. Doing the
lerp inside the borrow, so the closure returns a single `f64`, is what paid. The lookup was never
the cost; the copy out of it was.

Cumulatively, against the 30.3 ns/block vanilla takes for the same chunk:

```
first correct model:   34x slower
now:                    5.7x slower   (59.1 chunks/s serial, 488 on 16 cores)
```

Terrain is byte-for-byte unchanged throughout — the in-server integrity hash has been
`0x7ee6ba2f40f8c1c8` since before any of it.

### A micro-benchmark with a bug in it, and then the real answer

Round 36 said the cost was the work, not the shape of the code, and that my visit counter could not
tell the difference. To find out which, I wrote a micro-benchmark. Its first version took a
nullary closure and reported `ImprovedNoise::noise` at **0.4 ns** — because with constant arguments
LLVM hoisted the call straight out of the loop and measured nothing. Every number here depends on
the counter now.

With that fixed:

```
Constant leaf            :   2.3 ns/call
chain of 20 Adds         :  75.2 ns/call =  3.6 ns/node
ImprovedNoise::noise     :  12.0 ns/call
PerlinNoise (4 octaves)  :  66.1 ns/call = 16.5 ns/octave
NormalNoise::get         : 131.5 ns/call
```

**Dispatch is cheap** — 2.3 ns for a leaf, 3.6 to 4.2 ns for an arithmetic node. That settles four
rounds of failed optimisation: the evaluator was never the problem. **Noise sampling is the cost**,
at 131.5 ns a call.

And then the number that mattered: a chunk takes **86,849 noise samples** but has only about
**1,225 distinct cell-corner positions**. The corners behind the `interpolated` markers are shared
by up to eight neighbouring cells, and every one of those shares was being re-sampled.

Memoising noise samples by index and exact coordinates, cleared per chunk, is the single largest
win in the project:

```
before:  257 ns/block,  38.6 chunks/s serial
after:   188 ns/block,  52.9 chunks/s serial, 418 on 16 cores
```

**A 1.37x speedup, with the terrain byte-for-byte unchanged** — the in-server integrity hash is
still `0x7ee6ba2f40f8c1c8` and all 1,574,394 chunked column values still match vanilla.

Worth noting what this is: vanilla does *not* memoise noise samples and re-evaluates every one, so
this is the first place where the port is doing something the original does not. Against vanilla's
30.3 ns/block the gap is now about **6x, down from 34x** when the correct model first landed.

### The flattened evaluator: built, measured, and it is not faster

Round 34 concluded the remaining ~9x was the shape of the code — a recursive, non-inlined `match`
over a boxed tree — and that the fix was to compile the graph into a linear instruction array.
Round 35 built one and it disagreed with the tree everywhere. This round found both bugs in that,
and the answer is that **the hypothesis is wrong**.

The bugs first, because they are instructive. `RangeChoice`'s two branch bodies are laid out in
line, and without a jump at the end of the first the walk fell through the second and left three
values on the stack. And an instruction that *re-runs* a sub-range at different coordinates
(`interpolated`, `flat_cache`) still has its inline copy executed first, so it has to pop it. Both
were stack-discipline errors, and together they accounted for all 98,304 disagreements. With them
fixed, mismatches fell from 98,304 to **128** and the compiler emits **338 instructions**.

And the measurement:

| | ns/block |
|---|---|
| tree evaluator | 257.6 |
| flattened evaluator | 257.2 |

**No speedup at all.** So the ~13 ns per node visit is not dispatch, not the `Box` dereference, not
the call. Three rounds of leaning the tree down said the same thing from the other side, and now
the direct test says it too.

Which points somewhere I had not been looking. My visit counter counts entries to `compute`, and
it does not count the eight corner sub-evaluations each `interpolated` node performs per cell, nor
the subtree re-runs behind `flat_cache`. Those are the work — 8 corners x 768 cells x each
`interpolated` node, every one of them a full subtree walk. **The cost is the work, not the
shape of the code that schedules it.**

That is a better target than the one I have been aiming at for four rounds.

### A flattened evaluator, started and withdrawn

Round 34 concluded that the remaining ~9x is the shape of the code — a recursive, non-inlined
`match` over a boxed tree, one real call per node — and that closing it means compiling the graph
once into a linear instruction array and running that. So I built one.

It works as a compiler. The real overworld router flattens to **331 instructions**, most of them
`Const`, `Mul`, `Add`, `Min`, `YClamp` and `Noise`, with control flow for `RangeChoice` and
sub-program calls for the `interpolated` and `flat_cache` markers. The approach is feasible, and
that is worth knowing.

It does not, however, agree with the tree. **All 98,304 values disagreed** — a systematic error, not
a rounding one — and I had not isolated it when the round ran out. `squeeze` was one real bug (my
formula was a different curve from the tree's `clamp(v, -1, 1)` then `v/2 - v^3/24`); fixing it
changed nothing, so the cause is elsewhere, most likely in how the sub-program calls interact with
the value stack across `RangeChoice` branches.

I removed it. A non-functional evaluator is worse than no evaluator, and dead code that "almost
works" is worse still. The diagnosis and the 331-instruction result are recorded here so the next
attempt starts from them rather than from scratch.

### A second scripted edit, a second self-inflicted wound

Removing that module needed the example's wiring stripped, and I did it with a brace-matching
script rather than by hand. It matched the wrong block: a `rawcol` loop and a section header were
duplicated, and the Rust side began emitting **1,474,560 extra lines** — exactly one extra copy of
the per-route sweep. The parity suite caught it as a line-count mismatch rather than a value
mismatch, which is at least the right kind of failure, but it took two passes to unpick.

That is the second time a scripted structural edit has damaged this tree, after round 28 cut three
declarations out of `density.rs`. The lesson did not take the first time, so it is written down
here properly: **edit source by hand, with small anchored replacements that are verified after each
one. A script that removes a block by matching text does not know what the block is.**

### Three hypotheses, three negatives

The 12.9 ns per node visit was the whole remaining gap, so I attacked it three ways and all three
came back empty. The negatives are more useful than the attempts were.

**Enum size.** `DensityFunction` was 56 bytes — nearly a cache line — and `RangeChoice`'s five
boxed children and two `f64`s set that size, so *every* node paid it, `Constant` included. Boxing
`RangeChoice` and `ShiftedNoise` took it to 40. **Worth 1.7%** (258.3 to 254.0 ns/eval). The boxing
is kept because it is harmless and correct, but it bought nothing.

**Cache lookup.** Replacing the quart-corner map with a direct-indexed slot array — the pattern
that had already paid off once — was **no faster at all** (255.8 against 258.3). Written, measured,
reverted. It also produced a bug worth remembering: freshly `resize`d slots were tagged generation
0 while the counter started at 0, so every empty slot read as a hit holding `0.0` — 82,028 wrong
values, caught immediately by the `column` check.

**Thread-locals on the hot path.** Both `CHUNKED` and the visit counter were thread-locals read on
every node visit, which is not free. Moving them to plain statics gave **0%** — and **broke
multithreaded correctness**. The chunked reading is a property of one traversal, and the fill runs
one traversal per chunk across a rayon pool, so a global flag lets one thread's chunked walk leak
into another thread's raw one. Three FFI tests failed within seconds. Reverted; the thread-locals
were never the cost.

So the per-visit cost is not footprint, not lookup, and not the flags. At ~13 ns against vanilla's
~1.5 it is the price of a non-inlined recursive `match` over a boxed tree, where the JIT has a
monomorphic inlined call site. **Closing it is not a tweak.** It would mean a structurally
different evaluator — compiling the graph once into a flat instruction array and running that, so
the walk becomes an array sweep with no dispatch and no pointer chasing. That is a rewrite of the
evaluation core, and with the parity harness that now exists it is at least a rewrite that can be
checked.

### The lookup was never the bottleneck

Round 31's win came from giving `flat_cache` a memo. The natural next move was to make that memo
a direct-indexed array rather than a map — the same slot pattern that had worked before. It is
**no faster**: 255.8 ns/eval against 258.3. The rewrite is gone.

Two things came out of trying it, and both are worth more than the optimisation would have been.

**A per-variant tally**, now a permanent diagnostic. Counting visits by node type over a chunk:

```
Interpolated       19.06%
Constant           16.36%
Mul                13.06%
Add                10.05%
AbsWrapped          6.48%
RangeChoice         6.33%
```

There is no pathology to fix. `Interpolated` leads, but at 3.8 visits per block it is frequent
rather than expensive, and the rest are the cheapest nodes in the graph. **The cost is simply
spread across the walk**, which is what 19.8 visits at 12.9 ns looks like when nothing in
particular is slow.

**Two bugs in the attempt, both caught instantly by the `column` check.** Freshly `resize`d slots
were tagged generation 0 while the counter also started at 0, so every empty slot read as a hit
holding `0.0` — 82,028 wrong values. Fixing that left 484, from a pass-to-pass staleness the
example had because it walks a chunk by hand and never announces the boundary; `fill_chunk_chunked`
does, which is why the server was never affected.

So: vanilla spends ~1.5 ns per visit and this spends ~12.9, and the visits now match. That points
at memory locality — a graph of ~4,700 boxed nodes scattered across the heap, where every `Box`
dereference is a probable cache miss — and the fix for that is arena allocation, which is a
substantial change rather than a tweak.

### The algorithmic gap is closed; what is left is dispatch

Round 31 left the fill at 38 chunks/s against vanilla's 30.3 ns/block, about 9x apart. The obvious
next question was whether that is still too many node visits or too much per visit, and the
answer is the second.

Measured on the real fill path — cell order, not the `z, x, y` order the earlier probe used, which
empties the corner caches every eight blocks and describes something the fill never does:

```
RUST chunked: 98304 evals in 25 ms (258.3 ns/eval)
RUST visits:  1950082 for 98304 blocks = 19.8 node visits/block (13.0 ns/visit)
```

**19.8 visits per block. Vanilla makes roughly 20.** Two rounds ago this was 542. The library now
walks the same graph the same number of times, and the remaining factor of ~8.5 is that each visit
costs 13 ns where vanilla's costs about 1.5.

That is a much better problem than it was, and it has a likely cause: `DensityFunction` is **56
bytes**, nearly a full cache line, and `RangeChoice` — five boxed children and two `f64`s — is what
sets the size. Every `Box` dereference then pulls a fresh line. Boxing the two widest variants
(`RangeChoice`, `ShiftedNoise`) would bring the largest payload down to `YClampedGradient`'s 32
bytes, and that is the next thing to try.

I have not tried it yet. It is a change to the enum, both parse arms, the compute arms and the
tests, and after round 28's recovery I would rather start it at the top of a round than at the
bottom of one.

### `flat_cache` is not a cache — it is a snap, and that is the 30x

The keying question from last round was answered by reading `FlatCache.compute` properly rather
than by patching the keying.

```java
int k = QuartPos.fromBlock(ctx.blockX()) - this.firstNoiseX;
int l = QuartPos.fromBlock(ctx.blockZ()) - this.firstNoiseZ;
if (k >= 0 && l >= 0 && k < values.length && l < values.length) {
    return this.values[k][l];      // and it never reads ctx.blockY()
}
```

`firstNoiseX` is the **chunk's** first quart, so `values` is a 5 x **5 grid over the whole chunk**.
Two things follow, and I had been getting both wrong:

1. **The node reads as the value at its quart corner, not at the point asked for.** `flat_cache` is
   not a transparent wrapper in the chunk; it quantises to a 4 x 4 block grid.
2. **Therefore its subtree is evaluated 25 times per chunk**, not once per block.

Implementing the corner read alone moved nothing (it costs the same as the exact read) but took
the `column` check from 650 differences to **0**. Adding the 25-entry memo on top is where the
speed was:

```
before:  16203 ns/eval, 542.2 node visits/block,  11.1 chunks/s serial
after:    3260 ns/eval, 126.2 node visits/block,  38.4 chunks/s serial, 331 on 16 cores
```

**3.5x, and the terrain is byte-for-byte unchanged** — the in-server integrity hash is
`0x7ee6ba2f40f8c1c8`, the same value it had before any caching was attempted. All twelve checks
pass, including 1,574,394 chunked column values against vanilla's own `NoiseChunk`.

Against vanilla's 30.3 ns/block this is now about **9x slower rather than 34x**, and the remaining
gap is the five `interpolated` markers and the other `Marker` types, which are still passthroughs.

### 2.65x on the shelf, because it was wrong

Round 29 ended knowing the shape of the fix: make the cache lookup O(1), as vanilla's is. So I did
— a slot per (`flat_cache` node, in-cell x, in-cell z), tagged with a cell generation so
invalidation is a comparison rather than a clearing pass.

It worked, and it was wrong.

```
before:  16203 ns/eval, 542.2 node visits/block,  11.1 chunks/s serial
after:    4943 ns/eval, 178.6 node visits/block,  29.4 chunks/s serial
```

**2.65x faster and 3x fewer node visits** — and it changed **620 of the 98,304 chunked column
values**. The in-server benchmark's own integrity hash moved to prove it.

What makes this worth recording is that **all eleven suite checks stayed green**. The only thing
that caught it was the `column` diagnostic, which walks all 256 columns and compares against
vanilla's `NoiseChunk`, and which was sitting outside CI on a comment saying it "does not yet
agree". A check that catches a real regression does not belong outside CI, so it is now in the
list: the suite compares **1,574,394 values**, up from 17,517.

I also confirmed the assumption is sound before abandoning it. `NoiseChunk.FlatCache.compute`
reads only `QuartPos.fromBlock(x)` and `QuartPos.fromBlock(z)` — it never touches `y` — so these
nodes genuinely are 2D. The keying is therefore right in kind and wrong in detail, which is a
much narrower thing to fix than "the caches do not work". The 2.65x is real and is waiting on
that detail.

### 542 node visits per block

The 30x gap had to be explained, so rather than guess I made the evaluator count its own node
visits. One chunk's worth of the real router:

```
RUST chunked: 98304 evals in 1592 ms (16203.5 ns/eval)
RUST visits:  53298512 for 98304 blocks = 542.2 node visits/block (29.9 ns/visit)
```

**542 nodes per block**, at about 30 ns each. The per-node cost is not the problem — 30 ns for a
boxed, pointer-chasing enum dispatch is unremarkable. The problem is that there are **542 of
them where vanilla makes roughly 20**.

Vanilla gets there because its caches *return without descending*. `NoiseChunk.FlatCache`
materialises a 5 x 5 slice per cell and serves all 128 blocks of it; `Cache2D` keeps one value
per column; a `NoiseInterpolator` holds eight corners per cell. The subtree below a cache is
never walked. Here only the five `interpolated` markers are cached — 23,424 corner samples for
a whole chunk, a quarter of a visit per block — and the other 1,788 `flat_cache` and `cache_2d`
markers are passthroughs, so every heavy subtree under them is re-walked for every block.

That was the round-25 mistake seen clearly: caching those markers is the right idea, and it
removed 38% of the visits — 542 down to 336 — but the lookup was a linear scan over a growing
`Vec`, and it cost more than the visits it saved. Throughput went *down*, 11.2 to 10.0 chunks/s,
and the change was reverted.

**The shape of the fix is therefore known and is not "add a cache".** It is to make the lookup
free, the way vanilla's is: `Cache2D` is a single `(lastPos2D, lastValue)` pair, and `FlatCache`
is an indexed `double[25]` per cell — an O(1) slot, not a search. That is the next piece of work.

### The baseline, at last — and it says the opposite

With the toy graph gone, the fair comparison was finally possible: **one `NoiseChunk` for the
whole chunk**, driven exactly as `NoiseBasedChunkGenerator.fillFromNoise` drives it, with the
block-state rule reached through a `MethodHandle` rather than `Method.invoke`. Both of those
mattered — the first attempt built a chunk per *column* and reflectively invoked per block, and
dutifully reported 70 us per block, which measured the harness and nothing else.

```
note: vanilla chunk path: 294912 block states in 8 ms (30.3 ns/block)
```

**Vanilla: 30.3 ns per block, 2.67 ms per chunk.** And this is an *upper* bound on vanilla's
density cost, since `getInterpolatedState` also runs the aquifer and the block-state rule.

Against it:

| | per block | per chunk |
|---|---|---|
| vanilla, whole chunk path | 30.3 ns | 2.67 ms |
| this library, density only | ~926 ns | 91 ms |

**This library is roughly 30x slower than the code it was meant to accelerate.** It is a
correct reimplementation, and it is not an accelerator. That has been the open question since
round 1 and the answer is no.

The gap is not the corner sampling. Making the fill walk cells instead of blocks — so the
corner cache lives for a whole 4 x 8 x 4 cell, the way a `NoiseInterpolator` does — moved
throughput from 11.0 to 11.4 chunks/s. It is the ~30-node spine, traversed 98,304 times per
chunk, at something like 30 ns per node visit where vanilla spends about 1 ns. Vanilla gets
that by collapsing the spine through `FlatCache`, `Cache2D` and the interpolators; the one
attempt to do the same here (round 25) was unsound and, in any case, made no difference. So the
cause is still unidentified, and the two candidates are the caching strategy and the cost of
this enum's dispatch, in that order.

### A note on the method, and one self-inflicted wound

Removing the last round's diagnostic counters, done with a scripted edit against anchors rather
than by hand, cut three declarations out of `density.rs` — `ShiftKind`, `DensityFunction` and
`DensityBuilder` — and there is no version control in this tree. The file compiled to a pile of
`cannot find type` errors. It was recoverable only because the `impl` bodies survived and name
every field they use, so the declarations could be rebuilt from their own call sites; that took
about twenty minutes and could have cost the whole round. A scripted deletion against a text
anchor is not safer than a careful hand edit just because it is scripted.

### The benchmark was measuring a toy

Last round left a 7.6x gap between two measurements of the same work, and I could not say which
was wrong. It was the server, and for a mundane reason: `mcgen_world_create` built its world
from a hand-written `DEMO_DENSITY_JSON` — a graph with four node types and hardcoded noise
parameters — not from the real router.

So every in-server figure in this README, from the first round onward, was a demo benchmark.
The library now embeds the actual overworld `final_density`, its 1,100 noise occurrences and its
clamp bounds, assembled from the oracle's own output by `scripts/make-fixture.sh`. That script
reads `tools/oracle/cases/names.txt` and `clamps.txt`, which `check-parity.sh` now writes into
the repo rather than `/tmp`, so the fixture and the parity corpus are built from one source and
cannot drift apart.

The two measurements now agree, which is the point:

| | per chunk | per block |
|---|---|---|
| isolated Rust loop | 106 ms | 1086 ns |
| in-server serial fill | 91 ms | 926 ns |

and the honest headline:

```
fill   : 64 chunks  serial 11.0 cps | parallel 98.2 cps | speedup 8.93x (16 cpus)
noise  : 10,000,000 samples in 1032 ms -> 9.7 M samples/s
```

**11 chunks/s single-threaded, 98 on 16 cores** — for the real overworld router, with the real
seeding and the real clamp bounds. That is roughly 60x slower than the number this README
carried before, which says only that a toy graph is about 60x cheaper than a real one.

### A baseline I could not trust, and a gap I could not close

Chasing the 25x, the obvious move was to ask what the same work costs vanilla — 98,304
evaluations of the live `finalDensity` on the identical 256 x 384 grid. The answer came back
at **14.3 us per evaluation**, which is absurd: the game would be unplayable. My first attempt
was worse still (16.6 us) because it allocated a `FunctionContext` per iteration, so I hoisted
the context, warmed up, and re-measured. It did not move.

Timing the `NoiseChunk` path instead — the one the game actually uses — gave **70 us per
block**, five times slower than the raw graph. That is not a density measurement at all: the
oracle drives it through a reflected `getInterpolatedState.invoke(chunk)` once per block, and
that reflection dominates everything. So neither number isolates density work, and I removed
both rather than leave a baseline in the tree that would mislead the next person.

The measurement did surface something real, though, and it is unresolved. The Rust side, timed
in isolation over one chunk's worth of chunked evaluation:

```
RUST chunked: 98304 evals in 106 ms (1086.5 ns/eval)
```

The whole example agrees — 3.06 s for the timed chunk plus the raw-route sweep plus the column
loop. But the in-server benchmark reports 64 chunks of the same work at ~72 chunks/s, which is
**13.9 ms per chunk, or 142 ns per evaluation: a 7.6x gap** between two measurements of the same
operation on the same graph. One of them is wrong and I do not yet know which. Until that is
settled, the throughput figures in this README should be read as "whatever the two happen to
agree on", which is nothing in particular.

### The caches were the wrong lever

The plan was to recover the 25x by implementing `FlatCache` and `Cache2D`. I did, and measured,
and they did nothing — so they are gone.

Two findings, both worth keeping.

**They hit, and still didn't pay.** Instrumented over one chunk's worth of evaluation:

```
top=98304  corner_samples=23424  col_hits=1612133  col_misses=102930
```

94% hit rate on 1.7 million lookups, and the fill speed did not move: 70 chunks/s before,
69 with the caches, 72 after reverting them. The lookup itself costs about what the
subtree it saves costs to evaluate. Swapping Rust's default SipHash for a cheap hasher made
no difference either, so it was not hash cost. **The bottleneck is not the cacheable subtrees
— it is the 98,304 traversals of the outer graph spine per chunk**, which the coarse grid
avoided entirely by sampling 1,225 corners and interpolating. That is the 25x, and it is not
recoverable by caching the leaves.

**They were unsound, and CI caught it.** Keying on `(x, z)` alone is only valid for nodes that
do not vary with `y`. Vanilla gets away with it because `NoiseRouterData` *places* the
markers around expressions it knows are 2D; nothing in this parser establishes that, and the
corpus includes hand-built graphs where the assumption fails. The suite went red — 48 of 960
`density_json` values changed. I had a test for this that filled the same chunk with the
caches on and off and required identical bytes, but it only ran against the hand-built test
graph, which has no markers at all, so it passed vacuously and proved nothing. A green test
over an empty domain is worse than no test, because it reads like evidence.

### The FFI was shipping the approximation

The parity harness proved `compute_chunked`, but `WorldGen::fill_region` — the function behind
`mcgen_world_fill_region` — was still calling the coarse grid. The library's own benchmark was
therefore measuring an approximation under a vanilla name.

`fill_chunk_chunked` now does what a `NoiseChunk` does: per-block evaluation, with
`interpolated` markers sampling their eight corners once per 4 x 8 x 4 cell and lerping. The
per-cell cache is keyed by the node's address and the absolute cell coordinates, which is the
lazy equivalent of the `NoiseInterpolator` vanilla materialises up front.

**It costs 25x.** Serial fill went from 2,200 chunks/s to 70, parallel from 16,000 to 633. That
is the honest price of the coarse grid having been wrong, and it is not the final number:
the graph carries 1,071 `flat_cache`, 717 `cache_2d` and 19 `cache_once` nodes, and this parser
still treats every one of them as a passthrough. In vanilla those caches are what make the real
model affordable, and until they are implemented the correct path is doing roughly the work of
a vanilla generator with all its caches removed. Implementing them is the next piece of work,
and it should recover most of the gap.

I chose to ship the correct model and a worse number rather than keep a fast one that computes
something Minecraft does not.

### A correction: the boundary check is not a vanilla check

The mod logs

```
boundary: 1,572,864 bytes, rust=0x74be29b4fa7305b9 java=0x74be29b4fa7305b9 MATCH
```

and the word "java" there is misleading. The hash is `fnv1a` over the buffer that **the Rust
itself just filled** — it proves the bytes that crossed the FFI boundary match the fingerprint
Rust computed internally, and nothing more. It is a strong memory-integrity check and it is not
world-layer parity against Minecraft. The value changed this round because the fill changed,
and it still matches, which is exactly what it should do.

### SOLVED: `interpolated` is the interpolation, not the chunk

The previous round ended on "the cell grid is a different algorithm". Working out *what*
algorithm took one method: `NoiseChunk.wrapNew`.

```java
if (fn instanceof DensityFunctions.Marker marker) {
    switch (marker.type()) {
        case Interpolated  -> new NoiseInterpolator(this, marker.wrapped());
        case FlatCache     -> new FlatCache(this, marker.wrapped(), true);
        case Cache2D       -> new Cache2D(marker.wrapped());
        case CacheOnce     -> new CacheOnce(this, marker.wrapped());
        case CacheAllInCell-> new CacheAllInCell(this, marker.wrapped());
    }
}
```

**A `NoiseInterpolator` is created for `Marker.Type.Interpolated` nodes and for nothing
else.** The coarse cell sampling is a property of *that node*, not of the chunk. Everything
else in the graph is evaluated per block. So there is no separate cell-grid pass at all —
`final_density` is just a function, and the five `minecraft:interpolated` markers inside it
are where the 4 × 8 × 4 corners get sampled.

`DensityFunction::Interpolated` now does exactly that: `floor` into the cell, sample the
eight corners as `noise<dx><dy><dz>`, and feed them to `Mth.lerp3` in that order. The old
`CellGrid` — sample the whole density at corners and interpolate — was a different
approximation that happened to agree almost everywhere.

### The check had to be made to discriminate

Two readings of one graph are needed, and conflating them caused a round of confusion:

- `compute` leaves `interpolated` as a passthrough, which is what Java's `Marker.compute`
  does and what the direct per-route probes compare against;
- `compute_chunked` gives the marker its `NoiseInterpolator` behaviour, which is what the
  chunk actually produces.

The check was also widened from 5 columns to all 256, and that is what made it honest.
Running the *old* model against the *new* check:

```
old coarse-grid model vs 256-column chunked check:  98302 ok / 2 diff
new model:                                          98304 ok / 0 diff
```

**Two points out of 98,304.** The corrected model is strictly better, but only barely by
this observable — which is precisely why the sign test looked equivalent for two rounds.
The two failures are both at the chunk's very first block, `(0, -64, 0)`. The lesson is that
the sign of a density field is a very weak observable; almost any reasonable algorithm gets
it right almost everywhere.

### The cell grid is a different algorithm, not a slightly-off one

Chasing the value gap from the previous round turned up something bigger than a rounding
difference.

The plan was to compare the cell grid against the `NoiseInterpolator` that `NoiseChunk`
builds for `finalDensity`. Doing that properly meant matching the interpolator by
*reachability* rather than by guesswork — and the answer came back empty. **No interpolator
in the chunk wraps `finalDensity` at all.** The first attempt had silently latched onto a
`BlendDensity` whose input chain is a `YClampedGradient`/`Ap2` expression — a different
density function entirely, which is why its values were unrelated to the grid's.

That reframes the earlier result. Vanilla's `NoiseChunk` does not interpolate the whole
final density; it builds interpolators for the inner nodes that `NoiseRouterData` marks
`interpolated` (five of them in this graph), and evaluates everything else per block. My
`CellGrid` instead samples `finalDensity` at 5 × 49 × 5 corners and interpolates the whole
thing. Those are different algorithms that happen to agree in sign everywhere tested.

So the sign agreement at 9,600 points — solid and air never disagreeing, anywhere — is a
real and useful property, but it is **not** evidence that the interpolation is right. The
next step is to read how `NoiseChunk.wrapNew` decides which nodes become interpolators and
rebuild `CellGrid` around that, rather than around a single top-level interpolation.

### The interpolation is right in sign but not in value

The status table used to say the cell grid had "not yet been diffed against `NoiseChunk`".
It has now, and the result is more interesting than a pass.

`NoiseChunk` does not interpolate `finalDensity` itself. It first wraps it in
`DensityFunctions.blendDensity`, and that `BlendDensity` is what receives a
`NoiseChunk$NoiseInterpolator`. With `Blender.empty()` the wrapper is a passthrough, so the
interpolated value it holds should be exactly what the Rust cell grid computes. Feeding the
chunk itself as the `FunctionContext` — `NoiseInterpolator.compute` returns the interpolated
value for that context and delegates to the wrapped function for any other — makes the raw
value reachable.

Comparing that against `CellGrid::interpolate` over the same 9,600 block positions:

- **the sign agrees at 9,600 of 9,600** — solid and air never disagree, anywhere;
- **the value agrees at 0 of 9,600**, and not by rounding. First few pairs:

```
rust 0.03748242   java 0.1171875
rust 0.03850905   java 0.12040131
rust 0.03953569   java 0.12361512
```

The Java column steps by about 3.1× the Rust column's step, which points at the vertical
interpolation rather than the horizontal corners. So the cell machine is lined up and the
remaining gap is one stage inside `NoiseChunk` — most likely a `slide` or a second
interpolation layer applied before the `BlendDensity` that gets wrapped. Until that is
pinned, the check stays in its sign form, which is what CI runs.

### SOLVED: `final_density` now matches the live graph

The whole hunt is over, and the answer was one bytecode method.

`RandomState` wraps every density function in a visitor (`RandomState$1NoiseWiringHelper`)
whose `wrapNew` re-seeds the terrain noise:

```java
if (fn instanceof BlendedNoise)
    fn.withNewRandom(this$0.random.fromHashOf(ResourceLocation.withDefaultNamespace("terrain")));
if (fn instanceof EndIslandDensityFunction)
    fn = new EndIslandDensityFunction(val$seed);
```

`RandomState.random` is the *same* positional factory the noise names use — the one this
project already reproduces as `XoroshiroRandomSource(seed).forkPositional().fromHashOf(name)`.
The codec, meanwhile, rebuilds `BlendedNoise` from `XoroshiroRandomSource(0)` and never
mentions a seed, so the encoding loses it. Same shape as the clamp bounds: information that
lives in the *construction*, not in the text.

Supplying it took `final_density` from **8221/1379** to **9600/0**, and with it the whole
column:

```
rawcol_router_temperature        9600 ok / 0 diff
rawcol_router_continents         9600 ok / 0 diff
rawcol_router_depth              9600 ok / 0 diff
rawcol_router_ridges             9600 ok / 0 diff
rawcol_router_initial_density    9600 ok / 0 diff
rawcol_router_final_density      9600 ok / 0 diff
column (solid/air)               9600 ok / 0 diff
```

All fifteen routes and the full 384-block column now agree with the *live* graph bit for
bit — not with the encoding, with Minecraft.

Two smaller things fell out of the same method. `EndIslandDensityFunction` is seeded from the
world seed, which is why `end_islands` had to error loudly before: it now has a route to a
correct implementation. And the `noise_value_rs` diagnostic turns out to compare
*post-wiring* noises against the graph's *pre-wiring* parameters — a real vanilla quirk, not a
defect — so its 30 residual lines are expected.

### `System.err.println` goes nowhere in this environment

The single most useful thing found this round, and it was a tooling bug of mine rather
than anything about Minecraft.

**`System.err.println(...)` is silently swallowed when the oracle runs; `System.err.printf`
is not.** Every `println` diagnostic I have added to the oracle over several rounds has been
invisible — including a `catch` block that *was* firing and reporting the real reason a
comparison failed. Several "this cannot be happening in Java" dead ends were exactly this: a
line that ran, printed its explanation, and had the explanation dropped on the floor.

The lesson is cheap to state and was expensive to learn: when a diagnostic appears not to
run, check that it can be *seen* before concluding it was not reached.

Two smaller reflective facts, same flavour. `MulOrAdd` is a plain `final` class, not a
record, so `getRecordComponents()` returns null and a record-only accessor silently reads
nothing — it needs a declared-field fallback. And with the descent finally working, the pair
reads `MulOrAdd(argument=0.64, input=Marker)` against `Ap2(MUL, Constant(0.64), Marker)`,
which is arithmetically identical, over a walk that now finds class differences but **no
scalar differences at all**. So the operands are not the answer either, and the search
continues one level down.

### `mapAll` is blind, and the Rust had a real bug

Two findings this round, one of them a genuine defect in the Rust rather than in the
codec.

**`mapAll` never visits `Clamp`, `Mapped` or `MulOrAdd`.** They are `PureTransformer`s and
their `mapAll` does not apply the visitor to themselves. A reflective walk over the same
router sees 6 clamp nodes; `mapAll` sees zero — its census across all fifteen routes is
`Ap2, BlendAlpha, BlendDensity, BlendOffset, BlendedNoise, Constant, HolderHolder, Marker,
Noise, RangeChoice, ShiftA, ShiftB, ShiftedNoise, Spline, WeirdScaledSampler,
YClampedGradient` and nothing else. That is worth knowing beyond this project: anything
built on `mapAll` silently skips those node types, and some earlier reasoning of mine
(including part of what the 1,100-occurrence match proved) rested on not knowing it.

**A `clamp` node's JSON carries only its input.** Vanilla recomputes the bounds from the
input's *static range*; there is no `min_value`/`max_value` in the encoding at all. My Rust
was defaulting to `(-inf, +inf)`, i.e. **never clamping**. That is a straightforward
correctness bug, and it is now fed from an explicit side table — a reflective pre-order walk
emitting raw bit patterns, which the parser consumes in the same order.

Supplying those bounds did **not** fix `final_density`, so there is at least one more
difference and it is still not localised. The graph differ narrows it to the
`MulOrAdd`/`Ap2` pair, but it returns early on a class mismatch and therefore never looks
*inside* that pair; the descent added to fix that is not firing, which is where this stands.

### One construct, and a tool that had to be thrown away first

Round 17 found the last piece by first admitting a mistake. The bisect I built in round 15
compared each subtree against `parse(encode(subtree))` — but the codec's output is
**context-dependent**, so a subtree parsed alone is a *different graph* from that subtree
inside a larger expression. The tool therefore reported "no lossy subtree" while the two
graphs disagreed, and round 15's conclusion rested on it. It has been deleted.

Its replacement, `diffGraphs`, walks the two graphs *together* and descends only while the
classes match. Pointed at the live graph and at a properly rewired reparse, it reports
exactly one difference in the entire 335 KB graph:

```
DIFF rewired/argument1/input: class MulOrAdd vs Ap2
```

Everything else agrees. Every noise holder is correct, every scalar matches, the
structure is identical. **The whole of the remaining 1379-point disagreement is the
`MulOrAdd` ↔ `Ap2` expansion**: the live graph uses a specialised `MulOrAdd` node for
`mul(constant, x)` and `add(constant, x)`, and the codec re-expands it as a generic `Ap2`
whose `minValue`/`maxValue` are recomputed differently (`-0.158333…` against `-0.125`).

The same walk also explains why a naive reparse is far worse than this: its `NoiseHolder`s
come back as `Direct{…}` with `noise=null`, i.e. inert. That is a separate and fully
understood artefact — parsing without a registry cannot restore the noise instances — and
it is why the rewire step existed at all.

### The codec is not injective, and the mechanism is now pinned

Round 15 got to the bottom of it. `parse(encode(final_density))` re-wraps nodes in
`HolderHolder(Holder.Reference…)` — and those references do not resolve under a plain
`JsonOps` with no registry, so they compute something else. The evidence is a field-by-
field comparison of one node that disagrees:

```
input            live=Marker    reparse=HolderHolder   values differ 225/250
whenInRange      live=Constant  reparse=HolderHolder   values differ   0/250
whenOutOfRange   live=Ap2       reparse=HolderHolder   values differ 225/250
minInclusive     -1000000.0     -1000000.0             identical
maxExclusive     0.0            0.0                    identical
```

`Constant` is the tell: it is never holder-wrapped, and it survives untouched. Everything
that *is* re-wrapped changes value. `HolderHolder.compute` is a plain passthrough, so the
wrapper itself is harmless — what breaks is what it points at.

This also explains the two results that looked contradictory. Unwrapping every
`HolderHolder` before encoding is value-preserving (`live vs unwrapped = 0/250`), yet
`parse(encode(unwrapped))` still disagrees (`175/250`) — because the parse simply
**re-creates** the wrappers, and the re-created ones are the broken ones. And it explains
why a bisect that tests each subtree in isolation finds nothing: the codec's decision to
emit a reference is context-dependent, so a subtree parsed alone and the same subtree
parsed inside a larger graph do not agree.

The JSON text carries **no holder marker at all** (zero occurrences of `holder`,
`Holder`, `indirect` or `reference`), so the loss is invisible in the text and cannot be
repaired from it.

Round 16 put a number on the last piece. The oracle now prints the Java-against-Java
comparison as it runs:

```
note: JAVA-vs-JAVA: live final_density vs its own reparse differ at 1379 of 9600 points
```

which is *exactly* the count by which the Rust differs from the live graph. Two parsers,
written independently, landing on the same wrong answer from the same text — the text
does not contain the graph.

And the text is provably ambiguous rather than merely lossy: stripping every
`HolderHolder` first is value-preserving (`live vs unwrapped = 0/250`) and produces a
**byte-identical** encoding (335,302 chars both ways), yet parsing that encoding still
disagrees. `encode` maps two different graphs onto one string; `parse` cannot tell them
apart.

### The codec is not injective — and the loss is in composition, not in any node

A reflective bisect now walks the direct graph, and at every node compares it against
`parse(encode(node))`. It recurses through record components and lists, so it reaches
inside `Spline`'s `CubicSpline` and inside `HolderHolder`'s holder. It reports **no lossy
subtree anywhere** — every child round-trips faithfully.

And yet the root does not. Dumping its components shows why:

```
direct  type = MIN   argument1 = Mapped        argument2 = HolderHolder
after   type = MIN   argument1 = HolderHolder  argument2 = HolderHolder
```

`argument1` is a bare `Mapped` node in the live graph and comes back wrapped in a
`HolderHolder`. The same JSON, parsed deterministically, always chooses the wrapper — so
the text cannot distinguish the two forms, and roughly 3,764 such wrappers appear across
the graph (930 -> 4,694) while all 636 `MulOrAdd` nodes vanish into plain `Ap2`.

So the loss is **not attributable to a node type**. Each node on its own is fine; the
disagreement only appears once they are composed. That is a stronger and stranger
statement than "a node type is lost", and it is why the bisect comes back empty.

**The consequence stands and is uncomfortable:** every `density_json`, `density_dense`
and `density_router` check compares *parse against parse*, so none of them can detect any
of this, however many coordinates they sample. The 17,517 values are real, but they prove
the Rust agrees with the JSON — a weaker claim than "agrees with Minecraft". Closing it
needs the router graph to reach the Rust **by construction**, not by serialisation.

### The codec is not injective, and the router check cannot see it

This round settled where the round-12 contradiction lives, and it is not in the Rust port.

Java reconstructing `final_density` from **its own JSON** — parse the text, reattach
`RandomState`'s noise instances in traversal order — differs from Java's *direct*
evaluation at **exactly the same 1379 of 9600** chunk-local points where the Rust differs.
Two independently written implementations disagreeing with vanilla by the same amount, in
the same places, is about as clear as this kind of evidence gets: **the Rust is faithfully
implementing what the JSON says.**

What the JSON says, however, is not quite the real graph. `DIRECT_CODEC` is not injective:

```
encode(direct final_density)        = 335,302 chars
encode(parse(that))                 = 335,302 chars   -> byte-identical
```

yet the two graphs disagree on values. Structurally the round trip loses **all 636
`MulOrAdd` nodes** (they come back as plain `Ap2`) and gains **~3,764 `HolderHolder`
wrappers** (930 -> 4,694). `HolderHolder.compute` is a plain passthrough, so it is not the
culprit; the `MulOrAdd` expansion is presumably meant to be value-preserving, and either
it is not, or the noise association shifts when the node count changes even though the
occurrence *count* still lines up.

**The consequence is uncomfortable and worth stating plainly:** every `density_json`,
`density_dense` and `density_router` check compares *parse against parse*. They are
therefore structurally incapable of detecting a lossy codec, no matter how many
coordinates they sample. The 17,517 values above are real, but they prove the Rust
agrees with the JSON — which is a weaker claim than "agrees with Minecraft".

Closing this needs the router graph to reach the Rust **by construction** rather than by
serialisation — a different mechanism, not a bigger sample.

### Dense coordinates, and a contradiction that is still open

The checks above sample a fixed handful of coordinates. That is fine for a scalar, but
it means a whole class of bug can hide: **the sparse coordinates never land where a
chunk generator actually walks.** So there is now a `density_dense` check that runs every
one of the 36 corpus graphs over a 6 x 6 x 10 grid of chunk-local coordinates —
**14,400 values, bit-exact**, including the entire `final_density` graph and the node
types (`interpolated`, `blend_density`) the sparse set never touched.

That is a genuinely stronger statement than the sparse numbers were, and it narrows the
remaining problem sharply. Probing every one of the fifteen router routes at chunk-local
coordinates:

```
rawcol_router_temperature        9600 ok / 0 diff
rawcol_router_continents         9600 ok / 0 diff
rawcol_router_depth              9600 ok / 0 diff
rawcol_router_ridges             9600 ok / 0 diff
rawcol_router_initial_density    9600 ok / 0 diff
...  14 of 15, all perfect ...
rawcol_router_final_density      8221 ok / 1379 diff
```

**Fourteen routes are perfect across 9,600 points each. Only `final_density` differs.**
And every plausible cause has been eliminated:

* The encoded graph is byte-identical to the one `RandomState` hands out.
* All 25 of its noises, seeded per name, are bit-identical to
  `RandomState.getOrCreateNoise` — checked value by value.
* All 1,100 noise *occurrences* are associated with the right registry name, checked
  occurrence by occurrence rather than at first sight of each name.
* The whole `final_density` graph is bit-exact over dense coordinates when its noises
  are seeded uniformly.

So the graph is right, the noises are right, and the association is right, yet the
combination is not. That is a contradiction, and it is the honest state of this check. I
have not resolved it. The next experiment should compute `final_density` in the oracle
from the parsed JSON fed `RandomState`'s own noise instances — which is exactly what the
Rust does — and see whether Java reproduces its own direct evaluation. If it does not,
one of the four bullets above is subtler than it looks.

### The router corpus was lossy, and the fix exposed a second blind spot

**Fixed this round.** The corpus now carries a registry name for **every noise
occurrence**, in the order the graph is walked, instead of one name per distinct
parameter set. The Rust parser consumes them left to right and checks each against the
parameters it parsed, so a traversal-order mismatch fails loudly rather than silently
attaching the wrong noise. `final_density` then matches bit-for-bit.

That fix immediately paid for itself, because it exposed something worse:

**`density_router` samples 24 coordinates, and none of them lie anywhere near the
coordinates the cell grid uses.** The two sets are disjoint. So the "all fifteen router
functions bit-exact" result was always far weaker evidence than it read as — the probe
simply never visited the interesting region.

Re-probing at chunk-local coordinates (x and z in `0..15`) shows a real discrepancy in
the raw, uninterpolated density: **1379 of 9600 points differ**, by up to 14% relative.
The graph is right (the same function matches at the original 24 points), so the fault is
in how a *noise value* is evaluated at those coordinates — or in the per-occurrence name
association, where eight distinct registries share the parameters `-8 [1.0]`, so a
traversal-order difference there would swap names without the parameter check noticing.

This is the current frontier, and it is why the sampled checks above should be read as
"no counterexample found" rather than "proven".

### A blind spot the chunk probe found in my own corpus

The `density_router` check compares the real 1.21.1 overworld router against its own
JSON encoding. **That encoding is lossy**, and I had been treating it as faithful.

`final_density` alone references **25 distinct noises that collapse to 12 distinct
parameter sets.** The codec records a noise's *parameters*, not its registry name, and
distinct registries routinely share parameters — `minecraft:ore_veininess` and
`minecraft:spaghetti_roughness` are both `-8 [1.0]`. A reconstruction keyed on
parameters therefore cannot recover which noise feeds which node. The oracle now
prints this fact when it runs.

This stayed invisible for eight rounds because the oracle seeds *every* noise with the
same fixed `0`. Under uniform seeding, collapsing 25 noises into 12 changes nothing at
all, so the check passed. It only surfaces once noises are seeded per name, the way a
real chunk does. The chunk-level probe found it in a single afternoon.

The fix is to encode the corpus through registry ops, so noise references serialise as
names rather than inlined parameters — which is also how real datapacks write them. Until
then, `density_router` proves the *arithmetic and graph shape* of all fifteen router
functions, but not that the graph is fed the right noise per node.

**Not proven — the remaining gap is terrain *shaping*.** Real terrain also runs
aquifers, ore veins and surface rules, and vanilla samples a coarse cell grid and
interpolates between cells rather than evaluating every block. Our kernel does
none of that, so chunk *contents* still differ even though the density field
underneath them is now exact.

**Every density node type the overworld uses is now implemented.** Two remain,
both for things other than overworld terrain: `find_top_surface` (the surface
system) and `end_islands` (the End). They fail loudly at parse time rather than
approximating, so a graph containing one cannot silently produce wrong terrain.

Two things are approximated. Vanilla's `RandomState` seeds the root source from
the level seed in its own way, which we do not replicate — the derivation
*mechanism* is right, the root seeding is not. And the `NoiseParameters` are not
yet read from vanilla's own data.

So: **the noise and density layers are exact and the plumbing is proven; the
terrain is not.** Nothing here should be pointed at a world you care about.

---

## Quick start

```bash
scripts/fetch-jdk.sh      # Temurin JDK 21 into .tools/ (no sudo needed)
scripts/build-all.sh      # Rust tests + release build + FFM harness   (seconds)
scripts/setup-server.sh   # install NeoForge 21.1.257, build + stage the mod
scripts/check-parity.sh   # diff against the REAL Minecraft classes    (the important one)
scripts/run-server.sh     # boot the server, self-test, print results
```

`build-all.sh` is the fast inner loop — no Minecraft, no NeoForge, no gradle.
`check-parity.sh` is the one that matters: it runs the actual Mojang code.
`run-server.sh` is the real thing: an actual modded server.

---

## Proof it works

### Bit-exact parity against real Minecraft

`scripts/check-parity.sh` compiles `tools/oracle/VanillaOracle.java` against the
Minecraft classes in the server install and diffs its output with the Rust
equivalent, comparing raw `f64` bit patterns so a single differing mantissa bit
fails the check:

```
########## improved_noise ##########
  PASS: 144/144 values bit-identical to real Minecraft

########## xoroshiro ##########
  PASS: 288/288 values bit-identical to real Minecraft

########## normal_noise ##########
  PASS: 576/576 values bit-identical to real Minecraft

########## hash ##########
  PASS: 10/10 values bit-identical to real Minecraft

########## math ##########
  PASS: 144/144 values bit-identical to real Minecraft
########## density ##########
  PASS: 432/432 values bit-identical to real Minecraft

########## density_noise ##########
  PASS: 168/168 values bit-identical to real Minecraft

########## density_json ##########
  PASS: 960/960 values bit-identical to real Minecraft
########## density_dense ##########
  PASS: 14400/14400 values bit-identical to real Minecraft

########## density_router ##########
  PASS: 360/360 values bit-identical to real Minecraft
########## noise_seed ##########
  PASS: 35/35 values bit-identical to real Minecraft

=== PARITY OK: Rust matches Minecraft 1.21.1 bit for bit ===
```

`density_json` is the strongest of the seven: both sides parse the **same JSON
file** — [`tools/oracle/cases/density.jsonl`](../tools/oracle/cases/density.jsonl) —
so it exercises the *parser* as well as the evaluator. It now carries 36 graphs. The corpus starts from
vanilla's own serialisation of each node type (obtained via `density_encode`,
which prints `DIRECT_CODEC.encodeStart(...)`), so the schema is authoritative
rather than guessed, and adding a node type to the harness costs one line.

`density_router` goes further: its corpus is not hand-built at all. It is the
**real 1.21.1 overworld router**, obtained by running Minecraft's own data
generators (`tools/oracle/RouterOracle.java` implements the two-method
`BootstrapContext`, runs the noise / density-function / generator-settings
bootstraps in dependency order, and pulls the OVERWORLD router out). **All fifteen** router
functions verify bit-for-bit, including `initial_density` (95 KB of JSON),
`depth` (56 KB) and `final_density` (335 KB).

### What the oracle caught

**17,517 values, all bit-exact** (eleven checks) — and every one of the bugs below was found *only*
by diffing against the real classes. None would have been caught by a unit test,
and all of them would have shipped as subtly wrong terrain:

| Bug | Symptom |
|---|---|
| `valueFactor` folded into per-octave amplitudes instead of applied once at the end | every normal-noise value off by 1 ULP |
| `INPUT_FACTOR` baked into octave frequencies instead of applied at the call site | 1 ULP, subtly wrong |
| Octave streams derived by custom hash instead of `forkPositional().fromHashOf("octave_N")` | completely different noise |
| Density sampled at block *centres* (`+0.5`) instead of integer block coordinates | systematic half-block terrain offset |
| `shift_b` passing `(z, 0, x)` instead of `(z, x, 0)` | wrong cave/biome offsets |
| `shifted_noise` applying the shift before scaling instead of after | wrong terrain |
| `minecraft:constant` parsed as `{"value":…}` instead of `{"argument":…}` / a bare number | real datapack JSON would not load at all |
| nested spline point values parsed as `{"type":"minecraft:spline","spline":…}` instead of the bare body `{"coordinate":…,"points":…}` | splines with sub-splines would not load |
| noise references assumed to always be name strings | the real router inlines `{firstOctave, amplitudes}` instead, so nothing parsed |
| `shift_a`/`shift_b` `argument` assumed to be a name | same inline-parameter problem |
| `blend_alpha` / `blend_offset` unimplemented | the whole graph failed to load |
| `BlendedNoise` multipliers read as the raw scale instead of `684.412 * scale` | noise wrong by a factor of ~2700 |
| `getOctaveNoise(i)` read as a plain index instead of `noiseLevels[len-1-i]` | octaves consumed in reverse |
| `forkPositional()` assumed not to consume randomness | every derived noise seed wrong |
| The router corpus assumed faithful when the JSON codec is lossy (25 noises collapse to 12 parameter sets) | invisible under uniform seed-0 seeding; surfaced only at chunk level |
| `fromHashOf` assumed to be an MD5 digest (`XoroshiroPositionalFactory`) when the factory was a legacy one | wrong seeds; caught by asking vanilla which algorithm the overworld uses |

That is the case for building the oracle before building the accelerator.

This is the difference between "I ported the algorithm" and "I ported the
algorithm correctly". A 1-ULP error here would have been invisible to every
unit test in the repo and would have shown up as chunk seams in game.

### In a real NeoForge server

`scripts/run-server.sh` against a real NeoForge 21.1.257 server:

```
[mcgen] loading /…/server/mcgen/libmcgenjni.so
[mcgen] ABI OK (v1), native library live inside NeoForge
[mcgen] boundary: 1,572,864 bytes, rust=0x74be29b4fa7305b9 java=0x74be29b4fa7305b9 MATCH
[mcgen] fill   : 64 chunks  serial 69.6 cps | parallel 603.4 cps | speedup 8.67x (16 cpus)
[mcgen] noise  : 10,000,000 samples in 1433 ms -> 7.0 M samples/s
[mcgen] RESULT hash=0x74be29b4fa7305b9 chunks=64 serial_cps=69.6 parallel_cps=603.4 speedup=8.67 noise_mpps=7.0
```

`scripts/build-all.sh` against the same library through FFM:

```
[3] boundary integrity
    rust hash     : 0x74be29b4fa7305b9
    java hash     : 0x74be29b4fa7305b9
    PASS: 1.5 MB crossed the FFI boundary byte-for-byte intact

[5] parallel vs serial fill
    serial      : 28.80 ms  -> 2,221.9 chunks/s
    parallel    :  3.79 ms  -> 16,868.6 chunks/s
    speedup     : 8.56x on 16 cpus
```

Two independent bindings (FFM and JNI) producing the **same hash** is the real
signal here: it means both are calling the same native code, and that terrain
crosses the boundary without corruption.

---

## Architecture

```
NeoForge 21.1.257 server (JVM, Java 21)
   │  owns world state, ticking, mixins, ALL mod logic
   │
   ├── FFM (java.lang.foreign)   -> libmcgen.so    [harness, needs --enable-preview]
   └── JNI (jni_shim.c)          -> libmcgenjni.so [in-server, no preview flags]
                                        │
                                        ▼
                              mcgen-core (pure Rust)
                              noise · density eval · terrain fill
```

Three decisions that matter more than they look:

**One call per region, never per block.** Crossing the boundary is expensive;
the work behind it must be large. A per-block callback would turn an 8× win into
a large loss. Every exported function takes an `n × n` chunk request.

**JNI for the server, FFM for tooling.** FFM is much nicer, but it is a *preview*
feature in Java 21 ([JEP 442](https://openjdk.org/jeps/442)) and only final in
Java 22 ([JEP 454](https://openjdk.org/jeps/454)). NeoForge 1.21.1 targets Java
21, so an FFM-based mod would force every admin to add `--enable-preview` to
their launch arguments. The mod therefore uses a ~130-line JNI shim; the harness
uses FFM because we control the flags there.

**A note on the oracle's classpath.** The vanilla jar ships obfuscated
single-letter classes, including a top-level `com.class`. If the intermediate
`-unpacked`/`-slim` jars are on the classpath, that class shadows the entire
`com.*` package and `javac` reports the memorable `cannot find symbol: variable
mojang`. `check-parity.sh` excludes them; expect to rediscover this if you build
your own classpath.

**A panic must never reach the JVM.** Since Rust 1.81, unwinding out of an
`extern "C"` function aborts the process — for a game server, that means killing
someone's world. Every entry point wraps its body in `catch_unwind` and returns
an error code instead. The mod additionally treats *any* failure as
non-fatal: during development the shim was mis-linked and the server booted
anyway, logging `self-check failed, continuing without acceleration` instead of
dying. That behaviour is intentional and tested.

---

## What's in here

```
rust/
  mcgen-core/        pure Rust worldgen: noise, density functions, terrain, RNG
  mcgen-ffi/         C ABI (cdylib for FFM, staticlib for the JNI shim)
java/
  harness/           standalone FFM harness: 7 checks incl. benchmark + integrity
  neoforge-mod/      the NeoForge mod
    native/jni_shim.c
    stubs/           compile-only NeoForge stubs (never packaged)
scripts/             fetch-jdk, build-all, build-mod, setup-server, run-server
tools/javaref/       JVM program that generates ground-truth RNG values
server/              the installed NeoForge server (gitignored)
```

### Why compile-only stubs instead of NeoForgeGradle?

The mod touches exactly two NeoForge types: the `@Mod` annotation and the types
it injects into the constructor. Java resolves both by name at runtime, so a
stub with the right fully-qualified name is interchangeable with the real class.
Compiling against stubs takes **seconds**; a cold NeoForgeGradle/neoform run
takes tens of minutes because it decompiles and recompiles Minecraft. The build
asserts the stubs never leak into the jar.

---

## Tests

```bash
cd rust && cargo test
```

66 tests (58 in `mcgen-core`, 8 in `mcgen-ffi`), including ones that are not just
"does it run":

- RNG output asserted against values generated by the **real JVM**
  (`tools/javaref/RefValues.java`) — so the LCG is anchored to truth, not to my
  reading of it.
- Parallel and serial region fills must agree **byte for byte**.
- Repeated runs must produce identical output (catches data races).
- Unsupported density-function types must **fail loudly**, because silently
  approximating a node would silently generate the wrong world.
- Cell-grid interpolation must reproduce a sampled value **bit-for-bit at every
  cell corner**, and the interpolated fill must still differ from the direct fill
  somewhere — so neither a wrong layout nor a degenerate no-op can pass.
- `getOctaveNoise` must index from the end; out-of-range must be `None`.
- Bad arguments (short buffer, null handle, non-positive chunk count) must be
  rejected, not ignored.

---

## Next steps, in order

1. ~~`XoroshiroRandomSource` + splitters~~ — **done**, bit-exact.
2. ~~A vanilla parity harness~~ — **done**; `scripts/check-parity.sh` diffs against
   the real classes. Extend it with every new layer: it is far cheaper to debug a
   scalar than a chunk.
3. ~~Cell-grid interpolation~~ — **done**; vanilla's 4 × 8 × 4 grid, with the
   arithmetic bit-exact. Still worth diffing against a real `NoiseChunk`.
4. ~~Replicate `RandomState` root seeding~~ — **done**; the derivation chain is
   bit-exact (16/16 noises, four router graphs 24/24 each).
5. ~~Re-encode the router corpus so noise references survive as names~~ — **done**, by
   emitting one name per occurrence with a parameter cross-check.
6. ~~Settle `MulOrAdd`~~ — closed. It was never `MulOrAdd`: the whole of the remaining
   `final_density` disagreement was `BlendedNoise`'s lost seed (see above). The
   `MulOrAdd`/`Ap2` class difference is a genuine re-expansion, but it is arithmetically
   identical and was a red herring for the last four rounds.
7. **Reconsider the old step 7.** `transform` is `d * argument` and `argument1()`/`argument2()` are
   `(Constant(argument), input)`, which is arithmetically identical to the `Ap2` it becomes —
   so the remaining difference is *not* the arithmetic and must be found by descending into
   the pair rather than classifying it. Fix the descent in `diffGraphs` first; it returns
   early on a class mismatch and that is exactly the branch that matters here. Until this
   lands, treat every `density_*` result as "agrees with the JSON", not "agrees with
   Minecraft".
7. **Aquifers / ore veins / surface rules**, on top of the now-correct cell grid.
8. **Only then**: replace `NoiseBasedChunkGenerator` via mixin and measure
   end-to-end server tick time.

Worth knowing before investing: several people are already on this exact hill.
[tritium](https://github.com/Bumseltag/tritium) (Rust, GPU-accelerated, with a
datapack loader and an out-of-game previewer),
[worldgen-rs](https://github.com/garfxld/worldgen-rs) (faithful Rust port of
vanilla worldgen up to `ChunkStatus::Surface`), and
[tungsten-mc-compile](https://docs.rs/tungsten-mc-compile/latest/tungsten_mc_compile/)
(compiles datapack density functions to Rust/CUDA) are all worth reading first.
All are early-stage; the mature implementation of this idea today is Java
(C2ME).

---

## Scope reality check

This accelerates **worldgen**: fresh-world exploration, pregeneration, save/load.
It does **not** make a heavy modpack tick faster. On a large 1.21.1 pack, MSPT
problems are usually mod tick handlers and GC, not terrain generation. If your
pain is `Can't keep up!` during normal play rather than during exploring or
pregen, this is the wrong lever.

---

## Licence and attribution

GPL-3.0-or-later (see `LICENSE-NOTICE.md`).

The noise structure — octave layout, normalization constants, gradient table,
the "legacy fBm" shape — was written by studying
[Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) (`crates/pumpkin-util/src/noise/perlin.rs`),
which is itself a port of Minecraft's `ImprovedNoise`/`PerlinNoise`/`NormalNoise`.
Pumpkin is GPL-3.0, so this project is GPL-3.0 too. Our implementation differs
from Pumpkin's in one deliberate way: it stays in `f64` throughout, matching
vanilla's `double` math, whereas Pumpkin casts to `f32` early. That is a
correctness choice for a parity goal, and it is also why we cannot cross-check
against Pumpkin's fingerprint.
