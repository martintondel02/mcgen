# `/mcgen` command reference

Permission level 0 commands are safe for anyone; level 2 commands do real work and are
limited to operators. All commands are **read-only** — nothing here modifies a world or
a config.

---

## `/mcgen`

Lists the available commands and prints a one-line status. Permission 0.

---

## `/mcgen status`

Is the native library loaded, from where, and which seed is the self-test world using.
Permission 0.

```
[mcgen] library : /srv/minecraft/mcgen/libmcgenjni.so
[mcgen] ABI     : v2 (loaded)
[mcgen] self-test world seed: 1779920288596261407
[mcgen] this level seed     : -1031226922 (a world is built on demand when needed)
```

If loading failed, the reason is printed here instead.

---

## `/mcgen noises`

Lists every noise field the loaded density graph references, in index order. Permission 0.

```
[mcgen] 25 noises referenced by the loaded graph:
[mcgen]   [0] minecraft:offset
[mcgen]   [1] minecraft:continentalness
[mcgen]   [2] minecraft:erosion
...
[mcgen]   [24] minecraft:noodle_ridge_b
```

Useful for confirming which datapack actually contributed to a graph.

---

## `/mcgen density <x> <y> <z>`

Evaluates the density graph at one world position. Permission 0.

```
[mcgen] density at (0, 64, 0) = 0.198951363775  -> this position is solid
```

Two things to know:

- This is the **unchunked** reading. Inside a chunk, the `interpolated` markers blend over
  4×8×4-block cells, so a value near a cell edge can differ from what the generator
  actually uses there.
- "solid" means the density is above zero. It does not mean "stone" — the block placed
  also depends on aquifers and surface rules, which this engine does not implement.

---

## `/mcgen verify`

Re-runs the startup self-test on demand: generate a 2×2-chunk region, hash it in Rust,
hash the same bytes in Java after they cross JNI, compare. Permission 2.

```
[mcgen] pipeline: 393,216 bytes  rust=0xe13e0d2363bf6925  java=0xe13e0d2363bf6925  MATCH
```

**This is a boundary-integrity check, not a correctness check.** A `MATCH` means the bytes
crossed intact. For correctness, use `/mcgen chunk`.

---

## `/mcgen bench [chunks]`

Fills an *n*×*n* chunk region serially and in parallel and reports throughput. Permission 2.
`chunks` is the side length, 1–32, default 4.

```
[mcgen] serial   : 59.5 chunks/s  (16.8 ms/chunk)
[mcgen] parallel : 233.5 chunks/s  (4.3 ms/chunk) on 16 cpus
[mcgen] speedup  : 3.92x
```

Each figure is the best of three runs after two warm-up passes, so it is not measuring
page faults. Larger `chunks` values parallelise better but allocate more: an *n*×*n* region
is `n² × 98,304` bytes, so `32` needs about 100 MB of direct memory.

---

## `/mcgen chunk <chunkX> <chunkZ>`

**The useful one.** Compares this engine against a chunk the server has already generated,
for the server's own world seed, block by every block. Permission 2.

The chunk must be **already loaded** — this command never generates one. Move within view
distance of it first.

```
[mcgen] chunk 0,0  seed -1031226922  —  97,985 of 98,304 positions agree (99.675%)
[mcgen]   solid (non-air, non-fluid): engine 29,828  server 29,889   |  fill 17.7 ms, scan 8.4 ms
[mcgen] mismatches by depth (engine-only = engine says solid, server air):
[mcgen]   y   -64..  -17 :      96  (engine-only 96 / server-only 0)
[mcgen]   y   -16..   31 :      34  (engine-only 33 / server-only 1)
[mcgen]   y    32..   79 :     189  (engine-only 0 / server-only 189)
[mcgen] server block at those positions: minecraft:spruce_leaves x165, minecraft:air x96, minecraft:cave_air x23, minecraft:spruce_log x14
[mcgen] of those, 298 are blocks a later stage placed (trees, carved cave air) and
[mcgen] 21 are the density layer's own: 99.979% agreement there.
```

### How to read it

Minecraft generates in stages. Density decides whether a position is solid; carvers then
cut caves; features then plant trees. This engine implements **only the density stage**.

So a disagreement is only interesting if it is the density layer's fault:

- **A tree where the engine says air** is not a density error. The block name in the output
  tells you this is `spruce_leaves` or `spruce_log`.
- **Carved cave air where the engine says solid** is not a density error either.
- **Anything else** — stone, dirt, water, lava appearing where they should not — *is* a
  density error, and is worth reporting.

The command separates these for you, which is why it prints both a raw agreement figure
and an agreement figure for the density layer's own decisions.

### What the comparison treats as "solid"

The engine answers exactly one question: is the density above zero. So the comparison
treats a server block as solid only if it is neither air nor a fluid. Water and lava count
as *not* solid — vanilla's aquifers place them where the density is negative, so counting
them as solid would report the entire ocean as a disagreement. It did, before this was
fixed.

### Cost

Roughly 25 ms of server-thread time per call: about 17 ms to generate the chunk in Rust
and 8 ms to walk the server's blocks. It will show up as one tick spike. Do not put it in
a repeating command block.

### On a modded world

Mismatches are expected and are the point. If a datapack changes the density functions,
this engine — which implements the *vanilla* overworld router — will disagree, and the
disagreement is a measurement of what the datapack did.
