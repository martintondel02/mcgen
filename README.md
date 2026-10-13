# mcgen — a Rust world-generation engine for NeoForge

A Rust implementation of Minecraft's world-generation **density layer**, running inside a
real NeoForge 21.1.257 (Minecraft 1.21.1) server, driven by the `/mcgen` command tree.

It is bit-for-bit faithful to vanilla. It is **not fast**, and that is the first thing to
know — see [Status](#status).

```
/mcgen chunk 0 0
[mcgen] chunk 0,0  seed -1031226922  —  97,985 of 98,304 positions agree (99.675%)
[mcgen] of those, 298 are blocks a later stage placed (trees, carved cave air) and
[mcgen] 21 are the density layer's own: 99.979% agreement there.
```

That output is real, from a vanilla world on a live server, comparing this engine against
Minecraft's own chunk.

---

## Status

| | |
|---|---|
| **Correctness** | ✅ **1,591,911 values bit-identical** to Minecraft 1.21.1, across twelve independent checks |
| **In-server** | ✅ loads in NeoForge 21.1.257, commands work, matched a live vanilla chunk to 99.979% |
| **Speed** | ❌ **roughly 5-6x slower than vanilla's own code.** It does not accelerate anything. |
| **Chunk generation** | ❌ not replaced. Read-only inspection and benchmarking only. |
| **Aquifers / ores / surface rules** | ❌ not implemented |

**If you are looking for a faster server, this will not give you one.** The engine
reproduces vanilla's density layer and beats it on nothing. It is published because it is
a faithful, inspectable, verifiable implementation with a working FFI and command
surface, and because the honest account of why it is slow is worth more than another
"faster worldgen" claim.

See [PARITY.md](docs/PARITY.md) for exactly what is proven, and
[DEVELOPMENT.md](docs/DEVELOPMENT.md) for the twelve optimisations that were attempted
and what each one did.

---

## What you can actually do with it

- **Validate worldgen.** `/mcgen chunk <x> <z>` compares this engine's solid/air verdict
  against a chunk the server has already generated, for the server's own seed, block by
  block, and tells you *what kind* of block accounts for each disagreement. On a world
  with a worldgen datapack, the differences are the datapack's effect.
- **Inspect the density graph.** `/mcgen noises` lists every noise field the loaded graph
  references; `/mcgen density <x> <y> <z>` evaluates it at a point.
- **Benchmark.** `/mcgen bench [chunks]` reports chunk-fill throughput, serial and
  parallel.
- **Check the FFI boundary.** `/mcgen verify` regenerates a region and confirms the bytes
  crossed JNI intact.

---

## Install

You need a **NeoForge 21.1.257 server for Minecraft 1.21.1** and a **64-bit Linux** host.
Nothing else — no `--enable-preview`, no special JVM flags.

1. Build the jar (or take one from a release):

   ```bash
   git clone https://github.com/martintondel02/mcgen
   cd mcgen
   ./scripts/fetch-jdk.sh      # downloads a JDK 21 into .tools/
   ./scripts/setup-server.sh   # installs NeoForge, builds the mod
   ```

2. Copy two things into your server:

   ```
   build/mod/mcgen-0.2.0.jar   ->  <server>/mods/
   build/mod/libmcgenjni.so    ->  <server>/mcgen/
   ```

   The native library is deliberately *not* inside the jar: a jar with an embedded `.so`
   cannot be loaded identically on every platform, and extracting a native library to a
   temp directory at runtime is how mods acquire security advisories.

3. Start the server. You should see:

   ```
   [mcgen] ABI OK (v2), native library live inside NeoForge
   [mcgen] boundary: 1,572,864 bytes, rust=0x7ee6ba2f40f8c1c8 java=0x7ee6ba2f40f8c1c8 MATCH
   [mcgen] commands registered: /mcgen
   ```

4. In game or from the console, run `/mcgen`.

Full details, including how to point the mod at a different library path, are in
[INSTALL.md](docs/INSTALL.md).

---

## Commands

| Command | Permission | What it does |
|---|---|---|
| `/mcgen` | 0 | List the commands |
| `/mcgen status` | 0 | Is the native library live, and where from |
| `/mcgen noises` | 0 | List the noise fields the loaded graph references |
| `/mcgen density <x> <y> <z>` | 0 | Density value at a world position |
| `/mcgen verify` | 2 | Re-run the FFI pipeline self-test |
| `/mcgen bench [chunks]` | 2 | Chunk-fill throughput, serial and parallel |
| `/mcgen chunk <cx> <cz>` | 2 | Compare against a loaded server chunk, block by block |

`/mcgen chunk` never generates a chunk; it only reads one that is already loaded, so it
cannot stall your server generating terrain behind your back. It does hold the server
thread for roughly 25 ms per call.

Full reference with example output: [COMMANDS.md](docs/COMMANDS.md).

---

## How it works

```
   NeoForge server (JVM)
     └── mcgen-0.2.0.jar          @Mod entry point + /mcgen commands
           └── jni_shim.c          thin JNI → C ABI translation
                 └── libmcgenjni.so   statically links…
                       └── mcgen-core  Rust: noise, seeding, density graph, chunk fill
```

The Java side never sees the density graph. It asks for whole chunks; the boundary is
coarse-grained on purpose, because a per-block callback across JNI would cost more than
the generation.

There is a second, optional boundary: a standalone FFM harness
([java/harness](java/harness)) using Java 21's foreign-function API. It exists because FFM
is the future and JNI is what actually ships — FFM is still a preview feature in Java 21,
and forcing every server admin to add `--enable-preview` would be a bad trade.

Details: [ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## Building from source

```bash
./scripts/fetch-jdk.sh        # JDK 21 into .tools/ (NeoForge 1.21.1 needs 21, not 25)
./scripts/setup-server.sh     # NeoForge server + mod jar + native library
./scripts/check-parity.sh     # the twelve parity checks against real Minecraft
./scripts/test-commands.sh    # boots the server and drives every /mcgen command
cargo test --manifest-path rust/Cargo.toml
```

`setup-server.sh` downloads the official NeoForge installer and a Minecraft server jar.
Neither is redistributed here, and neither is committed.

The mod compiles against the **real** NeoForge and Minecraft jars that the server
installer produced, not against hand-written stubs — a stub that is subtly wrong compiles
cleanly and fails at runtime, and the `/mcgen` tree touches enough API surface to make
that a real risk.

More: [BUILDING.md](docs/BUILDING.md).

---

## Provenance and licence

**GPL-3.0-or-later.** The noise implementation was written by studying
[Pumpkin](https://github.com/Pumpkin-MC/Pumpkin), which is GPL-3.0; see
[CREDITS.md](CREDITS.md) for that and for the other prior art worth knowing about.

Minecraft is a trademark of Mojang Studios. This project is not affiliated with or
endorsed by Mojang or NeoForged. No Minecraft code or assets are redistributed.
